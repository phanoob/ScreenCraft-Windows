// Background music: clips of audio files placed on the edited timeline.
// scene.musicClips = [{ id, file, name, offsetMs, inMs, durMs, fileDurMs }]
import { invoke, dialog, convertFileSrc } from "../tauri.js";
import { t } from "../i18n.js";
import { S, P, dir, uid, hooks } from "./state.js";
import { getMusicBuffer } from "./audio.js";
import { change } from "./history.js";

export const clips = () => P().scene.musicClips || (P().scene.musicClips = []);

/** Older projects held a single `scene.music`; turn it into a clip. */
export function migrateMusic() {
  const sc = P().scene;
  if (!sc.musicClips) sc.musicClips = [];
  if (sc.music && sc.music.file) {
    sc.musicClips.push({
      id: uid(), file: sc.music.file, name: sc.music.name || "Music",
      offsetMs: sc.music.offsetMs || 0, inMs: 0, durMs: sc.music.durationMs, fileDurMs: sc.music.durationMs,
    });
  }
  sc.music = null;
}

// ---- waveform peaks (computed once per file)
const peakCache = new Map();
const BINS = 3000;
export function getPeaks(file) {
  if (!peakCache.has(file)) {
    peakCache.set(file, (async () => {
      const buf = await getMusicBuffer(file);
      if (!buf) return null;
      const n = buf.length, per = Math.max(1, Math.floor(n / BINS));
      const out = new Float32Array(Math.ceil(n / per));
      for (let c = 0; c < buf.numberOfChannels; c++) {
        const d = buf.getChannelData(c);
        for (let i = 0; i < out.length; i++) {
          let m = 0;
          for (let k = i * per, e = Math.min(n, k + per); k < e; k++) {
            const v = d[k] < 0 ? -d[k] : d[k];
            if (v > m) m = v;
          }
          if (m > out[i]) out[i] = m;
        }
      }
      let peak = 0;
      for (const v of out) if (v > peak) peak = v;
      if (peak > 0) for (let i = 0; i < out.length; i++) out[i] /= peak;
      return { peaks: out, durMs: (n / buf.sampleRate) * 1000 };
    })());
  }
  return peakCache.get(file);
}

/** Paint the visible part of the file's waveform into `canvas`. */
export async function drawWave(canvas, clip) {
  const info = await getPeaks(clip.file);
  if (!info || !canvas.isConnected) return;
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(canvas.clientWidth * dpr)), h = Math.max(1, Math.round(canvas.clientHeight * dpr));
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = "rgba(255,255,255,.75)";
  const total = info.peaks.length;
  const i0 = (clip.inMs / info.durMs) * total, i1 = ((clip.inMs + clip.durMs) / info.durMs) * total;
  const mid = h / 2;
  for (let x = 0; x < w; x++) {
    const a = Math.floor(i0 + ((i1 - i0) * x) / w), b = Math.max(a + 1, Math.floor(i0 + ((i1 - i0) * (x + 1)) / w));
    let m = 0;
    for (let i = a; i < b && i < total; i++) if (info.peaks[i] > m) m = info.peaks[i];
    const bar = Math.max(1, m * h * 0.9);
    ctx.fillRect(x, mid - bar / 2, 1, bar);
  }
}

export async function addMusic() {
  const picked = await dialog.open({
    multiple: false,
    filters: [{ name: t("Audio"), extensions: ["mp3", "m4a", "aac", "wav", "aif", "aiff", "caf", "flac", "ogg"] }],
  });
  const path = Array.isArray(picked) ? picked[0] : picked;
  if (!path) return;
  const base = path.split("/").pop();
  const ext = (base.split(".").pop() || "mp3").toLowerCase();
  const file = `music-${Date.now()}.${ext}`;
  await invoke("copy_file", { src: path, dst: `${dir}/${file}` });
  const durationMs = await new Promise((res) => {
    const a = new Audio(convertFileSrc(`${dir}/${file}`));
    a.preload = "metadata";
    a.onloadedmetadata = () => res(isFinite(a.duration) ? a.duration * 1000 : 0);
    a.onerror = () => res(0);
  });
  if (!durationMs) {
    alert(t("That audio file couldn't be read."));
    return;
  }
  const clip = {
    id: uid(), file, name: base.replace(/\.[^.]+$/, ""),
    offsetMs: Math.round(S.tOut), inMs: 0, durMs: durationMs, fileDurMs: durationMs,
  };
  change(null, () => {
    clips().push(clip);
    S.selected = { type: "music", id: clip.id };
    S.tab = "audio";
  }, { panel: true, timeline: true, track: false });
  hooks.syncMusic?.();
}

export function removeMusicClip(id) {
  change(null, () => {
    P().scene.musicClips = clips().filter((c) => c.id !== id);
    if (S.selected?.type === "music") S.selected = null;
  }, { panel: true, timeline: true, track: false });
  hooks.syncMusic?.();
}
