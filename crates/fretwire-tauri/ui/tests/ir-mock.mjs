// Contract check for the mock IR backend, run with plain `node` — no test runner, no dependency.
//
// The mock exists so the panel can be developed and demonstrated without a pedal, which is only
// worth anything if it behaves like one. So these assert the device's actual rules: occupancy is
// the declared length rather than the presence of a name, the two listings answer with different
// fields, an occupied slot refuses a write without `overwrite`, and a rename leaves the samples
// alone.
//
//   npm test        (from crates/fretwire-tauri/ui)

import * as mock from "../src/mock/backend.js";
let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };
const throws = async (fn, frag, m) => {
  try { await fn(); fail++; console.error("FAIL (no throw):", m); }
  catch (e) { ok(String(e).includes(frag), `${m} — got: ${e}`); }
};

// Must match `IrSlotDto`'s serialized keys exactly — `dto.rs` pins the same list from the Rust
// side. The two drifting is the one bug neither side's own tests can see: the mock would keep
// passing while the real backend rendered `undefined`.
const DTO_KEYS = ["checksum", "display_name", "index", "md5", "name", "samples", "used"];

const dir = await mock.invoke("ir_list");
ok(
  JSON.stringify(Object.keys(dir[0]).sort()) === JSON.stringify(DTO_KEYS),
  `mock rows carry the DTO's keys, got ${Object.keys(dir[0]).sort()}`,
);
ok(dir.length === 3, `directory lists 3 populated slots, got ${dir.length}`);
ok(dir.every((s) => s.used), "every directory row is used");
ok(dir.every((s) => s.md5 && s.checksum === null), "directory carries md5 but no checksum");

const silent = dir.find((s) => s.index === 7);
ok(silent.display_name === "(unnamed)", "nameless slot renders as (unnamed)");
ok(silent.used && silent.checksum === null, "nameless slot is still used");

const scan = await mock.invoke("ir_scan");
ok(scan.length === 128, `scan returns all 128, got ${scan.length}`);
ok(scan.filter((s) => s.used).length === 3, "scan agrees on the used count");
ok(scan[1].display_name === "—" && !scan[1].used, "empty slot renders as a dash");
ok(scan[0].checksum !== null && scan[0].md5 === null, "scan carries checksum but no md5");

await throws(() => mock.invoke("ir_upload", { slot: 0, path: "/x/a.wav", name: "n", overwrite: false, force: false }),
  "already holds", "occupied slot refuses without overwrite");
await throws(() => mock.invoke("ir_upload", { slot: 200, path: "/x/a.wav", name: "n", overwrite: true, force: false }),
  "out of range", "slot 200 is refused");
await throws(() => mock.invoke("ir_upload", { slot: 1, path: "/x/rate44100.wav", name: "n", overwrite: false, force: false }),
  "48000 Hz", "a 44.1 kHz file is refused without force");

let after = await mock.invoke("ir_upload", { slot: 1, path: "/x/rate44100.wav", name: "forced", overwrite: false, force: true });
ok(after.length === 4, "forced upload lands");
ok(after.find((s) => s.index === 1).name === "forced", "uploaded name is stored");

const before = after.find((s) => s.index === 1).md5;
after = await mock.invoke("ir_rename", { slot: 1, name: "renamed" });
const row = after.find((s) => s.index === 1);
ok(row.name === "renamed", "rename lands");
ok(row.md5 === before, "rename leaves the samples (and hash) alone");

ok((await mock.invoke("ir_export", { slot: 1, path: "/tmp/o.wav" })) === "renamed", "export returns the name");
await throws(() => mock.invoke("ir_export", { slot: 2, path: "/tmp/o.wav" }), "empty", "exporting an empty slot fails");

after = await mock.invoke("ir_delete", { slot: 1 });
ok(after.length === 3, "delete removes the row");
ok(!(await mock.invoke("ir_scan"))[1].used, "deleted slot reads empty");

const long = "x".repeat(60);
after = await mock.invoke("ir_upload", { slot: 9, path: "/x/a.wav", name: long, overwrite: false, force: false });
ok(after.find((s) => s.index === 9).name.length === 31, "a long name is cut to 31");

// --- the inline pair (a browser's files travel in the call) ---
const file = await mock.invoke("ir_export_inline", { slot: 0 });
ok(file.name === "G12-65 212 C Hi-Gn 421+57", "inline export carries the stored name");
const wav = Buffer.from(file.wav_base64, "base64");
ok(wav.subarray(0, 4).toString() === "RIFF" && wav.subarray(8, 12).toString() === "WAVE", "inline export is a WAV");
ok(wav.readUInt32LE(24) === 48000, "inline export declares 48 kHz");
ok(wav.length === 44 + 2048 * 4, `inline export is 2048 float32 samples, got ${wav.length} bytes`);
await throws(() => mock.invoke("ir_export_inline", { slot: 2 }), "empty", "inline export of an empty slot fails");

const b64 = (bytes) => Buffer.from(bytes).toString("base64");
after = await mock.invoke("ir_upload_inline", { slot: 2, wavBase64: file.wav_base64, name: "roundtrip", overwrite: false, force: false });
ok(after.find((s) => s.index === 2)?.name === "roundtrip", "an exported WAV uploads back inline");
await throws(() => mock.invoke("ir_upload_inline", { slot: 2, wavBase64: file.wav_base64, name: "x", overwrite: false, force: false }),
  "already holds", "inline upload to an occupied slot needs overwrite");
const wav441 = Buffer.from(wav);
wav441.writeUInt32LE(44100, 24);
await throws(() => mock.invoke("ir_upload_inline", { slot: 4, wavBase64: b64(wav441), name: "x", overwrite: false, force: false }),
  "44100 Hz", "inline upload reads the rate off the header");
after = await mock.invoke("ir_upload_inline", { slot: 4, wavBase64: b64(wav441), name: "forced441", overwrite: false, force: true });
ok(after.find((s) => s.index === 4)?.name === "forced441", "force accepts the 44.1 kHz header");
await throws(() => mock.invoke("ir_upload_inline", { slot: 5, wavBase64: b64(Buffer.from("not a wav at all, definitely not forty-four bytes")), name: "x", overwrite: false, force: false }),
  "not a WAV", "inline upload refuses a non-WAV");
await throws(() => mock.invoke("ir_upload_inline", { slot: 5, wavBase64: "@@@", name: "x", overwrite: false, force: false }),
  "wav_base64", "inline upload names a garbled payload");

// --- preset numbering ---
// The store in lib/numbering.svelte.js matches these two literals exactly and ignores anything
// else, so a mismatch here would silently leave the toggle on its default instead of failing.
// The Rust side pins the same pair in commands.rs::numbering_tests.
const numbering = await mock.invoke("device_numbering");
ok(
  ["flat", "banked"].includes(numbering),
  `device_numbering returns a word the UI knows, got ${JSON.stringify(numbering)}`,
);

console.log(`${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
