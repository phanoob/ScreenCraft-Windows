// Preview frames come from the native proxy (see src-tauri/src/proxy.rs): one small
// JPEG per frame served on sc://localhost. Nothing here plays or seeks a <video>.
import { SC_BASE as BASE } from "../tauri.js";
const KEEP = 90; // decoded bitmaps kept around the playhead
const MAX_INFLIGHT = 12;

export class Strip {
  constructor(kind, meta, dir) {
    this.kind = kind;
    this.n = meta.n;
    this.fps = meta.fps;
    this.dir = encodeURIComponent(dir);
    this.cache = new Map(); // frame -> ImageBitmap
    this.pending = new Set();
    this.onArrive = null;
    this.center = 0;
  }

  index(ms) {
    return Math.max(0, Math.min(this.n - 1, Math.round((ms * this.fps) / 1000)));
  }

  /** The exact frame if decoded, else the nearest decoded one (so the picture never blanks). */
  peek(i) {
    const hit = this.cache.get(i);
    if (hit) return hit;
    for (let d = 1; d <= 20; d++) {
      const a = this.cache.get(i - d) || this.cache.get(i + d);
      if (a) return a;
    }
    return null;
  }

  /** Make sure frame `i` (and a few ahead in `dir`) are on their way. */
  want(i, dir = 0) {
    this.center = i;
    const list = [i];
    if (dir !== 0) for (let k = 1; k <= 12; k++) list.push(i + dir * k);
    else list.push(i - 1, i + 1);
    for (const f of list) this.fetch(f);
  }

  fetch(i) {
    if (i < 0 || i >= this.n || this.cache.has(i) || this.pending.has(i)) return;
    if (this.pending.size >= MAX_INFLIGHT) return;
    this.pending.add(i);
    fetch(`${BASE}/frame/${this.kind}/${i}?d=${this.dir}`)
      .then((r) => (r.ok ? r.blob() : Promise.reject(new Error("frame " + r.status))))
      .then((b) => createImageBitmap(b, { premultiplyAlpha: "none", colorSpaceConversion: "none" }))
      .then((bmp) => {
        this.pending.delete(i);
        this.cache.set(i, bmp);
        this.trim();
        this.onArrive?.(i);
      })
      .catch(() => this.pending.delete(i));
  }

  trim() {
    if (this.cache.size <= KEEP) return;
    const keys = [...this.cache.keys()].sort((a, b) => Math.abs(b - this.center) - Math.abs(a - this.center));
    for (const k of keys.slice(0, this.cache.size - KEEP)) {
      this.cache.get(k)?.close?.();
      this.cache.delete(k);
    }
  }
}
