//! Decoder for the **MessagePack preset stream** the device sends when a preset is opened.
//!
//! The reassembled stream (concatenated chunk bodies) begins with a small envelope
//! (`marker:u16`, `type:u16`, `len:u32`, then an 8-byte context handle) before the MessagePack
//! root. Rather than hard-code that offset, [`locate_root`] scans for the offset whose
//! MessagePack value consumes the most of the buffer — robust to envelope-size changes. Callers
//! that know which key the envelope must carry should scan with [`locate_root_where`] instead;
//! "longest match" alone picks up a false root two lengths in every 256 (see that function).

use rmpv::Value;

/// Envelope map key whose value holds the nested preset blob (observed `104`).
const ENVELOPE_PRESET_KEY: i64 = 104;
/// Expected magic string at the head of the nested blob.
pub const PRESET_MAGIC: &str = "l6-helix";

/// A decoded preset stream: the `l6-helix` magic, an opaque header string, and the preset map.
#[derive(Debug, Clone)]
pub struct PresetStream {
    pub magic: String,
    /// The second sequence value — **not opaque**: a table of little-endian `u32` byte offsets into
    /// the blob. See [`HeaderSlot`] and [`PresetStream::header_slots`]. Kept verbatim so an
    /// unrecognised slot survives, but [`PresetStream::to_blob`] rewrites the ones it understands.
    pub header: Vec<u8>,
    /// The preset itself — an integer-keyed MessagePack map.
    pub preset: Value,
    /// What each 4-byte slot of `header` points at, worked out against the blob we parsed. Empty
    /// when the header isn't a whole number of `u32`s, in which case `to_blob` emits it verbatim.
    header_slots: Vec<HeaderSlot>,
}

/// One `u32` slot of the preset header, classified by what it addressed in the blob it came from.
///
/// The header is an **offset table**, which we spent a long time treating as an opaque uuid. Every
/// entry is a byte offset into the blob: where the preset map starts, where each of a handful of
/// top-level keys begins, and — twice, in the last two slots — the blob's total length. The device
/// seeks with these rather than walking the MessagePack, so a blob whose bytes have moved under a
/// stale table sends it reading into the middle of a value, or past the end of the buffer entirely.
///
/// [solid — 2026-07-31: `header[0]` is the map offset and `header[last]` the blob length on all four
/// captured presets; every interior slot lands exactly on a `<fixint key><container>` boundary]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderSlot {
    /// Offset of the preset map's first byte.
    MapStart,
    /// The blob's total length in bytes.
    TotalLen,
    /// Offset of the top-level map entry with this integer key.
    Key(i64),
    /// Not recognised — re-emitted unchanged.
    Raw(u32),
}

impl PresetStream {
    /// Parse a reassembled preset stream (concatenated chunk bodies) into its parts.
    ///
    /// Layout: a small envelope map `{.., 104: <blob>}` whose blob is a flat sequence of three
    /// MessagePack values — `str "l6-helix\0"`, a header string, and the preset map.
    pub fn parse(reassembled: &[u8]) -> crate::Result<PresetStream> {
        let root = locate_root_where(reassembled, 32, |v| {
            map_get(v, ENVELOPE_PRESET_KEY)
                .and_then(value_bytes)
                .is_some()
        })
        // Fall through to the unrestricted scan when nothing carries the key, so a genuinely
        // malformed stream still reports which part was wrong rather than "no root".
        .or_else(|| locate_root(reassembled, 32))
        .ok_or_else(|| crate::Error::Stream("no MessagePack envelope root".into()))?;
        let blob = map_get(&root.value, ENVELOPE_PRESET_KEY)
            .and_then(value_bytes)
            .ok_or_else(|| {
                // A refusal is well-formed, not corrupt — say so rather than accusing the decoder.
                // See `parse_preset_list` for the same check and the evidence behind it.
                if let Some((_, code)) = parse_edit_rejection(reassembled) {
                    return crate::Error::Stream(format!(
                        "the device refused to serve this preset (code {code})"
                    ));
                }
                crate::Error::Stream(format!(
                    "envelope key {ENVELOPE_PRESET_KEY} missing or not bytes"
                ))
            })?;

        let (seq, _) = read_sequence(blob, 3);
        let magic = match seq.first() {
            Some(Value::String(s)) => s.as_str().unwrap_or("").trim_end_matches('\0').to_string(),
            _ => {
                return Err(crate::Error::Stream(
                    "blob did not start with a magic string".into(),
                ));
            }
        };
        if magic != PRESET_MAGIC {
            return Err(crate::Error::Stream(format!("unexpected magic {magic:?}")));
        }
        let header = seq.get(1).and_then(value_bytes).unwrap_or(&[]).to_vec();
        let preset = seq
            .get(2)
            .filter(|v| matches!(v, Value::Map(_)))
            .cloned()
            .ok_or_else(|| crate::Error::Stream("preset map missing".into()))?;

        // Where the preset map begins in the blob we were handed: everything the first two values
        // consumed. Needed to classify the header's offsets against the bytes they were written for.
        let (_, map_at) = read_sequence(blob, 2);
        let header_slots = classify_header(&header, blob, map_at);

        Ok(PresetStream {
            magic,
            header,
            preset,
            header_slots,
        })
    }

    /// The header's offset table, as classified against the blob this stream was parsed from.
    /// Diagnostic — [`Self::to_blob`] consumes it directly.
    fn header_bytes(&self, map_at: usize, key_offsets: &[(i64, usize)], total: usize) -> Vec<u8> {
        if self.header_slots.is_empty() {
            return self.header.clone();
        }
        let mut out = Vec::with_capacity(self.header.len());
        for slot in &self.header_slots {
            let v = match *slot {
                HeaderSlot::MapStart => map_at as u32,
                HeaderSlot::TotalLen => total as u32,
                HeaderSlot::Key(k) => match key_offsets.iter().find(|(kk, _)| *kk == k) {
                    Some((_, off)) => *off as u32,
                    // The key vanished (nothing we do removes top-level keys, but don't guess an
                    // offset if it ever happens) — point at the end rather than into a value.
                    None => total as u32,
                },
                HeaderSlot::Raw(v) => v,
            };
            out.extend_from_slice(&v.to_le_bytes());
        }
        // Any trailing bytes that weren't a whole u32.
        out.extend_from_slice(&self.header[out.len().min(self.header.len())..]);
        out
    }

    /// Look up a top-level preset field by its integer key.
    pub fn field(&self, key: i64) -> Option<&Value> {
        map_get(&self.preset, key)
    }

    /// Mutable access to the preset map (for structural edits before re-serializing with [`to_blob`]).
    pub fn preset_mut(&mut self) -> &mut Value {
        &mut self.preset
    }

    /// Empty the slot at **wire slot** `slot`: set its kind to `8` (empty) and content to nil — the
    /// structural primitive behind **delete a block**. The slot number is global across DSPs
    /// (`dsp * 20 + index`), the same address the edit ops use. Returns `false` if that slot array
    /// or index isn't present. Re-serialize with [`to_blob`] and write via op 21 to apply.
    pub fn set_slot_empty(&mut self, slot: usize) -> bool {
        let (dsp, index) = split_wire_slot(slot as i64);
        let Some(&key) = DSP_GROUP_KEYS.get(dsp) else {
            return false;
        };
        let Some(group) = map_get_mut(&mut self.preset, key) else {
            return false;
        };
        let Some(Value::Array(slots)) = map_get_mut(group, 22) else {
            return false;
        };
        let Some(slot) = slots.get_mut(index) else {
            return false;
        };
        set_map_key(slot, 19, Value::from(slot_kind::EMPTY));
        set_map_key(slot, 20, Value::Nil);
        true
    }

    /// Set a structural node's signal-flow **column position** (2 = split, 3 = mixer) — the model
    /// holder's key `13`, the write mirror of [`Self::structural_node_pos`]. Only the holder carries
    /// the position (the companion 14/16 sub-map doesn't — [solid], dual_amp + split_preset
    /// fixtures). Returns `false` if the node or its holder isn't present. Re-serialize with
    /// [`to_blob`] and write via op 21 to apply — this is how the split/join points move along the
    /// top row without touching any block.
    pub fn set_node_pos(&mut self, kind: i64, pos: i64) -> bool {
        self.set_dsp_node_pos(0, kind, pos)
    }

    /// [`Self::set_node_pos`] for a specific DSP.
    pub fn set_dsp_node_pos(&mut self, dsp: usize, kind: i64, pos: i64) -> bool {
        let Some(&key) = DSP_GROUP_KEYS.get(dsp) else {
            return false;
        };
        let Some(group) = map_get_mut(&mut self.preset, key) else {
            return false;
        };
        let Some(Value::Array(slots)) = map_get_mut(group, 22) else {
            return false;
        };
        let Some(slot) = slots
            .iter_mut()
            .find(|s| map_get(s, 19).and_then(Value::as_i64) == Some(kind))
        else {
            return false;
        };
        let Some(Value::Map(content)) = map_get_mut(slot, 20) else {
            return false;
        };
        // The model holder is the content sub-map that carries key 8 (see `structural_node`).
        let Some(holder) = content
            .iter_mut()
            .map(|(_, v)| v)
            .find(|v| map_get(v, 8).is_some())
        else {
            return false;
        };
        set_map_key(holder, 13, Value::from(pos));
        true
    }

    /// Re-serialize to the nested blob the device round-trips: `magic ⧺ header ⧺ preset-map`, the
    /// exact byte sequence carried under read-stream key 104 and written back under op-21 key 110.
    ///
    /// The `magic` is re-NUL-terminated (parse strips it) and emitted as a `fixstr`; the preset map
    /// is re-encoded with rmpv (which preserves map key order and the f32/int value types); and the
    /// **header's offset table is rebuilt** against the bytes this call actually produces.
    ///
    /// That last part is the whole game. rmpv encodes integers minimally and the device does not —
    /// it writes plenty of `d1 00 00` (int16 zero) where one `00` would do — so our re-encode of an
    /// *untouched* preset comes out 117–216 bytes shorter than the device's, with everything after
    /// the first such integer shifted left. Copying the header verbatim across that shift left the
    /// device seeking to offsets that no longer began anything, and to a declared total length past
    /// the end of the buffer. Rebuilding the table makes the blob **self-consistent**, which is what
    /// the device actually requires; byte-identity with the original is not achievable through rmpv
    /// and isn't needed.
    ///
    /// [solid — 2026-07-31: a mixer drag froze a Floor twice, mid-write, with the stale table
    /// pointing 216 bytes past the end of the blob we were sending]
    pub fn to_blob(&self) -> Vec<u8> {
        let mut out = Vec::new();
        // magic, re-NUL-terminated → fixstr (e.g. "l6-helix\0" = 0xa9 …)
        let magic_z = format!("{}\0", self.magic);
        rmpv::encode::write_value(&mut out, &Value::from(magic_z))
            .expect("msgpack encode to Vec is infallible");
        // Reserve the header (its framing and length never change, so the map offset is fixed).
        push_str_header(&mut out, self.header.len());
        let header_at = out.len();
        out.extend_from_slice(&self.header);
        let map_at = out.len();
        // preset map
        rmpv::encode::write_value(&mut out, &self.preset)
            .expect("msgpack encode to Vec is infallible");

        // Re-point the offset table at what we just wrote.
        let key_offsets = top_level_entry_offsets(&out, map_at).unwrap_or_default();
        let header = self.header_bytes(map_at, &key_offsets, out.len());
        debug_assert_eq!(
            header.len(),
            self.header.len(),
            "header length must be fixed"
        );
        out[header_at..header_at + header.len()].copy_from_slice(&header);
        out
    }
}

