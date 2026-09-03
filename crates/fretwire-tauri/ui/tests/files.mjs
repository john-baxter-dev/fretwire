// The base64 helpers behind the `_inline` file commands, run with plain `node`. The Rust side
// decodes with the standard alphabet, padded; a helper that produced URL-safe or unpadded output
// would fail there and nowhere in the browser.
//
//   npm test        (from crates/fretwire-tauri/ui)

import { bytesToBase64, base64ToBytes, fileStem } from "../src/lib/files.js";
let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };

ok(bytesToBase64(new Uint8Array([])) === "", "empty in, empty out");
ok(bytesToBase64(new Uint8Array([0xfb, 0xff])) === "+/8=", "standard alphabet, padded");

// Larger than one chunk, with every byte value — round-trips and matches Node's own encoder.
const big = new Uint8Array(0x8000 * 3 + 7);
for (let i = 0; i < big.length; i++) big[i] = (i * 31) & 0xff;
const enc = bytesToBase64(big);
ok(enc === Buffer.from(big).toString("base64"), "matches Buffer's encoding across chunks");
const back = base64ToBytes(enc);
ok(back.length === big.length && back.every((b, i) => b === big[i]), "round-trips");

ok(bytesToBase64(big.buffer) === enc, "accepts an ArrayBuffer too");
ok(fileStem("Marshall 4x12.wav") === "Marshall 4x12", "stem drops the extension");
ok(fileStem("noext") === "noext", "stem of a bare name is itself");

console.log(`files: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
