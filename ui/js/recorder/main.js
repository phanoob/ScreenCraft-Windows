import { invoke, listen, IS_WINDOWS } from "../tauri.js";
import { t, languageItems } from "../i18n.js";
import { $, state, RESOLUTIONS, FPS_CHOICES, savePrefs, showError, hooks } from "./state.js";
import { win, showBar, enableDragging } from "./window.js";
import { openMenu, closeMenu, isMenuOpen } from "./menu.js";
import { syncPreview } from "./preview.js";
import { syncMeters } from "./meters.js";
import { cameraMenu, micMenu, systemMenu } from "./devices.js";
import { beginRecording, finishRecording } from "./capture.js";

function render() {
  document.querySelectorAll(".tool").forEach((b) => b.classList.toggle("active", b.dataset.mode === state.mode));
  const set = (id, on, text) => {
    const el = $(id);
    el.classList.toggle("off", !on);
    el.classList.toggle("on", on);
    el.querySelector("span.t").textContent = text;
  };
  set("optCamera", state.camera, state.camera ? state.cameraLabel || "Camera" : "No camera");
  set("optMic", state.mic, state.mic ? state.micName || "Microphone" : "No microphone");
  set("optSys", state.sys, state.sys ? "System audio" : "No system audio");
  const res = RESOLUTIONS.find((r) => r.value === state.resolution) || RESOLUTIONS[0];
  $("qualLabel").textContent = `${t(res.short)} · ${state.fps} fps`;
  $("recLabel").textContent = "Record";
  syncPreview();
  syncMeters();
}
hooks.render = render;
addEventListener("sc-langchange", render);

/** "Main Display" / "Display 2", in the UI language. */
const displayName = (d) => (d.isMain ? t("Main Display") : t("Display {n}", { n: d.name.match(/\d+$/)?.[0] ?? "" }));

async function refreshSources() {
  try {
    state.sources = await invoke("list_sources");
    if (state.displayId == null && state.sources.displays.length) {
      state.displayId = (state.sources.displays.find((d) => d.isMain) || state.sources.displays[0]).id;
    }
  } catch (e) {
    showError(String(e));
  }
}

async function chooseMode(mode) {
  if (isMenuOpen()) await closeMenu();
  state.mode = mode;
  if (mode !== "area") {
    invoke("hide_area_frame");
    invoke("close_area_picker"); // abandon a half-finished "draw the area"
    if (state.mode === "area" && !state.area) state.mode = mode;
  }
  savePrefs();
  render();
  await refreshSources();
  const anchor = document.querySelector(`.tool[data-mode="${mode}"]`);
  if (mode === "window" && state.sources) {
    const items = [{ header: "Windows" }];
    for (const w of state.sources.windows) {
      items.push({
        label: `${w.app}${w.title ? " — " + w.title : ""}`,
        sub: `${Math.round(w.width)} × ${Math.round(w.height)}`,
        checked: state.windowId === w.id,
        onPick: () => { state.windowId = w.id; state.windowTitle = w.title || w.app; render(); },
      });
    }
    if (items.length === 1) items.push({ label: "No windows found", onPick() {} });
    await openMenu(items, anchor);
  } else if (mode === "display" && state.sources && state.sources.displays.length > 1) {
    await openMenu(
      [
        { header: "Displays" },
        ...state.sources.displays.map((d) => ({
          label: displayName(d), sub: `${d.width} × ${d.height}`, checked: state.displayId === d.id,
          onPick: () => { state.displayId = d.id; render(); },
        })),
      ],
      anchor,
    );
  } else if (mode === "area") {
    await invoke("hide_area_frame");
    await invoke("open_area_picker", { displayId: state.displayId });
  }
}
hooks.chooseMode = chooseMode;

async function toggleMenu(fn) {
  if (isMenuOpen()) return closeMenu();
  await fn();
}

