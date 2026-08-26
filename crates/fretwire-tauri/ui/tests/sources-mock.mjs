// The controller-source ordinal space, which is the **device's size and not a constant**.
//
// Key `4` is a table of `footswitches + 5`: 0 none, 1/2 the expression inputs, one entry per
// footswitch from 3, then MIDI, then snapshots. So an HX Stomp runs 3..=7 with MIDI at 8, and an
// HX Stomp XL runs 3..=10 with MIDI at 11.
//
// It was read as a flat ten with 8 = MIDI until an XL owner assigned a Stupor OD's Drive to FS6 and
// sent the streams (issue #13, 2026-08-25). Ordinal 8 came back. Two things were wrong on an XL
// before that: the picker stopped at FS5, and an assignment the owner had made on the front panel
// displayed as "Driven by MIDI".
//
// Mirrors `fretwire_protocol::edit::source`, pinned on the Rust side against both devices' captures
// in `fretwire-core/tests/controller_table.rs`.
//
//   npm test        (from crates/fretwire-tauri/ui)

import * as mock from "../src/mock/backend.js";
let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };
const throws = async (fn, frag, m) => {
  try { await fn(); fail++; console.error("FAIL (no throw):", m); }
  catch (e) { ok(String(e).includes(frag), `${m} — got: ${e}`); }
};

// --- the arithmetic, at both counts we have captures for ---
ok(mock.sourceTableLen(5) === 10, "a five-switch device holds ten entries");
ok(mock.sourceTableLen(8) === 13, "an eight-switch device holds thirteen");
ok(mock.sourceSnapshots(5) === mock.sourceTableLen(5) - 1, "snapshots is the last entry (5)");
ok(mock.sourceSnapshots(8) === mock.sourceTableLen(8) - 1, "…and at 8 too");

// --- the same ordinal, two devices. This is the bug itself. ---
ok(mock.sourceName(8, 5) === "MIDI", "ordinal 8 is MIDI on a Stomp");
ok(mock.sourceName(8, 8) === "FS6", "ordinal 8 is FS6 on an XL");
ok(mock.sourceName(3, 5) === "FS1" && mock.sourceName(3, 8) === "FS1", "FS1 is 3 on both");
ok(mock.sourceName(9, 5) === "Snapshots", "9 is snapshots on a Stomp");
ok(mock.sourceName(9, 8) === "FS7", "…and FS7 on an XL");
ok(mock.sourceName(12, 8) === "Snapshots", "an XL's snapshots is 12");
ok(mock.sourceName(1, 8) === "EXP1" && mock.sourceName(2, 8) === "EXP2", "the pedals don't move");

// Past the end of the table, and the no-preset case: a bare ordinal, never a confident label.
ok(mock.sourceName(13, 8) === "Controller 13", "past the end is numbered, not named");
ok(mock.sourceName(8, 0) === "Controller 8", "with no device to size against, nothing is named");

// --- end to end, in whatever mode Node defaults to ---
const preset = await mock.invoke("read_preset", {});
const fs = preset.footswitch_count;
ok(fs > 0, `the mock preset reports a footswitch count, got ${fs}`);

const slot = preset.blocks.find((b) => b.params?.length)?.slot;
ok(slot !== undefined, "found a block with a parameter to drive");

for (const ordinal of [3, mock.sourceMidi(fs), mock.sourceSnapshots(fs)]) {
  const after = await mock.invoke("assign_param", { slot, paramIndex: 0, source: ordinal });
  const a = after.assignments.find((x) => x.source === ordinal);
  ok(a !== undefined, `ordinal ${ordinal} lands in the table`);
  ok(
    a?.source_name === mock.sourceName(ordinal, fs),
    `…named ${mock.sourceName(ordinal, fs)}, got ${a?.source_name}`,
  );
  await mock.invoke("assign_param", { slot, paramIndex: 0, source: 0 });
}

// The device accepts an out-of-range ordinal and silently does nothing, so we refuse it instead.
await throws(
  () => mock.invoke("assign_param", { slot, paramIndex: 0, source: mock.sourceTableLen(fs) }),
  "does not exist",
  "one past the end is refused rather than silently dropped",
);

// --- a bypass on an expression pedal ---
//
// Two destinations, chosen by the source: a footswitch writes the footswitch layout and arrives as
// `block.footswitch`, an expression pedal writes key `4` and arrives as an assignment with a target
// slot and **no parameter**. The param rows match on `param_index`, so these matched nothing and
// drew nothing — an XL owner's preset with bypasses on EXP1 and EXP2 rendered as if it had none.
//
// This pins the data path only. The badge itself is markup and there is no component harness here,
// so `ParamPanel`'s `bypassOnPedal` is checked by eye.
const presets = await mock.invoke("list_presets", { bank: 1 });
const wah = presets.find((p) => p.name === "The Blue Agave");
ok(wah !== undefined, "the mock ships a preset carrying one");

const loaded = await mock.invoke("goto_preset", { bank: 1, preset: wah.index });
const pedalBypass = loaded.assignments.filter((a) => a.param_index === null);
ok(pedalBypass.length === 1, `one bypass-on-pedal entry, got ${pedalBypass.length}`);
ok(pedalBypass[0]?.source_name === "EXP1", `named EXP1, got ${pedalBypass[0]?.source_name}`);
ok(pedalBypass[0]?.target_slot === 1, "pointing at the wah");
ok(pedalBypass[0]?.param_name === null, "and naming no parameter, because it drives none");

console.log(`${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
