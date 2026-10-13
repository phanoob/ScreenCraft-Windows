// UI language. Pages and code are written in English; this module translates in
// place every text node and title / placeholder / aria-label attribute whose English
// text is in the dictionary, including whatever the page adds later (a
// MutationObserver), so most of the UI needs no changes at all. Strings built from
// values go through t() with {placeholders}. Loaded first on every page.
//
// Element subtrees marked translate="no" (project names, file names) are left alone.
import fr from "./locales/fr.js";
import de from "./locales/de.js";
import { IS_WINDOWS } from "./tauri.js";

export const LANGS = [
  ["en", "English"],
  ["fr", "Français"],
  ["de", "Deutsch"],
];
const DICTS = { fr, de };
const STORE = "sc-lang";

function detect() {
  try {
    const saved = localStorage.getItem(STORE);
    if (saved === "en" || DICTS[saved]) return saved;
  } catch {}
  const sys = (navigator.language || "en").slice(0, 2).toLowerCase();
  return DICTS[sys] ? sys : "en";
}

let lang = detect();
let dict = DICTS[lang] || null;
export const getLang = () => lang;

const norm = (s) => s.replace(/\s+/g, " ").trim();

function lookup(en) {
  if (!dict) return null;
  const hit = dict[en];
  if (hit != null) return hit;
  // "Label (shortcut)": translate the label, keep the shortcut
  const m = /^(.*\S)(\s*\([^()]*\))$/.exec(en);
  if (m && dict[m[1]] != null) return dict[m[1]] + m[2];
  return null;
}

/** Translate an English UI string; {name} placeholders are filled from `vars`. */
export function t(en, vars) {
  let s = lookup(en) ?? en;
  if (vars) s = s.replace(/\{(\w+)\}/g, (all, k) => (k in vars ? String(vars[k]) : all));
  return s;
}

// ---------------------------------------------------------------- DOM

const ATTRS = ["title", "placeholder", "aria-label"];
// node -> { en, shown }: the English source and what we last wrote, so a language
// change re-translates from English and text set later by code is picked up anew
const memo = new WeakMap();

function translated(target, current, store) {
  const prev = store.get();
  const en = prev && prev.shown === current ? prev.en : norm(current);
  if (!en) return null;
  const out = lang === "en" ? (prev && prev.shown === current ? en : null) : lookup(en);
  if (out == null) return null;
  // keep the surrounding whitespace of text nodes
  const lead = current.match(/^\s*/)[0];
  const trail = current.slice(lead.length).match(/\s*$/)[0];
  const next = target === "text" ? lead + out + trail : out;
  store.set({ en, shown: next });
  return next === current ? null : next;
}

function skip(el) {
  return !el || el.closest?.('[translate="no"], script, style, [contenteditable="true"]');
}

function textNode(n) {
  if (skip(n.parentElement)) return;
  const next = translated("text", n.data, { get: () => memo.get(n), set: (v) => memo.set(n, v) });
  if (next != null) n.data = next;
}

function attr(el, name) {
  // translate="no" protects an element's content, not its tooltip
  if (el.closest('script, style') || skip(el.parentElement)) return;
  const cur = el.getAttribute(name);
  if (!cur) return;
  let m = memo.get(el);
  if (!m) memo.set(el, (m = {}));
  const next = translated("attr", cur, { get: () => m[name], set: (v) => (m[name] = v) });
  if (next != null) el.setAttribute(name, next);
}

function walk(root) {
  if (root.nodeType === Node.TEXT_NODE) return textNode(root);
  if (root.nodeType !== Node.ELEMENT_NODE) return;
  for (const a of ATTRS) if (root.hasAttribute(a)) attr(root, a);
  const it = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
  for (let n = it.nextNode(); n; n = it.nextNode()) {
    if (n.nodeType === Node.TEXT_NODE) textNode(n);
    else for (const a of ATTRS) if (n.hasAttribute(a)) attr(n, a);
  }
}

const englishTitle = document.title;
function translateDocument() {
  document.documentElement.lang = lang;
  walk(document.body);
  const title = t(englishTitle);
  if (document.title !== title) document.title = title;
  // the native title bar shows the title the window was created with
  if (title !== englishTitle || lang === "en") window.__TAURI__?.window.getCurrentWindow().setTitle(title).catch(() => {});
}

// Tooltips are written with macOS shortcuts; give the English source the Windows
// ones before translating it.
if (IS_WINDOWS) {
  for (const el of document.querySelectorAll("[title]")) {
    el.title = el.title.replace("⇧⌘", "Ctrl+Shift+").replace("⌘", "Ctrl+").replace("⌫", "Backspace").replace("Finder", "Explorer");
  }
}
translateDocument();
new MutationObserver((muts) => {
  for (const m of muts) {
    if (m.type === "characterData") textNode(m.target);
    else if (m.type === "attributes") attr(m.target, m.attributeName);
    else m.addedNodes.forEach(walk);
  }
}).observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRS });

// ---------------------------------------------------------------- switching

function apply(next) {
  if (next === lang || !(next === "en" || DICTS[next])) return;
  lang = next;
  dict = DICTS[next] || null;
  translateDocument();
  // pages re-render what they built with t()
  dispatchEvent(new CustomEvent("sc-langchange", { detail: lang }));
}

/** Change the language of every open window (and the tray menu). */
export async function setLang(next) {
  try {
    localStorage.setItem(STORE, next);
  } catch {}
  apply(next);
  const T = window.__TAURI__;
  await T.event.emit("sc-language", next).catch(() => {});
  await T.core.invoke("set_language", { lang: next }).catch(() => {});
}

/** Menu entries for a language picker ({ label, checked, pick }). */
export function languageItems() {
  return LANGS.map(([code, name]) => ({ code, label: name, checked: code === lang, pick: () => setLang(code) }));
}

window.__TAURI__?.event.listen("sc-language", (e) => apply(e.payload));
