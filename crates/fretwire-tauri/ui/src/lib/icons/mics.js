// Microphone silhouettes for the cab mic view.
//
// Same rules as the rest of `icons/`: nothing here is Line 6's artwork. The reference data names a
// mic only as a string ("57 Dynamic", "4038 Ribbon") — see the `mic` / `cabMICir` discrete controls
// in `HelixControls.json` — and this table turns that string into proportions and a finish. No
// logos, no lettering, no panel graphics: a mic is a body, a head and a band.
//
// Every spec is drawn **pointing +x with its capsule at the origin**, so the body runs back along
// -x. The view rotates the whole thing about that origin to apply the Angle parameter, which is
// what makes the capsule — not the body — stay put as the mic tilts.

import { C } from "./palette.js";

/**
 * Head shapes, which is most of what tells the families apart on screen:
 * - `ball`     a rounded grille wider than the body (57, 7, kick mics)
 * - `barrel`   a grille the width of the body, squared off (421, RE-style)
 * - `capsule`  a tall rounded head on a narrow body (ribbons)
 * - `slab`     a flat rectangular side-address head (906, 414, 87)
 * - `bottle`   a large sphere on a shoulder (47/67-style condensers)
 */

const DYNAMIC = {
  family: "Dynamic",
  bodyLen: 30,
  bodyR: 6,
  head: "ball",
  headLen: 10,
  headR: 7.5,
  body: C.charcoal,
  accent: C.steel,
};

const RIBBON = {
  family: "Ribbon",
  bodyLen: 26,
  bodyR: 7,
  head: "capsule",
  headLen: 13,
  headR: 9.5,
  body: C.graphite,
  accent: C.gold,
};

const CONDENSER = {
  family: "Condenser",
  bodyLen: 22,
  bodyR: 5.5,
  head: "slab",
  headLen: 16,
  headR: 11,
  body: C.silver,
  accent: C.chrome,
};

/**
 * Per-mic proportions, keyed by the number the label starts with — the one part of a mic's name
 * that is stable across the two mic lists (the legacy `mic` control names sixteen, the modern
 * `cabMICir` twelve, and they overlap but do not agree on order).
 *
 * The shapes are the recognisable outline of that *class* of microphone: a long thin barrel for a
 * broadcast dynamic, a lollipop for a wide ribbon, a slab for a side-address condenser. They are
 * schematic on purpose — enough to tell at a glance that the mic changed and roughly what kind it
 * is now, which is the whole job of this drawing.
 */
const MICS = {
  // ---- dynamics ----
  57: { ...DYNAMIC, bodyLen: 30, headLen: 10, headR: 7.5 },
  409: { ...DYNAMIC, head: "slab", bodyLen: 12, headLen: 17, headR: 8, body: C.black },
  421: { ...DYNAMIC, head: "barrel", bodyLen: 36, headLen: 11, headR: 8 },
  906: { ...DYNAMIC, head: "slab", bodyLen: 9, headLen: 20, headR: 7, body: C.black },
  30: { ...DYNAMIC, head: "barrel", bodyLen: 40, headLen: 9, headR: 7 },
  20: { ...DYNAMIC, head: "barrel", bodyLen: 42, headLen: 9, headR: 7.5 },
  7: { ...DYNAMIC, bodyLen: 24, headLen: 16, headR: 10, accent: C.black },
  112: { ...DYNAMIC, bodyLen: 18, headLen: 16, headR: 11 },
  12: { ...DYNAMIC, bodyLen: 21, headLen: 15, headR: 10.5 },
  // ---- ribbons ----
  121: { ...RIBBON, bodyLen: 26, headLen: 13, headR: 9.5 },
  160: { ...RIBBON, bodyLen: 30, headLen: 10, headR: 7.5 },
  4038: { ...RIBBON, bodyLen: 14, headLen: 20, headR: 12 },
  84: { ...RIBBON, bodyLen: 24, headLen: 11, headR: 8 },
  // ---- condensers ----
  414: { ...CONDENSER, head: "slab", headLen: 16, headR: 11 },
  87: { ...CONDENSER, head: "slab", headLen: 17, headR: 10.5 },
  47: { ...CONDENSER, head: "bottle", headLen: 20, headR: 11.5, body: C.steel },
  67: { ...CONDENSER, head: "bottle", headLen: 19, headR: 11, body: C.steel },
};

/** The family a label ends in, for anything the table doesn't name. */
function familyOf(label) {
  if (/ribbon/i.test(label)) return RIBBON;
  if (/cond/i.test(label)) return CONDENSER;
  return DYNAMIC;
}

/**
 * The spec for a mic label ("57 Dynamic"). Unknown mics — a firmware update adding one — fall back
 * to their family's shape rather than drawing nothing, exactly as `iconSpec` falls back to the
 * effect family. `null` only when there is no label at all to go on.
 */
export function micSpec(label) {
  if (!label) return null;
  const n = Number(String(label).match(/\d+/)?.[0]);
  return { label, ...(MICS[n] ?? familyOf(label)) };
}

/** Overall length of a spec, capsule to tail — the view needs it to keep the mic on the canvas. */
export function micLength(spec) {
  return spec ? spec.headLen + spec.bodyLen + 4 : 0;
}
