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
/// Operation id 24: **read a device setting by id** — `{118: id}`.
///
/// We met this as a "read-sequence prepare step" because the connect capture sends `{118: 128}`
/// and we only ever replayed it. It is not a prepare step: settings are a flat numbered namespace,
/// op 24 reads one and [`OP_SETTING`] (25) writes one, and the handshake is simply fetching id 128
/// along the way. [`read_prep`] is kept as the handshake's fixed call; [`read_setting`] is the
/// general one.
pub const OP_READ_PREP: i64 = 24;
/// Alias for [`OP_READ_PREP`] under the name that says what it does.
pub const OP_READ_SETTING: i64 = OP_READ_PREP;
/// Operation id 23: read-sequence query step (nil target; reply carries the preset identity).
pub const OP_READ_INFO: i64 = 23;
/// Operation id 4: **read the preset stored at an index, without loading it** —
/// `{107: bank, 108: slot, 101: 2}`. Streams that slot's whole document back exactly as op 22
/// streams the edit buffer, but the device's active preset and edit buffer are untouched.
///
/// This is what makes a setlist backup cheap: the alternative is `select` + read per slot, which
/// walks the user's pedal through all 128 presets and takes tens of minutes.
///
/// **Reads only.** Its inverse (op 5 / op 8, write a document into a slot) is deliberately not
/// built here — that is a persistent write and wants its own captures first.
///
/// [`tonepush`'s opcode table names this op; the argument shape and the streaming behaviour are
/// verified here against an HX Stomp.]
pub const OP_READ_PRESET_AT: i64 = 4;

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
    set_value_flagged(slot, model_sel, true, param_index, value, txn)
}

/// [`set_value_on`] with the addressing flag (target key 29) exposed.
///
/// **Key 29 chooses what key 28 indexes.** `true` — every ordinary edit — means "the parameter's
/// position in the model's `Helix.sym` order". `false` selects the block's *extra* values, the ones
/// the symbol doesn't list, and there `28: 0` is `Trails`. HX Edit toggles a reverb's trails with
/// `{98: slot, 29: false, 26: 0, 28: 0, 119: <bool>}` and nothing else — six of them in
/// `captures/dynamic_ambience_trails_on_off.pcapng`, against `29: true, 28: 5` for the same block's
/// Mix knob in `dynamic_ambience_mix_modify.pcapng`. Sending `29: true` for one of these is refused
/// with `-3`, which is what made `Trails` look unreachable.
/// [solid — 2026-08-02, capture + confirmed live on an HX Stomp]
pub fn set_value_flagged(
    slot: i64,
    model_sel: i64,
    by_param_index: bool,
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
                (Value::from(K_FLAG_29), Value::from(by_param_index)),
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
    set_setting_value(id, Value::from(value), txn)
}

/// [`set_setting`] with the value's **type** chosen by the caller.
///
/// The device stores each setting as a definite type and refuses a write of any other with `-3` —
/// tempo, for instance, is an `f32`, so the integer-only builder above could never write it. Read
/// the current value with [`read_setting`] and send back the same type.
pub fn set_setting_value(id: i64, value: Value, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_SETTING)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SETTING_ID), Value::from(id)),
                (Value::from(K_VALUE), value),
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

/// Build a **read-setting** body (op 24): `{102:txn, 100:24, 101:{118:id}}`.
///
/// Settings are a flat numbered namespace rather than a structured document — op 24 reads one and
/// [`set_setting`] writes it. The reply carries the value at key `119`, the same key a write puts
/// it in.
///
/// A write's value **type has to match what the device already holds** (a float where it wants a
/// bool is refused with `-3`), which is the other reason to read before writing.
pub fn read_setting(id: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_READ_SETTING)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![(Value::from(K_SETTING_ID), Value::from(id))]),
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

/// Build the **read-preset-at-index** body (op 4): `{102:txn, 100:4, 101:{107:bank, 108:slot,
/// 101:2}}`.
///
/// Same prologue and same chunked stream as the preset listing — only the target differs, by
/// carrying a slot alongside the bank. `2` is the same "kind" the listing sends; its meaning is
/// unknown beyond "HX Edit always sends it here".
///
/// Non-destructive: the device streams the stored document without selecting it, so the active
/// preset and any pending edits survive. [solid — read every slot in a setlist while the panel
/// stayed on its preset and the edit buffer kept an uncommitted change.]
pub fn read_preset_at(txn: u16, bank: i64, slot: i64) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_READ_PRESET_AT)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SELECT_BANK), Value::from(bank)),
                (Value::from(K_SELECT_PRESET), Value::from(slot)),
                (Value::from(K_LIST_KIND), Value::from(2)),
            ]),
        ),
    ]))
}

// ---- controller assignments: what drives a parameter, and what a footswitch carries ----
//
// Two separate mechanisms, and conflating them is the trap. A **parameter** under a controller
// lives in the preset document's top-level key `4` (see `PresetStream::assignments`) and is written
// with op 37. A **block bypass on a footswitch** is not in key `4` at all — it lives in the
// footswitch layout at `3 -> 8`, which we already decode, and is written with ops 56/57.
//
// Provenance: the opcode numbers and argument shapes below come from `tonepush`'s `PROTOCOL.md`,
// which recovered them from a macOS HX Edit capture we do not have. Everything here is
// `[hypothesis]` until this file says otherwise — each builder's doc records what the pedal
// actually did.

/// Op 33: **read a footswitch's configuration**. Answers with what the switch carries, its label,
/// its LED colour and its latching/momentary type.
///
/// **One-based on the way in** (`102: 1` is Footswitch 1), unlike ops 56-62, which count from zero;
/// the reply reports the zero-based number: asking for 1, 2, 3 answers `102: 0, 1, 2`.
/// [solid — verified live on an HX Stomp 2026-08-22]
pub const OP_READ_SWITCH: i64 = 33;

