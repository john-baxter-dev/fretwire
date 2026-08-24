//! The device-settings namespace — what each id *is*, where we know.
//!
//! Settings are a flat numbered space read with op 24 and written with op 25 (see
//! `docs/protocol.md`). Nothing here is documented by Line 6; every entry was read off a physical
//! pedal by changing one thing on its own menus and diffing two dumps — an HX Stomp for most of
//! them, an HX Stomp XL for the five marked `[XL]` below.
//!
//! **166 of ids 0..=600 answer on an HX Stomp. 28 are identified; 27 of them are here.** The odd
//! one out is id 28, the current preset index — device state rather than a preference, written
//! properly by `Session::goto_preset`, and deliberately not offered as a settings row.
//!
//! That ratio is the normal state of this table, not a gap to be filled in with plausible guesses:
//! an id whose meaning nobody has observed is simply absent, and the UI shows it as a raw number
//! rather than inventing a label. Adding one costs about thirty seconds with
//! `fretwire settings-diff`.

/// How a setting's value should be presented and edited.
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
/// XL owner read off that pedal the same way and contributed [2026-08-23]. Where the two units'
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
        name: "MIDI base channel",
        group: "MIDI",
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
        id: 11,
        name: "MIDI over USB",
        group: "MIDI",
        kind: Kind::Flag {
            on: "On",
            off: "Off",
        },
    },
    Setting {
        id: 14,
        name: "Tempo select",
        group: "Tempo",
        kind: Kind::Choice(&[(0, "Per snapshot"), (1, "Per preset"), (2, "Global")]),
    },
    Setting {
        id: 16,
        name: "Tempo",
        group: "Tempo",
        kind: Kind::Number {
            unit: "BPM",
            off: None,
        },
    },
    Setting {
        id: 27,
        name: "Preset Number",
        group: "Preferences",
        kind: Kind::Flag {
            on: "000-128",
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
        // [XL]
        name: "Tempo Pitch",
        group: "Preferences",
        kind: Kind::Flag {
            on: "Transpr",
            off: "Authentc",
        },
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
        id: 73,
        name: "Snapshot Edits",
        group: "Preferences",
        kind: Kind::Choice(&[(0, "Recall"), (1, "Discard")]),
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
        // The values are still unnamed — 0 and 1 were observed with the menu entry not recorded
        // either side, so which one is `First` is not recoverable after the fact.
        name: "Auto In-Z",
        group: "Preferences",
        kind: Kind::Choice(&[(0, "First"), (1, "Enabled")]),
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
        // [XL]
        name: "Return Type",
        group: "Ins/Outs",
        kind: Kind::Flag {
            on: "Aux In",
            off: "Return",
        },
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
pub const MENU_ORDER: &[i64] = &[
    31, 94, 2, 3, 154, 153, 158, 156, // Ins/Outs
    81, 73, 65, 95, 96, 68, 69, 27, 103, 127, 136, // Preferences
    9, 11, // MIDI
    14, 16, // Tempo
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

/// The order groups should be shown in. Anything not listed sorts last, alphabetically.
pub const GROUPS: &[&str] = &[
    "Global EQ",
    "Ins/Outs",
    "Preferences",
    "Tempo",
    "MIDI",
    "Displays",
];

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
                on: "000-128",
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

    /// A choice with no entries is how "observed, never explained" is recorded — it must stay
    /// legal, because the alternative is inventing labels.
    #[test]
    fn a_choice_may_be_empty() {
        let inz = by_id(127).unwrap();
        assert!(matches!(inz.kind, Kind::Choice(&[])));
    }
}
