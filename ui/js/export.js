import { masterGain, autoGain, makeLimiter, monoMix } from "./audiolevel.js";
import { Muxer, ArrayBufferTarget } from "../vendor/mp4-muxer.mjs";
import { GIFEncoder, quantize, applyPalette } from "../vendor/gifenc.esm.js";
import { invoke, convertFileSrc, SC_BASE } from "./tauri.js";
import { outputSize, paintFrame, camAt, lerpCam } from "./render.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// A small worker pool fetches + decodes full-resolution source frames (see srcframes.rs).
let pool = null;
let nextWorker = 0;
let nextId = 1;
const waiting = new Map();
function getPool() {
  if (pool) return pool;
  pool = [];
  for (let i = 0; i < 4; i++) {
    const w = new Worker("js/exportWorker.js");
    w.onmessage = (e) => {
      const { id, bmp, error } = e.data;
      const p = waiting.get(id);
      waiting.delete(id);
      if (!p) return;
      if (error) p.reject(new Error(error));
      else p.resolve(bmp);
    };
    pool.push(w);
  }
  return pool;
}
function endPool() {
  pool?.forEach((w) => w.terminate());
  pool = null;
  waiting.clear();
}

/** A full-resolution source frame decoded natively and bitmap-decoded in a worker. */
function srcFrame(dir, kind, tMs, quality = 90) {
  const url = `${SC_BASE}/src/${kind}?t=${Math.max(0, tMs).toFixed(3)}&d=${encodeURIComponent(dir)}&q=${quality}`;
  const id = nextId++;
  const w = getPool()[nextWorker++ % 4];
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    w.postMessage({ id, url });
  });
}

async function writeFile(path, bytes, onProgress) {
  await invoke("export_begin", { path });
  const CH = 4 * 1024 * 1024;
  for (let pos = 0; pos < bytes.byteLength; pos += CH) {
    const part = bytes.subarray(pos, Math.min(bytes.byteLength, pos + CH));
    await invoke("export_write", part, { headers: { "x-position": String(pos) } });
    onProgress?.(Math.min(1, (pos + CH) / bytes.byteLength));
  }
  await invoke("export_end");
}

async function decodeFile(url) {
  const bytes = await (await fetch(url)).arrayBuffer();
  const ctx = new (window.AudioContext || window.webkitAudioContext)();
  try {
    return await ctx.decodeAudioData(bytes);
  } catch {
    return null;
  } finally {
    ctx.close?.();
  }
}

/** Build the exported audio timeline: device audio and microphone mixed with their own levels. */
async function renderAudio(project, dir, outDurSec) {
  const a = project.config.audio;
  const tracks = [];
  const device = await decodeFile(convertFileSrc(`${dir}/${project.media.screen}`));
  if (device && device.length) tracks.push({ buf: device, gain: a.deviceVolume ?? 1, offset: 0, level: 1, mono: false });
  if (project.media.mic) {
    const mic = await decodeFile(convertFileSrc(`${dir}/${project.media.mic}`));
    if (mic && mic.length) tracks.push({ buf: mic, gain: a.micVolume ?? 1, offset: (project.media.micOffsetMs || 0) / 1000, level: autoGain(mic), mono: true });
  }
  const musicClips = [];
  const decoded = new Map();
  for (const c of project.scene.musicClips || []) {
    if (!decoded.has(c.file)) decoded.set(c.file, await decodeFile(convertFileSrc(`${dir}/${c.file}`)));
    const buf = decoded.get(c.file);
    if (buf && buf.length) musicClips.push({ c, buf });
  }
  if (!tracks.length && !musicClips.length) return null;
  const sr = 48000;
  const off = new OfflineAudioContext(2, Math.ceil(outDurSec * sr), sr);
  const master = masterGain(a.volume);
  const limiter = makeLimiter(off);
  limiter.connect(off.destination);
  for (const { c, buf } of musicClips) {
    const when = Math.max(0, c.offsetMs) / 1000;
    if (when >= outDurSec) continue;
    const g = off.createGain();
    g.gain.value = master * (a.musicVolume ?? 0.5);
    const node = off.createBufferSource();
    node.buffer = buf;
    node.connect(g).connect(limiter);
    node.start(when, Math.max(0, c.inMs) / 1000, Math.min(c.durMs / 1000, outDurSec - when));
  }
  let at = 0;
  for (const s of project.scene.slices) {
    const srcDur = (s.sourceEndMs - s.sourceStartMs) / 1000;
    const outDur = srcDur / s.timeScale;
    for (const t of tracks) {
      const from = s.sourceStartMs / 1000 + t.offset;
      const node = off.createBufferSource();
      node.buffer = t.buf;
      node.playbackRate.value = s.timeScale;
      const g = off.createGain();
      g.gain.value = s.volume * master * t.gain * t.level;
      if (t.mono) node.connect(monoMix(off)).connect(g);
      else node.connect(g);
      g.connect(limiter);
      // a mic that started slightly after the screen has a negative offset: delay it instead
      const when = at + Math.max(0, -from) / s.timeScale;
      const bufStart = Math.max(0, from);
      const dur = Math.max(0, srcDur - Math.max(0, -from));
      if (dur > 0) node.start(when, bufStart, dur);
    }
    at += outDur;
  }
  return off.startRendering();
}

