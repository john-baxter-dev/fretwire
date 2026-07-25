// In-memory mock of the fretwire-tauri Rust backend, for frontend development without an HX Stomp (or even
// a Rust/Tauri toolchain). It implements every `#[command]` in `crates/fretwire-tauri/src/commands.rs` and
// returns the exact DTO shapes from `src/dto.rs`, so the Svelte UI behaves identically to the real
// thing — connect, browse presets, edit params, swap/add/delete blocks, snapshots, split routing,
// drag-to-place, and live-follow device pushes.
//
// It is wired in via `../lib/ipc.js`, which routes `invoke`/`listen` here whenever the app runs
// outside a Tauri webview (i.e. the Vite dev server in a browser). The real backend is untouched.
//
// State is intentionally persistent for the session: edits mutate the in-memory setlist, so the app
// feels stateful. Reload the page to reset. Trigger live-follow pushes by hand from the devtools
// console via `window.fretwireMock` (see the bottom of this file).

const LATENCY_MS = 30; // a touch of fake IPC latency, to surface races the real backend would

// ---------------------------------------------------------------------------------------------
// Parameter factories — build ParamDto-shaped objects. `value_type`: 0 = enum, 1 = float, 2 = bool
// (matches `.models` valueType). `index` is the wire selector (unique within a block).
// ---------------------------------------------------------------------------------------------
const P = {
  float: (index, name, value, min = 0, max = 10) => ({
    index, name, value, kind: "float", min, max, value_type: 1, display_type: null, enum_labels: [], stops: [],
  }),
  int: (index, name, value, min = 0, max = 127) => ({
    index, name, value, kind: "int", min, max, value_type: null, display_type: null, enum_labels: [], stops: [],
  }),
  bool: (index, name, on) => ({
    index, name, value: on ? 1 : 0, kind: "bool", min: 0, max: 1, value_type: 2, display_type: null, enum_labels: [], stops: [],
  }),
  enum: (index, name, value, labels) => ({
    index, name, value, kind: "int", min: 0, max: labels.length - 1, value_type: 0, display_type: null, enum_labels: labels, stops: [],
  }),
  // Segmented float (cab mic Angle): a float on the wire, but rendered as discrete stop buttons.
  seg: (index, name, value, stops) => ({
    index, name, value, kind: "float", min: stops[0].value, max: stops[stops.length - 1].value,
    value_type: 1, display_type: null, enum_labels: [], stops,
  }),
};

// Per-category parameter templates. Each returns a *fresh* array so swaps/adds get independent state.
const PARAMS = {
  amp: () => [
    P.float(0, "Drive", 5.5), P.float(1, "Bass", 5), P.float(2, "Mid", 6), P.float(3, "Treble", 5.5),
    P.float(4, "Presence", 4), P.float(5, "Ch Vol", 5), P.bool(6, "Bright", false), P.float(7, "Master", 7),
  ],
  cab: () => [
    P.enum(0, "Mic", 2, ["57 Dynamic", "121 Dynamic", "4038 Ribbon", "67 Condenser", "87 Condenser"]),
    P.float(1, "Distance", 4, 1, 12),
    P.seg(2, "Angle", 0, [{ value: 0, label: "0 deg" }, { value: 45, label: "45 deg" }]),
    P.float(3, "Low Cut", 80, 20, 500),
    P.float(4, "High Cut", 8000, 2000, 12000), P.float(5, "Level", 0, -30, 12),
  ],
  reverb: () => [
    P.enum(0, "Type", 0, ["Room", "Hall", "Plate", "Spring", "Chamber"]),
    P.float(1, "Decay", 4), P.float(2, "Predelay", 20, 0, 500), P.float(3, "Mix", 35, 0, 100),
    P.float(4, "Low Cut", 120, 20, 1000), P.float(5, "High Cut", 8000, 1000, 20000), P.bool(6, "Trails", true),
  ],
  dynamics: () => [P.float(0, "Threshold", -48, -96, 0), P.float(1, "Decay", 30, 0, 100), P.float(2, "Level", 0, -12, 12)],
  distortion: () => [P.float(0, "Drive", 5), P.float(1, "Tone", 5), P.float(2, "Level", 5)],
  delay: () => [P.float(0, "Time", 380, 1, 2000), P.float(1, "Feedback", 30, 0, 100), P.float(2, "Mix", 25, 0, 100), P.bool(3, "Trails", true)],
  modulation: () => [P.float(0, "Speed", 3), P.float(1, "Depth", 5), P.float(2, "Mix", 50, 0, 100)],
  eq: () => [P.float(0, "Low", 0, -12, 12), P.float(1, "Mid", 0, -12, 12), P.float(2, "High", 0, -12, 12)],
  wah: () => [P.float(0, "Position", 5), P.float(1, "Mix", 100, 0, 100)],
  pitch: () => [P.float(0, "Position", 5), P.float(1, "Mix", 100, 0, 100)],
};

// ---------------------------------------------------------------------------------------------
// Model catalog — stands in for the shipped `Helix.sym` list. `index` is the stable wire id passed
// to swap_model / add_block. Categories mirror the real device's grouping.
// ---------------------------------------------------------------------------------------------
// Category ids match the real device catalog (fretwire-core editor::category_name).
const CATEGORY_NAMES = {
  1: "Amp", 2: "Cab", 3: "Distortion", 4: "Dynamics", 7: "Pitch/Synth",
  8: "Modulation", 9: "Delay", 10: "Reverb", 11: "Wah", 14: "EQ", 17: "Volume/Pan",
  100: "Amp+Cab", // synthetic: amps pre-paired with their suggested cab (editor::CATEGORY_AMP_CAB)
};

let _mi = 1000;
const model = (symbolic_id, name, category, pf, dsp_load) => ({
  index: _mi++, symbolic_id, name, category, variant: null, dsp_load, makeParams: pf,
});