/// Byte offset of every top-level entry of the map that starts at `map_at`, paired with its integer
/// key. The offset points at the entry's **key** byte, which is where the device's header table
/// points. `None` if the bytes at `map_at` aren't a map we can walk.
fn top_level_entry_offsets(blob: &[u8], map_at: usize) -> Option<Vec<(i64, usize)>> {
    let mut cur = blob.get(map_at..)?;
    // Map header: fixmap / map16 / map32. Read it by hand so we know how many entries follow and
    // where the first one starts.
    let (n, hdr) = match *cur.first()? {
        b @ 0x80..=0x8f => ((b & 0x0f) as usize, 1),
        0xde => (u16::from_be_bytes([*cur.get(1)?, *cur.get(2)?]) as usize, 3),
        0xdf => (
            u32::from_be_bytes([*cur.get(1)?, *cur.get(2)?, *cur.get(3)?, *cur.get(4)?]) as usize,
            5,
        ),
        _ => return None,
    };
    let mut pos = map_at + hdr;
    cur = blob.get(pos..)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let before = cur.len();
        let key = rmpv::decode::read_value(&mut cur).ok()?;
        // Skip the value; we only need where each entry began.
        rmpv::decode::read_value(&mut cur).ok()?;
        if let Some(k) = key.as_i64() {
            out.push((k, pos));
        }
        pos += before - cur.len();
    }
    Some(out)
}

/// Work out what each `u32` slot of `header` addressed in `blob` (whose preset map starts at
/// `map_at`), so [`PresetStream::to_blob`] can re-point them at the bytes it actually emits.
fn classify_header(header: &[u8], blob: &[u8], map_at: usize) -> Vec<HeaderSlot> {
    if !header.len().is_multiple_of(4) {
        return Vec::new();
    }
    let entries = top_level_entry_offsets(blob, map_at).unwrap_or_default();
    header
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| {
            let v = u32::from_le_bytes(*c);
            let at = v as usize;
            if at == blob.len() {
                HeaderSlot::TotalLen
            } else if at == map_at {
                HeaderSlot::MapStart
            } else if let Some((k, _)) = entries.iter().find(|(_, off)| *off == at) {
                HeaderSlot::Key(*k)
            } else {
                HeaderSlot::Raw(v)
            }
        })
        .collect()
}

/// Emit a MessagePack `str` length header for `len` bytes. Forces `str16` for the 32..65536 range
/// (what the device uses for the header/blob, rather than the minimal `str8`); `fixstr`/`str32` at
/// the extremes. The payload bytes are written by the caller (they may be non-UTF-8).
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

/// Slot kind (preset map `<dsp> → 22 → [i] → 19`).
pub mod slot_kind {
    pub const EFFECT: i64 = 6;
    /// A **Looper** block. Same role as [`EFFECT`] but a different content shape — model index at
    /// content key `8` (not `24 → 25`) and params at `7 → 4` (not `11 → 4`), like the structural
    /// nodes. Not device-specific: no Stomp fixture happened to contain a Looper, which is why it
    /// only surfaced in the Helix Floor captures. [solid — 2026-07-22, cross-checked against a
    /// `.hxb` backup]
    pub const LOOPER: i64 = 7;
    pub const EMPTY: i64 = 8;
    /// The parallel **split** node (kind 2) — fixed grid position; effect slots after it are row B.
    pub const SPLIT: i64 = 2;
    /// The parallel **mixer**/join node (kind 3) that recombines rows A and B.
    pub const MIXER: i64 = 3;
}

/// Preset keys holding a DSP's slot group, in DSP order. Each is `{21: split, 22: Array[20]}`.
///
/// Key `1` is `nil` on the single-DSP HX Stomp and populated on the Helix Floor — the device does
/// **not** widen the 20-slot array, it adds a second one. [solid — 2026-07-22]
pub const DSP_GROUP_KEYS: [i64; 2] = [0, 1];

/// Slots per DSP group, and therefore the stride between DSPs in the **wire slot** numbering.
///
/// Edit ops address a block by a single integer (target key `98`) that spans every DSP:
/// `wire_slot = dsp * DSP_SLOT_STRIDE + index`, so DSP1 is slots 0–19 and DSP2 is 20–39. There is
/// no DSP field in the envelope and none is needed — a DSP2 edit is byte-for-byte the same shape
/// as a DSP1 edit.
///
/// [solid — 2026-07-23] Established from a Helix Floor capture of five DSP2 blocks edited in HX
/// Edit: under this rule each `98` resolves to a block whose stored value for the swept parameter
/// is one UI increment from the first value on the wire, across five models and three parameter
/// scales — and it correctly distinguishes two identical `HD2_DelaySimpleDelayMono` blocks at
/// indices 7 and 17 of the same DSP. Consistent with every earlier capture (all slots < 20, all
/// DSP1). See `docs/helix-floor.md`.
///
/// Independently corroborated *inside* the preset: the footswitch layout (`3 → 8 → … → 11 → 8`)
/// numbers its targets the same way. In that Floor preset FS4/FS5 point at slots 27/28 and
/// FS10/FS11 at 37/38, and each layout entry's name matches the model found at that **global**
/// slot — including telling the two identical `Simple Delay` blocks apart as 27 and 37.
pub const DSP_SLOT_STRIDE: i64 = 20;

/// Wire slot number (edit target key `98`) for `index` within `dsp`'s slot array.
pub fn wire_slot(dsp: usize, index: usize) -> i64 {
    dsp as i64 * DSP_SLOT_STRIDE + index as i64
}

/// Split a wire slot number back into `(dsp, index)`.
pub fn split_wire_slot(slot: i64) -> (usize, usize) {
    (
        (slot / DSP_SLOT_STRIDE) as usize,
        (slot % DSP_SLOT_STRIDE) as usize,
    )
}

/// One block slot extracted from the device preset.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Which DSP's slot group this came from — the position of its preset key in
    /// [`DSP_GROUP_KEYS`]. `0` for every HX Stomp block; the Helix Floor also has `1`.
    pub dsp: usize,
    /// Position **within this DSP's** 20-slot array. Use [`Block::wire_slot`] for the edit address.
    pub index: usize,
    /// Raw slot type (`19`): 6 = effect, 7 = looper, 8 = empty, 0/1/2/3 = structural.
    pub kind: i64,
    /// Block bypass state — derived from content key `20 → 10` (the `enabled` bool, true = on);
    /// `bypassed = !enabled`. Verified live by toggling a block and diffing the stream.
    pub bypassed: Option<bool>,
    /// `24 → 25` — **the model identity**: an index into `Helix.sym`'s array order, which gives
    /// the device symbol (with `Mono`/`Stereo`) and thus the name + param order. (We'd earlier
    /// mistaken this for a buffer address by testing it against the *wrong* table — it doesn't
    /// index the 681-model `.models`, it indexes the 833-symbol `Helix.sym`.)
    pub model_ref: Option<i64>,
    /// `24 → 26` — paired model index (also into `Helix.sym`): the cab/IR fused into an amp+cab
    /// block. `-1` (→ `None`) when the block has no paired model.
    pub paired_ref: Option<i64>,
    /// `11 → 4` — ordered parameter values, in the model's `Helix.sym` order.
    pub params: Vec<ParamValue>,
    /// `12 → 4` — the paired model's parameter values (e.g. the cab's mic/cut params), empty when
    /// there is no paired model.
    pub paired_params: Vec<ParamValue>,
}

impl Block {
    /// This block's **wire slot** — the edit target (key `98`), global across DSPs.
    pub fn wire_slot(&self) -> i64 {
        wire_slot(self.dsp, self.index)
    }

    /// Whether this slot holds a real, addressable block (an effect or a Looper).
    pub fn is_block(&self) -> bool {
        self.kind == slot_kind::EFFECT || self.kind == slot_kind::LOOPER
    }
}

/// A single parameter value from a block's param vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Int(i64),
    Bool(bool),
}

impl ParamValue {
    pub(crate) fn from_value(v: &Value) -> Option<ParamValue> {
        match v {
            Value::F32(f) => Some(ParamValue::Float(*f)),
            Value::F64(f) => Some(ParamValue::Float(*f as f32)),
            Value::Integer(i) => i.as_i64().map(ParamValue::Int),
            Value::Boolean(b) => Some(ParamValue::Bool(*b)),
            _ => None,
        }
    }
}

impl PresetStream {
    /// Device model code (preset key `7 → 36`, e.g. `"P33"`).
    pub fn device_model(&self) -> Option<String> {
        self.field(7)
            .and_then(|m| map_get(m, 36))
            .and_then(value_bytes)
            .map(|b| {
                String::from_utf8_lossy(b)
                    .trim_end_matches('\0')
                    .to_string()
            })
    }

    /// Build stamp carried by the preset (key `7 → 37`), e.g. `"v3.71-32-g1039661"`.
    ///
    /// It looks like a firmware version and **is not one** [solid]: an HX Stomp running **3.80**
    /// reports `v3.71-32-g1039661`, and so does an HX Stomp XL running 3.80.0 — byte-identical, git
    /// hash included. One pedal is enough to refute it; the second only shows it is not per-unit.
    ///
    /// Read the suffix literally and it stops being a contradiction: `-32-g1039661` is `git
    /// describe` for *32 commits past the tag `v3.71`*. It names a build of some component inside
    /// the firmware image, whose last tag happens to be 3.71 — not a release. The Helix Floor gives
    /// a bare sha (`7d01f5e`) instead, i.e. a build with no tag behind it. [hypothesis]
    ///
    /// Do not reach for key `35` as the "real" version either: it reads `0x03800000` on this 3.80
    /// Stomp *and* in a 3.82 Floor's backup header, so it tracks something that did not change
    /// across those releases — a format revision, most likely. **No field we have decoded, on the
    /// wire or in a backup, reports the version on the pedal's boot screen.**
    /// [2026-08-21, owner report, issue #4]
    pub fn build_stamp(&self) -> Option<String> {
        self.field(7)
            .and_then(|m| map_get(m, 37))
            .and_then(value_bytes)
            .map(|b| {
                String::from_utf8_lossy(b)
                    .trim_end_matches('\0')
                    .to_string()
            })
    }

    /// The slot array of one DSP group (`DSP_GROUP_KEYS[dsp] → 22`), if that DSP is populated.
    fn dsp_slots(&self, dsp: usize) -> Option<&Vec<Value>> {
        let key = *DSP_GROUP_KEYS.get(dsp)?;
        match self.field(key).and_then(|m| map_get(m, 22)) {
            Some(Value::Array(a)) => Some(a),
            _ => None,
        }
    }

    /// Which DSPs this preset populates. `[0]` on the HX Stomp (key `1` is nil), `[0, 1]` on the
    /// Helix Floor.
    pub fn dsps(&self) -> Vec<usize> {
        (0..DSP_GROUP_KEYS.len())
            .filter(|&d| self.dsp_slots(d).is_some())
            .collect()
    }

