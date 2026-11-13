//! Direct, offline conversion of an HX Stomp `.hlx` preset. No USB or flash writes.
//!
//! The existing tone converter needs a device-authored donor for undecoded fields. Keep its
//! omissions in the result: callers must show them before offering a restore. This first import
//! path accepts Stomp presets only; it does not adapt Floor/XL presets to a smaller device.

use std::collections::HashSet;

use fretwire_data::Preset;
use fretwire_data::rmpv::Value;
use fretwire_data::stream::{PresetStream, map_get};
use fretwire_data::symbols::DeviceSymbols;
use fretwire_data::tone::{Conversion, ValueTypes};

use crate::{Error, Result};

pub const STOMP_DEVICE_ID: i64 = 0x0021_0006;

/// A parsed, validated Stomp preset, before a donor or the model catalog is needed.
pub struct HlxPreset(Preset);

pub struct PreparedHlx {
    pub name: String,
    pub raw: Vec<u8>,
    pub report: Conversion,
    pub active_snapshot: i64,
}

impl HlxPreset {
    pub fn parse(json: &str) -> Result<Self> {
        let preset = Preset::from_slice(json.as_bytes())?;
        if preset.schema.as_deref() != Some("L6Preset") {
            return Err(Error::Invalid(
                "expected an .hlx file with schema L6Preset".into(),
            ));
        }
        if preset.data.device != STOMP_DEVICE_ID {
            return Err(Error::Invalid(format!(
                "direct .hlx import currently accepts HX Stomp presets only (device 0x{STOMP_DEVICE_ID:08x}); \
                 this file declares 0x{:08x}",
                preset.data.device
            )));
        }
        if preset.name().trim().is_empty() || preset.name().contains('\0') {
            return Err(Error::Invalid(
                "the preset needs a nonempty name without NUL characters".into(),
            ));
        }
        let tone = &preset.data.tone;
        let dsp = tone
            .get("dsp0")
            .and_then(|v| v.as_object())
            .ok_or_else(|| Error::Invalid("the .hlx preset has no dsp0 object".into()))?;
        let topology = tone
            .get("global")
            .and_then(|v| v.get("@topology0"))
            .and_then(|v| v.as_str());
        if let Some(snapshot) = tone.get("global").and_then(|g| g.get("@current_snapshot")) {
            if !matches!(snapshot.as_i64(), Some(0..=2)) {
                return Err(Error::Invalid(
                    "invalid HX Stomp active snapshot (expected 0 to 2)".into(),
                ));
            }
            let key = format!("snapshot{}", snapshot.as_i64().unwrap());
            if !tone.get(&key).is_some_and(|v| v.is_object()) {
                return Err(Error::Invalid(format!("the file selects missing {key}")));
            }
        }
        if !matches!(topology, Some("A" | "SABJ")) {
            return Err(Error::Invalid(
                "direct HX Stomp import currently requires topology A or SABJ".into(),
            ));
        }
        // A topology is a coordinated group: changing its code while retaining the
        // donor's active split/join can make firmware discard the entire DSP group.
        for node in ["split", "join"] {
            let value = dsp.get(node).ok_or_else(|| {
                Error::Invalid(format!("dsp0.{node}: required routing node is missing"))
            })?;
            let model = value
                .get("@model")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let prefix = if node == "split" {
                "HD2_AppDSPFlowSplit"
            } else {
                "HD2_AppDSPFlowJoin"
            };
            if !model.starts_with(prefix)
                || !matches!(value.get("@position").and_then(|v| v.as_i64()), Some(0..=7))
            {
                return Err(Error::Invalid(format!(
                    "dsp0.{node}: invalid routing model or position"
                )));
            }
        }
        for (key, value) in tone {
            if key.starts_with("dsp")
                && key != "dsp0"
                && value
                    .as_object()
                    .is_some_and(|d| d.keys().any(|k| k.starts_with("block")))
            {
                return Err(Error::Invalid(format!(
                    "{key} contains blocks, but an HX Stomp has one DSP"
                )));
            }
        }
        let mut positions = HashSet::new();
        for (key, block) in dsp.iter().filter(|(key, _)| key.starts_with("block")) {
            let path = block.get("@path").and_then(|v| v.as_i64());
            let position = block.get("@position").and_then(|v| v.as_i64());
            let (Some(path @ 0..=1), Some(position @ 0..=7)) = (path, position) else {
                return Err(Error::Invalid(format!(
                    "dsp0.{key}: invalid @path/@position for an HX Stomp"
                )));
            };
            if path == 1 && topology == Some("A") {
                return Err(Error::Invalid(format!(
                    "dsp0.{key}: path B needs a parallel topology"
                )));
            }
            if !positions.insert((path, position)) {
                return Err(Error::Invalid(format!(
                    "dsp0.{key}: two blocks occupy the same path and position"
                )));
            }
        }
        if positions.len() > 8 {
            return Err(Error::Invalid(format!(
                "this preset has {} blocks; an HX Stomp holds at most 8",
                positions.len()
            )));
        }
        Ok(Self(preset))
    }