const CATALOG = [
  // Dynamics
  model("gate", "Noise Gate", 4, PARAMS.dynamics, 2.0),
  model("comp_la", "LA Studio Comp", 4, PARAMS.dynamics, 6.5),
  model("comp_deluxe", "Deluxe Comp", 4, PARAMS.dynamics, 5.0),
  model("boost_kinky", "Kinky Boost", 4, PARAMS.distortion, 3.0),
  // Distortion
  model("drive_minotaur", "Minotaur", 3, PARAMS.distortion, 4.5),
  model("drive_teemah", "Teemah!", 3, PARAMS.distortion, 4.5),
  model("drive_deranged", "Deranged Master", 3, PARAMS.distortion, 5.5),
  // Amp
  model("amp_princess", "US Princess", 1, PARAMS.amp, 30.0),
  model("amp_cali", "Cali Texas Ch1", 1, PARAMS.amp, 35.0),
  model("amp_brit", "Brit 2204", 1, PARAMS.amp, 36.5),
  model("amp_placater", "Placater Dirty", 1, PARAMS.amp, 33.0),
  model("amp_jazz", "Jazz Rivet 120", 1, PARAMS.amp, 34.0),
  // Cab
  model("cab_112", "1x12 Field Coil", 2, PARAMS.cab, 4.0),
  model("cab_212", "2x12 Double C12N", 2, PARAMS.cab, 4.5),
  model("cab_412", "4x12 Greenback 25", 2, PARAMS.cab, 5.0),
  // Reverb
  model("reverb_glitz", "Glitz", 10, PARAMS.reverb, 18.0),
  model("reverb_ganymede", "Ganymede", 10, PARAMS.reverb, 22.0),
  model("reverb_plateaux", "Plateaux", 10, PARAMS.reverb, 24.0),
  model("reverb_hall", "Hall", 10, PARAMS.reverb, 16.0),
  // Delay
  model("delay_simple", "Simple Delay", 9, PARAMS.delay, 9.0),
  model("delay_digital", "Vintage Digital", 9, PARAMS.delay, 12.0),
  model("delay_tape", "Transistor Tape", 9, PARAMS.delay, 13.5),
  // Modulation
  model("mod_trem", "Optical Trem", 8, PARAMS.modulation, 6.0),
  model("mod_chorus", "70s Chorus", 8, PARAMS.modulation, 7.5),
  model("mod_phaser", "Script Mod Phase", 8, PARAMS.modulation, 7.0),
  // EQ
  model("eq_simple", "Simple EQ", 14, PARAMS.eq, 3.0),
  model("eq_param", "Parametric", 14, PARAMS.eq, 4.5),
  model("eq_graphic", "10-Band Graphic", 14, PARAMS.eq, 6.0),
  // Wah / Volume
  model("vol_pedal", "Volume Pedal", 17, PARAMS.wah, 1.5),
  model("wah_teardrop", "Teardrop 310", 11, PARAMS.wah, 5.0),
  // Pitch / Synth
  model("pitch_wham", "Pitch Wham", 7, PARAMS.pitch, 11.0),
  model("pitch_capo", "Poly Capo", 7, PARAMS.pitch, 9.0),
];

const findModel = (symOrIndex) =>
  CATALOG.find((m) => m.symbolic_id === symOrIndex || m.index === symOrIndex);

// Amp → suggested-cab links (the real backend reads these from amp.models' `ircablink`), for the
// synthetic Amp+Cab category.
const CAB_LINKS = {
  amp_princess: "cab_112", amp_cali: "cab_212", amp_brit: "cab_212",
  amp_placater: "cab_412", amp_jazz: "cab_112",
};

// Split types — for the split node's split-type dropdown (`set_split_type`).
const SPLIT_TYPES = [
  { index: 257, symbolic_id: "SplitY", label: "Split Y" },
  { index: 258, symbolic_id: "SplitAB", label: "Split A/B" },
  { index: 259, symbolic_id: "SplitCrossover", label: "Split Crossover" },
];

// ---------------------------------------------------------------------------------------------
// Block / node factories — the mutable internal representation of a slot's contents.
// ---------------------------------------------------------------------------------------------
function makeBlock(sym, opts = {}) {
  const md = findModel(sym);
  const b = {
    kind: "effect",
    modelIndex: md.index,
    symbolic_id: md.symbolic_id,
    model_name: md.name,
    user_label: opts.label ?? null,
    category: md.category,
    variant: null,
    bypassed: opts.bypassed ?? false,
    footswitch: opts.footswitch ?? 0,
    dsp_load: md.dsp_load,
    params: md.makeParams(),
    paired_model_name: null,
    paired_index: null,
    paired_params: [],
  };
  if (opts.cab) {
    const cab = findModel(opts.cab);
    b.paired_model_name = cab.name;
    b.paired_index = cab.index;
    b.paired_params = cab.makeParams();
    b.dsp_load += cab.dsp_load;
  }
  return b;
}

// Fixed input/output nodes (slots 0 and 9) — same param shapes as the real backend resolves from
// io.models (`HelixStomp_AppDSPFlowInput` / `…OutputMain`).
const inputNode = () => ({
  kind: "input", modelIndex: null, symbolic_id: "HelixStomp_AppDSPFlowInput", model_name: "Input",
  dsp_load: 0,
  params: [P.bool(0, "Input Gate", false), P.float(1, "Threshold", -48, -96, 0), P.float(2, "Decay", 0.5, 0.01, 1)],
});
const outputNode = () => ({
  kind: "output", modelIndex: null, symbolic_id: "HelixStomp_AppDSPFlowOutputMain", model_name: "Output",
  dsp_load: 0,
  params: [P.float(0, "Pan", 0.5, 0, 1), P.float(1, "Level", 0, -120, 20)],
});

const splitNode = () => ({
  kind: "split", modelIndex: 257, symbolic_id: "SplitY", model_name: "Split Y", dsp_load: 0,
  params: [P.float(0, "Balance", 0, -100, 100), P.bool(1, "Level Match", true)],
});
const mixerNode = () => ({
  kind: "mixer", modelIndex: 151, symbolic_id: "Mixer", model_name: "Mixer", dsp_load: 0,
  params: [
    P.float(0, "A Volume", 0, -60, 12), P.float(1, "A Pan", -100, -100, 100),
    P.float(2, "B Volume", 0, -60, 12), P.float(3, "B Pan", 100, -100, 100),
  ],
});