/// Op 36: **read a parameter's controller assignment**. Answers with the source driving that
/// parameter, and `104: nil` when nothing does.
///
/// The reply's key 104 is the *same map* the document stores in its controller table, so this is a
/// second route to what `PresetStream::assignments` already decodes — useful as a cross-check, and
/// as the read half of the op-37 write.
/// [solid — verified live on an HX Stomp 2026-08-22]
pub const OP_READ_ASSIGN: i64 = 36;

/// Key 102 **inside an assignment target**: the footswitch number.
///
/// Note this is [`K_TXN`]'s number one level up. No collision — the transaction is at the body's
/// root and the switch is inside key 101 — but the two are easy to mix up when reading a raw dump.
pub const K_SWITCH: i64 = 102;

/// Key 29 in an assignment **request**: `true` when the thing addressed is a parameter.
///
/// Beware the mirror: in the *document* key 29 is the parameter index and key 28 is the path
/// (verified live 2026-08-21, `docs/preset-format.md`). In a request the two swap roles — 28 is the
/// parameter index and 29 is this flag.
pub const K_ASSIGN_IS_PARAM: i64 = 29;

/// Build a **read-footswitch** body (op 33): ask what footswitch `switch` carries.
/// `switch` is **one-based** — 1 is Footswitch 1. (`{102:txn, 100:33, 101:{102:switch}}`.)
/// Pure read. [solid — verified live on an HX Stomp 2026-08-22]
pub fn read_switch(switch: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_READ_SWITCH)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![(Value::from(K_SWITCH), Value::from(switch))]),
        ),
    ]))
}

/// Build a **read-assignment** body (op 36): ask what drives parameter `param_index` of the block in
/// `slot`. `paired` selects the paired cab's namespace, exactly as [`set_value_on`] does.
/// (`{102:txn, 100:36, 101:{98:slot, 26:paired, 28:param, 29:true}}`.)
/// Pure read. [solid — verified live on an HX Stomp 2026-08-22]
pub fn read_assignment(slot: i64, paired: bool, param_index: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_READ_ASSIGN)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SLOT), Value::from(slot)),
                (Value::from(K_MODEL_SEL), Value::from(i64::from(paired))),
                (Value::from(K_PARAM_INDEX), Value::from(param_index)),
                (Value::from(K_ASSIGN_IS_PARAM), Value::from(true)),
            ]),
        ),
    ]))
}

/// Op 56: **put a block's bypass on a footswitch**. Zero-based switch number.
/// [solid — verified live on an HX Stomp 2026-08-22]
pub const OP_BYPASS_TO_SWITCH: i64 = 56;

/// Op 57: **take a block's bypass off a footswitch**. Same arguments as [`OP_BYPASS_TO_SWITCH`].
/// [solid — verified live on an HX Stomp 2026-08-22]
pub const OP_BYPASS_OFF_SWITCH: i64 = 57;

/// Build an arbitrary edit body: `{102: txn, 100: op, 101: <target>}`.
///
/// Every builder in this module is a special case of this shape, and this is the escape hatch for
/// **probing an op we have not decoded** — the ops the footswitch record's unknown keys are set by,
/// for instance. Nothing validates `op` or `target`, which is the point: a refusal code is the
/// answer we are looking for.
///
/// Prefer a named builder wherever one exists. This exists so that finding the next one does not
/// require writing it first.
pub fn probe(op: i64, target: Vec<(Value, Value)>, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(op)),
        (Value::from(K_TARGET), Value::Map(target)),
    ]))
}

/// Build an **assign-bypass-to-footswitch** body (op 56): make footswitch `switch` toggle the
/// bypass of the block in `slot`. **`switch` is zero-based** — `0` is Footswitch 1, unlike
/// [`read_switch`], which is one-based.
///
/// Edit-buffer only: reloading the preset discards it, and it persists only through a save.
/// (`{102:txn, 100:56, 101:{98:slot, 102:switch}}`.)
/// [solid — verified live on an HX Stomp 2026-08-22: sending it on a preset with nothing bound
/// added exactly one entry to the footswitch layout at `3 -> 8[0]`, naming that block, and op 57
/// put the document back byte-for-byte]
pub fn assign_bypass_to_switch(slot: i64, switch: i64, txn: u16) -> Vec<u8> {
    switch_assign(OP_BYPASS_TO_SWITCH, slot, switch, txn)
}

/// Build an **unassign-bypass-from-footswitch** body (op 57) — the reverse of
/// [`assign_bypass_to_switch`], with the same arguments. [solid — verified live on an HX Stomp 2026-08-22]
pub fn unassign_bypass_from_switch(slot: i64, switch: i64, txn: u16) -> Vec<u8> {
    switch_assign(OP_BYPASS_OFF_SWITCH, slot, switch, txn)
}

/// The body ops 56 and 57 share: a block and a zero-based footswitch.
fn switch_assign(op: i64, slot: i64, switch: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(op)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SLOT), Value::from(slot)),
                (Value::from(K_SWITCH), Value::from(switch)),
            ]),
        ),
    ]))
}

/// Op 37: **put a parameter under a controller** (an expression pedal, a footswitch, MIDI, or
/// Snapshots), or take it back off by naming source [`SOURCE_NONE`].
/// [solid — verified live on an HX Stomp 2026-08-22]
pub const OP_ASSIGN_PARAM: i64 = 37;

/// Key 74: the assignment's **source**, as an ordinal.
///
/// The same ordinal the document's controller table is indexed by, which is how the two decodes
/// were cross-checked: our own front-panel diff put FS1 at 3 and FS2 at 4, and `tonepush`'s list —
/// 0 none, 1-2 expression pedals, 3-7 footswitches, 8 MIDI CC, 9 snapshots — agrees.
pub const K_ASSIGN_SOURCE: i64 = 74;

