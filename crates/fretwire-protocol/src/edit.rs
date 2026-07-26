//! Edit-channel command bodies.
//!
//! The body carried as the TLV value of an opcode-`0x0006` edit frame is **MessagePack** (the same
//! encoding as the preset stream). Decoded shape, confirmed across many single-knob captures
//! (`docs/protocol.md`):
//! ```text
//! {
//!   102: <u16>,        // a session-wide running transaction counter (whole 16-bit value)
//!   100: <op>,         // operation: 30 = set-value, 41 = bypass
//!   101: {             // the target
//!     98: <slot>,      // block slot index (= preset slot)
//!     // --- bypass (op 41): ---
//!     59: <bool>       //   the new bypass state (true = on)
//!     // --- set-value (op 30): ---
//!     29: true, 26: 0, // constant descriptor flags for knob params
//!     28: <param_idx>, // the PARAMETER INDEX in the model's device (Helix.sym) order
//!     119: <value>     // the new value (float32 for knobs; int for enums; bool for switches)
//!   }
//! }
//! ```
//! **Key result:** the parameter is selected by its **index** (key 28) in the model's device param
//! order — so parameter editing is *computable from shipped data* (`Helix.sym`), no per-param
//! captures required. Verified: Bucket Brigade Mix→3, 70s Chorus Mix→4, Dynamic Ambience
//! PreDelay→1/Mix→5/LowCut→6/Level→8 — each equals the param's index in its `Helix.sym` list.
//!
//! Builders [`bypass`] and [`set_value`] reproduce captured device bytes exactly
//! (`fretwire-data/tests/edit_body_msgpack.rs`, and this module's tests).

use crate::{Error, Result};
use rmpv::Value;

/// Envelope key: the u16 running transaction counter.
pub const K_TXN: i64 = 102;
/// Envelope key: the operation id.
pub const K_OP: i64 = 100;
/// Envelope key: the target sub-map.
pub const K_TARGET: i64 = 101;

/// Target key: block slot index.
pub const K_SLOT: i64 = 98;
/// Target key: parameter index (set-value).
pub const K_PARAM_INDEX: i64 = 28;
/// Target key: the value field (set-value).
pub const K_VALUE: i64 = 119;
/// Target key: the bypass bool (bypass op).
pub const K_BYPASS_VALUE: i64 = 59;
// Constant descriptor flag seen on knob set-value edits (always `true`).
const K_FLAG_29: i64 = 29;
/// Set-value target key 26: the **sub-model selector** inside a block — `0` = the block's main model
/// (amp/effect), `1` = the **paired cab/IR** fused into the same slot. (Same key number as the
/// model-ref's [`K_PAIRED_INDEX`], different context.) Decoded from the cab-mic/distance captures:
/// a paired-cab param edit is identical to a main one but with `26:1`. The main model is `26:0`.
pub const K_MODEL_SEL: i64 = 26;
/// Sub-model selector values for [`K_MODEL_SEL`].
pub const MODEL_MAIN: i64 = 0;
pub const MODEL_PAIRED: i64 = 1;

/// Select-target key: bank index.
pub const K_SELECT_BANK: i64 = 107;
/// Select-target key: preset index within the bank.
pub const K_SELECT_PRESET: i64 = 108;
/// Target key: a preset/snapshot **name** — a NUL-terminated string (save-preset, rename-snapshot,
/// and the value behind browse list entries).
pub const K_NAME: i64 = 109;
/// Read-prepare-target key (seen value 128); meaning TBD, replicated from the connect capture.
pub const K_READ_PREP: i64 = 118;
/// Target key: snapshot index (switch-snapshot / rename-snapshot ops).
pub const K_SNAPSHOT: i64 = 92;
/// Target key: the global/input **setting id** (op 25). Same key number as [`K_READ_PREP`], different
/// context. The id space is only partly mapped — `134` is a 3-state input setting (seen 0/1/2).
pub const K_SETTING_ID: i64 = 118;

/// Operation id (envelope key 100).
pub const OP_SET_VALUE: i64 = 30;
pub const OP_BYPASS: i64 = 41;
/// Operation id 20: **select (load) a preset** — `{107: bank, 108: preset}`. Changes device state.
pub const OP_SELECT: i64 = 20;
/// Operation id 76: open the current edit buffer for a (non-destructive) read.
pub const OP_READ_OPEN: i64 = 76;
/// Operation id 24: read-sequence prepare step (`{118: 128}` in the connect capture).
pub const OP_READ_PREP: i64 = 24;
/// Operation id 23: read-sequence query step (nil target; reply carries the preset identity).
pub const OP_READ_INFO: i64 = 23;
/// Operation id 22: start the paged stream of the opened edit buffer.
pub const OP_STREAM_START: i64 = 22;
/// Operation id 88: **switch the active snapshot** — `{92: index}`. Changes device state.
pub const OP_SWITCH_SNAPSHOT: i64 = 88;
/// Operation id 71: **save the current edit buffer to a preset slot** — `{107: bank, 108: slot,
/// 109: name}`. **Persistent write** (overwrites the slot in device flash).
pub const OP_SAVE_PRESET: i64 = 71;
/// Operation id 89: **rename a snapshot** — `{92: index, 109: name}`.
pub const OP_RENAME_SNAPSHOT: i64 = 89;
/// Operation id 6: **rename a preset by slot, name-only** — `{107: bank, 108: slot, 109: name}`. Rides
/// the **primary** channel (unlike save, op 71, on the edit channel). Unlike save it does **not**
/// commit the edit buffer: only the stored name changes, any pending param edits are discarded.
/// Decoded from `change_amp_drive_rename_..._name_sticks_change_doesnt.pcapng` (the pending drive
/// edit did not persist across the rename). Device ACKs with `{103:1}`.
pub const OP_RENAME_PRESET: i64 = 6;
/// Operation id 25: **set a global/input setting** — `{118: id, 119: value}`. Not block-addressed.
pub const OP_SETTING: i64 = 25;
/// Operation id 40: **swap a block's model** — `{98: slot, 100: {23: flag, 25: index, 26: paired}}`.
/// Writes the block's model-ref directly; the device then resets the block's params to the new
/// model's defaults. Decoded from `model_swap_delay_then_reverb.pcapng`.
pub const OP_SWAP_MODEL: i64 = 40;

