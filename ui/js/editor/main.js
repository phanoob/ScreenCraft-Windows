// Editor entry point: boot + event wiring. Everything else lives in sibling modules.
import { invoke, listen, currentWindow, FILE_BROWSER } from "../tauri.js";
import { t, languageItems } from "../i18n.js";
import { $, S, P, dir, hooks, media, clamp } from "./state.js";
import { outDuration } from "./timemap.js";
import { change, undo, redo, updateUndo, saveNow } from "./history.js";
import { draw, frameIndex, displayedRect } from "./draw.js";
import { play, pause, requestSeek, scheduleTrack, computeTrack, refreshSliceAudio, syncMusic } from "./playback.js";
import {
  renderTimeline, bindScrub, splitAtPlayhead, deleteSelected, addZoomAtPlayhead, autoZoom, duplicateSelected,
} from "./timeline.js";
import { renderPanel } from "./panels/index.js";
import { presetsMenu } from "./presets.js";
import { showMenu } from "./menus.js";
import { rawMenu } from "./rawfiles.js";
import { openExport, bindExport } from "./exportDialog.js";
import { contentRect, camAt } from "../render.js";
import { migrateMusic } from "./music.js";
import { Strip } from "./frames.js";
import { loadAudio } from "./audio.js";

hooks.updateTitle = () => {
  const t = $("title");
  if (t.isContentEditable) return;
  t.textContent = P().name + (S.dirty ? " *" : "");
  currentWindow().setTitle?.(P().name + (S.dirty ? " *" : "")).catch?.(() => {});
};
hooks.draw = draw;
hooks.syncMusic = () => { syncMusic(); syncBgImage(); };
/** Keep the decoded background image in step with the project (also after undo/redo). */
export function syncBgImage() {
  const bg = P()?.config.styles.background;
  if (!bg || bg.kind !== "image" || !bg.image) return;
  if (media.bgImage && media.bgImage.dataset.src === bg.image) return;
  const img = new Image();
  img.dataset.src = bg.image;
  img.onload = () => hooks.draw();
  img.src = bg.image;
  media.bgImage = img;
}
hooks.syncBgImage = syncBgImage;
hooks.renderTimeline = renderTimeline;
hooks.renderPanel = renderPanel;
hooks.scheduleTrack = scheduleTrack;
hooks.seek = (t) => requestSeek(t);
// rebuild what was built with t() (timeline labels, panels) in the new language
addEventListener("sc-langchange", () => {
  try {
    if (P()) { renderTimeline(); renderPanel(); }
  } catch {}
});

function syncControls() {
  $("selAspect").value = P().config.output.aspectRatio;
  const sel = S.selected?.type === "slice" && P().scene.slices.find((s) => s.id === S.selected.id);
  const s = sel || P().scene.slices[S.curSlice] || P().scene.slices[0];
  $("selSpeed").value = String(s.timeScale);
  hooks.updateTitle();
}
hooks.syncControls = syncControls;

/** Ask Save / Don't Save / Cancel; resolves to "save" | "discard" | "cancel". */
function askSave() {
  return new Promise((res) => {
    const m = $("saveAsk");
    m.hidden = false;
    $("saveAskName").textContent = P().name;
    const done = (v) => { m.hidden = true; res(v); };
    $("askSave").onclick = () => done("save");
    $("askDiscard").onclick = () => done("discard");
    $("askCancel").onclick = () => done("cancel");
  });
}

/** Closing the window (or quitting the app) with unsaved edits asks first. */
function guardClose() {
  currentWindow().onCloseRequested(async (ev) => {
    if (!S.dirty) return;
    ev.preventDefault();
    const choice = await askSave();
    if (choice === "cancel") {
      invoke("cancel_quit").catch(() => {});
      return;
    }
    if (choice === "save") {
      try {
        await saveNow();
      } catch (err) {
        alert(t("Could not save: {e}", { e: err }));
        invoke("cancel_quit").catch(() => {});
        return;
      }
    }
    S.dirty = false;
    currentWindow().close();
  });
}

