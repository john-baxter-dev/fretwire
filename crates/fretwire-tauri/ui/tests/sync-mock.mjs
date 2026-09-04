// A tempo-sync group is one control: the `Tempo Sync` switch, the `Note Sync` value and the knob
// they govern are linked on the param DTO (`sync`), the way the real backend links them from
// HX_ModelCatalog.json's grouping, so the panel can hide two rows and fold both onto the third.
//
//   npm test        (from crates/fretwire-tauri/ui)

import * as mock from "../src/mock/backend.js";
let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };

await mock.invoke("connect");
const preset = await mock.invoke("read_preset");
const delay = preset.blocks.find((b) => b.params.some((p) => p.name === "Tempo Sync"));
ok(!!delay, "the mock preset has a delay with a sync pair");
const params = delay.params;
const byName = (n) => params.find((p) => p.name === n);
const time = byName("Time"), tempo = byName("Tempo Sync"), note = byName("Note Sync");
ok(time.sync?.role === "governed" && tempo.sync?.role === "tempo" && note.sync?.role === "note", "each member knows its role");
for (const p of [time, tempo, note]) {
  ok(p.sync.tempo === tempo.index && p.sync.note === note.index && p.sync.governed === time.index,
    `${p.name} carries the group's three indices`);
}
ok(params.filter((p) => p.sync).length === 3, "no other param is linked");
ok(note.enum_labels.length === 13 && note.enum_base === 1, "the note value keeps its labels and 1-based range for the folded select");

// The fold is a rendering rule; the wire stays three params, so writing the switch through the
// governed row's button is an ordinary set of the switch's own index.
await mock.invoke("set_param_enum", { slot: delay.slot, paired: false, paramIndex: tempo.index, value: 1 });
const after = (await mock.invoke("read_preset")).blocks.find((b) => b.slot === delay.slot).params;
ok(after.find((p) => p.index === tempo.index).value === 1, "the switch is written by its own index");
ok(after.find((p) => p.index === time.index).sync.role === "governed", "the link survives a re-read");

console.log(`sync: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
