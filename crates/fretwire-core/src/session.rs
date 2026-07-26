//! Live device session over [`fretwire_usb::Transport`].
//!
//! Composes the transport (`fretwire-usb`), the wire types (`fretwire-protocol`), and the editor model
//! ([`crate::editor`]) into a connect → read-preset → edit loop.
//!
//! **Maturity:** the transport and frame building are solid and tested offline; the exact *live
//! sequence* (which channels to open, the per-channel `arg` stream offset, whether the device
//! ACKs each edit) is reconstructed from captures and **needs first-contact tuning on Linux** —
//! those spots are marked `LIVE:`. The design keeps raw frame access ([`Session::request`]) so the
//! sequence can be probed interactively before the convenience methods are trusted.
//!
//! **Per-DSP routing.** The grid/routing methods here (`add_block_at`, `place_block`,
//! `insert_block`, `reorder_block`, `set_node_pos`, …) plan moves within **one** DSP's 20-slot
//! array. They work in **global wire-slot space** (`dsp * 20 + index`, see
//! [`fretwire_data::stream::DSP_SLOT_STRIDE`]): each derives the target DSP from its slot argument,
//! reads that DSP's blocks/grid (`dsp_blocks(dsp)` / `dsp_grid(dsp)` via `Block::wire_slot()`), and
//! the `plan_*` helpers below operate on those wire slots directly. A move that spans two DSPs is
//! rejected — the UI can't express one, and the device's surgical ops address a single grid. Reading
//! and per-block edits were already DSP-agnostic; this makes the planning layer so too.

use crate::editor::{Catalog, EditorPreset};
use fretwire_protocol::{EditValue, Frame, Tlv, channel, cmd, edit, op};
use fretwire_usb::Transport;
use std::collections::HashMap;

/// A connected device session.
pub struct Session {
    transport: Transport,
    catalog: Catalog,
    /// Next sequence counter per host channel id (frames on a channel increment it).
    seq: HashMap<u16, u8>,
    /// Running stream byte-offset (frame `arg`) per host channel id.
    arg: HashMap<u16, u32>,
    /// Running edit transaction counter (envelope key 102).
    txn: u16,
    /// Set once the channels have been closed, so [`Drop`] doesn't double-close.
    closed: bool,
    /// The last reassembled preset read-stream — every read refreshes it, so edit-history snapshots
    /// cost no extra USB round-trip.
    last_raw: Option<Vec<u8>>,
    /// Edit history: a timeline of labeled preset-blob states, oldest first. Entry 0 is the loaded
    /// state; each later entry is the state *after* the edit its label names.
    history: Vec<HistoryEntry>,
    /// Index into `history` of the state currently in the edit buffer. Undo/redo move it; jumping
    /// to any entry writes that blob back (op 21). An edit truncates everything after the cursor.
    cursor: usize,
    /// Label of an edit in flight (set by [`Self::edit_begin`], consumed by [`Self::edit_commit`]).
    pending: Option<String>,
    /// History cursor whose state matches what's saved in flash — `Some(0)` on load (the buffer IS
    /// the flash copy), moved by [`Self::save_preset`], `None` when the saved state fell off the
    /// timeline (truncated redo branch / history cap). Drives [`Self::dirty`].
    saved_cursor: Option<usize>,
}

/// One state on the edit-history timeline: the op-21-writable preset blob plus the label of the
/// edit that produced it.
struct HistoryEntry {
    label: String,
    blob: Vec<u8>,
}

/// Edit-history length cap — blobs are ~3 KB, so this bounds history at ~150 KB.
const MAX_HISTORY: usize = 50;

