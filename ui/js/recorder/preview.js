// Live camera preview inside the toolbar (and a thumbnail in the HUD).
import { $, state, showError } from "./state.js";
import { t } from "../i18n.js";

let preview = null;
let drawLoop = 0;

const withTimeout = (p, ms, what) =>
  Promise.race([p, new Promise((_, rej) => setTimeout(() => rej(new Error(what + " timed out")), ms))]);

export async function openCameraStream() {
  const base = { width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30 } };
  const attempts = [];
  if (state.cameraId) attempts.push({ ...base, deviceId: { ideal: state.cameraId } });
  attempts.push(base, true);
  let last;
  for (const video of attempts) {
    try {
      return await withTimeout(navigator.mediaDevices.getUserMedia({ video, audio: false }), 8000, "Camera");
    } catch (e) {
      last = e;
    }
  }
  throw last;
}

function drawSquare(canvas, video) {
  const g = canvas.getContext("2d", { willReadFrequently: true });
  const vw = video.videoWidth, vh = video.videoHeight;
  if (!vw || !vh) return;
  const s = Math.min(vw, vh);
  g.drawImage(video, (vw - s) / 2, (vh - s) / 2, s, s, 0, 0, canvas.width, canvas.height);
}

export function startDrawLoop() {
  const v = $("camSrc");
  const id = ++drawLoop;
  const tick = () => {
    if (id !== drawLoop) return;
    drawSquare($("camPreview"), v);
    if (state.phase === "recording") drawSquare($("hudCam"), v);
    setTimeout(tick, 50);
  };
  tick();
  setTimeout(() => {
    if (id !== drawLoop) return;
    if (!v.videoWidth) showError("The camera is not delivering video. Check Camera access in Permissions.");
  }, 3500);
}

export const getPreview = () => preview;

export function stopPreview() {
  drawLoop++;
  if (preview) preview.getTracks().forEach((t) => t.stop());
  preview = null;
  $("camSrc").srcObject = null;
  $("optCamera").classList.remove("live");
}

export async function syncPreview() {
  const want = state.camera && state.phase === "idle" && !document.hidden;
  if (!want) {
    if (state.phase === "idle") stopPreview();
    return;
  }
  if (preview) return;
  try {
    const s = await openCameraStream();
    if (!(state.camera && state.phase === "idle" && !document.hidden) || preview) {
      s.getTracks().forEach((t) => t.stop());
      return;
    }
    preview = s;
    const v = $("camSrc");
    v.srcObject = s;
    await v.play().catch(() => {});
    startDrawLoop();
    $("optCamera").classList.add("live");
  } catch (e) {
    stopPreview();
    showError(t("Camera preview unavailable: {e}", { e: e?.message || e }));
  }
}

document.addEventListener("visibilitychange", syncPreview);
