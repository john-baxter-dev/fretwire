//! Human-readable renderings of the command layer's DTOs — what the tools hand back.
//!
//! The DTOs are shaped for a GUI to render (every param carries its range, format rules, enum
//! labels, step size). An assistant reasons better over "Drive 6.5 · Bass 5.0" than over that,
//! and pays for every token of it, so the tools return text: a preset as a short listing in signal
//! order, a parameter as HX Edit would display it, a diff as the lines that changed.

use fretwire_commands::dto::{BlockDto, NumFormatDto, ParamDto, PresetDto, PresetListItem};
use fretwire_core::editor::{FormatRule, NumFormat, category_name, dsp_percent};

/// A parameter's value the way HX Edit shows it: an enum's label, a switch's On/Off, a segmented
/// control's stop, a scaled number with its unit — or the raw number when nothing describes it.
pub fn param_value(p: &ParamDto) -> String {
    if !p.enum_labels.is_empty()
        && let Some(label) = enum_label(p, p.value)
    {
        return label.to_string();
    }
    if p.kind == "bool" || p.value_type == Some(2) {
        return if p.value != 0.0 { "On" } else { "Off" }.to_string();
    }
    show_number(p, p.value)
}

fn enum_label(p: &ParamDto, value: f64) -> Option<&str> {
    let i = usize::try_from(value.round() as i64 - p.enum_base).ok()?;
    p.enum_labels.get(i).map(String::as_str)
}

/// A stored number through the param's display rules (stops, then the numeric format).
fn show_number(p: &ParamDto, raw: f64) -> String {
    if !p.stops.is_empty()
        && let Some(stop) = p
            .stops
            .iter()
            .min_by(|a, b| (a.value - raw).abs().total_cmp(&(b.value - raw).abs()))
    {
        return stop.label.clone();
    }
    if let Some(f) = &p.format
        && let Some(s) = num_format(f).display(raw)
    {
        return s;
    }
    trim_float(raw)
}

fn num_format(f: &NumFormatDto) -> NumFormat {
    NumFormat {
        scale: f.scale,
        offset: f.offset,
        rules: f
            .rules
            .iter()
            .map(|r| FormatRule {
                lo: r.lo.unwrap_or(f64::NEG_INFINITY),
                hi: r.hi.unwrap_or(f64::INFINITY),
                mult: r.mult,
                template: r.template.clone(),
            })
            .collect(),
    }
}

/// The stored value for a number given in **display** units ("6.5" on a 0–10 knob, "450" on a
/// delay shown in ms) — the display rules run backwards. `unit` is whatever followed the number
/// ("ms", "s", "dB", "" for none): where a param switches rules by magnitude, a bare "1.4" is
/// ambiguous between 1.4 ms and 1.4 s, and the unit picks the rule. A param with no format takes
/// the number as stored.
pub fn raw_from_display(p: &ParamDto, shown: f64, unit: &str) -> f64 {
    let Some(f) = &p.format else {
        return shown;
    };
    if f.scale == 0.0 {
        return shown;
    }
    let unit = unit.trim();
    let named: Vec<&fretwire_commands::dto::FormatRuleDto> = f
        .rules
        .iter()
        .filter(|r| !unit.is_empty() && template_unit(&r.template).eq_ignore_ascii_case(unit))
        .collect();
    let rules: Vec<&fretwire_commands::dto::FormatRuleDto> = if named.is_empty() {
        f.rules.iter().collect()
    } else {
        named
    };
    let invert = |r: &fretwire_commands::dto::FormatRuleDto| {
        let mult = if r.mult == 0.0 { 1.0 } else { r.mult };
        let scaled = shown / mult;
        (scaled, (scaled - f.offset) / f.scale)
    };
    // The rule whose range contains the value it would produce — the same rule `display` picks.
    for r in &rules {
        let (scaled, raw) = invert(r);
        let lo = r.lo.unwrap_or(f64::NEG_INFINITY);
        let hi = r.hi.unwrap_or(f64::INFINITY);
        if scaled >= lo && scaled < hi {
            return raw;
        }
    }
    rules.last().map(|r| invert(r).1).unwrap_or(shown)
}

