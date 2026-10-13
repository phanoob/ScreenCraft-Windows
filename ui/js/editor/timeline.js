// Timeline rendering and interaction. Dragging touches only the dragged
// element's style; the DOM is rebuilt only when the edit is committed.
import { invoke } from "../tauri.js";
import { t } from "../i18n.js";
import { $, S, P, dir, uid, clamp, fmt, hooks } from "./state.js";
import { sliceLen, outDuration, outToSrc, spansOut } from "./timemap.js";
import { addMusic, clips as musicClips, drawWave, removeMusicClip } from "./music.js";
import { change } from "./history.js";
import { requestSeek, pause, play, updateTransport, setTimelineWidth } from "./playback.js";

const tlWidth = () => Math.max(100, $("tlInner").clientWidth);
let rulerKey = "";

function renderRuler(dur, W, pxPerMs) {
  const key = `${Math.round(dur)}|${W}`;
  if (key === rulerKey) return;
  rulerKey = key;
  const ruler = $("ruler");
  const steps = [100, 200, 500, 1000, 2000, 5000, 10000, 15000, 30000, 60000, 120000, 300000];
  const step = steps.find((s) => s * pxPerMs >= 80) || 300000;
  let html = "";
  for (let t = 0; t <= dur; t += step) {
    const x = t * pxPerMs;
    html += `<div class="tick" style="left:${x}px"></div><div class="lab" style="left:${x}px">${fmt(t).replace(/\.0$/, "")}</div>`;
  }
  ruler.innerHTML = html;
}

export function renderTimeline() {
  if (!P()) return;
  const dur = outDuration();
  const W = tlWidth();
  setTimelineWidth(W);
  const pxPerMs = W / Math.max(1, dur);
  renderRuler(dur, W, pxPerMs);

  // clip track
  const clip = $("trackClip");
  const frag = document.createDocumentFragment();
  let acc = 0;
  for (const s of P().scene.slices) {
    const len = sliceLen(s);
    const el = document.createElement("div");
    el.className = "block clip" + (S.selected?.type === "slice" && S.selected.id === s.id ? " sel" : "");
    el.style.left = acc * pxPerMs + "px";
    el.style.width = Math.max(4, len * pxPerMs - 2) + "px";
    el.dataset.id = s.id;
    el.innerHTML = `<div class="h l"></div><span>Clip</span><span class="t">${(len / 1000).toFixed(1)}s${s.timeScale !== 1 ? ` · ${s.timeScale}×` : ""}</span><div class="h r"></div>`;
    el.onpointerdown = (e) => beginClipDrag(e, s, el, pxPerMs);
    frag.appendChild(el);
    acc += len;
  }
  clip.replaceChildren(frag);

  // zoom track: one block per stretch of the timeline that shows the region
  const zt = $("trackZoom");
  const zf = document.createDocumentFragment();
  for (const z of P().scene.zooms) {
    for (const [a, b] of spansOut(z.sourceStartMs, z.sourceEndMs)) {
      if (b - a < 1) continue;
      const el = document.createElement("div");
      el.className = "block zoom" + (z.isDisabled ? " disabled" : "") + (S.selected?.type === "zoom" && S.selected.id === z.id ? " sel" : "");
      el.style.left = a * pxPerMs + "px";
      el.style.width = Math.max(14, (b - a) * pxPerMs - 2) + "px";
      el.innerHTML = `<div class="h l"></div><span>Zoom</span><span class="t">${z.zoom.toFixed(1)}× · ${t(z.kind === "manual" ? "Manual" : "Auto")}</span><div class="h r"></div>`;
      el.onpointerdown = (e) => beginZoomDrag(e, z, el, pxPerMs, a, b);
      zf.appendChild(el);
    }
  }
  zt.replaceChildren(zf);

  // music track: one block per clip, with the file's waveform drawn inside
  const mt = $("trackMusic");
  const list = musicClips();
  if (!list.length) {
    const b = document.createElement("button");
    b.className = "add-music";
    b.textContent = "+ Add music";
    b.onclick = addMusic;
    mt.replaceChildren(b);
  } else {
    const mf = document.createDocumentFragment();
    for (const c of list) {
      const el = document.createElement("div");
      el.className = "block music" + (S.selected?.type === "music" && S.selected.id === c.id ? " sel" : "");
      el.style.left = c.offsetMs * pxPerMs + "px";
      el.style.width = Math.max(14, c.durMs * pxPerMs - 2) + "px";
      el.dataset.id = c.id;
      el.innerHTML = `<div class="h l"></div><canvas class="wave"></canvas><span class="mname">♪ ${c.name || t("Music")}</span><div class="h r"></div>`;
      el.onpointerdown = (e) => beginMusicDrag(e, c, el, pxPerMs);
      mf.appendChild(el);
    }
    mt.replaceChildren(mf);
    for (const el of mt.children) {
      const c = list.find((x) => x.id === el.dataset.id);
      if (c) drawWave(el.querySelector("canvas.wave"), c);
    }
  }
  updateTransport();
}

