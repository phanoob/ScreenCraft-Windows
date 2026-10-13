// Shared recorder state + persisted preferences.
import { t } from "../i18n.js";

export const $ = (id) => document.getElementById(id);

export const BAR_W = 1100;
export const BAR_H = 76;

/** Resolution choices: cap on the long side in px (0 = the display's native resolution). */
export const RESOLUTIONS = [
  { value: 0, label: "Native", short: "Native" },
  { value: 2560, label: "2560 px", short: "2560" },
  { value: 1920, label: "1080p class · 1920 px", short: "1080p" },
  { value: 1600, label: "1600 px", short: "1600" },
  { value: 1280, label: "720p class · 1280 px", short: "720p" },
];
export const FPS_CHOICES = [30, 60];

/** Snap any stored value to the nearest available resolution choice. */
function snapResolution(v) {
  if (!v) return 0;
  return RESOLUTIONS.filter((r) => r.value).reduce((best, r) => (Math.abs(r.value - v) < Math.abs(best - v) ? r.value : best), 2560);
}

function migrateResolution(q) {
  return q === "native" ? 0 : q === "light" ? 1440 : 2560;
}

const prefs = (() => {
  try { return JSON.parse(localStorage.getItem("sc-recorder") || "{}"); } catch { return {}; }
})();

export const state = {
  mode: prefs.mode || "display",
  displayId: null,
  windowId: null,
  windowTitle: "",
  area: null,
  camera: !!prefs.camera,
  cameraId: prefs.cameraId || null,
  cameraLabel: prefs.cameraLabel || "",
  mic: !!prefs.mic,
  micId: prefs.micId || null,
  micName: prefs.micName || "",
  sys: !!prefs.sys,
  resolution: snapResolution(prefs.resolution ?? migrateResolution(prefs.quality)),
  fps: prefs.fps ?? (prefs.quality === "native" ? 60 : 30),
  phase: "idle", // idle | countdown | recording | finishing
  sources: null,
};

export function savePrefs() {
  try {
    const { mode, camera, cameraId, cameraLabel, mic, micId, micName, sys, resolution, fps } = state;
    localStorage.setItem("sc-recorder", JSON.stringify({ mode, camera, cameraId, cameraLabel, mic, micId, micName, sys, resolution, fps }));
  } catch {}
}

let errTimer = null;
export function showError(msg) {
  const e = $("err");
  e.textContent = msg ? t(msg) + "  " + t("(click to dismiss)") : "";
  e.style.display = msg ? "flex" : "none";
  e.onclick = () => (e.style.display = "none");
  clearTimeout(errTimer);
  if (msg) errTimer = setTimeout(() => (e.style.display = "none"), 9000);
}

/** Hooks registered by main.js so modules can trigger a re-render without importing each other. */
export const hooks = { render: () => {} };