/// Key 71: the MIDI CC number **when the source is MIDI**, and otherwise the constant `4`.
///
/// Worth the caution `tonepush` records: under any other source there is no CC to give, so the `4`
/// is meaningless — decide what this field means by the *source*, never by the value.
pub const K_ASSIGN_CC: i64 = 71;

/// Key 129, sent `false` by HX Edit on every assignment we have seen. Meaning unknown.
pub const K_ASSIGN_FLAG129: i64 = 129;

/// Source ordinal for **no controller** — what op 37 takes to remove an assignment.
pub const SOURCE_NONE: i64 = 0;

/// Source ordinal of **Footswitch 1**, on every device. FS`n` is `SOURCE_FS1 + n - 1`.
/// [solid — verified on an HX Stomp by front-panel diff 2026-08-21, and on an HX Stomp XL by the
/// same method 2026-08-25, where FS6 landed at ordinal 8.]
pub const SOURCE_FS1: i64 = 3;

/// Source ordinals above the footswitches, for a device with `footswitch_count` of them.
///
/// The ordinal space is **not fixed** — it stretches with the device, and reading it as a constant
/// ten was wrong. The key-`4` table is laid out `0` none, `1`/`2` the expression inputs, then one
/// entry per footswitch, then MIDI, then snapshots:
///
/// | | HX Stomp (5 switches) | HX Stomp XL (8) |
/// |---|---|---|
/// | footswitches | 3..=7 | 3..=10 |
/// | MIDI | 8 | 11 |
/// | snapshots | 9 | 12 |
/// | table length | 10 | 13 |
///
/// **`length == footswitch_count + 5` on every capture we hold** — six Stomp streams at 5 and 10,
/// four XL streams at 8 and 13 [solid]. The footswitch run is solid at both ends: FS1 = 3 on a
/// Stomp, and an XL's FS6 was diffed straight into index 8, which is the slot a Stomp calls MIDI.
/// That is the observation that killed the constant. A second XL preset then put a parameter under
/// **FS8** and it came back at ordinal **10**, which is what this computes — the far end of the run
/// confirmed rather than extrapolated [solid — issue #13, 2026-08-25].
///
/// **1 and 2 are EXP1 and EXP2** [solid — same report]. Two bypasses assigned to the two expression
/// inputs filed themselves at ordinals 1 and 2, each naming the block that had been put on that
/// pedal, so the labels are no longer `tonepush`'s word alone.
///
/// **MIDI is 11 and snapshots is 12 on an XL** [solid — issue #13, 2026-08-25,
/// `captures/xl_assign_midi_and_snapshots.msgpack.bin`]. Both were arithmetic until an owner put one
/// parameter under a MIDI CC and another under Snapshots on the same preset: the entries landed at
/// indices 11 and 12 of a 13-long table, and inner key `0` echoes the ordinal, so each is confirmed
/// by its position and by its own contents. That is the last of this table read off a device rather
/// than computed: the eight-switch shape is now observed end to end, so the formula is a description
/// of two pedals and not a fit to one. On a **Stomp** the top two remain `tonepush`'s naming plus an
/// op-37 write that was accepted at index 9 — the arithmetic agrees, but nobody has read 8 or 9 off
/// that panel, and it is the XL that carries the weight here.
pub mod source {
    /// Number of entries in the key-`4` controller table.
    pub fn table_len(footswitch_count: usize) -> usize {
        footswitch_count + 5
    }

    /// Ordinal of footswitch `n`, one-based. `None` if the device has no such switch.
    pub fn footswitch(n: usize, footswitch_count: usize) -> Option<i64> {
        (1..=footswitch_count)
            .contains(&n)
            .then(|| super::SOURCE_FS1 + n as i64 - 1)
    }

    /// Ordinal of the MIDI source. [solid on an XL — read back at 11; on a Stomp, 8 is `tonepush`'s
    /// naming of the same slot and has not been read off the panel]
    pub fn midi(footswitch_count: usize) -> i64 {
        super::SOURCE_FS1 + footswitch_count as i64
    }

    /// Ordinal of the snapshots source. [solid on an XL — read back at 12; on a Stomp, 9 was
    /// accepted by op 37 and filed at index 9, which is a write rather than a front-panel read]
    pub fn snapshots(footswitch_count: usize) -> i64 {
        midi(footswitch_count) + 1
    }

    /// Name the physical control an ordinal refers to, for display.
    ///
    /// Needs the device's footswitch count for the same reason the ordinals do: `8` is FS6 on an
    /// XL and MIDI on a Stomp, and naming it without asking showed an XL owner's front-panel
    /// assignment as "Driven by MIDI".
    ///
    /// A count of `0` means no preset is loaded and therefore no device to size against, so
    /// everything above the expression inputs prints as a bare ordinal rather than a guess.
    /// Anything outside the table does the same.
    pub fn name(ordinal: i64, footswitch_count: usize) -> String {
        match ordinal {
            // Verified on an XL, 2026-08-25: a bypass put on EXP1 filed itself at ordinal 1 and
            // named that block, EXP2 likewise at 2. These were `tonepush`'s names, held on the
            // reasoning that the footswitch run starting at 3 left them over; now they are read.
            1 => "EXP1".into(),
            2 => "EXP2".into(),
            _ if footswitch_count == 0 => format!("Controller {ordinal}"),
            n if (super::SOURCE_FS1..midi(footswitch_count)).contains(&n) => {
                format!("FS{}", n - super::SOURCE_FS1 + 1)
            }
            // Both read off an XL at 11 and 12 on 2026-08-25, which is where this puts them.
            n if n == midi(footswitch_count) => "MIDI".into(),
            n if n == snapshots(footswitch_count) => "Snapshots".into(),
            n => format!("Controller {n}"),
        }
    }
}

