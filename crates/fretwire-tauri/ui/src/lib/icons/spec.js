// Resolves a model to an icon spec.
//
// Four steps, most specific first:
//   1. the curated per-model table (`models.js`) — the pedal actually looks like that thing;
//   2. amp rules by symbolic-id prefix, and cabs derived from the driver array in the name;
//   3. effect family by keyword — every chorus that isn't listed still gets a chorus icon;
//   4. the category icon, so nothing is ever blank.
//
// Category ids are the device's effect-type enum from `fretwire_core::editor::category_name`, not
// `HX_ModelCatalog.json`'s (they disagree — see the note on `canonical_category`).

import { C, PANEL, CLOTH } from "./palette.js";
import { MODELS, AMP_RULES, CAB_FINISH } from "./models.js";

export const CAT = {
  AMP: 1,
  CAB: 2,
  DISTORTION: 3,
  DYNAMICS: 4,
  FILTER: 6,
  PITCH: 7,
  MODULATION: 8,
  DELAY: 9,
  REVERB: 10,
  WAH: 11,
  SEND_RETURN: 12,
  PREAMP: 13,
  EQ: 14,
  LOOPER: 15,
  IR: 16,
  VOLUME_PAN: 17,
  CAB_MIC_IR: 19,
  // `HX_ModelCatalog.json`'s Favorites category — not a device effect type, but the picker's
  // Favorites list and a block that *is* a favorite draw the star, as the pedal's screen does.
  FAVORITES: 23,
  AMP_CAB: 100,
};

/** One icon per category — the picker's category list, and the fallback for anything unmatched. */
export const CATEGORY_SPECS = {
  [CAT.AMP]: { shape: "head", body: C.black, panel: PANEL.plexi, knobs: 6, cloth: CLOTH.basketweave },
  [CAT.PREAMP]: { shape: "rack", body: C.charcoal, knobs: 5 },
  [CAT.CAB]: { shape: "cab", body: C.espresso, speakers: [2, 2], cloth: CLOTH.basketweave },
  [CAT.CAB_MIC_IR]: { shape: "cab", body: C.espresso, speakers: [2, 2], cloth: CLOTH.basketweave, mic: true },
  [CAT.IR]: { shape: "rack", body: C.graphite, knobs: 3, glyph: "wave" },
  [CAT.AMP_CAB]: { shape: "head", body: C.black, panel: PANEL.plexi, knobs: 6, cloth: CLOTH.basketweave, stack: true },
  [CAT.DISTORTION]: { shape: "stomp", body: C.orange, knobs: 3 },
  [CAT.DYNAMICS]: { shape: "stomp", body: C.red, knobs: 2 },
  [CAT.EQ]: { shape: "rack", body: C.black, sliders: 5 },
  [CAT.MODULATION]: { shape: "stomp", body: C.aqua, knobs: 3 },
  [CAT.DELAY]: { shape: "stomp", body: C.green, knobs: 4, layout: "grid", mark: "window" },
  [CAT.REVERB]: { shape: "spring", body: C.charcoal },
  [CAT.PITCH]: { shape: "stomp", body: C.purple, knobs: 3, mark: "window" },
  [CAT.FILTER]: { shape: "stompWide", body: C.steel, knobs: 3 },
  [CAT.WAH]: { shape: "wah", body: C.black },
  [CAT.VOLUME_PAN]: { shape: "pedalboard", body: C.graphite },
  [CAT.SEND_RETURN]: { shape: "util", jacks: 2, body: C.teal, arrow: true },
  [CAT.LOOPER]: { shape: "looper", body: C.graphite, sw: 2 },
  [CAT.FAVORITES]: { shape: "star", body: "#e8b93c" },
};

const GENERIC = { shape: "stomp", body: C.graphite, knobs: 3 };

/**
 * Effect families, matched against the symbolic id. First hit wins, so order matters: the specific
 * patterns (tape echo, spring reverb) sit above the general ones (delay, reverb).
 */
