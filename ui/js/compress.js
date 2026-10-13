import { invoke, listen, dialog, FILE_BROWSER } from "./tauri.js";
import { t } from "./i18n.js";

const $ = (id) => document.getElementById(id);

const PRESETS = [
  { name: "Fast 1080p30", sub: "H.264 · quality 55", codec: "h264", height: 1080, fps: 30, quality: 55, audio: "aac128" },
  { name: "HQ 1080p30", sub: "H.264 · quality 75", codec: "h264", height: 1080, fps: 30, quality: 75, audio: "aac160" },
  { name: "Super HQ 1080p60", sub: "H.265 · quality 85", codec: "hevc", height: 1080, fps: null, quality: 85, audio: "keep" },
  { name: "Fast 720p30", sub: "H.264 · quality 50", codec: "h264", height: 720, fps: 30, quality: 50, audio: "aac128" },
  { name: "Very Fast 480p30", sub: "H.264 · quality 40", codec: "h264", height: 480, fps: 30, quality: 40, audio: "aac96" },
  { name: "Tiny share (HEVC)", sub: "H.265 · 720p · quality 30", codec: "hevc", height: 720, fps: 30, quality: 30, audio: "aac96" },
  { name: "Archive (same size)", sub: "H.265 · quality 70", codec: "hevc", height: null, fps: null, quality: 70, audio: "keep" },
];

let src = null; // probe result + path
let running = false;
let lastOutput = null;