/// Build an **assign-parameter** body (op 37): put parameter `param_index` of the block in `slot`
/// under controller `source`. `paired` selects the paired cab's namespace. Pass
/// [`SOURCE_NONE`] to remove the assignment.
///
/// Edit-buffer only; a save is what makes it stick.
/// (`{102:txn, 100:37, 101:{98:slot, 26:paired, 28:param, 29:true, 74:source, 71:4, 129:false}}`.)
/// [solid — verified live on an HX Stomp 2026-08-22]
pub fn assign_param(slot: i64, paired: bool, param_index: i64, source: i64, txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_ASSIGN_PARAM)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SLOT), Value::from(slot)),
                (Value::from(K_MODEL_SEL), Value::from(i64::from(paired))),
                (Value::from(K_PARAM_INDEX), Value::from(param_index)),
                (Value::from(K_ASSIGN_IS_PARAM), Value::from(true)),
                (Value::from(K_ASSIGN_SOURCE), Value::from(source)),
                // Not a CC here: every source we can drive from a Stomp leaves this the constant
                // HX Edit sends. See [`K_ASSIGN_CC`].
                (Value::from(K_ASSIGN_CC), Value::from(4)),
                (Value::from(K_ASSIGN_FLAG129), Value::from(false)),
            ]),
        ),
    ]))
}

/// Op 65: set an assignment's **Min** — the value the parameter takes at the controller's heel /
/// off position. [solid — verified live on an HX Stomp 2026-08-22]
pub const OP_ASSIGN_MIN: i64 = 65;

/// Op 66: set an assignment's **Max** — the toe / on end. [solid — verified live on an HX Stomp 2026-08-22]
pub const OP_ASSIGN_MAX: i64 = 66;

/// Build a **set-assignment-travel** body (ops 65 and 66): move one end of an existing assignment's
/// range. `max` picks the end — `false` is Min (op 65), `true` is Max (op 66).
///
/// **The value is in the parameter's own units, not normalised** — a pitch block's ends are
/// semitones, a wah's are 0..1 because that is a wah's range. Reading them as percentages is what
/// made a pitch assignment look like it swept "700% to 1200%".
/// (`{102:txn, 100:65|66, 101:{98:slot, 26:paired, 28:param, 29:true, 119:value}}`.)
/// [solid — verified live on an HX Stomp 2026-08-22]
pub fn set_assign_travel(
    slot: i64,
    paired: bool,
    param_index: i64,
    max: bool,
    value: f32,
    txn: u16,
) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (
            Value::from(K_OP),
            Value::from(if max { OP_ASSIGN_MAX } else { OP_ASSIGN_MIN }),
        ),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_SLOT), Value::from(slot)),
                (Value::from(K_MODEL_SEL), Value::from(i64::from(paired))),
                (Value::from(K_PARAM_INDEX), Value::from(param_index)),
                (Value::from(K_ASSIGN_IS_PARAM), Value::from(true)),
                (Value::from(K_VALUE), Value::from(value)),
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

// ---- IR (impulse response) slots: the device's user-IR store ----
//
// A separate area from everything above: not the edit buffer, not the preset browse. The Stomp
// holds 128 user IR slots, each a fixed 2048-sample mono impulse, and HX Edit is the only way to
// put one there. All of it rides the **primary** channel inside a `SESSION_OPEN` (0x02) TLV — the
// same envelope the preset listing uses, not the 0x06 one every block edit uses.
//
// Provenance: `captures/{import,export}_ir.pcapng`, decoded 2026-06-28 and re-read 2026-08-22.
// Every builder here is byte-exact against those captures — see the tests.

/// Op 255: **open** an IR session. Brackets every IR transaction with [`OP_SESSION_END`].
///
/// The preset browse uses the same pair, though our listing only ever sends the 254 half — which
/// is worth knowing, because it means what `browse_open` calls an open is really this pair's
/// *close*, and the device is content to be handed one out of the blue.
pub const OP_SESSION_BEGIN: i64 = 255;
/// Op 254: **close** an IR session. The same number as [`OP_BROWSE_OPEN`]; see there.
pub const OP_SESSION_END: i64 = 254;
/// Op 9: **upload** an IR into a slot. Followed by [`OP_IR_COMMIT`].
pub const OP_IR_UPLOAD: i64 = 9;
/// Op 11: start the **blob stream** for the slot [`OP_IR_SELECT`] just named.
pub const OP_IR_STREAM: i64 = 11;
/// Op 12: **select** a slot for reading — and the reply carries that slot's whole metadata record,
/// which makes this the cheap way to enumerate the store without moving 8 KB per slot.
pub const OP_IR_SELECT: i64 = 12;
/// Op 13: **commit** a write. Its reply is the directory of populated slots.
pub const OP_IR_COMMIT: i64 = 13;
/// Op 15: **empty** a slot.
pub const OP_IR_DELETE: i64 = 15;
/// Op 10: **rename** the IR in a slot.
pub const OP_IR_RENAME: i64 = 10;

/// Target key: the IR **slot index**, zero-based.
pub const K_IR_SLOT: i64 = 112;
/// Target key: the blob's **checksum** — see [`ir_checksum`].
pub const K_IR_CHECKSUM: i64 = 113;
/// Target key: the IR **audio blob**, 8192 bytes.
pub const K_IR_BLOB: i64 = 110;
/// Target key 114: the **length multiplier**. With [`K_IR_LENGTH_EXP`] it declares how many
/// samples the device will store — see [`ir_length_code`].
pub const K_IR_LENGTH_MUL: i64 = 114;
/// Target key 115: the **length exponent**. See [`K_IR_LENGTH_MUL`].
pub const K_IR_LENGTH_EXP: i64 = 115;
/// Target key 123: echoed back verbatim; `false` everywhere seen. Preset list entries carry the
/// same trio, so it is not IR-specific.
pub const K_IR_FLAG_123: i64 = 123;
/// See [`K_IR_FLAG_123`].
pub const K_IR_FLAG_124: i64 = 124;
/// See [`K_IR_FLAG_123`].
pub const K_IR_FLAG_125: i64 = 125;

