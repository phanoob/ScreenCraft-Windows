// Start / stop recording (screen via Rust, camera via MediaRecorder).
import { invoke } from "../tauri.js";
import { $, state, showError, hooks } from "./state.js";
import { showBar, resizeBar } from "./window.js";
import { closeMenu, isMenuOpen } from "./menu.js";
import { getPreview, openCameraStream, startDrawLoop, syncPreview } from "./preview.js";
import { syncMeters } from "./meters.js";

let cameraRec = null;
let cameraStartedMs = null;
let micRec = null;
let micStartedMs = null;
let timerHandle = null;
let writeChain = Promise.resolve();
let micChain = Promise.resolve();

/** Record the microphone on its own so its level can be mixed independently of system audio. */
async function startMic() {
  if (!window.MediaRecorder) throw new Error("this web view cannot record audio");
  let deviceId;
  try {
    const devs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audioinput");
    deviceId = devs.find((d) => state.micName && d.label && d.label.includes(state.micName.slice(0, 10)))?.deviceId;
  } catch {}
  const stream = await Promise.race([
    navigator.mediaDevices.getUserMedia({
      audio: {
        deviceId: deviceId ? { ideal: deviceId } : undefined,
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
        channelCount: { ideal: 2 },
      },
      video: false,
    }),
    new Promise((_, rej) => setTimeout(() => rej(new Error("Microphone timed out")), 8000)),
  ]);
  // Always record two channels: a mono microphone is duplicated to left and right.
  const raw = stream;
  let recStream = raw;
  let ctx = null;
  try {
    ctx = new AudioContext({ sampleRate: 48000 });
    await ctx.resume();
    const src = ctx.createMediaStreamSource(raw);
    const dest = ctx.createMediaStreamDestination();
    dest.channelCount = 2;
    dest.channelCountMode = "explicit";
    dest.channelInterpretation = "speakers";
    // laptop mics record very quietly: lift the level, with a limiter against clipping
    const boost = ctx.createGain();
    boost.gain.value = 4;
    const lim = ctx.createDynamicsCompressor();
    lim.threshold.value = -3;
    lim.knee.value = 0;
    lim.ratio.value = 20;
    // average the channels (many mics leave one empty) and send the result to both ears
    boost.channelCount = 1;
    boost.channelCountMode = "explicit";
    boost.channelInterpretation = "speakers";
    src.connect(boost).connect(lim).connect(dest);
    recStream = dest.stream;
  } catch {
    ctx?.close?.().catch?.(() => {});
    ctx = null;
    recStream = raw;
  }
  const handle = {
    getTracks: () => [
      ...raw.getTracks(),
      { stop: () => ctx?.close?.().catch?.(() => {}) },
    ],
  };
  const mime = ["audio/mp4;codecs=mp4a.40.2", "audio/mp4", "audio/webm;codecs=opus", "audio/webm"].find((m) => MediaRecorder.isTypeSupported(m)) || "";
  let rec;
  try {
    rec = new MediaRecorder(recStream, mime ? { mimeType: mime, audioBitsPerSecond: 192000 } : {});
  } catch (e) {
    handle.getTracks().forEach((t) => t.stop());
    throw e;
  }
  return { stream: handle, rec, ext: mime.includes("mp4") ? "m4a" : "webm" };
}

async function startCamera() {
  if (!window.MediaRecorder) throw new Error("this web view cannot record camera video");
  const shared = getPreview();
  const stream = shared || (await openCameraStream());
  const candidates = [
    ["video/mp4;codecs=avc1", "mp4"],
    ["video/mp4", "mp4"],
    ["video/webm;codecs=vp9", "webm"],
    ["video/webm", "webm"],
  ];
  const pick = candidates.find(([m]) => MediaRecorder.isTypeSupported(m)) || ["", "webm"];
  let rec;
  try {
    rec = new MediaRecorder(stream, pick[0] ? { mimeType: pick[0], videoBitsPerSecond: 6_000_000 } : {});
  } catch (e) {
    if (!shared) stream.getTracks().forEach((t) => t.stop());
    throw e;
  }
  return { stream, rec, ext: pick[1], shared: !!shared };
}

async function ensurePermissions() {
  const p = await invoke("permission_status");
  if (!p.screen) {
    await invoke("show_permissions");
    return false;
  }
  if (state.camera && ["denied", "restricted"].includes(p.camera)) {
    showError("Camera access is off. Open Permissions from the ScreenCraft tray icon to fix it.");
    return false;
  }
  if (state.mic && ["denied", "restricted"].includes(p.microphone)) {
    showError("Microphone access is off. Open Permissions from the ScreenCraft tray icon to fix it.");
    return false;
  }
  return true;
}

