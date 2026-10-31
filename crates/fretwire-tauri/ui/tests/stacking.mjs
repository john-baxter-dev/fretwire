// The stacking order of the overlays, checked from the CSS itself.
//
// This exists because of a bug that shipped (issue #32): Dialog sat at z-index 100 and IrPanel's
// overlay at 105, so the rename/delete/upload dialogs IrPanel opens rendered *behind* it — and the
// panel's click-outside handler took the first click and closed everything. Nothing about it is
// visible to a unit test of either component alone; it is a relation between two numbers in two
// files. So: every z-index in a component goes through a token in app.css, and the tokens are
// ordered here.
//
//   npm test        (from crates/fretwire-tauri/ui)

import fs from "node:fs";
import path from "node:path";

let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };

const css = fs.readFileSync("src/app.css", "utf8");
const z = Object.fromEntries([...css.matchAll(/--z-([\w-]+):\s*(\d+)\s*;/g)].map((m) => [m[1], +m[2]]));

for (const t of ["menu", "panel", "progress", "dialog", "toast"]) ok(t in z, `app.css defines --z-${t}`);

// A dialog is opened from a panel and over a progress overlay, never the other way round.
ok(z.dialog > z.panel, `--z-dialog (${z.dialog}) above --z-panel (${z.panel})`);
ok(z.dialog > z.progress, `--z-dialog (${z.dialog}) above --z-progress (${z.progress})`);
ok(z.toast > z.dialog, `--z-toast (${z.toast}) above --z-dialog (${z.dialog})`);
ok(z.panel > z.menu, `--z-panel (${z.panel}) above --z-menu (${z.menu})`);

// Every z-index in a component uses a token — a bare number is how the two drifted apart.
const svelteFiles = (dir) =>
  fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? svelteFiles(path.join(dir, e.name)) : e.name.endsWith(".svelte") ? [path.join(dir, e.name)] : [],
  );
const uses = {};
for (const f of svelteFiles("src")) {
  for (const m of fs.readFileSync(f, "utf8").matchAll(/z-index:\s*([^;]+);/g)) {
    const v = m[1].trim();
    const tok = v.match(/^var\(--z-([\w-]+)\)$/);
    ok(tok && tok[1] in z, `${f}: z-index "${v}" is a defined --z-* token`);
    if (tok) uses[path.basename(f)] = tok[1];
  }
}

// And the components that matter use the token they should.
ok(uses["Dialog.svelte"] === "dialog", "Dialog uses --z-dialog");
ok(uses["IrPanel.svelte"] === "panel", "IrPanel uses --z-panel");
ok(uses["GlobalsPanel.svelte"] === "panel", "GlobalsPanel uses --z-panel");
ok(uses["Toast.svelte"] === "toast", "Toast uses --z-toast");

console.log(`stacking: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
