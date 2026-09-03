// The update check's contract — the shape App.svelte and FirstRun.svelte rely on, mirroring
// fretwire_core::update: opt-in, the automatic form probes only once opted in, a forced one
// always does, and "available" means strictly newer.
//
//   npm test        (from crates/fretwire-tauri/ui)

import * as mock from "../src/mock/backend.js";
let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };

let s = await mock.invoke("update_status");
ok(s.enabled === null, "starts unanswered, so the editor asks");
ok(typeof s.current === "string" && s.latest === null && !s.available && s.url === null, "nothing known before a probe");
ok(["appimage", "package", "cargo", "source", "unknown"].includes(s.install) && s.instruction, "carries the install kind and what to do");

s = await mock.invoke("update_check", { force: false });
ok(s.latest === null, "the automatic check does not probe while unanswered");

s = await mock.invoke("update_pref", { enabled: false });
ok(s.enabled === false, "a 'no' is remembered");
s = await mock.invoke("update_check", { force: false });
ok(s.latest === null, "…and the automatic check stays home");

s = await mock.invoke("update_check", { force: true });
ok(s.latest !== null && s.available && s.url?.endsWith(`/releases/tag/v${s.latest}`), "an explicit check probes regardless and links the release");
ok(s.checked_at > 0, "the probe is timestamped");
ok(s.enabled === false, "checking now does not flip the preference");

s = await mock.invoke("update_pref", { enabled: true });
ok(s.enabled === true, "a 'yes' is remembered");
s = await mock.invoke("update_status");
ok(s.available && s.latest !== null, "status answers from the cache without probing");

console.log(`update-mock: ${pass} passed, ${fail} failed`);
if (fail) process.exit(1);
