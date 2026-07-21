// Quantifies how preset blocks can be resolved to a unique model when the display NAME collides.
//
// Background: the device preset stream identifies a block by display name + the undecoded path
// `11→6` (category). Names are not unique (cab mic/pan variants, amp vs preamp, legacy delays).
// This script asks: can the **param count** (which we DO parse, from the value vector) disambiguate
// without needing the `11→6` category decoded?
//
// Run from the repo root:  node tools/analyze-name-collisions.js
// Reads the shipped *.models files in crates/fretwire-data/data/ (authoritative: 681 models).
//
// Result (fw v3.71): 164 colliding names → 150 resolved by (name, param_count) alone; 11 need
// category too (all amp/preamp pairs with equal param counts); 3 true residual (the Line 6
// "2x12 Match H30/G25" duplicate-name data defect + per-device Input/Output, resolved by device
// context). Conclusion: decoding `11→6` is NOT required for ~91% of collisions — param count is.

const fs = require('fs');
const path = require('path');
const dir = path.join(__dirname, '..', 'crates', 'fretwire-data', 'data');

const models = [];
for (const f of fs.readdirSync(dir).filter(f => f.endsWith('.models'))) {
  let arr;
  try { arr = JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8')); } catch { continue; }
  if (!Array.isArray(arr)) continue;
  for (const m of arr) {
    if (!m || !m.symbolicID) continue;
    models.push({ id: m.symbolicID, name: m.name, file: f.replace('.models', ''),
                  cat: m.category, pc: Array.isArray(m.params) ? m.params.length : -1 });
  }
}
console.log(`models: ${models.length}  unique symbolicIDs: ${new Set(models.map(m => m.id)).size}`);

const byName = {};
for (const m of models) (byName[m.name] = byName[m.name] || []).push(m);
const colliding = Object.entries(byName).filter(([, a]) => new Set(a.map(x => x.id)).size > 1);

let pcOnly = 0, needCat = 0; const residual = [];
for (const [name, raw] of colliding) {
  const arr = Object.values(Object.fromEntries(raw.map(m => [m.id, m])));
  if (new Set(arr.map(m => m.pc)).size === arr.length) { pcOnly++; continue; }
  if (new Set(arr.map(m => m.pc + '|' + m.file)).size === arr.length) { needCat++; continue; }
  residual.push({ name, arr });
}
console.log(`colliding names: ${colliding.length}`);
console.log(`  resolved by (name, param_count) alone: ${pcOnly}`);
console.log(`  need (name, param_count, category): ${needCat}`);
console.log(`  true residual (neither resolves): ${residual.length}`);

console.log('\namp/preamp pairs with IDENTICAL param_count (these genuinely need category):');
for (const [name, raw] of colliding) {
  const arr = Object.values(Object.fromEntries(raw.map(m => [m.id, m])));
  const a = arr.find(m => m.file === 'amp'), p = arr.find(m => m.file === 'preamp');
  if (a && p && a.pc === p.pc) console.log(`  "${name}"  amp=${a.id}(pc${a.pc})  preamp=${p.id}(pc${p.pc})`);
}
console.log('\ntrue residual:');
for (const r of residual) console.log(`  "${r.name}": ` + r.arr.map(m => `${m.id}[${m.file},pc${m.pc}]`).join('  '));