/// Model-ref sub-map key (inside the swap target): the `{23,25,26}` model-ref — same shape as the
/// preset's key `24`. (Note: this is key 100 *nested in the target*, distinct from the envelope's
/// [`K_OP`] = 100.)
pub const K_MODEL_REF: i64 = 100;
/// Model-ref key: a flag (observed `false` on swaps).
pub const K_MODEL_FLAG: i64 = 23;
/// Operation id 43: **move a block** to a new slot — `{75: src_slot, 76: dst_slot}` (the dst slot
/// encodes the row: a parallel-path slot moves the block to row B). **The destination slot must be
/// empty** — op 43 relocates into a free slot, it does not insert/shift. To reorder among occupied
/// slots, bubble through empty slots (a sequence of single moves), as HX Edit does. Decoded from the
/// move captures.
pub const OP_MOVE_BLOCK: i64 = 43;
/// Move target keys: 75 = source slot, 76 = destination slot.
pub const K_MOVE_SRC: i64 = 75;
pub const K_MOVE_DST: i64 = 76;
/// Operation id 28: **delete the block at a slot** — `{98: slot}`. A surgical delete (the device
/// clears the slot and removes any footswitch binding of that block itself, leaving other bindings
/// intact — unlike a whole-preset write, op 21, which makes the device re-derive and wipe the whole
/// footswitch layout). HX Edit sends it optionally preceded by op 78. Decoded from the delete captures.
pub const OP_DELETE_BLOCK: i64 = 28;
/// Operation id 78: **begin a structural edit** on a slot — `{98: slot, 26: 0}`. HX Edit sends this
/// immediately before each move (op 43) / add (op 39) in a drag, naming the slot the operation acts
/// on (`26:0` = the main sub-model, as in set-value). Decoded from `one_by_one_move_all_blocks…`.
pub const OP_BEGIN_STRUCT: i64 = 78;
/// Operation id 21: **whole-preset write** — `{110: <blob>}`, where the blob is the same
/// `magic ⧺ header ⧺ preset-map` sequence the read stream carries (under key 104). The blob is
/// wrapped as a MessagePack **str** (the device used `str16`). HX Edit uses this for edits that
/// surgical ops can't express (multi-slot reorder, delete, parallel restructure). Decoded from
/// `move_EQ_right_two_slots.pcapng`. **Writes the edit buffer (not flash) — recoverable by reload.**
pub const OP_WRITE_PRESET: i64 = 21;
/// Write-preset target key: the preset blob.
pub const K_WRITE_BLOB: i64 = 110;
/// Operation id 39: **add a block** at a slot — `{98: slot, 99: <block-spec>}`. Decoded from
/// `add_simple_eq_at_beginning_of_chain`.
pub const OP_ADD_BLOCK: i64 = 39;
/// Add target keys: 99 = the new block's spec map `{19: node_kind, 20: <content>}`; the content map
/// holds `24: <model-ref>` plus `9` and `10` (enabled). Node kind 6 = a normal DSP block.
pub const K_ADD_SPEC: i64 = 99;
pub const K_NODE_KIND: i64 = 19;
pub const K_BLOCK_CONTENT: i64 = 20;
pub const K_BLOCK_MODEL: i64 = 24;
pub const K_BLOCK_FLAG9: i64 = 9;
pub const K_BLOCK_ENABLED: i64 = 10;
pub const NODE_KIND_BLOCK: i64 = 6;
/// Model-ref key: the model's index in `Helix.sym` order (the block's identity).
pub const K_MODEL_INDEX: i64 = 25;
/// Model-ref key: paired cab/IR index in `Helix.sym` (`-1` = none).
pub const K_PAIRED_INDEX: i64 = 26;

/// A decoded edit-channel command body.
#[derive(Debug, Clone, PartialEq)]
pub struct EditBody {
    /// Key 102 — the running transaction counter (whole u16).
    pub txn: u16,
    /// Key 100 — the operation (`OP_SET_VALUE` / `OP_BYPASS`).
    pub op: i64,
    /// Target block slot (key 98).
    pub slot: Option<i64>,
    /// Parameter index within the model's device order (key 28; set-value only).
    pub param_index: Option<i64>,
    /// The value written (key 119 for set-value, key 59 for bypass).
    pub value: EditValue,
    /// The decoded root, for fields not surfaced above.
    pub raw: Value,
}

/// The value an edit writes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditValue {
    Bool(bool),
    Float(f32),
    Int(i64),
    None,
}

impl EditValue {
    fn from_value(v: &Value) -> EditValue {
        match v {
            Value::Boolean(b) => EditValue::Bool(*b),
            Value::F32(f) => EditValue::Float(*f),
            Value::F64(f) => EditValue::Float(*f as f32),
            Value::Integer(i) => i.as_i64().map(EditValue::Int).unwrap_or(EditValue::None),
            _ => EditValue::None,
        }
    }
}