/**
 * WebKit's AAC encoder hands back a whole MPEG-4 ES descriptor as the codec
 * description, but the muxer expects just the AudioSpecificConfig. Left alone the
 * descriptor ends up nested inside itself and QuickTime / AVFoundation see no audio.
 */
function fixAudioMeta(meta) {
  const d = meta?.decoderConfig?.description;
  if (!d) return meta;
  const b = d instanceof ArrayBuffer ? new Uint8Array(d) : new Uint8Array(d.buffer, d.byteOffset, d.byteLength);
  if (b[0] !== 0x03) return meta;
  let i = 0;
  while (i < b.length) {
    const tag = b[i++];
    let len = 0, k = 0;
    while (i < b.length) {
      const x = b[i++];
      len = (len << 7) | (x & 0x7f);
      if (!(x & 0x80) || ++k >= 4) break;
    }
    if (tag === 0x05) {
      const asc = b.slice(i, i + len);
      return { ...meta, decoderConfig: { ...meta.decoderConfig, description: asc } };
    }
    if (tag === 0x03) i += 3; // ES_ID + flags
    else if (tag === 0x04) i += 13; // object type, stream type, buffer size, bitrates
    else i += len;
  }
  return meta;
}

async function pickCodec(W, H, fps, bitrate, want) {
  const hevc = ["hvc1.1.6.L153.B0", "hvc1.1.6.L123.B0", "hvc1.1.6.L93.B0"];
  const avc = ["avc1.640034", "avc1.64002A", "avc1.640028", "avc1.4d0028", "avc1.42001f"];
  const groups = want === "hevc" ? [["hevc", hevc], ["avc", avc]] : [["avc", avc]];
  for (const [kind, list] of groups) {
    for (const codec of list) {
      const cfg = { codec, width: W, height: H, bitrate, framerate: fps };
      try {
        const sup = await VideoEncoder.isConfigSupported(cfg);
        if (sup.supported) return { cfg, kind };
      } catch {}
    }
  }
  throw new Error("This system cannot encode video in the web view.");
}

/** Bits-per-pixel model shared with the Rust compressor. */
export function qualityBitrate(W, H, fps, quality, hevc) {
  const q = Math.min(1, Math.max(0, quality / 100));
  let bpp = 0.015 + q * q * 0.2;
  if (hevc) bpp *= 0.62;
  return Math.round(Math.min(100e6, Math.max(250e3, W * H * fps * bpp)));
}