    pub fn name(&self) -> &str {
        self.0.name()
    }

    pub fn prepare(
        &self,
        donor: &PresetStream,
        symbols: &DeviceSymbols,
        types: &ValueTypes,
    ) -> Result<PreparedHlx> {
        if donor.device_model().as_deref() != Some("P33") {
            return Err(Error::Invalid(
                "the .hlx donor must come from an HX Stomp (P33)".into(),
            ));
        }
        fretwire_data::tone::validate_tone_parameters(&self.0.data.tone, symbols)?;
        let mut target = donor.clone();
        let mut report =
            fretwire_data::tone::apply_tone(&mut target, &self.0.data.tone, symbols, types)?;
        let active_snapshot = self
            .0
            .data
            .tone
            .get("global")
            .and_then(|g| g.get("@current_snapshot"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        // Live Stomp switching 0 → 2 changes 10/6 to 2; 10/8 counts
        // controller places and must not be used as the selected snapshot.
        if let Value::Map(groups) = target.preset_mut()
            && let Some((_, Value::Map(snapshot))) =
                groups.iter_mut().find(|(key, _)| key.as_i64() == Some(10))
            && let Some((_, selected)) =
                snapshot.iter_mut().find(|(key, _)| key.as_i64() == Some(6))
        {
            *selected = Value::from(active_snapshot);
        }
        report.not_carried.insert(0, "Undecoded settings, including Stomp input/output routing, use the currently loaded preset as a donor. They do not come from the destination slot.".into());
        for key in self.0.data.tone.keys() {
            if !matches!(
                key.as_str(),
                "dsp0" | "dsp1" | "global" | "controller" | "footswitch" | "irUuidTable" | "variax"
            ) && !key.starts_with("snapshot")
            {
                report
                    .not_carried
                    .push(format!("{key}: this tone section is not imported"));
            }
        }
        Ok(PreparedHlx {
            name: self.name().to_string(),
            raw: target.to_stream(0),
            report,
            active_snapshot,
        })
    }
}

impl PreparedHlx {
    pub fn require_acknowledgement(&self, allow_partial: bool) -> Result<()> {
        if !allow_partial && !self.report.not_carried.is_empty() {
            return Err(Error::Invalid(
                "review and acknowledge the .hlx conversion omissions before restoring".into(),
            ));
        }
        Ok(())
    }

    /// A transport ACK alone is not proof that firmware accepted the DSP document.
    /// Compare the decoded edit buffer before allowing a persistent save.
    pub fn verify_readback(&self, actual: &PresetStream) -> Result<()> {
        let expected = PresetStream::parse(&self.raw)?;
        if !same_document(
            &verification_document(&expected),
            &verification_document(actual),
        ) {
            return Err(Error::Invalid(
                "the device changed or rejected part of the converted .hlx preset; it has not been saved".into(),
            ));
        }
        Ok(())
    }

    /// Import into an explicit slot, verifying the edit buffer before writing flash.
    pub fn restore(
        &self,
        session: &mut crate::Session,
        bank: i64,
        slot: i64,
        allow_partial: bool,
    ) -> Result<crate::EditorPreset> {
        validate_destination(bank, slot)?;
        self.require_acknowledgement(allow_partial)?;
        if session.device().model_code != Some("P33") {
            return Err(Error::Invalid(".hlx restore requires an HX Stomp".into()));
        }
        session.goto_preset(bank, slot)?;
        // Open the freshly selected edit buffer before the chunked write.
        session.read_preset_raw()?;
        let expected = PresetStream::parse(&self.raw)?;
        session.write_preset(expected.to_blob())?;
        // A whole-document write resets the live selection. Restore the file's
        // snapshot before saving, so save commits its live controller values
        // into the matching scene.
        session.set_snapshot(self.active_snapshot)?;
        let actual = PresetStream::parse(&session.read_preset_raw()?)?;
        if let Err(error) = self.verify_readback(&actual) {
            // Discard a semantically rejected edit buffer; the stored slot is intact.
            if let Err(revert_error) = session.revert_preset() {
                return Err(Error::Invalid(format!(
                    "{error}; reloading the stored preset also failed: {revert_error}"
                )));
            }
            return Err(error);
        }
        session.save_preset(bank, slot, &self.name)?;
        session.clear_history();
        session.read_preset()
    }

    /// A single-preset export, readable by `backup-show` and the existing restore dialog.
    pub fn into_backup(self) -> crate::backup::Backup {
        crate::backup::Backup {
            device: "HX Stomp".into(),
            presets: vec![crate::backup::BackupPreset {
                bank: 0,
                index: 0,
                name: self.name,
                raw: self.raw,
            }],
            ..Default::default()
        }
    }
}

/// Stomp regenerates a bypass binding's display name and model colour from its
/// referenced block. Custom labels/colours live on the enclosing entry and are
/// still compared, together with the binding target, type, enable and mode.
fn verification_document(stream: &PresetStream) -> Value {
    fn field_mut(value: &mut Value, key: i64) -> Option<&mut Value> {
        let Value::Map(entries) = value else {
            return None;
        };
        entries
            .iter_mut()
            .find(|(k, _)| k.as_i64() == Some(key))
            .map(|(_, v)| v)
    }
    let mut document = stream.preset.clone();
    if let Some(Value::Array(switches)) = field_mut(&mut document, 3).and_then(|v| field_mut(v, 8))
    {
        for switch in switches {
            let Value::Array(bindings) = switch else {
                continue;
            };
            for binding in bindings {
                let Some(node) = field_mut(binding, 11) else {
                    continue;
                };
                if map_get(node, 0).and_then(Value::as_i64) == Some(1)
                    && let Value::Map(fields) = node
                {
                    fields.retain(|(key, _)| !matches!(key.as_i64(), Some(5 | 6)));
                }
            }
        }
    }
    document
}

// MessagePack maps are unordered. Firmware may reorder a controller row while
// preserving every key, value and type; arrays still have positional meaning.
fn same_document(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Map(a), Value::Map(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.iter()
                        .find(|(k, _)| k == key)
                        .is_some_and(|(_, other)| same_document(value, other))
                })
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_document(a, b))
        }
        _ => a == b,
    }
}

/// Stomp destination slots are zero-based, in its single setlist.
pub fn validate_destination(bank: i64, slot: i64) -> Result<()> {
    let size = fretwire_protocol::Device::by_pid(0x4246)
        .and_then(|d| d.setlist_size)
        .expect("known HX Stomp setlist size") as i64;
    if bank != 0 || !(0..size).contains(&slot) {
        return Err(Error::Invalid(format!(
            "choose an HX Stomp slot in bank 0, from 0 to {}",
            size - 1
        )));
    }
    Ok(())
}
