import { invoke, dialog, listen, FILE_BROWSER } from "../tauri.js";
import { t } from "../i18n.js";
import { $, P, dir } from "./state.js";
import { pause } from "./playback.js";
import { exportMp4, exportGif, qualityBitrate } from "../export.js";
import { outputSize } from "../render.js";
import { outDuration } from "./timemap.js";

const exp = {
  format: "mp4", res: 1080, fps: 60, blur: 1, codec: "avc", quality: 55, running: false, cancel: false,
  compress: false, cpreset: "balanced", keep: false,
};

/** Post-export compression presets (same encoder as the standalone compressor). */
const COMPRESS = {
  balanced: { codec: "h264", quality: 60, height: null, audio: "aac160" },
  small: { codec: "hevc", quality: 45, height: null, audio: "aac128" },
  tiny: { codec: "hevc", quality: 30, height: 720, audio: "aac96" },
};

const fmtMB = (b) => (b / 1e6).toFixed(b < 1e7 ? 2 : 1) + " MB";
const dirname = (p) => p.slice(0, p.lastIndexOf("/"));
const stripExt = (p) => p.replace(/\.[^./]+$/, "");

const AUDIO_KBPS = { aac160: 160, aac128: 128, aac96: 96 };

/** Rough output sizes from the same bit-per-pixel model the encoders are given. */
async function updateEstimates() {
  const dur = outDuration() / 1000;
  if (!P() || !dur) return;
  const { W, H } = outputSize(P(), exp.res);
  const hasAudio = P().media.hasMicrophone || P().media.hasSystemAudio || !!P().media.mic || !!(P().scene.musicClips || []).length;
  const audio = hasAudio ? 160_000 : 0;
  const fmt = (b) => fmtMB(b);
  let original;
  if (exp.format === "gif") {
    const w = Math.min(Math.min(exp.res, 540), 540);
    const { W: gw, H: gh } = outputSize(P(), w);
    original = gw * gh * 15 * dur * 0.09; // 256-colour LZW, screen content compresses well
  } else {
    original = ((qualityBitrate(W, H, exp.fps, exp.quality, exp.codec === "hevc") + audio) * dur) / 8;
  }
  $("estOriginal").textContent = "≈ " + fmt(original);

  const showC = exp.compress && exp.format === "mp4";
  $("estCompressedRow").hidden = !showC;
  $("estTotalRow").hidden = !(showC && exp.keep);
  if (!showC) return;
  const c = COMPRESS[exp.cpreset];
  const scale = c.height && c.height < H ? c.height / H : 1;
  const w = Math.round(W * scale), h = Math.round(H * scale);
  let rate = await invoke("estimate_bitrate", { width: w, height: h, fps: exp.fps, quality: c.quality, hevc: c.codec === "hevc" });
  const srcRate = qualityBitrate(W, H, exp.fps, exp.quality, exp.codec === "hevc");
  rate = Math.min(rate, srcRate * 0.98);
  const compAudio = hasAudio ? (AUDIO_KBPS[c.audio] || 128) * 1000 : 0;
  const comp = ((rate + compAudio) * dur) / 8;
  const pct = Math.max(0, Math.round((1 - comp / original) * 100));
  $("estCompressed").textContent = `≈ ${fmt(comp)}  ` + t("({pct}% smaller)", { pct });
  $("estTotal").textContent = "≈ " + fmt(original + comp);
}

function syncCompressUI() {
  $("expCompress").classList.toggle("on", exp.compress);
  $("compressOpts").hidden = !(exp.compress && exp.format === "mp4");
  $("rowCompress").hidden = exp.format !== "mp4";
  $("expKeep").checked = exp.keep;
  updateEstimates();
}

function bindSeg(id, key, conv = (v) => v) {
  const seg = $(id);
  seg.querySelectorAll("button").forEach((b) => {
    b.onclick = () => {
      seg.querySelectorAll("button").forEach((x) => x.classList.remove("on"));
      b.classList.add("on");
      exp[key] = conv(b.dataset.v);
      syncCompressUI();
    };
  });
}

export function openExport() {
  $("modal").hidden = false;
  updateEstimates();
}

function close() {
  if (exp.running) {
    exp.cancel = true;
    invoke("cancel_compress").catch(() => {});
    return;
  }
  $("modal").hidden = true;
  $("expProgress").hidden = true;
  $("expBar").style.width = "0";
}

