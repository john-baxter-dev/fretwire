// In-memory mock of the fretwire-tauri Rust backend, for frontend development without an HX Stomp (or even
// a Rust/Tauri toolchain). It implements every `#[command]` in `crates/fretwire-tauri/src/commands.rs` and
// returns the exact DTO shapes from `src/dto.rs`, so the Svelte UI behaves identically to the real
// thing — connect, browse presets, edit params, swap/add/delete blocks, snapshots, split routing,
// drag-to-place, and live-follow device pushes.
//
// It is wired in via `../lib/ipc.js`, which routes `invoke`/`listen` here whenever the app runs
// outside a Tauri webview (i.e. the Vite dev server in a browser). The real backend is untouched.
//
// It can present as either supported unit, so the device-dependent UI is testable without hardware:
//   fretwireMock.device("floor")   // two DSPs, eight setlists  (the default)
//   fretwireMock.device("stomp")   // one DSP, one flat list, no setlist picker
// The choice sticks across reloads; reload after switching, since it applies on the next connect.
//
// State is intentionally persistent for the session: edits mutate the in-memory setlist, so the app
// feels stateful. Reload the page to reset. Trigger live-follow pushes by hand from the devtools
// console via `window.fretwireMock` (see the bottom of this file).

import { base64ToBytes, bytesToBase64 } from "../lib/files.js";

const LATENCY_MS = 30; // a touch of fake IPC latency, to surface races the real backend would

// ---------------------------------------------------------------------------------------------
// Parameter factories — build ParamDto-shaped objects. `value_type`: 0 = enum, 1 = float, 2 = bool
// (matches `.models` valueType). `index` is the wire selector (unique within a block).
// ---------------------------------------------------------------------------------------------
// Link a tempo-sync group — the switch at `tempo`, the note value at `note`, the knob they take
// over at `governed` — the way the real backend does from HX_ModelCatalog.json's grouping.
const withSync = (params, tempo, note, governed) => {
  for (const [index, role] of [[tempo, "tempo"], [note, "note"], [governed, "governed"]]) {
    params.find((p) => p.index === index).sync = { role, tempo, note, governed };
  }
  return params;
};

const P = {
  float: (index, name, value, min = 0, max = 10) => ({
    index, name, value, kind: "float", min, max, value_type: 1, display_type: null, enum_labels: [], stops: [], extra_index: null,
  }),
  int: (index, name, value, min = 0, max = 127) => ({
    index, name, value, kind: "int", min, max, value_type: null, display_type: null, enum_labels: [], stops: [], extra_index: null,
  }),
  // `extraIndex` marks a value the block carries *past* its model's param list (Trails). It is
  // addressed in a separate index space that also starts at 0 — the distinction the device makes
  // with key 29, and the one a push has to preserve. See EditorParam::extra_index.
  bool: (index, name, on, extraIndex = null) => ({
    index, name, value: on ? 1 : 0, kind: "bool", min: 0, max: 1, value_type: 2, display_type: null, enum_labels: [], stops: [], extra_index: extraIndex,
  }),
  // `base` is the wire value of the first label — a discrete control's list spans min..=max, and
  // that is not always 0 (the real `Note Sync` runs 1..=19). See ParamMeta::enum_base.
  enum: (index, name, value, labels, base = 0) => ({
    index, name, value, kind: "int", min: base, max: base + labels.length - 1, value_type: 0,
    display_type: null, enum_labels: labels, enum_base: base, stops: [], extra_index: null,
  }),
  // Segmented float (cab mic Angle): a float on the wire, but rendered as discrete stop buttons.
  seg: (index, name, value, stops) => ({
    index, name, value, kind: "float", min: stops[0].value, max: stops[stops.length - 1].value,
    value_type: 1, display_type: null, enum_labels: [], stops, extra_index: null,
  }),
};

// Per-category parameter templates. Each returns a *fresh* array so swaps/adds get independent state.
const PARAMS = {
  amp: () => [
    P.float(0, "Drive", 5.5), P.float(1, "Bass", 5), P.float(2, "Mid", 6), P.float(3, "Treble", 5.5),
    P.float(4, "Presence", 4), P.float(5, "Ch Vol", 5), P.bool(6, "Bright", false), P.float(7, "Master", 7),
  ],
  // The modern cab shape (category 19), in the device's own order and with the mic names the
  // `cabMICir` control actually carries. Kept faithful because two of its indices are load-bearing
  // for regressions: Distance is **2**, the same index the amp's Mid sits at, which is what made a
  // cab push land on the amp (issue #11) — `fretwireMock.knob(3, 2, 6, false, true)` is that case.
  cab: () => [
    P.enum(0, "Mic", 0, [
      "57 Dynamic", "421 Dynamic", "7 Dynamic", "906 Dynamic", "30 Dynamic", "121 Ribbon",
      "160 Ribbon", "4038 Ribbon", "84 Ribbon", "414 Cond", "47 Cond FET", "67 Cond",
    ]),
    P.float(1, "Position", 0.23, 0, 1),
    P.float(2, "Distance", 2, 1, 12),
    P.seg(3, "Angle", 0, [{ value: 0, label: "0 deg" }, { value: 45, label: "45 deg" }]),
    P.float(4, "Low Cut", 80, 19.9, 500),
    P.float(5, "High Cut", 8000, 500, 20100), P.float(6, "Level", 0, -60, 6),
  ],
  reverb: () => [
    P.enum(0, "Type", 0, ["Room", "Hall", "Plate", "Spring", "Chamber"]),
    P.float(1, "Decay", 4), P.float(2, "Predelay", 20, 0, 500), P.float(3, "Mix", 35, 0, 100),
    P.float(4, "Low Cut", 120, 20, 1000), P.float(5, "High Cut", 8000, 1000, 20000), P.bool(6, "Trails", true, 0),
  ],
  dynamics: () => [P.float(0, "Threshold", -48, -96, 0), P.float(1, "Decay", 30, 0, 100), P.float(2, "Level", 0, -12, 12)],
  distortion: () => [P.float(0, "Drive", 5), P.float(1, "Tone", 5), P.float(2, "Level", 5)],
  // Note Sync carries the real control's 1-based range, so the enum offset stays exercised here.
  // The three are one tempo-sync group, as the real DTO links them from the catalog: the panel
  // shows one control on Time. See ParamMeta::sync.
  delay: () => withSync([
    P.float(0, "Time", 380, 1, 2000), P.float(1, "Feedback", 30, 0, 100), P.float(2, "Mix", 25, 0, 100),
    P.bool(3, "Tempo Sync", false),
    P.enum(4, "Note Sync", 6, ["1/1", "1/2 Dotted", "1/2", "1/2 Triplet", "1/4 Dotted", "1/4", "1/4 Triplet",
      "1/8 Dotted", "1/8", "1/8 Triplet", "1/16 Dotted", "1/16", "1/16 Triplet"], 1),
    P.bool(5, "Trails", true, 0),
  ], 3, 4, 0),
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
    custom_color: null,
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
// The grid is 8 columns wide in *both* rows: top slots 1..8 are columns 1..8, row-B slots 11..18
// are columns 1..8 too, in the same absolute column space. Matches PresetStream::dsp_grid.
const topCol = (slot) => localOf(slot);
const bCol = (slot) => localOf(slot) - SPLIT_LOCAL;
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
  // Column 9 — one past the 8-wide grid — is as far right as the mixer goes.
  const mixerPos = Math.max(np.mixer ?? Math.min(maxB + 2, 9), maxB + 1, splitPos + 1);
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
    deviceModel: opts.deviceModel ?? "HX Stomp", buildStamp: opts.buildStamp ?? "v3.71-32-g1039661",
  };
}

function serialPreset(name, index, defs, snapshot_names = [], assignments = []) {
  const slots = {};
  defs.forEach((d, i) => {
    slots[i + 1] = makeBlock(d.sym, d);
  });
  return { ...makePreset(name, index, slots, { snapshot_names }), assignments };
}

