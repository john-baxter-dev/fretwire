//! `.hxb`/`.pgb` backup container. Built from **synthetic** files so this runs on a clean clone —
//! the real backups we have are contributors' personal device dumps and are deliberately not in
//! git (see CLAUDE.md). Two layouts are asserted, both validated against real files:
//!
//! - the **section-table** format every real backup carries (`build_with_table`, shaped like the
//!   POD Go `.pgb`: PGDI + SLNM + GLOB + one IR + UMDS + two setlists, table at the end);
//! - the **legacy fallback** reading for a file without a valid table (`build`) — the shape this
//!   parser assumed before the table was found, kept so a file we misunderstand degrades to the
//!   old behaviour instead of failing.

use flate2::{Compression, write::ZlibEncoder};
use fretwire_data::hxb::Hxb;
use std::io::Write;

fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

/// A setlist stream in the shape the device writes: empty slots carry no `tone` key.
fn setlist(name: &str, presets: &[Option<&str>]) -> Vec<u8> {
    let slots: Vec<String> = presets
        .iter()
        .map(|p| match p {
            Some(n) => format!(r#"{{"meta":{{"name":"{n}"}},"tone":{{"dsp0":{{}}}}}}"#),
            None => "{}".to_string(),
        })
        .collect();
    format!(
        r#"{{"schema":"L6Setlist","version":2,"data":{{"meta":{{"name":"{name}"}},"presets":[{}]}}}}"#,
        slots.join(",")
    )
    .into_bytes()
}

fn build() -> Vec<u8> {
    let mut f = vec![0u8; 0x70];
    f[0..4].copy_from_slice(b"AF6L");
    f[0x04..0x08].copy_from_slice(&1u32.to_le_bytes());
    f[0x18..0x1c].copy_from_slice(&0x0021_0001u32.to_le_bytes()); // Helix Floor
    f[0x1c..0x20].copy_from_slice(&0x0380_0000u32.to_le_bytes());
    f[0x28..0x2c].copy_from_slice(&1_784_776_984u32.to_le_bytes());
    let comment = b"test backup";
    f[0x30..0x30 + comment.len()].copy_from_slice(comment);

    f.extend(zlib(br#"{"System":{"PresetNumbering":false}}"#)); // #0 globals
    f.extend(zlib(b"RIFF\0\0\0\0WAVEfmt ")); // #1 one IR slot
    f.extend(zlib(br#"{"schema":"L6UMDArchive","data":{}}"#)); // #2 model-usage table
    f.extend(zlib(&setlist(
        "FACTORY 1",
        &[Some("Pull Me Under"), Some("Richeese")],
    )));
    f.extend(zlib(&setlist("USER 1", &[None, Some("Sludge")])));
    f.extend([0u8, 0u8]); // the real file ends with two NUL bytes; must not upset the walker
    f
}

/// A `.pgb`-shaped file: sections indexed by the 36-byte-entry table at the end. Offsets and
/// lengths are computed, not hardcoded, so the fixture can't drift from its own table.
fn build_with_table(extra: &[(&[u8; 4], &[u8], bool)]) -> Vec<u8> {
    let mut f = vec![0u8; 0x30];
    f[0..4].copy_from_slice(b"AF6L");
    f[0x04..0x08].copy_from_slice(&1u32.to_le_bytes());
    f[0x18..0x1c].copy_from_slice(&0x0021_0007u32.to_le_bytes()); // POD Go
    f[0x1c..0x20].copy_from_slice(&0x0250_0000u32.to_le_bytes());
    f[0x28..0x2c].copy_from_slice(&1_787_931_000u32.to_le_bytes());

    // (tag as read, inflated data, compressed) — the builder stores the tag reversed, as the
    // editor does, and deflates the compressed ones itself so the declared inflated length is
    // the true one. `extra` appends sections this parser has no reading for.
    let mut sections: Vec<(&[u8; 4], Vec<u8>, bool)> = vec![
        (b"PGDI", f[0x18..0x30].to_vec(), false), // points at the header fields; dropped below
        (b"SLNM", b"Factory\0User\0".to_vec(), false),
        (
            b"GLOB",
            br#"{"System":{"PresetNumbering":false}}"#.to_vec(),
            true,
        ),
        (b"I000", b"RIFF\0\0\0\0WAVEfmt ".to_vec(), true),
        (
            b"UMDS",
            br#"{"schema":"L6UMDArchive","data":{}}"#.to_vec(),
            true,
        ),
        (
            b"SL00",
            setlist("Factory", &[Some("US Deluxe Nrm"), None]),
            true,
        ),
        (
            b"SL01",
            setlist("User", &[None, Some("Acoustic split")]),
            true,
        ),
    ];
    sections.extend(extra.iter().map(|(t, d, c)| (*t, d.to_vec(), *c)));
    let mut table = Vec::new();
    for (tag, data, compressed) in &sections {
        let (off, len) = if *tag == b"PGDI" {
            (0x18u64, 0x18u64) // the one section that points back into the header
        } else {
            let off = f.len() as u64;
            let stored = if *compressed {
                zlib(data)
            } else {
                data.clone()
            };
            f.extend_from_slice(&stored);
            (off, stored.len() as u64)
        };
        let mut e = Vec::with_capacity(36);
        e.extend(tag.iter().rev()); // stored reversed
        e.extend(off.to_le_bytes());
        e.extend(len.to_le_bytes());
        e.extend((u32::from(*compressed)).to_le_bytes());
        e.extend((data.len() as u64).to_le_bytes()); // declared inflated length
        e.extend([0u8; 4]);
        table.push(e);
    }
    let table_off = f.len() as u32;
    f[0x08..0x0c].copy_from_slice(&table_off.to_le_bytes());
    f[0x10..0x18].copy_from_slice(&(sections.len() as u64).to_le_bytes());
    for e in table {
        f.extend(e);
    }
    f
}

#[test]
fn the_section_table_names_the_setlists_and_bounds_the_streams() {
    let h = Hxb::parse(&build_with_table(&[])).unwrap();
    assert_eq!(h.device_id, 0x0021_0007);
    assert_eq!(h.device_version, 0x0250_0000);
    // No DESC section: the comment is empty, not the bytes that happen to sit at 0x30 — which
    // here are the SLNM section, exactly the misread the fixed-offset parser used to make.
    assert_eq!(h.comment, "");
    assert_eq!(h.setlist_names, ["Factory", "User"]);
    assert_eq!(h.streams.len(), 5, "GLOB + IR + UMDS + 2 setlists");
    assert_eq!(h.impulse_responses().len(), 1);
    assert!(h.globals().is_some());
    let sets = h.setlists();
    assert_eq!(sets.len(), 2);
    assert_eq!((sets[0].bank, sets[0].name.as_str()), (0, "Factory"));
    assert_eq!((sets[1].bank, sets[1].name.as_str()), (1, "User"));
    assert_eq!(sets[0].presets[0].as_ref().unwrap().name, "US Deluxe Nrm");
}

/// The table is kept whole, every tag classified, and a tag this reading has never seen — the
/// shape a Favorites or User Defaults store would first appear in — is called out rather than
/// folded silently into `streams`.
#[test]
fn the_section_table_is_kept_and_unknown_tags_are_reported() {
    let h = Hxb::parse(&build_with_table(&[])).unwrap();
    let tags: Vec<&str> = h.sections.iter().map(|s| s.tag.as_str()).collect();
    assert_eq!(
        tags,
        ["PGDI", "SLNM", "GLOB", "I000", "UMDS", "SL00", "SL01"]
    );
    assert!(h.sections.iter().all(|s| s.known_as().is_some()));
    assert!(h.unknown_sections().is_empty());
    let glob = &h.sections[2];
    assert!(glob.compressed);
    assert_eq!(
        glob.inflated_len,
        br#"{"System":{"PresetNumbering":false}}"#.len() as u64
    );
    assert_ne!(glob.stored_len, glob.inflated_len);
    let slnm = &h.sections[1];
    assert_eq!(
        (slnm.compressed, slnm.stored_len, slnm.inflated_len),
        (false, 13, 13)
    );

    let h = Hxb::parse(&build_with_table(&[
        (b"QRST", br#"{"schema":"L6Whatever","data":{}}"#, true),
        (b"XYZW", b"raw bytes", false),
    ]))
    .unwrap();
    let unknown: Vec<&str> = h
        .unknown_sections()
        .iter()
        .map(|s| s.tag.as_str())
        .collect();
    assert_eq!(unknown, ["QRST", "XYZW"]);
    // The unknown compressed one still inflates into `streams`, as before — nothing is lost,
    // it is just now also named.
    assert_eq!(h.streams.len(), 6);
    assert_eq!(
        h.setlists().len(),
        2,
        "unknown sections do not disturb the setlist reading"
    );
    // A legacy file has no table to keep.
    assert!(Hxb::parse(&build()).unwrap().sections.is_empty());
}

/// `F000`… is a favorite: the shape HX Edit wrote for an HX Stomp with two saved (2026-09-04).
/// An amp carries its cab as `slot1`; anything else has `slot0` alone.
#[test]
fn favorites_are_one_section_each() {
    let amp = br#"{"data":{"device":2162694,"device_version":58720256,"favorite":{"slot0":{"@enabled":true,"@model":"HD2_AmpUSPrincess","@type":3,"Drive":0.42},"slot1":{"@model":"HD2_CabMicIr_1x12USDeluxe","@type":2,"Mic":2}},"meta":{"name":"US Princess"}},"schema":"L6ModelFavorite","version":1}"#;
    let verb = br#"{"data":{"device":2162694,"device_version":58720256,"favorite":{"slot0":{"@enabled":true,"@model":"VIC_DynPlate","@stereo":true,"@trails":false,"@type":7,"Decay":2.0}},"meta":{"name":"Dynamic Plate"}},"schema":"L6ModelFavorite","version":1}"#;
    let h = Hxb::parse(&build_with_table(&[
        (b"F000", amp, true),
        (b"F001", verb, true),
    ]))
    .unwrap();
    assert!(h.unknown_sections().is_empty(), "F-tags are known");
    let favs = h.favorites();
    assert_eq!(favs.len(), 2);
    assert_eq!(
        (favs[0].name.as_str(), favs[0].model.as_str()),
        ("US Princess", "HD2_AmpUSPrincess")
    );
    assert_eq!(
        favs[0].favorite["slot1"]["@model"],
        "HD2_CabMicIr_1x12USDeluxe"
    );
    assert_eq!(
        (favs[1].name.as_str(), favs[1].model.as_str()),
        ("Dynamic Plate", "VIC_DynPlate")
    );
    assert!(favs[1].favorite.get("slot1").is_none());
    // Favorites are not setlists and not IRs.
    assert_eq!(h.setlists().len(), 2);
    assert_eq!(h.impulse_responses().len(), 1);
}

#[test]
fn parses_header_and_walks_the_concatenated_zlib_streams() {
    let h = Hxb::parse(&build()).unwrap();
    assert_eq!(h.version, 1);
    assert_eq!(h.device_id, 0x0021_0001);
    assert_eq!(h.device_version, 0x0380_0000);
    assert_eq!(h.timestamp, 1_784_776_984);
    assert_eq!(h.comment, "test backup");
    // Trailing NUL padding must not become a phantom stream.
    assert_eq!(h.streams.len(), 5, "globals + IR + archive + 2 setlists");
    assert_eq!(h.impulse_responses().len(), 1);
    assert!(h.globals().is_some());
}

#[test]
fn setlist_order_is_the_bank_numbering() {
    let h = Hxb::parse(&build()).unwrap();
    let sets = h.setlists();
    assert_eq!(sets.len(), 2);
    assert_eq!((sets[0].bank, sets[0].name.as_str()), (0, "FACTORY 1"));
    assert_eq!((sets[1].bank, sets[1].name.as_str()), (1, "USER 1"));

    // An empty slot is a slot, not a gap: indices stay aligned with the device's numbering.
    assert_eq!(sets[1].presets.len(), 2);
    assert!(sets[1].presets[0].is_none());
    assert_eq!(sets[1].populated(), 1);
    let p = sets[1].presets[1].as_ref().unwrap();
    assert_eq!((p.index, p.name.as_str()), (1, "Sludge"));
    assert!(
        p.tone.get("dsp0").is_some(),
        "the tone object is kept whole"
    );
}

#[test]
fn rejects_a_file_that_is_not_a_backup() {
    assert!(Hxb::parse(b"not an hxb file at all, really").is_err());
    assert!(Hxb::parse(&[]).is_err());
}