/** Clip block: edge handles trim, dragging the body re-orders, a click selects + seeks. */
function beginClipDrag(e, s, el, pxPerMs) {
  e.stopPropagation();
  e.preventDefault();
  el.setPointerCapture(e.pointerId);
  S.selected = { type: "slice", id: s.id };
  document.querySelectorAll("#trackZoom .block.sel, #trackClip .block.sel").forEach((b) => b.classList.remove("sel"));
  el.classList.add("sel");
  hooks.renderPanel();
  hooks.syncControls();

  const mode = e.target.classList.contains("l") ? "l" : e.target.classList.contains("r") ? "r" : "move";
  const x0 = e.clientX;
  const rect = $("tlInner").getBoundingClientRect();
  const left0 = parseFloat(el.style.left), width0 = parseFloat(el.style.width);
  const total = P().media.durationMs;
  const start0 = s.sourceStartMs, end0 = s.sourceEndMs;
  let moved = false;
  let ns = start0, ne = end0, drop = -1;

  el.onpointermove = (ev) => {
    const dx = ev.clientX - x0;
    if (!moved && Math.abs(dx) < 4) return;
    moved = true;
    if (mode === "move") {
      el.classList.add("dragging");
      el.style.transform = `translateX(${dx}px)`;
      // drop slot = number of other blocks whose centre is left of the pointer
      const px = ev.clientX - rect.left;
      let idx = 0;
      for (const b of $("trackClip").children) {
        if (b === el) continue;
        const c = parseFloat(b.style.left) + parseFloat(b.style.width) / 2;
        if (c < px) idx++;
      }
      drop = idx;
    } else if (mode === "l") {
      const dSrc = (dx / pxPerMs) * s.timeScale;
      ns = clamp(start0 + dSrc, 0, end0 - 200);
      const w = (end0 - ns) / s.timeScale * pxPerMs;
      el.style.left = left0 + width0 - w + "px";
      el.style.width = Math.max(8, w) + "px";
    } else {
      const dSrc = (dx / pxPerMs) * s.timeScale;
      ne = clamp(end0 + dSrc, start0 + 200, total);
      el.style.width = Math.max(8, ((ne - start0) / s.timeScale) * pxPerMs) + "px";
    }
  };
  el.onpointerup = (ev) => {
    el.onpointermove = el.onpointerup = null;
    try { el.releasePointerCapture(ev.pointerId); } catch {}
    el.classList.remove("dragging");
    el.style.transform = "";
    if (!moved) {
      requestSeek((ev.clientX - rect.left) / pxPerMs);
      return;
    }
    if (mode === "move") {
      const from = P().scene.slices.findIndex((x) => x.id === s.id);
      if (drop < 0 || drop === from) return renderTimeline();
      change(null, () => {
        const [it] = P().scene.slices.splice(from, 1);
        P().scene.slices.splice(drop, 0, it);
      }, { timeline: true });
    } else {
      change(null, () => { s.sourceStartMs = ns; s.sourceEndMs = ne; }, { timeline: true });
    }
  };
}

