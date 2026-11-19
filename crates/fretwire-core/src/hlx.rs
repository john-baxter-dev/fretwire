//! Direct, offline conversion of an HX Stomp `.hlx` preset. No USB or flash writes.
//!
//! The existing tone converter needs a device-authored donor for undecoded fields. Keep its
//! omissions in the result: callers must show them before offering a restore. This first import
//! path accepts Stomp presets only; it does not adapt Floor/XL presets to a smaller device.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};

use fretwire_data::Preset;
use fretwire_data::rmpv::Value;
use fretwire_data::stream::{PresetStream, map_get};
use fretwire_data::symbols::DeviceSymbols;
use fretwire_data::tone::{Conversion, ValueTypes};
use fretwire_protocol::{Device, PID_HX_STOMP};

use crate::{EditorPreset, Error, Result, Session};

pub const STOMP_DEVICE_ID: i64 = 0x0021_0006;
const MAX_PRESET_NAME_BYTES: usize = 32;

/// A parsed, validated Stomp preset, before a donor or the model catalog is needed.
pub struct HlxPreset(Preset);

#[derive(Clone)]
pub struct PreparedHlx {
    pub name: String,
    pub raw: Vec<u8>,
    pub report: Conversion,
    pub active_snapshot: i64,
}

#[derive(Clone, Copy)]
pub(crate) enum Recovery {
    Reload(i64, i64),
    Confirm(i64, i64),
}

impl Session {
    /// Cache the exact conversion shown in the preview, using the currently loaded preset as donor.
    pub fn preview_hlx(&mut self, source: &HlxPreset) -> Result<(u32, &PreparedHlx)> {
        static NEXT_ID: AtomicU32 = AtomicU32::new(1);

        if self.device().pid != PID_HX_STOMP {
            return Err(Error::Invalid(
                "direct .hlx import currently supports the HX Stomp only".into(),
            ));
        }
        let donor = PresetStream::parse(&self.read_preset_raw()?)?;
        let prepared =
            source.prepare(&donor, &self.catalog().symbols, &self.catalog().value_types)?;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (_, prepared) = self.hlx_preview.insert((id, prepared));
        Ok((id, prepared))
    }

    pub fn restore_hlx_preview(
        &mut self,
        preview_id: u32,
        bank: i64,
        slot: i64,
        allow_partial: bool,
    ) -> Result<EditorPreset> {
        let Some((_, prepared)) = self
            .hlx_preview
            .as_ref()
            .filter(|(id, _)| *id == preview_id)
        else {
            return Err(Error::Invalid(
                "this .hlx preview expired; preview the file again".into(),
            ));
        };
        let prepared = prepared.clone();
        prepared.write_verified(self, bank, slot, allow_partial)?;
        self.hlx_preview = None;
        read_saved_hlx(self, bank, slot)
    }

    pub(crate) fn reload_hlx_destination(&mut self) -> Result<()> {
        if let Some(Recovery::Reload(bank, slot)) = self.hlx_recovery {
            self.goto_preset(bank, slot)?;
            self.read_preset_raw_with_identity()?;
            self.hlx_recovery = Some(Recovery::Confirm(bank, slot));
        }
        Ok(())
    }
}