impl EditBody {
    /// Parse an edit body (the TLV value bytes) from its MessagePack encoding.
    pub fn parse(body: &[u8]) -> Result<EditBody> {
        let mut cur = body;
        let root = rmpv::decode::read_value(&mut cur)
            .map_err(|e| Error::Edit(format!("msgpack decode: {e}")))?;
        if !matches!(root, Value::Map(_)) {
            return Err(Error::Edit("edit body root is not a map".into()));
        }
        let txn = get(&root, K_TXN).and_then(Value::as_i64).unwrap_or(0) as u16;
        let op = get(&root, K_OP).and_then(Value::as_i64).unwrap_or(-1);
        let target = get(&root, K_TARGET);
        let slot = target.and_then(|t| get(t, K_SLOT)).and_then(Value::as_i64);
        let param_index = target
            .and_then(|t| get(t, K_PARAM_INDEX))
            .and_then(Value::as_i64);
        let value = target
            .and_then(|t| get(t, K_VALUE).or_else(|| get(t, K_BYPASS_VALUE)))
            .map(EditValue::from_value)
            .unwrap_or(EditValue::None);

        Ok(EditBody {
            txn,
            op,
            slot,
            param_index,
            value,
            raw: root,
        })
    }

    /// Re-encode this body to MessagePack (round-trips a parsed body byte-exact).
    pub fn to_msgpack(&self) -> Vec<u8> {
        let mut out = Vec::new();
        rmpv::encode::write_value(&mut out, &self.raw)
            .expect("msgpack encode to Vec is infallible");
        out
    }

    /// Wrap this body as the value of an opcode-`0x0006` edit TLV (host command).
    pub fn to_tlv(&self) -> crate::Tlv {
        crate::Tlv::command(crate::op::PARAM_SET, self.to_msgpack())
    }
}

fn encode(root: Value) -> Vec<u8> {
    let mut out = Vec::new();
    rmpv::encode::write_value(&mut out, &root).expect("msgpack encode to Vec is infallible");
    out
}

/// Build a **bypass/enable** edit body (op 41): set the block in `slot`'s **enabled** state
/// (key 59 — `true` = block active/on, `false` = bypassed), with transaction counter `txn`.
/// Reproduces the device's bytes exactly.
pub fn bypass(slot: i64, enabled: bool, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_BYPASS)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SLOT), Value::from(slot)),
                (Value::from(K_BYPASS_VALUE), Value::from(enabled)),
            ]),
        ),
    ]))
}

/// Build a **set-value** edit body for a knob/float parameter on the block's **main** model: set
/// parameter `param_index` (its index in the model's `Helix.sym` device order) of the block in
/// `slot` to `value`. Reproduces the device's bytes exactly.
pub fn set_value(slot: i64, param_index: i64, value: f32, txn: u16) -> Vec<u8> {
    set_value_on(slot, MODEL_MAIN, param_index, EditValue::Float(value), txn)
}

/// Build a **set-value** edit body for a parameter on the block's **paired cab/IR** (the second model
/// fused into an amp+cab slot): set parameter `param_index` (its index in the cab's `Helix.sym`
/// order) of the block in `slot` to `value`. Same envelope as [`set_value`] but selects the paired
/// model (`26:1`). Reproduces the device's bytes exactly (cab mic-distance/position/angle captures).
pub fn set_paired_value(slot: i64, param_index: i64, value: f32, txn: u16) -> Vec<u8> {
    set_value_on(
        slot,
        MODEL_PAIRED,
        param_index,
        EditValue::Float(value),
        txn,
    )
}

/// Build a **set-value** edit body, fully parameterized: choose the sub-model (`model_sel` —
/// [`MODEL_MAIN`] or [`MODEL_PAIRED`]) and the value's wire type ([`EditValue`]). Continuous knobs
/// are `Float`; enum/list params (e.g. the cab's mic selection) are `Int`; switches are `Bool`.
/// (`{102:txn, 100:30, 101:{98:slot, 29:true, 26:model_sel, 28:param_index, 119:value}}`.)
pub fn set_value_on(
    slot: i64,
    model_sel: i64,
    param_index: i64,
    value: EditValue,
    txn: u16,
) -> Vec<u8> {
    let wire_value = match value {
        EditValue::Bool(b) => Value::from(b),
        EditValue::Float(f) => Value::F32(f),
        EditValue::Int(i) => Value::from(i),
        EditValue::None => Value::Nil,
    };
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_SET_VALUE)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SLOT), Value::from(slot)),
                (Value::from(K_FLAG_29), Value::from(true)),
                (Value::from(K_MODEL_SEL), Value::from(model_sel)),
                (Value::from(K_PARAM_INDEX), Value::from(param_index)),
                (Value::from(K_VALUE), wire_value),
            ]),
        ),
    ]))
}

/// Build a **select-preset** body (op 20): load `preset` from `bank` into the edit buffer.
/// **This changes the active preset on the device** — it is navigation, not a read.
/// (`{102:txn, 100:20, 101:{107:bank, 108:preset}}`.)
pub fn select_preset(bank: i64, preset: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_SELECT)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SELECT_BANK), Value::from(bank)),
                (Value::from(K_SELECT_PRESET), Value::from(preset)),
            ]),
        ),
    ]))
}

/// Build a **save-preset** body (op 71): write the current edit buffer to `bank`/`slot` under
/// `name`. **This is a persistent write — it overwrites the target slot in device flash.** The name
/// is stored NUL-terminated, as HX Edit sends it. (`{102:txn, 100:71, 101:{107:bank,108:slot,109:name\0}}`.)
/// Reproduces device bytes exactly.
pub fn save_preset(bank: i64, slot: i64, name: &str, txn: u16) -> Vec<u8> {
    let mut name_z = String::with_capacity(name.len() + 1);
    name_z.push_str(name);
    name_z.push('\0');
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_SAVE_PRESET)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SELECT_BANK), Value::from(bank)),
                (Value::from(K_SELECT_PRESET), Value::from(slot)),
                (Value::from(K_NAME), Value::from(name_z)),
            ]),
        ),
    ]))
}