    /// Extract the block slots of **every** populated DSP (`0 → 22`, and `1 → 22` on a two-DSP
    /// device). Empty slots are included (kind 8). Each [`Block`] carries its `dsp` and its
    /// index *within that DSP*; [`Block::wire_slot`] gives the global edit address.
    ///
    /// Reading only key `0` silently drops every DSP2 block, which is what we used to do.
    pub fn blocks(&self) -> Vec<Block> {
        self.dsps()
            .into_iter()
            .flat_map(|dsp| self.dsp_blocks(dsp))
            .collect()
    }

    /// The block slots of a single DSP group.
    pub fn dsp_blocks(&self, dsp: usize) -> Vec<Block> {
        let Some(slots) = self.dsp_slots(dsp) else {
            return Vec::new();
        };
        slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let kind = map_get(slot, 19).and_then(Value::as_i64).unwrap_or(-1);
                let content = map_get(slot, 20);
                let meta = content.and_then(|c| map_get(c, 24));
                // Block enabled/bypass: content key 10 is the `enabled` bool (true = on), verified
                // live by toggling a block and diffing the stream. `bypassed = !enabled`. Same key
                // for kind 6 and kind 7.
                let bypassed = content
                    .and_then(|c| map_get(c, 10))
                    .and_then(Value::as_bool)
                    .map(|enabled| !enabled);
                // Param vectors live at `<content> → <key> → 4` (key 11 = main, 12 = paired model;
                // key 7 for a Looper).
                let param_vec = |key: i64| -> Vec<ParamValue> {
                    content
                        .and_then(|c| map_get(c, key))
                        .and_then(|p| map_get(p, 4))
                        .and_then(|a| match a {
                            Value::Array(items) => {
                                Some(items.iter().filter_map(ParamValue::from_value).collect())
                            }
                            _ => None,
                        })
                        .unwrap_or_default()
                };
                // A Looper (kind 7) stores its identity and params like a structural node rather
                // than like an effect: model index directly at content key `8`, params at `7 → 4`.
                // It never has a paired model.
                let (model_ref, paired_ref, params, paired_params) = if kind == slot_kind::LOOPER {
                    let model = content.and_then(|c| map_get(c, 8)).and_then(Value::as_i64);
                    (model, None, param_vec(7), Vec::new())
                } else {
                    let model = meta.and_then(|m| map_get(m, 25)).and_then(Value::as_i64);
                    // `26` = paired cab/IR index; -1 means "none".
                    let paired = meta
                        .and_then(|m| map_get(m, 26))
                        .and_then(Value::as_i64)
                        .filter(|&r| r >= 0);
                    (model, paired, param_vec(11), param_vec(12))
                };
                Block {
                    dsp,
                    index,
                    kind,
                    bypassed,
                    model_ref,
                    paired_ref,
                    params,
                    paired_params,
                }
            })
            .collect()
    }

    /// Whether **DSP 0** uses a parallel (split) topology. See [`Self::dsp_is_split`].
    pub fn is_split(&self) -> bool {
        self.dsp_is_split(0)
    }

    /// Whether `dsp` uses a parallel (split) topology. Its group key `21`: `0` = single serial
    /// path, **non-zero = split, and the value is the split type**. Proven by diffing a serial
    /// preset against the same blocks moved to a parallel row (the flag flipped `0 → 1`).
    ///
    /// This used to test `== 1`, which was an HX Stomp undergeneralization — the Floor uses `2`
    /// and `3` for its other split types. Across all five presets we can check, `21 == 0` holds
    /// exactly when the DSP's row-B slots (11..18) are all empty: Stomp `preset1` = 0/none,
    /// `dual_amp` = 1/one, `split_preset` = 1/one, Floor "Pull Me Under" = 2/two on DSP1 and
    /// 3/six on DSP2. [solid — 2026-07-23]
    ///
    /// Per-DSP rather than preset-wide: each DSP has its own A/B branch and its own flag, and on
    /// the Floor the two commonly differ within one preset.
    pub fn dsp_is_split(&self, dsp: usize) -> bool {
        let Some(&key) = DSP_GROUP_KEYS.get(dsp) else {
            return false;
        };
        !matches!(
            self.field(key)
                .and_then(|m| map_get(m, 21))
                .and_then(Value::as_i64),
            None | Some(0)
        )
    }

    /// Just the populated blocks — effects **and** loopers — across every DSP.
    pub fn effect_blocks(&self) -> Vec<Block> {
        self.blocks().into_iter().filter(Block::is_block).collect()
    }

    /// The editor's view of every DSP block in the preset, enumerated **from the slot array**
    /// (`0 → 22`, kind 6) — the device's authoritative block store. Identity comes from the
    /// `Helix.sym` index (`24 → 25`), so this works for serial **and** split presets and includes
    /// blocks that aren't on any footswitch (which the `3 → 8` layout omits entirely). Footswitch
    /// binding, node kind, and user label are enriched from the footswitch layout when the block
    /// is bound to a switch.
    pub fn loaded_blocks(&self) -> Vec<LoadedBlock> {
        // Index the (optional) footswitch layout by slot for label/footswitch enrichment. **Only
        // DSP-type nodes (`11 → 0 == 1`) enrich a block** — a controller node (type 2) can point at a
        // DSP block's slot (a controller assigned to one of its params), and matching that by slot
        // would wrongly reclassify the real block as a controller. Controllers are surfaced
        // separately via [`assignments`]. A kind-6 slot-array entry is always a DSP block.
        let path: std::collections::HashMap<i64, PathBlock> = self
            .footswitch_layout()
            .into_iter()
            .flatten()
            .filter(|pb| pb.node_kind != Some(2))
            .filter_map(|pb| pb.slot.map(|s| (s, pb)))
            .collect();

        let all = self.blocks();
        // The 20-slot array is a fixed topology grid: input, row A, a split node (kind 2), row B,
        // then a mixer (kind 3). Row B = effect slots **after** the split node. [solid] — from the
        // slot dump (split at 10, row B 11-18) and the move-to-parallel capture (block → slot 12).
        // Each DSP has its own split node, so this is resolved per DSP, not once for the preset.
        let split_idx = |dsp: usize| {
            all.iter()
                .find(|b| b.dsp == dsp && b.kind == slot_kind::SPLIT)
                .map(|b| b.index)
        };
        all.iter()
            .filter(|b| b.is_block())
            .map(|b| {
                let slot = b.wire_slot();
                let pb = path.get(&slot);
                LoadedBlock {
                    dsp: b.dsp,
                    slot,
                    model_index: b.model_ref,
                    paired_index: b.paired_ref,
                    user_label: pb.and_then(|p| p.user_label.clone()),
                    bypassed: b.bypassed,
                    params: b.params.clone(),
                    paired_params: b.paired_params.clone(),
                    node_kind: pb.and_then(|p| p.node_kind),
                    // Layout position + 1 when the block is on a footswitch; 0 = not bound.
                    footswitch: pb.map_or(0, |p| p.footswitch),
                    // Row B = a slot after this DSP's split node; row A otherwise.
                    row: match split_idx(b.dsp) {
                        Some(s) if b.index > s => 1,
                        _ => 0,
                    },
                }
            })
            .collect()
    }

    /// The structural routing node of a given `kind` (2 = split, 3 = mixer/join) as a
    /// [`LoadedBlock`], if the grid has one — used to expose the split-type selector and the
    /// mixer's A/B level/pan params (both edited with the ordinary ops on the node's own slot).
    ///
    /// Unlike an effect block (model at `content → 24 → 25`, params at `content → 11 → 4`), a routing
    /// node stores its model + params in a **content sub-map** (key 15 for a split, 17 for a mixer):
    /// `8` = model index, `7 → 4` = param values, `10` = enabled. The node also has a companion
    /// sub-map (14/16) for the merge side with no model — so we locate the model holder as the
    /// content value that actually carries key `8`. Verified against `split_preset_stream` (split =
    /// Split Y, index 257; mixer = HD2_AppDSPFlowJoin, index 151, 6 A/B params). [solid]
    pub fn structural_node(&self, kind: i64) -> Option<LoadedBlock> {
        self.dsp_structural_node(0, kind)
    }

    /// [`Self::structural_node`] for a specific DSP. Each DSP has its own split and mixer.
    pub fn dsp_structural_node(&self, dsp: usize, kind: i64) -> Option<LoadedBlock> {
        let slots = self.dsp_slots(dsp)?;
        let (index, slot) = slots
            .iter()
            .enumerate()
            .find(|(_, s)| map_get(s, 19).and_then(Value::as_i64) == Some(kind))?;
        let content = map_get(slot, 20)?;
        let holder = match content {
            Value::Map(m) => m.iter().map(|(_, v)| v).find(|v| map_get(v, 8).is_some())?,
            _ => return None,
        };
        let model_index = map_get(holder, 8).and_then(Value::as_i64);
        let bypassed = map_get(holder, 10)
            .and_then(Value::as_bool)
            .map(|enabled| !enabled);
        let params = map_get(holder, 7)
            .and_then(|m| map_get(m, 4))
            .and_then(|a| match a {
                Value::Array(items) => {
                    Some(items.iter().filter_map(ParamValue::from_value).collect())
                }
                _ => None,
            })
            .unwrap_or_default();
        Some(LoadedBlock {
            dsp,
            slot: wire_slot(dsp, index),
            model_index,
            paired_index: None,
            user_label: None,
            bypassed,
            params,
            paired_params: Vec::new(),
            node_kind: None,
            footswitch: 0,
            row: 0,
        })
    }

    /// The fixed **input / output node** (kind 0 at slot 0, kind 1 at slot 9) as a [`LoadedBlock`].
    /// Unlike split/mixer these carry no model reference — their content is
    /// `{5: input-from (input) / 6: output-to (output), 7 → 4: [param values]}`. The stored values
    /// are the leading params of the device symbol's order (`HelixStomp_AppDSPFlowInput`:
    /// noiseGate/threshold/decay; `…OutputMain`: pan/gain), and they're edited with the ordinary
    /// set-value op on the node's own slot — `switch_input_gate_and_guitar_pad.pcapng` shows the
    /// input gate toggle as op 30 `{98:0, 28:0, 119:bool}`. [solid]
    pub fn io_node(&self, kind: i64) -> Option<LoadedBlock> {
        self.dsp_io_node(0, kind)
    }

    /// [`Self::io_node`] for a specific DSP. Each DSP has its own input and output node.
    pub fn dsp_io_node(&self, dsp: usize, kind: i64) -> Option<LoadedBlock> {
        if kind != 0 && kind != 1 {
            return None;
        }
        let slots = self.dsp_slots(dsp)?;
        let (index, slot) = slots
            .iter()
            .enumerate()
            .find(|(_, s)| map_get(s, 19).and_then(Value::as_i64) == Some(kind))?;
        let content = map_get(slot, 20)?;
        let params = map_get(content, 7)
            .and_then(|m| map_get(m, 4))
            .and_then(|a| match a {
                Value::Array(items) => {
                    Some(items.iter().filter_map(ParamValue::from_value).collect())
                }
                _ => None,
            })
            .unwrap_or_default();
        Some(LoadedBlock {
            dsp,
            slot: wire_slot(dsp, index),
            model_index: None,
            paired_index: None,
            user_label: None,
            bypassed: None,
            params,
            paired_params: Vec::new(),
            node_kind: None,
            footswitch: 0,
            row: 0,
        })
    }

    /// The editor's **grid** view: one cell per draggable slot (effect or empty), with its row and
    /// display column, for the routing UI. The 20-slot array is a fixed topology — `[0 = input,
    /// 1..=8 = top row, 9 = output, 10 = split node, 11..=18 = row B, 19 = mixer node]` [solid,
    /// verified against preset1/dual_amp: slot 0 is kind 0, slot 9 is kind 1] — so the grid is
    /// **8 columns wide in both rows**, and each row's slot index *is* its column:
    /// - top row: `row 0`, `column = slot` (slots 1..=8 → columns 1..=8);
    /// - row B: `row 1`, `column = slot − 10` (slots 11..=18 → columns 1..=8), so B sits in the
    ///   same absolute column space as A — a row-B block at column `c` is directly under the
    ///   row-A slot `c`.
    ///
    /// Row B's column is **not** derived from the split node's signal-flow position: the node
    /// positions ([`Self::structural_node_pos`]) say where the bracket opens and closes, the slot
    /// index says which column a block occupies, and the device keeps the two consistent.
    ///
    /// The split/mixer node slots exist **even on serial presets** (kinds 2/3 with `is_split() ==
    /// false` — [solid], preset1 fixture), so the empty row-B cells are always emitted: dropping a
    /// block into one on a serial preset is how the split gets *created* (the device activates it).
    ///
    /// A top-row cell's routing *role* (common-before / path A / common-after) is derived from its
    /// column vs [`Self::structural_node_pos`] — the device recomputes those as blocks move, so the UI
    /// re-reads after each placement. Each cell maps to exactly one slot: dropping a block onto an
    /// empty cell is a single move to that slot.
    pub fn grid(&self) -> Vec<GridCell> {
        self.dsps()
            .into_iter()
            .flat_map(|d| self.dsp_grid(d))
            .collect()
    }

    /// [`Self::grid`] for a single DSP. `row` is 0/1 **within that DSP** — a two-DSP device draws
    /// four rows, which the caller composes from `cell.dsp` and `cell.row`.
    pub fn dsp_grid(&self, dsp: usize) -> Vec<GridCell> {
        let blocks = self.dsp_blocks(dsp);
        let split_idx = blocks
            .iter()
            .find(|b| b.kind == slot_kind::SPLIT)
            .map(|b| b.index);
        let mixer_idx = blocks
            .iter()
            .find(|b| b.kind == slot_kind::MIXER)
            .map(|b| b.index);
        let is_cell =
            |k: i64| k == slot_kind::EFFECT || k == slot_kind::LOOPER || k == slot_kind::EMPTY;
        let mut cells = Vec::new();
        for b in &blocks {
            if b.index == 0 || !is_cell(b.kind) {
                continue; // skip the input slot and the split/mixer nodes
            }
            let occupied = b.is_block();
            let (row, column) = match split_idx {
                Some(s) if b.index > s => match mixer_idx {
                    // Row B: between the split and mixer nodes.
                    Some(m) if b.index < m => (1u8, (b.index - s) as i64),
                    None => (1u8, (b.index - s) as i64),
                    // A slot at/after the mixer node index (only the mixer itself) — skip.
                    Some(_) => continue,
                },
                // Top row: serial preset, or before the split node.
                _ => (0u8, b.index as i64),
            };
            cells.push(GridCell {
                dsp,
                slot: b.wire_slot(),
                row,
                column,
                occupied,
            });
        }
        cells
    }

    /// The signal-flow **column position** of a structural node (2 = split, 3 = mixer) — the node's
    /// model-holder key `13`. This locates the split/mixer along the top row: a top-row block at slot
    /// `s` is **common-before** if `s < split_pos`, **path A** if `split_pos ≤ s < mixer_pos`, and
    /// **common-after** if `s ≥ mixer_pos`. Bottom-row slots (after the split node) are always path B.
    /// Decoded from the "Dual Amp" preset (split `13=5`, mixer `13=7`; Tremolo@4 before, US Princess@6
    /// on A, Reverb@7 after, GSG@15 on B). [solid]
    pub fn structural_node_pos(&self, kind: i64) -> Option<i64> {
        self.dsp_structural_node_pos(0, kind)
    }

    /// [`Self::structural_node_pos`] for a specific DSP.
    ///
    /// **A Floor's split can span both DSPs, and then one end of the bracket is not on this DSP.**
    /// The `pullmeunder` dump splits after a common Volume+Comp on DSP1 and rejoins at the end of
    /// DSP2: DSP1 reports `split = 2, mixer = 0` and DSP2 reports `split = 0, mixer = 9`, with
    /// both DSPs `is_split()`. Read a `0` on the side that has no bracket end as "the path is
    /// already open here / closes downstream", not as column 0 — the "common-before / path A /
    /// common-after" rule on [`Self::structural_node_pos`] assumes both ends are present and does
    /// **not** hold for such a preset. [hypothesis — one observed preset; the `0` could equally be
    /// an absent-value default, and no screenshot of this preset's grid has been seen, so how the
    /// device *draws* the row-B columns across the boundary is still unconfirmed.]
    pub fn dsp_structural_node_pos(&self, dsp: usize, kind: i64) -> Option<i64> {
        let slots = self.dsp_slots(dsp)?;
        let slot = slots
            .iter()
            .find(|s| map_get(s, 19).and_then(Value::as_i64) == Some(kind))?;
        let content = map_get(slot, 20)?;
        let holder = match content {
            Value::Map(m) => m.iter().map(|(_, v)| v).find(|v| map_get(v, 8).is_some())?,
            _ => return None,
        };
        map_get(holder, 13).and_then(Value::as_i64)
    }

    /// The **footswitch / stomp layout** (preset key `3 → 8`): one entry per footswitch position,
    /// `None` where that switch is unbound. Each populated entry names the block whose **bypass**
    /// that footswitch toggles, with its slot (`11 → 8`) and user label. **This is not the signal
    /// path** — it lists only footswitch-bound blocks, in footswitch order, and is empty when
    /// nothing is on a switch. Proven by binding a block to FS1 and watching position `[0]` go from
    /// `nil` → that block's node (and earlier by an FS1↔FS2 swap diff). Use [`loaded_blocks`] for
    /// the full block list.
    pub fn footswitch_layout(&self) -> Vec<Option<PathBlock>> {
        let positions = match self.field(3).and_then(|m| map_get(m, 8)) {
            Some(Value::Array(a)) => a,
            _ => return Vec::new(),
        };
        positions
            .iter()
            .enumerate()
            .map(|(pos_index, pos)| {
                // each populated position is Array[1] of a Map{7} node.
                let node = match pos {
                    Value::Array(a) => a.first()?,
                    _ => return None,
                };
                let model = map_get(node, 11)?;
                let model_name = map_get(model, 5).and_then(value_bytes).map(|b| {
                    String::from_utf8_lossy(b)
                        .trim_end_matches('\0')
                        .to_string()
                })?;
                let model_id = map_get(model, 6).and_then(Value::as_i64);
                // Node type (`11 → 0`): 1 = DSP block, 2 = controller/footswitch node (e.g. an amp
                // switch mapped to a button — its name like "OD Sw" is the footswitch label, not a
                // model). Verified by comparing a serial preset (all type 1) to a dual-amp preset.
                let node_kind = map_get(model, 0).and_then(Value::as_i64);
                let user_label = map_get(node, 14)
                    .and_then(value_bytes)
                    .map(|b| {
                        String::from_utf8_lossy(b)
                            .trim_end_matches('\0')
                            .to_string()
                    })
                    .filter(|s| !s.is_empty());
                let slot = map_get(model, 8).and_then(Value::as_i64);
                // Footswitch = layout position + 1 (FS1 = pos 0); empty positions leave that switch
                // unbound (→ global tap/tuner). Proven by an FS1↔FS2 swap diff + an FS1-bind diff.
                Some(PathBlock {
                    model_name,
                    model_id,
                    user_label,
                    slot,
                    node_kind,
                    footswitch: pos_index as i64 + 1,
                })
            })
            .collect()
    }
}

