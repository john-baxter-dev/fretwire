//! The device-settings namespace — what each id *is*, where we know.
//!
//! Settings are a flat numbered space read with op 24 and written with op 25 (see
//! `docs/protocol.md`). Nothing here is documented by Line 6; every entry was read off a physical
//! pedal by changing one thing on its own menus and diffing two dumps — an HX Stomp for most of
//! them, an HX Stomp XL for those marked `[XL]` below.
//!
//! **166 of ids 0..=600 answer on an HX Stomp. 54 are identified; 53 of them are here.** The odd
//! one out is id 28, the current preset index — device state rather than a preference, written
//! properly by `Session::goto_preset`, and deliberately not offered as a settings row.
//!
//! Those two numbers are not a coverage fraction of each other. Nineteen of the identified ids came
//! off an XL and have never been read on a Stomp, and a three-switch pedal has no `FS7 Function` to
//! report — which of them refuse there is unchecked, and a refusal is absence, not an error.
//!
//! That ratio is the normal state of this table, not a gap to be filled in with plausible guesses:
//! an id whose meaning nobody has observed is simply absent, and the UI shows it as a raw number
//! rather than inventing a label. Adding one costs about thirty seconds with
//! `fretwire settings-diff`.

/// How a setting's value should be presented and edited.
///
/// The split between [`Kind::Flag`] and [`Kind::Choice`] is **the type the wire holds**, not a
/// style preference — `false`/`true` in a dump is a flag, `0`/`1`/`2` is a choice. A two-option
/// `Choice` sitting beside a `Flag` (117 and 135 against 10, 25, 26 and 129) is therefore a
/// recorded difference between those ids, not an inconsistency to tidy away. Id 154 was declared a
/// `Flag` here until an XL owner read it back as `1 [int]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A `bool`. The two labels are what the pedal's own menu calls the states, in `(true, false)`
    /// order — the wire has no notion of which one is "on".
    Flag { on: &'static str, off: &'static str },
    /// An `int` from a fixed set. Values not listed have never been observed; they display as the
    /// bare number rather than being clamped to a neighbour.
    Choice(&'static [(i64, &'static str)]),
    /// A number, in `unit`. `off` is the sentinel the device uses for "off" where it has one — the
    /// global EQ cuts park at 19.9 Hz and 20100 Hz rather than carrying a separate enable.
    Number {
        unit: &'static str,
        off: Option<f64>,
    },
}

/// One identified setting.
#[derive(Debug, Clone, Copy)]
pub struct Setting {
    pub id: i64,
    /// Short label, as the pedal's menus name it.
    pub name: &'static str,
    /// Menu grouping, used to section a settings panel.
    pub group: &'static str,
    pub kind: Kind,
}