/// Build a **rename-preset** body (op 6): change only the stored name of `bank`/`slot`, without
/// committing the edit buffer (name-only rename). The name is stored NUL-terminated, as HX Edit
/// sends it. **Rides the primary channel, not the edit channel.** (`{102:txn, 100:6,
/// 101:{107:bank,108:slot,109:name\0}}`.) Reproduces device bytes exactly.
pub fn rename_preset(bank: i64, slot: i64, name: &str, txn: u16) -> Vec<u8> {
    let mut name_z = String::with_capacity(name.len() + 1);
    name_z.push_str(name);
    name_z.push('\0');
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_RENAME_PRESET)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SELECT_BANK), Value::from(bank)),
                (Value::from(K_SELECT_PRESET), Value::from(slot)),
                (Value::from(K_NAME), Value::from(name_z)),
            ]),
        ),
    ]))
}

/// Build a **swap-model** body (op 40): replace the model of the block in `slot` with `model_index`
/// (its `Helix.sym` index), and `paired_index` for the paired cab/IR (`-1` = none). The device
/// resets the block's params to the new model's defaults. (`{102:txn, 100:40, 101:{98:slot,
/// 100:{23:<paired?>, 25:model_index, 26:paired_index}}}`.) Reproduces device bytes exactly.
///
/// Key `23` is the **paired-model-active flag** [solid — live 2026-07-09]: with it `false` the
/// device stores the `26` index but never instantiates the cab (empty param vector, no cab in the
/// signal path); preset blobs of real amp+cab blocks carry `23: true`. So it mirrors
/// `paired_index >= 0`.
pub fn swap_model(slot: i64, model_index: i64, paired_index: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_SWAP_MODEL)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SLOT), Value::from(slot)),
                (
                    Value::from(K_MODEL_REF),
                    Value::Map(vec![
                        (Value::from(K_MODEL_FLAG), Value::from(paired_index >= 0)),
                        (Value::from(K_MODEL_INDEX), Value::from(model_index)),
                        (Value::from(K_PAIRED_INDEX), Value::from(paired_index)),
                    ]),
                ),
            ]),
        ),
    ]))
}

/// Build a **move-block** body (op 43): move the block in `src` slot to `dst` slot. The destination
/// slot encodes the row (a parallel-path slot index moves the block to row B). HX Edit re-reads the
/// preset after a move. (`{102:txn, 100:43, 101:{75:src, 76:dst}}`.) Reproduces device bytes exactly.
pub fn move_block(src: i64, dst: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_MOVE_BLOCK)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_MOVE_SRC), Value::from(src)),
                (Value::from(K_MOVE_DST), Value::from(dst)),
            ]),
        ),
    ]))
}

/// Build a **whole-preset write** body (op 21): `{102:txn, 100:21, 101:{110:<blob>}}` with the blob
/// wrapped as a MessagePack `str` (the device's `str16` framing). The blob (from
/// [`fretwire_data::stream::PresetStream::to_blob`]) contains non-UTF-8 bytes, so the envelope is
/// hand-emitted rather than built through `rmpv` (which can't hold a non-UTF-8 string). Reproduces
/// the device's envelope framing (byte-exact through the blob length prefix; see the test).
pub fn write_preset(blob: &[u8], txn: u16) -> Vec<u8> {
    // {102: txn(cd u16), 100: 21, 101: {110: str<blob>}}
    let mut out = vec![
        0x83, // fixmap 3
        0x66,
        0xcd,
        (txn >> 8) as u8,
        txn as u8, // 102 => cd u16 txn
        0x64,
        OP_WRITE_PRESET as u8, // 100 => 21
        0x65,
        0x81,               // 101 => fixmap 1
        K_WRITE_BLOB as u8, // 110
    ];
    push_str_header(&mut out, blob.len());
    out.extend_from_slice(blob);
    out
}

/// Emit a MessagePack `str` length header (forces `str16` for the 32..65536 range, matching the
/// device's non-minimal framing for the preset blob/header). Payload may be non-UTF-8.
fn push_str_header(out: &mut Vec<u8>, len: usize) {
    if len < 32 {
        out.push(0xa0 | len as u8);
    } else if len < 65536 {
        out.push(0xda);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0xdb);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

/// Build a **begin-structural** body (op 78): announce that a structural edit (move/add) is about to
/// act on `slot`. (`{102:txn, 100:78, 101:{98:slot, 26:0}}`.) Reproduces device bytes exactly.
pub fn begin_structural(slot: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_BEGIN_STRUCT)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SLOT), Value::from(slot)),
                (Value::from(K_MODEL_SEL), Value::from(MODEL_MAIN)),
            ]),
        ),
    ]))
}

/// Build a **delete-block** body (op 28): remove the block at `slot`. Surgical — the device clears the
/// slot and drops that block's own footswitch binding, preserving the rest of the layout.
/// (`{102:txn, 100:28, 101:{98:slot}}`.) Reproduces device bytes exactly.
pub fn delete_block(slot: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_DELETE_BLOCK)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![(Value::from(K_SLOT), Value::from(slot))]),
        ),
    ]))
}