/// A footswitch/controller assignment from the preset's assignment table (key `4`).
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    /// Controller/footswitch number (assignment-table index, echoed at inner key `0`).
    pub controller: i64,
    /// Controller type (inner key `1`; e.g. 4 = footswitch). Raw — semantics TBD.
    pub ctype: Option<i64>,
    /// Target block slot the control drives (inner key `5`).
    pub target_slot: Option<i64>,
    /// Target parameter index within that block (inner key `6 → 28`).
    pub param_index: Option<i64>,
    /// Min / max values the control sweeps between (inner keys `4` / `7`).
    pub min: Option<i64>,
    pub max: Option<i64>,
}

/// One snapshot's stored state, from a preset's key `10 → 10` array.
///
/// A snapshot is a per-preset scene: which blocks are on, at what tempo. Decoded from the two
/// Stomp split fixtures; the bypass matrix is [solid] (see [`PresetStream::snapshot_details`]),
/// the rest [partial].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    /// Display name (key `4`), NUL-trimmed.
    pub name: String,
    /// Key `0` — true on every snapshot in the fixtures, so read as "slot in use" rather than
    /// anything the user toggles. [hypothesis]
    pub in_use: bool,
    /// Key `5` — 120 in every fixture, which is the Helix's default BPM. [hypothesis — plausible
    /// as the snapshot's stored tempo, but never observed at another value]
    pub tempo: Option<i64>,
    /// Per-slot **enabled** state in this snapshot, indexed by slot in the same array the blocks
    /// use (key `3`: one `[_, enabled]` pair per slot). `true` = block active, `false` = bypassed
    /// — the inverse of [`Block::bypassed`].
    ///
    /// The array spans the device's **whole** slot space, indexed by [`Block::wire_slot`]
    /// (`dsp * 20 + index`) — 20 entries on the Stomp, **40 on a two-DSP Floor**, as one flat
    /// array rather than one per DSP. [solid — the `pullmeunder` Floor dump: 40 entries, and its
    /// DSP2 entries (27/28 clean delay+reverb, 36..=38 gain delay+reverb) are what make its
    /// snapshots read as coherent scenes.] Index this by wire slot, never by the per-DSP index:
    /// a DSP2 block looked up at its local index silently reports DSP1's state.
    pub block_enabled: Vec<bool>,
}