/// The length of an IR name field on the wire: 32 bytes, NUL-padded.
pub const IR_NAME_LEN: usize = 32;
/// The longest IR the device will store: 2048 samples. Declaring more, or sending more data than
/// the declared length covers, wedges the device's transfer state machine hard enough to need the
/// power pulled — so this is a ceiling, not a suggestion.
pub const IR_MAX_SAMPLES: usize = 2048;
/// The shortest stored length the code can express: `1 x 256 x 2^0`.
pub const IR_MIN_SAMPLES: usize = 256;
/// The length of a full-size IR audio blob: [`IR_MAX_SAMPLES`] little-endian `f32`.
pub const IR_BLOB_LEN: usize = IR_MAX_SAMPLES * 4;

/// The `113` checksum: the blob read as little-endian `u32` words and summed, truncated to 32 bits.
///
/// Not a CRC — crc32, crc32-inverted, adler32, byte-sum, big-endian sum and xor were all checked
/// against `import_ir.pcapng` and all differ. [solid — reproduces the captured `0xc0a076ed`]
pub fn ir_checksum(blob: &[u8]) -> u32 {
    blob.as_chunks::<4>()
        .0
        .iter()
        .fold(0u32, |acc, w| acc.wrapping_add(u32::from_le_bytes(*w)))
}

/// Build the **session-begin** body (op 255): `{102:txn, 100:255, 101:{}}`.
pub fn ir_session_begin(txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_SESSION_BEGIN)),
        (Value::from(K_TARGET), Value::Map(Vec::new())),
    ]))
}

/// Build the **session-end** body (op 254): `{102:txn, 100:254, 101:{}}`.
pub fn ir_session_end(txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_SESSION_END)),
        (Value::from(K_TARGET), Value::Map(Vec::new())),
    ]))
}

/// Build the **IR-select** body (op 12): `{102:txn, 100:12, 101:{112:slot}}`.
///
/// The reply is the slot's metadata record — index, checksum, name and the format flags — so this
/// alone reads the whole store. Selecting is a read; nothing is written and no blob moves.
pub fn ir_select(txn: u16, slot: i64) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_IR_SELECT)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![(Value::from(K_IR_SLOT), Value::from(slot))]),
        ),
    ]))
}

/// Build the **IR-stream-start** body (op 11): `{102:txn, 100:11, 101:{112:slot, 101:2}}`.
///
/// Sent after [`ir_select`] for the same slot. The blob pages back exactly like a preset document:
/// a declared length up front, then chunks.
pub fn ir_stream(txn: u16, slot: i64) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_IR_STREAM)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![
                (Value::from(K_IR_SLOT), Value::from(slot)),
                (Value::from(K_LIST_KIND), Value::from(2)),
            ]),
        ),
    ]))
}

/// Build the **IR-commit** body (op 13): `{102:txn, 100:13, 101:{101:2}}`.
///
/// Closes an [`ir_upload`], and answers with the directory of populated slots.
pub fn ir_commit(txn: u16) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_IR_COMMIT)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![(Value::from(K_LIST_KIND), Value::from(2))]),
        ),
    ]))
}