export async function exportMp4({ project, dir, path, resolution, fps, motionBlur, codec = "avc", quality = 55, onProgress, shouldCancel }) {
  if (typeof VideoEncoder === "undefined") throw new Error("WebCodecs is not available in this web view.");
  const { W, H } = outputSize(project, resolution);
  endPool();
  onProgress(0, "Computing camera path…");
  const track = await invoke("compute_render_track", { dir, project, fps });
  const hasCam = !!project.media.camera;

  const canvas = document.createElement("canvas");
  canvas.width = W; canvas.height = H;
  const ctx = canvas.getContext("2d", { willReadFrequently: false });
  const tmp = document.createElement("canvas");
  tmp.width = W; tmp.height = H;
  const tctx = tmp.getContext("2d");

  let bgImage = null;
  if (project.config.styles.background.kind === "image" && project.config.styles.background.image) {
    bgImage = new Image();
    bgImage.src = project.config.styles.background.image;
    await bgImage.decode().catch(() => {});
  }

  const bitrate = qualityBitrate(W, H, fps, quality, codec === "hevc");
  const { cfg: vcfg, kind: vkind } = await pickCodec(W, H, fps, bitrate, codec);

  onProgress(0.02, "Preparing audio…");
  const outDurSec = track.durationMs / 1000;
  let audio = null;
  try { audio = await renderAudio(project, dir, outDurSec); } catch { audio = null; }

  const target = new ArrayBufferTarget();
  const muxer = new Muxer({
    target,
    video: { codec: vkind, width: W, height: H, frameRate: fps },
    audio: audio ? { codec: "aac", sampleRate: audio.sampleRate, numberOfChannels: audio.numberOfChannels } : undefined,
    fastStart: "in-memory",
  });
  let encError = null;
  const venc = new VideoEncoder({
    output: (chunk, meta) => muxer.addVideoChunk(chunk, meta),
    error: (e) => (encError = e),
  });
  venc.configure(vcfg);

  const total = track.count;
  const src = { screen: null, camera: null, bgImage, cache: {} };
  const fetchPair = async (ti) => ({
    screen: await srcFrame(dir, "screen", track.tSrc[ti], 92),
    camera: hasCam ? await srcFrame(dir, "camera", track.tSrc[ti] + project.media.cameraOffsetMs, 88).catch(() => null) : null,
  });
  // keep a few source frames in flight: native decode and JPEG encode run ahead of painting
  const prof = { wait: 0, paint: 0, enc: 0 };
  const AHEAD = 6;
  const queue = [];
  const topUp = (i) => {
    for (let k = queue.length; k < AHEAD; k++) {
      const next = (queue.length ? queue[queue.length - 1].i : i - 1) + 1;
      if (next >= total) break;
      queue.push({ i: next, p: fetchPair(srcRatio(next)) });
    }
  };
  const srcRatio = (i) => Math.max(0, Math.min(total - 1, i));
  for (let i = 0; i < total; i++) {
    if (shouldCancel?.()) { venc.close(); throw new Error("cancelled"); }
    if (encError) throw encError;
    const t = srcRatio(i);
    const tOut = (i * 1000) / fps;
    src.screen?.close?.();
    src.camera?.close?.();
    // frame i+1 is requested as soon as frame i arrives, so native decode/encode overlaps painting
    if (!queue.length) topUp(i);
    const tw = performance.now();
    const head = queue.shift();
    const got = await head.p;
    prof.wait += performance.now() - tw;
    const tp = performance.now();
    topUp(i + 1);
    src.screen = got.screen;
    src.camera = got.camera;
    const cam = camAt(track, i);
    const prev = camAt(track, srcRatio(i - 1)), next = camAt(track, srcRatio(i + 1));
    const motion = Math.abs(next.zoom - prev.zoom) * 4 + Math.abs(next.cx - prev.cx) * 12 + Math.abs(next.cy - prev.cy) * 12;
    if (motionBlur > 0 && motion > 0.01) {
      const N = Math.min(6, 2 + Math.round(motion * 40));
      ctx.clearRect(0, 0, W, H);
      for (let k = 0; k < N; k++) {
        const f = N === 1 ? 0 : (k / (N - 1) - 0.5) * motionBlur;
        const c = f < 0 ? lerpCam(cam, prev, -f) : lerpCam(cam, next, f);
        paintFrame(tctx, W, H, project, c, tOut, track.clicks, src);
        ctx.globalAlpha = 1 / (k + 1);
        ctx.drawImage(tmp, 0, 0);
      }
      ctx.globalAlpha = 1;
    } else {
      paintFrame(ctx, W, H, project, cam, tOut, track.clicks, src);
    }
    prof.paint += performance.now() - tp;
    const te = performance.now();
    const frame = new VideoFrame(canvas, { timestamp: Math.round((i * 1e6) / fps), duration: Math.round(1e6 / fps) });
    venc.encode(frame, { keyFrame: i % (fps * 2) === 0 });
    frame.close();
    while (venc.encodeQueueSize > 12) await sleep(1);
    prof.enc += performance.now() - te;
    if ((i + 1) % 120 === 0) {
      invoke("dbg_log", { msg: `export 120 frames: wait ${(prof.wait / 120).toFixed(1)} ms, paint ${(prof.paint / 120).toFixed(1)} ms, encode ${(prof.enc / 120).toFixed(1)} ms per frame` }).catch(() => {});
      prof.wait = prof.paint = prof.enc = 0;
    }
    if (i % 4 === 0) onProgress(0.05 + 0.85 * (i / total), `Rendering frame ${i + 1} of ${total}`);
  }
  await venc.flush();
  venc.close();

  if (audio) {
    onProgress(0.92, "Encoding audio…");
    const aenc = new AudioEncoder({
      output: (chunk, meta) => muxer.addAudioChunk(chunk, fixAudioMeta(meta)),
      error: (e) => (encError = e),
    });
    aenc.configure({ codec: "mp4a.40.2", sampleRate: audio.sampleRate, numberOfChannels: audio.numberOfChannels, bitrate: 160000 });
    const STEP = 4096 * 4;
    const nch = audio.numberOfChannels;
    for (let pos = 0; pos < audio.length; pos += STEP) {
      const n = Math.min(STEP, audio.length - pos);
      const data = new Float32Array(n * nch);
      for (let c = 0; c < nch; c++) data.set(audio.getChannelData(c).subarray(pos, pos + n), c * n);
      const ad = new AudioData({
        format: "f32-planar", sampleRate: audio.sampleRate, numberOfFrames: n, numberOfChannels: nch,
        timestamp: Math.round((pos / audio.sampleRate) * 1e6), data,
      });
      aenc.encode(ad);
      ad.close();
      while (aenc.encodeQueueSize > 8) await sleep(2);
    }
    await aenc.flush();
    aenc.close();
  }
  if (encError) throw encError;
  muxer.finalize();
  onProgress(0.95, "Writing file…");
  await writeFile(path, new Uint8Array(target.buffer), (f) => onProgress(0.95 + 0.05 * f, "Writing file…"));
  endPool();
  onProgress(1, "Done");
}

