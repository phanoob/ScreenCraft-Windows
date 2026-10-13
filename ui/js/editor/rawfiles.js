// Access to the untouched recordings (raw screen / camera files).
import { invoke, dialog, FILE_BROWSER } from "../tauri.js";
import { t } from "../i18n.js";
import { P, dir } from "./state.js";
import { showMenu } from "./menus.js";

async function saveCopy(rel, label) {
  const src = `${dir}/${rel}`;
  const ext = rel.split(".").pop();
  const name = `${(P().name || "recording").replace(/[\\/:]/g, "-")} ${label}.${ext}`;
  const dst = await dialog.save({ defaultPath: name, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  if (dst) {
    await invoke("copy_file", { src, dst });
    invoke("reveal_in_finder", { path: dst });
  }
}

export function rawMenu(anchor) {
  const m = P().media;
  const items = [
    { header: "Raw recording files" },
    { label: "Save raw screen video as…", run: () => saveCopy(m.screen, "(raw screen)") },
  ];
  if (m.camera) items.push({ label: "Save raw camera video as…", run: () => saveCopy(m.camera, "(raw camera)") });
  items.push(
    "-",
    { label: t("Show raw files in {app}", { app: t(FILE_BROWSER) }), run: () => invoke("reveal_in_finder", { path: `${dir}/${m.screen}` }) },
    { label: "Open raw video in compressor…", run: () => invoke("show_compressor", { input: `${dir}/${m.screen}` }) },
  );
  showMenu(anchor, items);
}