impl Session {
    /// Open the device, claim the interface, and run the session handshake.
    ///
    /// A previous session can leave stale state on the device so the first handshake reply never
    /// comes; releasing the interface resets it. So we retry: on a handshake failure we drop the
    /// transport (releasing the interface) and re-open, which clears the stale state.
    pub fn connect() -> crate::Result<Session> {
        const ATTEMPTS: u32 = 3;
        let mut last_err = None;
        for attempt in 1..=ATTEMPTS {
            let transport = Transport::open()?;
            let catalog = Catalog::load()?;
            let mut s = Session {
                transport,
                catalog,
                seq: HashMap::new(),
                arg: HashMap::new(),
                txn: 0,
                closed: false,
                last_raw: None,
                history: Vec::new(),
                cursor: 0,
                pending: None,
                saved_cursor: Some(0),
            };
            // Clear any frames a previous session left on the wire so the handshake starts aligned.
            s.transport
                .drain_wire(std::time::Duration::from_millis(120), 64);
            match s.handshake() {
                Ok(()) => return Ok(s),
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        "handshake failed ({e}); releasing interface and retrying"
                    );
                    last_err = Some(e);
                    drop(s); // release the interface — resets the device's stale session state
                }
            }
        }
        Err(last_err.expect("loop runs at least once"))
    }

    /// The catalog (model table + device param orders) used to interpret presets.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The connected device — its DSP and snapshot counts, model code and support status. Note
    /// this is what the *USB PID* says; a preset's own `device_model` is the device's own claim,
    /// and [`Self::device_matches_preset`] compares the two.
    pub fn device(&self) -> &'static fretwire_protocol::Device {
        self.transport.device()
    }

    /// Whether the connected device's model code matches the one stamped into `preset`. `None`
    /// when either side is unknown (an untested device, or a preset with no model code) — that is
    /// not a mismatch, just an unanswerable question.
    pub fn device_matches_preset(&self, preset: &EditorPreset) -> Option<bool> {
        let expected = self.device().model_code?;
        let actual = preset.device_model.as_deref()?;
        Some(expected == actual)
    }

    fn next_seq(&mut self, src: u16) -> u8 {
        let e = self.seq.entry(src).or_insert(0);
        let v = *e;
        *e = e.wrapping_add(1);
        v
    }

    /// The current stream offset (`arg`) to stamp on the next frame for channel `src`.
    fn cur_arg(&mut self, src: u16) -> u32 {
        *self.arg.entry(src).or_insert(0)
    }

    /// Advance a channel's stream offset by `delta` bytes (post-chunk, by the payload received).
    fn advance_arg(&mut self, src: u16, delta: u32) {
        let e = self.arg.entry(src).or_insert(0);
        *e = e.wrapping_add(delta);
    }

    fn bump_txn(&mut self) -> u16 {
        self.txn = self.txn.wrapping_add(1);
        self.txn
    }

    /// Send one frame on `chan` and return its matched reply, maintaining the channel's running
    /// `arg` offset: the frame is stamped with the current offset, then the offset is advanced by
    /// the reply's body length (the rule observed across the connect capture).
    fn channel_request(
        &mut self,
        chan: (u16, u16),
        cmd: u8,
        body: Vec<u8>,
    ) -> crate::Result<Frame> {
        let (src, dst) = chan;
        let seq = self.next_seq(src);
        let arg = self.cur_arg(src);
        let reply = self
            .transport
            .request(&Frame::new(src, dst, seq, cmd, arg, body))?;
        self.advance_arg(src, reply.body.len() as u32);
        Ok(reply)
    }

    /// Same as [`Self::channel_request`] on the edit channel.
    fn edit_request(&mut self, cmd: u8, body: Vec<u8>) -> crate::Result<Frame> {
        self.channel_request(channel::EDIT, cmd, body)
    }

    /// An edit-channel request whose reply is correlated by the **transaction id** echoed in the
    /// reply body (msgpack key 102), not just "next non-keepalive frame on the channel". The device
    /// interleaves state-push/keepalive frames; without this a stray frame can be mistaken for the
    /// reply — e.g. as a preset stream's chunk #0, yielding a stream with no envelope (key 104).
    /// `txn` is the counter embedded in `body`. Maintains the channel's `arg` offset like
    /// [`Self::channel_request`].
    fn edit_request_txn(&mut self, cmd: u8, body: Vec<u8>, txn: u16) -> crate::Result<Frame> {
        self.channel_request_txn(channel::EDIT, cmd, body, txn)
    }

    /// Like [`Self::edit_request_txn`] but on an arbitrary channel — correlates the reply by the
    /// transaction id (msgpack key 102) echoed in the reply body, skipping interleaved
    /// keepalive/state-push frames. Used for the primary-channel rename.
    fn channel_request_txn(
        &mut self,
        chan: (u16, u16),
        cmd: u8,
        body: Vec<u8>,
        txn: u16,
    ) -> crate::Result<Frame> {
        let (src, dst) = chan;
        let seq = self.next_seq(src);
        let arg = self.cur_arg(src);
        let frame = Frame::new(src, dst, seq, cmd, arg, body);
        let reply =
            self.transport
                .request_matching(&frame, std::time::Duration::from_secs(3), |f| {
                    reply_txn(&f.body) == Some(txn)
                })?;
        self.advance_arg(src, reply.body.len() as u32);
        Ok(reply)
    }

    /// Run the device handshake reconstructed byte-exact from `startup.pcapng`: it brings up all
    /// three channels (primary/edit/status), and the identity replies carry the model string.
    /// Returns the device model code (e.g. `"P33"`) if seen in a reply.
    ///
    /// LIVE: the capture grouped vs interleaved channels and repeated some opens; if a reply stalls,
    /// compare the `RUST_LOG=trace` frames against `tools/dump-control.ps1 startup.pcapng`.
    /// `fretwire_protocol::session::primary_handshake()` (the HX Stomp XL sequence) is an alternative.
    pub fn handshake(&mut self) -> crate::Result<()> {
        let mut model: Option<String> = None;
        // The identity reply embeds the connected device's model code as ASCII (e.g. "P33"/"P33Main"
        // on the Stomp, "P21…" on the Floor). Key off the code we opened the device with rather than
        // a hard-coded "P33", or every non-Stomp connect logs a spurious "no model string seen".
        let want = self.device().model_code;
        for (i, f) in fretwire_protocol::session::device_handshake()
            .into_iter()
            .enumerate()
        {
            let reply = self.transport.request(&f)?;
            if model.is_none()
                && let Some(s) = ascii_run(&reply.body)
            {
                let hit = match want {
                    Some(code) => s.starts_with(code),
                    // Untested device with no known code: accept any "P##"-style identity.
                    None => {
                        s.starts_with('P')
                            && s.len() >= 3
                            && s[1..3].bytes().all(|b| b.is_ascii_digit())
                    }
                };
                if hit {
                    model = Some(s);
                }
            }
            tracing::debug!(
                packet = i + 1,
                src = f.src,
                cmd = reply.cmd,
                body = reply.body.len(),
                bytes = format_args!("{:02x?}", reply.body),
                "handshake reply"
            );
        }
        // After bring-up, continue each channel's seq counter past the handshake frames.
        self.seq.insert(channel::PRIMARY.0, 5);
        self.seq.insert(channel::EDIT.0, 4);
        self.seq.insert(channel::STATUS.0, 4);
        // Seed each channel's running `arg` offset to where the handshake left it — the channel's
        // last sent arg (its `device_handshake` chunk_arg), which the next frame continues from.
        // (The edit read-open and the primary browse-open both ride these.) LIVE: re-tune if the
        // trace shows the device rejecting a base.
        self.arg.insert(channel::PRIMARY.0, 0x1020);
        self.arg.insert(channel::EDIT.0, 0x1009);
        self.arg.insert(channel::STATUS.0, 0x1009);
        match model {
            Some(m) => tracing::info!("handshake OK — device reports {m:?}"),
            None => tracing::warn!("handshake completed but no model string seen — verify replies"),
        }
        Ok(())
    }

    /// Send one raw frame and read one reply — the escape hatch for probing the live sequence.
    pub fn request(&mut self, frame: &Frame) -> crate::Result<Frame> {
        Ok(self.transport.request(frame)?)
    }

    /// Service the open session: send an idle keepalive on each channel and consume the frames the
    /// device has queued. HX Edit sends this heartbeat continuously; without it the device stops
    /// responding on the edit channel after a few seconds idle (and its own keepalives pile up
    /// unread behind the next edit). Call on a timer (~4×/s) while a session is held open between
    /// edits. Fire-and-forget sends; a short drain clears the device's queued frames.
    pub fn keepalive(&mut self) -> crate::Result<()> {
        for (src, dst) in [channel::STATUS, channel::EDIT, channel::PRIMARY] {
            let seq = self.next_seq(src);
            let arg = self.cur_arg(src);
            self.transport
                .send_frame(&Frame::new(src, dst, seq, cmd::IDLE, arg, Vec::new()))?;
        }
        // Drain the device's queued keepalives/meters so they don't sit in front of the next edit's
        // reply. Short per-frame quiet window; bounded so a chatty device can't stall the tick.
        self.transport
            .drain_wire(std::time::Duration::from_millis(15), 64);
        Ok(())
    }

    /// Heartbeat **and** collect the device's unsolicited state-pushes (footswitch bypass, panel
    /// snapshot/preset changes) so the editor can live-follow the hardware. Same idle-on-each-channel
    /// beat as [`Self::keepalive`], but the drained status-channel `{105,106}` frames are parsed into
    /// [`StatusPush`] events instead of discarded. Call on the same timer the GUI uses for keepalive.
    pub fn poll_events(&mut self) -> crate::Result<Vec<fretwire_data::stream::StatusPush>> {
        for (src, dst) in [channel::STATUS, channel::EDIT, channel::PRIMARY] {
            let seq = self.next_seq(src);
            let arg = self.cur_arg(src);
            self.transport
                .send_frame(&Frame::new(src, dst, seq, cmd::IDLE, arg, Vec::new()))?;
        }
        let frames = self
            .transport
            .drain_collect(std::time::Duration::from_millis(15), 96);
        let pushes = frames
            .iter()
            .filter_map(|f| fretwire_data::stream::parse_status_push(&f.body))
            .collect();
        Ok(pushes)
    }

    /// Send an edit-command body on the edit channel and wait for the device's ACK, then issue the
    /// `cmd 0x08` follow-up HX Edit sends after each discrete edit (both via [`Self::edit_request`],
    /// so the channel's `arg` offset stays correct). The edit itself rides `cmd 0x04`.
    fn send_edit(&mut self, body: Vec<u8>) -> crate::Result<Frame> {
        // Clear any frames buffered since the last heartbeat so the edit's reply is the next one we
        // read (the device interleaves keepalives/meters on a held session).
        self.transport.drain();
        let tlv = Tlv::command(op::PARAM_SET, body);
        let ack = self.edit_request(cmd::OPEN, tlv.to_bytes())?;
        tracing::debug!(reply = ?ack.body, "edit ACK");
        // Flush HX Edit sends after each edit — fire-and-forget: the edit is already ACKed/applied,
        // and it gets no distinct reply of its own (only keepalives follow), so don't wait on one.
        let (src, dst) = channel::EDIT;
        let seq = self.next_seq(src);
        let arg = self.cur_arg(src);
        let _ = self
            .transport
            .send_frame(&Frame::new(src, dst, seq, cmd::CHUNK, arg, Vec::new()));
        Ok(ack)
    }

    /// Set a block's enabled state: `enabled = true` activates the block, `false` bypasses it.
    /// Slot = preset slot index.
    pub fn set_enabled(&mut self, slot: i64, enabled: bool) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::bypass(slot, enabled, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Navigate the device to `preset` in `bank` (op 20 SELECT). **Changes the active preset** —
    /// this is the destructive counterpart to `read_preset`. Rides the edit channel like an edit.
    pub fn goto_preset(&mut self, bank: i64, preset: i64) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::select_preset(bank, preset, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Save the current edit buffer to a preset slot (op 71). **Persistent write — overwrites
    /// `slot` in device flash.** `bank` is normally 0; `slot` is the flat preset index (as `goto`
    /// and `list_presets` use). `name` is stored NUL-terminated. Rides the edit channel like an edit.
    pub fn save_preset(&mut self, bank: i64, slot: i64, name: &str) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::save_preset(bank, slot, name, txn);
        self.send_edit(body)?;
        // The buffer's current state is now in flash — this history position is the clean one.
        self.saved_cursor = Some(self.cursor);
        Ok(())
    }

    /// Rename the preset in `slot` (bank normally 0) to `name`, **without committing the edit
    /// buffer** (name-only rename, op 6). Any live edits stay in the buffer, unsaved — only the
    /// stored name changes. Non-destructive to the signal chain. The device ACKs with `{103:1}`; the
    /// name is stored NUL-terminated.
    ///
    /// HX Edit sends this on the **primary** channel, but our reconstructed handshake doesn't leave
    /// primary command-ready (a PRI send times out with no reply); the **edit** channel serves the
    /// same command dispatch and is what we use — the same substitution `list_presets` makes for the
    /// browse ops. Correlated by transaction id so an interleaved keepalive isn't mistaken for the ACK.
    pub fn rename_preset(&mut self, bank: i64, slot: i64, name: &str) -> crate::Result<()> {
        let txn = self.bump_txn();
        let tlv =
            Tlv::command(op::SESSION_OPEN, edit::rename_preset(bank, slot, name, txn)).to_bytes();
        self.transport.drain();
        let reply = self.edit_request_txn(cmd::OPEN, tlv, txn)?;
        tracing::debug!(reply = ?reply.body, "rename ACK");
        Ok(())
    }

    /// Switch the active snapshot (op 88). `index` is the snapshot's position (0-based) — the order
    /// `read_preset` reports in `snapshot_names`. Changes device state; rides the edit channel.
    pub fn set_snapshot(&mut self, index: i64) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::switch_snapshot(index, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Swap the model of the block in `slot` to `model_index` (its `Helix.sym` index), with
    /// `paired_index` for a paired cab/IR (`-1` = none). The device resets the block's params to the
    /// new model's defaults (confirmed by on-device diff). Op 40; rides the edit channel.
    pub fn swap_model(
        &mut self,
        slot: i64,
        model_index: i64,
        paired_index: i64,
    ) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::swap_model(slot, model_index, paired_index, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Move the block in `src` slot to `dst` slot (op 43). The destination slot encodes the row, so a
    /// parallel-path slot index moves the block to row B. The caller should re-read afterward (HX Edit
    /// does), as positions shift. Rides the edit channel.
    pub fn move_block(&mut self, src: i64, dst: i64) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::move_block(src, dst, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Add a block at `slot` with model `model_index` (its `Helix.sym` index) and `paired_index`
    /// (`-1` = no paired cab/IR), enabled (op 39). The device fills the new block's params with the
    /// model's defaults; re-read to see them. Rides the edit channel.
    pub fn add_block(
        &mut self,
        slot: i64,
        model_index: i64,
        paired_index: i64,
    ) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::add_block(slot, model_index, paired_index, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// [`add_block`] with an occupancy guard: `slot` must be an **empty** grid slot (op 39 into an
    /// occupied slot would clobber the block there). The primitive behind the grid's
    /// click-an-empty-cell add flow.
    pub fn add_block_at(
        &mut self,
        slot: i64,
        model_index: i64,
        paired_index: i64,
    ) -> crate::Result<()> {
        use fretwire_data::stream::{PresetStream, slot_kind, split_wire_slot};
        let raw = self.read_preset_raw()?;
        let ps = PresetStream::parse(&raw)?;
        let (dsp, _) = split_wire_slot(slot);
        let empty = ps
            .dsp_blocks(dsp)
            .iter()
            .any(|b| b.wire_slot() == slot && b.kind == slot_kind::EMPTY);
        if !empty {
            return Err(fretwire_data::Error::Stream(format!(
                "slot {slot} is not an empty grid slot (refusing add — it would overwrite)"
            ))
            .into());
        }
        self.add_block(slot, model_index, paired_index)
    }

    /// Write a whole preset blob to the device's **edit buffer** (op 21) — the general structural
    /// edit, used when surgical ops can't express the change (delete, dense reorder, parallel). The
    /// blob comes from [`fretwire_data::stream::PresetStream::to_blob`] after mutating the tree. **This
    /// writes the edit buffer, not flash** — recoverable by reloading the preset; persist with
    /// `save_preset` only when intended.
    ///
    /// Transport: the large edit TLV is sent across `cmd 0x04` frames on the edit channel (the device
    /// reassembles by the TLV's declared length), then a terminating `cmd 0x08`, mirroring the
    /// chunked-read in reverse. The device interleaves `cmd 0x08` flow-control ACKs, which we drain.
    ///
    /// LIVE: the exact chunk size and `arg` cadence are reconstructed from `move_EQ_right_two_slots`
    /// and need first-contact tuning — every frame is logged at debug to diff the trace against the
    /// capture. The device's `{103:1}` apply-ACK is best-effort (logged, not required); the caller
    /// confirms by re-reading.
    pub fn write_preset(&mut self, blob: Vec<u8>) -> crate::Result<()> {
        const CHUNK: usize = 496; // ≤ one 512-byte bulk packet incl. the 16-byte frame header
        let txn = self.bump_txn();
        let tlv = Tlv::command(op::PARAM_SET, edit::write_preset(&blob, txn)).to_bytes();
        let (src, dst) = channel::EDIT;

        self.transport.drain();
        // arg stays at the channel cursor for the whole transfer (the capture barely advances it,
        // and small edits via `send_edit` don't advance per frame). LIVE: advance per chunk if the
        // device rejects a stalled offset.
        let arg = self.cur_arg(src);
        let total = tlv.len();
        let mut sent = 0usize;
        for chunk in tlv.chunks(CHUNK) {
            let seq = self.next_seq(src);
            self.transport.send_frame(&Frame::new(
                src,
                dst,
                seq,
                cmd::OPEN,
                arg,
                chunk.to_vec(),
            ))?;
            sent += chunk.len();
            tracing::debug!(arg, len = chunk.len(), sent, total, "write-preset chunk");
            // Consume the device's interleaved flow-control ACKs so neither side's queue backs up.
            let _ = self
                .transport
                .drain_collect(std::time::Duration::from_millis(5), 8);
        }
        // Terminate the transfer (empty cmd 0x08), as HX Edit does after the last data frame.
        let seq = self.next_seq(src);
        self.transport
            .send_frame(&Frame::new(src, dst, seq, cmd::CHUNK, arg, Vec::new()))?;

        // Best-effort: note whether the device echoed our txn in its apply-ACK (key 103). Not gated
        // on — a re-read by the caller is the real confirmation.
        let acks = self
            .transport
            .drain_collect(std::time::Duration::from_millis(80), 32);
        let acked = acks.iter().any(|f| reply_txn(&f.body) == Some(txn));
        tracing::info!(bytes = total, acked, "write-preset sent");
        Ok(())
    }

    // ---- edit history (undo / redo / timeline jump) ------------------------------------------
    //
    // History is uniform across every edit type: a timeline of **preset blob snapshots**, any of
    // which can be restored with the op-21 whole-preset write (edit buffer only — flash untouched).
    // No per-op inverse logic. Entry 0 is the loaded state; each later entry is the state *after*
    // one edit, labeled. The cursor is the state currently in the buffer: undo/redo step it, a
    // timeline jump sets it anywhere, and a fresh edit truncates everything after it. The caller
    // (the GUI command layer) brackets each undoable edit with `edit_begin(label)`/`edit_commit()`;
    // the blobs come from `last_raw` — refreshed by every read — so snapshots cost no USB traffic.

    /// The raw preset stream from the last read, if any. Cached by every read, so this costs no
    /// USB traffic — for callers that need the undecoded bytes (diagnostics, `PresetStream`
    /// fields the editor view doesn't surface) after a `read_preset`.
    pub fn last_raw(&self) -> Option<&[u8]> {
        self.last_raw.as_deref()
    }

    /// The current edit buffer as an op-21-writable blob, from the read cache (or a fresh read).
    fn current_blob(&mut self) -> Option<Vec<u8>> {
        let raw = match &self.last_raw {
            Some(r) => r.clone(),
            None => self.read_preset_raw().ok()?,
        };
        fretwire_data::stream::PresetStream::parse(&raw)
            .ok()
            .map(|ps| ps.to_blob())
    }

    /// Human name for the block at `slot` — the user label if set, else the model name — resolved
    /// from the cached pre-edit preset. Falls back to `"slot N"` when nothing is cached (or the
    /// slot is empty/structural). For history-entry labels.
    pub fn slot_label(&self, slot: i64) -> String {
        self.last_raw
            .as_ref()
            .and_then(|raw| self.catalog.load_preset(raw).ok())
            .and_then(|p| {
                p.block(slot)
                    .map(|b| b.user_label.clone().unwrap_or_else(|| b.model_name.clone()))
            })
            .unwrap_or_else(|| format!("slot {slot}"))
    }

    /// `"<param> — <block>"` for history labels (e.g. `"Drive — US Princess"`); falls back through
    /// progressively less specific forms when the metadata isn't resolvable.
    pub fn param_label(&self, slot: i64, paired: bool, param_index: i64) -> String {
        let block_name = self.slot_label(slot);
        let param_name = self
            .last_raw
            .as_ref()
            .and_then(|raw| self.catalog.load_preset(raw).ok())
            .and_then(|p| {
                p.block(slot).and_then(|b| {
                    let params = if paired { &b.paired_params } else { &b.params };
                    params
                        .iter()
                        .find(|q| q.index as i64 == param_index)
                        .map(|q| q.name.clone())
                })
            });
        match param_name {
            Some(n) => format!("{n} — {block_name}"),
            None => block_name,
        }
    }

    /// A model's display name by `Helix.sym` index, for labeling adds/swaps.
    pub fn model_label(&self, model_index: i64) -> String {
        self.catalog
            .model_name_by_index(model_index)
            .unwrap_or_else(|| "block".into())
    }

    /// Start an undoable edit: seed the timeline with the loaded state if empty, drop any redo
    /// branch after the cursor, and remember `label` for [`Self::edit_commit`]. Best-effort — the
    /// edit itself must never be blocked by history bookkeeping.
    pub fn edit_begin(&mut self, label: &str) {
        if self.history.is_empty() {
            match self.current_blob() {
                Some(blob) => {
                    self.history.push(HistoryEntry {
                        label: "Loaded".into(),
                        blob,
                    });
                    self.cursor = 0;
                }
                None => {
                    tracing::warn!("edit history: no preset state to seed; skipping this entry");
                    self.pending = None;
                    return;
                }
            }
        }
        self.history.truncate(self.cursor + 1);
        // If the flash-saved state lived in the discarded redo branch, no cursor matches it now.
        if self.saved_cursor.is_some_and(|i| i > self.cursor) {
            self.saved_cursor = None;
        }
        self.pending = Some(label.to_string());
    }

    /// Finish an undoable edit: snapshot the (re-read) post-edit state as a new timeline entry and
    /// move the cursor to it. No-op if [`Self::edit_begin`] didn't run or the edit failed first.
    pub fn edit_commit(&mut self) {
        let Some(label) = self.pending.take() else {
            return;
        };
        let Some(blob) = self.current_blob() else {
            tracing::warn!("edit history: no post-edit state to snapshot");
            return;
        };
        self.history.push(HistoryEntry { label, blob });
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
            // Timeline indices shifted down by one; the saved marker moves with them (or falls off).
            self.saved_cursor = match self.saved_cursor {
                Some(i) if i > 0 => Some(i - 1),
                _ => None,
            };
        }
        self.cursor = self.history.len() - 1;
    }

    /// Steps available behind the cursor (enables the Undo button).
    pub fn undo_depth(&self) -> usize {
        self.cursor
    }

    /// Steps available ahead of the cursor (enables the Redo button).
    pub fn redo_depth(&self) -> usize {
        self.history.len().saturating_sub(self.cursor + 1)
    }

    /// The timeline's labels, oldest first, and the cursor — for the GUI's history pane.
    pub fn history_labels(&self) -> Vec<String> {
        self.history.iter().map(|e| e.label.clone()).collect()
    }

    pub fn history_cursor(&self) -> usize {
        self.cursor
    }

    /// Drop all history — call when the editing context changes (e.g. switching preset).
    pub fn clear_history(&mut self) {
        self.history.clear();
        self.cursor = 0;
        self.pending = None;
        // A fresh context means the buffer was just (re)loaded from flash — clean.
        self.saved_cursor = Some(0);
    }

    /// `true` when the edit buffer differs from what's saved in flash — i.e. the history cursor
    /// has moved off the last saved state. Tracks **history-bracketed** edits (everything the GUI
    /// does); direct un-bracketed sends (CLI probes) and device-panel edits don't register.
    pub fn dirty(&self) -> bool {
        self.saved_cursor != Some(self.cursor)
    }

    /// Jump the edit buffer to timeline entry `index` (op-21 write of that snapshot) — the primitive
    /// behind undo, redo, history-pane clicks, and A/B compare. Pure navigation: no entry is added
    /// or removed, so jumping around is free of side effects until the next edit truncates the
    /// future. Errors (cursor unmoved) on a bad index or write failure. Re-reads and returns.
    pub fn history_jump(&mut self, index: usize) -> crate::Result<EditorPreset> {
        if index >= self.history.len() {
            return Err(fretwire_data::Error::Stream(format!(
                "no history entry {index} (have {})",
                self.history.len()
            ))
            .into());
        }
        if index == self.cursor {
            return self.read_preset();
        }
        self.write_preset(self.history[index].blob.clone())?;
        self.cursor = index;
        self.read_preset()
    }

    /// Step back one timeline entry.
    pub fn undo(&mut self) -> crate::Result<EditorPreset> {
        if self.cursor == 0 {
            return Err(fretwire_data::Error::Stream("nothing to undo".into()).into());
        }
        self.history_jump(self.cursor - 1)
    }

    /// Step forward one timeline entry.
    pub fn redo(&mut self) -> crate::Result<EditorPreset> {
        if self.cursor + 1 >= self.history.len() {
            return Err(fretwire_data::Error::Stream("nothing to redo".into()).into());
        }
        self.history_jump(self.cursor + 1)
    }

    /// Change the parallel **split node** at `split_slot` to a different split type (`model_index`
    /// from [`crate::editor::SPLIT_TYPES`]) via swap-model (op 40) — surgical/FS-safe. The device
    /// resets the node's params to the new type's defaults, as with any model swap. Re-reads and
    /// returns the new preset.
    pub fn set_split_type(
        &mut self,
        split_slot: i64,
        model_index: i64,
    ) -> crate::Result<EditorPreset> {
        self.swap_model(split_slot, model_index, -1)?;
        self.read_preset()
    }

    /// Move the block at `src_slot` to the other signal-path row at insertion index `pos` among that
    /// row's blocks: into the **parallel/bottom (B)** row when `parallel` is true, or back to the
    /// **series/top (A)** row otherwise. `pos` is how many of the target row's blocks precede the drop
    /// point (`0` = front of the row, `n` = end) — so the block lands where it was dropped, not at the
    /// first free slot. The **column it lands in decides the split/mixer positions**: the device
    /// recomputes them (verified — a low column pulls the split earlier, a high column leaves it put),
    /// so targeting the dropped position gives the routing the user drew. `move_block` (op 43) is
    /// surgical (footswitch-safe); the device activates/retires the split as needed. Re-reads and
    /// returns the preset.
    pub fn move_block_to_row(
        &mut self,
        src_slot: i64,
        parallel: bool,
        pos: usize,
    ) -> crate::Result<EditorPreset> {
        use fretwire_data::stream::{PresetStream, slot_kind, split_wire_slot};
        let raw = self.read_preset_raw()?;
        let ps = PresetStream::parse(&raw)?;
        // Plan in wire space (`dsp * 20 + index`) against the source block's own DSP, so this works
        // on either DSP of a Floor rather than silently planning against DSP 0.
        let (dsp, _) = split_wire_slot(src_slot);
        let blocks = ps.dsp_blocks(dsp);
        let base = blocks.first().map(|b| b.wire_slot()).unwrap_or(0);
        let split_slot = blocks
            .iter()
            .find(|b| b.kind == slot_kind::SPLIT)
            .map(|b| b.wire_slot() as usize)
            .ok_or_else(|| {
                fretwire_data::Error::Stream("no split node in the preset grid".into())
            })?;
        let mixer_slot = blocks
            .iter()
            .find(|b| b.kind == slot_kind::MIXER)
            .map(|b| b.wire_slot() as usize)
            .unwrap_or(base as usize + blocks.len());
        // The target row's slot window (exclusive of the structural nodes): bottom = between split and
        // mixer; top = before the split node.
        let (lo, hi) = if parallel {
            (split_slot, mixer_slot)
        } else {
            (base as usize, split_slot)
        };
        // Blocks already in that window, and its empty slots — both in slot order.
        let occ: Vec<usize> = blocks
            .iter()
            .filter(|b| {
                let s = b.wire_slot() as usize;
                b.kind == slot_kind::EFFECT && s > lo && s < hi && b.wire_slot() != src_slot
            })
            .map(|b| b.wire_slot() as usize)
            .collect();
        let free: Vec<usize> = blocks
            .iter()
            .filter(|b| {
                let s = b.wire_slot() as usize;
                b.kind == slot_kind::EMPTY && s > lo && s < hi
            })
            .map(|b| b.wire_slot() as usize)
            .collect();
        let no_slot = || {
            let row = if parallel { "parallel" } else { "series" };
            fretwire_data::Error::Stream(format!("no free slot in the {row} row"))
        };
        let (moves, target) = if parallel {
            // Parallel (B) row: bubble the suffix right so a front insert anchors the section's left
            // edge — the device then leaves the split (and common-before blocks) where they are.
            plan_row_insert(&occ, &free, pos).ok_or_else(no_slot)?
        } else {
            // Top (A/common) row: drop into the free slot **between** the neighboring blocks, so the
            // block lands in the region it was dropped in and no block is bubbled across the split or
            // mixer boundary (which would silently re-classify it A ↔ common). `usize::MAX` = end.
            let pos = pos.min(occ.len());
            let lower = if pos == 0 { lo } else { occ[pos - 1] };
            let upper = if pos < occ.len() { occ[pos] } else { hi };
            let target = free
                .iter()
                .copied()
                .find(|&s| s > lower && s < upper)
                .or_else(|| free.iter().copied().find(|&s| s > lower))
                .or_else(|| free.first().copied())
                .ok_or_else(no_slot)?;
            (Vec::new(), target)
        };
        let occupied: std::collections::BTreeSet<usize> = blocks
            .iter()
            .filter(|b| b.kind != slot_kind::EMPTY)
            .map(|b| b.wire_slot() as usize)
            .collect();
        self.apply_row_moves(moves, src_slot, target as i64, occupied)?;
        self.read_preset()
    }

    /// Move `src_slot` into the **common (pre-split) section, just before the split** on a split
    /// preset. Places it at the rightmost common-before slot, shifting the existing common blocks
    /// **left** (`plan_insert_right_end`) so the split's column — and thus the whole parallel section
    /// — stays anchored. Re-reads and returns the preset.
    pub fn move_before_split(&mut self, src_slot: i64) -> crate::Result<EditorPreset> {
        use fretwire_data::stream::{PresetStream, slot_kind, split_wire_slot};
        let raw = self.read_preset_raw()?;
        let ps = PresetStream::parse(&raw)?;
        // Wire space, against the source block's own DSP (see `move_block_to_row`).
        let (dsp, _) = split_wire_slot(src_slot);
        let blocks = ps.dsp_blocks(dsp);
        let base = blocks.first().map(|b| b.wire_slot()).unwrap_or(0) as usize;
        // The split's signal-flow *column*; on the top row column == local index, so the wire slot
        // that column corresponds to is `base + split_pos`.
        let split_pos = ps
            .dsp_structural_node_pos(dsp, slot_kind::SPLIT)
            .ok_or_else(|| fretwire_data::Error::Stream("preset is not split".into()))?
            as usize;
        let split_wire = base + split_pos;
        // Common-before window = top slots below the split column.
        let occ: Vec<usize> = blocks
            .iter()
            .filter(|b| {
                let s = b.wire_slot() as usize;
                b.kind == slot_kind::EFFECT && s < split_wire && b.wire_slot() != src_slot
            })
            .map(|b| b.wire_slot() as usize)
            .collect();
        let free: Vec<usize> = blocks
            .iter()
            .filter(|b| {
                let s = b.wire_slot() as usize;
                b.kind == slot_kind::EMPTY && s > base && s < split_wire
            })
            .map(|b| b.wire_slot() as usize)
            .collect();
        let (moves, target) = plan_insert_right_end(&occ, &free, split_wire)
            .ok_or_else(|| fretwire_data::Error::Stream("no free slot before the split".into()))?;
        let occupied: std::collections::BTreeSet<usize> = blocks
            .iter()
            .filter(|b| b.kind != slot_kind::EMPTY)
            .map(|b| b.wire_slot() as usize)
            .collect();
        self.apply_row_moves(moves, src_slot, target as i64, occupied)?;
        self.read_preset()
    }

    /// Place the block at `src_slot` into the exact grid slot `dst_slot` — one op-43 move (the device
    /// recomputes the split/mixer columns from where blocks land). `dst_slot` must be empty:
    /// [`Self::apply_row_moves`]' guard refuses moving onto an occupied slot (op 43 would overwrite
    /// the block there). This is the primitive behind the routing grid — every cell is one exact
    /// slot, so a drop is a single move. Re-reads and returns the preset.
    pub fn place_block(&mut self, src_slot: i64, dst_slot: i64) -> crate::Result<EditorPreset> {
        use fretwire_data::stream::{PresetStream, slot_kind, split_wire_slot};
        if src_slot == dst_slot {
            return self.read_preset();
        }
        let (dsp, _) = split_wire_slot(src_slot);
        if split_wire_slot(dst_slot).0 != dsp {
            return Err(
                fretwire_data::Error::Stream("can't move a block between DSPs".into()).into(),
            );
        }
        let raw = self.read_preset_raw()?;
        let ps = PresetStream::parse(&raw)?;
        // Everything that isn't an empty slot is "occupied" — incl. the split/mixer nodes, so a drop
        // can never land on a structural node either. Wire slots, so the guard matches the ops.
        let occupied: std::collections::BTreeSet<usize> = ps
            .dsp_blocks(dsp)
            .iter()
            .filter(|b| b.kind != slot_kind::EMPTY)
            .map(|b| b.wire_slot() as usize)
            .collect();
        self.apply_row_moves(Vec::new(), src_slot, dst_slot, occupied)?;
        self.read_preset()
    }

    /// Insert the block at `src_slot` **before or after** the occupied `dst_slot` (a drop on a
    /// block's left/right half in the grid), shifting neighbors to make room — the occupied-drop
    /// counterpart of [`place_block`], with HX Edit's insert semantics rather than swap/overwrite.
    ///
    /// Within the same row this is a [`plan_reorder`] bubble (park src in a scratch empty slot,
    /// shift the between-blocks one step, drop src into the freed slot — the row's occupied slot
    /// *set* is unchanged). Across rows it's a [`plan_row_insert`] into the destination row (shift
    /// the suffix right into free slots), then the cross-row move. All single guarded op-43s.
    /// Re-reads once and returns the preset.
    pub fn insert_block(
        &mut self,
        src_slot: i64,
        dst_slot: i64,
        before: bool,
    ) -> crate::Result<EditorPreset> {
        use fretwire_data::stream::{DSP_SLOT_STRIDE, PresetStream, slot_kind, split_wire_slot};
        if src_slot == dst_slot {
            return self.read_preset();
        }
        let (dsp, _) = split_wire_slot(src_slot);
        if split_wire_slot(dst_slot).0 != dsp {
            return Err(
                fretwire_data::Error::Stream("can't move a block between DSPs".into()).into(),
            );
        }
        let base = dsp as i64 * DSP_SLOT_STRIDE;
        let raw = self.read_preset_raw()?;
        let ps = PresetStream::parse(&raw)?;
        let blocks = ps.dsp_blocks(dsp);
        for s in [src_slot, dst_slot] {
            if !blocks
                .iter()
                .any(|b| b.wire_slot() == s && b.kind == slot_kind::EFFECT)
            {
                return Err(fretwire_data::Error::Stream(format!("slot {s} has no block")).into());
            }
        }
        // The fixed topology's row windows, in wire slots: top row base+1..=base+8, row B
        // base+11..=base+18 (the nodes at base+0/9/10/19 bound them).
        let row_of = |s: i64| {
            let local = s - base;
            if (11..=18).contains(&local) {
                (base + 11, base + 18)
            } else {
                (base + 1, base + 8)
            }
        };
        let (lo, hi) = row_of(dst_slot);
        let same_row = row_of(src_slot) == (lo, hi);
        // The destination row's blocks and empties, in slot order (src excluded from `occ` so
        // insertion positions are counted among the blocks that will surround it).
        let occ: Vec<usize> = blocks
            .iter()
            .filter(|b| {
                b.kind == slot_kind::EFFECT
                    && (lo..=hi).contains(&b.wire_slot())
                    && b.wire_slot() != src_slot
            })
            .map(|b| b.wire_slot() as usize)
            .collect();
        let dst_pos = occ
            .iter()
            .position(|&s| s as i64 == dst_slot)
            .expect("dst is an effect block in this row");
        let pos = if before { dst_pos } else { dst_pos + 1 };

        if same_row {
            // Order-position reorder through a scratch slot (prefer one in this row so the split
            // topology never transiently changes; any empty slot works).
            let with_src: Vec<usize> = blocks
                .iter()
                .filter(|b| b.kind == slot_kind::EFFECT && (lo..=hi).contains(&b.wire_slot()))
                .map(|b| b.wire_slot() as usize)
                .collect();
            let from_pos = with_src
                .iter()
                .position(|&s| s as i64 == src_slot)
                .expect("src is in this row");
            let scratch = blocks
                .iter()
                .filter(|b| b.kind == slot_kind::EMPTY)
                .map(|b| b.wire_slot() as usize)
                .min_by_key(|&s| {
                    if (lo..=hi).contains(&(s as i64)) {
                        0
                    } else {
                        1
                    }
                })
                .ok_or_else(|| {
                    fretwire_data::Error::Stream("no empty slot to reorder through".into())
                })?;
            // `pos` is the insertion index among the *other* blocks (src excluded from `occ`), which
            // is exactly src's final order index — plan_reorder's `to`. (No −1 adjustment here:
            // that applies only when the drop gap is counted over the src-included list, as in
            // `reorder_block`.)
            let to_pos = pos.min(with_src.len().saturating_sub(1));
            for (a, b) in plan_reorder(&with_src, scratch, from_pos, to_pos) {
                let t1 = self.bump_txn();
                self.send_edit(edit::begin_structural(a as i64, t1))?;
                let t2 = self.bump_txn();
                self.send_edit(edit::move_block(a as i64, b as i64, t2))?;
            }
        } else {
            let free: Vec<usize> = blocks
                .iter()
                .filter(|b| b.kind == slot_kind::EMPTY && (lo..=hi).contains(&b.wire_slot()))
                .map(|b| b.wire_slot() as usize)
                .collect();
            let (moves, target) = plan_row_insert(&occ, &free, pos).ok_or_else(|| {
                fretwire_data::Error::Stream("no free slot in the destination row".into())
            })?;
            let occupied: std::collections::BTreeSet<usize> = blocks
                .iter()
                .filter(|b| b.kind != slot_kind::EMPTY)
                .map(|b| b.wire_slot() as usize)
                .collect();
            self.apply_row_moves(moves, src_slot, target as i64, occupied)?;
        }
        self.read_preset()
    }

    /// Run a planned sequence of op-43 moves (each preceded by op-78), then move `src_slot` into
    /// `target`. **Refuses any move onto an occupied slot** (op 43 would overwrite/destroy the block
    /// there), aborting instead — a backstop so a planner bug can never delete a block. `occupied` is
    /// the live occupancy at the start; it's updated as moves apply.
    fn apply_row_moves(
        &mut self,
        moves: Vec<(usize, usize)>,
        src_slot: i64,
        target: i64,
        mut occupied: std::collections::BTreeSet<usize>,
    ) -> crate::Result<()> {
        let guard =
            |occupied: &std::collections::BTreeSet<usize>, dst: usize| -> crate::Result<()> {
                if occupied.contains(&dst) {
                    Err(fretwire_data::Error::Stream(format!(
                        "refusing move onto occupied slot {dst} (would delete a block)"
                    ))
                    .into())
                } else {
                    Ok(())
                }
            };
        for (from, to) in moves {
            guard(&occupied, to)?;
            occupied.remove(&from);
            occupied.insert(to);
            let t1 = self.bump_txn();
            self.send_edit(edit::begin_structural(from as i64, t1))?;
            let t2 = self.bump_txn();
            self.send_edit(edit::move_block(from as i64, to as i64, t2))?;
        }
        guard(&occupied, target as usize)?;
        let t1 = self.bump_txn();
        self.send_edit(edit::begin_structural(src_slot, t1))?;
        let t2 = self.bump_txn();
        self.send_edit(edit::move_block(src_slot, target, t2))?;
        Ok(())
    }

    /// Append a block to the end of the serial chain: add `model_index` (with `paired_index`, `-1`
    /// for none) at the first empty slot **after** the last occupied block, via the surgical op 39
    /// (the device fills the new block's default params, and footswitch bindings are preserved).
    /// Re-reads and returns the new preset. The user can then drag it into position. Returns an error
    /// if the chain has no free slot.
    pub fn add_block_append(
        &mut self,
        model_index: i64,
        paired_index: i64,
    ) -> crate::Result<EditorPreset> {
        use fretwire_data::stream::{PresetStream, slot_kind};
        let raw = self.read_preset_raw()?;
        let ps = PresetStream::parse(&raw)?;
        let blocks = ps.dsp_blocks(0);
        // On a split preset, "append" means the end of the **series (row A)** row — the slots before
        // the split node. Prefer the first empty row-A slot after the last row-A block; fall back to
        // any empty row-A slot, then any empty slot at all (row B), so it never fails when A is full.
        let split_idx = blocks
            .iter()
            .find(|b| b.kind == slot_kind::SPLIT)
            .map(|b| b.index);
        let in_row_a = |idx: usize| split_idx.is_none_or(|s| idx < s);
        let series_last = blocks
            .iter()
            .filter(|b| b.kind == slot_kind::EFFECT && in_row_a(b.index))
            .map(|b| b.index)
            .max()
            .unwrap_or(0);
        let target = blocks
            .iter()
            .find(|b| b.kind == slot_kind::EMPTY && in_row_a(b.index) && b.index > series_last)
            .or_else(|| {
                blocks
                    .iter()
                    .find(|b| b.kind == slot_kind::EMPTY && in_row_a(b.index))
            })
            .or_else(|| blocks.iter().find(|b| b.kind == slot_kind::EMPTY))
            .map(|b| b.index as i64)
            .ok_or_else(|| fretwire_data::Error::Stream("no free slot to add a block".into()))?;
        self.add_block(target, model_index, paired_index)?;
        self.read_preset_settled(target)
    }

    /// Move a structural node's signal-flow **column position** along the top row — `kind` 2 = split,
    /// 3 = mixer (`slot_kind::{SPLIT, MIXER}`). Repositioning the split/join points is what
    /// re-classifies top-row blocks between common-before (`col < split_pos`), path A
    /// (`split_pos ≤ col < mixer_pos`) and common-after (`col ≥ mixer_pos`) without moving any block.
    ///
    /// There is no surgical op for this — the position is the node holder's key 13 in the preset
    /// data — so it goes through the **op-21 whole-preset write** (edit buffer only, like every op-21;
    /// persist with `save_preset`). Guards keep the topology coherent before writing: the bracket
    /// must still enclose every occupied row-B column, and split < mixer. The device honors a
    /// written position verbatim — verified live 2026-07-06 (drag ⋔/⋉ in the GUI; the re-read and
    /// the pedal's own routing display both follow). [solid]
    pub fn set_node_pos(&mut self, dsp: usize, kind: i64, pos: i64) -> crate::Result<EditorPreset> {
        use fretwire_data::stream::{PresetStream, slot_kind};
        if kind != slot_kind::SPLIT && kind != slot_kind::MIXER {
            return Err(
                fretwire_data::Error::Stream(format!("not a movable node kind: {kind}")).into(),
            );
        }
        let raw = self.read_preset_raw()?;
        let mut ps = PresetStream::parse(&raw)?;
        if !ps.dsp_is_split(dsp) {
            return Err(fretwire_data::Error::Stream(format!("DSP {dsp} is not split")).into());
        }
        let other_kind = if kind == slot_kind::SPLIT {
            slot_kind::MIXER
        } else {
            slot_kind::SPLIT
        };
        let other = ps
            .dsp_structural_node_pos(dsp, other_kind)
            .ok_or_else(|| fretwire_data::Error::Stream("missing peer node position".into()))?;
        // Occupied row-B columns (grid row 1) — the bracket must keep enclosing them.
        let b_cols: Vec<i64> = ps
            .dsp_grid(dsp)
            .iter()
            .filter(|c| c.row == 1 && c.occupied)
            .map(|c| c.column)
            .collect();
        let (lo, hi) = if kind == slot_kind::SPLIT {
            // split: ≥ 1, ≤ first B block's column, and strictly left of the mixer.
            (
                1,
                b_cols
                    .iter()
                    .min()
                    .copied()
                    .unwrap_or(other - 1)
                    .min(other - 1),
            )
        } else {
            // mixer: past the last B block's column, and strictly right of the split. The grid is
            // 8 columns wide, so column 9 (just past the last one) is as far right as it goes.
            (
                (b_cols.iter().max().copied().unwrap_or(other) + 1).max(other + 1),
                9,
            )
        };
        if pos < lo || pos > hi {
            return Err(fretwire_data::Error::Stream(format!(
                "node position {pos} out of range {lo}..={hi} (bracket must enclose the B row)"
            ))
            .into());
        }
        if !ps.set_dsp_node_pos(dsp, kind, pos) {
            return Err(
                fretwire_data::Error::Stream("node holder not found in preset".into()).into(),
            );
        }
        self.write_preset(ps.to_blob())?;
        self.read_preset()
    }

    /// **Probe:** read the current preset and write it straight back **unchanged** via op 21, then
    /// re-read. Exercises the serializer + chunked write end-to-end without altering anything — the
    /// safe first hardware test of the op-21 path. If the re-read matches, the foundation works
    /// (incl. the device tolerating our re-encoded blob's now-stale header).
    pub fn rewrite_preset_unchanged(&mut self) -> crate::Result<EditorPreset> {
        let raw = self.read_preset_raw()?;
        let ps = fretwire_data::stream::PresetStream::parse(&raw)?;
        self.write_preset(ps.to_blob())?;
        self.read_preset()
    }

    /// Delete the block at `slot` (op 28 — **surgical**). HX Edit precedes the delete with a
    /// begin-structural marker (op 78), which we mirror; both ride the edit channel. Unlike the old
    /// whole-preset-write approach (op 21), this **preserves the footswitch layout** of the remaining
    /// blocks — the device drops only the deleted block's own binding, exactly as HX Edit does.
    /// Re-reads and returns the new preset.
    pub fn delete_block(&mut self, slot: i64) -> crate::Result<EditorPreset> {
        let t1 = self.bump_txn();
        self.send_edit(edit::begin_structural(slot, t1))?;
        let t2 = self.bump_txn();
        self.send_edit(edit::delete_block(slot, t2))?;
        self.read_preset()
    }

    /// Reorder a block within the serial chain: move the block at `src_slot` so it lands at order
    /// position `gap` among the serial blocks (`gap` = how many blocks precede the drop point;
    /// `0` = front, `n` = end), shifting the others to make room. Since op 43 only relocates a block
    /// into an **empty** slot, this is realized as a sequence of single moves bubbling through a
    /// spare empty slot — each preceded by op 78 — exactly as HX Edit does. Re-reads once at the end
    /// and returns the new preset. Serial presets only for now (errors on a split preset).
    pub fn reorder_block(&mut self, src_slot: i64, gap: usize) -> crate::Result<EditorPreset> {
        use fretwire_data::stream::{PresetStream, slot_kind, split_wire_slot};
        let (dsp, _) = split_wire_slot(src_slot);
        let raw = self.read_preset_raw()?;
        let ps = PresetStream::parse(&raw)?;
        if ps.dsp_is_split(dsp) {
            return Err(fretwire_data::Error::Stream(
                "reordering on split (parallel) presets isn't supported yet".into(),
            )
            .into());
        }
        let blocks = ps.dsp_blocks(dsp); // this DSP's 20 slots, in index order
        let occupied: Vec<usize> = blocks
            .iter()
            .filter(|b| b.kind == slot_kind::EFFECT)
            .map(|b| b.wire_slot() as usize)
            .collect();
        let from_pos = occupied
            .iter()
            .position(|&s| s as i64 == src_slot)
            .ok_or_else(|| {
                fretwire_data::Error::Stream("source slot is not an effect block".into())
            })?;
        let scratch = blocks
            .iter()
            .find(|b| b.kind == slot_kind::EMPTY)
            .map(|b| b.wire_slot() as usize)
            .ok_or_else(|| {
                fretwire_data::Error::Stream("no empty slot to reorder through".into())
            })?;

        let n = occupied.len();
        // The drop gap (blocks-before-drop) maps to a final order position. Removing the dragged
        // block shifts later positions down by one, so a gap past it lands one slot earlier.
        let to_pos = if gap > from_pos { gap - 1 } else { gap }.min(n.saturating_sub(1));

        for (a, b) in plan_reorder(&occupied, scratch, from_pos, to_pos) {
            let t1 = self.bump_txn();
            self.send_edit(edit::begin_structural(a as i64, t1))?;
            let t2 = self.bump_txn();
            self.send_edit(edit::move_block(a as i64, b as i64, t2))?;
        }
        self.read_preset()
    }

    /// Rename a snapshot (op 89). `index` is 0-based, as `read_preset` reports `snapshot_names`.
    pub fn rename_snapshot(&mut self, index: i64, name: &str) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::rename_snapshot(index, name, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Set a global/input setting (op 25, `{118: id, 119: value}`) — not block-addressed. The id
    /// space is only partly mapped, so this is mainly a live-probe primitive for now (id 134 is a
    /// 3-state input setting). Not a flash write; recoverable.
    pub fn set_setting(&mut self, id: i64, value: i64) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::set_setting(id, value, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Set a knob/continuous parameter by its index in the model's device param order.
    pub fn set_param(&mut self, slot: i64, param_index: i64, value: f32) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::set_value(slot, param_index, value, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Set a knob/continuous parameter on the block's **paired cab/IR** (the second model fused into
    /// an amp+cab slot), by its index in the cab's param order. Same as [`Self::set_param`] but
    /// targets the paired sub-model (wire `26:1`).
    pub fn set_paired_param(
        &mut self,
        slot: i64,
        param_index: i64,
        value: f32,
    ) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::set_paired_value(slot, param_index, value, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Set an **integer/enum** parameter (e.g. the cab `Mic` selector) by its param index. `paired`
    /// targets the block's cab/IR sub-model (`26:1`) rather than the main model. The value is the
    /// option index, sent on the wire as an int (not a float).
    pub fn set_param_enum(
        &mut self,
        slot: i64,
        paired: bool,
        param_index: i64,
        value: i64,
    ) -> crate::Result<()> {
        let txn = self.bump_txn();
        let model_sel = if paired {
            edit::MODEL_PAIRED
        } else {
            edit::MODEL_MAIN
        };
        let body = edit::set_value_on(slot, model_sel, param_index, EditValue::Int(value), txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Read the currently-loaded preset live and decode it into the editor model. Confirmed working
    /// on hardware (reassembles ~2.8 KB → the loaded blocks, repeatable). The paged-stream wire
    /// sequence — and the per-channel `arg` accounting it relies on — lives in [`Self::read_preset_raw`].
    pub fn read_preset(&mut self) -> crate::Result<EditorPreset> {
        // The txn-matched structured steps pin chunk #0, but the raw pagination chunks (cmd 0x08)
        // carry no txn — so a state-push interleaved mid-stream could still corrupt the blob. If the
        // decode fails, drain harder and re-read once; transient interleaving clears on the retry.
        let mut last_err: Option<crate::Error> = None;
        for attempt in 0..2 {
            match self.read_preset_inner() {
                Ok((payload, info)) => match self.catalog.load_preset(&payload) {
                    Ok(mut preset) => {
                        preset.current = info;
                        // The blob's active snapshot is the one that was *stored* with the preset,
                        // which is not always the one the device is *currently* on — the unit has a
                        // global snapshot-recall preference, and a panel-side switch only reaches us
                        // as a status push. Logged so a hardware run can correlate the two; see
                        // docs/helix-floor.md.
                        tracing::debug!(
                            stored_active_snapshot = ?preset.active_snapshot,
                            snapshot_names = preset.snapshot_names.len(),
                            "decoded preset snapshot state"
                        );
                        self.last_raw = Some(payload);
                        // Seed the edit-history timeline with the loaded state (entry 0) the first
                        // time a preset is read after connect / preset switch — so the history pane
                        // exists (and A/B against "as loaded" works) before any edit is made.
                        if self.history.is_empty()
                            && let Some(blob) = self.current_blob()
                        {
                            self.history.push(HistoryEntry {
                                label: "Loaded".into(),
                                blob,
                            });
                            self.cursor = 0;
                        }
                        return Ok(preset);
                    }
                    Err(e) => last_err = Some(e),
                },
                Err(e) => last_err = Some(e),
            }
            tracing::warn!(attempt, "preset read/decode failed; draining and retrying");
            self.transport
                .drain_wire(std::time::Duration::from_millis(60), 256);
        }
        Err(last_err.expect("loop runs at least once"))
    }

    /// [`Self::read_preset`] for the read-back after a **structural edit** (model swap, add block):
    /// re-read until the block in `slot` stops changing between reads, so the caller can't catch the
    /// device mid-apply.
    ///
    /// Op 40 / op 39 are ACKed as soon as the device has taken the new model *reference*, but it
    /// rewrites that block's parameter area a moment later. A read issued straight after the ACK can
    /// therefore decode the new model's identity against the outgoing model's values — the chain
    /// shows the new block while the param panel still shows the one it replaced. (A second swap
    /// appeared to "fix" it only because its read landed after the device had settled.)
    ///
    /// Comparing consecutive decodes of the block needs no per-model knowledge, so it holds for any
    /// model pair. The budget is a ceiling, not a wait: a device that has already settled costs one
    /// confirming read. If it never settles we return the latest read rather than failing — a stale
    /// panel beats an error.
    pub fn read_preset_settled(&mut self, slot: i64) -> crate::Result<EditorPreset> {
        const SETTLE_WAIT: std::time::Duration = std::time::Duration::from_millis(40);
        const SETTLE_TRIES: usize = 4;

        let at = |p: &EditorPreset| p.blocks.iter().find(|b| b.slot == slot).cloned();
        let mut prev = self.read_preset()?;
        for attempt in 0..SETTLE_TRIES {
            std::thread::sleep(SETTLE_WAIT);
            let next = self.read_preset()?;
            if at(&prev) == at(&next) {
                return Ok(next);
            }
            tracing::debug!(slot, attempt, "block still settling after edit; re-reading");
            prev = next;
        }
        tracing::warn!(
            slot,
            "block never settled within the read budget; returning the last read"
        );
        Ok(prev)
    }

    /// Read the current preset stream and return the raw reassembled bytes (before decoding).
    /// Useful for diffing two device states to decode stream fields.
    pub fn read_preset_raw(&mut self) -> crate::Result<Vec<u8>> {
        let raw = self.read_preset_inner()?.0;
        self.last_raw = Some(raw.clone());
        Ok(raw)
    }

    /// Back up every preset in the setlist to a [`crate::backup::Backup`]. Walks the whole list
    /// (`goto` each slot → read its stream), then returns the device to the preset it started on.
    /// Reads only — flash is never written — but the active preset *cursor* moves during the sweep
    /// (audible if you're playing through the unit). `progress` is called once per preset with
    /// `(done, total, name)`.
    pub fn backup_setlist(
        &mut self,
        mut progress: impl FnMut(usize, usize, &str),
    ) -> crate::Result<crate::backup::Backup> {
        let listing = self.list_presets()?;
        let total = listing.len();
        // Note where we are so the sweep can put the user back afterwards.
        let (_, start) = self.read_preset_inner()?;

        let mut presets = Vec::with_capacity(total);
        for (done, (index, listed_name)) in listing.iter().enumerate() {
            let index = *index as i64;
            self.goto_preset(0, index)?;
            let (raw, info) = self.read_preset_inner()?;
            // The stream must parse (it's what restore replays), and the op-23 identity must be
            // the slot we selected — a mismatch means the sweep desynced; stop rather than save
            // mislabeled blobs.
            fretwire_data::stream::PresetStream::parse(&raw)?;
            if let Some(i) = &info
                && i.index != index
            {
                return Err(crate::Error::Backup(format!(
                    "device reports preset {} while backing up slot {index} — sweep desynced, aborting",
                    i.index
                )));
            }
            let name = info
                .map(|i| i.name)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| listed_name.clone());
            progress(done + 1, total, &name);
            presets.push(crate::backup::BackupPreset { index, name, raw });
        }

        if let Some(start) = start {
            self.goto_preset(start.bank, start.index)?;
        }
        // The sweep changed the editing context out from under any cached state.
        self.last_raw = None;
        self.clear_history();
        Ok(crate::backup::Backup {
            device: "HX Stomp".into(),
            presets,
        })
    }

    /// Restore one backed-up preset into setlist `slot`: select the slot, replay the stored stream
    /// into the edit buffer (op 21), and commit it to flash under `name` (op 71). **Overwrites
    /// `slot` persistently.** Returns the re-read preset as confirmation.
    ///
    /// LIVE: op-21 has only been proven with mutated blobs of the *current* preset; a foreign
    /// blob (different preset) is the same mechanism but unverified until first restore test.
    pub fn restore_preset(
        &mut self,
        raw: &[u8],
        slot: i64,
        name: &str,
    ) -> crate::Result<EditorPreset> {
        let ps = fretwire_data::stream::PresetStream::parse(raw)?;
        self.goto_preset(0, slot)?;
        self.write_preset(ps.to_blob())?;
        self.save_preset(0, slot, name)?;
        self.clear_history();
        self.read_preset()
    }

    /// The read sequence, returning both the reassembled stream and the current preset's identity
    /// (parsed from the op-23 read-info reply that the sequence issues anyway).
    fn read_preset_inner(
        &mut self,
    ) -> crate::Result<(Vec<u8>, Option<fretwire_data::stream::PresetInfo>)> {
        // Start aligned: clear any frames left on the wire from a prior edit's fire-and-forget
        // follow-up or, crucially, the device's **unsolicited state pushes** (a footswitch bypass or
        // panel knob/snapshot change). Mid-session those would otherwise be mis-matched as this
        // read's first reply and desync the whole sequence into a bulk-IN timeout. At connect the
        // wire is already quiet, so this just costs one short read.
        self.transport.drain();
        self.transport
            .drain_wire(std::time::Duration::from_millis(30), 128);

        // The non-destructive read sequence HX Edit issues on connect (decoded from startup.pcapng):
        // open the edit buffer (op 76), prepare (op 24), query identity (op 23), start the stream
        // (op 22), then paginate. This reads the *current* preset and does NOT select/change it.
        // Each structured step's reply echoes its txn (key 102) — match on it so an interleaved
        // state-push/keepalive can't be mistaken for the reply (the stream-start reply *is* chunk #0,
        // so a mismatch there yields a stream with no envelope → "key 104 missing").
        let txn = self.bump_txn();
        let open = self.edit_request_txn(
            cmd::OPEN,
            Tlv::command(op::PARAM_SET, edit::read_open(txn)).to_bytes(),
            txn,
        )?;
        tracing::info!(arg = open.arg, body = open.body.len(), "read-open reply");

        let txn = self.bump_txn();
        self.edit_request_txn(
            cmd::STREAM,
            Tlv::command(op::PARAM_SET, edit::read_prep(txn)).to_bytes(),
            txn,
        )?;

        let txn = self.bump_txn();
        let info = self.edit_request_txn(
            cmd::STREAM,
            Tlv::command(op::PARAM_SET, edit::read_info(txn)).to_bytes(),
            txn,
        )?;
        let preset_info = fretwire_data::stream::parse_preset_info(&info.body);
        tracing::info!(?preset_info, "read-info reply (current preset identity)");

        // Start the paged stream; the reply carries chunk #0 (and echoes the txn).
        let txn = self.bump_txn();
        let first = self.edit_request_txn(
            cmd::STREAM,
            Tlv::command(op::PARAM_SET, edit::stream_start(txn)).to_bytes(),
            txn,
        )?;
        tracing::info!(
            arg = first.arg,
            body = first.body.len(),
            "stream-start reply (chunk #0)"
        );

        // Reassemble: each reply's body is a chunk; request more (cmd 0x08, empty body) until the
        // stream ends. `edit_request` advances the channel offset per reply.
        //
        // The stream's envelope declares its own length (`declared_stream_len`), and we make that
        // the authority for "done". The older rule — "the first chunk shorter than chunk #0 ends
        // the stream" — is only a heuristic: on the Floor a single **empty** chunk reply can arrive
        // mid-stream (a batched keepalive/state-push mistaken for a chunk, or a zero-length packet),
        // and treating it as the terminator truncated the payload at an exact 256-byte boundary,
        // then desynced the wire. Live evidence: every truncated read Sean captured landed on a
        // multiple of 256 while every good read ended mid-chunk. With the declared length we skip a
        // premature short/empty chunk and keep reading until the payload is actually whole; the
        // heuristic still governs when the envelope length can't be read (fallback below).
        let mut payload = first.body.clone();
        let full_chunk = first.body.len();
        let target = fretwire_data::stream::declared_stream_len(&first.body);
        // Bounds so a garbage length or a device that never terminates can't loop forever.
        let max_chunks = target.map_or(4096, |t| t / full_chunk.max(1) + 8);
        let mut empties = 0usize;
        for _ in 0..max_chunks {
            if target.is_some_and(|t| payload.len() >= t) {
                break; // whole declared payload is in hand
            }
            let chunk = self.edit_request(cmd::CHUNK, Vec::new())?;
            let n = chunk.body.len();
            tracing::debug!(arg = chunk.arg, body = n, "chunk reply");
            payload.extend_from_slice(&chunk.body);
            if n < full_chunk {
                match target {
                    // No declared length: fall back to "short chunk ends the stream".
                    None => break,
                    // A short/empty chunk that completes the declared payload is the real terminator.
                    Some(t) if payload.len() >= t => break,
                    // A short/empty chunk *before* the declared end is spurious — skip it and keep
                    // reading. Bound consecutive empties so a wedged device still errors out.
                    Some(t) => {
                        empties += 1;
                        tracing::warn!(
                            got = payload.len(),
                            want = t,
                            empties,
                            "short chunk before declared stream end — skipping, continuing read",
                        );
                        if empties >= 8 {
                            break;
                        }
                    }
                }
            } else {
                empties = 0;
            }
        }

        self.transport.drain(); // clear any batched epilogue frames
        tracing::info!(
            bytes = payload.len(),
            declared = target,
            "reassembled preset stream",
        );
        Ok((payload, preset_info))
    }

    /// List all presets on the device as `(index, name)` pairs (non-destructive). Drives the
    /// browse stream (decoded from `startup.pcapng`): open the browse session (op 254) -> open the
    /// PRESETS resource (op 0) -> start the stream (op 1) -> paginate. HX Edit runs this on the
    /// primary channel, but our reconstructed handshake doesn't leave primary browse-ready; the
    /// edit channel serves the same browse resource and works, so we use it. Verified live: 126
    /// presets on the HX Stomp.
    ///
    /// Lists **bank 0**. On a device with setlists (the Helix Floor has eight) use
    /// [`Self::list_presets_in`] — this is the flat-list case and the Stomp's only one.
    pub fn list_presets(&mut self) -> crate::Result<Vec<(u16, String)>> {
        self.list_presets_in(0)
    }

    /// [`Self::list_presets`] for a specific **setlist** (`bank`). The index of each returned
    /// preset is relative to that setlist, and pairs with `goto_preset(bank, index)`.
    pub fn list_presets_in(&mut self, bank: i64) -> crate::Result<Vec<(u16, String)>> {
        // HX Edit lists on the primary channel, but our reconstructed handshake doesn't leave
        // primary browse-ready; the edit channel is browse-capable in our session. LIVE experiment.
        let chan = channel::EDIT;
        let tlv = |body: Vec<u8>| Tlv::command(op::SESSION_OPEN, body).to_bytes();

        // Start aligned, exactly like the preset read: a prior `read_preset` (or a device state-push)
        // can leave stream frames on the wire that would otherwise be reassembled here as the list —
        // yielding a preset *blob* (binary key 104) instead of the list *array* ("key 104 is not an
        // array"). Drain, then txn-match the structured steps so chunk #0 is the real stream start.
        self.transport.drain();
        self.transport
            .drain_wire(std::time::Duration::from_millis(30), 128);

        let txn = self.bump_txn();
        let open = self.edit_request_txn(cmd::OPEN, tlv(edit::browse_open(txn)), txn)?;
        tracing::info!(arg = open.arg, body = open.body.len(), "browse-open reply");

        let txn = self.bump_txn();
        self.edit_request_txn(cmd::OPEN, tlv(edit::presets_open(txn)), txn)?;

        let txn = self.bump_txn();
        let first =
            self.edit_request_txn(cmd::STREAM, tlv(edit::presets_stream(txn, bank)), txn)?;
        tracing::info!(
            arg = first.arg,
            body = first.body.len(),
            bank,
            "preset-list stream chunk #0"
        );

        let mut payload = first.body.clone();
        let full_chunk = first.body.len();
        loop {
            let chunk = self.channel_request(chan, cmd::CHUNK, Vec::new())?;
            let n = chunk.body.len();
            tracing::debug!(arg = chunk.arg, body = n, "list chunk");
            payload.extend_from_slice(&chunk.body);
            if n == 0 || n < full_chunk {
                break;
            }
        }
        self.transport.drain();
        tracing::info!(
            bytes = payload.len(),
            bank,
            "reassembled preset-list stream"
        );
        Ok(fretwire_data::stream::parse_preset_list(&payload)?)
    }

    /// Cleanly tear down the session, returning the pedal to standalone operation.
    ///
    /// HX Edit ends every session by sending a **session-close** frame (cmd `0x02`, *empty* body)
    /// on each channel — in the order **status → edit → primary** — which the device acks with the
    /// same opcode (verified in `launch_hx_*_close*.pcapng`). Skipping this leaves the device
    /// believing the editor is still attached: the front panel stays in the "connected" state and
    /// acts locked/wonky until power-cycled. Calling this (or just dropping the `Session`) is the fix.
    ///
    /// Sent **request/response** — HX Edit reads each ack at shutdown, and (verified on hardware) the
    /// device needs that beat plus a brief settle before the interface is released; firing the frames
    /// blind and dropping the interface immediately does **not** release the panel. Each close frame
    /// carries the channel's running `seq`/`arg` (the read path tracks them; un-tracked channels
    /// continue from the handshake seed). Idempotent.
    ///
    /// status and edit ack promptly; **primary often doesn't** (our reconstructed handshake diverges
    /// on that channel) — and the panel releases regardless — so we cap each ack wait short
    /// ([`CLOSE_ACK_WAIT`]) rather than block the full transport timeout on the primary miss.
    pub fn close(&mut self) -> crate::Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        for (src, dst) in [channel::STATUS, channel::EDIT, channel::PRIMARY] {
            let seq = self.next_seq(src);
            let arg = self.cur_arg(src);
            let frame = Frame::new(src, dst, seq, cmd::SESSION_CLOSE, arg, Vec::new());
            match self.transport.request_within(&frame, CLOSE_ACK_WAIT) {
                Ok(reply) => tracing::debug!(
                    src = format_args!("{src:#06x}"),
                    ack_arg = reply.arg,
                    "session-close acked"
                ),
                Err(e) => tracing::debug!(
                    src = format_args!("{src:#06x}"),
                    "session-close ack skipped ({e})"
                ),
            }
        }
        // Let the device finish processing the close before the interface is released.
        self.transport.drain();
        std::thread::sleep(std::time::Duration::from_millis(150));
        tracing::info!("session closed — pedal returned to standalone");
        Ok(())
    }
}

/// How long to wait for each channel's session-close ack before moving on. Short by design: the
/// primary channel often never acks, and blocking the full transport timeout on it would add ~3 s
/// to every session teardown (each `Session` closes on `Drop`).
const CLOSE_ACK_WAIT: std::time::Duration = std::time::Duration::from_millis(300);

impl Drop for Session {
    /// Best-effort clean teardown so a dropped `Session` never leaves the pedal panel-locked.
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// The transaction id (msgpack key 102) the device echoes in an edit-channel reply body, if the
/// body is one of the `{102, …}` envelopes (open/prep/info/stream-start replies). `None` for raw
/// stream chunks and non-envelope frames. Used to correlate a reply to the request that sent `txn`.
///
/// The reply is `<TLV header><msgpack map {102:txn, …}>`, and **key 102 is the map's first entry**.
/// We must NOT fully parse the map: the stream-start reply's value at key 104 is a streamed blob
/// whose declared length spans many chunks, so the visible bytes are truncated and a full decode
/// fails. So we hand-read just the map header + the leading `102:<int>` entry.
fn reply_txn(body: &[u8]) -> Option<u16> {
    for i in 0..body.len().min(24) {
        // A fixmap header (0x81..=0x8f) whose first key is positive-fixint 102 (0x66).
        if (0x81..=0x8f).contains(&body[i]) && body.get(i + 1) == Some(&0x66) {
            return match body.get(i + 2)? {
                v @ 0x00..=0x7f => Some(*v as u16),         // positive fixint
                0xcc => body.get(i + 3).map(|x| *x as u16), // uint8
                0xcd => Some(u16::from_be_bytes([*body.get(i + 3)?, *body.get(i + 4)?])), // uint16
                _ => None,
            };
        }
    }
    None
}

/// The first run of ≥3 printable ASCII bytes in `data`, as a string — used to spot the model code
/// (`"P33"`/`"P33Main"`) embedded in an identity reply.
fn ascii_run(data: &[u8]) -> Option<String> {
    let mut best: &[u8] = &[];
    let mut start = 0;
    for i in 0..=data.len() {
        let printable = i < data.len() && data[i].is_ascii_graphic();
        if !printable {
            if i - start > best.len() {
                best = &data[start..i];
            }
            start = i + 1;
        }
    }
    (best.len() >= 3).then(|| String::from_utf8_lossy(best).into_owned())
}

/// Plan a same-row reorder as a sequence of single op-43 moves, each into an empty slot.
///
/// `occupied` is the chain's effect-block slot indices in order; `scratch` is a spare empty slot
/// index to stage through; `from`/`to` are positions within `occupied` (move the block at `from` so
/// it ends at position `to`). The dragged block is parked in `scratch`, the blocks between shift one
/// slot to close the hole and open the destination, then it drops into place — so every move's
/// destination is empty at the moment it runs, and the occupied slot set is unchanged at the end
/// (`scratch` ends empty again). Returns an empty list when `from == to`.
fn plan_reorder(occupied: &[usize], scratch: usize, from: usize, to: usize) -> Vec<(usize, usize)> {
    if from == to || from >= occupied.len() || to >= occupied.len() {
        return Vec::new();
    }
    let s = occupied;
    let mut moves = Vec::with_capacity(to.abs_diff(from) + 2);
    moves.push((s[from], scratch)); // park the dragged block
    if to > from {
        for k in (from + 1)..=to {
            moves.push((s[k], s[k - 1])); // shift each later block one slot earlier
        }
    } else {
        for k in (to..from).rev() {
            moves.push((s[k], s[k + 1])); // shift each earlier block one slot later
        }
    }
    moves.push((scratch, s[to])); // drop the dragged block into the freed destination
    moves
}

/// Plan the op-43 moves to open a slot for a block being inserted at index `pos` among a row's
/// `occupied` slots (sorted), shifting the blocks at `pos..` toward **higher** slots to make room.
/// `free` is the row's empty slots (sorted). Returns `(moves, target)`: run `moves` (each lands in an
/// empty slot), then move the incoming block into `target`.
///
/// Shifting the suffix right — rather than dropping the incoming block in the lowest free slot —
/// keeps a parallel section's **left edge anchored**: inserting at the front of path B moves the
/// existing B blocks right and the newcomer takes their old leading column, so the device leaves the
/// split (and any common-before blocks) where they are instead of yanking the split to the start.
/// Returns `None` if there's no free slot to shift into.
fn plan_row_insert(
    occupied: &[usize],
    free: &[usize],
    pos: usize,
) -> Option<(Vec<(usize, usize)>, usize)> {
    use std::collections::BTreeSet;
    if pos >= occupied.len() {
        // Append: the first free slot past the last block (or any free slot if the row is empty).
        let last = occupied.last().copied();
        let target = free
            .iter()
            .copied()
            .find(|&s| last.is_none_or(|l| s > l))
            .or_else(|| free.first().copied())?;
        return Some((Vec::new(), target));
    }
    // Shift occupied[pos..] one step right (rightmost first) into free slots, freeing occupied[pos].
    let mut freeset: BTreeSet<usize> = free.iter().copied().collect();
    let mut moves = Vec::new();
    for i in (pos..occupied.len()).rev() {
        let from = occupied[i];
        let dest = *freeset.range((from + 1)..).next()?; // smallest free slot to its right
        moves.push((from, dest));
        freeset.remove(&dest);
        freeset.insert(from);
    }
    Some((moves, occupied[pos]))
}

/// Plan the op-43 moves to open the **rightmost** slot of a window (`hi` exclusive) for a block being
/// inserted at the high end — shifting the contiguous occupied run ending there one step **left** into
/// a free slot below. Mirror of [`plan_row_insert`]: used for the common-before section so inserting
/// just before the split anchors the split's column (right edge) instead of moving it. Returns
/// `(moves, target)`, or `None` if there's no free slot below the run.
fn plan_insert_right_end(
    occ: &[usize],
    free: &[usize],
    hi: usize,
) -> Option<(Vec<(usize, usize)>, usize)> {
    use std::collections::BTreeSet;
    let target = hi.checked_sub(1)?; // rightmost slot in the window
    let freeset: BTreeSet<usize> = free.iter().copied().collect();
    if freeset.contains(&target) {
        return Some((Vec::new(), target));
    }
    let occset: BTreeSet<usize> = occ.iter().copied().collect();
    // The contiguous occupied run ending at `target`; the slot just below it must be free to shift into.
    let mut run_start = target;
    while run_start > 0 && occset.contains(&(run_start - 1)) {
        run_start -= 1;
    }
    let g = run_start.checked_sub(1)?;
    if !freeset.contains(&g) {
        return None;
    }
    // Shift the run down by one (ascending order, each destination free when it runs).
    let moves: Vec<(usize, usize)> = (run_start..=target).map(|s| (s, s - 1)).collect();
    Some((moves, target))
}

#[cfg(test)]
mod right_end_tests {
    use super::plan_insert_right_end;

    #[test]
    fn insert_before_split_shifts_common_left() {
        // Common-before window (slots 1..5): Tremolo at 4, free 1/2/3. Insert just before the split
        // (rightmost = 4) → shift Tremolo 4→3, newcomer takes 4 (anchors the split's column).
        let (moves, target) = plan_insert_right_end(&[4], &[1, 2, 3], 5).unwrap();
        assert_eq!(moves, vec![(4, 3)]);
        assert_eq!(target, 4);
    }

    #[test]
    fn insert_before_split_cascades_run_left() {
        // Contiguous 3,4 with free 2 → cascade 3→2, 4→3, newcomer takes 4.
        let (moves, target) = plan_insert_right_end(&[3, 4], &[2], 5).unwrap();
        assert_eq!(moves, vec![(3, 2), (4, 3)]);
        assert_eq!(target, 4);
    }

    #[test]
    fn insert_before_split_rightmost_free_no_shift() {
        let (moves, target) = plan_insert_right_end(&[1, 2], &[3, 4], 5).unwrap();
        assert!(moves.is_empty());
        assert_eq!(target, 4);
    }

    #[test]
    fn insert_before_split_no_room() {
        // Common-before window full (1..5 all occupied) → nowhere to shift.
        assert!(plan_insert_right_end(&[1, 2, 3, 4], &[], 5).is_none());
    }
}

#[cfg(test)]
mod insert_pos_tests {
    use super::plan_reorder;

    /// Apply a plan_reorder move list to a slot→label board and return the final row order.
    fn simulate(
        labels: &[&str],
        slots: &[usize],
        scratch: usize,
        from: usize,
        to: usize,
    ) -> Vec<String> {
        let mut board: std::collections::BTreeMap<usize, String> = slots
            .iter()
            .copied()
            .zip(labels.iter().map(|s| s.to_string()))
            .collect();
        for (a, b) in plan_reorder(slots, scratch, from, to) {
            let block = board.remove(&a).expect("move source occupied");
            assert!(!board.contains_key(&b), "move destination must be empty");
            board.insert(b, block);
        }
        assert!(!board.contains_key(&scratch), "scratch ends empty");
        board.into_values().collect()
    }

    // `insert_block`'s same-row mapping: the drop position is counted over the *other* blocks (src
    // excluded), which is src's final order index — plan_reorder's `to` directly, no −1 adjustment.
    // Pins the off-by-one found in review: dragging block 0 to "after the 3rd other block" (pos 3)
    // must land it at final index 3, not 2.
    #[test]
    fn drop_after_maps_pos_to_final_index() {
        // [NG, Glitz, Min, Amp, SD]: drag NG onto Amp's right half → others [Glitz, Min, Amp, SD],
        // pos = 3 → final [Glitz, Min, Amp, NG, SD].
        let fin = simulate(
            &["NG", "Glitz", "Min", "Amp", "SD"],
            &[1, 2, 3, 4, 5],
            6,
            0,
            3,
        );
        assert_eq!(fin, ["Glitz", "Min", "Amp", "NG", "SD"]);
    }

    #[test]
    fn drop_before_maps_pos_to_final_index() {
        // Drag SD onto Glitz's left half → others [NG, Glitz, Min, Amp], pos = 1 → final
        // [NG, SD, Glitz, Min, Amp].
        let fin = simulate(
            &["NG", "Glitz", "Min", "Amp", "SD"],
            &[1, 2, 3, 4, 5],
            6,
            4,
            1,
        );
        assert_eq!(fin, ["NG", "SD", "Glitz", "Min", "Amp"]);
    }

    #[test]
    fn adjacent_drop_is_a_no_op() {
        // Dragging Min onto Amp's left half: pos over others = 2 = Min's own final index → no moves.
        assert!(plan_reorder(&[1, 2, 3, 4, 5], 6, 2, 2).is_empty());
    }
}

#[cfg(test)]
mod row_insert_tests {
    use super::plan_row_insert;

    #[test]
    fn insert_at_end_takes_next_free_slot_no_shift() {
        // B row: block at 15, free 16/17/18. Append (pos = len) → slot 16, no shifts. (Matches the
        // live "drop at end of B path" experiment: split/mixer stayed put.)
        let (moves, target) = plan_row_insert(&[15], &[16, 17, 18], 1).unwrap();
        assert!(moves.is_empty());
        assert_eq!(target, 16);
    }

    #[test]
    fn insert_at_front_shifts_suffix_right() {
        // Insert before the block at 15 → shift 15→16, newcomer takes 15 (anchors the left edge).
        let (moves, target) = plan_row_insert(&[15], &[16, 17, 18], 0).unwrap();
        assert_eq!(moves, vec![(15, 16)]);
        assert_eq!(target, 15);
    }

    #[test]
    fn insert_front_contiguous_cascades() {
        // Two adjacent blocks 14,15 with free 16.. → cascade 15→16, 14→15, newcomer takes 14.
        let (moves, target) = plan_row_insert(&[14, 15], &[16, 17], 0).unwrap();
        assert_eq!(moves, vec![(15, 16), (14, 15)]);
        assert_eq!(target, 14);
    }

    #[test]
    fn insert_no_room_returns_none() {
        // Block at 18 (top of the region) with no free slot to its right → can't shift.
        assert!(plan_row_insert(&[18], &[], 0).is_none());
    }
}

/// The routing methods now plan in global wire-slot space (`dsp * 20 + index`) so they work on
/// either DSP. That is only sound if the `plan_*` helpers are base-agnostic — they compare and pick
/// slots, never assuming a 0 base. These mirror the DSP-0 cases above at DSP 2's base (20), and the
/// output must be exactly the DSP-0 result shifted by 20.
#[cfg(test)]
mod dsp2_base_tests {
    use super::{plan_reorder, plan_row_insert};

    #[test]
    fn row_insert_matches_dsp0_shifted_by_20() {
        // DSP2 B row: block at 35 (= 15 + 20), free 36/37/38. Insert at front → shift 35→36,
        // newcomer takes 35 — the DSP-0 `(15→16), target 15` case, shifted by a DSP.
        let (moves, target) = plan_row_insert(&[35], &[36, 37, 38], 0).unwrap();
        assert_eq!(moves, vec![(35, 36)]);
        assert_eq!(target, 35);

        // Append at the end takes the next free slot with no shifts, same as DSP 0.
        let (moves, target) = plan_row_insert(&[35], &[36, 37, 38], 1).unwrap();
        assert!(moves.is_empty());
        assert_eq!(target, 36);
    }

    #[test]
    fn reorder_bubbles_through_a_dsp2_scratch_slot() {
        // DSP2 top row blocks [21,22,23,24], scratch empty at 25. Move the first block to the end:
        // park it in 25, shift the rest left, drop it into the vacated last slot — all in DSP2's
        // slots, never touching DSP1 (0..19).
        let moves = plan_reorder(&[21, 22, 23, 24], 25, 0, 3);
        assert_eq!(
            moves,
            vec![(21, 25), (22, 21), (23, 22), (24, 23), (25, 24)]
        );
        assert!(
            moves
                .iter()
                .flat_map(|&(a, b)| [a, b])
                .all(|s| (20..40).contains(&s))
        );
    }
}

#[cfg(test)]
mod reorder_tests_legacy {
    use super::plan_reorder;

    /// Apply a move list to an occupancy/content model and assert every destination was empty.
    fn simulate(slots: &[usize], scratch: usize, moves: &[(usize, usize)]) -> Vec<Option<usize>> {
        // cell[slot] = Some(block_id) | None. Block ids = original position in `slots`.
        let max = slots
            .iter()
            .copied()
            .chain(std::iter::once(scratch))
            .max()
            .unwrap_or(0);
        let mut cell = vec![None; max + 1];
        for (id, &sl) in slots.iter().enumerate() {
            cell[sl] = Some(id);
        }
        for &(a, b) in moves {
            assert!(cell[a].is_some(), "move source {a} was empty");
            assert!(
                cell[b].is_none(),
                "move dest {b} was occupied (op 43 needs an empty slot)"
            );
            cell[b] = cell[a].take();
        }
        cell
    }

    #[test]
    fn noop_when_same_position() {
        assert!(plan_reorder(&[0, 1, 2, 3], 6, 2, 2).is_empty());
    }

    #[test]
    fn drag_right_inserts_and_shifts() {
        let slots = [0, 1, 2, 3, 4, 5];
        let moves = plan_reorder(&slots, 6, 1, 4);
        // park, shift 2,3,4 down, drop
        assert_eq!(moves, vec![(1, 6), (2, 1), (3, 2), (4, 3), (6, 4)]);
        let cell = simulate(&slots, 6, &moves);
        // resulting order by slot: B0,B2,B3,B4,B1,B5 (the dragged B1 now sits at position 4)
        let order: Vec<usize> = (0..=5).map(|s| cell[s].unwrap()).collect();
        assert_eq!(order, vec![0, 2, 3, 4, 1, 5]);
        assert_eq!(cell[6], None); // scratch restored
    }

    #[test]
    fn drag_left_inserts_and_shifts() {
        let slots = [0, 1, 2, 3, 4, 5];
        let moves = plan_reorder(&slots, 9, 4, 1);
        assert_eq!(moves, vec![(4, 9), (3, 4), (2, 3), (1, 2), (9, 1)]);
        let cell = simulate(&slots, 9, &moves);
        let order: Vec<usize> = (0..=5).map(|s| cell[s].unwrap()).collect();
        assert_eq!(order, vec![0, 4, 1, 2, 3, 5]); // dragged B4 now at position 1
    }

    #[test]
    fn works_with_noncontiguous_slots() {
        // blocks at slots 2,3,4,5,6,7 with scratch 8 — the capture's layout.
        let slots = [2, 3, 4, 5, 6, 7];
        let moves = plan_reorder(&slots, 8, 0, 5);
        // each destination must be empty at its turn (validated in simulate)
        let cell = simulate(&slots, 8, &moves);
        let order: Vec<usize> = slots.iter().map(|&s| cell[s].unwrap()).collect();
        assert_eq!(order, vec![1, 2, 3, 4, 5, 0]); // first block bubbled to the end
        assert_eq!(cell[8], None);
    }
}

#[cfg(test)]
mod tests {
    use super::reply_txn;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn reads_txn_from_a_complete_envelope_reply() {
        // read-info reply (startup.pcapng): TLV header + {102:0x3ea, 103:0, 104:{...}}.
        let body = hex(
            "00000600260000008366cd03ea670068866bcd00006ccd00146da9447561\
                        6c20416d700075c35392cd2292005c00",
        );
        assert_eq!(reply_txn(&body), Some(0x03ea));
    }

    #[test]
    fn reads_txn_from_a_truncated_stream_start_reply() {
        // stream-start reply = chunk #0: {102:0x3eb, 103:0, 104:<da 0a1e ...>} where key 104 is a
        // 2590-byte streamed blob — only the head is present, so a full msgpack decode would fail.
        // reply_txn must still recover the txn from the leading 102 entry.
        let body = hex("00001e28290a00008366cd03eb670068da0a1ea96c362d68656c697800");
        assert_eq!(reply_txn(&body), Some(0x03eb));
    }

    #[test]
    fn no_txn_in_a_raw_chunk() {
        // A mid-stream pagination chunk is raw blob bytes, not a {102,…} envelope.
        let body = hex("c093c240c093c240c093c240c093c240");
        assert_eq!(reply_txn(&body), None);
    }
}