/// Build an **add-block** body (op 39): create a block at `slot` with `model_index` (its `Helix.sym`
/// index) and `paired_index` (`-1` = no paired cab/IR), enabled. After adding, HX Edit issues
/// `set_value`s for the new block's params. (`{102:txn, 100:39, 101:{98:slot, 99:{19:6, 20:{24:
/// {23:<paired?>, 25:model, 26:paired}, 9:1, 10:true}}}}`.) Reproduces device bytes exactly.
/// Key `23` mirrors `paired_index >= 0` — the paired-model-active flag (see [`swap_model`]).
pub fn add_block(slot: i64, model_index: i64, paired_index: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_ADD_BLOCK)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SLOT), Value::from(slot)),
                (
                    Value::from(K_ADD_SPEC),
                    Value::Map(vec![
                        (Value::from(K_NODE_KIND), Value::from(NODE_KIND_BLOCK)),
                        (
                            Value::from(K_BLOCK_CONTENT),
                            Value::Map(vec![
                                (
                                    Value::from(K_BLOCK_MODEL),
                                    Value::Map(vec![
                                        // 23 = paired-model-active flag (see `swap_model`).
                                        (Value::from(K_MODEL_FLAG), Value::from(paired_index >= 0)),
                                        (Value::from(K_MODEL_INDEX), Value::from(model_index)),
                                        (Value::from(K_PAIRED_INDEX), Value::from(paired_index)),
                                    ]),
                                ),
                                (Value::from(K_BLOCK_FLAG9), Value::from(1)),
                                (Value::from(K_BLOCK_ENABLED), Value::from(true)),
                            ]),
                        ),
                    ]),
                ),
            ]),
        ),
    ]))
}

/// Build a **rename-snapshot** body (op 89): set snapshot `index`'s name (stored NUL-terminated, as
/// HX Edit sends it). (`{102:txn, 100:89, 101:{92:index, 109:name\0}}`.) Reproduces device bytes exactly.
pub fn rename_snapshot(index: i64, name: &str, txn: u16) -> Vec<u8> {
    let mut name_z = String::with_capacity(name.len() + 1);
    name_z.push_str(name);
    name_z.push('\0');
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_RENAME_SNAPSHOT)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SNAPSHOT), Value::from(index)),
                (Value::from(K_NAME), Value::from(name_z)),
            ]),
        ),
    ]))
}

/// Build a **set-setting** body (op 25): set global/input setting `id` to integer `value`. Not
/// block-addressed. The `id` space is only partly mapped (see [`K_SETTING_ID`]); this builder is the
/// wire primitive for both known settings and live probing. (`{102:txn, 100:25, 101:{118:id, 119:value}}`.)
pub fn set_setting(id: i64, value: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_SETTING)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SETTING_ID), Value::from(id)),
                (Value::from(K_VALUE), Value::from(value)),
            ]),
        ),
    ]))
}

/// Build a **switch-snapshot** body (op 88): make snapshot `index` active in the edit buffer.
/// **Changes device state.** (`{102:txn, 100:88, 101:{92:index}}`.) Reproduces device bytes exactly.
pub fn switch_snapshot(index: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_SWITCH_SNAPSHOT)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![(Value::from(K_SNAPSHOT), Value::from(index))]),
        ),
    ]))
}

// ---- non-destructive read of the current edit buffer (the connect-time sequence HX Edit uses) ----

/// Build the **read-open** body (op 76): open the current edit buffer for reading. Target is an
/// empty map. Does not change the active preset. (`{102:txn, 100:76, 101:{}}`.)
pub fn read_open(txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_READ_OPEN)),
        (Value::from(K_TARGET), Value::Map(Vec::new())),
    ]))
}

/// Build the **read-prepare** body (op 24): `{118: 128}`, replicated from the connect capture.
pub fn read_prep(txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_READ_PREP)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![(Value::from(K_READ_PREP), Value::from(128))]),
        ),
    ]))
}

/// Build the **read-info** body (op 23): nil target; the reply carries the preset identity/name.
pub fn read_info(txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_READ_INFO)),
        (Value::from(K_TARGET), Value::Nil),
    ]))
}

/// Build a **stream-start** body (op 22): begin the paged stream of the opened edit buffer, with
/// transaction counter `txn`. The target is nil (`{102:txn, 100:22, 101:nil}`).
pub fn stream_start(txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_STREAM_START)),
        (Value::from(K_TARGET), Value::Nil),
    ]))
}

// ---- preset-list browse (primary channel, TLV opcode 0x0002) ----
// From startup.pcapng: HX Edit lists all presets via op 254 (browse open) -> op 0 (open the
// "PRESETS" resource) -> op 1 (start the stream) -> cmd 0x08 pagination. Reply is an array of
// `{index: {109: name, …}}`.

/// Operation id 254: open the browse session.
pub const OP_BROWSE_OPEN: i64 = 254;
/// Operation id 0: open the PRESETS resource for listing.
pub const OP_PRESETS_OPEN: i64 = 0;
/// Operation id 1: start the preset-list stream.
pub const OP_PRESETS_STREAM: i64 = 1;
/// Target key inside the preset-list stream-start (seen value 2).
const K_LIST_KIND: i64 = 101;

/// Build the **browse-open** body (op 254): `{102:txn, 100:254, 101:{}}`.
pub fn browse_open(txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_BROWSE_OPEN)),
        (Value::from(K_TARGET), Value::Map(Vec::new())),
    ]))
}

/// Build the **presets-open** body (op 0): `{102:txn, 100:0, 101:nil}`.
pub fn presets_open(txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_PRESETS_OPEN)),
        (Value::from(K_TARGET), Value::Nil),
    ]))
}

/// Build the **presets-stream-start** body (op 1): `{102:txn, 100:1, 101:{107:bank, 101:2}}`.
///
/// `bank` is the **setlist** to list — the same index `select_preset`/`save_preset` take, and the
/// one a preset reports as [`fretwire_data::stream::PresetInfo::bank`]. This was hardcoded to 0,
/// which is why a Helix Floor sitting in User 1 (bank 2) still listed the Factory 1 names.
pub fn presets_stream(txn: u16, bank: i64) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_PRESETS_STREAM)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SELECT_BANK), Value::from(bank)),
                (Value::from(K_LIST_KIND), Value::from(2)),
            ]),
        ),
    ]))
}

