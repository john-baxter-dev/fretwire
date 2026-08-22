// How preset slots are written in the UI: the pedal's banked form (`01A`, `01B`, `01C`, `02A`) or
// the flat slot number (`000`, `001`, …).
//
// Both are correct — which one the *hardware* shows is a Global Setting on the device, so there is
// no single right answer to bake in. As of 2026-08-22 we can read it: setting id 27, `true` for the
// flat form and `false` for the banked one (`docs/protocol.md`). So the pedal supplies the default
// and the user's own toggle overrides it — a preference is only stored once someone sets one, which
// is what keeps "the pedal says flat" from being permanently overruled by a default nobody chose.
// See `Device::presets_per_bank`.

const KEY = "fretwire.presetNumbering";
const BANKED = "banked";
const FLAT = "flat";

function load() {
  // Storage throws outright in some webview configurations rather than returning null, and a
  // preference is never worth failing to start over. `null` means "nobody has chosen", which is
  // distinct from having chosen the banked form — only the former defers to the device.
  try {
    const v = localStorage.getItem(KEY);
    return v === FLAT || v === BANKED ? v : null;
  } catch {
    return null;
  }
}

const chosen = load();

/** The live preference. Read `numbering.mode`; write through `setNumbering`. */
export const numbering = $state({ mode: chosen ?? BANKED, explicit: chosen != null });

export function setNumbering(mode) {
  numbering.mode = mode === FLAT ? FLAT : BANKED;
  numbering.explicit = true;
  try {
    localStorage.setItem(KEY, numbering.mode);
  } catch {
    // Not persisting is a smaller problem than not running.
  }
}

/**
 * Adopt what the pedal itself is set to, as the *default* only — a user who has picked a form keeps
 * it, and this never writes to storage, so the device stays the source of truth on every start.
 *
 * `mode` is whatever `device_numbering` returned, including `null` for a device that doesn't answer
 * setting 27. Anything unrecognised leaves the current mode alone rather than falling back to a
 * guess, because the whole point here is to stop guessing.
 */
export function adoptDeviceNumbering(mode) {
  if (numbering.explicit) return false;
  if (mode !== FLAT && mode !== BANKED) return false;
  numbering.mode = mode;
  return true;
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