/// Every setting id we have identified.
///
/// **Read off a physical HX Stomp, 2026-08-22**, except those marked `[XL]`, which an HX Stomp
/// XL owner read off that pedal the same way and contributed — Ins/Outs and Preferences on
/// [2026-08-23], then all of Footswitches, EXP Pedals and Displays on [2026-08-25]. Where the two units'
/// menus name a thing differently the XL's wording is used, because it is the one we have in
/// writing; an id the XL has and the Stomp does not simply refuses on a Stomp, and `scan_settings`
/// treats a refusal as absence rather than as an error.
///
/// **Ordered by id**, which is how the table is searched — presentation order lives in
/// [`MENU_ORDER`], so that finding id 154 here never needs a text search. Ids not here answer but
/// are unidentified — see the module note.
pub const SETTINGS: &[Setting] = &[
    Setting {
        id: 2,
        // [XL] The XL lists the loop's two sides separately. Whether both ids answer on a plain
        // Stomp has not been checked; if one doesn't, it is absent from the panel and that is all.
        name: "Send/Return L",
        group: "Ins/Outs",
        kind: Kind::Flag {
            on: "Line",
            off: "Instrument",
        },
    },
    Setting {
        id: 3,
        // [XL]
        name: "Send/Return R",
        group: "Ins/Outs",
        kind: Kind::Flag {
            on: "Line",
            off: "Instrument",
        },
    },
    Setting {
        id: 9,
        name: "MIDI Base Channel",
        group: "MIDI/Tempo",
        // Zero-based on the wire: the pedal's channel 4 reads back as 3. Presented one-based, since
        // that is what the pedal's screen and every other MIDI device call it.
        kind: Kind::Choice(&[
            (0, "1"),
            (1, "2"),
            (2, "3"),
            (3, "4"),
            (4, "5"),
            (5, "6"),
            (6, "7"),
            (7, "8"),
            (8, "9"),
            (9, "10"),
            (10, "11"),
            (11, "12"),
            (12, "13"),
            (13, "14"),
            (14, "15"),
            (15, "16"),
        ]),
    },
    Setting {
        id: 10,
        // [XL]
        name: "MIDI Thru",
        group: "MIDI/Tempo",
        kind: Kind::Flag {
            on: "On",
            off: "Off",
        },
    },
    Setting {
        id: 11,
        name: "USB MIDI",
        group: "MIDI/Tempo",
        kind: Kind::Flag {
            on: "On",
            off: "Off",
        },
    },
    Setting {
        id: 12,
        // [XL]
        name: "MIDI PC Rx",
        group: "MIDI/Tempo",
        kind: Kind::Choice(&[(0, "Off"), (1, "MIDI"), (2, "USB"), (3, "MIDI+USB")]),
    },
    Setting {
        id: 13,
        // [XL]
        name: "Rx MIDI Clock",
        group: "MIDI/Tempo",
        kind: Kind::Choice(&[(0, "Off"), (1, "MIDI"), (2, "USB"), (3, "Auto")]),
    },
    Setting {
        id: 14,
        name: "Tempo Select",
        group: "MIDI/Tempo",
        kind: Kind::Choice(&[(0, "Snapsht"), (1, "Preset"), (2, "Global")]),
    },
    Setting {
        id: 16,
        name: "BPM",
        group: "MIDI/Tempo",
        kind: Kind::Number {
            unit: "BPM",
            off: None,
        },
    },
    Setting {
        id: 17,
        // [XL]
        name: "Stomp Select",
        group: "Footswitches",
        kind: Kind::Choice(&[(0, "Off"), (1, "Touch"), (2, "Press"), (3, "Both")]),
    },
    Setting {
        id: 18,
        // [XL]
        name: "Preset Mode",
        group: "Footswitches",
        kind: Kind::Choice(&[(0, "Moment"), (1, "Latch")]),
    },
    Setting {
        id: 19,
        // [XL]
        name: "Stomp Mode",
        group: "Footswitches",
        kind: Kind::Choice(&[(0, "4 Swtch"), (1, "6 Swtch")]),
    },
    Setting {
        id: 20,
        // [XL]
        name: "Up/Down Switches",
        group: "Footswitches",
        kind: Kind::Choice(&[(0, "Banks"), (1, "Preset"), (2, "Snapsht")]),
    },
    Setting {
        id: 25,
        // [XL]
        name: "LED Rings",
        group: "Displays",
        kind: Kind::Flag {
            on: "Dim/Brt",
            off: "Off/Brt",
        },
    },
    Setting {
        id: 26,
        // [XL]
        name: "Tap LED",
        group: "Displays",
        kind: Kind::Flag {
            on: "On",
            off: "Off",
        },
    },
    Setting {
        id: 27,
        // **These labels are the XL's, and they are wrong on a plain Stomp.** The flat form spells
        // out the range, so it differs with the preset count: an HX Stomp shows `000-125` and
        // `01A-42C` (126 presets, 42 banks of three), an XL `01A-32D` (128, 32 banks of four).
        // `Setting` has no notion of the device, so what stands here is a **fallback**:
        // `Device::preset_numbering_labels` derives the right pair from each unit's measured counts
        // and the DTO substitutes it, so a connected Stomp or XL is labelled from its own screen.
        // These are what a device with no measured bank size gets — today the Floor and the LT.
        // [Stomp: owner, 2026-08-24. XL: owner, 2026-08-23]
        //
        // **The XL's screen says `000-128` and we deliberately do not repeat it.** That is a
        // firmware bug [confirmed by the owner, 2026-08-24]: the menu draws `000-128` while
        // scrolling that unit's presets stops at 127. The Stomp's `000-125` is the correct max
        // index for its 126, so Line 6 got this right on one unit and wrong on the other.
        //
        // This is the one place the "match what the pedal shows" rule is knowingly broken, and it is
        // not the same case as `Authentc` — that is the pedal's own spelling of a real word, while
        // this is a range that is simply wrong. Repeating it would mean the editor telling an XL
        // owner they have a preset 128, which they do not. `preset_numbering_labels` derives
        // `000-127` for an XL and the panel shows that.
        name: "Preset Number",
        group: "Preferences",
        kind: Kind::Flag {
            // 128 slots is what both fall-back devices hold, so this pair is right for them.
            on: "000-127",
            off: "01A-32D",
        },
    },
    Setting {
        id: 31,
        name: "Input Level",
        group: "Ins/Outs",
        kind: Kind::Flag {
            on: "Line",
            off: "Instrument",
        },
    },
    Setting {
        id: 65,
        // [XL] `Authentc` is the pedal's own spelling, kept [sic] — almost certainly "Authentic"
        // truncated to the width of the screen.
        name: "Tempo Pitch",
        group: "Preferences",
        kind: Kind::Flag {
            on: "Transpr",
            off: "Authentc",
        },
    },
    Setting {
        id: 66,
        // [XL]
        name: "EXP 1 Position",
        group: "EXP Pedals",
        kind: Kind::Choice(&[(0, "Snapsht"), (1, "Preset"), (2, "Global")]),
    },
    Setting {
        id: 67,
        // [XL]
        name: "Snapsht Mode",
        group: "Footswitches",
        kind: Kind::Choice(&[(0, "Moment"), (1, "Latch"), (2, "Toggle")]),
    },
    Setting {
        id: 68,
        // [XL]
        name: "Tip Polarity",
        group: "Preferences",
        kind: Kind::Choice(&[(0, "Normal"), (1, "Inverted")]),
    },
    Setting {
        id: 69,
        // [XL]
        name: "Ring Polarity",
        group: "Preferences",
        kind: Kind::Choice(&[(0, "Normal"), (1, "Inverted")]),
    },
    Setting {
        id: 71,
        // [XL]
        name: "EXP 2 Position",
        group: "EXP Pedals",
        kind: Kind::Choice(&[(0, "Snapsht"), (1, "Preset"), (2, "Global")]),
    },
    Setting {
        id: 73,
        name: "Snapshot Edits",
        group: "Preferences",
        kind: Kind::Choice(&[(0, "Recall"), (1, "Discard")]),
    },
    Setting {
        id: 76,
        // [XL]
        name: "Tx MIDI Clock",
        group: "MIDI/Tempo",
        kind: Kind::Choice(&[(0, "Off"), (1, "MIDI"), (2, "USB"), (3, "MIDI+USB")]),
    },
    Setting {
        id: 77,
        // [XL]
        name: "MIDI PC Tx",
        group: "MIDI/Tempo",
        kind: Kind::Choice(&[(0, "Off"), (1, "MIDI"), (2, "USB"), (3, "MIDI+USB")]),
    },
    Setting {
        id: 81,
        name: "Bypass Type",
        group: "Preferences",
        kind: Kind::Flag {
            on: "DSP",
            off: "Analog",
        },
    },
    Setting {
        id: 94,
        name: "Output Level",
        group: "Ins/Outs",
        kind: Kind::Flag {
            on: "Line",
            off: "Instrument",
        },
    },
    Setting {
        id: 95,
        // [XL]
        name: "EXP/FS Tip",
        group: "Preferences",
        kind: Kind::Flag {
            on: "FS7",
            off: "EXP 1",
        },
    },
    Setting {
        id: 96,
        // [XL]
        name: "EXP/FS Ring",
        group: "Preferences",
        kind: Kind::Flag {
            on: "FS8",
            off: "EXP 2",
        },
    },
    Setting {
        id: 103,
        // [XL]
        name: "Snapshot Reselect",
        group: "Preferences",
        kind: Kind::Choice(&[(0, "Reload"), (1, "Toggle")]),
    },
    Setting {
        id: 117,
        // [XL]
        name: "Swap Up/Down",
        group: "Footswitches",
        kind: Kind::Choice(&[(0, "Off"), (1, "On")]),
    },
    Setting {
        id: 127,
        // **Called "Guitar In-Z" here from 2026-08-22 to 2026-08-23, which is not a name this
        // pedal has ever shown.** The pedal said *Auto In-Z*; the write-up supplied a Helix
        // setting that sounds like it, and the Ins/Outs group came along with it because that is
        // where a Helix keeps *Guitar In-Z*. An XL owner reported the discrepancy and the Stomp's
        // owner confirmed which one they had actually read out.
        //
        // Worth keeping the comment: this entry refused to name its two values because nobody had
        // seen them, and then carried an invented name for a day. The rule this module states
        // applies to the `name` field too.
        //
        // **Read off an HX Stomp, 2026-08-24**: the menu showed *First* at 0 and *Enabled* at 1,
        // each recorded beside its value. The label text came from an XL owner [2026-08-23]; this
        // is the reading that says which way round they go. Until then 0 and 1 had been seen with
        // the menu text not noted either side, so the mapping was not recoverable from that dump
        // and the labels stayed empty rather than being guessed — which is the rule working, not a
        // gap in it.
        name: "Auto In-Z",
        group: "Preferences",
        kind: Kind::Choice(&[(0, "First"), (1, "Enabled")]),
    },
    Setting {
        id: 129,
        // [XL]
        name: "TAP Function",
        group: "Footswitches",
        kind: Kind::Flag {
            on: "AllBypas",
            off: "TAP/Tunr",
        },
    },
    Setting {
        id: 130,
        // [XL]
        name: "FS7 Function",
        group: "Footswitches",
        kind: Kind::Choice(&[
            (0, "TAP/Tunr"),
            (1, "Stomp 7"),
            (2, "Bank Up"),
            (3, "Bank Dn"),
            (4, "PresetUp"),
            (5, "PresetDn"),
            (6, "SnpshtUp"),
            (7, "SnpshtDn"),
            (8, "AllBypas"),
            (9, "TogglEXP"),
        ]),
    },
    Setting {
        id: 131,
        // [XL]
        name: "FS8 Function",
        group: "Footswitches",
        kind: Kind::Choice(&[
            (0, "TAP/Tunr"),
            (1, "Stomp 8"),
            (2, "Bank Up"),
            (3, "Bank Dn"),
            (4, "PresetUp"),
            (5, "PresetDn"),
            (6, "SnpshtUp"),
            (7, "SnpshtDn"),
            (8, "AllBypas"),
            (9, "TogglEXP"),
        ]),
    },
    Setting {
        id: 135,
        // [XL]
        name: "Snapshot CC Send",
        group: "MIDI/Tempo",
        kind: Kind::Choice(&[(0, "Off"), (1, "On")]),
    },
    Setting {
        id: 136,
        // [XL]
        name: "Link Dual Cabs",
        group: "Preferences",
        kind: Kind::Choice(&[(0, "Off"), (1, "On")]),
    },
    Setting {
        id: 153,
        // [XL]
        name: "USB In 1/2 Trim",
        group: "Ins/Outs",
        kind: Kind::Number {
            unit: "dB",
            off: None,
        },
    },
    Setting {
        id: 154,
        // [XL] Landed as a `Flag` and corrected to a `Choice` when `setting-get 154` printed
        // `154 = 1  [int]` [XL, 2026-08-25]. The labels did not move: a dump prints the value, so
        // `0` Return / `1` Aux In is what the original diff showed — only the wire type was
        // transcribed wrong. Whether a third position exists has not been checked; as a `Choice`
        // an unlisted value shows as its number rather than as one of these two.
        name: "Return Type",
        group: "Ins/Outs",
        kind: Kind::Choice(&[(0, "Return"), (1, "Aux In")]),
    },
    Setting {
        id: 156,
        // Both observed states named by the owner: 1 -> 2 was the move to "main + headphones", so
        // 1 is the headphone-only position, where the knob leaves the main outputs alone. `0` has
        // never been seen and is not assumed to exist. The two labels are the XL menu's own
        // wording for those positions [2026-08-23].
        name: "Volume Controls",
        group: "Ins/Outs",
        kind: Kind::Choice(&[(1, "Phones"), (2, "Main+HP")]),
    },
    Setting {
        id: 158,
        // [XL]
        name: "Phones Monitor",
        group: "Ins/Outs",
        kind: Kind::Choice(&[(1, "Main L/R"), (2, "Send")]),
    },
    Setting {
        id: 190,
        name: "EQ low frequency",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "Hz",
            off: None,
        },
    },
    Setting {
        id: 191,
        name: "EQ low Q",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "",
            off: None,
        },
    },
    Setting {
        id: 192,
        name: "EQ low gain",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "dB",
            off: None,
        },
    },
    Setting {
        id: 193,
        name: "EQ mid frequency",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "Hz",
            off: None,
        },
    },
    Setting {
        id: 194,
        name: "EQ mid Q",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "",
            off: None,
        },
    },
    Setting {
        id: 195,
        name: "EQ mid gain",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "dB",
            off: None,
        },
    },
    Setting {
        id: 196,
        name: "EQ high frequency",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "Hz",
            off: None,
        },
    },
    Setting {
        id: 197,
        name: "EQ high Q",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "",
            off: None,
        },
    },
    Setting {
        id: 198,
        name: "EQ high gain",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "dB",
            off: None,
        },
    },
    Setting {
        id: 199,
        name: "EQ low cut",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "Hz",
            off: Some(19.9),
        },
    },
    Setting {
        id: 200,
        name: "EQ high cut",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "Hz",
            off: Some(20100.0),
        },
    },
];

