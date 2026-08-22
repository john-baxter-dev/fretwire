//! The device-settings namespace — what each id *is*, where we know.
//!
//! Settings are a flat numbered space read with op 24 and written with op 25 (see
//! `docs/protocol.md`). Nothing here is documented by Line 6; every entry was read off a physical
//! HX Stomp by changing one thing on the pedal's own menus and diffing two dumps.
//!
//! **166 of ids 0..=600 answer on an HX Stomp, and 19 are named.** That ratio is the normal state
//! of this table, not a gap to be filled in with plausible guesses: an id whose meaning nobody has
//! observed is simply absent, and the UI shows it as a raw number rather than inventing a label.
//! Adding one costs about thirty seconds with `fretwire settings-diff`.

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

/// Every setting id we have identified. **Read off a physical HX Stomp, 2026-08-22.**
///
/// Ordered by id. Ids not here answer but are unidentified — see the module note.
pub const SETTINGS: &[Setting] = &[
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
        name: "Preset numbering",
        group: "Displays",
        kind: Kind::Flag {
            on: "000-127",
            off: "01A-32D",
        },
    },
    Setting {
        id: 31,
        name: "Input level",
        group: "Ins/Outs",
        kind: Kind::Flag {
            on: "Line",
            off: "Instrument",
        },
    },
    Setting {
        id: 73,
        name: "Snapshot edits",
        group: "Preferences",
        kind: Kind::Choice(&[(0, "Recall"), (1, "Discard")]),
    },
    Setting {
        id: 81,
        name: "Bypass type",
        group: "Preferences",
        kind: Kind::Flag {
            on: "DSP bypass",
            off: "Analog bypass",
        },
    },
    Setting {
        id: 94,
        name: "Output level",
        group: "Ins/Outs",
        kind: Kind::Flag {
            on: "Line",
            off: "Instrument",
        },
    },
    Setting {
        id: 127,
        // Observed as 0 and 1 with the menu entries not recorded either side, so the values are
        // listed without names rather than being labelled from memory.
        name: "Guitar In-Z",
        group: "Ins/Outs",
        kind: Kind::Choice(&[]),
    },
    Setting {
        id: 156,
        // 1 -> 2 was "main + headphones"; what 1 alone is called was not recorded, and 0 has never
        // been seen. Only the one confirmed label is here.
        name: "Volume knob controls",
        group: "Ins/Outs",
        kind: Kind::Choice(&[(2, "Main + headphones")]),
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
        id: 196,
        name: "EQ high frequency",
        group: "Global EQ",
        kind: Kind::Number {
            unit: "Hz",
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
    "Tempo",
    "MIDI",
    "Preferences",
    "Displays",
];

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(s.name, "Preset numbering");
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

    /// A choice with no entries is how "observed, never explained" is recorded — it must stay
    /// legal, because the alternative is inventing labels.
    #[test]
    fn a_choice_may_be_empty() {
        let inz = by_id(127).unwrap();
        assert!(matches!(inz.kind, Kind::Choice(&[])));
    }
}
