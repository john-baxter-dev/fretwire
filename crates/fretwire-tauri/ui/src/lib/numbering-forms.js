// The two preset-numbering forms, and how they map onto Global Setting 27.
//
// Split out of `numbering.svelte.js` so plain node can import it: the store there is a `$state`
// rune and needs the Svelte compiler, and the one thing here that must not be got wrong — which
// way round the flag goes — is worth a test that runs without one.
//
// **Polarity.** Setting 27 is a flag whose `on` label is the flat range and whose `off` label is
// the banked one (`fretwire-protocol/src/settings.rs`), so `true` means the *flat* form. The Rust
// side turns the same bool into the same two words in `commands.rs::numbering_word`. Getting this
// backwards would not fail anywhere — it would quietly set the user's pedal to the other form.
//
// Don't quote the labels here: they are **device-specific** and derived per unit from its preset
// count (`Device::preset_numbering_labels`) — a Stomp draws `000-125`/`01A-42C` where an XL draws
// `000-127`/`01A-32D`. Only the polarity is fixed, and only the polarity is what this file is for.

export const BANKED = "banked";
export const FLAT = "flat";

/** Is `v` one of the two words the UI and `device_numbering` agree on? */
export const isForm = (v) => v === FLAT || v === BANKED;

/** The form that is *not* this one — what a toggle switches to. */
export const other = (mode) => (mode === BANKED ? FLAT : BANKED);

/** Setting 27's value for a form. The flag reads "flat numbering on". */
export const flagFor = (mode) => (mode === FLAT ? 1 : 0);

/** ...and back: the form a read-back of setting 27 means. */
export const formForFlag = (flag) => (flag ? FLAT : BANKED);