/// The order the pedal's own menus list these in, for the ids where somebody has walked the menu.
///
/// [`SETTINGS`] stays in id order because that is how it is read and edited here; a panel wants the
/// order the pedal puts them in, and the two are not the same list. Keeping the second one as ids
/// rather than as the table's order is what lets both be true at once.
///
/// Ids absent from here sort after every id present, by id — see [`menu_rank`]. That is the honest
/// default: nobody has placed them, so they keep their numeric position rather than being guessed
/// into the middle of a menu.
///
/// One *identified* id is absent for exactly that reason: **135**, `Snapshot CC Send`. The pass that
/// named it placed the rest of its section and not this one. Rows sort by
/// `(group_rank, menu_rank, id)`, so it draws at the foot of MIDI/Tempo instead of somewhere wrong —
/// a guessed position would look no different and be worse.
pub const MENU_ORDER: &[i64] = &[
    31, 94, 2, 3, 154, 153, 158, 156, // Ins/Outs
    81, 73, 65, 95, 96, 68, 69, 27, 103, 127, 136, // Preferences
    17, 19, 18, 67, 20, 117, 129, 130, 131, // Footswitches
    66, 71, // EXP Pedals
    9, 10, 13, 76, 14, 16, 11, 12, 77, 135, // MIDI/Tempo
    25, 26, // Displays
];