function beginZoomDrag(e, z, el, pxPerMs, a0, b0) {
  e.stopPropagation();
  e.preventDefault();
  el.setPointerCapture(e.pointerId);
  S.selected = { type: "zoom", id: z.id };
  S.tab = "zoom";
  document.querySelectorAll("#trackZoom .block.sel, #trackClip .block.sel").forEach((b) => b.classList.remove("sel"));
  el.classList.add("sel");
  hooks.renderPanel();
  hooks.syncControls();

  const mode = e.target.classList.contains("l") ? "l" : e.target.classList.contains("r") ? "r" : "move";
  const x0 = e.clientX;
  const total = outDuration();
  let na = a0, nb = b0, moved = false;

  el.onpointermove = (ev) => {
    if (Math.abs(ev.clientX - x0) < 3 && !moved) return;
    moved = true;
    const d = (ev.clientX - x0) / pxPerMs;
    let a = a0, b = b0;
    if (mode === "move") {
      const w = b0 - a0;
      a = clamp(a0 + d, 0, total - w);
      b = a + w;
    } else if (mode === "l") {
      a = clamp(a0 + d, 0, b0 - 300);
    } else {
      b = clamp(b0 + d, a0 + 300, total);
    }
    na = a; nb = b;
    el.style.left = a * pxPerMs + "px";
    el.style.width = Math.max(14, (b - a) * pxPerMs - 2) + "px";
  };
  el.onpointerup = (ev) => {
    el.onpointermove = el.onpointerup = null;
    try { el.releasePointerCapture(ev.pointerId); } catch {}
    if (!moved) return;
    const ns = outToSrc(na).src, ne = outToSrc(nb).src;
    change("zoom-drag-" + z.id, () => {
      const spans = spansOut(z.sourceStartMs, z.sourceEndMs);
      // the 1 ms eases a split leaves behind only make sense while the halves touch
      if (z.easeInMs === 1) z.easeInMs = null;
      if (z.easeOutMs === 1) z.easeOutMs = null;
      if (spans.length <= 1) {
        z.sourceStartMs = ns;
        z.sourceEndMs = ne;
        return;
      }
      // The region shows up in several places (clips were re-ordered): detach the stretch that
      // was dragged and keep every other stretch as its own region, so nothing disappears.
      const k = Math.max(0, spans.findIndex((sp) => Math.abs(sp[0] - a0) < 2 && Math.abs(sp[1] - b0) < 2));
      const copies = spans.map((sp, j) => (j === k ? null : { ...JSON.parse(JSON.stringify(z)), id: uid(), sourceStartMs: sp[2], sourceEndMs: sp[3] }));
      const zs = P().scene.zooms;
      const at = zs.indexOf(z);
      z.sourceStartMs = ns;
      z.sourceEndMs = ne;
      zs.splice(at + 1, 0, ...copies.filter(Boolean));
    }, { timeline: true });
  };
}