async function folderMenu() {
  const [projects, settings] = await Promise.all([invoke("recent_projects"), invoke("get_settings")]);
  const items = [{ header: "Recent Projects" }];
  for (const p of projects.slice(0, 8)) {
    items.push({ label: p.name, sub: new Date(p.modified * 1000).toLocaleString(), onPick: () => invoke("open_editor", { dir: p.dir }) });
  }
  if (items.length === 1) items.push({ label: "No projects yet", onPick() {} });
  items.push(
    { header: "Raw recordings folder" },
    { label: "Open recordings folder", sub: settings.projectsDir, onPick: () => invoke("reveal_in_finder", { path: settings.projectsDir }) },
    {
      label: "Choose recordings folder…",
      onPick: async () => {
        const dir = await window.__TAURI__.dialog.open({ directory: true, defaultPath: settings.projectsDir });
        if (dir) await invoke("set_projects_dir", { path: Array.isArray(dir) ? dir[0] : dir });
      },
    },
    { header: "Language" },
    ...languageItems().map((l) => ({ label: l.label, checked: l.checked, onPick: l.pick })),
  );
  await openMenu(items, $("optFolder"));
}

async function captureMenu() {
  // show real pixel sizes for the selected display when we know them
  const d = state.sources?.displays.find((x) => x.id === state.displayId);
  const dims = (long) => {
    if (!d || !long) return "";
    const k = long / Math.max(d.width, d.height);
    return `${Math.round(d.width * k)} × ${Math.round(d.height * k)}`;
  };
  await openMenu(
    [
      { header: "Resolution" },
      ...RESOLUTIONS.map((r) => ({
        label: r.label,
        sub: r.value
          ? dims(r.value)
          : d && !IS_WINDOWS
            ? t("{w} × {h} on Retina", { w: d.width * 2, h: d.height * 2 })
            : "Full pixel size of the display",
        checked: state.resolution === r.value,
        onPick: () => { state.resolution = r.value; savePrefs(); render(); },
      })),
      { header: "Frame rate" },
      ...FPS_CHOICES.map((f) => ({
        label: `${f} fps`,
        sub: f === 60 ? "Smoother motion, larger files" : "Lighter to record and edit",
        checked: state.fps === f,
        onPick: () => { state.fps = f; savePrefs(); render(); },
      })),
    ],
    $("optQuality"),
  );
}


document.querySelectorAll(".tool").forEach((b) => (b.onclick = () => chooseMode(b.dataset.mode)));
$("optCamera").onclick = () => toggleMenu(cameraMenu);
$("optMic").onclick = () => toggleMenu(micMenu);
$("optSys").onclick = () => toggleMenu(systemMenu);
$("optQuality").onclick = () => toggleMenu(captureMenu);
$("optFolder").onclick = () => toggleMenu(folderMenu);
$("btnRec").onclick = beginRecording;
$("btnStop").onclick = finishRecording;
$("btnClose").onclick = async () => {
  if (state.phase === "recording" || state.phase === "countdown") return;
  await invoke("quit_app");
};

await listen("area-selected", async (e) => {
  state.area = e.payload;
  state.mode = "area";
  savePrefs();
  render();
  try { await invoke("show_area_frame", { rect: e.payload, displayId: state.displayId }); } catch (err) { showError(String(err)); }
});
// the area picker closed without a selection: fall back to capturing the display
await listen("area-cancelled", async () => {
  if (state.mode !== "area") return;
  if (!state.area) await chooseMode("display");
  else invoke("show_area_frame", { rect: state.area, displayId: state.displayId }).catch(() => {});
});
await listen("toggle-record", () => (state.phase === "recording" ? finishRecording() : beginRecording()));
await listen("stop-record", () => finishRecording());
addEventListener("keydown", (e) => { if (e.key === "Escape" && isMenuOpen()) closeMenu(); });

enableDragging();
render();
if ((await invoke("permission_status")).screen) refreshSources();
else await invoke("show_permissions");