impl PresetStream {
    /// Per-snapshot stored state: name, tempo, and the **bypass matrix** (which blocks are on in
    /// each snapshot).
    ///
    /// The matrix is [solid]: in `preset1_stream`, the live blocks are bypassed at slots 2/3/4/7
    /// and active at 5/6, which is exactly snapshot 0's `key 3` — and `key 10 → 8` reports 0.
    ///
    /// **`key 10 → 8` is not reliably the live snapshot.** In `dual_amp_stream` it reports 1, but
    /// the live block state (slot 4 bypassed, 6/7/15 active) matches snapshot **0**; snapshots 1
    /// and 2 there are pristine "everything on". So the stored index and the stored scene disagree
    /// in that fixture. This is the most likely explanation for the GUI highlighting the wrong
    /// snapshot on hardware — see `docs/helix-floor.md`. Deriving the live snapshot by matching
    /// this matrix against the live block states is a candidate fix, but it is ambiguous whenever
    /// two snapshots hold identical scenes (as snapshots 1 and 2 do in `preset1_stream`), so it is
    /// deliberately not done here.
    pub fn snapshot_details(&self) -> Vec<SnapshotInfo> {
        let Some(Value::Array(snaps)) = self.field(10).and_then(|m| map_get(m, 10)) else {
            return Vec::new();
        };
        snaps
            .iter()
            .map(|s| SnapshotInfo {
                name: map_get(s, 4)
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim_end_matches('\0').to_string())
                    .unwrap_or_default(),
                in_use: map_get(s, 0).and_then(Value::as_bool).unwrap_or(false),
                tempo: map_get(s, 5).and_then(Value::as_i64),
                // Each entry is a 2-element array; the *second* element is the enabled flag (the
                // first is false throughout every fixture, so it discriminates nothing).
                block_enabled: match map_get(s, 3) {
                    Some(Value::Array(slots)) => slots
                        .iter()
                        .map(|e| match e {
                            Value::Array(pair) => {
                                pair.get(1).and_then(Value::as_bool).unwrap_or(false)
                            }
                            _ => false,
                        })
                        .collect(),
                    _ => Vec::new(),
                },
            })
            .collect()
    }

    /// Snapshot names and the active snapshot index (preset key `10`: `8` = active index,
    /// `10` = `Array` of snapshot maps each with `4` = name). See [`Self::snapshot_details`] for
    /// the full per-snapshot state — and for why the active index is not fully trustworthy.
    pub fn snapshots(&self) -> (Option<i64>, Vec<String>) {
        let root = self.field(10);
        let active = root.and_then(|m| map_get(m, 8)).and_then(Value::as_i64);
        let names = match root.and_then(|m| map_get(m, 10)) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|snap| {
                    map_get(snap, 4)
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim_end_matches('\0').to_string())
                })
                .collect(),
            _ => Vec::new(),
        };
        (active, names)
    }

    /// Decode the footswitch/controller assignment table (preset key `4`, `Array[10]`). Returns
    /// only populated entries. Each is `Array[1]` of `{0:0, 1: {0:ctrl, 1:type, 5:slot,
    /// 6:{28:param}, 4:min, 7:max, …}}`. [partial — structure verified live, some flags raw]
    pub fn assignments(&self) -> Vec<Assignment> {
        let table = match self.field(4) {
            Some(Value::Array(a)) => a,
            _ => return Vec::new(),
        };
        table
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let def = match entry {
                    Value::Array(a) => map_get(a.first()?, 1)?,
                    _ => return None,
                };
                let g = |k: i64| map_get(def, k).and_then(Value::as_i64);
                Some(Assignment {
                    controller: g(0).unwrap_or(i as i64),
                    ctype: g(1),
                    target_slot: g(5),
                    param_index: map_get(def, 6)
                        .and_then(|p| map_get(p, 28))
                        .and_then(Value::as_i64),
                    min: g(4),
                    max: g(7),
                })
            })
            .collect()
    }
}

/// One draggable cell in the routing [`grid`](PresetStream::grid): the slot it maps to, its display
/// row (0 = top, 1 = parallel B) and column, and whether a block currently occupies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridCell {
    /// Which DSP this cell belongs to. A two-DSP device draws `dsp × row` rows.
    pub dsp: usize,
    /// Wire slot (global across DSPs) — the drop target address.
    pub slot: i64,
    /// Row **within** this DSP: 0 = path A, 1 = path B.
    pub row: u8,
    pub column: i64,
    pub occupied: bool,
}

/// A block as an editor sees it: identity (the `Helix.sym` index) plus current state, enumerated
/// from the slot array. Produced by [`PresetStream::loaded_blocks`]. The caller resolves the
/// index to a name + param order via [`crate::symbols::DeviceSymbols::by_index`].
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedBlock {
    /// Which DSP holds this block (`0` for every HX Stomp block; the Floor also has `1`).
    pub dsp: usize,
    /// **Wire slot** — global across DSPs (`dsp * 20 + index`), and the edit address (key 98).
    pub slot: i64,
    /// `Helix.sym` index of the block's model (`24 → 25`, or content key `8` for a Looper).
    pub model_index: Option<i64>,
    /// `Helix.sym` index of the paired cab/IR (`24 → 26`), if any.
    pub paired_index: Option<i64>,
    pub user_label: Option<String>,
    /// Bypass state (content key `10` = enabled; `bypassed = !enabled`).
    pub bypassed: Option<bool>,
    /// Ordered parameter values, in the model's `Helix.sym` order.
    pub params: Vec<ParamValue>,
    /// Paired model's parameter values (the cab's mic/cut params), empty if no paired model.
    pub paired_params: Vec<ParamValue>,
    /// Node type from the footswitch layout (`… → 11 → 0`): 1 = DSP block, 2 = controller node;
    /// `None` when the block isn't on a footswitch.
    pub node_kind: Option<i64>,
    /// Footswitch this block's bypass is bound to (= layout position + 1); `0` = not on a switch.
    pub footswitch: i64,
    /// Signal row **within this block's DSP**: 0 = main (top), 1 = parallel (B). Derived from the
    /// split node — a slot after this DSP's split (index 10) is row B. [solid — the `pullmeunder`
    /// Floor dump puts DSP1 11/12 and DSP2 33..=38 on row B, DSP2 27/28 on row A.]
    pub row: u8,
}

/// A block as listed in the **footswitch layout** (`3 → 8`): its identity plus which footswitch
/// toggles its bypass. Only blocks bound to a switch appear here. Produced by [`PresetStream::footswitch_layout`].
#[derive(Debug, Clone, PartialEq)]
pub struct PathBlock {
    /// Display name (key `3 → 8 → [i] → [0] → 11 → 5`), e.g. `"Harmonic Tremolo"`.
    /// Matches the `name` field in the `.models` files.
    pub model_name: String,
    /// Numeric model id (key `… → 11 → 6`); exact encoding TBD.
    pub model_id: Option<i64>,
    /// User-assigned block label (key `… → 14`), if any.
    pub user_label: Option<String>,
    /// Index into the block-slots array (`0 → 22`) this entry refers to (key `… → 11 → 8`).
    pub slot: Option<i64>,
    /// Node type (`… → 11 → 0`): 1 = DSP block, 2 = controller/footswitch node.
    pub node_kind: Option<i64>,
    /// Footswitch this block's bypass is bound to (= layout position + 1; FS1 = position 0).
    pub footswitch: i64,
}

/// Fetch a value from a MessagePack map by integer key.
/// MessagePack key carrying a preset's display name in the preset-list stream.
const PRESET_NAME_KEY: i64 = 109;

/// One row of a **preset-list** browse response.
///
/// It carries two numbers and they are **not** interchangeable — mistaking one for the other is
/// what made a device with reordered presets unusable (see
/// `fretwire_core::session::list_preset_entries_in`):
///
/// * `slot` is the row's **position in the stream**, and that is the preset's real slot: the
///   number the pedal displays, the one `goto_preset`/`save_preset` take, and the one op-23
///   reports back as [`PresetInfo::index`]. [solid — HX Edit's own listing of a Helix Floor with
///   three moved presets matches that unit's `.hxb` backup position-for-position, and all 38
///   op-23 identities across the field logs agree with it]
/// * `key` is the row's MessagePack map key. It is the preset's index *before* it was last
///   reordered, and no command on this protocol accepts it as an address. [solid] Most likely the
///   physical storage slot, with the displayed order a permutation laid over it — reordering then
///   costs an order-table write instead of relocating a ~7 KB blob [hypothesis]. **Diagnostics
///   only:** `key != slot` is how we know the user has moved presets on this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetListEntry {
    /// The preset's slot, from this row's position in the stream. What `goto_preset` takes.
    pub slot: u16,
    /// The row's map key — the pre-reorder index. Never an address; see the type docs.
    pub key: u16,
    /// Display name, NUL-trimmed.
    pub name: String,
}

impl PresetListEntry {
    /// Whether this row's stored key disagrees with the slot the device actually lists it in — the
    /// signature of a preset that has been moved on the pedal. Keys are numbered globally, so
    /// `base` is the bank's offset (`bank * setlist_size`); pass `0` for a flat-list device.
    ///
    /// Diagnostic only. Nothing may address a preset by its key. See the type docs.
    pub fn key_disagrees(&self, base: i64) -> bool {
        self.key as i64 - base != self.slot as i64
    }
}

/// Parse a reassembled **preset-list** stream into one [`PresetListEntry`] per slot, in slot
/// order. The stream is an envelope `{.., 104: Array[ {key: {109: name, …}} ]}`, one entry per
/// preset slot; the **array position is the slot** and the map key is not (see
/// [`PresetListEntry`]). Verified against `startup.pcapng` (HX Stomp, 126 presets) and against
/// HX Edit's own listing of a reordered Helix Floor.
pub fn parse_preset_list(reassembled: &[u8]) -> crate::Result<Vec<PresetListEntry>> {
    let root = locate_root_where(reassembled, 32, |v| {
        matches!(map_get(v, ENVELOPE_PRESET_KEY), Some(Value::Array(_)))
    })
    .or_else(|| locate_root(reassembled, 32))
    .ok_or_else(|| crate::Error::Stream("no MessagePack envelope root".into()))?;
    let list = match map_get(&root.value, ENVELOPE_PRESET_KEY) {
        Some(Value::Array(a)) => a,
        // Before blaming the decoder, check whether the device simply said no. Asking a Stomp for
        // setlist 1 (it has one) returns a complete, well-formed 20-byte stream carrying
        // `{102: txn, 103: 255, 104: {111: -3}}` — the same refusal shape an edit gets. Reporting
        // that as "key 104 is not an array" sent us hunting a parser bug that wasn't there, and it
        // is the launch-time error the field has been reporting. [solid — 2026-08-02, HX Stomp]
        _ => {
            if let Some((_, code)) = parse_edit_rejection(reassembled) {
                return Err(crate::Error::Stream(format!(
                    "the device refused to list this setlist (code {code}) — it may not exist"
                )));
            }
            return Err(crate::Error::Stream(format!(
                "envelope key {ENVELOPE_PRESET_KEY} is not an array"
            )));
        }
    };
    let mut out = Vec::with_capacity(list.len());
    // The slot comes from `enumerate` over the *stream*, not from `out.len()`: a malformed row
    // that hits one of the `continue`s below then leaves a hole in the numbering instead of
    // shifting every slot after it down by one — one missing row rather than a whole listing that
    // addresses the wrong presets.
    for (pos, entry) in list.iter().enumerate() {
        // Each entry is a 1-key map: { <key>: { 109: name, … } }.
        let (map_key, inner) = match entry {
            Value::Map(m) => match m.first() {
                Some((k, v)) => (k, v),
                None => continue,
            },
            _ => continue,
        };
        let name = map_get(inner, PRESET_NAME_KEY)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim_end_matches('\0')
            .to_string();
        out.push(PresetListEntry {
            slot: pos as u16,
            key: map_key.as_u64().unwrap_or(0) as u16,
            name,
        });
    }
    Ok(out)
}