async function run() {
  if (exp.running) return;
  const ext = exp.format;
  const base = (P().name || "ScreenCraft").replace(/[\\/:]/g, "-");
  const folder = await invoke("default_export_dir").catch(() => "");
  const forced = await invoke("test_export_path").catch(() => null);
  const path = forced || await dialog.save({ defaultPath: folder ? `${folder}/${base}.${ext}` : `${base}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  if (!path) return;
  exp.running = true;
  exp.cancel = false;
  pause();
  $("expProgress").hidden = false;
  $("expGo").disabled = true;
  const doCompress = exp.compress && ext === "mp4";
  // with compression the render is the first half of the bar, compressing the second half
  const scale = doCompress ? 0.55 : 1;
  const onProgress = (f, text) => {
    $("expBar").style.width = Math.round(f * scale * 100) + "%";
    $("expText").textContent = (doCompress ? t("Step 1 of 2 · ") : "") + text;
  };
  let unlisten = null;
  let rawPath = null;
  try {
    const project = JSON.parse(JSON.stringify(P()));
    const quality = exp.quality;
    // outputs are "name" (the plain export) and "name compressed"
    const compressedPath = `${stripExt(path)} compressed.mp4`;
    rawPath = doCompress ? (exp.keep ? path : `${stripExt(path)}.sc-tmp-${Date.now()}.mp4`) : path;
    const common = { project, dir, path: rawPath, resolution: exp.res, onProgress, shouldCancel: () => exp.cancel };
    if (ext === "gif") await exportGif(common);
    else await exportMp4({ ...common, fps: exp.fps, motionBlur: exp.blur ? 0.8 : 0, codec: exp.codec, quality });

    let summary = t("Saved! Revealing in {app}…", { app: t(FILE_BROWSER) });
    let finalPath = path;
    if (doCompress) {
      const c = COMPRESS[exp.cpreset];
      unlisten = await listen("compress-progress", (e) => {
        $("expBar").style.width = Math.round((0.55 + e.payload * 0.45) * 100) + "%";
        $("expText").textContent = t("Step 2 of 2 · Compressing… {pct}%", { pct: Math.round(e.payload * 100) });
      });
      $("expText").textContent = "Step 2 of 2 · Compressing…";
      const info = await invoke("probe_video", { path: rawPath });
      const size = await invoke("compress_video", {
        opts: { input: rawPath, output: compressedPath, codec: c.codec, quality: c.quality, height: c.height, fps: null, audio: c.audio, optimizeForWeb: true },
      });
      if (!exp.keep) await invoke("remove_file", { path: rawPath }).catch(() => {});
      rawPath = null;
      finalPath = compressedPath;
      summary = t("Saved! {a} → {b} · revealing in {app}…", { a: fmtMB(info.fileSize), b: fmtMB(size), app: t(FILE_BROWSER) });
    }
    $("expBar").style.width = "100%";
    $("expText").textContent = summary;
    invoke("reveal_in_finder", { path: finalPath });
    setTimeout(close, 1400);
  } catch (e) {
    $("expText").textContent = exp.cancel ? "Cancelled" : t("Export failed: {e}", { e: e?.message || e });
    if (rawPath && rawPath !== path && !exp.keep) invoke("remove_file", { path: rawPath }).catch(() => {});
  } finally {
    unlisten?.();
    exp.running = false;
    $("expGo").disabled = false;
  }
}

export function bindExport() {
  $("expCancel").onclick = close;
  $("expGo").onclick = run;
  bindSeg("expFormat", "format");
  bindSeg("expRes", "res", (v) => parseInt(v, 10));
  bindSeg("expFps", "fps", (v) => parseInt(v, 10));
  bindSeg("expBlur", "blur", (v) => parseInt(v, 10));
  bindSeg("expCodec", "codec");
  bindSeg("expQuality", "quality", (v) => parseInt(v, 10));
  bindSeg("expCPreset", "cpreset");
  $("expCompress").onclick = () => { exp.compress = !exp.compress; syncCompressUI(); };
  $("expKeep").onchange = (e) => { exp.keep = e.target.checked; syncCompressUI(); };
  syncCompressUI();
}