/** Music clip: body moves it freely, the ends trim it. */
function beginMusicDrag(e, c, el, pxPerMs) {
  e.stopPropagation();
  e.preventDefault();
  el.setPointerCapture(e.pointerId);
  S.selected = { type: "music", id: c.id };
  S.tab = "audio";
  document.querySelectorAll(".block.sel").forEach((b) => b.classList.remove("sel"));
  el.classList.add("sel");
  hooks.renderPanel();

  const mode = e.target.classList.contains("l") ? "l" : e.target.classList.contains("r") ? "r" : "move";
  const x0 = e.clientX;
  const total = outDuration();
  const { offsetMs: o0, inMs: i0, durMs: d0, fileDurMs: fd } = c;
  const cv = el.querySelector("canvas.wave");
  let v = { offsetMs: o0, inMs: i0, durMs: d0 };
  let moved = false;
  el.onpointermove = (ev) => {
    if (!moved && Math.abs(ev.clientX - x0) < 3) return;
    moved = true;
    const d = (ev.clientX - x0) / pxPerMs;
    if (mode === "move") {
      v = { offsetMs: clamp(o0 + d, 0, Math.max(0, total - 200)), inMs: i0, durMs: d0 };
    } else if (mode === "l") {
      const delta = clamp(d, Math.max(-i0, -o0), d0 - 200);
      v = { offsetMs: o0 + delta, inMs: i0 + delta, durMs: d0 - delta };
    } else {
      v = { offsetMs: o0, inMs: i0, durMs: clamp(d0 + d, 200, Math.max(200, (fd || d0 + i0) - i0)) };
    }
    el.style.left = v.offsetMs * pxPerMs + "px";
    el.style.width = Math.max(14, v.durMs * pxPerMs - 2) + "px";
    // keep the waveform where it was while an end is being trimmed
    if (cv) {
      cv.style.width = d0 * pxPerMs - 2 + "px";
      cv.style.left = (mode === "l" ? (o0 - v.offsetMs) * pxPerMs : 0) + "px";
    }
  };
  el.onpointerup = (ev) => {
    el.onpointermove = el.onpointerup = null;
    try { el.releasePointerCapture(ev.pointerId); } catch {}
    if (!moved) return;
    change(null, () => {
      c.offsetMs = Math.round(v.offsetMs);
      c.inMs = Math.round(v.inMs);
      c.durMs = Math.round(v.durMs);
    }, { track: false, timeline: true });
  };
}

// ---- scrubbing on ruler / empty track space
export function bindScrub() {
  const inner = $("tlInner");
  inner.addEventListener("pointerdown", (e) => {
    if (e.target.closest(".block, .add-music")) return;
    S.selected = null;
    document.querySelectorAll(".block.sel").forEach((b) => b.classList.remove("sel"));
    hooks.renderPanel();
    const wasPlaying = S.playing;
    if (wasPlaying) pause();
    S.scrubbing = true;
    inner.setPointerCapture(e.pointerId);
    const seekTo = (ev, fast) => {
      const r = inner.getBoundingClientRect();
      requestSeek(clamp(((ev.clientX - r.left) / r.width) * outDuration(), 0, outDuration()), fast);
    };
    seekTo(e, true);
    const mv = (ev) => seekTo(ev, true);
    const up = (ev) => {
      inner.removeEventListener("pointermove", mv);
      inner.removeEventListener("pointerup", up);
      try { inner.releasePointerCapture(ev.pointerId); } catch {}
      S.scrubbing = false;
      seekTo(ev, false); // exact frame + full-resolution redraw
      if (wasPlaying) play();
    };
    inner.addEventListener("pointermove", mv);
    inner.addEventListener("pointerup", up);
  });
}

// ---- edit operations
/**
 * Split at the playhead. Nothing selected: everything (clips and zoom regions) is cut.
 * A clip or zoom selected: only that one.
 */
