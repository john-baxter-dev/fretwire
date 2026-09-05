// Favorites through the mock: the list, and adding one (block + cab, the star on the block by
// name). Run with plain `node`.
//
//   npm test        (from crates/fretwire-tauri/ui)

import * as mock from "../src/mock/backend.js";
let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };

await mock.invoke("connect");
const favs = await mock.invoke("favorites");
ok(favs.length === 2 && favs[0].paired_index != null && favs[1].paired_index == null, "two favorites, the amp with a cab");
ok(favs.every((f) => f.symbolic_id && f.model_name && typeof f.dsp_load === "number"), "each carries its model, drawn and costed like any");

await mock.invoke("clear_preset");
let p = await mock.invoke("add_favorite", { index: 0 });
const added = p.blocks.find((b) => b.favorite === favs[0].name);
ok(added && added.model_index === favs[0].model_index && added.paired_index === favs[0].paired_index, "the favorite's block and cab are in the chain, starred by name");
ok(p.history.at(-1)?.includes(favs[0].name), `history names it: ${p.history.at(-1)}`);

p = await mock.invoke("add_favorite_at", { slot: added.slot + 1, index: 1 });
ok(p.blocks.filter((b) => b.favorite).length === 2, "a second favorite into a chosen slot");

let threw = false;
try { await mock.invoke("add_favorite", { index: 9 }); } catch (e) { threw = String(e).includes("no favorite"); }
ok(threw, "a missing index is named");

console.log(`favorites: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