/// Identity of the preset currently loaded in the edit buffer, from an **op-23 read-info** reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetInfo {
    /// Setlist/bank index (key 107). Normally 0 on the Stomp.
    pub bank: i64,
    /// Preset index within the setlist (key 108) — matches `list_presets`/`goto_preset`.
    pub index: i64,
    /// Preset name (key 109), NUL-trimmed.
    pub name: String,
    /// **The live active snapshot** (key 92), 0-based — what the pedal is showing *right now*.
    ///
    /// This is the authority, and it is not the same thing as the preset blob's own
    /// `10 → 8` ([`PresetStream::snapshots`]), which is the snapshot that was **stored** with the
    /// preset. The two genuinely disagree: on an HX Stomp parked on SNAPSHOT 3, the blob reported 0.
    /// [solid — key 92 is the snapshot index in status pushes too (`{105:42, 106:{92:n}}`), and in
    /// the `Dual Amp` read-info capture it reads 0, matching that preset's live block scene while
    /// its stored index says 1]
    pub snapshot: Option<i64>,
}

/// Parse an **op-23 read-info** reply body into the current preset's identity. The reply envelope
/// is `{102:txn, 103:_, 104:{107:bank, 108:index, 109:name, …}}` — note key 104 here holds a *map*
/// (the identity), not the preset blob it carries in a full read. Returns `None` if the envelope or
/// the index is missing.
pub fn parse_preset_info(reply: &[u8]) -> Option<PresetInfo> {
    let root = locate_root_where(reply, 32, |v| {
        map_get(v, ENVELOPE_PRESET_KEY).is_some_and(|p| map_get(p, 108).is_some())
    })?;
    let payload = map_get(&root.value, ENVELOPE_PRESET_KEY)?;
    let index = map_get(payload, 108).and_then(Value::as_i64)?;
    let bank = map_get(payload, 107).and_then(Value::as_i64).unwrap_or(0);
    let name = map_get(payload, PRESET_NAME_KEY)
        .and_then(value_bytes)
        .map(|b| {
            String::from_utf8_lossy(b)
                .trim_end_matches('\0')
                .to_string()
        })
        .unwrap_or_default();
    // Key 92 = the live active snapshot. Same key the snapshot status-push uses, so the device
    // reports its current scene on every read; we simply used to throw it away.
    let snapshot = map_get(payload, 92).and_then(Value::as_i64);
    Some(PresetInfo {
        bank,
        index,
        name,
        snapshot,
    })
}

/// Envelope key `103` — the reply's **kind**, which we spent a long time treating as a don't-care:
/// `0` = key 104 carries a payload (identity, stream chunk, the echo of an applied edit), `1` = a
/// bare ack with 104 nil, `255` = the device **threw the command out** and 104 holds `{111: code}`.
const ENVELOPE_KIND_KEY: i64 = 103;
/// Value of [`ENVELOPE_KIND_KEY`] meaning the device refused the command.
const KIND_REJECTED: i64 = 255;
/// Key holding the refusal's numeric reason, inside key 104 of a rejection reply.
const REJECT_CODE_KEY: i64 = 111;

/// The device's refusal code if `reply` is a rejection, `None` if it is an ordinary ack or payload.
///
/// The device answers a command it won't apply with `{102: txn, 103: 255, 104: {111: code}}` and
/// otherwise carries on as if nothing happened — no error frame, no state change, and the very next
/// read returns the unmodified preset. Nothing in the reply says which command it is about beyond
/// the echoed transaction, so the caller must match `txn` itself.
///
/// [solid — 2026-07-30 Floor log: two `add_block` commands answered `{102:44/60, 103:255,
/// 104:{111:-21}}`, with the preset stream byte-identical before and after each]
pub fn parse_edit_rejection(reply: &[u8]) -> Option<(u16, i64)> {
    let root = locate_root_where(reply, 32, |v| map_get(v, ENVELOPE_KIND_KEY).is_some())?;
    if map_get(&root.value, ENVELOPE_KIND_KEY).and_then(Value::as_i64)? != KIND_REJECTED {
        return None;
    }
    let txn = map_get(&root.value, 102)
        .and_then(Value::as_u64)
        .unwrap_or(0) as u16;
    let code = map_get(&root.value, ENVELOPE_PRESET_KEY)
        .and_then(|p| map_get(p, REJECT_CODE_KEY))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Some((txn, code))
}

/// A state-change the device pushes **unsolicited** on the status channel when something changes on
/// the pedal itself (footswitch, panel knob, snapshot/preset switch). The wire shape is
/// `{105: type, 106: payload}`; `parse_status_push` distills it to these typed events so the editor
/// can live-follow the hardware.
#[derive(Debug, Clone, PartialEq)]
pub enum StatusPush {
    /// A block's bypass changed (`type 49`): slot + new enabled state (`enabled = !bypassed`).
    Bypass { slot: i64, enabled: bool },
    /// The active snapshot changed (`type 42/46`): 0-based index.
    Snapshot(i64),
    /// The active preset changed (`type 4/8`): 0-based index within the setlist.
    Preset(i64),
    /// A parameter was changed **on the pedal itself** (`type 30`) — a panel knob, or anything else
    /// that moves a value without going through us.
    Param {
        slot: i64,
        param: i64,
        value: ParamValue,
        /// `true` when `param` indexes the block's **extra** values rather than its model's param
        /// list — the two are separate spaces that both start at 0, so a consumer that ignores
        /// this applies `Trails` (extra 0) to whatever the model's param 0 happens to be.
        extra: bool,
    },
    /// The device's idle mirror (`type 22` with a nil payload) — "nothing changed", sent
    /// continuously. Distinct from [`StatusPush::Other`] so logging the undecoded pushes doesn't
    /// mean logging this several times a second.
    Idle,
    /// A recognized push `type` we don't decode further (kept so callers can log/ignore).
    Other(i64),
}

/// Parse one status-channel frame body into a [`StatusPush`], if it's a `{105,106}` state mirror.
/// `None` for meters/keepalives/other frames. The body includes the leading TLV-ish header, which
/// `locate_root` scans past. Most changes nest under an inner key `106`; snapshot is flat (`92`).
pub fn parse_status_push(frame_body: &[u8]) -> Option<StatusPush> {
    let root = locate_root_where(frame_body, 16, |v| map_get(v, 105).is_some())?;
    let typ = map_get(&root.value, 105).and_then(Value::as_i64)?;
    let payload = map_get(&root.value, 106)?;

    // Snapshot pushes carry {92: index} directly in the payload.
    if let Some(idx) = map_get(payload, 92).and_then(Value::as_i64) {
        return Some(StatusPush::Snapshot(idx));
    }
    // The idle mirror: type 22 whose inner payload is nil. The device sends it continuously while
    // nothing is happening — 100 identical copies in 30 seconds of an untouched pedal — so it is
    // not "a push we haven't decoded", it is a push that says nothing, and callers that log the
    // undecoded ones would otherwise drown a session's log in it. Scoped to type 22 on purpose: a
    // nil payload under a type we *haven't* seen is a discovery, and must stay visible as `Other`.
    // [solid — 2026-08-02, HX Stomp; the carrying form of type 22 has a map here instead]
    if typ == 22 && matches!(map_get(payload, 106), Some(Value::Nil)) {
        return Some(StatusPush::Idle);
    }
    // Everything else nests the actual change under an inner key 106.
    let inner = map_get(payload, 106).unwrap_or(payload);
    if let (Some(slot), Some(enabled)) = (
        map_get(inner, 98).and_then(Value::as_i64),
        map_get(inner, 59).and_then(Value::as_bool),
    ) {
        return Some(StatusPush::Bypass { slot, enabled });
    }
    if let Some(index) = map_get(inner, 108).and_then(Value::as_i64) {
        return Some(StatusPush::Preset(index));
    }
    // A panel parameter change mirrors back the *same* `{98: slot, 29: by_index, 26: 0, 28: index,
    // 119: value}` map the op-30 `set_value` edit sends, under the same push type number (30).
    // Turning the Drive knob on a US Princess emits a run of these with slot 5, index 0 and a
    // descending f32 — which is how it was identified. [solid — 2026-08-02, HX Stomp]
    //
    // **Key 29 is load-bearing and was ignored until 2026-08-21.** It selects which index space key
    // 28 is in, exactly as it does on the way out: `true` = the model's param list, `false` = the
    // block's extra values (`Trails` and friends, see `EditorParam::extra_index`). Both start at 0,
    // so dropping it made `{29: false, 28: 0}` — trails — arrive as the model's param 0, and
    // toggling Trails on the pedal moved the *Time* slider in the editor. Absent, assume the
    // ordinary space, which is what every pre-2026-08-21 caller assumed for all of them.
    // [solid — `captures/dynamic_ambience_trails_on_off.pcapng` vs `dynamic_ambience_mix_modify`]
    if let (Some(slot), Some(param), Some(value)) = (
        map_get(inner, 98).and_then(Value::as_i64),
        map_get(inner, 28).and_then(Value::as_i64),
        map_get(inner, 119).and_then(ParamValue::from_value),
    ) {
        let extra = map_get(inner, 29).and_then(Value::as_bool) == Some(false);
        return Some(StatusPush::Param {
            slot,
            param,
            value,
            extra,
        });
    }
    Some(StatusPush::Other(typ))
}

pub fn map_get(v: &Value, key: i64) -> Option<&Value> {
    match v {
        Value::Map(m) => m
            .iter()
            .find(|(k, _)| k.as_i64() == Some(key))
            .map(|(_, val)| val),
        _ => None,
    }
}

/// Mutable [`map_get`].
fn map_get_mut(v: &mut Value, key: i64) -> Option<&mut Value> {
    match v {
        Value::Map(m) => m
            .iter_mut()
            .find(|(k, _)| k.as_i64() == Some(key))
            .map(|(_, val)| val),
        _ => None,
    }
}

/// Set integer-keyed `key` of a map `Value` to `val`, inserting it if absent. No-op on non-maps.
fn set_map_key(v: &mut Value, key: i64, val: Value) {
    if let Value::Map(m) = v {
        match m.iter_mut().find(|(k, _)| k.as_i64() == Some(key)) {
            Some(e) => e.1 = val,
            None => m.push((Value::from(key), val)),
        }
    }
}

/// View a string/binary value as bytes.
pub fn value_bytes(v: &Value) -> Option<&[u8]> {
    match v {
        Value::String(s) => Some(s.as_bytes()),
        Value::Binary(b) => Some(b),
        _ => None,
    }
}

/// Result of locating the MessagePack root within a reassembled preset stream.
#[derive(Debug)]
pub struct Root {
    /// Byte offset within the stream where the MessagePack value begins.
    pub offset: usize,
    /// Number of bytes the value consumed.
    pub consumed: usize,
    /// The decoded value.
    pub value: Value,
}

/// Scan the first `max_scan` bytes for the MessagePack root: the container value that decodes
/// cleanly and consumes the most input. Returns `None` if nothing container-like parses.
///
/// Prefer [`locate_root_where`] when the caller knows a key the envelope must carry — "longest
/// match" on its own is not sufficient to identify the root.
pub fn locate_root(stream: &[u8], max_scan: usize) -> Option<Root> {
    locate_root_where(stream, max_scan, |_| true)
}