/// Where `id` sits in the pedal's menus — `MENU_ORDER.len()` for an id nobody has placed.
///
/// Sort a row list by `(menu_rank(id), id)` to get menu order where it is known and id order
/// everywhere else.
pub fn menu_rank(id: i64) -> usize {
    MENU_ORDER
        .iter()
        .position(|&m| m == id)
        .unwrap_or(MENU_ORDER.len())
}

/// The device's own factory value for `id`, where we have observed one.
///
/// **Only the global EQ, and only from one unit.** The pedal resets a setting when you push its
/// knob in; these are what an HX Stomp reported after every Global EQ knob had been pushed
/// [2026-08-22]. Id 193 is the one that proves the gesture rather than the state — it had been left
/// at 1900 by hand and came back as 2000.
///
/// Nothing about this is documented: no default appears in any shipped `.models` file, in
/// `HelixControls.json`, or anywhere in the wire protocol, which offers a value and neither a
/// default nor a range. So this is one unit's factory EQ, recorded with its provenance rather than
/// asserted as universal — a Floor or an LT may well differ, the same caveat the Floor's setlist
/// names carry. Every other id returns `None`: their defaults have simply never been observed.
pub fn default_of(id: i64) -> Option<f64> {
    Some(match id {
        190 => 110.0,
        191 => 0.707,
        192 => 0.0,
        193 => 2000.0,
        194 => 0.707,
        195 => 0.0,
        196 => 8000.0,
        197 => 0.707,
        198 => 0.0,
        // The cuts have no separate enable — off *is* a frequency, parked at the end of the range.
        199 => 19.9,
        200 => 20100.0,
        _ => return None,
    })
}