export function splitAtPlayhead() {
  const sel = S.selected;
  const { src, idx } = outToSrc(S.tOut);
  const slices = P().scene.slices;
  const doClips = !sel || sel.type === "slice";
  const doZooms = !sel || sel.type === "zoom";

  // the clip to cut: the selected one if the playhead is on it, else the one under the playhead
  let clipIdx = -1;
  if (doClips) {
    if (sel?.type === "slice") {
      const si = slices.findIndex((x) => x.id === sel.id);
      if (si === idx) clipIdx = si;
    } else clipIdx = idx;
    if (clipIdx >= 0) {
      const s = slices[clipIdx];
      if (src <= s.sourceStartMs + 50 || src >= s.sourceEndMs - 50) clipIdx = -1;
    }
  }
  const doMusic = !sel || sel.type === "music";
  const mclips = doMusic
    ? musicClips().filter((c) => (sel ? c.id === sel.id : true) && S.tOut > c.offsetMs + 50 && S.tOut < c.offsetMs + c.durMs - 50)
    : [];
  const zooms = doZooms
    ? P().scene.zooms.filter((z) => (sel ? z.id === sel.id : true) && src > z.sourceStartMs + 50 && src < z.sourceEndMs - 50)
    : [];
  if (clipIdx < 0 && !zooms.length && !mclips.length) return;

  change(null, () => {
    if (clipIdx >= 0) {
      const s = slices[clipIdx];
      const right = { ...s, id: uid(), sourceStartMs: src };
      s.sourceEndMs = src;
      slices.splice(clipIdx + 1, 0, right);
      if (sel?.type === "slice") S.selected = { type: "slice", id: right.id };
    }
    for (const c of mclips) {
      const cut = S.tOut - c.offsetMs;
      const right = { ...c, id: uid(), offsetMs: S.tOut, inMs: c.inMs + cut, durMs: c.durMs - cut };
      c.durMs = cut;
      const list = musicClips();
      list.splice(list.indexOf(c) + 1, 0, right);
      if (sel?.type === "music") S.selected = { type: "music", id: right.id };
    }
    for (const z of zooms) {
      const right = { ...JSON.parse(JSON.stringify(z)), id: uid(), sourceStartMs: src };
      // tiny eases at the cut so the camera doesn't dip between the two halves
      z.sourceEndMs = src;
      z.easeOutMs = 1;
      right.easeInMs = 1;
      const at = P().scene.zooms.indexOf(z);
      P().scene.zooms.splice(at + 1, 0, right);
      if (sel?.type === "zoom") S.selected = { type: "zoom", id: right.id };
    }
  }, { panel: true, timeline: true });
}

export function duplicateSelected() {
  const sel = S.selected;
  if (!sel || sel.type !== "slice") return;
  const idx = P().scene.slices.findIndex((x) => x.id === sel.id);
  if (idx < 0) return;
  change(null, () => {
    const copy = { ...P().scene.slices[idx], id: uid() };
    P().scene.slices.splice(idx + 1, 0, copy);
    S.selected = { type: "slice", id: copy.id };
  }, { panel: true, timeline: true });
}

export function deleteSelected() {
  const sel = S.selected;
  if (!sel) return;
  if (sel.type === "music") {
    removeMusicClip(sel.id);
  } else if (sel.type === "zoom") {
    change(null, () => { P().scene.zooms = P().scene.zooms.filter((z) => z.id !== sel.id); S.selected = null; }, { panel: true, timeline: true });
  } else if (P().scene.slices.length > 1) {
    change(null, () => { P().scene.slices = P().scene.slices.filter((s) => s.id !== sel.id); S.selected = null; }, { panel: true, timeline: true });
    requestSeek(Math.min(S.tOut, outDuration()));
  }
}

export function addZoomAtPlayhead() {
  const { src, idx } = outToSrc(S.tOut);
  const sl = P().scene.slices[idx];
  const end = Math.min(src + 2500, sl.sourceEndMs);
  if (end - src < 400) return;
  const z = {
    id: uid(), zoom: P().config.animations.defaultZoom, kind: "follow-cursor", manualTargetPoint: { x: 0.5, y: 0.5 },
    isDisabled: false, sourceStartMs: src, sourceEndMs: end, instant: false,
  };
  change(null, () => { P().scene.zooms.push(z); S.selected = { type: "zoom", id: z.id }; S.tab = "zoom"; }, { panel: true, timeline: true });
}

export async function autoZoom() {
  const zooms = await invoke("auto_zoom", { dir, project: P(), zoom: P().config.animations.defaultZoom });
  change(null, () => { P().scene.zooms = zooms.map((z) => ({ ...z, id: uid() })); S.selected = null; }, { panel: true, timeline: true });
}
