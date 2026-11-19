// Confirming import is the only operation allowed to change the selected slot.
import assert from "node:assert/strict";
globalThis.window = {};
const mock = await import("../src/mock/backend.js");
const file = { version: 6, schema: "L6Preset", data: { device: 0x210006,
  meta: { name: "Imported drive" }, tone: { dsp0: { block0: {
    "@model": "drive_minotaur", "@path": 0, "@position": 0, "@enabled": false,
  }, split: { "@model": "HD2_AppDSPFlowSplitY", "@position": 0 },
  join: { "@model": "HD2_AppDSPFlowJoin", "@position": 0 } },
  global: { "@topology0": "A" } } } };
const json = JSON.stringify(file);
await mock.invoke("connect");
await assert.rejects(mock.invoke("hlx_preview", { json }), /HX Stomp only/);
window.fretwireMock.device("stomp");
await mock.invoke("connect");
const before = await mock.invoke("export_setlists_inline", { banks: [0] });
const current = await mock.invoke("read_preset");
const preview = await mock.invoke("hlx_preview", { json });
const previewId = preview.preview_id;
assert.ok(Number.isInteger(previewId));
assert.equal(preview.name, "Imported drive");
assert.equal(preview.blocks, 1);
assert.ok(preview.not_carried.length);
assert.deepEqual(await mock.invoke("read_preset"), current, "preview keeps current state");
assert.equal((await mock.invoke("export_setlists_inline", { banks: [0] })).json, before.json);
await assert.rejects(mock.invoke("restore_hlx", { previewId, bank: 0, slot: 2, allowPartial: false }), /acknowledge/);
for (const [bank, slot] of [[1, 0], [0, -1], [0, 126]]) {
  await assert.rejects(mock.invoke("restore_hlx", { previewId, bank, slot, allowPartial: true }), /slot 0 to 125/);
}
const foreign = structuredClone(file);
foreign.data.device = 0x21000b;
await assert.rejects(mock.invoke("hlx_preview", { json: JSON.stringify(foreign) }), /HX Stomp presets only/);
const duplicate = structuredClone(file);
duplicate.data.tone.dsp0.block1 = duplicate.data.tone.dsp0.block0;
await assert.rejects(mock.invoke("hlx_preview", { json: JSON.stringify(duplicate) }), /same path/);
const incomplete = structuredClone(file);
delete incomplete.data.tone.dsp0.join;
await assert.rejects(mock.invoke("hlx_preview", { json: JSON.stringify(incomplete) }), /routing node/);
const longName = structuredClone(file);
longName.data.meta.name = "é".repeat(17);
await assert.rejects(mock.invoke("hlx_preview", { json: JSON.stringify(longName) }), /32 UTF-8 bytes/);
const latest = await mock.invoke("hlx_preview", { json });
await assert.rejects(mock.invoke("restore_hlx", { previewId, bank: 0, slot: 2, allowPartial: true }), /preview expired/);
const list = await mock.invoke("list_presets", { bank: 0 });
window.fretwireMock.preset(2);
const switched = await mock.invoke("read_preset");
assert.notDeepEqual(switched.snapshot_names, current.snapshot_names);
const restore = { previewId: latest.preview_id, bank: 0, slot: 2, allowPartial: true };
const imported = await mock.invoke("restore_hlx", restore);
assert.equal(imported.name, "Imported drive");
assert.equal(imported.index, 2);
assert.equal(imported.device_model, "P33");
assert.deepEqual(imported.snapshot_names, current.snapshot_names, "restore uses the previewed donor");
await assert.rejects(mock.invoke("restore_hlx", restore), /preview expired/);
const reconnect = await mock.invoke("hlx_preview", { json });
await mock.invoke("disconnect");
await mock.invoke("connect");
await assert.rejects(mock.invoke("restore_hlx", { ...restore, previewId: reconnect.preview_id }), /preview expired/);
const after = await mock.invoke("list_presets", { bank: 0 });
assert.equal(after.find((p) => p.index === 2).name, "Imported drive");
assert.deepEqual(after.filter((p) => p.index !== 2), list.filter((p) => p.index !== 2));
console.log("hlx: preview, donor consistency, refusals and explicit destination passed");