/// The identified setting for `id`, if we have one.
pub fn by_id(id: i64) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.id == id)
}

/// Whether `id` is one we are willing to **write**.
///
/// Only identified ids. Writing an id whose meaning nobody has observed is the one operation in
/// this area that could change something the user cannot find their way back from, and the upside
/// — mapping the space faster — is already served by reading it and changing it on the pedal.
pub fn is_writable(id: i64) -> bool {
    by_id(id).is_some()
}

/// The order the pedal lists its **Global Settings** sections in, read off an HX Stomp and an HX
/// Stomp XL independently — the two agree, so this is the platform's structure rather than one
/// unit's [2026-08-24].
///
/// `Global EQ` is the exception and is **not** one of those sections: it is a separate top-level
/// menu on the pedal. It leads the list because the panel gives it its own tab, and because every
/// id in it sorts before the rest anyway; it is here so that [`group_rank`] can place it and so
/// `every_group_is_declared` keeps covering it.
pub const GROUPS: &[&str] = &[
    "Global EQ",
    "Ins/Outs",
    "Preferences",
    "Footswitches",
    "EXP Pedals",
    "MIDI/Tempo",
    "Displays",
];

/// Where `group` sits in the pedal's menus — `GROUPS.len()` for a name nobody has placed, which is
/// what `"Unidentified"` gets.
///
/// Sort rows by `(group_rank(group), menu_rank(id), id)` for the pedal's own layout. This is the
/// only ordering authority: the panel renders groups in the order its rows arrive rather than
/// keeping a second copy of this list, because that copy went stale the moment [`GROUPS`] changed.
pub fn group_rank(group: &str) -> usize {
    GROUPS
        .iter()
        .position(|&g| g == group)
        .unwrap_or(GROUPS.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Id order is the table's contract, not an accident: this file is searched by number as often
    /// as it is read top to bottom. Menu order is [`MENU_ORDER`]'s job, so a new setting goes in at
    /// its number and gets placed there, not spliced into the middle of the table.
    #[test]
    fn ids_are_unique_and_sorted() {
        let ids: Vec<i64> = SETTINGS.iter().map(|s| s.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            ids, sorted,
            "SETTINGS must be id-ordered with no duplicates"
        );
    }

    #[test]
    fn every_group_is_declared() {
        for s in SETTINGS {
            assert!(
                GROUPS.contains(&s.group),
                "{} is in group {:?}, which GROUPS does not list",
                s.name,
                s.group
            );
        }
    }

    /// The one this whole area was blocked on. Its two labels are the strings the preset list
    /// numbers itself by, so they are not free to drift.
    #[test]
    fn preset_numbering_is_id_27() {
        let s = by_id(27).unwrap();
        assert_eq!(s.name, "Preset Number");
        assert_eq!(
            s.kind,
            Kind::Flag {
                on: "000-127",
                off: "01A-32D"
            }
        );
    }

    /// Nothing may be written that we have not identified — see [`is_writable`].
    #[test]
    fn only_identified_ids_are_writable() {
        assert!(is_writable(27));
        // 128 answers on a Stomp (the handshake reads it) and has never been identified.
        assert!(!is_writable(128));
        assert!(!is_writable(-1));
    }

    /// Defaults are known for the EQ and nothing else, and that asymmetry is the point — it says
    /// which ones we have watched the pedal reset.
    #[test]
    fn defaults_cover_the_eq_and_only_the_eq() {
        for s in SETTINGS {
            let eq = s.group == "Global EQ";
            assert_eq!(
                default_of(s.id).is_some(),
                eq,
                "{} ({}) disagrees about having a default",
                s.name,
                s.id
            );
        }
        // Flat: no boost or cut on any of the three bands.
        assert_eq!(default_of(192), Some(0.0));
        assert_eq!(default_of(195), Some(0.0));
        assert_eq!(default_of(198), Some(0.0));
        // And both cuts parked at their off sentinels rather than at an audible corner.
        assert_eq!(default_of(199), Some(19.9));
        assert_eq!(default_of(200), Some(20100.0));
        assert_eq!(default_of(27), None);
    }

    /// `MENU_ORDER` is a view onto the table, so every id in it must be one the table has, and no
    /// id may appear twice — either would silently drop a row or render one in two places.
    #[test]
    fn menu_order_places_real_ids_once() {
        let mut seen = Vec::new();
        for &id in MENU_ORDER {
            assert!(
                by_id(id).is_some(),
                "MENU_ORDER places id {id}, which SETTINGS does not have"
            );
            assert!(!seen.contains(&id), "MENU_ORDER places id {id} twice");
            seen.push(id);
        }
    }

    /// The counterpart to `menu_order_places_real_ids_once`: an identified id outside the Global EQ
    /// tab should have a place in the pedal's menus, and the ones that don't are named here rather
    /// than left to be noticed. Id 135 arrived without its position — see [`MENU_ORDER`].
    #[test]
    fn only_the_listed_ids_are_unplaced() {
        const UNPLACED: &[i64] = &[135];
        let missing: Vec<i64> = SETTINGS
            .iter()
            .filter(|s| s.group != "Global EQ" && menu_rank(s.id) == MENU_ORDER.len())
            .map(|s| s.id)
            .collect();
        assert_eq!(missing, UNPLACED, "an identified id has no menu position");
    }

    /// Footswitches and EXP Pedals were declared and empty from 2026-08-24 to 2026-08-25, with a
    /// paragraph in [`GROUPS`] explaining why. That paragraph is gone because they are populated;
    /// this is what keeps them that way, and says a section arrives with its ids, not ahead of them.
    #[test]
    fn every_declared_group_has_rows() {
        for g in GROUPS {
            assert!(
                SETTINGS.iter().any(|s| s.group == *g),
                "group {g:?} is declared but has no settings"
            );
        }
    }

    /// An id nobody has placed sorts after every id somebody has, rather than to the top — the
    /// unplaced majority must not push the menu-ordered block down the panel.
    #[test]
    fn unplaced_ids_sort_after_placed_ones() {
        assert!(menu_rank(31) < menu_rank(94));
        assert!(menu_rank(156) < menu_rank(190));
        assert_eq!(menu_rank(190), MENU_ORDER.len());
        // Not a setting at all, and not a panic either.
        assert_eq!(menu_rank(-1), MENU_ORDER.len());
    }

    /// The pedal's section order is the one thing the panel must not re-derive, so it has to be
    /// stated once and be reachable. A group that exists in the table and not in [`GROUPS`] would
    /// sort last silently; `every_group_is_declared` catches that, and this catches the inverse —
    /// that the order is the one read off the hardware.
    #[test]
    fn groups_rank_in_the_pedals_menu_order() {
        assert!(group_rank("Ins/Outs") < group_rank("Preferences"));
        assert!(group_rank("Preferences") < group_rank("Footswitches"));
        assert!(group_rank("Footswitches") < group_rank("EXP Pedals"));
        assert!(group_rank("EXP Pedals") < group_rank("MIDI/Tempo"));
        assert!(group_rank("MIDI/Tempo") < group_rank("Displays"));
        // Not a group at all — the raw tier, which must land after every named section.
        assert_eq!(group_rank("Unidentified"), GROUPS.len());
        assert!(GROUPS.iter().all(|g| group_rank(g) < GROUPS.len()));
    }

    /// MIDI and Tempo are one section on the pedal, not two. They were two here until 2026-08-24,
    /// which is why this is pinned rather than left to the group list to imply.
    #[test]
    fn midi_and_tempo_are_one_section() {
        for id in [9, 11, 14, 16] {
            assert_eq!(by_id(id).unwrap().group, "MIDI/Tempo", "id {id}");
        }
    }

    /// A choice with no entries is how "observed, never explained" is recorded — it must stay
    /// legal, because the alternative is inventing labels.
    ///
    /// Deliberately not pinned to an id. It used to assert this of id 127, which was the table's
    /// only unexplained choice until that id's values were read off the pedal [2026-08-24] — at
    /// which point the test failed for the one reason it should never fail: the table got *better*.
    /// The invariant is about the shape, so it is stated about the shape, and the next id to arrive
    /// unexplained inherits it for free.
    #[test]
    fn a_choice_may_be_empty() {
        const UNEXPLAINED: Setting = Setting {
            id: -1,
            name: "observed, never explained",
            group: "Preferences",
            kind: Kind::Choice(&[]),
        };
        let Kind::Choice(options) = UNEXPLAINED.kind else {
            panic!("an empty choice must stay a choice, not decay to another kind");
        };
        assert!(options.is_empty());
    }
}