export async function beginRecording() {
  if (state.phase !== "idle") return;
  if (!(await ensurePermissions())) return;
  if (isMenuOpen()) await closeMenu();
  if (state.mode === "window" && !state.windowId) return hooks.chooseMode("window");
  if (state.mode === "area" && !state.area) return hooks.chooseMode("area");

  let cam = null;
  let cameraNote = "";
  if (state.camera) {
    try {
      cam = await startCamera();
    } catch (e) {
      cameraNote = `Camera unavailable (${e?.message || e}) — recording without it.`;
    }
  }

  let mic = null;
  if (state.mic) {
    try {
      mic = await startMic();
    } catch (e) {
      cameraNote += (cameraNote ? " " : "") + `Microphone unavailable (${e?.message || e}).`;
    }
  }

  state.phase = "countdown";
  document.body.classList.add("countdown");
  $("hudLeft").style.display = "none";
  $("btnStop").style.display = "none";
  $("count").style.display = "block";
  await resizeBar(220, 60);
  for (let i = 3; i > 0; i--) {
    $("count").textContent = String(i);
    await new Promise((r) => setTimeout(r, 1000));
    if (state.phase !== "countdown") {
      if (cam && !cam.shared) cam.stream.getTracks().forEach((t) => t.stop());
      mic?.stream.getTracks().forEach((t) => t.stop());
      return;
    }
  }
  $("count").style.display = "none";

  try {
    await invoke("start_recording", {
      args: {
        mode: state.mode,
        displayId: state.displayId,
        windowId: state.mode === "window" ? state.windowId : null,
        area: state.mode === "area" ? state.area : null,
        systemAudio: state.sys,
        microphone: false, // the mic is recorded separately (see startMic)
        microphoneId: null,
        fps: state.fps,
        maxLongSide: state.resolution || null,
      },
    });
  } catch (e) {
    if (cam && !cam.shared) cam.stream.getTracks().forEach((t) => t.stop());
    mic?.stream.getTracks().forEach((t) => t.stop());
    showError(String(e));
    return resetUI();
  }

  if (mic) {
    await invoke("mic_begin", { ext: mic.ext });
    mic.rec.ondataavailable = (ev) => {
      if (!ev.data || !ev.data.size) return;
      micChain = micChain.then(async () => invoke("mic_chunk", await ev.data.arrayBuffer()));
    };
    micStartedMs = Date.now();
    mic.rec.start(500);
    micRec = mic;
  }

  if (cam) {
    await invoke("camera_begin", { ext: cam.ext });
    cam.rec.ondataavailable = (ev) => {
      if (!ev.data || !ev.data.size) return;
      writeChain = writeChain.then(async () => invoke("camera_chunk", await ev.data.arrayBuffer()));
    };
    cameraStartedMs = Date.now();
    cam.rec.start(500);
    cameraRec = cam;
    if (!cam.shared) {
      $("camSrc").srcObject = cam.stream;
      $("camSrc").play().catch(() => {});
      startDrawLoop();
    }
    $("hudCam").classList.add("on");
  } else {
    $("hudCam").classList.remove("on");
  }

  state.phase = "recording";
  if (cameraNote) showError(cameraNote);
  document.body.classList.remove("countdown");
  document.body.classList.add("hud");
  $("hudLeft").style.display = "flex";
  $("btnStop").style.display = "block";
  await resizeBar(cam ? 330 : 290, 60);
  const t0 = performance.now();
  timerHandle = setInterval(() => {
    const s = Math.floor((performance.now() - t0) / 1000);
    $("timer").textContent = `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  }, 250);
}

export async function finishRecording() {
  if (state.phase === "countdown") {
    state.phase = "idle";
    return resetUI();
  }
  if (state.phase !== "recording") return;
  state.phase = "finishing";
  clearInterval(timerHandle);
  $("timer").textContent = "Saving…";
  if (cameraRec) {
    await new Promise((res) => {
      cameraRec.rec.onstop = res;
      cameraRec.rec.stop();
    });
    if (!cameraRec.shared) cameraRec.stream.getTracks().forEach((t) => t.stop());
    $("hudCam").classList.remove("on");
    await writeChain;
  }
  if (micRec) {
    await new Promise((res) => {
      micRec.rec.onstop = res;
      micRec.rec.stop();
    });
    micRec.stream.getTracks().forEach((t) => t.stop());
    await micChain;
  }
  try {
    const dir = await invoke("stop_recording", {
      cameraStartedUnixMs: cameraRec ? cameraStartedMs : null,
      micStartedUnixMs: micRec ? micStartedMs : null,
    });
    await invoke("hide_area_frame");
    await invoke("open_editor", { dir });
  } catch (e) {
    showError(String(e));
  }
  cameraRec = null;
  micRec = null;
  resetUI();
}

async function resetUI() {
  state.phase = "idle";
  document.body.classList.remove("hud", "countdown");
  $("timer").textContent = "0:00";
  $("hudLeft").style.display = "flex";
  $("btnStop").style.display = "block";
  $("count").style.display = "none";
  await showBar();
  hooks.render();
  syncPreview();
  syncMeters();
}