/// [`locate_root`], restricted to candidates `accept` recognizes as the envelope.
///
/// The scan needs this, because longest-match is wrong for exactly the streams the device sends.
/// The four bytes at offset 4 are the declared length, little-endian, so its **low byte sits
/// directly in front of the real root** — and when that byte happens to be a MessagePack container
/// marker whose element count is satisfied by the three remaining length bytes plus one more
/// value, the decoder swallows the whole envelope as that container's last element. It ends where
/// the real root ends and starts four bytes earlier, so it consumes *more* and wins the scan; its
/// keys are then `{26: 0, 0: <the real envelope>}`, which carry none of the keys a caller wants.
///
/// Two lengths in every 256 do this: low byte `0x82` (fixmap, 2 pairs) and `0x94` (fixarray, 4
/// elements). Everything else either isn't a container marker or needs more elements than the
/// buffer has left, and fails to decode. It is not rare in practice — a 6794-byte preset stream
/// declares 6786 = `0x1A82`, and every read of that preset failed identically.
///
/// [solid — 2026-08-01, `fretwire35.log`: three consecutive reads of the same "New Preset" after
/// an add-block reassembled 6794/6794 bytes and all three failed with "envelope key 104 missing or
/// not bytes"; the same tester saw the preset-list spelling of it ("not an array") at launch]
pub fn locate_root_where(
    stream: &[u8],
    max_scan: usize,
    accept: impl Fn(&Value) -> bool,
) -> Option<Root> {
    let mut best: Option<Root> = None;
    let scan = max_scan.min(stream.len());
    for offset in 0..scan {
        let mut cur = &stream[offset..];
        let start = cur.len();
        if let Ok(value) = rmpv::decode::read_value(&mut cur) {
            let consumed = start - cur.len();
            // We want the real payload root: a map or array, not an incidental scalar.
            let container = matches!(value, Value::Array(_) | Value::Map(_));
            if container && best.as_ref().is_none_or(|b| consumed > b.consumed) && accept(&value) {
                best = Some(Root {
                    offset,
                    consumed,
                    value,
                });
            }
        }
    }
    best
}

/// Byte length of the fixed prefix that precedes the MessagePack envelope: `marker:u16`,
/// `type:u16`, `len:u32` (little-endian).
const STREAM_PREFIX: usize = 8;

/// The total reassembled length the preset-stream envelope declares, if it can be read from the
/// first chunk. The stream opens with `marker:u16, type:u16, len:u32(LE)`; the whole stream is
/// `len + STREAM_PREFIX` bytes (the device may append a trailing pad byte, so treat this as a
/// **minimum** target, not an exact size — `preset1_stream` carries one extra byte).
///
/// Returns `None` when `chunk0` is too short to hold the prefix or the declared size is
/// implausible, so the reassembler falls back to its short-chunk terminator heuristic. The upper
/// bound rejects a garbage length (e.g. from a frame that isn't really chunk #0) that would
/// otherwise make the reader request chunks forever.
pub fn declared_stream_len(chunk0: &[u8]) -> Option<usize> {
    /// Presets are single-digit KB; 1 MiB is a generous ceiling that still rejects noise.
    const MAX_STREAM: usize = 1 << 20;
    if chunk0.len() < STREAM_PREFIX {
        return None;
    }
    let len = u32::from_le_bytes(chunk0[4..STREAM_PREFIX].try_into().ok()?) as usize;
    let total = len.checked_add(STREAM_PREFIX)?;
    (total > STREAM_PREFIX && total <= MAX_STREAM).then_some(total)
}

/// Read a flat sequence of concatenated MessagePack values (the device encodes the preset
/// body this way rather than as one container). Stops at end-of-input, the first decode error,
/// or after `max` values. Returns the values plus the number of bytes consumed.
pub fn read_sequence(data: &[u8], max: usize) -> (Vec<Value>, usize) {
    let mut cur = data;
    let mut out = Vec::new();
    while !cur.is_empty() && out.len() < max {
        match rmpv::decode::read_value(&mut cur) {
            Ok(v) => out.push(v),
            Err(_) => break,
        }
    }
    (out, data.len() - cur.len())
}

/// A recursive human-readable summary of a MessagePack value's shape, descending `depth`
/// levels into containers (for exploration/logging).
pub fn summarize(v: &Value, depth: usize) -> String {
    fn go(v: &Value, depth: usize, indent: usize) -> String {
        let pad = "  ".repeat(indent + 1);
        match v {
            Value::Map(m) if depth > 0 => {
                let mut s = format!("Map({} entries)", m.len());
                for (k, val) in m.iter().take(24) {
                    s.push_str(&format!(
                        "\n{pad}{} => {}",
                        key_str(k),
                        go(val, depth - 1, indent + 1)
                    ));
                }
                s
            }
            Value::Array(a) if depth > 0 => {
                let mut s = format!("Array({} items)", a.len());
                for (i, val) in a.iter().take(24).enumerate() {
                    s.push_str(&format!(
                        "\n{pad}[{i}] => {}",
                        go(val, depth - 1, indent + 1)
                    ));
                }
                s
            }
            other => short(other),
        }
    }
    go(v, depth, 0)
}

fn key_str(v: &Value) -> String {
    match v {
        Value::String(s) => format!("{:?}", s.as_str().unwrap_or("<non-utf8>")),
        other => short(other),
    }
}

fn short(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::F32(f) => format!("f32({f})"),
        Value::F64(f) => format!("f64({f})"),
        Value::String(s) => format!("str({:?})", s.as_str().unwrap_or("<non-utf8>")),
        Value::Binary(b) => format!("bin[{}]", b.len()),
        Value::Array(a) => format!("Array[{}]", a.len()),
        Value::Map(m) => format!("Map{{{}}}", m.len()),
        Value::Ext(t, d) => format!("ext({t},[{}])", d.len()),
    }
}

#[cfg(test)]
mod list_tests {
    use super::*;
    use rmpv::Value;

    fn enc(v: &Value) -> Vec<u8> {
        let mut b = Vec::new();
        rmpv::encode::write_value(&mut b, v).unwrap();
        b
    }

    #[test]
    fn parses_status_pushes() {
        use super::{StatusPush, parse_status_push};
        // Real dev.STATUS bodies (8-byte header + msgpack) from the panel-change captures.
        // snapshot -> 1: {105:42, 106:{92:1}}
        assert_eq!(
            parse_status_push(&hex("000004000700000082692a6a815c01")),
            Some(StatusPush::Snapshot(1))
        );
        // footswitch bypass: {105:49, 106:{82:0,68:5,121:17, 106:{98:2, 59:false}}}
        assert_eq!(
            parse_status_push(&hex("00000400110000008269316a845200440579116a8262023bc2")),
            Some(StatusPush::Bypass {
                slot: 2,
                enabled: false
            })
        );
        // bypass on: ...{98:2, 59:true}
        assert_eq!(
            parse_status_push(&hex("00000400110000008269316a845200440579116a8262023bc3")),
            Some(StatusPush::Bypass {
                slot: 2,
                enabled: true
            })
        );
    }

    #[test]
    fn parses_current_preset_info() {
        // The real op-23 read-info reply body from startup.pcapng (8-byte TLV header + msgpack):
        // {102:0x3ea, 103:0, 104:{107:0, 108:20, 109:"Dual Amp\0", 117:true, 83:[8850,0], 92:0}}.
        let body = hex(
            "00000600260000008366cd03ea670068866bcd00006ccd00146da9447561\
             6c20416d700075c35392cd2292005c00",
        );
        let info = parse_preset_info(&body).expect("parse read-info reply");
        assert_eq!(info.bank, 0);
        assert_eq!(info.index, 20);
        assert_eq!(info.name, "Dual Amp");
    }

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// A stream whose declared length ends in a MessagePack container marker used to lose the
    /// A setlist the device doesn't have. The reply is not corrupt — it is a complete 20-byte
    /// stream carrying the ordinary refusal envelope, `{102: txn, 103: 255, 104: {111: -3}}`.
    /// Captured from an HX Stomp (one setlist) asked for bank 1, 2026-08-02.
    #[test]
    fn a_refused_listing_reports_the_device_code_not_a_decode_error() {
        const REFUSAL: &[u8] = &[
            0x00, 0x00, 0x06, 0x00, 0x0c, 0x00, 0x00, 0x00, // marker/type/len prefix
            0x83, 0x66, 0xcd, 0x00, 0x03, 0x67, 0xcc, 0xff, 0x68, 0x81, 0x6f, 0xfd,
        ];
        assert_eq!(parse_edit_rejection(REFUSAL), Some((3, -3)));
        let err = parse_preset_list(REFUSAL).unwrap_err().to_string();
        assert!(err.contains("refused"), "got: {err}");
        assert!(
            err.contains("-3"),
            "the device's own code has to survive: {err}"
        );
        assert!(
            !err.contains("is not an array"),
            "stop blaming the decoder: {err}"
        );
    }

    /// scan to a false root starting inside the length field itself. A Floor hit this on a
    /// 6794-byte read (declared 6786 = `0x1A82`) and every retry failed the same way, so the whole
    /// preset was unreadable — not flaky, just that size.
    #[test]
    fn a_length_that_looks_like_a_container_marker_does_not_hijack_the_root() {
        // Every byte value, so this keeps covering 0x82/0x94 if the scan is ever rewritten.
        for low in 0..=u8::MAX {
            // Payload sized so the encoded envelope's length lands on `low`. The three envelope
            // keys and the bin header are fixed overhead, so walking the payload walks the length.
            let mut stream = None;
            for pad in 0..400usize {
                let env = Value::Map(vec![
                    (Value::from(102), Value::from(1)),
                    (Value::from(103), Value::from(0)),
                    (Value::from(104), Value::Binary(vec![0xAB; pad])),
                ]);
                let body = enc(&env);
                if body.len() > u8::MAX as usize && (body.len() & 0xff) as u8 == low {
                    let mut s = vec![0u8, 0, 0x0f, 0x00];
                    s.extend_from_slice(&(body.len() as u32).to_le_bytes());
                    s.extend_from_slice(&body);
                    stream = Some((s, pad));
                    break;
                }
            }
            let Some((stream, pad)) = stream else {
                continue;
            };
            let root = locate_root_where(&stream, 32, |v| map_get(v, 104).is_some())
                .unwrap_or_else(|| panic!("no root for low byte {low:#04x}"));
            assert_eq!(root.offset, 8, "false root for low byte {low:#04x}");
            assert_eq!(
                map_get(&root.value, 104)
                    .and_then(value_bytes)
                    .map(<[u8]>::len),
                Some(pad),
                "wrong payload for low byte {low:#04x}"
            );
        }
    }

    /// Bytes captured off an HX Stomp (2026-08-02) while the Drive knob on a US Princess in slot 5
    /// was swept down — one frame from a run of fifteen, each carrying the next value. The payload
    /// is the same `{98: slot, 28: index, 119: value}` triple the op-30 edit *sends*, under the
    /// same op number, which is what identified it.
    #[test]
    fn a_panel_knob_pushes_the_same_shape_the_edit_sends() {
        let frame = [
            0, 0, 4, 0, 27, 0, 0, 0, 130, 105, 30, 106, 132, 82, 0, 68, 6, 121, 20, 106, 133, 98,
            5, 29, 195, 26, 0, 28, 0, 119, 202, 62, 204, 204, 204,
        ];
        match parse_status_push(&frame) {
            Some(StatusPush::Param {
                slot,
                param,
                value,
                extra,
            }) => {
                assert_eq!((slot, param), (5, 0));
                assert!(!extra, "key 29 is true here — the model's param space");
                match value {
                    ParamValue::Float(f) => assert!((f - 0.4).abs() < 1e-6, "got {f}"),
                    other => panic!("expected a float, got {other:?}"),
                }
            }
            other => panic!("expected a Param push, got {other:?}"),
        }
    }