const FAMILIES = [
  // Drive
  [/fuzz/i, { shape: "stompWide", body: C.silver, knobs: 3 }],
  [/boost/i, { shape: "stompNarrow", body: C.gold, knobs: 1 }],
  [/dist/i, { shape: "stomp", body: C.orange, knobs: 3 }],
  [/(^|_)(od|overdrive|drive|screamer)/i, { shape: "stomp", body: C.green, knobs: 3 }],
  [/bassdi|\bdi\b/i, { shape: "stompWide", body: C.black, knobs: 5, layout: "grid" }],
  // Dynamics
  [/gate/i, { shape: "stomp", body: C.charcoal, knobs: 2, led: "#7cffb0" }],
  [/comp/i, { shape: "stomp", body: C.red, knobs: 2 }],
  // EQ
  [/graphic|caliq/i, { shape: "rack", body: C.black, sliders: 5 }],
  [/^HD2_EQ/i, { shape: "rack", body: C.charcoal, knobs: 4 }],
  // Modulation
  [/rotary|leslie/i, { shape: "rotary", body: C.espresso }],
  [/flanger|flange/i, { shape: "stompWide", body: C.steel, knobs: 4 }],
  [/phaser|phase|vibe/i, { shape: "stomp", body: C.orange, knobs: 2 }],
  [/chorus/i, { shape: "stomp", body: C.aqua, knobs: 3 }],
  [/trem/i, { shape: "stomp", body: C.blue, knobs: 3 }],
  [/vibrato/i, { shape: "stomp", body: C.blue, knobs: 3 }],
  [/ringmod|ring/i, { shape: "stomp", body: C.charcoal, knobs: 3, mark: "window" }],
  [/panner|pan/i, { shape: "util", jacks: 2, body: C.teal, arrow: true }],
  // Delay
  [/tape|reel|echoplat|multihead/i, { shape: "reel", body: C.charcoal, knobs: 3 }],
  [/^HD2_DL4/i, { shape: "stompWide", body: C.green, knobs: 4, sw: 4 }],
  [/analog|bucket|bbd/i, { shape: "stomp", body: C.blue, knobs: 3 }],
  [/delay|echo/i, { shape: "stomp", body: C.graphite, knobs: 4, layout: "grid", mark: "window" }],
  // Reverb
  [/spring/i, { shape: "spring", body: C.charcoal }],
  [/plate/i, { shape: "rack", body: C.steel, knobs: 3, glyph: "plate" }],
  [/hall|chamber|room|cave|tile/i, { shape: "rack", body: C.charcoal, knobs: 3, glyph: "room" }],
  [/reverb|verb/i, { shape: "rack", body: C.navy, knobs: 3, glyph: "arch" }],
  // Pitch / synth
  [/wham/i, { shape: "wah", body: C.red }],
  [/synth|osc|generator/i, { shape: "rack", body: C.black, knobs: 6, layout: "grid" }],
  [/octav|harmon|pitch|string|capo|detune/i, { shape: "stomp", body: C.purple, knobs: 3, mark: "window" }],
  // Filter / wah
  [/wah/i, { shape: "wah", body: C.black }],
  [/filter|tron|^HD2_FM4/i, { shape: "stompWide", body: C.steel, knobs: 3 }],
  // Utility
  [/looper/i, { shape: "looper", body: C.graphite, sw: 2 }],
  [/send|return|fxloop/i, { shape: "util", jacks: 2, body: C.teal, arrow: true }],
  [/vol|gain/i, { shape: "pedalboard", body: C.graphite }],
  [/^HD2_CabMicIr/i, { shape: "cab", body: C.espresso, speakers: [2, 2], cloth: CLOTH.basketweave, mic: true }],
  [/^HD2_Cab/i, { shape: "cab", body: C.espresso, speakers: [2, 2], cloth: CLOTH.basketweave }],
  [/^HD2_Preamp/i, { shape: "rack", body: C.charcoal, knobs: 5 }],
  [/^HD2_Amp/i, { shape: "head", body: C.black, panel: PANEL.chrome, knobs: 6, cloth: CLOTH.black }],
];

// Amp rules are matched longest-prefix-first so `HD2_AmpMandarinBass200` beats `HD2_AmpMandarin`.
const AMP_SORTED = [...AMP_RULES].sort((a, b) => b[0].length - a[0].length);

/** "4x12 Greenback 25" → `[4, 12]`; the display name is the only place the array is recorded. */
function driverArray(name) {
  const m = /(\d+)\s*x\s*(\d+)/i.exec(name ?? "");
  if (!m) return null;
  return [Number(m[1]), Number(m[2])];
}

/** Lay `n` drivers out the way the cabinet does: 4x12 is 2x2, 8x10 is 4x2, 1x12 is 1x1. */
function driverGrid(count) {
  switch (count) {
    case 1:
      return [1, 1];
    case 2:
      return [2, 1];
    case 4:
      return [2, 2];
    case 6:
      return [3, 2];
    case 8:
      return [4, 2];
    default:
      return [Math.min(count, 4), 1];
  }
}

function cabSpec(name) {
  const arr = driverArray(name);
  const finish = CAB_FINISH.find(([re]) => re.test(name ?? ""))?.[1] ?? { body: C.espresso, cloth: CLOTH.basketweave };
  return { shape: "cab", ...finish, speakers: driverGrid(arr ? arr[0] : 1) };
}

/**
 * The icon spec for a model. `symbolicId` is the `Helix.sym` base symbol (blocks and picker rows
 * both carry it); `name` is the display name (only cabs need it, for the driver array); `category`
 * is the device effect-type id.
 */
export function iconSpec(symbolicId, category = null, name = "") {
  const id = symbolicId ?? "";
  // A favorite is the star whatever its model — the category wins over the symbol here alone.
  if (category === CAT.FAVORITES) return CATEGORY_SPECS[CAT.FAVORITES];
  if (MODELS[id]) return MODELS[id];

  if (category === CAT.CAB || category === CAT.CAB_MIC_IR || /^HD2_Cab/i.test(id)) {
    const spec = cabSpec(name || id);
    return category === CAT.CAB_MIC_IR || /MicIr/i.test(id) ? { ...spec, mic: true } : spec;
  }
  if (category === CAT.AMP || category === CAT.PREAMP || category === CAT.AMP_CAB || /^HD2_(Amp|Preamp)/i.test(id)) {
    // Preamps mirror the amp symbols (`HD2_PreampBritPlexi` ↔ `HD2_AmpBritPlexi`), so normalise
    // before matching and they inherit the right finish.
    const ampId = id.replace(/^HD2_Preamp/, "HD2_Amp");
    const hit = AMP_SORTED.find(([prefix]) => ampId.startsWith(prefix));
    if (hit) {
      const spec = hit[1];
      // A preamp is the same amp without the power section — draw it as a rack unit so the two are
      // never confused in the chain.
      if (category === CAT.PREAMP || /^HD2_Preamp/.test(id))
        return { shape: "rack", body: spec.body, knobs: spec.knobs ?? 5, panel: spec.panel };
      return category === CAT.AMP_CAB ? { ...spec, stack: true } : spec;
    }
  }
  const fam = FAMILIES.find(([re]) => re.test(id));
  if (fam) return fam[1];

  return CATEGORY_SPECS[category] ?? GENERIC;
}

/** The icon for a category itself (the picker's category list). */
export function categorySpec(category) {
  return CATEGORY_SPECS[category] ?? GENERIC;
}
