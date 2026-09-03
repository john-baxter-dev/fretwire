// The export/restore round trip through the `_inline` commands — the browser path, where the
// file is a download on the way out and text on the way back — run with plain `node`.
//
//   npm test        (from crates/fretwire-tauri/ui)

import * as mock from "../src/mock/backend.js";
let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };
const throws = async (fn, frag, m) => {
  try { await fn(); fail++; console.error("FAIL (no throw):", m); }
  catch (e) { ok(String(e).includes(frag), `${m} — got: ${e}`); }
};

const list = await mock.invoke("list_presets", { bank: 0 });
ok(list.length > 1, "the mock has presets to export");

// Before any export, the path variants have nothing to show; the inline ones don't need one.
await throws(() => mock.invoke("backup_show", { path: "/x.json" }), "nothing exported", "path show needs a prior export");
await throws(() => mock.invoke("backup_show_inline", { json: "{" }), "not a fretwire export file", "garbage is named as such");
await throws(() => mock.invoke("backup_show_inline", { json: JSON.stringify({ hello: 1 }) }), "not a fretwire export file", "the wrong shape is refused");

const progress = [];
const off = await mock.listen("backup-progress", (e) => progress.push(e.payload));
const file = await mock.invoke("export_setlists_inline", { banks: [0] });
off();
ok(file.count === list.length, `inline export counts every preset, got ${file.count}/${list.length}`);
ok(progress.length === list.length && progress.at(-1).done === list.length, "progress streamed once per preset");
const parsed = JSON.parse(file.json);
ok(parsed.format === "fretwire-backup" && parsed.presets.length === file.count, "the text is a fretwire-backup file");

const entries = await mock.invoke("backup_show_inline", { json: file.json });
ok(entries.length === file.count, "inline show lists the file's presets");
ok(entries[0].setlist != null, "entries carry the setlist name the file recorded");

const from = entries[1];
const target = list[0].index;
const restored = await mock.invoke("restore_preset_inline", { json: file.json, index: from.index, slot: target, bank: from.bank });
ok(restored.name === from.name && restored.index === target, `inline restore lands "${from.name}" in slot ${target}`);
await throws(() => mock.invoke("restore_preset_inline", { json: file.json, index: 999, slot: target, bank: 0 }), "no preset at", "a missing entry is named");

console.log(`backup: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
