// How preset slots are written in the UI: the pedal's banked form (`01A`, `01B`, `01C`, `02A`) or
// the flat slot number (`000`, `001`, …).
//
// **This is the pedal's setting, not ours.** Which form the hardware shows is Global Setting id 27
// (`true` = flat, `false` = banked — see `docs/protocol.md`), and as of 2026-08-22 we can both read
// and write it. So the sidebar's "Number presets" item is a view of that setting: it shows what the
// pedal is set to, and picking the other form changes the pedal. Nothing to keep in sync, because
// there is only one value — the same one the Globals panel's "Preset numbering" row edits.
//
// The local preference below is the fallback for the case where that isn't true: a device that
// doesn't answer id 27 (any model we haven't measured), or no device at all. There the toggle can
// only be cosmetic, so it is, and the choice is remembered.

import { BANKED, FLAT, isForm, other, flagFor, formForFlag } from "./numbering-forms.js";

const KEY = "fretwire.presetNumbering";

function load() {
  // Storage throws outright in some webview configurations rather than returning null, and a
  // preference is never worth failing to start over.
  try {
    const v = localStorage.getItem(KEY);
    return isForm(v) ? v : null;
  } catch {
    return null;
  }
}

/**
 * The live form. Read `numbering.mode`; write through `setNumbering` (local) or
 * `applyDeviceNumbering` (what the pedal reported).
 *
 * `deviceBacked` is what the sidebar checks to decide whether toggling should write to the pedal.
 * It is set only by a device that actually answered id 27, and cleared on disconnect, so it is
 * never true while there is nothing to write to.
 */
export const numbering = $state({ mode: load() ?? BANKED, deviceBacked: false });

/** Remember a form locally. Only reached when the pedal has no say — see the header. */
export function setNumbering(mode) {
  numbering.mode = mode === FLAT ? FLAT : BANKED;
  try {
    localStorage.setItem(KEY, numbering.mode);
  } catch {
    // Not persisting is a smaller problem than not running.
  }
}

/**
 * Take the form the pedal reports. The device is the authority whenever it has one, so unlike the
 * old default-only adoption this always wins — that asymmetry is what used to let a local toggle
 * silently outrank the Globals panel's own "Preset numbering" row.
 *
 * `mode` is whatever `device_numbering` returned, including `null` for a device that doesn't answer
 * id 27. Anything unrecognised leaves the current mode alone and leaves `deviceBacked` false,
 * because the whole point here is to stop guessing.
 */
export function applyDeviceNumbering(mode) {
  if (!isForm(mode)) return false;
  numbering.mode = mode;
  numbering.deviceBacked = true;
  return true;
}

/** Back to a local-only preference — the pedal is gone, so it can no longer be the authority. */
export function forgetDeviceNumbering() {
  numbering.deviceBacked = false;
  const stored = load();
  if (stored) numbering.mode = stored;
}

/** The form that is *not* the current one — what a toggle would switch to. */
export const otherMode = () => other(numbering.mode);

export { flagFor as numberingFlag, formForFlag };

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