function fmtSize(bytes) {
  if (bytes >= 1e9) return (bytes / 1e9).toFixed(2) + " GB";
  return (bytes / 1e6).toFixed(bytes < 1e7 ? 2 : 1) + " MB";
}
function fmtDur(s) {
  s = Math.round(s);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function applyPreset(p) {
  $("codec").value = p.codec;
  $("height").value = p.height ? String(p.height) : "";
  $("fps").value = p.fps ? String(p.fps) : "";
  $("quality").value = p.quality;
  $("audio").value = p.audio;
  document.querySelectorAll(".preset").forEach((b) => b.classList.toggle("on", b.dataset.name === p.name));
  update();
}

function buildPresets() {
  const box = $("presets");
  for (const p of PRESETS) {
    const b = document.createElement("button");
    b.className = "preset";
    b.dataset.name = p.name;
    // "quality" in its own text node, so the page translation picks it up
    b.innerHTML = `${p.name}<small>${p.sub.replace("quality", "<span>quality</span>")}</small>`;
    b.onclick = () => applyPreset(p);
    box.appendChild(b);
  }
}

function currentOpts() {
  return {
    codec: $("codec").value,
    quality: parseFloat($("quality").value),
    height: $("height").value ? parseInt($("height").value, 10) : null,
    fps: $("fps").value ? parseFloat($("fps").value) : null,
    audio: $("audio").value,
    optimizeForWeb: $("web").checked,
  };
}

async function update() {
  $("qLabel").textContent = $("quality").value;
  if (!src) return;
  const o = currentOpts();
  const scale = o.height && o.height < src.height ? o.height / src.height : 1;
  const w = Math.round(src.width * scale), h = Math.round(src.height * scale);
  const fps = Math.min(o.fps || src.fps || 30, src.fps || 30);
  let rate = await invoke("estimate_bitrate", { width: w, height: h, fps, quality: o.quality, hevc: o.codec === "hevc" });
  if (src.videoKbps > 0) rate = Math.min(rate, src.videoKbps * 1000 * 0.98);
  const audioKbps = !src.hasAudio || o.audio === "none" ? 0 : o.audio === "keep" ? 192 : parseInt(o.audio.replace("aac", ""), 10);
  const bytes = ((rate + audioKbps * 1000) * src.durationS) / 8;
  $("estSize").textContent = `${fmtSize(bytes)}${src.fileSize ? " " + t("({pct}% smaller)", { pct: Math.max(0, Math.round((1 - bytes / src.fileSize) * 100)) }) : ""}`;
  $("estRate").textContent = (rate / 1e6).toFixed(2) + " Mbps";
}

async function loadFile(path) {
  try {
    const p = await invoke("probe_video", { path });
    src = { ...p, path };
    $("srcName").textContent = path.split("/").pop();
    $("srcInfo").textContent = `${p.width}×${p.height} · ${p.fps.toFixed(0)} fps · ${fmtDur(p.durationS)} · ${fmtSize(p.fileSize)} · ${Math.round(p.videoKbps)} kbps${p.hasAudio ? " · " + t("audio") : ""}`;
    $("btnStart").disabled = false;
    $("btnReveal").hidden = true;
    if (!document.querySelector(".preset.on")) applyPreset(PRESETS[0]);
const presetInput = new URLSearchParams(location.search).get("input");
if (presetInput) loadFile(presetInput);
    update();
  } catch (e) {
    $("srcInfo").textContent = t("Could not read that file: {e}", { e });
  }
}

async function chooseFile() {
  const p = await dialog.open({
    multiple: false,
    filters: [{ name: t("Video"), extensions: ["mp4", "mov", "m4v", "mkv", "avi", "webm", "mpg", "mpeg"] }],
  });
  if (p) loadFile(Array.isArray(p) ? p[0] : p);
}

async function start() {
  if (!src || running) return;
  const base = src.path.split("/").pop().replace(/\.[^.]+$/, "");
  const out = await dialog.save({ defaultPath: `${base} (compressed).mp4`, filters: [{ name: "MP4", extensions: ["mp4"] }] });
  if (!out) return;
  running = true;
  lastOutput = out;
  $("progressCard").hidden = false;
  $("fill").style.width = "0";
  $("status").textContent = "Compressing…";
  $("btnStart").disabled = true;
  $("btnCancel").hidden = false;
  $("btnReveal").hidden = true;
  const t0 = performance.now();
  try {
    const size = await invoke("compress_video", { opts: { input: src.path, output: out, ...currentOpts() } });
    const secs = ((performance.now() - t0) / 1000).toFixed(0);
    const pct = src.fileSize ? Math.round((1 - size / src.fileSize) * 100) : 0;
    $("fill").style.width = "100%";
    $("status").textContent = t("Done in {secs}s — {a} → {b} ({pct}% smaller)", { secs, a: fmtSize(src.fileSize), b: fmtSize(size), pct });
    $("btnReveal").hidden = false;
  } catch (e) {
    $("status").textContent = String(e) === "cancelled" ? "Cancelled" : t("Failed: {e}", { e });
  } finally {
    running = false;
    $("btnStart").disabled = false;
    $("btnCancel").hidden = true;
  }
}

buildPresets();
for (const id of ["codec", "height", "fps", "audio", "quality", "web"]) {
  $(id).addEventListener("input", () => {
    document.querySelectorAll(".preset").forEach((b) => b.classList.remove("on"));
    update();
  });
}
$("btnOpen").onclick = chooseFile;
$("btnStart").onclick = start;
$("btnCancel").onclick = () => invoke("cancel_compress");
$("btnReveal").onclick = () => lastOutput && invoke("reveal_in_finder", { path: lastOutput });
await listen("compress-progress", (e) => {
  $("fill").style.width = Math.round(e.payload * 100) + "%";
  $("status").textContent = t("Compressing… {pct}%", { pct: Math.round(e.payload * 100) });
});
await listen("tauri://drag-drop", (e) => {
  $("drag").classList.remove("on");
  const p = e.payload?.paths?.[0];
  if (p) loadFile(p);
});
await listen("tauri://drag-enter", () => $("drag").classList.add("on"));
await listen("tauri://drag-leave", () => $("drag").classList.remove("on"));
applyPreset(PRESETS[0]);
const presetInput = new URLSearchParams(location.search).get("input");
if (presetInput) loadFile(presetInput);
const revealLabel = () => ($("btnReveal").textContent = t("Show in {app}", { app: t(FILE_BROWSER) }));
revealLabel();
addEventListener("sc-langchange", () => {
  revealLabel();
  update();
});