function dualAmpPreset() {
  const slots = {};
  slots[1] = makeBlock("gate", { label: "Gate" });
  slots[3] = makeBlock("amp_princess", { cab: "cab_112", label: "Amp A" }); // path A (col 3)
  slots[5] = makeBlock("reverb_glitz", { footswitch: 1 }); // common-after (col ≥ mixer pos)
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
    buildStamp: "v3.71-32-g1039661",
    snapshot_names: ["Intro", "B&C", "Solo", "", "", "", "", ""],
  });
}

// ---------------------------------------------------------------------------------------------
// Device modes. The mock can present as either unit so the UI differences are visible without
// hardware: an HX Stomp has one flat preset list (HX Edit shows no setlist control at all), while
// a Helix Floor has eight setlists and two DSPs. Switch with `fretwireMock.device("stomp")`.
// ---------------------------------------------------------------------------------------------
const DEVICES = {
  stomp: {
    name: "HX Stomp",
    // One unnamed list — matches Device::setlist_names() falling back to ["Presets"].
    setlists: ["Presets"],
    // Three per bank, so the list reads 01A/01B/01C/02A as the pedal's screen does.
    presetsPerBank: 3,
    setlistSize: 126,
    // Three on the panel, two on the external switch jack. Op 33 refuses a sixth with `-3`.
    footswitches: 5,
  },
  xl: {
    name: "HX Stomp XL",
    setlists: ["Presets"],
    // Four per bank — 01A/01B/01C/01D/02A, read off a real XL's screen. The one place the mock can
    // exercise the numbering toggle against a device that banks differently from the Stomp.
    presetsPerBank: 4,
    setlistSize: 128,
    // Eight, per the owner and the 3.80 manual (pp. 50-51). [reported — issue #13]
    footswitches: 8,
    // Reported, not Verified: the UI has to show this, so the mock has to be able to produce it.
    caveat: "reported working, but not verified against a capture",
  },
  floor: {
    name: "Helix Floor",
    setlists: ["Factory 1", "Factory 2", "User 1", "User 2", "User 3", "User 4", "User 5", "Templates"],
    // Unknown for real, and the mock says so too: the list falls back to slot numbers, which is
    // what a Floor shows today. See Device::presets_per_bank.
    presetsPerBank: null,
    setlistSize: 128,
    // Unread. The Floor has eight preset footswitches, but how many positions its layout carries
    // has never been looked at, so this keeps the Stomp's number rather than inventing one.
    footswitches: 5,
  },
};

