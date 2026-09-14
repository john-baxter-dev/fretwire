// Every device push the backend can emit must be handled by the frontend.
//
// PushDto is `#[serde(tag = "kind")]`, so each variant's Rust name is a string the UI compares
// against — a pairing the compiler cannot check from either side. A variant added in dto.rs with
// no matching branch in App.svelte is silently inert: the device sends the change, the DTO
// serializes fine, and the GUI just never follows it. That is how `Selected` (the pedal's block
// cursor, issue #21) could have shipped decoded-but-ignored, and it is the same class of drift as
// the snake_case/camelCase argument bug that `invoke-args.mjs` now guards.
//
//   npm test        (from crates/fretwire-tauri/ui)

import fs from "node:fs";

let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };

const dto = fs.readFileSync("../../fretwire-commands/src/dto.rs", "utf8");
const app = fs.readFileSync("src/App.svelte", "utf8");
const mock = fs.readFileSync("src/mock/backend.js", "utf8");

// The variants of `pub enum PushDto { … }` — the block runs to the first line that closes it.
const body = dto.match(/pub enum PushDto \{([\s\S]*?)\n\}/)?.[1];
ok(!!body, "found the PushDto enum in dto.rs");
const variants = [...body.matchAll(/^\s{4}([A-Z][A-Za-z0-9]*)\s*\{/gm)].map((m) => m[1]);
ok(variants.length >= 4, `read the push kinds out of dto.rs (${variants.join(", ")})`);

for (const v of variants) {
  ok(app.includes(`p.kind === "${v}"`), `App.svelte handles a "${v}" push`);
}

// The mock is how the live-follow path is exercised without hardware, so a kind with no way to
// trigger it is a kind nobody will notice is broken until a pedal is plugged in.
for (const v of variants) {
  ok(mock.includes(`kind: "${v}"`), `the mock backend can emit a "${v}" push`);
}

console.log(`push-kinds: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
