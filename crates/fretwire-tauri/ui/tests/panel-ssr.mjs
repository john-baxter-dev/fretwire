// Renders ParamPanel for a handful of synthetic blocks, server-side, and fails if any throws.
//
// This exists because of a bug that shipped: `syncOf` read a bare `params`, which is the render
// snippet's parameter and not in scope in the script — so every block with a tempo-sync group
// (a delay's Time, a chorus's Rate) threw `ReferenceError: params is not defined` and its panel
// never opened, while the block still highlighted as selected. Nothing caught it: the data was
// valid, the Rust side was right, and a free variable is not a compile error.
//
//   npm test        (from crates/fretwire-tauri/ui)

import { compile } from "svelte/compiler";
import { render } from "svelte/server";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };

// Enough of a browser for the modules that touch one at import time.
globalThis.location = { hash: "", search: "", href: "http://localhost/", origin: "http://localhost" };
globalThis.window = globalThis;
globalThis.document = { addEventListener() {}, removeEventListener() {} };

const outDir = ".ssr-tmp";
fs.rmSync(outDir, { recursive: true, force: true });
fs.mkdirSync(outDir + "/lib/icons", { recursive: true });
fs.mkdirSync(outDir + "/mock", { recursive: true });
fs.copyFileSync("src/mock/backend.js", outDir + "/mock/backend.js");
for (const dir of ["", "icons/"]) {
  for (const f of fs.readdirSync("src/lib/" + dir)) {
    const src = "src/lib/" + dir + f;
    if (f.endsWith(".svelte")) {
      const { js } = compile(fs.readFileSync(src, "utf8"), { generate: "server", filename: f });
      fs.writeFileSync(
        `${outDir}/lib/${dir}${f}.js`.replace(/\.svelte\.js$/, ".svelte.js"),
        js.code.replace(/from "(\.[^"]*?)\.svelte"/g, 'from "$1.svelte.js"'),
      );
    } else if (f.endsWith(".js")) {
      fs.copyFileSync(src, `${outDir}/lib/${dir}${f}`);
    }
  }
}

const mod = await import(pathToFileURL(path.resolve(outDir, "lib/ParamPanel.svelte.js")).href);
const noop = () => {};

const param = (index, name, extra = {}) => ({
  index, name, value: 0.5, kind: "float", value_type: 1, min: 0, max: 1,
  enum_base: 0, enum_labels: [], stops: [], display_type: "generic_knob",
  settable: true, sync: null, format: null, default: 0.5, step: null, extra_index: null,
  ...extra,
});

// A tempo-sync group as the backend sends one: the governed knob, the note list, the switch.
const syncGroup = (governed, note, tempo) => [
  param(governed, "Time", { sync: { governed, note, tempo, role: "governed" } }),
  param(note, "Note Sync", {
    kind: "int", value_type: 0, value: 6, min: 1, max: 19, enum_base: 1,
    enum_labels: ["1/1", "1/2", "1/4"], display_type: "sync_note",
    sync: { governed, note, tempo, role: "note" },
  }),
  param(tempo, "Tempo Sync", {
    kind: "bool", value_type: 2, value: 0, min: null, max: null,
    display_type: "off_on", sync: { governed, note, tempo, role: "tempo" },
  }),
];

const block = (name, params, extra = {}) => ({
  slot: 3, dsp: 0, row: 0, model_name: name, user_label: null, custom_color: null,
  momentary: false, symbolic_id: "HD2_Test", category: 5, bypassed: false, variant: null,
  is_controller: false, footswitch: 0, dsp_load: 5, params, model_index: 100,
  paired_model_name: null, paired_index: null, paired_symbolic_id: null,
  paired_category: null, paired_params: [], favorite: null, ...extra,
});

const cases = [
  ["a plain block", block("Plain", [param(0, "Gain"), param(1, "Level")])],
  ["a tempo-sync group", block("Bucket Brigade", syncGroup(0, 6, 7))],
  ["sync off the front of the list", block("Trinity Chorus", [param(0, "Rate", { sync: { governed: 0, note: 12, tempo: 13, role: "governed" } }), ...syncGroup(0, 12, 13).slice(1)])],
  ["a sync group on the paired cab", block("Amp", [param(0, "Drive")], {
    paired_model_name: "1x12", paired_index: 709, paired_symbolic_id: "HD2_CabMicIr_1x12USDeluxe",
    paired_category: 19, paired_params: syncGroup(0, 6, 7),
  })],
  ["a governed param whose group members are missing", block("Half a group", [
    param(0, "Time", { sync: { governed: 0, note: 9, tempo: 10, role: "governed" } }),
  ])],
];

for (const [what, b] of cases) {
  try {
    const { body } = render(mod.default, {
      props: {
        block: b, dspLoad: 10, budget: 75, isNode: false, isSplit: false, splitTypes: [],
        assignments: [], irSlots: [], footswitchCount: 6, blockClip: null,
        onFloat: noop, onEnum: noop, onPreview: noop, onBypass: noop, onSwap: noop,
        onSplitType: noop, onDelete: noop, onCopyBlock: noop, onPasteBlock: noop,
        onBypassSwitch: noop, onSwitchLabel: noop, onSwitchColor: noop, onSwitchType: noop,
        onAssignParam: noop, onAssignTravel: noop,
      },
    });
    ok(body.includes(b.model_name), `${what}: panel renders`);
  } catch (e) {
    ok(false, `${what}: ${String(e).split("\n")[0]}`);
  }
}

fs.rmSync(outDir, { recursive: true, force: true });
console.log(`panel-ssr: ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
