// How preset slots are written in the UI: the pedal's banked form (`01A`, `01B`, `01C`, `02A`) or
// the flat slot number (`000`, `001`, …).
//
// Both are correct — which one the *hardware* shows is a Global Setting on the device, so there is
// no single right answer to bake in. We can't read that setting off the wire either: it lives
// behind op-25 (globals), which isn't decoded yet. So the user tells us, and this remembers it.
// If op-25 ever lands, the honest default becomes "whatever the pedal says" and this stays as the
// override. See `Device::presets_per_bank`.

const KEY = "fretwire.presetNumbering";
const BANKED = "banked";
const FLAT = "flat";

function load() {
  // Storage throws outright in some webview configurations rather than returning null, and a
  // preference is never worth failing to start over.
  try {
    const v = localStorage.getItem(KEY);
    return v === FLAT || v === BANKED ? v : BANKED;
  } catch {
    return BANKED;
  }
}

/** The live preference. Read `numbering.mode`; write through `setNumbering`. */
export const numbering = $state({ mode: load() });

export function setNumbering(mode) {
  numbering.mode = mode === FLAT ? FLAT : BANKED;
  try {
    localStorage.setItem(KEY, numbering.mode);
  } catch {
    // Not persisting is a smaller problem than not running.
  }
}

export function toggleNumbering() {
  setNumbering(numbering.mode === BANKED ? FLAT : BANKED);
}

export const isBanked = () => numbering.mode === BANKED;

/**
 * The slot column for a preset row — `p` is a listing entry with `index` and (where the backend
 * knows this device's banking) `label`.
 *
 * The flat form is always available, so it is the fallback in both directions: a device whose
 * banking we haven't seen has no `label` to show even in banked mode.
 */
export function slotLabel(p) {
  const flat = String(p?.index ?? 0).padStart(3, "0");
  return numbering.mode === FLAT ? flat : (p?.label ?? flat);
}

/** Whether this listing can offer the banked form at all — i.e. whether the toggle is meaningful. */
export function canBank(presets) {
  return presets?.some((p) => p?.label != null) ?? false;
}