/// The unit text a printf-ish template carries after its number: "%.0f ms" → "ms", "%.1f%%" → "%",
/// "%+.1f dB" → "dB". The text after the last conversion (`%…f`, `%…d`), `%%` unescaped.
fn template_unit(template: &str) -> String {
    let b = template.as_bytes();
    let mut i = 0;
    let mut after = 0;
    while i < b.len() {
        if b[i] != b'%' {
            i += 1;
            continue;
        }
        if i + 1 < b.len() && b[i + 1] == b'%' {
            i += 2;
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && !b[j].is_ascii_alphabetic() {
            j += 1;
        }
        after = (j + 1).min(b.len());
        i = after;
    }
    template[after..].replace("%%", "%").trim().to_string()
}

/// "min–max" in display units, for a continuous param the table bounds. `None` for enums (their
/// options are the range) and unbounded params.
pub fn param_range(p: &ParamDto) -> Option<String> {
    if !p.enum_labels.is_empty() || p.kind == "bool" || p.value_type == Some(2) {
        return None;
    }
    match (p.min, p.max) {
        (Some(lo), Some(hi)) => Some(format!("{}–{}", show_number(p, lo), show_number(p, hi))),
        _ => None,
    }
}

fn trim_float(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

fn params_line(params: &[ParamDto]) -> String {
    params
        .iter()
        .map(|p| format!("{} {}", p.name, param_value(p)))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// One block as a line: slot, model (with its paired cab), label, footswitch, bypass, DSP cost.
pub fn block_line(b: &BlockDto) -> String {
    let mut s = format!("slot {:<2}", b.slot);
    if b.row == 1 {
        s.push_str(" (path B)");
    }
    s.push_str("  ");
    if let Some(cat) = b.category.and_then(category_name) {
        s.push_str(cat);
        s.push_str(": ");
    }
    s.push_str(&b.model_name);
    if let Some(cab) = &b.paired_model_name {
        s.push_str(" + ");
        s.push_str(cab);
    }
    if let Some(l) = &b.user_label {
        s.push_str(&format!(" \"{l}\""));
    }
    if b.footswitch > 0 {
        s.push_str(&format!(" [FS{}]", b.footswitch));
    }
    if b.bypassed == Some(true) {
        s.push_str(" — BYPASSED");
    }
    if let Some(l) = b.dsp_load {
        s.push_str(&format!(" ({:.0}% DSP)", dsp_percent(l)));
    }
    s
}

/// The preset as a listing: identity and headroom, snapshots, then the blocks in signal order,
/// with their parameter values when `params` is set.
pub fn preset_summary(p: &PresetDto, params: bool) -> String {
    let mut out = String::new();
    let name = p.name.as_deref().unwrap_or("(unnamed)");
    let where_ = match (p.bank, p.index) {
        (Some(b), Some(i)) => format!(" (setlist {b}, slot {i})"),
        (None, Some(i)) => format!(" (slot {i})"),
        _ => String::new(),
    };
    out.push_str(&format!("Preset \"{name}\"{where_}"));
    if let Some(d) = p.device_name.as_deref().or(p.device_model.as_deref()) {
        out.push_str(&format!(" — {d}"));
    }
    out.push('\n');
    let used = if p.dsp_ceiling > 0.0 {
        p.dsp_load / p.dsp_ceiling * 100.0
    } else {
        0.0
    };
    out.push_str(&format!(
        "{} path · DSP {used:.0}% used, {:.0}% free\n",
        if p.split {
            "parallel (split)"
        } else {
            "serial"
        },
        (100.0 - used).max(0.0)
    ));
    if !p.snapshot_names.is_empty() {
        let snaps: Vec<String> = p
            .snapshot_names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let star = if p.active_snapshot == Some(i as i64) {
                    "*"
                } else {
                    ""
                };
                format!("{}{star} {n}", i + 1)
            })
            .collect();
        out.push_str(&format!("Snapshots (* = active): {}\n", snaps.join(", ")));
    }
    if p.dirty {
        out.push_str("Unsaved edits in the edit buffer.\n");
    }
    let mut blocks: Vec<&BlockDto> = p.blocks.iter().filter(|b| !b.is_controller).collect();
    blocks.sort_by_key(|b| (b.dsp, b.slot));
    if blocks.is_empty() {
        out.push_str("No blocks.\n");
    } else {
        out.push_str("Blocks:\n");
    }
    for b in blocks {
        out.push_str("  ");
        out.push_str(&block_line(b));
        out.push('\n');
        if params {
            if !b.params.is_empty() {
                out.push_str("      ");
                out.push_str(&params_line(&b.params));
                out.push('\n');
            }
            if !b.paired_params.is_empty() {
                out.push_str("      cab: ");
                out.push_str(&params_line(&b.paired_params));
                out.push('\n');
            }
        }
    }
    out.trim_end().to_string()
}

/// Every parameter of one block, with what it can be set to: the range in display units, or the
/// options of an enum. The detail `preset_summary` leaves out.
pub fn block_params(b: &BlockDto) -> String {
    let mut out = block_line(b);
    out.push('\n');
    let section = |out: &mut String, heading: &str, params: &[ParamDto]| {
        if params.is_empty() {
            return;
        }
        out.push_str(heading);
        for p in params {
            out.push_str(&format!("  [{}] {} = {}", p.index, p.name, param_value(p)));
            if !p.enum_labels.is_empty() {
                out.push_str(&format!("   options: {}", p.enum_labels.join(" | ")));
            } else if let Some(r) = param_range(p) {
                out.push_str(&format!("   range: {r}"));
            }
            if !p.settable {
                out.push_str("   (read-only)");
            }
            out.push('\n');
        }
    };
    section(&mut out, "Parameters:\n", &b.params);
    section(
        &mut out,
        "Paired cab/IR parameters (paired=true):\n",
        &b.paired_params,
    );
    out.trim_end().to_string()
}

/// What differs between two presets, slot by slot: blocks added, removed or swapped, bypass
/// changes, and parameter values that moved. "No differences." when nothing did.
pub fn preset_diff(a: &PresetDto, b: &PresetDto) -> String {
    let mut lines = Vec::new();
    if a.name != b.name {
        lines.push(format!(
            "name: {:?} → {:?}",
            a.name.as_deref().unwrap_or(""),
            b.name.as_deref().unwrap_or("")
        ));
    }
    if a.split != b.split {
        lines.push(format!(
            "path: {} → {}",
            if a.split { "parallel" } else { "serial" },
            if b.split { "parallel" } else { "serial" }
        ));
    }
    if a.snapshot_names != b.snapshot_names {
        lines.push(format!(
            "snapshots: {} → {}",
            a.snapshot_names.join(", "),
            b.snapshot_names.join(", ")
        ));
    }
    let mut slots: Vec<i64> = a
        .blocks
        .iter()
        .chain(&b.blocks)
        .filter(|x| !x.is_controller)
        .map(|x| x.slot)
        .collect();
    slots.sort_unstable();
    slots.dedup();
    for slot in slots {
        let ba = a.blocks.iter().find(|x| x.slot == slot && !x.is_controller);
        let bb = b.blocks.iter().find(|x| x.slot == slot && !x.is_controller);
        match (ba, bb) {
            (Some(x), None) => lines.push(format!("slot {slot}: removed {}", x.model_name)),
            (None, Some(y)) => lines.push(format!("slot {slot}: added {}", y.model_name)),
            (Some(x), Some(y)) => {
                if x.model_name != y.model_name || x.paired_model_name != y.paired_model_name {
                    lines.push(format!("slot {slot}: {} → {}", full_name(x), full_name(y)));
                    continue; // different models: their params don't line up
                }
                if x.bypassed != y.bypassed {
                    lines.push(format!(
                        "slot {slot} {}: {} → {}",
                        x.model_name,
                        onoff(x.bypassed),
                        onoff(y.bypassed)
                    ));
                }
                param_diffs(&mut lines, slot, &x.model_name, "", &x.params, &y.params);
                param_diffs(
                    &mut lines,
                    slot,
                    &x.model_name,
                    "cab ",
                    &x.paired_params,
                    &y.paired_params,
                );
            }
            (None, None) => {}
        }
    }
    if lines.is_empty() {
        "No differences.".to_string()
    } else {
        lines.join("\n")
    }
}

fn full_name(b: &BlockDto) -> String {
    match &b.paired_model_name {
        Some(c) => format!("{} + {c}", b.model_name),
        None => b.model_name.clone(),
    }
}

fn onoff(bypassed: Option<bool>) -> &'static str {
    match bypassed {
        Some(true) => "bypassed",
        _ => "active",
    }
}