    /// The same push shape, but for an **extra** value: `Trails` on a Dynamic Ambience in slot 7,
    /// toggled with the pedal's own knob. Bytes from
    /// `captures/dynamic_ambience_trails_on_off.pcapng`.
    ///
    /// Key 29 is the only thing separating this from the test above: `false` (`0xc2`) here against
    /// `true` (`0xc3`) there, and both carry `28: 0`. Ignoring it is what made toggling Trails on
    /// the pedal drive the *Time* slider in the editor — the model's param 0 — which is exactly how
    /// it was reported. [issue #5]
    #[test]
    fn an_extra_value_push_is_not_the_models_param_zero() {
        let on = [
            0, 0, 4, 0, 23, 0, 0, 0, 130, 105, 30, 106, 132, 82, 0, 68, 6, 121, 20, 106, 133, 98,
            7, 29, 194, 26, 0, 28, 0, 119, 195,
        ];
        assert_eq!(
            parse_status_push(&on),
            Some(StatusPush::Param {
                slot: 7,
                param: 0,
                value: ParamValue::Bool(true),
                extra: true,
            })
        );
        // The off frame differs in exactly one byte — the value — so a decoder that got the index
        // space wrong would still look right here.
        let mut off = on;
        off[30] = 194;
        assert_eq!(
            parse_status_push(&off),
            Some(StatusPush::Param {
                slot: 7,
                param: 0,
                value: ParamValue::Bool(false),
                extra: true,
            })
        );
    }

    /// The type-22 frame the Stomp emits continuously while idle. It is a `{105,106}` mirror like
    /// the ones we decode, so it must stay classified as `Other` rather than be mistaken for a
    /// change — 154 identical copies of it arrived in a two-minute capture, and 100 in 30 seconds
    /// of an untouched pedal. It gets its own variant so that logging the *undecoded* pushes stays
    /// useful: as `Other(22)` it was 100% of a session's push log.
    #[test]
    fn the_idle_status_mirror_is_idle_not_undecoded() {
        let frame = [
            0, 0, 4, 0, 13, 0, 0, 0, 130, 105, 22, 106, 132, 82, 0, 68, 10, 121, 27, 106, 192,
        ];
        assert_eq!(parse_status_push(&frame), Some(StatusPush::Idle));
    }

    /// The other type 22: same outer key, a real payload under the inner 106, and different
    /// discriminators (`68:9, 121:25` where the idle mirror has `68:10, 121:27`). This one carries
    /// something, so it must stay visible as `Other` rather than be filed away as idle.
    /// Captured 2026-08-02 while the tester worked the pedal's footswitch modes.
    #[test]
    fn a_type_22_that_carries_a_payload_is_not_idle() {
        let frame = [
            0, 0, 4, 0, 19, 0, 0, 0, 130, 105, 22, 106, 132, 82, 0, 68, 9, 121, 25, 106, 130, 118,
            205, 0, 21, 119, 2,
        ];
        assert_eq!(parse_status_push(&frame), Some(StatusPush::Other(22)));
    }

    /// The array position is the slot; the row's map key is not. Keyed deliberately out of order —
    /// the shape a device with a moved preset really sends — so an implementation that trusts the
    /// key fails here instead of in someone's setlist.
    #[test]
    fn preset_names_are_numbered_by_position_not_by_key() {
        let entry = |i: i64, name: &str| {
            Value::Map(vec![(
                Value::from(i),
                Value::Map(vec![(Value::from(109), Value::from(format!("{name}\0")))]),
            )])
        };
        let env = Value::Map(vec![
            (Value::from(102), Value::from(1)),
            (Value::from(103), Value::from(0)),
            (
                Value::from(104),
                Value::Array(vec![entry(2, "Alpha"), entry(0, "Beta"), entry(1, "Gamma")]),
            ),
        ]);
        let mut stream = vec![0u8; 8]; // fake TLV header for locate_root to scan past
        stream.extend(enc(&env));
        let row = |slot: u16, key: u16, name: &str| PresetListEntry {
            slot,
            key,
            name: name.to_string(),
        };
        assert_eq!(
            parse_preset_list(&stream).unwrap(),
            vec![row(0, 2, "Alpha"), row(1, 0, "Beta"), row(2, 1, "Gamma"),]
        );
    }

    #[test]
    fn parses_assignments() {
        // Mirrors preset 20: footswitch 7 -> slot 15, param 0.
        let def = Value::Map(vec![
            (Value::from(0), Value::from(7)),
            (Value::from(1), Value::from(4)),
            (Value::from(4), Value::from(0)),
            (Value::from(5), Value::from(15)),
            (
                Value::from(6),
                Value::Map(vec![(Value::from(28), Value::from(0))]),
            ),
            (Value::from(7), Value::from(0)),
        ]);
        let entry = Value::Array(vec![Value::Map(vec![
            (Value::from(0), Value::from(0)),
            (Value::from(1), def),
        ])]);
        let mut table = vec![Value::Nil; 10];
        table[7] = entry;
        let preset = Value::Map(vec![(Value::from(4), Value::Array(table))]);
        let ps = PresetStream {
            magic: "l6-helix".to_string(),
            header: vec![],
            preset,
            header_slots: Vec::new(),
        };
        let a = ps.assignments();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].controller, 7);
        assert_eq!(a[0].ctype, Some(4));
        assert_eq!(a[0].target_slot, Some(15));
        assert_eq!(a[0].param_index, Some(0));
    }

    #[test]
    fn parses_snapshots() {
        let snap =
            |name: &str| Value::Map(vec![(Value::from(4), Value::from(format!("{name}\0")))]);
        let key10 = Value::Map(vec![
            (Value::from(8), Value::from(1)),
            (
                Value::from(10),
                Value::Array(vec![snap("CLEAN"), snap("Just TS"), snap("LEAD")]),
            ),
        ]);
        let preset = Value::Map(vec![(Value::from(10), key10)]);
        let ps = PresetStream {
            magic: "l6-helix".to_string(),
            header: vec![],
            preset,
            header_slots: Vec::new(),
        };
        let (active, names) = ps.snapshots();
        assert_eq!(active, Some(1));
        assert_eq!(names, vec!["CLEAN", "Just TS", "LEAD"]);
    }

    #[test]
    fn footswitch_layout_binds_block_to_switch() {
        // Mirrors the live diff that settled what `3 → 8` is: binding a block to FS1 populated
        // layout position [0] with that block's node (`11 → 8 = 6`). So a block at slot 6 that is
        // present in the layout at position 0 must report footswitch = 1; a block absent from the
        // layout reports 0. Enumeration still comes from the slot array either way.
        let block = Value::Map(vec![
            (Value::from(19), Value::from(slot_kind::EFFECT)),
            (
                Value::from(20),
                Value::Map(vec![
                    (
                        Value::from(24),
                        Value::Map(vec![(Value::from(25), Value::from(80))]),
                    ),
                    (Value::from(10), Value::from(true)),
                    (
                        Value::from(11),
                        Value::Map(vec![(Value::from(4), Value::Array(vec![Value::F32(0.5)]))]),
                    ),
                ]),
            ),
        ]);
        let mut slots = vec![
            Value::Map(vec![
                (Value::from(19), Value::from(slot_kind::EMPTY)),
                (Value::from(20), Value::Nil),
            ]);
            20
        ];
        slots[6] = block;
        let slot_map = Value::Map(vec![(Value::from(22), Value::Array(slots))]);

        // Footswitch layout: position [0] (= FS1) holds the slot-6 block's node.
        let fs_node = Value::Array(vec![Value::Map(vec![
            (
                Value::from(11),
                Value::Map(vec![
                    (Value::from(0), Value::from(1)),
                    (Value::from(5), Value::from("Simple Delay\0")),
                    (Value::from(8), Value::from(6)),
                ]),
            ),
            (Value::from(14), Value::from("\0")),
        ])]);

        let bound = PresetStream {
            magic: "l6-helix".into(),
            header: vec![],
            preset: Value::Map(vec![
                (Value::from(0), slot_map.clone()),
                (
                    Value::from(3),
                    Value::Map(vec![(Value::from(8), Value::Array(vec![fs_node]))]),
                ),
            ]),
            header_slots: Vec::new(),
        };
        let lb = bound.loaded_blocks();
        assert_eq!(lb.len(), 1);
        assert_eq!(lb[0].slot, 6);
        assert_eq!(lb[0].model_index, Some(80));
        assert_eq!(lb[0].footswitch, 1, "bound at layout position 0 ⇒ FS1");

        // Same block, no footswitch layout → still enumerated, but not on a switch.
        let unbound = PresetStream {
            magic: "l6-helix".into(),
            header: vec![],
            preset: Value::Map(vec![(Value::from(0), slot_map)]),
            header_slots: Vec::new(),
        };
        assert_eq!(unbound.loaded_blocks()[0].footswitch, 0);
    }

    #[test]
    fn controller_node_does_not_reclassify_a_dsp_block() {
        // A controller assigned to one of a block's params puts a *controller* node (`11 → 0 == 2`)
        // in the footswitch layout pointing at that block's slot. That must NOT reclassify the real
        // kind-6 DSP block as a controller — the bug where a parallel-path amp with a controller
        // assigned vanished from `pull` (and from the DSP total).
        let block = Value::Map(vec![
            (Value::from(19), Value::from(slot_kind::EFFECT)),
            (
                Value::from(20),
                Value::Map(vec![
                    (
                        Value::from(24),
                        Value::Map(vec![(Value::from(25), Value::from(79))]),
                    ),
                    (Value::from(10), Value::from(true)),
                    (
                        Value::from(11),
                        Value::Map(vec![(Value::from(4), Value::Array(vec![Value::F32(0.5)]))]),
                    ),
                ]),
            ),
        ]);
        let mut slots = vec![
            Value::Map(vec![
                (Value::from(19), Value::from(slot_kind::EMPTY)),
                (Value::from(20), Value::Nil),
            ]);
            20
        ];
        slots[15] = block;
        let slot_map = Value::Map(vec![(Value::from(22), Value::Array(slots))]);

        // Footswitch layout position [0] = a CONTROLLER node (`11 → 0 == 2`) pointing at slot 15.
        let ctrl_node = Value::Array(vec![Value::Map(vec![
            (
                Value::from(11),
                Value::Map(vec![
                    (Value::from(0), Value::from(2)), // node type 2 = controller, not a DSP block
                    (Value::from(5), Value::from("Grammatico GSG\0")),
                    (Value::from(8), Value::from(15)),
                ]),
            ),
            (Value::from(14), Value::from("\0")),
        ])]);

        let ps = PresetStream {
            magic: "l6-helix".into(),
            header: vec![],
            preset: Value::Map(vec![
                (Value::from(0), slot_map),
                (
                    Value::from(3),
                    Value::Map(vec![(Value::from(8), Value::Array(vec![ctrl_node]))]),
                ),
            ]),
            header_slots: Vec::new(),
        };
        let lb = ps.loaded_blocks();
        assert_eq!(lb.len(), 1, "the kind-6 block must still be enumerated");
        assert_eq!(lb[0].slot, 15);
        assert_eq!(lb[0].model_index, Some(79));
        assert_ne!(
            lb[0].node_kind,
            Some(2),
            "a controller pointing at a block must not reclassify it"
        );
    }
}