// ---------------------------------------------------------------------------------------------
// Preset factories. `slots` is keyed by slot index in the device's **fixed 20-slot topology**:
// [0=input, 1..8 top row, 9=node, 10=split node, 11..18 row B, 19=mixer node]. The split/mixer node
// slots exist even on serial presets [solid — preset1 fixture]; whether the preset *is* split is
// derived from row-B occupancy, exactly like the device: dropping a block into an empty B slot
// creates the split, moving the last B block out retires it.
// ---------------------------------------------------------------------------------------------
// A DSP occupies 20 wire slots (stride 20), addressed globally as `dsp * 20 + local`. Within a DSP
// the local layout is [0=input, 1..8 top row, 9=output, 10=split node, 11..18 row B, 19=mixer node]
// — the Floor's real per-DSP topology (Pull Me Under's DSP2 sits at 27/28 on the top row and 33..38
// on row B, i.e. base 20 + those same local indices). A single-DSP HX Stomp preset uses only base 0.
const STRIDE = 20;
const SPLIT_LOCAL = 10, MIXER_LOCAL = 19;
const dspOf = (slot) => Math.floor(slot / STRIDE);
const baseOf = (slot) => dspOf(slot) * STRIDE;
const localOf = (slot) => slot - baseOf(slot);
const topSlots = (base) => [1, 2, 3, 4, 5, 6, 7, 8].map((i) => base + i);
const bSlots = (base) => [11, 12, 13, 14, 15, 16, 17, 18].map((i) => base + i);
const inputSlot = (base) => base + 0;
const outputSlot = (base) => base + 9;
const splitSlot = (base) => base + SPLIT_LOCAL;
const mixerSlot = (base) => base + MIXER_LOCAL;
// Display column within the DSP: top slot base+n → col n; row-B slot base+10+n → col n+1.
const topCol = (slot) => localOf(slot);
const bCol = (slot) => localOf(slot) - SPLIT_LOCAL + 1;
const isRowB = (slot) => localOf(slot) >= 11 && localOf(slot) <= 18;
const rowSlots = (slot) => (isRowB(slot) ? bSlots(baseOf(slot)) : topSlots(baseOf(slot)));
// The bases of every DSP this preset carries (one on the Stomp, two on the Floor).
const dspBases = (p) => Array.from({ length: p.dspCount }, (_, k) => k * STRIDE);
// Slots a block can occupy in one DSP / across the whole preset.
const editSlots = (base) => [...topSlots(base), ...bSlots(base)];
const allEditSlots = (p) => dspBases(p).flatMap(editSlots);

const isSplit = (p, base) => bSlots(base).some((i) => p.slots[i]?.kind === "effect");

// The split/mixer signal columns (node key 13) for one DSP. User-set positions (set_node_pos, like
// dragging the nodes in the UI) are stored per-DSP on the preset and honored; otherwise they derive
// from where the B blocks sit — and either way they're clamped so the bracket keeps enclosing the
// occupied B row, approximating the device's recompute when blocks move.
function splitMixerPos(p, base) {
  const cols = bSlots(base).filter((i) => p.slots[i]?.kind === "effect").map(bCol);
  if (!cols.length) return { splitPos: null, mixerPos: null };
  const minB = Math.min(...cols), maxB = Math.max(...cols);
  const np = p.nodePos[dspOf(base)] ?? {};
  const splitPos = Math.min(np.split ?? minB, minB);
  const mixerPos = Math.max(np.mixer ?? Math.min(maxB + 2, 10), maxB + 1, splitPos + 1);
  return { splitPos, mixerPos };
}

function makePreset(name, index, slots, opts = {}) {
  const dspCount = opts.dspCount ?? 1;
  for (const base of Array.from({ length: dspCount }, (_, k) => k * STRIDE)) {
    slots[inputSlot(base)] = inputNode();
    slots[outputSlot(base)] = outputNode();
    slots[splitSlot(base)] = splitNode();
    slots[mixerSlot(base)] = mixerNode();
  }
  return {
    name, index, active_snapshot: 0, snapshot_names: opts.snapshot_names ?? [], slots,
    nodePos: {}, dspCount,
    deviceModel: opts.deviceModel ?? "HX Stomp", firmware: opts.firmware ?? "3.80 (mock)",
  };
}

function serialPreset(name, index, defs, snapshot_names = []) {
  const slots = {};
  defs.forEach((d, i) => {
    slots[i + 1] = makeBlock(d.sym, d);
  });
  return makePreset(name, index, slots, { snapshot_names });
}

function dualAmpPreset() {
  const slots = {};
  slots[1] = makeBlock("gate", { label: "Gate" });
  slots[3] = makeBlock("amp_princess", { cab: "cab_112", label: "Amp A" }); // path A (col 3)
  slots[5] = makeBlock("reverb_glitz"); // common-after (col ≥ mixer pos)
  slots[12] = makeBlock("amp_placater", { cab: "cab_412", label: "Amp B" }); // row B (col 3)
  return makePreset("Dual Amp", 1, slots, { snapshot_names: ["Verse", "Chorus", "Solo"] });
}

