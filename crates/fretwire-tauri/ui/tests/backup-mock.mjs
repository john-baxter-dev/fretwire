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

// ---- whole-device backup and restore (format v3) ----
// The same sweep with the IR store and the global settings behind it, and a restore that writes
// only what the pedal does not already hold.
const irsBefore = await mock.invoke("ir_list");
const settingsBefore = await mock.invoke("settings_read", { all: false });
const stages = [];
const off2 = await mock.listen("backup-progress", (e) => stages.push(e.payload.stage));
const dev = await mock.invoke("backup_device_inline", { banks: [0], irs: true, settings: true, favorites: false, user_defaults: false });
off2();
ok(dev.count === list.length && dev.irs === irsBefore.length && dev.settings > settingsBefore.length,
  `device backup counts presets, IRs and every answering setting (${dev.count}/${dev.irs}/${dev.settings})`);
ok(stages.includes("presets") && stages.includes("irs") && stages.at(-1) === "settings", "progress names its stage, settings last");
const devFile = JSON.parse(dev.json);
ok(devFile.version === 3 && Array.isArray(devFile.irs) && Array.isArray(devFile.settings), "a device backup is a version-3 file");
ok(devFile.settings.every((s) => ["bool", "int", "f32"].includes(s.type)), "settings are typed");
const info = await mock.invoke("backup_info_inline", { json: dev.json });
ok(info.presets === dev.count && info.irs === dev.irs && info.settings === dev.settings && info.version === 3, "backup_info reads the counts back");
ok(info.device === restored.device_name, "the file names the device it came off");

// ---- favorites and user defaults (format v4) ----
// Asked for, they make the file version 4 and are counted back; left out (as above), the file
// stays version 3 so a 0.5 build reads it. An older client that names neither gets both.
const stages4 = [];
const off3 = await mock.listen("backup-progress", (e) => stages4.push(e.payload.stage));
const dev4 = await mock.invoke("backup_device_inline", { banks: [0], irs: true, settings: true });
off3();
const devFile4 = JSON.parse(dev4.json);
ok(devFile4.version === 4 && dev4.favorites === 2 && dev4.user_defaults === 1, `favorites and user defaults make a version-4 file (${dev4.favorites}/${dev4.user_defaults})`);
ok(devFile4.favorites[0].name === "US Princess" && devFile4.favorites[0].paired_cab === 709 && devFile4.favorites[1].paired_cab === null, "favorites carry name, model and paired cab");
ok(stages4.includes("favorites") && stages4.at(-1) === "user_defaults", "the two new stages are reported, user defaults last");
const info4 = await mock.invoke("backup_info_inline", { json: dev4.json });
ok(info4.favorites === 2 && info4.user_defaults === 1 && info4.version === 4, "backup_info counts them");
ok(!("favorites" in devFile), "a file that did not ask for them has no favorites section");

// A presets-only export is still version 2, so an older build reads it.
ok(parsed.version === 2 && !("irs" in parsed), "a presets-only export stays version 2");

// Restoring a fresh backup onto the same pedal writes nothing.
const same = await mock.invoke("restore_device_inline", { json: dev.json, presets: true, irs: true, settings: true });
ok(same.presets_written === 0 && same.presets_unchanged === dev.count, `nothing to write for presets (${same.presets_written}/${same.presets_unchanged})`);
ok(same.irs_written === 0 && same.irs_unchanged === dev.irs, "nothing to write for IRs");
ok(same.settings_written === 0 && same.settings_unchanged === settingsBefore.length, `nothing to write for settings (${same.settings_unchanged})`);
ok(same.settings_skipped.length === dev.settings - settingsBefore.length && same.settings_skipped[0].includes("not an identified"), "unidentified settings are skipped, never written");
ok(same.failures.length === 0 && same.skipped.length === 0, "no failures, nothing left unattempted");

// Change one setting and delete one IR; the restore puts exactly those back.
const tempo = settingsBefore.find((s) => s.id === 16);
await mock.invoke("settings_write", { id: 16, value: tempo.value + 7 });
await mock.invoke("ir_delete", { slot: irsBefore[0].index });
const fixed = await mock.invoke("restore_device_inline", { json: dev.json, presets: true, irs: true, settings: true });
ok(fixed.settings_written === 1 && fixed.irs_written === 1 && fixed.presets_written === 0, `one setting and one IR written (${fixed.settings_written}/${fixed.irs_written}/${fixed.presets_written})`);
const after = await mock.invoke("settings_read", { all: false });
ok(after.find((s) => s.id === 16).value === tempo.value, "the setting is back at its backed-up value");
ok((await mock.invoke("ir_list")).length === irsBefore.length, "the IR is back");

// Parts can be left out.
await mock.invoke("settings_write", { id: 16, value: tempo.value + 7 });
const partial = await mock.invoke("restore_device_inline", { json: dev.json, presets: false, irs: false, settings: false });
ok(partial.settings_written === 0 && partial.settings_unchanged === 0 && partial.presets_unchanged === 0, "nothing chosen, nothing touched");
await mock.invoke("settings_write", { id: 16, value: tempo.value });

// A file from another device is refused before anything is written.
const foreign = JSON.stringify({ ...devFile, device: "Some Other Pedal" });
await throws(() => mock.invoke("restore_device_inline", { json: foreign, presets: true, irs: true, settings: true }), "came off a", "a foreign file is refused");

console.log(`backup: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
