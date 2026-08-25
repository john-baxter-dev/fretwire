// Contract check for the mock globals backend, run with plain `node` — no test runner.
//
// These settings write the pedal itself with no undo, so the rules the panel depends on are pinned
// here rather than discovered by clicking: which ids are writable, that a refused write throws
// rather than silently succeeding, that types survive a round trip, and that the DTO keys match the
// Rust side.
//
//   npm test        (from crates/fretwire-tauri/ui)

import * as mock from "../src/mock/backend.js";
let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };
const throws = async (fn, frag, m) => {
  try { await fn(); fail++; console.error("FAIL (no throw):", m); }
  catch (e) { ok(String(e).includes(frag), `${m} — got: ${e}`); }
};

// Must match `SettingDto`'s serialized keys exactly — `dto.rs::setting_tests` pins the same list
// from the Rust side. The two drifting renders `undefined` in the real app while the mock passes.
const DTO_KEYS = [
  "default", "group", "id", "kind", "labels", "name", "off", "options", "unit", "value", "writable",
];

const named = await mock.invoke("settings_read", { all: false });
ok(
  JSON.stringify(Object.keys(named[0]).sort()) === JSON.stringify(DTO_KEYS),
  `mock rows carry the DTO's keys, got ${Object.keys(named[0]).sort()}`,
);
// Bump this when the table grows. It went stale at 9dc105b and stayed that way, because CI builds
// the UI but never runs these — see `package.json`'s `test` script.
ok(named.length === 34, `34 settings in the catalog, got ${named.length}`);
ok(named.every((s) => s.writable), "every identified setting is writable");
ok(named.every((s) => s.group !== "Unidentified"), "no identified setting lands in the raw group");

const all = await mock.invoke("settings_read", { all: true });
ok(all.length > named.length, "the full sweep returns more than the named ones");
const raw = all.filter((s) => s.kind === "raw");
ok(raw.length > 0, "the full sweep includes unidentified ids");
ok(raw.every((s) => !s.writable), "unidentified ids are never writable");
ok(raw.every((s) => s.group === "Unidentified"), "unidentified ids are grouped as such");

// --- the three value types survive, because a type mismatch is what the device answers -3 to ---
const byId = (rows, id) => rows.find((s) => s.id === id);
ok(typeof byId(named, 27).value === "boolean", "a flag reads back as a bool");
ok(Number.isInteger(byId(named, 14).value), "a choice reads back as an int");
ok(typeof byId(named, 191).value === "number", "a number reads back as a number");

// --- writes ---
const wrote = await mock.invoke("settings_write", { id: 14, value: 2 });
ok(wrote.value === 2, `tempo scope wrote through, got ${wrote.value}`);
ok(wrote.id === 14 && wrote.kind === "choice", "the write echoes the same setting back");

const flag = await mock.invoke("settings_write", { id: 27, value: 1 });
ok(flag.value === true, "writing 1 to a flag stores a bool, not 1");
ok(
  (await mock.invoke("device_numbering")) === "flat",
  "device_numbering follows setting 27 — the preset list numbers itself from this",
);
await mock.invoke("settings_write", { id: 27, value: 0 });
ok((await mock.invoke("device_numbering")) === "banked", "…and back again");

const rounded = await mock.invoke("settings_write", { id: 73, value: 1.4 });
ok(rounded.value === 1, "a choice rounds rather than storing a float");

const float = await mock.invoke("settings_write", { id: 191, value: 1.4 });
ok(float.value === 1.4, "a float keeps its fraction");

await throws(
  () => mock.invoke("settings_write", { id: 128, value: 1 }),
  "not one fretwire has identified",
  "writing an unidentified id is refused",
);

// The EQ cuts have no separate enable — parking at the sentinel is how they turn off, so the panel
// has to be told which value that is.
ok(byId(named, 199).off === 19.9, "the low cut carries its off sentinel");
ok(byId(named, 200).off === 20100, "the high cut carries its off sentinel");
ok(byId(named, 16).off === null, "a setting with no off sentinel says null");

// Auto In-Z: the labels were read off the pedal on 2026-08-24, closing a gap this id carried while
// its two values were known and its menu text was not. An empty option list is still legal — that
// is asserted of the shape in `settings.rs`, not of an id, so it survives ids being explained.
ok(byId(named, 127).options.length === 2, "Auto In-Z carries both menu labels");
ok(byId(named, 127).writable, "…and is writable, because the id itself is identified");

// --- factory defaults ---
// The panel's reset buttons key off these, so "no observed default" must be null rather than 0 —
// resetting an unknown setting to zero would be a write nobody asked for.
ok(byId(named, 192).default === 0, "the EQ carries its factory gain");
ok(byId(named, 193).default === 2000, "…and its factory mid frequency");
ok(byId(named, 199).default === 19.9, "…and the cuts default to off");
ok(byId(named, 27).default === null, "a setting we have never watched reset offers no default");
ok(byId(named, 14).default === null, "…and neither does tempo scope");
ok(
  named.filter((s) => s.default != null).every((s) => s.group === "Global EQ"),
  "only the EQ claims a default",
);
ok(
  named.filter((s) => s.group === "Global EQ").every((s) => s.default != null),
  "…and every EQ parameter has one",
);
ok(all.filter((s) => s.kind === "raw").every((s) => s.default === null), "raw ids have no default");




// --- the toolbar's BPM field reads and writes the same two ids ---
// It is a second view of settings 16 and 14, not its own state, so the contract it depends on is
// that `settings_read` carries both and a write echoes the new value back.
ok(byId(named, 16).kind === "number" && byId(named, 16).unit === "BPM", "tempo is a number in BPM");
ok(byId(named, 14).kind === "choice", "tempo scope is a choice");
ok(
  byId(named, 14).options.some(([v, l]) => v === 2 && /global/i.test(l)),
  "…and names the global scope, which the toolbar shows beside the BPM",
);
const bpm = await mock.invoke("settings_write", { id: 16, value: 132.5 });
ok(bpm.value === 132.5, `a fractional BPM survives the write, got ${bpm.value}`);
ok(bpm.id === 16, "the write echoes the id the toolbar keys off");
await mock.invoke("settings_write", { id: 16, value: 120 });

// --- category colours ---
// The real backend reads these from the user's HX_ModelCatalog.json; the mock has no such file and
// must answer null, which is the fallback path a fresh install takes.
const cats = await mock.invoke("categories");
ok(cats.length > 0, "the mock lists categories");
ok(cats.every((c) => "color" in c), "every category row carries a colour field");
ok(cats.every((c) => c.color === null), "the mock has no reference data, so every colour is null");

console.log(`${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