fn param_diffs(
    lines: &mut Vec<String>,
    slot: i64,
    model: &str,
    prefix: &str,
    a: &[ParamDto],
    b: &[ParamDto],
) {
    for pa in a {
        let Some(pb) = b.iter().find(|p| p.name == pa.name) else {
            continue;
        };
        let (va, vb) = (param_value(pa), param_value(pb));
        if va != vb {
            lines.push(format!(
                "slot {slot} {model}: {prefix}{} {va} → {vb}",
                pa.name
            ));
        }
    }
}

/// A preset listing, one per line, with the pedal's own label where it is known.
pub fn preset_list(items: &[PresetListItem]) -> String {
    if items.is_empty() {
        return "No presets.".to_string();
    }
    items
        .iter()
        .map(|i| {
            let label = i
                .label
                .as_deref()
                .map(|l| format!("{l} "))
                .unwrap_or_default();
            let list = i
                .setlist
                .as_deref()
                .map(|s| format!("  [{s}]"))
                .unwrap_or_default();
            format!("{label}slot {:<3} {}{list}", i.index, i.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fretwire_commands::dto::{FormatRuleDto, SegStopDto};

    fn knob(name: &str, value: f64) -> ParamDto {
        ParamDto {
            name: name.into(),
            value,
            kind: "float",
            min: Some(0.0),
            max: Some(1.0),
            format: Some(NumFormatDto {
                scale: 10.0,
                offset: 0.0,
                rules: vec![FormatRuleDto {
                    lo: None,
                    hi: None,
                    mult: 1.0,
                    template: "%.1f".into(),
                }],
            }),
            settable: true,
            ..Default::default()
        }
    }

    fn block(slot: i64, model: &str, params: Vec<ParamDto>) -> BlockDto {
        BlockDto {
            slot,
            model_name: model.into(),
            params,
            ..Default::default()
        }
    }

    #[test]
    fn values_read_as_hx_edit_shows_them() {
        assert_eq!(param_value(&knob("Drive", 0.65)), "6.5");
        let sw = ParamDto {
            name: "Trails".into(),
            value: 1.0,
            kind: "bool",
            ..Default::default()
        };
        assert_eq!(param_value(&sw), "On");
        let mic = ParamDto {
            name: "Mic".into(),
            value: 2.0,
            kind: "int",
            enum_labels: vec![
                "57 Dynamic".into(),
                "409 Dynamic".into(),
                "421 Dynamic".into(),
            ],
            enum_base: 1,
            ..Default::default()
        };
        assert_eq!(
            param_value(&mic),
            "409 Dynamic",
            "labels start at enum_base"
        );
        let angle = ParamDto {
            name: "Angle".into(),
            value: 0.9,
            kind: "float",
            stops: vec![
                SegStopDto {
                    value: 0.0,
                    label: "0°".into(),
                },
                SegStopDto {
                    value: 1.0,
                    label: "45°".into(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(param_value(&angle), "45°", "nearest stop");
        assert_eq!(
            param_value(&ParamDto {
                value: 0.5,
                ..Default::default()
            }),
            "0.5"
        );
    }

    /// The display rules run backwards: what the assistant types is what HX Edit would show.
    #[test]
    fn display_units_invert_to_stored_values() {
        let d = knob("Drive", 0.0);
        assert!((raw_from_display(&d, 6.5, "") - 0.65).abs() < 1e-9);
        let delay = delay_time(0.0);
        assert_eq!(param_value(&delay_time(0.225)), "450 ms");
        assert_eq!(param_value(&delay_time(0.7)), "1.40 s");
        assert!((raw_from_display(&delay, 450.0, "") - 0.225).abs() < 1e-9);
        assert!((raw_from_display(&delay, 1400.0, "ms") - 0.7).abs() < 1e-9);
        assert!(
            (raw_from_display(&delay, 1.4, "s") - 0.7).abs() < 1e-9,
            "the unit picks the seconds rule"
        );
        assert!(
            (raw_from_display(&delay, 1.4, "") - 0.0007).abs() < 1e-9,
            "a bare 1.4 reads as the first rule's 1.4 ms"
        );
        assert_eq!(template_unit("%.0f ms"), "ms");
        assert_eq!(template_unit("%.1f%%"), "%");
        assert_eq!(template_unit("%+.1f dB"), "dB");
        assert_eq!(param_range(&d), Some("0.0–10.0".into()));
        assert_eq!(
            raw_from_display(&ParamDto::default(), 0.3, ""),
            0.3,
            "no format: as stored"
        );
    }

    #[test]
    fn a_diff_names_what_moved() {
        let a = PresetDto {
            name: Some("A".into()),
            blocks: vec![
                block(1, "Deluxe Comp", vec![knob("Level", 0.5)]),
                block(2, "Scream 808", vec![knob("Gain", 0.4)]),
            ],
            ..Default::default()
        };
        let mut b = PresetDto {
            name: Some("A".into()),
            blocks: vec![
                block(1, "Deluxe Comp", vec![knob("Level", 0.7)]),
                block(3, "Simple Delay", vec![]),
            ],
            ..Default::default()
        };
        b.blocks[0].bypassed = Some(true);
        let d = preset_diff(&a, &b);
        assert_eq!(
            d,
            "slot 1 Deluxe Comp: active → bypassed\n\
             slot 1 Deluxe Comp: Level 5.0 → 7.0\n\
             slot 2: removed Scream 808\n\
             slot 3: added Simple Delay"
        );
        assert_eq!(preset_diff(&a, &a), "No differences.");
    }

    #[test]
    fn a_summary_lists_blocks_in_order() {
        let p = PresetDto {
            name: Some("Clean".into()),
            index: Some(5),
            bank: Some(0),
            dsp_load: 37.5,
            dsp_ceiling: 75.0,
            snapshot_names: vec!["Verse".into(), "Chorus".into()],
            active_snapshot: Some(1),
            blocks: vec![
                block(2, "Simple Delay", vec![]),
                BlockDto {
                    footswitch: 1,
                    bypassed: Some(true),
                    ..block(1, "Scream 808", vec![knob("Gain", 0.4)])
                },
            ],
            ..Default::default()
        };
        let s = preset_summary(&p, true);
        assert!(
            s.starts_with(
                "Preset \"Clean\" (setlist 0, slot 5)\nserial path · DSP 50% used, 50% free\n"
            ),
            "{s}"
        );
        assert!(
            s.contains("Snapshots (* = active): 1 Verse, 2* Chorus\n"),
            "{s}"
        );
        let scream = s.find("Scream 808").unwrap();
        let delay = s.find("Simple Delay").unwrap();
        assert!(scream < delay, "slot order");
        assert!(s.contains("[FS1] — BYPASSED"), "{s}");
        assert!(s.contains("      Gain 4.0"), "{s}");
        assert!(
            !preset_summary(&p, false).contains("Gain"),
            "params off by default"
        );
    }

    /// A range-switched delay time: ms below a second, seconds above (via a unitsMultiplier),
    /// stored as a fraction of two seconds.
    fn delay_time(value: f64) -> ParamDto {
        ParamDto {
            name: "Time".into(),
            value,
            kind: "float",
            format: Some(NumFormatDto {
                scale: 2000.0,
                offset: 0.0,
                rules: vec![
                    FormatRuleDto {
                        lo: Some(0.0),
                        hi: Some(1000.0),
                        mult: 1.0,
                        template: "%.0f ms".into(),
                    },
                    FormatRuleDto {
                        lo: Some(1000.0),
                        hi: None,
                        mult: 0.001,
                        template: "%.2f s".into(),
                    },
                ],
            }),
            ..Default::default()
        }
    }
}

/// Renders a real preset through the reference data — only where that data is imported.
#[cfg(all(test, have_bundled_data))]
mod data_tests {
    use super::*;
    use fretwire_core::editor::Catalog;

    #[test]
    fn a_captured_preset_summarizes_with_names() {
        let catalog = Catalog::from_data_dir(&fretwire_core::data_dir())
            .expect("load reference data (run `fretwire import-data`)");
        let raw = include_bytes!("../../../captures/preset1_stream.msgpack.bin");
        let preset = catalog.load_preset(raw).expect("decode the fixture");
        let dto = PresetDto::from(&preset);
        let s = preset_summary(&dto, true);
        assert!(s.contains("Blocks:"), "{s}");
        assert!(!dto.blocks.is_empty());
        // With the data present, the first block's params carry names and display values —
        // the whole point of decoding through the catalog rather than dumping the stream.
        let b = dto
            .blocks
            .iter()
            .find(|b| !b.params.is_empty())
            .expect("a block with params");
        let detail = block_params(b);
        assert!(detail.contains("Parameters:"), "{detail}");
        assert!(preset_diff(&dto, &dto) == "No differences.");
    }
}
