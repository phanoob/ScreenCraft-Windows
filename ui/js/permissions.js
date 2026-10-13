import { invoke, currentWindow, IS_WINDOWS } from "./tauri.js";

const $ = (id) => document.getElementById(id);

function setRow(rowId, stateId, btnId, status, labels) {
  const row = $(rowId);
  const ok = status === "granted";
  row.classList.toggle("ok", ok);
  row.classList.toggle("bad", !ok && status !== "notDetermined");
  $(stateId).textContent = { granted: "Allowed", denied: "Denied", restricted: "Restricted", notDetermined: "Not set" }[status] || status;
  const b = $(btnId);
  b.hidden = ok;
  b.textContent = status === "denied" || status === "restricted" ? "Open Settings" : labels;
}

async function refresh() {
  const s = await invoke("permission_status");
  setRow("rowScreen", "sScreen", "bScreen", s.screen ? "granted" : "notDetermined", "Grant");
  setRow("rowAx", "sAx", "bAx", s.accessibility ? "granted" : "denied", "Open Settings");
  setRow("rowInput", "sInput", "bInput", s.inputMonitoring ? "granted" : "notDetermined", "Allow");
  setRow("rowCamera", "sCamera", "bCamera", s.camera, "Allow");
  setRow("rowMic", "sMic", "bMic", s.microphone, "Allow");
  // Screen Recording / Input Monitoring only take effect after a restart when granted mid-session.
  const needsRestart = (s.screen && !s.screenReady) || (s.inputMonitoring && !s.inputReady);
  const all = s.screen && s.accessibility && s.inputMonitoring;
  $("bContinue").disabled = !all || needsRestart;
  $("note").classList.toggle("show", needsRestart);
  $("bRestart").hidden = !needsRestart;
  $("bSkip").hidden = !(s.screen && !needsRestart && !all);
  return s;
}

$("bScreen").onclick = async () => {
  const granted = await invoke("request_screen_access");
  if (!granted) await invoke("open_privacy_pane", { kind: "screen" });
  refresh();
};
async function media(kind, constraints, id) {
  const st = (await invoke("permission_status"))[kind === "camera" ? "camera" : "microphone"];
  if (st === "denied" || st === "restricted") return invoke("open_privacy_pane", { kind });
  try {
    const s = await navigator.mediaDevices.getUserMedia(constraints);
    s.getTracks().forEach((t) => t.stop());
  } catch {
    await invoke("open_privacy_pane", { kind });
  }
  refresh();
}
$("bCamera").onclick = () => media("camera", { video: true });
$("bMic").onclick = () => media("microphone", { audio: true });
$("bAx").onclick = () => invoke("open_privacy_pane", { kind: "accessibility" });
$("bInput").onclick = async () => {
  const ok = await invoke("request_input_monitoring");
  if (!ok) await invoke("open_privacy_pane", { kind: "input" });
  refresh();
};
$("bSkip").onclick = async () => {
  await invoke("show_recorder");
  currentWindow().close();
};
// Windows needs no grant for capture, typing focus or input; only camera and mic have switches.
if (IS_WINDOWS) {
  for (const id of ["rowScreen", "rowAx", "rowInput"]) $(id).style.display = "none";
  $("intro").textContent = "Recording needs no special permission on Windows. Camera and microphone follow the switches in Settings › Privacy & security.";
}
$("bRecheck").onclick = refresh;
$("bRestart").onclick = () => invoke("restart_app");
$("bContinue").onclick = async () => {
  await invoke("show_recorder");
  currentWindow().close();
};

await refresh();
setInterval(refresh, 1500);
addEventListener("focus", refresh);