// The Stomp's single flat list. Index 0 is the split "Dual Amp" preset.
const stompPresets = () => [
  dualAmpPreset(),
  // `footswitch` = the pedal's own bypass binding (key `3 → 8`, position + 1). Set here so the
  // chain's FS badges have something to draw without hardware.
  serialPreset("Crunch Lead", 1, [
    { sym: "gate" }, { sym: "drive_minotaur", footswitch: 1 },
    { sym: "amp_cali", cab: "cab_212", label: "Amp" }, { sym: "delay_simple", footswitch: 2 },
    { sym: "reverb_glitz" },
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
].map((p, i) => ({ ...p, index: i }));

// The Floor's eight setlists. Only some are populated — User 3..5 are empty on a stock unit, which
// is worth being able to see. Preset indices restart at 0 in every setlist, exactly as the wire
// reports them (PresetInfo.index is relative to its bank).
function floorSetlists() {
  const bank = (...ps) => ps.map((p, i) => ({ ...p, index: i }));
  return [
    // Factory 1 — index 0 is the two-DSP preset, so connecting shows both routing grids.
    bank(floorPreset(), dualAmpPreset(), serialPreset("Riffs And Beards", 0, [
      { sym: "gate" }, { sym: "drive_teemah" }, { sym: "amp_placater", cab: "cab_412", label: "Amp" },
    ]), serialPreset("Felix Mark IV", 0, [
      { sym: "comp_la" }, { sym: "amp_brit", cab: "cab_212", label: "Amp" }, { sym: "reverb_hall" },
    ])),
    // Factory 2
    bank(serialPreset("Bumble Acoustic", 0, [{ sym: "comp_deluxe" }, { sym: "eq_graphic" }]),
      // The wah carries a **bypass on an expression pedal**: a key-`4` entry with a target slot and
      // no parameter. That is the other destination a bypass has, chosen by the source rather than
      // by what is driven (`docs/preset-format.md`). Seeded because the UI hid these entirely until
      // 2026-08-25 — the param rows match on `param_index`, so an XL owner's preset with two of them
      // drew no badge at all (issue #13). A wah auto-engaging off a pedal is the case `tonepush`
      // documents, so this is where it belongs, and this bank is one the default mock mode reaches.
      // Read-only: which opcode writes one is unread, so nothing here offers it.
      serialPreset("The Blue Agave", 0, [{ sym: "wah_teardrop" }, { sym: "amp_jazz", cab: "cab_112" }],
        [], [{ source: 1, target_slot: 1, param_index: null, paired: false, min: null, max: null }])),
    // User 1 — where the tester's "Sludge" lives.
    bank(serialPreset("Sludge", 0, [
      { sym: "gate" }, { sym: "drive_minotaur" }, { sym: "amp_placater", cab: "cab_412", label: "Amp" },
      { sym: "delay_simple" },
    ]), serialPreset("Richeese", 0, [{ sym: "boost_kinky" }, { sym: "amp_cali", cab: "cab_212" }])),
    // User 2
    bank(serialPreset("Scratch Pad", 0, [{ sym: "amp_cali", cab: "cab_412" }])),
    [], // User 3 — empty
    [], // User 4 — empty
    [], // User 5 — empty
    // Templates
    bank(serialPreset("Blank", 0, []), serialPreset("Basic Amp", 0, [{ sym: "amp_jazz", cab: "cab_112" }])),
  ];
}

// ---------------------------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------------------------
let connected = false;

// Which unit the mock is pretending to be, and its setlists: banks[bank] is a list of presets.
// The choice is remembered across reloads (`fretwireMock.device(…)` writes it), because switching
// device only takes effect on the next connect — so you reload anyway, and losing the setting on
// every reload would make Stomp mode almost impossible to actually sit in.
const MODE_KEY = "fretwire.mock.device";
const storage = (() => {
  try {
    return typeof localStorage !== "undefined" ? localStorage : null;
  } catch {
    return null; // e.g. a sandboxed iframe, or Node
  }
})();
const savedMode = storage?.getItem(MODE_KEY);
let deviceMode = savedMode && DEVICES[savedMode] ? savedMode : "floor";
const buildBanks = () => (deviceMode === "floor" ? floorSetlists() : [stompPresets()]);
let banks = buildBanks();
let currentBank = 0;
const setlistNames = () => DEVICES[deviceMode].setlists;
// Device::preset_label — how the pedal's screen writes a slot (`09A`), or null where we don't know
// this device's banking and the UI falls back to the slot number.
const presetLabel = (slot) => {
  const per = DEVICES[deviceMode].presetsPerBank;
  if (!per) return null;
  return String(Math.floor(slot / per) + 1).padStart(2, "0") + String.fromCharCode(65 + (slot % per));
};
// Device::preset_numbering_labels — setting 27's menu spells out the preset range, so its text is
// the pedal's, not the table's. Null where the bank size was never measured, and the static labels
// then stand.
const presetNumberingLabels = () => {
  const { presetsPerBank: per, setlistSize: size } = DEVICES[deviceMode];
  if (!per || !size || size % per !== 0) return null;
  return [`000-${size - 1}`, `01A-${String(size / per).padStart(2, "0")}${String.fromCharCode(64 + per)}`];
};
const bankOf = (b) => banks[b] ?? [];
let current = bankOf(0)[0];
// Export/restore round-trip (see the export_setlists handler): the last export made this session,
// and whether a sweep in flight has been called off.
let lastBackup = null;
let exportCancelled = false;

const hexEncode = (str) =>
  Array.from(new TextEncoder().encode(str), (b) => b.toString(16).padStart(2, "0")).join("");
const hexDecode = (hex) =>
  new TextDecoder().decode(new Uint8Array(hex.match(/.{2}/g)?.map((h) => parseInt(h, 16)) ?? []));

// A preset's "raw" in a mock file: the mock state, hex-encoded, standing in for the stream.
const presetRaw = (p) =>
  hexEncode(JSON.stringify({
    name: p.name, snapshot_names: p.snapshot_names, active_snapshot: p.active_snapshot,
    slots: p.slots, nodePos: p.nodePos, dspCount: p.dspCount,
    deviceModel: p.deviceModel, buildStamp: p.buildStamp,
  }));
// A setting as a v3 file records it: typed, so a restore sends it back in the type held.
const settingEntry = (id, d) => ({
  id,
  type: d.kind === "flag" ? "bool" : d.kind === "choice" ? "int" : "f32",
  value: d.kind === "flag" ? !!d.v : d.v,
  name: d.name,
});
// An IR's "raw" in a mock file: the mock has no samples, so the stored digest stands in for the
// blob; what matters to the restore is that the same file restores to the same hash.
const irRaw = (held) => hexEncode(`${held.md5}:${held.name}`);

// Walks whichever setlists it is given, as the real sweep does; with `irs`/`settings` set it is
// `backup_device`, and the file is version 3 with those two sections.
async function sweepBackup(banks, { irs = false, settings = false } = {}) {
  exportCancelled = false;
  const entries = [];
  const lists = banks.map((b) => [b, bankOf(b)]);
  const irSlots = irs ? [...irStore.keys()].sort((a, b) => a - b) : [];
  const total = lists.reduce((n, [, l]) => n + l.length, 0) + irSlots.length + (settings ? 1 : 0);
  let done = 0;
  outer: for (const [bank, list] of lists) {
    for (const p of list) {
      await sleep(120); // the real sweep takes ~a second per preset; make the progress UI visible
      done++;
      entries.push({ bank, index: p.index, name: p.name, raw_hex: presetRaw(p) });
      emit("backup-progress", {
        done, total, stage: "presets", bank, setlist: setlistNames()[bank] ?? "Presets", name: p.name,
      });
      // Checked after the entry is kept, like the real sweep: a cancelled export still holds
      // everything read up to the moment it was called off.
      if (exportCancelled) break outer;
    }
  }
  const file = {
    format: "fretwire-backup", version: irs || settings ? 3 : 2,
    device: DEVICES[deviceMode].name,
    setlists: banks.map((b) => ({ bank: b, name: setlistNames()[b] ?? "Presets" })),
    presets: entries,
  };
  if (irs || settings) {
    file.irs = [];
    file.settings = [];
    if (!exportCancelled) {
      for (const slot of irSlots) {
        await sleep(60);
        const held = irStore.get(slot);
        file.irs.push({ slot, name: held.name, raw_hex: irRaw(held) });
        done++;
        emit("backup-progress", { done, total, stage: "irs", bank: 0, setlist: "IRs", name: held.name || "(unnamed)" });
        if (exportCancelled) break;
      }
    }
    if (settings && !exportCancelled) {
      for (const [id, d] of [...SETTINGS.entries()].sort((a, b) => a[0] - b[0])) file.settings.push(settingEntry(id, d));
      // The unidentified ids answer too and are recorded; a restore never writes them.
      for (const id of RAW_IDS) file.settings.push({ id, type: "int", value: id % 3 });
      done++;
      emit("backup-progress", { done, total, stage: "settings", bank: 0, setlist: "Settings", name: "global settings" });
    }
  }
  lastBackup = file;
  return lastBackup;
}

const backupInfo = (file) => ({
  device: file.device,
  version: file.version ?? 1,
  presets: file.presets.length,
  irs: file.irs?.length ?? 0,
  settings: file.settings?.length ?? 0,
  setlists: (file.setlists ?? []).map((s) => s.name),
});

// Like Session::restore_device: each part only when asked, nothing written that the pedal already
// holds, unidentified settings skipped. The mock cannot fail a write, so `failures` stays empty.
async function restoreDevice(file, { presets = true, irs = true, settings = true }) {
  if (file.device !== DEVICES[deviceMode].name) {
    throw new Error(`this file came off a ${file.device} and this is a ${DEVICES[deviceMode].name} — pass force to restore anyway`);
  }
  const r = {
    presets_written: 0, presets_unchanged: 0, irs_written: 0, irs_unchanged: 0,
    settings_written: 0, settings_unchanged: 0, settings_skipped: [], failures: [], skipped: [],
  };
  const total = (presets ? file.presets.length : 0) + (irs ? file.irs?.length ?? 0 : 0) + (settings && file.settings?.length ? 1 : 0);
  let done = 0;
  if (presets) {
    for (const e of file.presets) {
      await sleep(60);
      const bank = e.bank ?? 0;
      const list = bankOf(bank);
      const at = list.findIndex((p) => p.index === e.index);
      if (at >= 0 && presetRaw(list[at]) === e.raw_hex) r.presets_unchanged++;
      else {
        let state;
        try { state = JSON.parse(hexDecode(e.raw_hex)); }
        catch { throw new Error("mock backend: only files made by the mock itself can be restored here"); }
        const restored = { ...state, index: e.index, nodePos: state.nodePos ?? {} };
        if (at >= 0) list[at] = restored;
        else { list.push(restored); list.sort((a, b) => a.index - b.index); }
        if (bank === currentBank && current.index === e.index) current = restored;
        r.presets_written++;
      }
      done++;
      emit("backup-progress", { done, total, stage: "presets", bank, setlist: setlistNames()[bank] ?? "Presets", name: e.name });
    }
  }
  if (irs) {
    for (const ir of file.irs ?? []) {
      await sleep(60);
      const held = irStore.get(ir.slot);
      if (held && irRaw(held) === ir.raw_hex) r.irs_unchanged++;
      else {
        const [md5] = hexDecode(ir.raw_hex).split(":");
        irStore.set(ir.slot, { name: ir.name, samples: 2048, checksum: parseInt(md5.slice(0, 8), 16) >>> 0, md5 });
        r.irs_written++;
      }
      done++;
      emit("backup-progress", { done, total, stage: "irs", bank: 0, setlist: "IRs", name: ir.name || "(unnamed)" });
    }
  }
  if (settings && file.settings?.length) {
    for (const s of file.settings) {
      const d = SETTINGS.get(s.id);
      if (!d) { r.settings_skipped.push(`${s.id}: not an identified setting`); continue; }
      const held = settingEntry(s.id, d);
      if (held.type !== s.type) { r.settings_skipped.push(`${s.id}: the device holds it as ${held.type}, the file as ${s.type}`); continue; }
      if (held.value === s.value) { r.settings_unchanged++; continue; }
      d.v = s.type === "bool" ? !!s.value : s.value;
      r.settings_written++;
    }
    done++;
    emit("backup-progress", { done, total, stage: "settings", bank: 0, setlist: "Settings", name: "global settings" });
  }
  clearHistory();
  seedHistory();
  return r;
}

// The file as the UI hands it back — its text. Only the mock's own files parse, since their
// "raw" is the mock state and not a preset stream; that is the honest limit of a mock.
function parseBackup(json) {
  let file;
  try {
    file = JSON.parse(json);
  } catch (e) {
    throw new Error(`not a fretwire export file: ${e.message}`);
  }
  if (file?.format !== "fretwire-backup" || !Array.isArray(file.presets)) {
    throw new Error("not a fretwire export file");
  }
  return file;
}

const backupEntries = (backup) =>
  backup.presets.map((p) => ({
    index: p.index,
    name: p.name,
    bank: p.bank ?? 0,
    setlist: backup.setlists?.find((s) => s.bank === (p.bank ?? 0))?.name ?? null,
  }));

function restoreFrom(backup, index, slot, bank) {
  const entry = backup.presets.find((p) => p.index === index && (p.bank ?? 0) === bank);
  if (!entry) throw new Error(`export file has no preset at bank ${bank} slot ${index}`);
  let state;
  try {
    state = JSON.parse(hexDecode(entry.raw_hex));
  } catch {
    throw new Error("mock backend: only files exported by the mock itself can be restored here");
  }
  // Files from before the sweep recorded the DSP layout take the current preset's, as the real
  // restore takes the device's.
  const restored = {
    name: state.name, index: slot, active_snapshot: state.active_snapshot ?? 0,
    snapshot_names: state.snapshot_names ?? [], slots: state.slots, nodePos: state.nodePos ?? {},
    dspCount: state.dspCount ?? current.dspCount,
    deviceModel: state.deviceModel ?? current.deviceModel,
    buildStamp: state.buildStamp ?? current.buildStamp,
  };
  const list = bankOf(currentBank);
  const at = list.findIndex((p) => p.index === slot);
  if (at >= 0) list[at] = restored;
  else {
    list.push(restored);
    list.sort((a, b) => a.index - b.index);
  }
  // Like Session::restore_preset: the device ends up on the restored slot, history reset.
  current = restored;
  clearHistory();
  seedHistory();
  return toDto(current);
}


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
  clear_preset: () => "Clear preset",
  revert_preset: () => "Revert to saved",
  place_block: (a) => `Move ${slotName(a.srcSlot)}`,
  insert_block: (a) => `Move ${slotName(a.srcSlot)} ${a.before ? "before" : "after"} ${slotName(a.dstSlot)}`,
  reorder_block: (a) => `Move ${slotName(a.srcSlot)}`,
  move_block_to_row: (a) => `Move ${slotName(a.srcSlot)}`,
  move_before_split: (a) => `Move ${slotName(a.srcSlot)}`,
  set_node_pos: (a) => `Move ${a.node} node → col ${a.pos}`,
  set_split_type: (a) => `Split type → ${SPLIT_TYPES.find((t) => t.index === a.modelIndex)?.label ?? "?"}`,
  assign_bypass: (a) => `FS${a.switch + 1} → ${slotName(a.slot)}`,
  set_switch_label: (a) => (a.label ? `FS${a.switch + 1} label “${a.label}”` : `FS${a.switch + 1} label cleared`),
  set_switch_color: (a) => (a.color != null ? `FS${a.switch + 1} colour ${a.color}` : `FS${a.switch + 1} colour cleared`),
  set_switch_momentary: (a) => `FS${a.switch + 1} ${a.momentary ? "momentary" : "latching"}`,
  unassign_bypass: (a) => `${slotName(a.slot)} off FS${a.switch + 1}`,
  assign_param: (a) => (a.source === 0 ? "Unassign parameter" : `${sourceName(a.source)} → parameter`),
  set_assign_travel: (a) => `${a.max ? "Max" : "Min"} ${a.value}`,
};

// ---------------------------------------------------------------------------------------------
// DTO projection — mirrors `From<&EditorPreset> for PresetDto` etc. in src/dto.rs.
// ---------------------------------------------------------------------------------------------
const clone = (x) => structuredClone(x);

function blockDto(p, slot, e) {
  const cab = e.paired_index != null ? findModel(e.paired_index) : null;
  return {
    slot, dsp: dspOf(slot), row: isRowB(slot) ? 1 : 0,
    model_name: e.model_name, user_label: e.user_label, custom_color: e.custom_color ?? null,
    momentary: e.momentary ?? false,
    symbolic_id: e.symbolic_id,
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
    name: p.name, index: p.index, bank: currentBank, device_model: p.deviceModel, build_stamp: p.buildStamp,
    device_name: DEVICES[deviceMode].name,
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
    assignments: (p.assignments ?? []).map((a) => ({
      ...a,
      source_name: sourceName(a.source),
      param_name:
        (a.paired ? p.slots[a.target_slot]?.paired_params : p.slots[a.target_slot]?.params)?.find(
          (q) => q.index === a.param_index,
        )?.name ?? null,
    })),
    // The real backend reads this off the preset's own footswitch layout, so it is the device's
    // number without the UI knowing any model. Mirrored per mock device for the same reason — issue
    // #13 was the bypass picker capping this at 5, which an XL owner hit and no mock run could.
    footswitch_count: DEVICES[deviceMode].footswitches,
  };
}

// Mirrors `fretwire_protocol::edit::source`. The ordinal space is the device's size, not a
// constant: 0 none, 1/2 the expression inputs, one per footswitch from 3, then MIDI, then
// snapshots — so the table is `footswitches + 5` and ordinal 8 is MIDI on a Stomp but FS6 on an XL.
// [solid — six Stomp captures at 5/10, two XL at 8/13; issue #13]
// Exported for `tests/sources-mock.mjs`: the XL is only reachable through `fretwireMock.device`,
// which needs a `window`, so the arithmetic is checked directly rather than not at all.
export const sourceTableLen = (fs) => fs + 5;
export const sourceMidi = (fs) => 3 + fs;
export const sourceSnapshots = (fs) => 4 + fs;

export function sourceName(n, fs = DEVICES[deviceMode].footswitches) {
  if (n === 1) return "EXP1";
  if (n === 2) return "EXP2";
  if (fs === 0) return `Controller ${n}`;
  if (n >= 3 && n < sourceMidi(fs)) return `FS${n - 2}`;
  if (n === sourceMidi(fs)) return "MIDI";
  if (n === sourceSnapshots(fs)) return "Snapshots";
  return `Controller ${n}`;
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

// The update check. The preference starts unanswered so `npm run dev` walks the ask bar, and
// the "latest release" is one patch past the mock's own version so the badge is visible.
const MOCK_VERSION = "0.5.1";
const MOCK_LATEST = MOCK_VERSION.replace(/\d+$/, (n) => String(Number(n) + 1));
let updatePref = null; // null = not asked
let updateLatest = null;
let updateCheckedAt = null;
const updateDto = () => ({
  current: MOCK_VERSION,
  latest: updateLatest,
  available: updateLatest != null && updateLatest !== MOCK_VERSION,
  url: updateLatest ? `https://github.com/john-baxter-dev/fretwire/releases/tag/v${updateLatest}` : null,
  enabled: updatePref,
  locked: false,
  checked_at: updateCheckedAt,
  install: "source",
  install_label: "built from a checkout",
  instruction: "Pull the repository and rebuild.",
});


// ---------------------------------------------------------------------------------------------
// The user IR store. Mirrors the device's rules rather than a convenient subset, because those
// rules are what the panel is built against: occupancy is the declared length (not the name), the
// directory listing carries the stored hash but no checksum, a per-slot scan carries the checksum
// but no hash, and every write here is irreversible.
// ---------------------------------------------------------------------------------------------
const IR_SLOTS = 128;
const irStore = new Map([
  [0, { name: "G12-65 212 C Hi-Gn 421+57", samples: 2048, checksum: 0xc0a076ed, md5: "4b41c57b04c05b1471277ecf74231a7d" }],
  [3, { name: "OwnHammer HD412 M25", samples: 2048, checksum: 0x51ba30d1, md5: "1f0d9c2ab4e75630fd8814c2b0a99e77" }],
  // A nameless silent one, so the panel's "silent" tag has something to render: the device does
  // distinguish this from an empty slot, and reading occupancy off the name would not.
  [7, { name: "", samples: 2048, checksum: 0, md5: "620f0b67a91f7f74151bc5be745b7110" }],
]);

function irDto(index, withHash) {
  const held = irStore.get(index);
  const name = held?.name ?? "";
  const used = !!held;
  return {
    index,
    name,
    display_name: used ? (name || "(unnamed)") : "—",
    used,
    samples: held?.samples ?? 0,
    // The two listings answer with different fields; the panel has to cope with either.
    checksum: withHash ? null : (held?.checksum ?? 0),
    md5: withHash ? (held?.md5 ?? null) : null,
  };
}

const irDirectory = () =>
  [...irStore.keys()].sort((a, b) => a - b).map((i) => irDto(i, true));

function irCheckSlot(slot) {
  if (!Number.isInteger(slot) || slot < 0 || slot >= IR_SLOTS) {
    throw new Error(`IR slot ${slot} is out of range (0..${IR_SLOTS})`);
  }
}

// A real (if silent) WAV for `ir_export_inline`: 32-bit float mono, the device's format, so what
// the browser downloads opens in an audio editor. 44-byte canonical header.
function silentWav(samples, rate = 48000) {
  const data = samples * 4;
  const buf = new ArrayBuffer(44 + data);
  const v = new DataView(buf);
  const tag = (at, str) => [...str].forEach((c, i) => v.setUint8(at + i, c.charCodeAt(0)));
  tag(0, "RIFF"); v.setUint32(4, 36 + data, true); tag(8, "WAVE");
  tag(12, "fmt "); v.setUint32(16, 16, true); v.setUint16(20, 3, true); v.setUint16(22, 1, true);
  v.setUint32(24, rate, true); v.setUint32(28, rate * 4, true); v.setUint16(32, 4, true);
  v.setUint16(34, 32, true);
  tag(36, "data"); v.setUint32(40, data, true);
  return new Uint8Array(buf);
}

// The sample rate a WAV declares, read off its header — the real backend's parser does the same
// before refusing a 44.1 kHz file. Throws for anything that is not a WAV at all.
function wavRate(bytes) {
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = (at) => String.fromCharCode(...bytes.subarray(at, at + 4));
  if (bytes.length < 44 || tag(0) !== "RIFF" || tag(8) !== "WAVE") throw new Error("not a WAV file");
  return v.getUint32(24, true);
}

// The rate check both IR uploads share; the message is the real backend's, word for word.
function irCheckRate(rate, force) {
  if (rate !== 48000 && !force) {
    throw new Error(`that file is ${rate} Hz and the device runs at 48000 Hz. Nothing here resamples, so it would play short and bright — convert it first`);
  }
}

// A fake but stable digest, so re-uploading the same thing twice reports the same hash.
function irStash(slot, name, seed) {
  const digest = [...seed].reduce((a, c) => (a * 33 + c.charCodeAt(0)) >>> 0, 5381);
  irStore.set(slot, {
    name: (name ?? "").slice(0, 31),
    samples: 2048,
    checksum: digest,
    md5: digest.toString(16).padStart(8, "0").repeat(4),
  });
}

// The mock pedal's globals. Only the ids fretwire has identified are named; `raw` stands in for the
// unnamed majority that answer and have never been explained, so the panel's read-only tier is
// exercised too. FS7/FS8 Function are XL-only on real hardware; they are here because the panel
// still has to draw a ten-option dropdown somewhere.
// Keyed in id order, like `fretwire_protocol::settings::SETTINGS`; MENU_ORDER below is what puts
// them in the pedal's own menu order, exactly as the real backend does it.
const SETTINGS = new Map([
  [2, { v: false, name: "Send/Return L", group: "Ins/Outs", kind: "flag",
        labels: ["Line", "Instrument"] }],
  [3, { v: false, name: "Send/Return R", group: "Ins/Outs", kind: "flag",
        labels: ["Line", "Instrument"] }],
  [9, { v: 0, name: "MIDI Base Channel", group: "MIDI/Tempo", kind: "choice",
        options: Array.from({ length: 16 }, (_, i) => [i, String(i + 1)]) }],
  [11, { v: true, name: "USB MIDI", group: "MIDI/Tempo", kind: "flag", labels: ["On", "Off"] }],
  [10, { v: false, name: "MIDI Thru", group: "MIDI/Tempo", kind: "flag", labels: ["On", "Off"] }],
  [12, { v: 3, name: "MIDI PC Rx", group: "MIDI/Tempo", kind: "choice",
         options: [[0, "Off"], [1, "MIDI"], [2, "USB"], [3, "MIDI+USB"]] }],
  [13, { v: 3, name: "Rx MIDI Clock", group: "MIDI/Tempo", kind: "choice",
         options: [[0, "Off"], [1, "MIDI"], [2, "USB"], [3, "Auto"]] }],
  [14, { v: 1, name: "Tempo Select", group: "MIDI/Tempo", kind: "choice",
         options: [[0, "Snapsht"], [1, "Preset"], [2, "Global"]] }],
  [16, { v: 120, name: "BPM", group: "MIDI/Tempo", kind: "number", unit: "BPM" }],
  [17, { v: 3, name: "Stomp Select", group: "Footswitches", kind: "choice",
         options: [[0, "Off"], [1, "Touch"], [2, "Press"], [3, "Both"]] }],
  [18, { v: 0, name: "Preset Mode", group: "Footswitches", kind: "choice",
         options: [[0, "Moment"], [1, "Latch"]] }],
  [19, { v: 1, name: "Stomp Mode", group: "Footswitches", kind: "choice",
         options: [[0, "4 Swtch"], [1, "6 Swtch"]] }],
  [20, { v: 0, name: "Up/Down Switches", group: "Footswitches", kind: "choice",
         options: [[0, "Banks"], [1, "Preset"], [2, "Snapsht"]] }],
  [25, { v: true, name: "LED Rings", group: "Displays", kind: "flag",
         labels: ["Dim/Brt", "Off/Brt"] }],
  [26, { v: true, name: "Tap LED", group: "Displays", kind: "flag", labels: ["On", "Off"] }],
  [27, { v: false, name: "Preset Number", group: "Preferences", kind: "flag",
         labels: ["000-127", "01A-32D"] }],
  [31, { v: false, name: "Input Level", group: "Ins/Outs", kind: "flag",
         labels: ["Line", "Instrument"] }],
  [65, { v: false, name: "Tempo Pitch", group: "Preferences", kind: "flag",
         labels: ["Transpr", "Authentc"] }],
  [66, { v: 1, name: "EXP 1 Position", group: "EXP Pedals", kind: "choice",
         options: [[0, "Snapsht"], [1, "Preset"], [2, "Global"]] }],
  [67, { v: 1, name: "Snapsht Mode", group: "Footswitches", kind: "choice",
         options: [[0, "Moment"], [1, "Latch"], [2, "Toggle"]] }],
  [68, { v: 0, name: "Tip Polarity", group: "Preferences", kind: "choice",
         options: [[0, "Normal"], [1, "Inverted"]] }],
  [69, { v: 0, name: "Ring Polarity", group: "Preferences", kind: "choice",
         options: [[0, "Normal"], [1, "Inverted"]] }],
  [71, { v: 1, name: "EXP 2 Position", group: "EXP Pedals", kind: "choice",
         options: [[0, "Snapsht"], [1, "Preset"], [2, "Global"]] }],
  [73, { v: 0, name: "Snapshot Edits", group: "Preferences", kind: "choice",
         options: [[0, "Recall"], [1, "Discard"]] }],
  [76, { v: 0, name: "Tx MIDI Clock", group: "MIDI/Tempo", kind: "choice",
         options: [[0, "Off"], [1, "MIDI"], [2, "USB"], [3, "MIDI+USB"]] }],
  [77, { v: 3, name: "MIDI PC Tx", group: "MIDI/Tempo", kind: "choice",
         options: [[0, "Off"], [1, "MIDI"], [2, "USB"], [3, "MIDI+USB"]] }],
  [81, { v: false, name: "Bypass Type", group: "Preferences", kind: "flag",
         labels: ["DSP", "Analog"] }],
  [94, { v: true, name: "Output Level", group: "Ins/Outs", kind: "flag",
         labels: ["Line", "Instrument"] }],
  [95, { v: false, name: "EXP/FS Tip", group: "Preferences", kind: "flag",
         labels: ["FS7", "EXP 1"] }],
  [96, { v: false, name: "EXP/FS Ring", group: "Preferences", kind: "flag",
         labels: ["FS8", "EXP 2"] }],
  [103, { v: 0, name: "Snapshot Reselect", group: "Preferences", kind: "choice",
          options: [[0, "Reload"], [1, "Toggle"]] }],
  [117, { v: 0, name: "Swap Up/Down", group: "Footswitches", kind: "choice",
          options: [[0, "Off"], [1, "On"]] }],
  [127, { v: 0, name: "Auto In-Z", group: "Preferences", kind: "choice",
          options: [[0, "First"], [1, "Enabled"]] }],
  [129, { v: false, name: "TAP Function", group: "Footswitches", kind: "flag",
          labels: ["AllBypas", "TAP/Tunr"] }],
  [130, { v: 0, name: "FS7 Function", group: "Footswitches", kind: "choice",
          options: [[0, "TAP/Tunr"], [1, "Stomp 7"], [2, "Bank Up"], [3, "Bank Dn"],
                   [4, "PresetUp"], [5, "PresetDn"], [6, "SnpshtUp"], [7, "SnpshtDn"],
                   [8, "AllBypas"], [9, "TogglEXP"]] }],
  [131, { v: 1, name: "FS8 Function", group: "Footswitches", kind: "choice",
          options: [[0, "TAP/Tunr"], [1, "Stomp 8"], [2, "Bank Up"], [3, "Bank Dn"],
                   [4, "PresetUp"], [5, "PresetDn"], [6, "SnpshtUp"], [7, "SnpshtDn"],
                   [8, "AllBypas"], [9, "TogglEXP"]] }],
  [135, { v: 0, name: "Snapshot CC Send", group: "MIDI/Tempo", kind: "choice",
          options: [[0, "Off"], [1, "On"]] }],
  [136, { v: 0, name: "Link Dual Cabs", group: "Preferences", kind: "choice",
          options: [[0, "Off"], [1, "On"]] }],
  [153, { v: 0, name: "USB In 1/2 Trim", group: "Ins/Outs", kind: "number", unit: "dB" }],
  [154, { v: 0, name: "Return Type", group: "Ins/Outs", kind: "choice",
          options: [[0, "Return"], [1, "Aux In"]] }],
  [156, { v: 1, name: "Volume Controls", group: "Ins/Outs", kind: "choice",
          options: [[1, "Phones"], [2, "Main+HP"]] }],
  [158, { v: 1, name: "Phones Monitor", group: "Ins/Outs", kind: "choice",
          options: [[1, "Main L/R"], [2, "Send"]] }],
  [190, { v: 110, name: "EQ low frequency", group: "Global EQ", kind: "number", unit: "Hz" }],
  [191, { v: 0.707, name: "EQ low Q", group: "Global EQ", kind: "number", unit: "" }],
  [192, { v: 0, name: "EQ low gain", group: "Global EQ", kind: "number", unit: "dB" }],
  [193, { v: 2000, name: "EQ mid frequency", group: "Global EQ", kind: "number", unit: "Hz" }],
  [194, { v: 0.707, name: "EQ mid Q", group: "Global EQ", kind: "number", unit: "" }],
  [195, { v: 0, name: "EQ mid gain", group: "Global EQ", kind: "number", unit: "dB" }],
  [196, { v: 8000, name: "EQ high frequency", group: "Global EQ", kind: "number", unit: "Hz" }],
  [197, { v: 0.707, name: "EQ high Q", group: "Global EQ", kind: "number", unit: "" }],
  [198, { v: 0, name: "EQ high gain", group: "Global EQ", kind: "number", unit: "dB" }],
  [199, { v: 19.9, name: "EQ low cut", group: "Global EQ", kind: "number", unit: "Hz", off: 19.9 }],
  [200, { v: 20100, name: "EQ high cut", group: "Global EQ", kind: "number", unit: "Hz", off: 20100 }],
]);
// Ids that answer but have never been identified. Read-only, and shown only with `all`.
const RAW_IDS = [128, 210, 226];

// The pedal's own menu order, for the ids somebody has placed — mirrors
// `fretwire_protocol::settings::MENU_ORDER`. Anything absent sorts after all of it, by id.
const MENU_ORDER = [
  31, 94, 2, 3, 154, 153, 158, 156, // Ins/Outs
  81, 73, 65, 95, 96, 68, 69, 27, 103, 127, 136, // Preferences
  17, 19, 18, 67, 20, 117, 129, 130, 131, // Footswitches
  66, 71, // EXP Pedals
  9, 10, 13, 76, 14, 16, 11, 12, 77, 135, // MIDI/Tempo
  25, 26, // Displays
];
const menuRank = (id) => {
  const i = MENU_ORDER.indexOf(id);
  return i === -1 ? MENU_ORDER.length : i;
};

// Section order — mirrors `fretwire_protocol::settings::GROUPS`. "Unidentified" is absent on
// purpose: an unplaced name ranks last, which is where the raw tier belongs.
const GROUPS = [
  "Global EQ", "Ins/Outs", "Preferences", "Footswitches", "EXP Pedals", "MIDI/Tempo", "Displays",
];
const groupRank = (g) => {
  const i = GROUPS.indexOf(g);
  return i === -1 ? GROUPS.length : i;
};
const groupOf = (id) => SETTINGS.get(id)?.group ?? "Unidentified";

// The pedal's factory EQ, read off a unit after every Global EQ knob was pushed in. Only the EQ
// has one — a null default is how "we have never watched this reset" is expressed, and the panel
// offers no reset button for it.
const DEFAULTS = new Map([
  [190, 110], [191, 0.707], [192, 0], [193, 2000], [194, 0.707], [195, 0],
  [196, 8000], [197, 0.707], [198, 0], [199, 19.9], [200, 20100],
]);

function settingDto(id) {
  const d = SETTINGS.get(id);
  if (!d) {
    return { id, name: `Setting ${id}`, group: "Unidentified", kind: "raw", value: id % 3,
             labels: null, options: [], unit: "", off: null, default: null, writable: false };
  }
  return {
    id, name: d.name, group: d.group, kind: d.kind, value: d.v,
    labels: (id === 27 ? presetNumberingLabels() : null) ?? d.labels ?? null,
    options: d.options ?? [], unit: d.unit ?? "",
    off: d.off ?? null, default: DEFAULTS.has(id) ? DEFAULTS.get(id) : null, writable: true,
  };
}

const HANDLERS = {
  // The mock pedal is set to the banked form, like the hardware ships. Returning a value here (not
  // null) is what exercises the adopt path; `null` would exercise only the fallback.
  device_numbering: () => (SETTINGS.get(27).v ? "flat" : "banked"),
  settings_read: ({ all }) => {
    const ids = all ? [...SETTINGS.keys(), ...RAW_IDS] : [...SETTINGS.keys()];
    ids.sort(
      (a, b) =>
        groupRank(groupOf(a)) - groupRank(groupOf(b)) || menuRank(a) - menuRank(b) || a - b,
    );
    return ids.map(settingDto);
  },
  settings_write: ({ id, value }) => {
    const d = SETTINGS.get(id);
    // The device refuses an id it doesn't implement, and fretwire refuses one it can't name — the
    // panel must handle a rejected write, so the mock has to be able to reject one.
    if (!d) throw new Error(`setting ${id} is not one fretwire has identified`);
    // Typed: a bool takes a bool, an int rounds, a float stays a float. Writing the wrong type is
    // what the device answers -3 to.
    d.v = d.kind === "flag" ? Number(value) !== 0
        : d.kind === "choice" ? Math.round(Number(value))
        : Number(value);
    return settingDto(id);
  },
  ir_list: () => irDirectory(),
  ir_scan: () =>
    Array.from({ length: IR_SLOTS }, (_, i) => irDto(i, false)),
  ir_export: ({ slot, path }) => {
    irCheckSlot(slot);
    const held = irStore.get(slot);
    if (!held) throw new Error(`IR slot ${slot} is empty`);
    console.info(`[fretwire mock] would write ${path}`);
    return held.name;
  },
  ir_upload: ({ slot, path, name, overwrite, force }) => {
    irCheckSlot(slot);
    const held = irStore.get(slot);
    if (held && !overwrite) {
      throw new Error(`IR slot ${slot} already holds "${held.name || "(unnamed)"}" — pass overwrite to replace it`);
    }
    // A browser can't read the path, so the rate is guessed from its name.
    irCheckRate(/44100|44\.1/.test(path) ? 44100 : 48000, force);
    irStash(slot, name, path);
    return irDirectory();
  },
  // The inline pair: the file travels in the call. The export is a genuine WAV and the upload
  // reads the header of what it is given, so the browser flows are exercised end to end.
  ir_export_inline: ({ slot }) => {
    irCheckSlot(slot);
    const held = irStore.get(slot);
    if (!held) throw new Error(`IR slot ${slot} is empty`);
    return { name: held.name, wav_base64: bytesToBase64(silentWav(held.samples)) };
  },
  ir_upload_inline: ({ slot, wavBase64, name, overwrite, force }) => {
    irCheckSlot(slot);
    const held = irStore.get(slot);
    if (held && !overwrite) {
      throw new Error(`IR slot ${slot} already holds "${held.name || "(unnamed)"}" — pass overwrite to replace it`);
    }
    let bytes;
    try {
      bytes = base64ToBytes(wavBase64);
    } catch (e) {
      throw new Error(`wav_base64: ${e.message ?? e}`);
    }
    irCheckRate(wavRate(bytes), force);
    irStash(slot, name, wavBase64);
    return irDirectory();
  },
  ir_delete: ({ slot }) => {
    irCheckSlot(slot);
    irStore.delete(slot);
    return irDirectory();
  },
  ir_rename: ({ slot, name }) => {
    irCheckSlot(slot);
    const held = irStore.get(slot);
    if (!held) throw new Error(`IR slot ${slot} is empty`);
    // Renaming leaves the samples — and so the hash — untouched, as the device does.
    held.name = (name ?? "").slice(0, 31);
    return irDirectory();
  },

  update_status: () => updateDto(),
  update_check: async ({ force = false } = {}) => {
    // Mirrors fretwire_core::update::check — the automatic form only probes when opted in and
    // not already checked; a forced one always does.
    if (!force && (updatePref !== true || updateCheckedAt != null)) return updateDto();
    await sleep(300); // the real one is a round trip to github.com
    updateLatest = MOCK_LATEST;
    updateCheckedAt = Math.floor(Date.now() / 1000);
    return updateDto();
  },
  update_pref: ({ enabled }) => {
    updatePref = !!enabled;
    return updateDto();
  },

  data_status: () => ({
    present: dataPresent,
    // Which vendors' data is held. The mock imports the HX family; a POD Go owner would see
    // "POD Go Edit" here, and someone with both would see both.
    families: dataPresent ? ["HX Edit"] : [],
    dir: "~/.local/share/fretwire/data",
    files: dataPresent ? 21 : 0,
  }),
  import_data: async ({ source }) => {
    await sleep(600); // the real one unpacks an installer — show the spinner
    if (!source || source === "/") throw new Error(`no reference data found in ${source}`);
    dataPresent = true;
    return {
      family: "HX Edit",
      copied: 21,
      dest: "~/.local/share/fretwire/data",
      missing: [],
    };
  },
  detect: () => [{ name: DEVICES[deviceMode].name, caveat: DEVICES[deviceMode].caveat ?? null }],
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

  // ---- controller assignments ----
  // The mock models what the device does, including the parts that surprised us: op 56 *moves* a
  // bypass binding rather than adding a second, and op 37 with source 0 is the removal.
  assign_bypass: ({ slot, switch: sw }) => {
    for (const e of Object.values(current.slots)) {
      if (e?.footswitch === sw + 1) e.footswitch = 0;
    }
    const e = current.slots[slot];
    if (e) e.footswitch = sw + 1;
    return toDto(current);
  },
  unassign_bypass: ({ slot }) => {
    const e = current.slots[slot];
    if (e) e.footswitch = 0;
    return toDto(current);
  },
  // Label and colour ride the binding entries, so they apply to whatever sits on that switch —
  // and clearing keeps nothing visible, like the device's gate flip.
  set_switch_label: ({ switch: sw, label }) => {
    const e = Object.values(current.slots).find((b) => b?.footswitch === sw + 1);
    if (!e) throw new Error(`switch ${sw} has no bindings — bind a block first`);
    e.user_label = label ?? null;
    return toDto(current);
  },
  set_switch_color: ({ switch: sw, color }) => {
    const e = Object.values(current.slots).find((b) => b?.footswitch === sw + 1);
    if (!e) throw new Error(`switch ${sw} has no bindings — bind a block first`);
    e.custom_color = color ?? null;
    return toDto(current);
  },
  set_switch_momentary: ({ switch: sw, momentary }) => {
    const bound = Object.values(current.slots).filter((b) => b?.footswitch === sw + 1);
    if (!bound.length) throw new Error(`switch ${sw} has no bindings — bind a block first`);
    for (const e of bound) e.momentary = !!momentary;
    return toDto(current);
  },
  assign_param: ({ slot, paramIndex, source, paired }) => {
    // The device does not range-check this — an ordinal past the end of the table is accepted and
    // silently does nothing — so `Session::assign_param` refuses it, and so does the mock. The
    // bound is the device's own: 9 on a Stomp, 12 on an XL.
    const last = sourceTableLen(DEVICES[deviceMode].footswitches) - 1;
    if (source < 0 || source > last) {
      throw new Error(
        `controller ${source} does not exist — this device's sources run 0 (none) to ${last}`,
      );
    }
    current.assignments ??= [];
    const same = (a) =>
      a.target_slot === slot && a.param_index === paramIndex && a.paired === !!paired;
    current.assignments = current.assignments.filter((a) => !same(a));
    if (source !== 0) {
      // One source drives one thing here; the real table allows more, but the picker cannot make
      // that state, so the mock does not pretend to.
      current.assignments = current.assignments.filter((a) => a.source !== source);
      const p = findParam(slot, !!paired, paramIndex);
      current.assignments.push({
        source,
        target_slot: slot,
        param_index: paramIndex,
        paired: !!paired,
        min: p?.min ?? 0,
        max: p?.max ?? 1,
      });
    }
    return toDto(current);
  },
  set_assign_travel: ({ slot, paramIndex, max, value, paired }) => {
    const a = (current.assignments ?? []).find(
      (x) => x.target_slot === slot && x.param_index === paramIndex && x.paired === !!paired,
    );
    if (a) a[max ? "max" : "min"] = value;
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
  // Empty the preset: every block goes, and every snapshot name returns to the device's default
  // (`SNAPSHOT 1`…). Assignments live on the blocks here as they do on the pedal, so they leave
  // with them.
  clear_preset: () => {
    for (const slot of Object.keys(current.slots)) {
      if (current.slots[slot]?.kind === "effect") delete current.slots[slot];
    }
    current.snapshot_names = current.snapshot_names.map((_, i) => `SNAPSHOT ${i + 1}`);
    return toDto(current);
  },
  // Reload the preset as last saved. The mock's "flash" is the state the preset was seeded with —
  // history entry 0 — restored as an edit (so it leaves an undo entry), like the real backend.
  revert_preset: () => {
    seedHistory();
    restore(history[0].state);
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
  goto_preset: ({ bank = currentBank, preset }) => {
    const target = bankOf(bank).find((p) => p.index === preset);
    if (target) {
      current = target;
      currentBank = bank;
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
  save_preset: ({ bank = currentBank, slot, name }) => {
    savedCursor = historyCursor; // the buffer's current state is now "in flash"
    if (bank === currentBank && slot === current.index) {
      if (name) current.name = name;
    } else {
      // Save As to a different slot: deep-copy the current preset into that slot of that setlist.
      const copy = {
        ...current, index: slot, name: name || current.name,
        slots: clone(current.slots), nodePos: { ...current.nodePos },
        snapshot_names: [...current.snapshot_names],
      };
      const list = bankOf(bank);
      const at = list.findIndex((p) => p.index === slot);
      if (at >= 0) list[at] = copy;
      else list.push(copy);
      list.sort((a, b) => a.index - b.index);
    }
    return toDto(current);
  },
  rename_preset: ({ bank = currentBank, slot, name }) => {
    const target = bankOf(bank).find((p) => p.index === slot);
    if (target && name) target.name = name;
    return null;
  },
  rename_snapshot: ({ index, name }) => {
    if (index >= 0 && index < current.snapshot_names.length) current.snapshot_names[index] = name;
    return toDto(current);
  },
  list_presets: ({ bank = currentBank } = {}) =>
    bankOf(bank).map((p) => ({ index: p.index, name: p.name, label: presetLabel(p.index) })),
  // The connected device's setlist names. One entry on a Stomp, so the UI hides the picker.
  setlists: () => setlistNames().slice(),
  // Browsing setlists is ungated everywhere now; only *writing* into one the device isn't in is
  // held back on real hardware (FRETWIRE_SETLISTS=1 — see commands.rs `cross_setlist_write_enabled`).
  // The mock allows it: it can't touch a device, and the Save As path still needs to be workable in
  // the browser. Don't read the mock as proof of what ships against hardware.
  cross_setlist_write_allowed: () => true,

  // ---- export / import ------------------------------------------------------------------------
  // The real backend sweeps the device and writes a `fretwire-backup` JSON file at `path`. The mock
  // mirrors the file *shape* exactly (its "raw" is hex of the mock preset state instead of a
  // MessagePack stream) and keeps it in memory so the path-taking backup_show/restore_preset work
  // in the same session — a browser can't read arbitrary paths. The `_inline` variants below are
  // what the UI actually uses in a browser: the file goes out as a download and comes back as
  // text. Walks whichever setlists it is given, as the real sweep does.
  export_setlists: async ({ path, banks }) => {
    await sweepBackup(banks);
    console.info(`[fretwire mock] would write ${path}`);
    return lastBackup.presets.length;
  },
  export_setlists_inline: async ({ banks }) => {
    await sweepBackup(banks);
    return { count: lastBackup.presets.length, json: JSON.stringify(lastBackup, null, 2) };
  },
  cancel_export: () => {
    exportCancelled = true;
  },
  backup_show: ({ path }) => {
    if (!lastBackup)
      throw new Error(`mock backend: nothing exported this session — run Export… first (a browser can't read ${path})`);
    return backupEntries(lastBackup);
  },
  backup_show_inline: ({ json }) => backupEntries(parseBackup(json)),
  restore_preset: ({ index, slot, bank = 0 }) => {
    if (!lastBackup) throw new Error("mock backend: nothing exported this session — run Export… first");
    return restoreFrom(lastBackup, index, slot, bank);
  },
  restore_preset_inline: ({ json, index, slot, bank = 0 }) =>
    restoreFrom(parseBackup(json), index, slot, bank),
  // ---- whole-device backup / restore ----
  backup_device: async ({ path, banks, irs, settings }) => {
    await sweepBackup(banks, { irs, settings });
    console.info(`[fretwire mock] would write ${path}`);
    return { presets: lastBackup.presets.length, irs: lastBackup.irs.length, settings: lastBackup.settings.length };
  },
  backup_device_inline: async ({ banks, irs, settings }) => {
    await sweepBackup(banks, { irs, settings });
    return {
      count: lastBackup.presets.length, irs: lastBackup.irs.length, settings: lastBackup.settings.length,
      json: JSON.stringify(lastBackup, null, 2),
    };
  },
  backup_info: ({ path }) => {
    if (!lastBackup)
      throw new Error(`mock backend: nothing backed up this session — run Back up device… first (a browser can't read ${path})`);
    return backupInfo(lastBackup);
  },
  backup_info_inline: ({ json }) => backupInfo(parseBackup(json)),
  restore_device: (args) => {
    if (!lastBackup) throw new Error("mock backend: nothing backed up this session — run Back up device… first");
    return restoreDevice(lastBackup, args);
  },
  restore_device_inline: ({ json, ...args }) => restoreDevice(parseBackup(json), args),
  split_types: () => clone(SPLIT_TYPES),
  categories: () => {
    const ids = [...new Set(CATALOG.map((m) => m.category))].sort((a, b) => a - b);
    ids.push(100); // synthetic Amp+Cab
    return ids
      .map((id) => ({
        id,
        name: CATEGORY_NAMES[id] ?? `Cat ${id}`,
        // The real backend reads these out of the user's own HX_ModelCatalog.json and returns null
        // when the data was never imported. The mock has no such file, so it answers null and
        // exercises the fallback path — the one a fresh install actually takes.
        color: null,
      }))
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
    /**
     * Turn a parameter with the pedal's own knob. `extra` and `paired` select the index space:
     * with both `false` (the default) `param` indexes the model's own param list, `extra` means
     * the block's extra values, `paired` means its cab/IR's list. All three start at 0.
     *
     * `fretwireMock.knob(2, 0, 1, true)` is Trails on, which must **not** move the model's param 0
     * (issue #5); `fretwireMock.knob(6, 2, 3, false, true)` is a cab's Distance, which must **not**
     * move the amp's Mid (issue #11).
     */
    knob(slot, param, value, extra = false, paired = false) {
      const list = paired
        ? (current.slots[slot]?.paired_params ?? [])
        : (current.slots[slot]?.params ?? []);
      const p = list.find((q) =>
        extra ? q.extra_index === param : q.extra_index == null && q.index === param,
      );
      if (p) p.value = value;
      emit("device-pushes", [{ kind: "Param", slot, param, value, extra, paired }]);
    },
    /** Switch snapshot as if from the panel. */
    snapshot(index) {
      current.active_snapshot = index;
      emit("device-pushes", [{ kind: "Snapshot", index }]);
    },
    /** Switch preset as if from the panel, optionally in another setlist. */
    preset(index, bank = currentBank) {
      const target = bankOf(bank).find((p) => p.index === index);
      if (target) {
        current = target;
        currentBank = bank;
        clearHistory(); // like the heartbeat clearing on a panel preset push
      }
      emit("device-pushes", [{ kind: "Preset", index }]);
    },
    /**
     * Pretend to be another unit: "stomp" (one DSP, one flat preset list, no setlist picker),
     * "xl" (as the Stomp, but banked by four and carrying a support caveat) or "floor" (two DSPs,
     * eight setlists). Reload or reconnect after switching.
     */
    device(mode) {
      if (mode === undefined) {
        console.info(
          `[fretwire] mock device: ${DEVICES[deviceMode].name} (${deviceMode}). ` +
            `Switch with fretwireMock.device("stomp"), ("xl") or ("floor").`,
        );
        return deviceMode;
      }
      if (!DEVICES[mode]) {
        console.warn(`[fretwire] unknown device ${mode} — use "stomp", "xl" or "floor"`);
        return deviceMode;
      }
      deviceMode = mode;
      storage?.setItem(MODE_KEY, mode); // remembered across reloads
      banks = buildBanks();
      currentBank = 0;
      current = bankOf(0)[0];
      clearHistory();
      console.info(
        `[fretwire] mock is now a ${DEVICES[mode].name} — reload the page (the setting sticks).`,
      );
      return mode;
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