// A two-DSP Helix Floor preset, so the mock exercises the same dual-grid path a real Floor drives —
// modeled on "Pull Me Under": DSP1 an amp path that splits to a parallel drive, DSP2 a second amp
// that splits into a wide reverb/delay wash. Blocks sit at the same global slots the Floor reports.
function floorPreset() {
  const slots = {};
  // DSP1 (base 0): top row 1..5, split to row B 11..12.
  slots[1] = makeBlock("vol_pedal", { label: "Volume" });
  slots[2] = makeBlock("drive_minotaur");
  slots[3] = makeBlock("amp_jazz", { cab: "cab_212", label: "Amp A" });
  slots[4] = makeBlock("mod_chorus");
  slots[5] = makeBlock("eq_param");
  slots[11] = makeBlock("drive_teemah", { label: "Weeper" });
  slots[12] = makeBlock("boost_kinky", { label: "Scream 808" });
  // DSP2 (base 20): top row 27/28, split to a parallel wash on row B 33..37 (38 left free to demo
  // dropping/adding a block). Kept under the per-DSP ~100% budget so nothing renders impossible.
  slots[27] = makeBlock("amp_placater", { cab: "cab_412", label: "Amp B" });
  slots[28] = makeBlock("comp_deluxe");
  slots[33] = makeBlock("mod_chorus");
  slots[34] = makeBlock("delay_simple");
  slots[35] = makeBlock("mod_phaser");
  slots[36] = makeBlock("eq_graphic");
  slots[37] = makeBlock("reverb_hall");
  return makePreset("Pull Me Under", 0, slots, {
    dspCount: 2,
    deviceModel: "P21",
    firmware: "7d01f5e (mock)",
    snapshot_names: ["Intro", "B&C", "Solo", "", "", "", "", ""],
  });
}

// The setlist. Index 0 is the two-DSP Floor preset (shows both routing grids on connect).
const presets = [
  floorPreset(),
  dualAmpPreset(),
  serialPreset("Crunch Lead", 1, [
    { sym: "gate" }, { sym: "drive_minotaur" },
    { sym: "amp_cali", cab: "cab_212", label: "Amp" }, { sym: "delay_simple" }, { sym: "reverb_glitz" },
  ]),
  serialPreset("Ambient Clean", 2, [
    { sym: "comp_la" }, { sym: "mod_chorus" },
    { sym: "amp_jazz", cab: "cab_112", label: "Amp" }, { sym: "delay_digital" }, { sym: "reverb_plateaux" },
  ], ["Intro", "Build", "Peak"]),
  serialPreset("Metal Rhythm", 3, [
    { sym: "gate", label: "Gate" }, { sym: "drive_teemah" },
    { sym: "amp_placater", cab: "cab_412", label: "Amp" }, { sym: "eq_param" }, { sym: "delay_simple", bypassed: true },
  ]),
  serialPreset("Wah Funk", 4, [{ sym: "wah_teardrop" }, { sym: "comp_deluxe" }, { sym: "amp_jazz", cab: "cab_112" }]),
  serialPreset("Clean DI", 5, [{ sym: "comp_la" }, { sym: "eq_graphic" }]),
  serialPreset("Lead Boost", 6, [{ sym: "boost_kinky" }, { sym: "amp_brit", cab: "cab_212" }, { sym: "reverb_hall" }]),
  serialPreset("Init Tone", 7, [{ sym: "amp_cali", cab: "cab_412" }]),
];

// ---------------------------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------------------------
let connected = false;
let current = presets[0];
// Backup/restore round-trip (see the backup_setlist handler): the last backup made this session.
let lastBackup = null;

const hexEncode = (str) =>
  Array.from(new TextEncoder().encode(str), (b) => b.toString(16).padStart(2, "0")).join("");
const hexDecode = (hex) =>
  new TextDecoder().decode(new Uint8Array(hex.match(/.{2}/g)?.map((h) => parseInt(h, 16)) ?? []));


// ---------------------------------------------------------------------------------------------
// Edit history — mirrors the real backend: a labeled timeline of full-state snapshots with a
// cursor. Entry 0 = the loaded state; each edit appends the post-edit state (truncating any redo
// branch first); undo/redo/jump just move the cursor and restore. Cleared on preset switch. Which
// commands are history-tracked is decided at the dispatch layer, same as the Rust command layer.
// ---------------------------------------------------------------------------------------------
const HISTORY_MAX = 50;
let history = []; // [{ label, state }]
let historyCursor = 0;
// History cursor whose state matches "flash" — mirrors Session::saved_cursor. -1 = the saved
// state fell off the timeline (truncated redo branch / cap), so nothing matches until a save.
let savedCursor = 0;

const snapshot = () => ({
  slots: clone(current.slots), nodePos: { ...current.nodePos }, name: current.name,
  snapshot_names: [...current.snapshot_names],
});
function restore(snap) {
  const s = clone(snap); // entries are canonical — never hand out a mutable reference
  current.slots = s.slots;
  current.nodePos = s.nodePos;
  current.name = s.name;
  current.snapshot_names = s.snapshot_names;
}
function editBegin() {
  if (!history.length) {
    history = [{ label: "Loaded", state: snapshot() }];
    historyCursor = 0;
  }
  history.length = historyCursor + 1; // truncate the redo branch
  if (savedCursor > historyCursor) savedCursor = -1; // saved state was in the discarded branch
}
function editCommit(label) {
  history.push({ label, state: snapshot() });
  if (history.length > HISTORY_MAX) {
    history.shift();
    savedCursor = savedCursor > 0 ? savedCursor - 1 : -1;
  }
  historyCursor = history.length - 1;
}
function historyJump(index) {
  if (index < 0 || index >= history.length) throw new Error(`no history entry ${index} (have ${history.length})`);
  if (index !== historyCursor) {
    restore(history[index].state);
    historyCursor = index;
  }
  return toDto(current);
}
function clearHistory() {
  history = [];
  historyCursor = 0;
  savedCursor = 0; // fresh context = just loaded from "flash" = clean
}
// Seed entry 0 ("Loaded") on read, like the real backend — so the pane exists before any edit.
function seedHistory() {
  if (!history.length) {
    history = [{ label: "Loaded", state: snapshot() }];
    historyCursor = 0;
  }
}

// Name helpers for history labels — resolved from the pre-edit state, like the real backend
// (`Session::slot_label` / `param_label` / `model_label`).
function slotName(slot) {
  const e = current.slots[slot];
  if (e?.kind === "effect") return e.user_label ?? e.model_name;
  if (e?.kind === "input" || e?.kind === "output") return e.model_name;
  return `slot ${slot}`;
}
function paramName(slot, paired, index) {
  const e = current.slots[slot];
  const list = paired ? e?.paired_params : e?.params;
  const n = list?.find((q) => q.index === index)?.name;
  return n ? `${n} — ${slotName(slot)}` : slotName(slot);
}
const modelName = (index) => findModel(index)?.name ?? "block";