fn get(v: &Value, key: i64) -> Option<&Value> {
    match v {
        Value::Map(m) => m
            .iter()
            .find(|(k, _)| k.as_i64() == Some(key))
            .map(|(_, val)| val),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // From captures (tools/dump-control.ps1): tremolo bypass on (slot 4); Dynamic Ambience Mix=0.62
    // (slot 7, Mix is param index 5 in VIC_ReverbDynAmbience).
    const BYPASS_ON: &[u8] = &[
        0x83, 0x66, 0xcd, 0x03, 0xf2, 0x64, 0x29, 0x65, 0x82, 0x62, 0x04, 0x3b, 0xc3,
    ];
    const AMBIENCE_MIX: &[u8] = &[
        0x83, 0x66, 0xcd, 0x04, 0xa1, 0x64, 0x1e, 0x65, 0x85, 0x62, 0x07, 0x1d, 0xc3, 0x1a, 0x00,
        0x1c, 0x05, 0x77, 0xca, 0x3f, 0x1e, 0xb8, 0x52,
    ];

    #[test]
    fn parses_bypass() {
        let e = EditBody::parse(BYPASS_ON).unwrap();
        assert_eq!(e.op, OP_BYPASS);
        assert_eq!(e.txn, 0x03f2);
        assert_eq!(e.slot, Some(4));
        assert_eq!(e.value, EditValue::Bool(true));
        assert_eq!(e.param_index, None);
    }

    #[test]
    fn parses_set_value_with_param_index() {
        let e = EditBody::parse(AMBIENCE_MIX).unwrap();
        assert_eq!(e.op, OP_SET_VALUE);
        assert_eq!(e.txn, 0x04a1);
        assert_eq!(e.slot, Some(7));
        assert_eq!(e.param_index, Some(5)); // Mix is index 5 in this reverb
        assert_eq!(e.value, EditValue::Float(0.62));
    }

    #[test]
    fn builders_reproduce_captured_bytes() {
        assert_eq!(bypass(4, true, 0x03f2), BYPASS_ON);
        assert_eq!(set_value(7, 5, 0.62, 0x04a1), AMBIENCE_MIX);
    }

    #[test]
    fn builders_round_trip_through_parser() {
        let e = EditBody::parse(&set_value(2, 3, 0.43, 0x04cf)).unwrap();
        assert_eq!(e.slot, Some(2));
        assert_eq!(e.param_index, Some(3));
        assert_eq!(e.value, EditValue::Float(0.43));
        assert_eq!(e.op, OP_SET_VALUE);
    }

    #[test]
    fn rejects_non_msgpack_map() {
        assert!(EditBody::parse(&[0x01, 0x02, 0x03]).is_err());
    }

    // swap-model: msgpack slices from model_swap_delay_then_reverb.pcapng — HX Edit swapping slot 6
    // to Helix.sym index 79 (Mod/Chorus Echo) then 607 (a reverb), paired = -1 (no cab).
    #[test]
    fn swap_model_reproduces_captured_bytes() {
        // slot 6 -> 79: {102:0x03f1, 100:40, 101:{98:6, 100:{23:false, 25:79, 26:-1}}}
        let to_79 = [
            0x83, 0x66, 0xcd, 0x03, 0xf1, 0x64, 0x28, 0x65, 0x82, 0x62, 0x06, 0x64, 0x83, 0x17,
            0xc2, 0x19, 0x4f, 0x1a, 0xff,
        ];
        assert_eq!(swap_model(6, 79, -1, 0x03f1), to_79);
        // slot 6 -> 607 (uint16 0x025f): {102:0x03f2, 100:40, 101:{98:6, 100:{23:false, 25:607, 26:-1}}}
        let to_607 = [
            0x83, 0x66, 0xcd, 0x03, 0xf2, 0x64, 0x28, 0x65, 0x82, 0x62, 0x06, 0x64, 0x83, 0x17,
            0xc2, 0x19, 0xcd, 0x02, 0x5f, 0x1a, 0xff,
        ];
        assert_eq!(swap_model(6, 607, -1, 0x03f2), to_607);
    }

    // rename-preset (op 6, primary channel): from the rename capture — slot 24 (0x18), bank 0,
    // renamed to an 11-char name (stored NUL-terminated), txn 0x03ee. The capture's scratch-preset
    // name is replaced here by a same-length placeholder, so only the 11 name bytes differ from the
    // wire; the framing, keys, and the 0xac length prefix are verbatim.
    #[test]
    fn rename_preset_reproduces_captured_bytes() {
        let bytes = [
            0x83, 0x66, 0xcd, 0x03, 0xee, 0x64, 0x06, 0x65, 0x83, 0x6b, 0x00, 0x6c, 0x18, 0x6d,
            0xac, 0x53, 0x63, 0x72, 0x61, 0x74, 0x63, 0x68, 0x50, 0x61, 0x64, 0x32, 0x00,
        ];
        assert_eq!(rename_preset(0, 24, "ScratchPad2", 0x03ee), bytes);
    }

    // delete-block (op 28): from the delete captures — slot 5 (txn 0x0401) and slot 6 (txn 0x0410).
    #[test]
    fn delete_block_reproduces_captured_bytes() {
        let slot5 = [
            0x83, 0x66, 0xcd, 0x04, 0x01, 0x64, 0x1c, 0x65, 0x81, 0x62, 0x05,
        ];
        assert_eq!(delete_block(5, 0x0401), slot5);
        let slot6 = [
            0x83, 0x66, 0xcd, 0x04, 0x10, 0x64, 0x1c, 0x65, 0x81, 0x62, 0x06,
        ];
        assert_eq!(delete_block(6, 0x0410), slot6);
    }

    // paired-cab set-value: msgpack slices from the cab-edit captures (slot 6, an amp+cab block).
    // change_to_sm57_mic: cab mic (param idx 0) = enum int 1 -> {…26:1, 28:0, 119:1(int)}.
    // move_mic_distance: cab distance (idx 2) = float 1.75 -> {…26:1, 28:2, 119:1.75(f32)}.
    // toggle_cab_mic_to_45deg: cab angle (idx 3) = float 45.0 -> {…26:1, 28:3, 119:45.0(f32)}.
    #[test]
    fn paired_set_value_reproduces_captured_bytes() {
        let mic = [
            0x83, 0x66, 0xcd, 0x04, 0xf9, 0x64, 0x1e, 0x65, 0x85, 0x62, 0x06, 0x1d, 0xc3, 0x1a,
            0x01, 0x1c, 0x00, 0x77, 0x01,
        ];
        assert_eq!(
            set_value_on(6, MODEL_PAIRED, 0, EditValue::Int(1), 0x04f9),
            mic
        );
        let distance = [
            0x83, 0x66, 0xcd, 0x04, 0xbe, 0x64, 0x1e, 0x65, 0x85, 0x62, 0x06, 0x1d, 0xc3, 0x1a,
            0x01, 0x1c, 0x02, 0x77, 0xca, 0x3f, 0xe0, 0x00, 0x00,
        ];
        assert_eq!(set_paired_value(6, 2, 1.75, 0x04be), distance);
        let angle = [
            0x83, 0x66, 0xcd, 0x04, 0xf3, 0x64, 0x1e, 0x65, 0x85, 0x62, 0x06, 0x1d, 0xc3, 0x1a,
            0x01, 0x1c, 0x03, 0x77, 0xca, 0x42, 0x34, 0x00, 0x00,
        ];
        assert_eq!(set_paired_value(6, 3, 45.0, 0x04f3), angle);
    }

    #[test]
    fn write_preset_envelope_matches_capture_prefix() {
        // move_EQ_right_two_slots.pcapng first op-21 frame: TLV value begins
        // 83 66 cd 04 c6 64 15 65 81 6e da 0b 9a <blob…>  (txn 0x04c6, op 21, key 110, str16 2970)
        let blob = vec![0u8; 2970];
        let body = write_preset(&blob, 0x04c6);
        assert_eq!(
            &body[..13],
            &[
                0x83, 0x66, 0xcd, 0x04, 0xc6, 0x64, 0x15, 0x65, 0x81, 0x6e, 0xda, 0x0b, 0x9a
            ]
        );
        assert_eq!(body.len(), 13 + 2970);
    }

    #[test]
    fn begin_structural_reproduces_captured_bytes() {
        // one_by_one_move_all_blocks_one_right: 78 on slot 7, {102:0x040b, 100:78, 101:{98:7, 26:0}}
        // bytes: 83 66 cd 04 0b 64 4e 65 82 62 07 1a 00
        let bytes = [
            0x83, 0x66, 0xcd, 0x04, 0x0b, 0x64, 0x4e, 0x65, 0x82, 0x62, 0x07, 0x1a, 0x00,
        ];
        assert_eq!(begin_structural(7, 0x040b), bytes);
    }

    #[test]
    fn move_block_reproduces_captured_bytes() {
        // one_by_one_move_all_blocks_one_right: move slot 7 -> 8, {102:0x040c, 100:43, 101:{75:7,76:8}}
        let bytes = [
            0x83, 0x66, 0xcd, 0x04, 0x0c, 0x64, 0x2b, 0x65, 0x82, 0x4b, 0x07, 0x4c, 0x08,
        ];
        assert_eq!(move_block(7, 8, 0x040c), bytes);
    }

    #[test]
    fn add_block_reproduces_captured_bytes() {
        // add_simple_eq_at_beginning_of_chain: add EQ (model 132) at slot 2, no cab, txn 0x041a:
        // {102:0x041a, 100:39, 101:{98:2, 99:{19:6, 20:{24:{23:false, 25:132, 26:-1}, 9:1, 10:true}}}}
        let bytes = [
            0x83, 0x66, 0xcd, 0x04, 0x1a, 0x64, 0x27, 0x65, 0x82, 0x62, 0x02, 0x63, 0x82, 0x13,
            0x06, 0x14, 0x83, 0x18, 0x83, 0x17, 0xc2, 0x19, 0xcc, 0x84, 0x1a, 0xff, 0x09, 0x01,
            0x0a, 0xc3,
        ];
        assert_eq!(add_block(2, 132, -1, 0x041a), bytes);
    }

    // rename-snapshot: msgpack slice from launch_hx_..._rename_snapshot_..._closehx.pcapng —
    // HX Edit naming snapshot 0 "test_snap", {102:0x03f1, 100:89, 101:{92:0, 109:"test_snap\0"}}.
    #[test]
    fn rename_snapshot_reproduces_captured_bytes() {
        let expect = [
            0x83, 0x66, 0xcd, 0x03, 0xf1, 0x64, 0x59, 0x65, 0x82, 0x5c, 0x00, 0x6d, 0xaa, 0x74,
            0x65, 0x73, 0x74, 0x5f, 0x73, 0x6e, 0x61, 0x70, 0x00,
        ];
        assert_eq!(rename_snapshot(0, "test_snap", 0x03f1), expect);
    }

    // set-setting: msgpack slice from switch_input_gate_and_guitar_pad.pcapng — setting id 134 = 0,
    // {102:0x03fa, 100:25, 101:{118:134, 119:0}}.
    #[test]
    fn set_setting_reproduces_captured_bytes() {
        let expect = [
            0x83, 0x66, 0xcd, 0x03, 0xfa, 0x64, 0x19, 0x65, 0x82, 0x76, 0xcc, 0x86, 0x77, 0x00,
        ];
        assert_eq!(set_setting(134, 0, 0x03fa), expect);
    }

    // save-preset: msgpack slice from launch_hx_..._savepreset_..._closehx.pcapng — HX Edit saving
    // the edit buffer to bank 0 / slot 22 as "Serial", {102:0x03f2, 100:71, 101:{107:0,108:22,109:"Serial\0"}}.
    #[test]
    fn save_preset_reproduces_captured_bytes() {
        let expect = [
            0x83, 0x66, 0xcd, 0x03, 0xf2, 0x64, 0x47, 0x65, 0x83, 0x6b, 0x00, 0x6c, 0x16, 0x6d,
            0xa7, 0x53, 0x65, 0x72, 0x69, 0x61, 0x6c, 0x00,
        ];
        assert_eq!(save_preset(0, 22, "Serial", 0x03f2), expect);
    }

    // switch-snapshot: msgpack slice from launch_hx_..._switchsnapshot_closehx.pcapng —
    // HX Edit switching to snapshot 1, {102: 0x03f3, 100: 88, 101: {92: 1}}.
    #[test]
    fn switch_snapshot_reproduces_captured_bytes() {
        let expect = [
            0x83, 0x66, 0xcd, 0x03, 0xf3, 0x64, 0x58, 0x65, 0x81, 0x5c, 0x01,
        ];
        assert_eq!(switch_snapshot(1, 0x03f3), expect);
    }

    // select-preset: the msgpack slice of OPEN_2321 (real_frames.rs) — HX Edit *selecting* a
    // preset, {102: 0x043a, 100: 20, 101: {107: 0, 108: 19}}.
    #[test]
    fn select_preset_reproduces_captured_bytes() {
        let expect = [
            0x83, 0x66, 0xcd, 0x04, 0x3a, 0x64, 0x14, 0x65, 0x82, 0x6b, 0x00, 0x6c, 0x13,
        ];
        assert_eq!(select_preset(0, 19, 0x043a), expect);
    }

    // The non-destructive read sequence, decoded from startup.pcapng (the connect-time read).
    // txns 0x3e8..0x3eb as HX Edit sent them.
    #[test]
    fn read_sequence_reproduces_connect_capture() {
        // op 76, target {}  ->  83 66 cd03e8 64 4c 65 80
        assert_eq!(
            read_open(0x03e8),
            [0x83, 0x66, 0xcd, 0x03, 0xe8, 0x64, 0x4c, 0x65, 0x80]
        );
        // op 24, target {118:128}  ->  83 66 cd03e9 64 18 65 81 76 cc80
        assert_eq!(
            read_prep(0x03e9),
            [
                0x83, 0x66, 0xcd, 0x03, 0xe9, 0x64, 0x18, 0x65, 0x81, 0x76, 0xcc, 0x80
            ]
        );
        // op 23, target nil  ->  83 66 cd03ea 64 17 65 c0
        assert_eq!(
            read_info(0x03ea),
            [0x83, 0x66, 0xcd, 0x03, 0xea, 0x64, 0x17, 0x65, 0xc0]
        );
        // op 22, target nil  ->  83 66 cd03eb 64 16 65 c0
        assert_eq!(
            stream_start(0x03eb),
            [0x83, 0x66, 0xcd, 0x03, 0xeb, 0x64, 0x16, 0x65, 0xc0]
        );
    }

    // The preset-list browse sequence, decoded from startup.pcapng (primary channel, txns 0x3e8..0x3ea).
    #[test]
    fn browse_sequence_reproduces_capture() {
        // op 254, target {}  ->  83 66 cd03e8 64 ccfe 65 80
        assert_eq!(
            browse_open(0x03e8),
            [0x83, 0x66, 0xcd, 0x03, 0xe8, 0x64, 0xcc, 0xfe, 0x65, 0x80]
        );
        // op 0, target nil  ->  83 66 cd03e9 64 00 65 c0
        assert_eq!(
            presets_open(0x03e9),
            [0x83, 0x66, 0xcd, 0x03, 0xe9, 0x64, 0x00, 0x65, 0xc0]
        );
        // op 1, target {107:0, 101:2}  ->  83 66 cd03ea 64 01 65 82 6b 00 65 02
        assert_eq!(
            presets_stream(0x03ea, 0),
            [
                0x83, 0x66, 0xcd, 0x03, 0xea, 0x64, 0x01, 0x65, 0x82, 0x6b, 0x00, 0x65, 0x02
            ]
        );
        // Same frame for a different setlist: only the 107 value moves (bank 2 = "User 1" on the
        // Helix Floor). The bank was hardcoded to 0 here, so every list came back as Factory 1.
        assert_eq!(
            presets_stream(0x03ea, 2),
            [
                0x83, 0x66, 0xcd, 0x03, 0xea, 0x64, 0x01, 0x65, 0x82, 0x6b, 0x02, 0x65, 0x02
            ]
        );
    }

    #[test]
    fn bypass_tlv_wraps_with_correct_header() {
        let bytes = EditBody::parse(BYPASS_ON).unwrap().to_tlv().to_bytes();
        assert_eq!(&bytes[0..4], &[0x01, 0x00, 0x06, 0x00]); // marker + opcode 0x0006
        assert_eq!(&bytes[4..8], &[0x0d, 0x00, 0x00, 0x00]); // ilen = 13
        assert_eq!(&bytes[8..], BYPASS_ON);
    }
}