/// Write a MessagePack `str16` header, whatever the length.
///
/// `rmpv` picks the shortest encoding, which would put a 32-byte name in a `str8`. HX Edit sends
/// `str16` for both the name and the blob, and the difference is one byte on a command that writes
/// flash — so the two fixed-width fields are laid down by hand rather than left to the encoder.
fn write_str16(out: &mut Vec<u8>, bytes: &[u8]) {
    out.push(0xda);
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Pad `name` into the 32-byte NUL-padded field the wire carries. Over-long names are cut to 31
/// bytes on a `char` boundary so the field always ends in a NUL.
pub fn ir_name_field(name: &str) -> [u8; IR_NAME_LEN] {
    let mut field = [0u8; IR_NAME_LEN];
    let mut end = name.len().min(IR_NAME_LEN - 1);
    while end > 0 && !name.is_char_boundary(end) {
        end -= 1;
    }
    field[..end].copy_from_slice(&name.as_bytes()[..end]);
    field
}

/// The `114`/`115` pair declaring how many samples the device will store.
///
/// The device stores **`114 x 256 x 2^115`** samples, so with the multiplier pinned at 1 the
/// exponent alone selects 256, 512, 1024 or 2048. Returns `None` for any other count.
///
/// This is not decoration. Data **shorter** than the declared length is zero-padded and harmless;
/// data **longer** than it wedges the device's transfer state machine badly enough to need the
/// power pulled. Deriving the code from the sample count — rather than letting a caller state one —
/// is what makes that impossible to do by accident.
/// [solid — `tonepush`'s measured table, cross-checked against this device's own records: slot 0
/// reports `1, 3` and holds exactly 2048 samples.]
pub fn ir_length_code(samples: usize) -> Option<(i64, i64)> {
    if !(IR_MIN_SAMPLES..=IR_MAX_SAMPLES).contains(&samples) || !samples.is_power_of_two() {
        return None;
    }
    // samples = 256 << exp
    Some((1, (samples / IR_MIN_SAMPLES).trailing_zeros() as i64))
}

/// Build an **IR upload** (op 9): the slot, the checksum, the 32-byte name, the declared length
/// and the samples.
///
/// `blob` is little-endian `f32`, and its length has to be one the device stores — 256, 512, 1024
/// or 2048 samples. Anything else returns `None` rather than being padded silently, because the
/// declared length is derived from it and a mismatch is the one thing here that can wedge the
/// device.
///
/// **This writes device flash.** It must be followed by [`ir_commit`]. Its immediate reply carries
/// `103: 1`, not the usual `0` — the real verdict arrives afterwards as a status push echoing the
/// same transaction. [solid — `import_ir.pcapng`]
pub fn ir_upload(txn: u16, slot: i64, name: &str, blob: &[u8]) -> Option<Vec<u8>> {
    let (mul, exp) = ir_length_code(blob.len() / 4)?;
    if !blob.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(blob.len() + 128);
    // Three top-level pairs, then nine in the target. Hand-rolled only because of the two
    // `str16` fields; every scalar still goes through the encoder.
    out.push(0x83);
    let pair = |out: &mut Vec<u8>, k: i64, v: Value| {
        out.extend_from_slice(&encode(Value::from(k)));
        out.extend_from_slice(&encode(v));
    };
    pair(&mut out, K_TXN, Value::from(txn));
    pair(&mut out, K_OP, Value::from(OP_IR_UPLOAD));
    out.extend_from_slice(&encode(Value::from(K_TARGET)));
    out.push(0x89);
    pair(&mut out, K_IR_SLOT, Value::from(slot));
    pair(&mut out, K_IR_CHECKSUM, Value::from(ir_checksum(blob)));
    out.extend_from_slice(&encode(Value::from(K_NAME)));
    write_str16(&mut out, &ir_name_field(name));
    pair(&mut out, K_IR_LENGTH_MUL, Value::from(mul));
    pair(&mut out, K_IR_LENGTH_EXP, Value::from(exp));
    pair(&mut out, K_IR_FLAG_123, Value::from(false));
    pair(&mut out, K_IR_FLAG_124, Value::from(false));
    pair(&mut out, K_IR_FLAG_125, Value::from(0));
    out.extend_from_slice(&encode(Value::from(K_IR_BLOB)));
    write_str16(&mut out, blob);
    Some(out)
}

/// Build an **IR delete** (op 15): `{102:txn, 100:15, 101:{112:slot}}`. Empties the slot.
///
/// **Writes device flash.** Emptying an already-empty slot is not an error — the device answers 0.
pub fn ir_delete(txn: u16, slot: i64) -> Vec<u8> {
    encode(Value::Map(vec![
        (Value::from(K_TXN), Value::from(txn)),
        (Value::from(K_OP), Value::from(OP_IR_DELETE)),
        (
            Value::from(K_TARGET),
            Value::Map(vec![(Value::from(K_IR_SLOT), Value::from(slot))]),
        ),
    ]))
}

/// Build an **IR rename** (op 10): `{102:txn, 100:10, 101:{112:slot, 109:name}}`.
///
/// The name goes in the same 32-byte NUL-padded field an upload carries. **Writes device flash**,
/// but only the name — the samples are untouched.
pub fn ir_rename(txn: u16, slot: i64, name: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.push(0x83);
    let pair = |out: &mut Vec<u8>, k: i64, v: Value| {
        out.extend_from_slice(&encode(Value::from(k)));
        out.extend_from_slice(&encode(v));
    };
    pair(&mut out, K_TXN, Value::from(txn));
    pair(&mut out, K_OP, Value::from(OP_IR_RENAME));
    out.extend_from_slice(&encode(Value::from(K_TARGET)));
    out.push(0x82);
    pair(&mut out, K_IR_SLOT, Value::from(slot));
    out.extend_from_slice(&encode(Value::from(K_NAME)));
    write_str16(&mut out, &ir_name_field(name));
    out
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

    // ---- controller assignments ----
    //
    // These pin structure, not captured bytes: we have no HX Edit capture of an assignment being
    // made, and the wire shapes came from `tonepush`'s. What makes them trustworthy is the other
    // end — an HX Stomp accepted each of these bodies and changed its document accordingly
    // (2026-08-22), which is recorded on the builders themselves. A byte-exact test here would only
    // pin the encoder against itself.

    fn target_of(body: &[u8]) -> Value {
        let parsed = EditBody::parse(body).expect("builder output parses");
        get(&parsed.raw, K_TARGET)
            .expect("body has a target")
            .clone()
    }

    fn key(body: &[u8], k: i64) -> Option<i64> {
        get(&target_of(body), k).and_then(Value::as_i64)
    }

    #[test]
    fn a_footswitch_is_one_based_to_read_and_zero_based_to_write() {
        // The asymmetry is the device's, and it is the trap in this whole area: op 33 takes
        // Footswitch 1 as `1` (and answers `0`), while op 56 takes the same switch as `0`. Both
        // confirmed live — asking 33 for 1, 2, 3 answered 0, 1, 2, and `assign_bypass_to_switch(_, 0)`
        // landed on the layout's first position.
        assert_eq!(key(&read_switch(1, 0), K_SWITCH), Some(1));
        assert_eq!(key(&assign_bypass_to_switch(16, 0, 0), K_SWITCH), Some(0));
    }

    #[test]
    fn assigning_and_unassigning_a_bypass_differ_only_in_the_opcode() {
        let on = assign_bypass_to_switch(16, 2, 7);
        let off = unassign_bypass_from_switch(16, 2, 7);
        assert_eq!(EditBody::parse(&on).unwrap().op, OP_BYPASS_TO_SWITCH);
        assert_eq!(EditBody::parse(&off).unwrap().op, OP_BYPASS_OFF_SWITCH);
        assert_eq!(target_of(&on), target_of(&off));
    }

    #[test]
    fn removing_a_parameter_assignment_is_the_same_op_with_source_none() {
        // There is no separate "unassign parameter" opcode: op 37 with source 0 is the removal, and
        // it left the document back at its baseline live.
        let make = assign_param(16, false, 2, SOURCE_FS1, 9);
        let drop = assign_param(16, false, 2, SOURCE_NONE, 9);
        assert_eq!(EditBody::parse(&make).unwrap().op, OP_ASSIGN_PARAM);
        assert_eq!(EditBody::parse(&drop).unwrap().op, OP_ASSIGN_PARAM);
        assert_eq!(key(&make, K_ASSIGN_SOURCE), Some(SOURCE_FS1));
        assert_eq!(key(&drop, K_ASSIGN_SOURCE), Some(SOURCE_NONE));
    }

    #[test]
    fn a_request_puts_the_parameter_index_in_key_28() {
        // The document stores it the other way round — key 29 is the parameter and 28 is the path —
        // and reading one convention into the other is what made every assignment decode as
        // "param 0" until 2026-08-21. Key 29 in a *request* is the is-a-parameter flag.
        let body = assign_param(16, false, 2, SOURCE_FS1, 1);
        assert_eq!(key(&body, K_PARAM_INDEX), Some(2));
        assert_eq!(
            get(&target_of(&body), K_ASSIGN_IS_PARAM).and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn the_paired_cab_gets_its_own_namespace() {
        assert_eq!(
            key(&assign_param(3, true, 1, SOURCE_FS1, 1), K_MODEL_SEL),
            Some(1)
        );
        assert_eq!(
            key(&assign_param(3, false, 1, SOURCE_FS1, 1), K_MODEL_SEL),
            Some(0)
        );
    }

    #[test]
    fn min_and_max_are_two_opcodes_over_one_body() {
        let min = set_assign_travel(16, false, 2, false, 0.1, 4);
        let max = set_assign_travel(16, false, 2, true, 0.9, 4);
        assert_eq!(EditBody::parse(&min).unwrap().op, OP_ASSIGN_MIN);
        assert_eq!(EditBody::parse(&max).unwrap().op, OP_ASSIGN_MAX);
        assert_eq!(
            get(&target_of(&min), K_VALUE).and_then(Value::as_f64),
            Some(0.1_f32 as f64)
        );
    }

    #[test]
    fn bypass_tlv_wraps_with_correct_header() {
        let bytes = EditBody::parse(BYPASS_ON).unwrap().to_tlv().to_bytes();
        assert_eq!(&bytes[0..4], &[0x01, 0x00, 0x06, 0x00]); // marker + opcode 0x0006
        assert_eq!(&bytes[4..8], &[0x0d, 0x00, 0x00, 0x00]); // ilen = 13
        assert_eq!(&bytes[8..], BYPASS_ON);
    }

    // ---- IR slots: byte-exact against captures/{import,export}_ir.pcapng ----

    #[test]
    fn the_ir_session_brackets_match_the_capture() {
        assert_eq!(
            ir_session_begin(1005),
            [0x83, 0x66, 0xcd, 0x03, 0xed, 0x64, 0xcc, 0xff, 0x65, 0x80]
        );
        assert_eq!(
            ir_session_end(1008),
            [0x83, 0x66, 0xcd, 0x03, 0xf0, 0x64, 0xcc, 0xfe, 0x65, 0x80]
        );
    }

    #[test]
    fn selecting_and_streaming_an_ir_match_the_capture() {
        assert_eq!(
            ir_select(1006, 0),
            [
                0x83, 0x66, 0xcd, 0x03, 0xee, 0x64, 0x0c, 0x65, 0x81, 0x70, 0x00
            ]
        );
        assert_eq!(
            ir_stream(1007, 0),
            [
                0x83, 0x66, 0xcd, 0x03, 0xef, 0x64, 0x0b, 0x65, 0x82, 0x70, 0x00, 0x65, 0x02
            ]
        );
    }

    #[test]
    fn committing_an_ir_write_matches_the_capture() {
        assert_eq!(
            ir_commit(1011),
            [
                0x83, 0x66, 0xcd, 0x03, 0xf3, 0x64, 0x0d, 0x65, 0x81, 0x65, 0x02
            ]
        );
    }

    #[test]
    fn the_upload_header_matches_the_capture_byte_for_byte() {
        // The capture's own audio is a commercial IR and is not in this repo, so the blob here is
        // synthetic — one word carrying the whole checksum, the rest zero. That reproduces the
        // captured `113` exactly, which is what the header is being compared for: slot, checksum
        // encoding, the 32-byte NUL-padded `str16` name, all five flags, key order, and the blob's
        // own `str16` header.
        const HEADER: &[u8] = &[
            0x83, 0x66, 0xcd, 0x03, 0xf2, 0x64, 0x09, 0x65, 0x89, 0x70, 0x01, 0x71, 0xce, 0xc0,
            0xa0, 0x76, 0xed, 0x6d, 0xda, 0x00, 0x20, 0x47, 0x31, 0x32, 0x2d, 0x36, 0x35, 0x20,
            0x32, 0x31, 0x32, 0x20, 0x43, 0x20, 0x48, 0x69, 0x2d, 0x47, 0x6e, 0x20, 0x34, 0x32,
            0x31, 0x2b, 0x35, 0x37, 0x20, 0x43, 0x65, 0x6c, 0x65, 0x73, 0x00, 0x72, 0x01, 0x73,
            0x03, 0x7b, 0xc2, 0x7c, 0xc2, 0x7d, 0x00, 0x6e, 0xda, 0x20, 0x00,
        ];
        let mut blob = vec![0u8; IR_BLOB_LEN];
        blob[..4].copy_from_slice(&0xc0a0_76edu32.to_le_bytes());
        let body = ir_upload(0x03f2, 1, "G12-65 212 C Hi-Gn 421+57 Celes", &blob).unwrap();
        assert_eq!(&body[..HEADER.len()], HEADER);
        assert_eq!(body.len(), HEADER.len() + IR_BLOB_LEN);
        assert_eq!(&body[HEADER.len()..], &blob[..]);
    }

    #[test]
    fn the_checksum_is_a_little_endian_word_sum() {
        let mut blob = vec![0u8; IR_BLOB_LEN];
        blob[..4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        blob[4..8].copy_from_slice(&2u32.to_le_bytes());
        // Wraps rather than saturating or panicking in debug.
        assert_eq!(ir_checksum(&blob), 1);
    }

    #[test]
    fn a_blob_of_the_wrong_length_is_refused_rather_than_padded() {
        // Not a stored length: 2047 samples, and one sample past the ceiling.
        assert!(ir_upload(1, 0, "short", &vec![0u8; IR_BLOB_LEN - 4]).is_none());
        assert!(ir_upload(1, 0, "long", &vec![0u8; IR_BLOB_LEN + 4]).is_none());
        assert!(ir_upload(1, 0, "empty", &[]).is_none());
        // Not a whole number of samples.
        assert!(ir_upload(1, 0, "ragged", &vec![0u8; 1026]).is_none());
    }

    #[test]
    fn the_length_code_covers_exactly_the_stored_lengths() {
        assert_eq!(ir_length_code(256), Some((1, 0)));
        assert_eq!(ir_length_code(512), Some((1, 1)));
        assert_eq!(ir_length_code(1024), Some((1, 2)));
        assert_eq!(ir_length_code(2048), Some((1, 3)));
        // The formula the device applies, back the other way.
        for (mul, exp) in [(1, 0), (1, 1), (1, 2), (1, 3)] {
            let samples = mul * 256 * (1usize << exp);
            assert_eq!(ir_length_code(samples), Some((mul as i64, exp as i64)));
        }
        // Everything else is refused rather than rounded — over the ceiling is the case that
        // wedges the device.
        assert_eq!(ir_length_code(4096), None);
        assert_eq!(ir_length_code(2049), None);
        assert_eq!(ir_length_code(1500), None);
        assert_eq!(ir_length_code(128), None);
        assert_eq!(ir_length_code(0), None);
    }

    #[test]
    fn a_shorter_ir_declares_its_own_length() {
        // A 1024-sample upload must say `1, 2`, not the 2048 the capture happens to carry.
        let body = ir_upload(1, 0, "half", &vec![0u8; 4096]).unwrap();
        let hay = |k: u8, v: u8| body.windows(2).any(|w| w == [k, v]);
        assert!(hay(0x72, 0x01), "114 should be 1");
        assert!(hay(0x73, 0x02), "115 should be 2 for 1024 samples");
    }

    #[test]
    fn a_rename_carries_the_slot_and_the_padded_name() {
        let body = ir_rename(0x0102, 7, "new name");
        assert_eq!(&body[..2], &[0x83, 0x66]);
        // The name field is a 32-byte str16 like the upload's.
        let at = body
            .windows(3)
            .position(|w| w == [0xda, 0x00, 0x20])
            .unwrap();
        assert_eq!(&body[at + 3..at + 11], b"new name");
        assert_eq!(body.len(), at + 3 + IR_NAME_LEN);
    }

    #[test]
    fn a_delete_names_only_its_slot() {
        assert_eq!(
            ir_delete(0x0304, 9),
            [
                0x83, 0x66, 0xcd, 0x03, 0x04, 0x64, 0x0f, 0x65, 0x81, 0x70, 0x09
            ]
        );
    }

    #[test]
    fn an_overlong_ir_name_still_ends_in_a_nul() {
        let field = ir_name_field("this name is very much longer than the field allows");
        assert_eq!(field.len(), IR_NAME_LEN);
        assert_eq!(field[IR_NAME_LEN - 1], 0);
        assert!(field.starts_with(b"this name is very much longer t"));
    }

    #[test]
    fn a_multibyte_name_is_cut_on_a_char_boundary() {
        // 30 `a`s plus a two-byte `é` is 32 bytes, so the 31-byte cut would land mid-character;
        // the field backs off to 30 and stays valid UTF-8.
        let field = ir_name_field("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaé");
        assert_eq!(&field[..30], b"a".repeat(30).as_slice());
        assert_eq!(field[30], 0);
        assert!(std::str::from_utf8(&field[..30]).is_ok());
    }
    #[test]
    fn reading_a_setting_names_its_id() {
        // {102: 1, 100: 24, 101: {118: 16}}
        assert_eq!(
            read_setting(16, 1),
            [0x83, 0x66, 0x01, 0x64, 0x18, 0x65, 0x81, 0x76, 0x10]
        );
        // The handshake's fixed call is the same opcode with id 128.
        let prep = read_prep(0x03e9);
        assert_eq!(EditBody::parse(&prep).unwrap().op, OP_READ_SETTING);
    }

    #[test]
    fn a_setting_write_carries_the_type_it_is_given() {
        // An integer setting and a float one differ only in how 119 is encoded, and the device
        // refuses the wrong one with -3 — so the builder must not normalise them together.
        let as_int = set_setting(16, 132, 1);
        let as_f32 = set_setting_value(16, Value::F32(132.0), 1);
        assert_ne!(as_int, as_f32);
        assert!(as_int.ends_with(&[0x77, 0xcc, 0x84]), "119 -> uint8 132");
        assert!(
            as_f32.ends_with(&[0x77, 0xca, 0x43, 0x04, 0x00, 0x00]),
            "119 -> f32 132.0"
        );
        // Both still address the same setting.
        for body in [as_int, as_f32] {
            let e = EditBody::parse(&body).unwrap();
            assert_eq!(e.op, OP_SETTING);
        }
    }
}