// Commands that land on the history timeline, with their entry labels (same as commands.rs).
const EDIT_LABELS = {
  set_bypass: (a) => `${a.bypassed ? "Bypass" : "Enable"} ${slotName(a.slot)}`,
  set_param: (a) => `Set ${paramName(a.slot, false, a.paramIndex)}`,
  set_paired_param: (a) => `Set ${paramName(a.slot, true, a.paramIndex)}`,
  set_param_enum: (a) => `Set ${paramName(a.slot, a.paired, a.paramIndex)}`,
  swap_model: (a) =>
    `${slotName(a.slot)} → ${modelName(a.modelIndex)}` +
    (a.pairedIndex >= 0 ? ` + ${modelName(a.pairedIndex)}` : ""),
  rename_snapshot: (a) => `Rename snapshot ${a.index + 1}`,
  add_block: (a) => `Add ${modelName(a.modelIndex)}`,
  add_block_at: (a) => `Add ${modelName(a.modelIndex)}`,
  delete_block: (a) => `Delete ${slotName(a.slot)}`,
  place_block: (a) => `Move ${slotName(a.srcSlot)}`,
  insert_block: (a) => `Move ${slotName(a.srcSlot)} ${a.before ? "before" : "after"} ${slotName(a.dstSlot)}`,
  reorder_block: (a) => `Move ${slotName(a.srcSlot)}`,
  move_block_to_row: (a) => `Move ${slotName(a.srcSlot)}`,
  move_before_split: (a) => `Move ${slotName(a.srcSlot)}`,
  set_node_pos: (a) => `Move ${a.node} node → col ${a.pos}`,
  set_split_type: (a) => `Split type → ${SPLIT_TYPES.find((t) => t.index === a.modelIndex)?.label ?? "?"}`,
};

// ---------------------------------------------------------------------------------------------
// DTO projection — mirrors `From<&EditorPreset> for PresetDto` etc. in src/dto.rs.
// ---------------------------------------------------------------------------------------------
const clone = (x) => structuredClone(x);

function blockDto(p, slot, e) {
  const cab = e.paired_index != null ? findModel(e.paired_index) : null;
  return {
    slot, dsp: dspOf(slot), row: isRowB(slot) ? 1 : 0,
    model_name: e.model_name, user_label: e.user_label, symbolic_id: e.symbolic_id,
    model_index: e.modelIndex,
    category: e.category, bypassed: e.bypassed, variant: e.variant, is_controller: false,
    footswitch: e.footswitch, dsp_load: e.dsp_load, params: clone(e.params),
    paired_model_name: e.paired_model_name, paired_index: e.paired_index,
    paired_symbolic_id: cab?.symbolic_id ?? null, paired_category: cab?.category ?? null,
    paired_params: clone(e.paired_params),
  };
}

function nodeDto(p, slot, e) {
  return {
    slot, dsp: dspOf(slot), row: 0, model_name: e.model_name, user_label: null, symbolic_id: e.symbolic_id,
    category: null, bypassed: null, variant: null, is_controller: false, footswitch: 0,
    dsp_load: e.dsp_load ?? 0, params: clone(e.params), paired_model_name: null, paired_index: null, paired_params: [],
  };
}

// The real `PresetStream::grid()` emits the empty row-B cells **even on serial presets** (the node
// slots always exist in the fixed array) — that's what lets a drop into B create the split.
function toGrid(p, base) {
  const cells = [];
  for (const i of topSlots(base)) {
    cells.push({ dsp: dspOf(base), slot: i, row: 0, column: topCol(i), occupied: p.slots[i]?.kind === "effect" });
  }
  for (const i of bSlots(base)) {
    cells.push({ dsp: dspOf(base), slot: i, row: 1, column: bCol(i), occupied: p.slots[i]?.kind === "effect" });
  }
  return cells;
}

// One DSP's routing view — mirrors the backend's DspDto.
function dspDto(p, base) {
  const split = isSplit(p, base);
  const { splitPos, mixerPos } = splitMixerPos(p, base);
  const load = editSlots(base)
    .filter((i) => p.slots[i]?.kind === "effect")
    .reduce((s, i) => s + (p.slots[i].dsp_load ?? 0), 0);
  return {
    dsp: dspOf(base),
    split,
    split_pos: splitPos,
    mixer_pos: mixerPos,
    // Like EditorPreset: the node slots exist regardless, but are surfaced only when split.
    split_node: split ? nodeDto(p, splitSlot(base), p.slots[splitSlot(base)]) : null,
    mixer_node: split ? nodeDto(p, mixerSlot(base), p.slots[mixerSlot(base)]) : null,
    input_node: nodeDto(p, inputSlot(base), p.slots[inputSlot(base)]),
    output_node: nodeDto(p, outputSlot(base), p.slots[outputSlot(base)]),
    grid: toGrid(p, base),
    dsp_load: load,
  };
}

function toDto(p) {
  const dsps = dspBases(p).map((base) => dspDto(p, base));
  const d0 = dsps[0];
  const occupied = allEditSlots(p).filter((i) => p.slots[i]?.kind === "effect");
  return {
    name: p.name, index: p.index, bank: 0, device_model: p.deviceModel, firmware: p.firmware,
    // Flat fields mirror DSP 0, exactly like the real PresetDto, so a single-DSP UI still works.
    split: d0.split, dsp_load: dsps.reduce((s, v) => s + v.dsp_load, 0),
    split_pos: d0.split_pos, mixer_pos: d0.mixer_pos,
    active_snapshot: p.active_snapshot, snapshot_names: p.snapshot_names,
    blocks: occupied.map((i) => blockDto(p, i, p.slots[i])),
    split_node: d0.split_node, mixer_node: d0.mixer_node,
    input_node: d0.input_node, output_node: d0.output_node,
    grid: d0.grid,
    dsps,
    undo_depth: historyCursor,
    redo_depth: Math.max(0, history.length - historyCursor - 1),
    history: history.map((e) => e.label),
    history_cursor: historyCursor,
    dirty: historyCursor !== savedCursor,
  };
}