fn read_saved_hlx(session: &mut Session, bank: i64, slot: i64) -> Result<EditorPreset> {
    session.hlx_recovery = Some(Recovery::Confirm(bank, slot));
    session.read_preset_with_identity().map_err(|error| {
        Error::Invalid(format!(
            "the preset was saved, but the editor could not read it: {error}"
        ))
    })
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
        if preset.name().len() > MAX_PRESET_NAME_BYTES {
            return Err(Error::Invalid(format!(
                "the preset name must be at most {MAX_PRESET_NAME_BYTES} UTF-8 bytes"
            )));
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
        report.not_carried.insert(0, "Undecoded settings, including Stomp input/output routing, come from the donor preset that was loaded when this preview was prepared.".into());
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

    /// Compare the entire decoded document, including its snapshot selection.
    pub fn verify_readback(&self, actual: &PresetStream) -> Result<()> {
        Self::verify_document(&PresetStream::parse(&self.raw)?, actual, true)
    }

    fn verify_document(
        expected: &PresetStream,
        actual: &PresetStream,
        include_selection: bool,
    ) -> Result<()> {
        if !same_document(
            &verification_document(expected, include_selection),
            &verification_document(actual, include_selection),
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
        session: &mut Session,
        bank: i64,
        slot: i64,
        allow_partial: bool,
    ) -> Result<EditorPreset> {
        self.write_verified(session, bank, slot, allow_partial)?;
        read_saved_hlx(session, bank, slot)
    }

    fn write_verified(
        &self,
        session: &mut Session,
        bank: i64,
        slot: i64,
        allow_partial: bool,
    ) -> Result<()> {
        validate_destination(bank, slot)?;
        self.require_acknowledgement(allow_partial)?;
        if session.device().pid != PID_HX_STOMP {
            return Err(Error::Invalid(".hlx restore requires an HX Stomp".into()));
        }
        let expected = PresetStream::parse(&self.raw)?;
        if session.hlx_recovery.is_some() {
            session.read_preset_with_identity()?;
        }
        // The selection may take effect even if the host misses the reply. Undo entries from the
        // previous slot must never be available after this point.
        session.clear_history();
        let save = (|| {
            session.goto_preset(bank, slot)?;
            // Open the freshly selected edit buffer before the chunked write.
            session.read_preset_raw_with_identity()?;
            session.write_preset(expected.to_blob())?;
            session.confirm_next_preset_address(bank, slot);
            let actual = PresetStream::parse(&session.read_preset_raw_with_identity()?)?;
            // A snapshot switch can recall stored parameters and bypass states over the live
            // values. Verify those values before switching, then check the live selection via
            // op 23, separately from the document's 10/6 field.
            Self::verify_document(&expected, &actual, false)?;
            let info = session
                .last_identity()
                .filter(|i| (i.bank, i.index) == (bank, slot))
                .ok_or_else(|| {
                    Error::Invalid("the .hlx destination changed during import".into())
                })?;
            let mut snapshot = info.snapshot;
            if snapshot != Some(self.active_snapshot) {
                session.set_snapshot(self.active_snapshot)?;
                // The switch ACK can precede the identity update. Retry a confirmed read of
                // this slot if the device still reports the previous snapshot.
                for attempt in 0..3 {
                    if attempt > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(40));
                    }
                    session.confirm_next_preset_address(bank, slot);
                    session.read_preset_raw_with_identity()?;
                    snapshot = session.last_identity().and_then(|i| i.snapshot);
                    if snapshot == Some(self.active_snapshot) {
                        break;
                    }
                }
            }
            if snapshot != Some(self.active_snapshot) {
                return Err(Error::Invalid(
                    "the device did not confirm the imported preset's snapshot".into(),
                ));
            }
            session.save_preset(bank, slot, &self.name)
        })();
        if let Err(error) = save {
            // Reload the destination even if the op-21 write or flash save returned an error:
            // the device may have applied a command whose reply never arrived.
            session.hlx_recovery = Some(Recovery::Reload(bank, slot));
            if let Err(revert_error) = session.reload_hlx_destination() {
                return Err(Error::Invalid(format!(
                    "{error}; reloading the stored preset also failed: {revert_error}"
                )));
            }
            return Err(error);
        }
        Ok(())
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
fn verification_document(stream: &PresetStream, include_selection: bool) -> Value {
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
    if !include_selection && let Some(Value::Map(fields)) = field_mut(&mut document, 10) {
        fields.retain(|(key, _)| key.as_i64() != Some(6));
    }
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
    let size = Device::by_pid(PID_HX_STOMP)
        .and_then(|d| d.setlist_size)
        .ok_or_else(|| Error::Invalid("no setlist size is defined for the HX Stomp".into()))?
        as i64;
    if bank != 0 || !(0..size).contains(&slot) {
        return Err(Error::Invalid(format!(
            "choose an HX Stomp slot in bank 0, from 0 to {}",
            size - 1
        )));
    }
    Ok(())
}