export async function exportGif({ project, dir, path, resolution, onProgress, shouldCancel }) {
  const fps = 15;
  const res = Math.min(resolution, 540);
  const { W, H } = outputSize(project, res);
  endPool();
  onProgress(0, "Computing camera path…");
  const track = await invoke("compute_render_track", { dir, project, fps });
  const hasCam = !!project.media.camera;
  let bgImage = null;
  if (project.config.styles.background.kind === "image" && project.config.styles.background.image) {
    bgImage = new Image();
    bgImage.src = project.config.styles.background.image;
    await bgImage.decode().catch(() => {});
  }
  const canvas = document.createElement("canvas");
  canvas.width = W; canvas.height = H;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  const gifCache = {};
  const gifSrc = { screen: null, camera: null, bgImage, cache: gifCache };
  const gif = GIFEncoder();
  const delay = Math.round(1000 / fps);
  for (let i = 0; i < track.count; i++) {
    if (shouldCancel?.()) throw new Error("cancelled");
    gifSrc.screen?.close?.();
    gifSrc.camera?.close?.();
    gifSrc.screen = await srcFrame(dir, "screen", track.tSrc[i], 88);
    gifSrc.camera = hasCam ? await srcFrame(dir, "camera", track.tSrc[i] + project.media.cameraOffsetMs, 85).catch(() => null) : null;
    paintFrame(ctx, W, H, project, camAt(track, i), (i * 1000) / fps, track.clicks, gifSrc);
    const { data } = ctx.getImageData(0, 0, W, H);
    const palette = quantize(data, 256, { format: "rgb565" });
    const index = applyPalette(data, palette, "rgb565");
    gif.writeFrame(index, W, H, { palette, delay, repeat: 0 });
    if (i % 3 === 0) onProgress(0.9 * (i / track.count), `Encoding frame ${i + 1} of ${track.count}`);
    if (i % 8 === 0) await sleep(0);
  }
  gif.finish();
  onProgress(0.95, "Writing file…");
  await writeFile(path, gif.bytes());
  endPool();
  onProgress(1, "Done");
}