// Find a mutable param on the current preset by (slot, paired, index).
function findParam(slot, paired, index) {
  const e = current.slots[slot];
  if (!e) return null;
  const list = paired ? e.paired_params : e.params;
  return list?.find((x) => x.index === index) ?? null;
}

// ---------------------------------------------------------------------------------------------
// Event bus (Tauri `listen`/emit). App.svelte listens for "device-pushes".
// ---------------------------------------------------------------------------------------------
const listeners = new Map();
function emit(event, payload) {
  for (const h of listeners.get(event) ?? []) h({ event, payload, id: 0 });
}
export function listen(event, handler) {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event).add(handler);
  return Promise.resolve(() => listeners.get(event)?.delete(handler));
}

// ---------------------------------------------------------------------------------------------
// Command dispatch — one arm per `#[command]` in commands.rs.
// ---------------------------------------------------------------------------------------------
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// First-run reference data. The mock ships its own catalog, so data is "present" by default;
// flip it with `fretwireMock.needsData()` to exercise the import screen.
let dataPresent = true;

const HANDLERS = {
  data_status: () => ({
    present: dataPresent,
    dir: "~/.local/share/fretwire/data",
    files: dataPresent ? 21 : 0,
  }),
  import_data: async ({ source }) => {
    await sleep(600); // the real one unpacks an installer — show the spinner
    if (!source || source === "/") throw new Error(`no reference data found in ${source}`);
    dataPresent = true;
    return { copied: 21, dest: "~/.local/share/fretwire/data", missing: [] };
  },
  detect: () => true,
  is_connected: () => connected,
  connect: () => {
    connected = true;
    seedHistory();
    return toDto(current);
  },
  disconnect: () => {
    connected = false;
    return null;
  },
  read_preset: () => {
    seedHistory();
    return toDto(current);
  },

  set_bypass: ({ slot, bypassed }) => {
    const e = current.slots[slot];
    if (e && e.kind === "effect") e.bypassed = bypassed;
    return toDto(current);
  },
  set_param: ({ slot, paramIndex, value }) => {
    const p = findParam(slot, false, paramIndex);
    if (p) p.value = value;
    return toDto(current);
  },
  set_paired_param: ({ slot, paramIndex, value }) => {
    const p = findParam(slot, true, paramIndex);
    if (p) p.value = value;
    return toDto(current);
  },
  // Mid-drag previews: value only, no history entry, no DTO (mirrors the fire-and-forget commands).
  preview_param: ({ slot, paramIndex, value }) => {
    const p = findParam(slot, false, paramIndex);
    if (p) p.value = value;
    return null;
  },
  preview_paired_param: ({ slot, paramIndex, value }) => {
    const p = findParam(slot, true, paramIndex);
    if (p) p.value = value;
    return null;
  },
  set_param_enum: ({ slot, paired, paramIndex, value }) => {
    const p = findParam(slot, paired, paramIndex);
    if (p) p.value = value;
    return toDto(current);
  },
  swap_model: ({ slot, modelIndex, pairedIndex }) => {
    const md = findModel(modelIndex);
    const e = current.slots[slot];
    if (md && e && e.kind === "effect") {
      // Re-sending the same model (the change-cab flow) keeps the block's knob values; a real swap
      // resets to defaults. LIVE: the device's behavior for the same-model case is unverified.
      const sameModel = e.modelIndex === md.index;
      const sameCab = e.paired_index != null && e.paired_index === pairedIndex;
      e.modelIndex = md.index;
      e.symbolic_id = md.symbolic_id;
      e.model_name = md.name;
      e.category = md.category;
      if (!sameModel) {
        e.user_label = null;
        e.params = md.makeParams();
      }
      e.dsp_load = md.dsp_load;
      const cab = pairedIndex >= 0 ? findModel(pairedIndex) : null;
      if (cab) {
        e.paired_model_name = cab.name;
        e.paired_index = cab.index;
        if (!sameCab) e.paired_params = cab.makeParams();
        e.dsp_load += cab.dsp_load;
      } else {
        e.paired_model_name = null;
        e.paired_index = null;
        e.paired_params = [];
      }
    }
    return toDto(current);
  },
  add_block: ({ modelIndex, pairedIndex }) => {
    // Like Session::add_block_append: end of row A first, then any free A slot, then row B.
    const md = findModel(modelIndex);
    const cab = pairedIndex >= 0 ? findModel(pairedIndex) : null;
    // Append into the first DSP with room (row A first, then row B) — DSP 0 preferred.
    let free = null;
    for (const base of dspBases(current)) {
      const tops = topSlots(base);
      const lastTop = Math.max(base, ...tops.filter((i) => current.slots[i]?.kind === "effect"));
      free =
        tops.find((i) => i > lastTop && !current.slots[i]) ??
        tops.find((i) => !current.slots[i]) ??
        bSlots(base).find((i) => !current.slots[i]);
      if (free != null) break;
    }
    if (md && free != null) current.slots[free] = makeBlock(md.symbolic_id, { cab: cab?.symbolic_id });
    return toDto(current);
  },
  add_block_at: ({ slot, modelIndex, pairedIndex }) => {
    const md = findModel(modelIndex);
    const cab = pairedIndex >= 0 ? findModel(pairedIndex) : null;
    if (current.slots[slot]) throw new Error(`slot ${slot} is not an empty grid slot (refusing add — it would overwrite)`);
    if (md && editSlots(baseOf(slot)).includes(slot)) current.slots[slot] = makeBlock(md.symbolic_id, { cab: cab?.symbolic_id });
    return toDto(current);
  },
  delete_block: ({ slot }) => {
    if (current.slots[slot]?.kind === "effect") delete current.slots[slot];
    return toDto(current);
  },
  place_block: ({ srcSlot, dstSlot }) => {
    if (srcSlot === dstSlot) return toDto(current);
    const src = current.slots[srcSlot];
    // op-43 semantics: a move only into an empty slot (the guarded case the UI offers).
    if (src?.kind === "effect" && !current.slots[dstSlot]) {
      current.slots[dstSlot] = src;
      delete current.slots[srcSlot];
    }
    return toDto(current);
  },
  // Drop onto an occupied cell: insert src before/after it, shifting neighbors — mirrors
  // Session::insert_block (same-row = reorder among the row's existing occupied slots; cross-row =
  // shift the suffix right into free slots, like plan_row_insert).
  insert_block: ({ srcSlot, dstSlot, before }) => {
    if (srcSlot === dstSlot) return toDto(current);
    // The routing planner is per-DSP; the UI can't drag across DSPs (each grid has its own drag
    // state), but guard anyway so a bad call is a clear error, not a corrupt move.
    if (baseOf(srcSlot) !== baseOf(dstSlot)) throw new Error("cross-DSP move is not supported");
    const row = rowSlots(dstSlot);
    if (current.slots[srcSlot]?.kind !== "effect" || current.slots[dstSlot]?.kind !== "effect")
      throw new Error("both slots must hold a block");
    const occ = row.filter((i) => current.slots[i]?.kind === "effect" && i !== srcSlot);
    const pos = occ.indexOf(dstSlot) + (before ? 0 : 1);
    if (isRowB(srcSlot) === isRowB(dstSlot)) {
      // Same row: reorder the blocks among the row's occupied slots (slot set unchanged).
      if (!editSlots(baseOf(srcSlot)).some((i) => !current.slots[i])) throw new Error("no empty slot to reorder through");
      const slots = row.filter((i) => current.slots[i]?.kind === "effect");
      const order = slots.map((i) => current.slots[i]);
      const [moved] = order.splice(slots.indexOf(srcSlot), 1);
      // `pos` is indexed over the src-excluded list, i.e. exactly the insertion index here.
      order.splice(pos, 0, moved);
      slots.forEach((s, i) => (current.slots[s] = order[i]));
    } else {
      // Cross-row: shift occ[pos..] one step right into free slots, then src takes the freed slot.
      const free = row.filter((i) => !current.slots[i]);
      let target;
      if (pos >= occ.length) {
        const last = occ[occ.length - 1];
        target = free.find((s) => last == null || s > last) ?? free[0];
        if (target == null) throw new Error("no free slot in the destination row");
      } else {
        const freeSet = new Set(free);
        for (let i = occ.length - 1; i >= pos; i--) {
          const dest = [...freeSet].filter((s) => s > occ[i]).sort((a, b) => a - b)[0];
          if (dest == null) throw new Error("no free slot in the destination row");
          current.slots[dest] = current.slots[occ[i]];
          delete current.slots[occ[i]];
          freeSet.delete(dest);
          freeSet.add(occ[i]);
        }
        target = occ[pos];
      }
      current.slots[target] = current.slots[srcSlot];
      delete current.slots[srcSlot];
    }
    return toDto(current);
  },
  // Legacy structural commands still registered by the Rust side; the current UI drives place_block.
  reorder_block: (a) => HANDLERS.place_block(a),
  move_block_to_row: () => toDto(current),
  move_before_split: () => toDto(current),

  set_node_pos: ({ node, pos, dsp = 0 }) => {
    const base = dsp * STRIDE;
    if (!isSplit(current, base)) throw new Error("preset is not split");
    const cols = bSlots(base).filter((i) => current.slots[i]?.kind === "effect").map(bCol);
    const { splitPos, mixerPos } = splitMixerPos(current, base);
    const np = (current.nodePos[dsp] ??= {});
    // Same guards as Session::set_node_pos: the bracket must keep enclosing the B row.
    if (node === "split") {
      const hi = Math.min(Math.min(...cols), mixerPos - 1);
      if (pos < 1 || pos > hi) throw new Error(`node position ${pos} out of range 1..=${hi} (bracket must enclose the B row)`);
      np.split = pos;
    } else if (node === "mixer") {
      const lo = Math.max(Math.max(...cols) + 1, splitPos + 1);
      if (pos < lo || pos > 16) throw new Error(`node position ${pos} out of range ${lo}..=16 (bracket must enclose the B row)`);
      np.mixer = pos;
    } else {
      throw new Error(`unknown node kind "${node}" (want "split" or "mixer")`);
    }
    return toDto(current);
  },
  set_split_type: ({ splitSlot: slot, modelIndex }) => {
    const base = slot != null ? baseOf(slot) : 0;
    const t = SPLIT_TYPES.find((s) => s.index === modelIndex);
    const node = isSplit(current, base) ? current.slots[splitSlot(base)] : null;
    if (t && node) {
      node.modelIndex = t.index;
      node.symbolic_id = t.symbolic_id;
      node.model_name = t.label;
    }
    return toDto(current);
  },
  set_snapshot: ({ index }) => {
    current.active_snapshot = index;
    return toDto(current);
  },
  goto_preset: ({ preset }) => {
    const target = presets.find((p) => p.index === preset);
    if (target) {
      current = target;
      clearHistory(); // the history belongs to the old preset
      seedHistory();
    }
    return toDto(current);
  },
  undo: () => {
    if (historyCursor === 0) throw new Error("nothing to undo");
    return historyJump(historyCursor - 1);
  },
  redo: () => {
    if (historyCursor + 1 >= history.length) throw new Error("nothing to redo");
    return historyJump(historyCursor + 1);
  },
  history_jump: ({ index }) => historyJump(index),
  save_preset: ({ slot, name }) => {
    savedCursor = historyCursor; // the buffer's current state is now "in flash"
    if (slot === current.index) {
      if (name) current.name = name;
    } else {
      // Save As to a different slot: deep-copy the current preset into that slot.
      const copy = {
        ...current, index: slot, name: name || current.name,
        slots: clone(current.slots), nodePos: { ...current.nodePos },
        snapshot_names: [...current.snapshot_names],
      };
      const at = presets.findIndex((p) => p.index === slot);
      if (at >= 0) presets[at] = copy;
      else presets.push(copy);
      presets.sort((a, b) => a.index - b.index);
    }
    return toDto(current);
  },
  rename_preset: ({ slot, name }) => {
    const target = presets.find((p) => p.index === slot);
    if (target && name) target.name = name;
    return null;
  },
  rename_snapshot: ({ index, name }) => {
    if (index >= 0 && index < current.snapshot_names.length) current.snapshot_names[index] = name;
    return toDto(current);
  },
  list_presets: () => presets.map((p) => ({ index: p.index, name: p.name })),

  // ---- backup / restore -----------------------------------------------------------------------
  // The real backend sweeps the device and writes a `fretwire-backup` JSON file at `path`. The mock
  // mirrors the file *shape* exactly (its "raw" is hex of the mock preset state instead of a
  // MessagePack stream), triggers a browser download of it, and keeps it in memory so
  // backup_show/restore_preset work in the same session — a browser can't read arbitrary paths.
  backup_setlist: async ({ path }) => {
    const entries = [];
    for (let i = 0; i < presets.length; i++) {
      const p = presets[i];
      await sleep(120); // the real sweep takes ~a second per preset; make the progress UI visible
      emit("backup-progress", { done: i + 1, total: presets.length, name: p.name });
      entries.push({
        index: p.index,
        name: p.name,
        raw_hex: hexEncode(JSON.stringify({
          name: p.name, snapshot_names: p.snapshot_names, active_snapshot: p.active_snapshot,
          slots: p.slots, nodePos: p.nodePos,
        })),
      });
    }
    lastBackup = { format: "fretwire-backup", version: 1, device: "HX Stomp (mock)", presets: entries };
    // Offer the file as a download, named like the requested path.
    const blob = new Blob([JSON.stringify(lastBackup, null, 2)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = path.split("/").pop() || "fretwire-backup.json";
    a.click();
    URL.revokeObjectURL(a.href);
    return entries.length;
  },
  backup_show: ({ path }) => {
    if (!lastBackup)
      throw new Error(`mock backend: no backup this session — run Backup… first (a browser can't read ${path})`);
    return lastBackup.presets.map((p) => ({ index: p.index, name: p.name }));
  },
  restore_preset: ({ index, slot }) => {
    if (!lastBackup) throw new Error("mock backend: no backup this session — run Backup… first");
    const entry = lastBackup.presets.find((p) => p.index === index);
    if (!entry) throw new Error(`backup has no preset at index ${index}`);
    const state = JSON.parse(hexDecode(entry.raw_hex));
    const restored = {
      name: state.name, index: slot, active_snapshot: state.active_snapshot ?? 0,
      snapshot_names: state.snapshot_names ?? [], slots: state.slots, nodePos: state.nodePos,
    };
    const at = presets.findIndex((p) => p.index === slot);
    if (at >= 0) presets[at] = restored;
    else {
      presets.push(restored);
      presets.sort((a, b) => a.index - b.index);
    }
    // Like Session::restore_preset: the device ends up on the restored slot, history reset.
    current = restored;
    clearHistory();
    seedHistory();
    return toDto(current);
  },
  split_types: () => clone(SPLIT_TYPES),
  categories: () => {
    const ids = [...new Set(CATALOG.map((m) => m.category))].sort((a, b) => a - b);
    ids.push(100); // synthetic Amp+Cab
    return ids
      .map((id) => ({ id, name: CATEGORY_NAMES[id] ?? `Cat ${id}` }))
      .sort((a, b) => a.name.localeCompare(b.name));
  },
  models_in_category: ({ category }) => {
    // Synthetic Amp+Cab: the amp list with its linked cab pre-paired and the combined DSP cost.
    if (category === 100) {
      return CATALOG.filter((m) => m.category === 1 && CAB_LINKS[m.symbolic_id]).map((m) => {
        const cab = findModel(CAB_LINKS[m.symbolic_id]);
        return {
          index: m.index, symbolic_id: m.symbolic_id, name: m.name, category: 100,
          variant: m.variant, dsp_load: m.dsp_load + cab.dsp_load, default_paired_index: cab.index,
        };
      });
    }
    return CATALOG.filter((m) => m.category === category).map((m) => ({
      index: m.index, symbolic_id: m.symbolic_id, name: m.name,
      category: m.category, variant: m.variant, dsp_load: m.dsp_load, default_paired_index: null,
    }));
  },
};

export async function invoke(cmd, args = {}) {
  await sleep(LATENCY_MS);
  const h = HANDLERS[cmd];
  if (!h) throw new Error(`mock backend: unknown command "${cmd}"`);
  const label = EDIT_LABELS[cmd]?.(args);
  if (label == null) return h(args);
  // History-tracked edit: bracket it, and rebuild the DTO after the commit so the returned
  // history/depths are fresh (mirrors mutate_edit/returning_edit in commands.rs).
  editBegin();
  h(args); // throws → no entry appended
  editCommit(label);
  return toDto(current);
}

// ---------------------------------------------------------------------------------------------
// Console helper — simulate device-originated pushes (footswitch bypass, panel snapshot/preset
// switch) to exercise live-follow. Try `fretwireMock.bypass(1, false)` in devtools with a preset open.
// ---------------------------------------------------------------------------------------------
if (typeof window !== "undefined") {
  window.fretwireMock = {
    /** Toggle a block's bypass as if from a footswitch. `enabled` = active (not bypassed). */
    bypass(slot, enabled) {
      const e = current.slots[slot];
      if (e && e.kind === "effect") e.bypassed = !enabled;
      emit("device-pushes", [{ kind: "Bypass", slot, enabled }]);
    },
    /** Switch snapshot as if from the panel. */
    snapshot(index) {
      current.active_snapshot = index;
      emit("device-pushes", [{ kind: "Snapshot", index }]);
    },
    /** Switch preset as if from the panel. */
    preset(index) {
      const target = presets.find((p) => p.index === index);
      if (target) {
        current = target;
        clearHistory(); // like the heartbeat clearing on a panel preset push
      }
      emit("device-pushes", [{ kind: "Preset", index }]);
    },
    /** Pretend the reference data was never imported, so the first-run screen shows. */
    needsData() {
      dataPresent = false;
      console.info("[fretwire] mock data cleared — reload to see the first-run import screen.");
    },
    /** Inspect the current in-memory preset. */
    state: () => current,
  };
}