/** Click the title to rename the project (and its folder). */
function bindTitle() {
  const el = $("title");
  el.title = "Click to rename";
  el.onclick = () => {
    if (el.isContentEditable) return;
    el.textContent = P().name; // drop the unsaved marker while editing
    el.contentEditable = "plaintext-only";
    el.classList.add("editing");
    el.focus();
    getSelection().selectAllChildren(el);
  };
  el.onkeydown = (e) => {
    e.stopPropagation();
    if (e.key === "Enter") { e.preventDefault(); el.blur(); }
    else if (e.key === "Escape") { el.textContent = P().name; el.blur(); }
  };
  el.onblur = async () => {
    el.contentEditable = "false";
    el.classList.remove("editing");
    const name = el.textContent.trim();
    if (!name || name === P().name) { hooks.updateTitle(); return; }
    try {
      await invoke("save_project", { dir, project: P() });
      const next = await invoke("rename_project", { dir, name });
      location.replace(`editor.html?dir=${encodeURIComponent(next)}`);
    } catch (err) {
      hooks.updateTitle();
      alert(String(err));
    }
  };
}

function bindUI() {
  bindTitle();
  $("btnPlay").onclick = () => (S.playing ? pause() : play());
  $("btnStart").onclick = () => { pause(); requestSeek(0); };
  $("btnEnd").onclick = () => { pause(); requestSeek(outDuration()); };
  $("btnUndo").onclick = undo;
  $("btnRedo").onclick = redo;
  $("btnSplit").onclick = splitAtPlayhead;
  $("btnDelete").onclick = deleteSelected;
  $("btnZoomHere").onclick = addZoomAtPlayhead;
  $("btnDuplicate").onclick = duplicateSelected;
  $("btnAutoZoom").onclick = autoZoom;
  $("btnFolder").onclick = async (e) => {
    const anchor = e.currentTarget; // currentTarget is cleared once we await
    const list = await invoke("recent_projects");
    const items = [{ header: "Projects" }];
    for (const pr of list.slice(0, 10)) items.push({ label: pr.name, run: () => invoke("open_editor", { dir: pr.dir }) });
    items.push("-", { label: t("Show this project in {app}", { app: t(FILE_BROWSER) }), run: () => invoke("reveal_in_finder", { path: dir }) });
    items.push("-", { header: "Language" });
    for (const l of languageItems()) items.push({ label: (l.checked ? "✓ " : "") + l.label, run: l.pick });
    showMenu(anchor, items);
  };
  $("btnCrop").onclick = () => { S.tab = "crop"; renderPanel(); };
  $("btnExport").onclick = openExport;
  $("btnRaw").onclick = (e) => rawMenu(e.currentTarget);
  $("btnPresets").onclick = (e) => presetsMenu(e.currentTarget);
  $("selAspect").onchange = (e) => change(null, () => (P().config.output.aspectRatio = e.target.value), { track: false });
  $("selPreview").onchange = (e) => { S.previewRes = parseInt(e.target.value, 10); draw(); };
  $("selSpeed").onchange = (e) => {
    const v = parseFloat(e.target.value);
    const target = S.selected?.type === "slice" ? P().scene.slices.find((s) => s.id === S.selected.id) : P().scene.slices[S.curSlice];
    if (!target) return;
    change(null, () => (target.timeScale = v), { timeline: true });
    if (S.playing) refreshSliceAudio();
  };

  // preview click: play/pause, or pick the manual zoom target
  $("preview").addEventListener("pointerdown", (e) => {
    const z = S.selected?.type === "zoom" && P().scene.zooms.find((x) => x.id === S.selected.id);
    if (!z || z.kind !== "manual") {
      S.playing ? pause() : play();
      return;
    }
    const c = $("preview"), rect = displayedRect();
    const px = ((e.clientX - rect.left) / rect.width) * c.width, py = ((e.clientY - rect.top) / rect.height) * c.height;
    const r = contentRect(P(), c.width, c.height);
    const cam = camAt(S.track, frameIndex());
    const crop = P().config.crop;
    const vwN = crop.width / cam.zoom, vhN = crop.height / cam.zoom;
    const nx = cam.cx - vwN / 2 + ((px - r.x) / r.w) * vwN, ny = cam.cy - vhN / 2 + ((py - r.y) / r.h) * vhN;
    change("manual-" + z.id, () => (z.manualTargetPoint = { x: clamp(nx, 0, 1), y: clamp(ny, 0, 1) }));
  });

  addEventListener("keydown", (e) => {
    if (["INPUT", "SELECT", "TEXTAREA"].includes(e.target.tagName)) return;
    const mod = e.metaKey || e.ctrlKey;
    if (e.code === "Space") { e.preventDefault(); S.playing ? pause() : play(); }
    else if (e.key === "s" && !mod) splitAtPlayhead();
    else if (e.key === "z" && !mod) addZoomAtPlayhead();
    else if (e.key === "d" && !mod) duplicateSelected();
    else if (e.key === "Backspace" || e.key === "Delete") { e.preventDefault(); deleteSelected(); }
    else if (e.key === "ArrowLeft") { pause(); requestSeek(S.tOut - (e.shiftKey ? 1000 : 1000 / 30)); }
    else if (e.key === "ArrowRight") { pause(); requestSeek(S.tOut + (e.shiftKey ? 1000 : 1000 / 30)); }
    else if (mod && e.key.toLowerCase() === "z") { e.preventDefault(); e.shiftKey ? redo() : undo(); }
    else if (mod && e.key.toLowerCase() === "y") { e.preventDefault(); redo(); }
    else if (mod && e.key.toLowerCase() === "e") { e.preventDefault(); openExport(); }
    else if (mod && e.key.toLowerCase() === "s") { e.preventDefault(); saveNow().catch((err) => alert(t("Could not save: {e}", { e: err }))); }
  });
  let rz = null;
  addEventListener("resize", () => {
    clearTimeout(rz);
    rz = setTimeout(() => { renderTimeline(); draw(); }, 80);
  });
  bindScrub();
  bindExport();
}

async function boot() {
  if (!dir) { $("loading").textContent = "No project specified."; return; }
  try {
    const loaded = await invoke("load_project", { dir });
    S.project = loaded.project;
    $("title").textContent = P().name;
    currentWindow().setTitle?.(P().name).catch?.(() => {});

    // Natively decoded preview proxy (built once per project, reused afterwards).
    $("app").inert = true;
    $("prep").hidden = false;
    $("loading").hidden = true;
    const un = await listen("proxy-progress", (e) => {
      $("prepBar").style.width = Math.round(e.payload * 100) + "%";
    });
    let meta;
    try {
      meta = await invoke("ensure_proxy", { dir, screen: P().media.screen, camera: P().media.camera || null });
    } finally {
      un();
    }
    $("prep").hidden = true;
    $("app").inert = false;
    if (!meta?.screen) throw new Error(t("could not prepare the preview"));
    media.strips.screen = new Strip("screen", meta.screen, dir);
    media.strips.camera = P().media.camera && meta.camera ? new Strip("camera", meta.camera, dir) : null;
    const arrive = () => { if (!S.playing) requestAnimationFrame(() => hooks.draw()); };
    media.strips.screen.onArrive = arrive;
    if (media.strips.camera) media.strips.camera.onArrive = arrive;

    // trust the real file over the recorded numbers
    const realMs = (meta.screen.n / meta.screen.fps) * 1000;
    if (realMs > 0 && Math.abs(realMs - P().media.durationMs) > 400) {
      for (const s of P().scene.slices) s.sourceEndMs = Math.min(s.sourceEndMs || realMs, realMs);
      P().media.durationMs = realMs;
    }

    const bg = P().config.styles.background;
    if (bg.kind === "image" && bg.image) syncBgImage();
    migrateMusic();
    bindUI();
    guardClose();
    renderPanel();
    syncControls();
    updateUndo();
    await computeTrack();
    requestSeek(0);
    loadAudio().then(() => { syncMusic(); renderTimeline(); }).catch((e) => console.warn("audio", e));
  } catch (e) {
    $("prep").hidden = true;
    $("app").inert = false;
    $("loading").hidden = false;
    $("loading").textContent = t("Could not open project: {e}", { e: e?.message || e });
    console.error(e);
  }
}
boot();
