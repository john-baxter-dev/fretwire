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
use fretwire_data::stream::ParamValue;
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
    /// Latched when a heartbeat send times out — the device has stopped draining its OUT endpoint
    /// and will not resume without a power cycle. See [`Session::device_lost`].
    device_lost: bool,
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
    /// The device's own identity as of the last [`Self::read_preset`] — see [`Self::last_identity`].
    last_info: Option<fretwire_data::stream::PresetInfo>,
    /// `(bank, index)` we last asked the device to load via [`Self::goto_preset`], pending
    /// confirmation. Consumed by the next [`Self::read_preset`], which re-reads until the identity
    /// catches up to it. See `read_preset_inner`'s staleness check for why asking twice isn't enough.
    expect_identity: Option<(i64, i64)>,
}

/// One state on the edit-history timeline: the op-21-writable preset blob plus the label of the
/// edit that produced it.
struct HistoryEntry {
    label: String,
    blob: Vec<u8>,
}

/// Edit-history length cap — blobs are ~3 KB, so this bounds history at ~150 KB.
const MAX_HISTORY: usize = 50;

/// Wall-clock ceiling on one preset read (see `read_preset_inner`).
///
/// A healthy read of a full Helix Floor preset takes ~20 ms end to end. This is three orders of
/// magnitude of headroom, so it can only fire on a device that has genuinely stopped answering —
/// which is the point: without it, a wedged-but-still-enumerated pedal costs ~36 chunks × the
/// bulk-IN timeout before we notice, and `read_preset` retries three times on top of that.
const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Read the op (key 100) and transaction (key 102) back out of an edit body we're about to send, so
/// a log line can say which command a reply belongs to. `None` for either field when the body isn't
/// one of the `edit::` builders' maps — the caller degrades to a less specific message.
fn edit_op_txn(body: &[u8]) -> (Option<i64>, Option<u16>) {
    use fretwire_data::stream::{locate_root_where, map_get};
    let Some(root) = locate_root_where(body, 4, |v| map_get(v, edit::K_TXN).is_some()) else {
        return (None, None);
    };
    (
        map_get(&root.value, edit::K_OP).and_then(|v| v.as_i64()),
        map_get(&root.value, edit::K_TXN)
            .and_then(|v| v.as_u64())
            .map(|t| t as u16),
    )
}

/// Render an edit body's **target map** (key 101) as a compact `{98:4, 24:390}` line, for the log
/// entry that reports a refusal.
///
/// A refusal that says only the op number and the device's code cannot be diagnosed at all. A Floor
/// log on 2026-08-02 caught op 40 refused with `-306` and there was no way to learn what it had been
/// asked to swap to — three plausible reconstructions all completed fine on a Stomp. What was sent
/// has to be in the same line as what came back.
fn edit_target_str(body: &[u8]) -> String {
    use fretwire_data::rmpv::Value;
    use fretwire_data::stream::{locate_root_where, map_get};
    fn render(v: &Value) -> String {
        match v {
            Value::Map(m) => {
                let inner: Vec<String> = m
                    .iter()
                    .map(|(k, val)| format!("{k}:{}", render(val)))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            other => format!("{other}"),
        }
    }
    locate_root_where(body, 4, |v| map_get(v, edit::K_TXN).is_some())
        .and_then(|root| map_get(&root.value, edit::K_TARGET).map(render))
        .unwrap_or_else(|| "?".to_string())
}

/// Human name for an edit op id, for error messages the GUI shows verbatim. Unknown ids get a bare
/// "edit" — the numeric op rides alongside it at the call site.
fn op_name(op: i64) -> &'static str {
    match op {
        edit::OP_SET_VALUE => "parameter change",
        edit::OP_BYPASS => "bypass toggle",
        edit::OP_SELECT => "preset change",
        edit::OP_SAVE_PRESET => "save",
        edit::OP_RENAME_PRESET => "rename",
        edit::OP_RENAME_SNAPSHOT => "snapshot rename",
        edit::OP_SWITCH_SNAPSHOT => "snapshot change",
        edit::OP_SWAP_MODEL => "model swap",
        edit::OP_ADD_BLOCK => "block add",
        edit::OP_DELETE_BLOCK => "block delete",
        edit::OP_MOVE_BLOCK => "block move",
        edit::OP_WRITE_PRESET => "preset write",
        edit::OP_SETTING => "setting change",
        _ => "edit",
    }
}

/// A plain-language gloss for the refusal codes we have pinned down, appended in parentheses after
/// the raw number (which stays, so a log still identifies the case).
///
/// `-306` on op 40 is **not enough DSP** [solid — 2026-08-02, HX Stomp]. The same swap (slot 7 →
/// `HD2_DelayCosmosEchoStereo`) is refused with the preset at 71.8% on our meter and accepted at
/// 58.8%, nothing else changed. A ladder of targets brackets the ceiling between a landing total of
/// 74.9% (accepted) and 75.3% (refused) — the meter reads low, so "28% free" can still be full. See
/// `docs/protocol.md`.
///
/// `-3` is the parameter write the block would not take — see the wire-type and key-29 addressing
/// sections of the same doc.
fn reject_hint(op: Option<i64>, code: i64) -> String {
    let hint = match (op, code) {
        (Some(edit::OP_SWAP_MODEL), -306) => Some(
            "not enough DSP for that model — the pedal fills up near 75% on our meter, so free \
             some up by simplifying or removing a block",
        ),
        (_, -3) => Some(
            "the block would not take that write — wrong value type for the parameter, or no \
             parameter at that index",
        ),
        _ => None,
    };
    hint.map_or_else(String::new, |h| format!(" ({h})"))
}

/// Does the identity the device reported (`got`) confirm the one we asked [`Session::goto_preset`]
/// for (`want`)?
///
/// `want == None` means nothing is pending — any identity is fine, including none. Otherwise the
/// bank **and** the index must both match: after a preset change the device can serve the new
/// preset's stream while still reporting the old identity, and either field alone can be the one
/// that lags. See the call site in [`Session::read_preset`] for the log this came from.
fn identity_confirms(
    want: Option<(i64, i64)>,
    got: Option<&fretwire_data::stream::PresetInfo>,
) -> bool {
    match want {
        None => true,
        Some((bank, index)) => got.is_some_and(|g| (g.bank, g.index) == (bank, index)),
    }
}

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
                device_lost: false,
                last_raw: None,
                history: Vec::new(),
                cursor: 0,
                pending: None,
                saved_cursor: Some(0),
                last_info: None,
                expect_identity: None,
            };
            // Clear any frames a previous session left on the wire so the handshake starts aligned.
            s.transport
                .drain_wire(std::time::Duration::from_millis(120), 64);
            match s.handshake() {
                Ok(()) => return Ok(s),
                // A *write* timeout means the pedal never took the bytes. Releasing and retrying
                // cannot fix a stalled OUT endpoint — it only spends another ~6 s of the user's time
                // per attempt — so stop and say what actually clears it.
                Err(crate::Error::Usb(fretwire_usb::Error::WriteTimeout)) => {
                    tracing::error!(
                        attempt,
                        "the pedal is not accepting data — it needs a power cycle"
                    );
                    drop(s);
                    return Err(fretwire_usb::Error::WriteTimeout.into());
                }
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
            self.beat_channel(src, dst, seq, arg)?;
        }
        // Drain the device's queued keepalives/meters so they don't sit in front of the next edit's
        // reply. Short per-frame quiet window; bounded so a chatty device can't stall the tick.
        self.transport
            .drain_wire(std::time::Duration::from_millis(15), 64);
        Ok(())
    }

    /// Send one channel's idle beat, latching [`Self::device_lost`] if the endpoint has stalled.
    ///
    /// A heartbeat send that times out is not a hiccup: the OUT endpoint only stops draining when the
    /// device has stopped reading, and it will not start again without a power cycle. Recording that
    /// lets `close()` skip a teardown handshake nobody is listening to, and lets the caller stop
    /// beating instead of retrying every 250 ms forever.
    fn beat_channel(&mut self, src: u16, dst: u16, seq: u8, arg: u32) -> crate::Result<()> {
        let frame = Frame::new(src, dst, seq, cmd::IDLE, arg, Vec::new());
        match self.transport.send_frame(&frame) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Only a *write* timeout means the pedal stopped taking bytes. A read timeout on a
                // heartbeat is ordinary — the device owes us no reply to an idle frame.
                if matches!(e, fretwire_usb::Error::WriteTimeout) {
                    self.device_lost = true;
                }
                Err(e.into())
            }
        }
    }

    /// Whether the device has stopped accepting frames — latched by a heartbeat send that timed out.
    /// Once true it stays true: only a power cycle clears it, so the caller should tear the session
    /// down rather than keep polling.
    pub fn device_lost(&self) -> bool {
        self.device_lost
    }

    /// Heartbeat **and** collect the device's unsolicited state-pushes (footswitch bypass, panel
    /// snapshot/preset changes) so the editor can live-follow the hardware. Same idle-on-each-channel
    /// beat as [`Self::keepalive`], but the drained status-channel `{105,106}` frames are parsed into
    /// [`StatusPush`] events instead of discarded. Call on the same timer the GUI uses for keepalive.
    pub fn poll_events(&mut self) -> crate::Result<Vec<fretwire_data::stream::StatusPush>> {
        for (src, dst) in [channel::STATUS, channel::EDIT, channel::PRIMARY] {
            let seq = self.next_seq(src);
            let arg = self.cur_arg(src);
            self.beat_channel(src, dst, seq, arg)?;
        }
        let frames = self
            .transport
            .drain_collect(std::time::Duration::from_millis(15), 96);
        // Diagnostic: `FRETWIRE_TRACE_STATUS=1` logs every drained frame body, parsed or not. Off
        // by default because the status channel also carries meters and idles, which would bury a
        // log. On, because we cannot decode a push we never wrote down — the tester has asked twice
        // why turning a knob on the pedal doesn't move the UI, and the answer is that the push for
        // it (if there is one) has never been captured. Turn this on, touch the hardware, and the
        // bytes are in the log.
        let trace = std::env::var_os("FRETWIRE_TRACE_STATUS").is_some();
        let mut pushes = Vec::new();
        for f in &frames {
            let push = fretwire_data::stream::parse_status_push(&f.body);
            if trace {
                tracing::debug!(src = f.src, cmd = f.cmd, arg = f.arg, body = ?f.body, ?push, "status frame");
            } else if let Some(fretwire_data::stream::StatusPush::Other(typ)) = push {
                // A `{105,106}` state mirror whose shape we don't decode yet — rare, and the only
                // thing we would ever want the bytes of. Always worth a line.
                tracing::debug!(typ, body = ?f.body, "undecoded status push");
            }
            if let Some(p) = push {
                pushes.push(p);
            }
        }
        self.reopen_push_window(&frames)?;
        Ok(pushes)
    }

    /// Acknowledge the pushes we just drained, so the device keeps sending them.
    ///
    /// **Without this the status channel goes dead partway into every session.** The device mirrors
    /// panel activity only until ~4 KiB of it is outstanding and then stops: 4075 bytes in three
    /// captures and 4040 in a fourth, from frame counts of 179/191/195/386 — a byte ceiling, not a
    /// timeout (4075 + 21 = 4096, the body of the next frame it declined to send). After that the
    /// channel carries only empty keepalives, and the pedal's footswitches, knobs and preset changes
    /// stop reaching the host entirely until the session is reopened. An *idle* session never
    /// reaches the ceiling, which is why this hid for so long — it only bites a session someone is
    /// actually using, and it is the real reason the editor "stops following the hardware".
    ///
    /// The fix is the same page request the chunked read uses to pull its next window: a `cmd 0x08`
    /// carrying the channel's advanced offset. The device's own `arg` on these frames stays pinned
    /// (at 521), exactly as it does during a paged read, which is what suggested it.
    ///
    /// Status channel only. That is where every measurement was taken and where the pushes live,
    /// and it keeps this off the edit channel — the one that wedges Helix Floors mid-write. In the
    /// verifying capture all 501 requests went to the status channel anyway: the other two never
    /// delivered bytes here, because their reads consume and acknowledge their own frames.
    ///
    /// [solid — 2026-08-02, HX Stomp: 300 s, 1117 mirror frames, **23457 bytes**, pushes still
    /// arriving at 299.9 s against a ceiling of 4075 without it. Refuted on the way: advancing the
    /// idle beat's `arg` without the page request, which changed nothing (4075 → 4040).]
    fn reopen_push_window(&mut self, frames: &[Frame]) -> crate::Result<()> {
        let (src, dst) = channel::STATUS;
        let bytes: u32 = frames
            .iter()
            .filter(|f| f.src == dst)
            .map(|f| f.body.len() as u32)
            .sum();
        if bytes == 0 {
            return Ok(());
        }
        self.advance_arg(src, bytes);
        let seq = self.next_seq(src);
        let arg = self.cur_arg(src);
        self.transport
            .send_frame(&Frame::new(src, dst, seq, cmd::CHUNK, arg, Vec::new()))?;
        Ok(())
    }

    /// Send an edit-command body on the edit channel and wait for the device's ACK, then issue the
    /// `cmd 0x08` follow-up HX Edit sends after each discrete edit (both via [`Self::edit_request`],
    /// so the channel's `arg` offset stays correct). The edit itself rides `cmd 0x04`.
    fn send_edit(&mut self, body: Vec<u8>) -> crate::Result<Frame> {
        // Clear any frames buffered since the last heartbeat so the edit's reply is the next one we
        // read (the device interleaves keepalives/meters on a held session).
        self.transport.drain();
        // Pull the op and transaction back out of the body we're about to send. Every `edit::`
        // builder puts them at keys 100/102, and a log that says only what came *back* is what made
        // the 2026-07-30 rejection take frame-size archaeology to identify.
        let (sent_op, sent_txn) = edit_op_txn(&body);
        // Kept for the rejection log below. These bodies are tens of bytes — the whole-preset write
        // goes through `write_preset`, not here — so the copy costs nothing.
        let edit_body = body.clone();
        let tlv = Tlv::command(op::PARAM_SET, body);
        // Correlate the ACK by the transaction id it echoes, not by "next non-keepalive frame on the
        // channel". The device interleaves empty `cmd 0x08` credit frames and other channels' traffic
        // on the edit channel, and the loose match happily takes one of those as the verdict: across
        // the field logs 86 "ACKs" had an empty body and 50 more echoed a *previous* transaction —
        // op 43 (move-block) never once correlated. Each miss both invents a success the pedal never
        // reported and shifts every later reply by one, so a refusal lands on the wrong command and
        // the mismatch silently suppresses the rejection check below.
        let ack = match sent_txn {
            Some(txn) => self.edit_request_txn(cmd::OPEN, tlv.to_bytes(), txn)?,
            None => self.edit_request(cmd::OPEN, tlv.to_bytes())?,
        };
        tracing::debug!(op = ?sent_op, txn = ?sent_txn, reply = ?ack.body, "edit ACK");
        // The ACK is not always an ack. Key 103 is the reply's kind, and `255` means the device
        // threw the command away — it applies nothing, reports nothing else, and the next read comes
        // back unchanged. We used to log this line and `Ok(())` on top of it, so the GUI cheerfully
        // announced edits the pedal had refused. Only trust the verdict when it is answering the
        // transaction we just sent.
        if let Some((txn, code)) = fretwire_data::stream::parse_edit_rejection(&ack.body)
            && sent_txn.is_none_or(|s| s == txn)
        {
            let what = sent_op.map_or_else(
                || "command".to_string(),
                |o| format!("{} (op {o})", op_name(o)),
            );
            tracing::warn!(
                op = ?sent_op,
                txn,
                code,
                target = %edit_target_str(&edit_body),
                "the pedal rejected the edit"
            );
            return Err(crate::Error::Rejected(format!(
                "{what} — device code {code}{}",
                reject_hint(sent_op, code)
            )));
        }
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

    /// Reject a `(bank, slot)` pair the device cannot address, **before** it reaches the wire.
    ///
    /// This exists because a real incident: the preset-list browse numbers presets globally
    /// (`bank * setlist_size + slot`) while these commands take the bank-relative slot, and the GUI
    /// passed the global number straight through. Selecting a TEMPLATES preset therefore sent
    /// `goto_preset(bank = 7, preset = 906)` — far past the end of a 128-slot setlist — and **locked
    /// the device up hard enough to need a reboot**. An out-of-range slot must never be sent again:
    /// there is no reason to believe the firmware handles one gracefully, and `save_preset` with a
    /// bogus slot would be a persistent write to who-knows-where.
    /// The device's own reported identity as of the last [`Self::read_preset`] — bank, slot and
    /// name, straight from the op-23 reply. `None` before the first read.
    ///
    /// This is the authority on *which setlist the pedal is actually in*, which a caller needs to
    /// tell an in-place overwrite from a write into a different setlist. Note it is only as fresh
    /// as the last read, and that the op-23 identity lags a preset change by one read (see
    /// `docs/protocol.md`) — `read_preset` resolves that lag before this is set, but a caller
    /// making a **destructive** decision on it should still be reading first.
    pub fn last_identity(&self) -> Option<&fretwire_data::stream::PresetInfo> {
        self.last_info.as_ref()
    }

    fn check_preset_addr(&self, bank: i64, slot: i64, what: &str) -> crate::Result<()> {
        let d = self.device();
        let banks = d.setlist_names().len() as i64;
        let stride = d.setlist_stride();
        if bank < 0 || bank >= banks {
            return Err(fretwire_data::Error::Stream(format!(
                "{what}: bank {bank} out of range (device has {banks} setlist(s))"
            ))
            .into());
        }
        if slot < 0 || slot >= stride {
            return Err(fretwire_data::Error::Stream(format!(
                "{what}: preset slot {slot} out of range 0..{stride} for bank {bank} — this looks \
                 like a global preset number ({}) rather than a slot within the setlist",
                bank * stride + slot
            ))
            .into());
        }
        Ok(())
    }

    /// Navigate the device to `preset` in `bank` (op 20 SELECT). **Changes the active preset** —
    /// this is the destructive counterpart to `read_preset`. Rides the edit channel like an edit.
    pub fn goto_preset(&mut self, bank: i64, preset: i64) -> crate::Result<()> {
        self.check_preset_addr(bank, preset, "goto_preset")?;
        let txn = self.bump_txn();
        let body = edit::select_preset(bank, preset, txn);
        self.send_edit(body)?;
        // Remember what we asked for: the identity the device reports after a switch can lag its own
        // edit buffer by longer than a read takes, and we can only tell because we know the answer.
        self.expect_identity = Some((bank, preset));
        Ok(())
    }

    /// Save the current edit buffer to a preset slot (op 71). **Persistent write — overwrites
    /// `slot` in device flash.** `bank` is normally 0; `slot` is the flat preset index (as `goto`
    /// and `list_presets` use). `name` is stored NUL-terminated. Rides the edit channel like an edit.
    pub fn save_preset(&mut self, bank: i64, slot: i64, name: &str) -> crate::Result<()> {
        self.check_preset_addr(bank, slot, "save_preset")?;
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
        self.check_preset_addr(bank, slot, "rename_preset")?;
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
    ///
    /// **A paired add is two commands, not one.** Op 39 carrying a cab is refused outright by the
    /// device (`{103:255, 104:{111:-21}}`), so the pair is realized the way HX Edit does it: add the
    /// amp bare, then op-40 the cab onto it — and op 40 with a paired index is the byte-exact path
    /// the capture tests cover. [solid — 2026-07-30 Floor log: two paired adds refused, nothing
    /// applied; the tester fell back to adding the amp and a cab as separate blocks. Verified on
    /// hardware 2026-07-31: the two-command form lands both blocks, paired.]
    pub fn add_block(
        &mut self,
        slot: i64,
        model_index: i64,
        paired_index: i64,
    ) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body = edit::add_block(slot, model_index, -1, txn);
        self.send_edit(body)?;
        if paired_index >= 0 {
            self.swap_model(slot, model_index, paired_index)?;
        }
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
    /// chunked-read in reverse. The device answers each **512-byte unit** with an empty `cmd 0x08`
    /// frame — **a flow-control credit.** We wait a real timeout for each one, because outrunning the
    /// device is what wedged it back when we fired all fourteen chunks with a 5 ms glance between
    /// them. A unit is two frames, 496 + 16, so that it ends on a short USB packet; see the loop.
    ///
    /// **Three consecutive unanswered chunks means the pedal is gone, and this was tested.** For
    /// half a day the guard was removed on the theory that it might be aborting writes that would
    /// otherwise have finished — `fretwire24.log` had shown the same Floor complete a 14-chunk write
    /// of the same preset, so the credit stall looked like something a transfer might recover from.
    /// `fretwire26.log` is that experiment: with nothing stopping it, the write pushed on past the
    /// three quiet chunks and the very next send timed out — the device had stopped draining its
    /// endpoint entirely. It does not recover. Aborting is not what wedges the pedal, and it is
    /// already wedged by the time we notice.
    ///
    /// So the guard is back, purely as a faster and more legible failure: it reports
    /// `sent`/`total`/`credits` after ~0.75 s instead of surfacing a bare USB write timeout after
    /// 2 s. It never fires on a healthy transfer — an HX Stomp credits every chunk (1,2,4,5,6 on a
    /// 5-chunk write, unchanged across 90 reads of channel history), and the one Floor write on
    /// record that completed credited all 14.
    ///
    /// **A doomed write is doomed by chunk three, and the device is never outrun.** Over all 21
    /// recorded Floor writes the credit count is the whole story: the thirteen that wedged received
    /// **2 or 3 credits and not one more** — chunk 3 is never credited, in any of them — while the
    /// eight that completed were credited at every single chunk, climbing to 14–19 with `silent`
    /// never once reaching 1. So the device does not degrade across the transfer and is not being
    /// pushed too hard; it stops dead after two or three chunks, and everything we send afterwards
    /// goes into an endpoint that has already stopped. That is why the tester always reports the
    /// same "2480 of N bytes": 2480 is `MAX_SILENT_CHUNKS`'s stop point, not the device's.
    ///
    /// The first chunk's credit latency is a good tell but not a rule — 4–8 ms on all eight
    /// completed writes and 32–198 ms on twelve of the thirteen wedged ones, with `fretwire24`'s
    /// third write the exception: credited in 3 ms, then dead after the second. `first_credit_ms`
    /// on the summary line records it for future reports; the credit ceiling is the reliable one.
    ///
    /// Nothing about the blob explains which writes wedge. The same paste of the same 6883 bytes
    /// wedged the pedal and then, after a power cycle, completed 43 seconds later in the same GUI
    /// session (`fretwire35`); `fretwire24` wedged, recovered across a power cycle, completed, and
    /// wedged again 56 s later on the same preset. Preset size doesn't separate the groups either.
    /// Whatever the state is, it is on the device and invisible from here — settling it needs the
    /// bytes of a stalling write and a succeeding one side by side, which `FRETWIRE_DUMP_WRITES`
    /// collects.
    ///
    /// [solid — 2026-08-01, `fretwire12`/`17`/`22b`/`23`/`24`/`26`/`27`/`30`/`32`/`33`/`35`: 21
    /// Floor writes, 13 wedged. **Open:** what device state stops it consuming. Refuted along the
    /// way: that the abort causes it (`fretwire26`), and that the edit channel's `arg` offset
    /// drives it (see `write_preset`'s body).]
    ///
    /// LIVE: the exact chunk size and `arg` cadence are reconstructed from `move_EQ_right_two_slots`.
    /// The device's `{103:1}` apply-ACK is best-effort (logged, not required); the caller confirms by
    /// re-reading.
    pub fn write_preset(&mut self, blob: Vec<u8>) -> crate::Result<()> {
        /// Payload bytes per flow-control credit — HX Edit's unit, and it is **512, not 496**.
        const UNIT: usize = 512;
        /// Biggest body that still fits one 512-byte bulk packet once the 16-byte frame header is
        /// added. A `UNIT` therefore goes out as two frames, 496 + 16.
        const FRAME_BODY: usize = 496;
        /// How long to wait for a chunk's flow-control credit before counting it missing. Generous
        /// next to the ~7 ms a healthy chunk round-trips in: the point is to let a busy device catch
        /// up, since outrunning it is what wedges it.
        const CREDIT_WAIT: std::time::Duration = std::time::Duration::from_millis(250);
        /// Wall-clock ceiling on the whole transfer. A pure backstop against looping forever — each
        /// individual send is already bounded by `fretwire_usb::WRITE_TIMEOUT`, so this only fires
        /// if the device keeps taking bytes but never finishes. Generous: 14 chunks that each wait
        /// out `CREDIT_WAIT` is ~3.5 s, so 30 s cannot be reached by mere slowness.
        const WRITE_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);
        /// Consecutive unanswered chunks before we call the device wedged and stop.
        const MAX_SILENT_CHUNKS: usize = 3;

        let txn = self.bump_txn();
        let tlv = Tlv::command(op::PARAM_SET, edit::write_preset(&blob, txn)).to_bytes();
        let (src, dst) = channel::EDIT;

        // Diagnostic: `FRETWIRE_DUMP_WRITES=<dir>` saves the exact blob we are about to send before
        // the first byte goes out, so a write that wedges the pedal can be reproduced offline from
        // the bytes that did it. Off unless the variable is set; failures here never block the write.
        if let Some(dir) = std::env::var_os("FRETWIRE_DUMP_WRITES") {
            let dir = std::path::PathBuf::from(dir);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let path = dir.join(format!("write-{stamp}-txn{txn}.bin"));
            match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, &blob)) {
                Ok(()) => tracing::info!(path = %path.display(), bytes = blob.len(),
                    "dumped the op-21 blob before sending"),
                Err(e) => tracing::warn!("could not dump the op-21 blob: {e}"),
            }
        }

        self.transport.drain();
        // arg stays at the channel cursor for the whole transfer (the capture barely advances it,
        // and small edits via `send_edit` don't advance per frame). LIVE: advance per chunk if the
        // device rejects a stalled offset.
        //
        // This field was the leading suspect for the Floor lockup and it is **not the cause**. The
        // cursor counts bytes received on the channel, so it climbs ~7 KB per preset read, and
        // across the first six recorded writes it separated the outcomes perfectly (completed low,
        // wedged high). A `FRETWIRE_WRITE_ARG` override pinned it to 0 and then 1 for eight more
        // writes: three completed and five wedged anyway, and one of the wedged ones started at a
        // *lower* cursor (29397) than a write that completed (50635). The split was session age
        // wearing the cursor as a costume. [solid — 2026-08-01, `fretwire30`/`32`/`33`/`35`]
        let arg = self.cur_arg(src);
        let total = tlv.len();
        let chunks = tlv.len().div_ceil(UNIT);
        let started = std::time::Instant::now();
        let mut sent = 0usize;
        let mut credits = 0usize;
        let mut silent = 0usize;
        // How long the *first* chunk's credit took. On this device that single number predicts the
        // whole transfer (see the doc comment), so every write reports it whether it succeeds or
        // not — it is the one field a bug report needs and the cheapest one to collect.
        let mut first_credit_ms = 0u128;
        for (n, unit) in tlv.chunks(UNIT).enumerate() {
            // **Two frames per unit, and the second one is what makes this work.** A 496-byte body
            // plus the 16-byte header is exactly 512 bytes — the bulk endpoint's `wMaxPacketSize`.
            // Sending only 496-byte bodies, as we did, means every packet is a maximum-size packet
            // and the device never sees the short packet that ends a USB bulk transfer. HX Edit
            // never does that: in both captures that carry a bulk upload it splits each 512-byte
            // unit into 496 + 16, so every unit ends on a 32-byte packet, and only then does the
            // credit come back — `move_EQ_right_two_slots` (op 21: 496,16,496,16,496,16,8,496,8,8,
            // 496,16,423 for a 2991-byte TLV) and `import_ir` (fifteen 496+16 pairs). Same 512
            // payload bytes per credit either way; the difference is purely how they are packetised.
            //
            // That is the shape of the Floor lockup: it takes two or three units and then stops
            // draining its endpoint, which is what a device does when its receive path is waiting
            // for a transfer that never terminates. It also explains why the blob never mattered
            // and why session age looked like a predictor — how much slack the endpoint had left.
            // [hypothesis — 2026-08-02. Mechanism and captures agree; a Floor confirms it.]
            for part in unit.chunks(FRAME_BODY) {
                let seq = self.next_seq(src);
                self.transport.send_frame(&Frame::new(
                    src,
                    dst,
                    seq,
                    cmd::OPEN,
                    arg,
                    part.to_vec(),
                ))?;
                sent += part.len();
            }
            // Block for this chunk's credit, then sweep up anything else already queued so the
            // device's backlog doesn't accumulate. Waiting for the *first* frame is what paces us;
            // the second call only mops up and must not add latency.
            let waited = std::time::Instant::now();
            let got = self.transport.drain_collect(CREDIT_WAIT, 1).len()
                + self
                    .transport
                    .drain_collect(std::time::Duration::from_millis(2), 8)
                    .len();
            if n == 0 {
                first_credit_ms = waited.elapsed().as_millis();
            }
            credits += got;
            // Count *consecutive* unanswered chunks, not the running total. The device batches its
            // credits — a healthy transfer can be several behind and catch up in one sweep — so a
            // cumulative deficit says "busy" as often as it says "dead". Going quiet and staying
            // quiet is the signature that actually distinguishes them.
            silent = if got == 0 { silent + 1 } else { 0 };
            tracing::debug!(
                arg,
                len = unit.len(),
                sent,
                total,
                credits,
                silent,
                "write-preset chunk"
            );
            // Only worth stopping while there is still data to withhold: past the last chunk the
            // blob is already in the device and quitting would just deny it the terminator.
            if silent >= MAX_SILENT_CHUNKS && n + 1 < chunks {
                tracing::error!(
                    sent,
                    total,
                    credits,
                    first_credit_ms,
                    chunks = n + 1,
                    "device stopped acknowledging mid-write — abandoning the transfer"
                );
                self.last_raw = None;
                return Err(crate::Error::WriteStalled(format!(
                    "the pedal stopped responding {sent} of {total} bytes into a preset write \
                     (no reply to the last {silent} frames). The edit buffer may be inconsistent — \
                     reload the preset, and power-cycle the pedal if it is unresponsive"
                )));
            }
            if started.elapsed() > WRITE_BUDGET {
                tracing::error!(
                    sent,
                    total,
                    credits,
                    first_credit_ms,
                    chunks = n + 1,
                    "preset write exceeded its wall-clock budget — abandoning the transfer"
                );
                // The edit buffer holds a partial preset now, so the read cache no longer describes
                // the device. Drop it: the next read has to come off the wire.
                self.last_raw = None;
                return Err(crate::Error::WriteStalled(format!(
                    "a preset write ran past {WRITE_BUDGET:?} with {sent} of {total} bytes sent. \
                     The edit buffer may be inconsistent — reload the preset, and power-cycle \
                     the pedal if it is unresponsive"
                )));
            }
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
        tracing::info!(bytes = total, acked, first_credit_ms, "write-preset sent");
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
        self.pending = Some(label.to_string());
    }

    /// Abandon the edit [`Self::edit_begin`] opened, leaving the timeline exactly as it was. Call
    /// this when the edit failed — a refusal from the pedal, or a read that wouldn't settle.
    ///
    /// This matters more than it used to: until edit ACKs were correlated by transaction id, a
    /// refused edit usually arrived as some other frame and was reported as success, so the failure
    /// path here was rarely taken. Now that refusals actually surface, every one of them would
    /// otherwise leave a stale `pending` label behind.
    pub fn edit_abort(&mut self) {
        self.pending = None;
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
        // Discard the redo branch here rather than in `edit_begin`, so an edit that fails between
        // the two leaves the timeline untouched instead of eating the user's redo stack.
        self.history.truncate(self.cursor + 1);
        // If the flash-saved state lived in the discarded redo branch, no cursor matches it now.
        if self.saved_cursor.is_some_and(|i| i > self.cursor) {
            self.saved_cursor = None;
        }
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
        // Structural range only: the split stays left of the mixer, the mixer right of the split,
        // and both within the 8-wide grid (column 9 — just past the last — is the mixer's far
        // right). Enclosing the occupied B row is **not** enforced.
        //
        // It used to be, and that guard was ours, not the device's. Op 43 will move a loop block
        // clean out past the mixer column, and the pedal saves it and plays it —
        // `somehinged3_var1.bin` is a Floor preset with the mixer before column 3 and both loop
        // blocks at columns 3 and 4. Refusing to move a node into the same arrangement the device
        // reaches by another route blocked the tester three times in one evening (the mixer between
        // blocks 1 and 2, twice; the split after block 3). Warn and send it. [2026-08-02]
        let (lo, hi) = if kind == slot_kind::SPLIT {
            (1, other - 1)
        } else {
            (other + 1, 9)
        };
        if pos < lo || pos > hi {
            return Err(fretwire_data::Error::Stream(format!(
                "node position {pos} out of range {lo}..={hi} (the split stays left of the mixer)"
            ))
            .into());
        }
        let strays: Vec<i64> = b_cols
            .iter()
            .copied()
            .filter(|&c| {
                let (sp, mp) = if kind == slot_kind::SPLIT {
                    (pos, other)
                } else {
                    (other, pos)
                };
                c < sp || c >= mp
            })
            .collect();
        if !strays.is_empty() {
            tracing::warn!(
                ?strays,
                pos,
                kind,
                "moving this node leaves row-B blocks outside the bracket — the device accepts \
                 that, but say so in case it turns out to matter"
            );
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

    /// Clamp `value` into the range the reference data declares for this param, when it declares
    /// one. Params we have no metadata for pass through unchanged — there is nothing to clamp to.
    ///
    /// This sits here, immediately above the wire, rather than in the callers, because **the device
    /// does not range-check what it is sent**. A `Heads 1-2` selector (a 0..=3 enum on the legacy
    /// DL4 delays) driven to 77 by a slider that had fallen back to a 0..=127 span wedged a Helix
    /// Floor hard enough that it dropped off USB mid-session and needed a power cycle.
    /// [solid — 2026-07-30 Floor session, `Massif`]
    ///
    /// The metadata miss that produced that particular 77 is fixed in `editor::param_meta_from`,
    /// but a guard that only holds while every model resolves is not a guard: this one costs a
    /// decode we already pay for labeling, and bounds the damage from the next gap in the data.
    /// **Caveat:** the range comes from the *cached* preset, so a caller that changes a block's
    /// model and then writes its params without re-reading will be clamped against the model that
    /// used to be there. `swap_model` does not refresh the cache (the GUI's re-read normally does),
    /// so any swap-then-set sequence inside one operation has to read in between.
    fn clamp_param(&self, slot: i64, paired: bool, param_index: i64, value: f64) -> f64 {
        let meta = self
            .last_raw
            .as_ref()
            .and_then(|raw| self.catalog.load_preset(raw).ok())
            .and_then(|p| {
                p.block(slot).and_then(|b| {
                    let params = if paired { &b.paired_params } else { &b.params };
                    params
                        .iter()
                        .find(|q| q.index as i64 == param_index)
                        .map(|q| q.meta.clone())
                })
            });
        let Some(meta) = meta else { return value };
        let (Some(min), Some(max)) = (meta.min, meta.max) else {
            return value;
        };
        let clamped = value.clamp(min.min(max), max.max(min));
        if clamped != value {
            tracing::warn!(
                slot,
                paired,
                param_index,
                requested = value,
                sent = clamped,
                min,
                max,
                "parameter value out of the model's declared range — clamping before sending"
            );
        }
        clamped
    }

    /// Set a knob/continuous parameter by its index in the model's device param order.
    pub fn set_param(&mut self, slot: i64, param_index: i64, value: f32) -> crate::Result<()> {
        self.ensure_blob();
        // The four split models are the only ones in the whole catalog that carry a `bypass`
        // *parameter*, and the device will not write it with op 30 — it answers `{103:255,
        // 104:{111:-3}}` and applies nothing. Bypass has its own op; send that instead.
        // [solid — 2026-07-31: two op-30 writes to a Split Y's bypass, both refused with code -3]
        if self.param_is_split_bypass(slot, param_index) {
            // Param semantics are "bypassed"; `set_enabled` takes the opposite.
            return self.set_enabled(slot, value < 0.5);
        }
        // Same shape of mistake, one layer down: a switch takes a bool on the wire and refuses a
        // float with the same `-3`. See [`Self::param_is_bool`].
        let is_bool = self.param_is_bool(slot, false, param_index);
        let wire = if is_bool {
            EditValue::Bool(value >= 0.5)
        } else {
            EditValue::Float(value)
        };
        if self.send_if_extra(slot, false, param_index, wire)? {
            return Ok(());
        }
        if is_bool {
            return self.set_param_bool(slot, false, param_index, value >= 0.5);
        }
        let value = self.clamp_param(slot, false, param_index, value as f64) as f32;
        let txn = self.bump_txn();
        let body = edit::set_value(slot, param_index, value, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Whether `(slot, param_index)` names the `bypass` pseudo-parameter that only the split models
    /// carry. Deliberately scoped to the structural split/mixer nodes: no ordinary effect model in
    /// the catalog has a `bypass` param, so this can never divert a real parameter write.
    fn param_is_split_bypass(&self, slot: i64, param_index: i64) -> bool {
        let Some(raw) = self.last_raw.as_ref() else {
            return false;
        };
        let Ok(p) = self.catalog.load_preset(raw) else {
            return false;
        };
        let is_bypass = |b: &crate::EditorBlock| {
            b.slot == slot
                && b.params
                    .iter()
                    .any(|q| q.index as i64 == param_index && q.name.eq_ignore_ascii_case("bypass"))
        };
        p.dsps.iter().any(|d| {
            d.split_node.as_ref().is_some_and(&is_bypass)
                || d.mixer_node.as_ref().is_some_and(&is_bypass)
        })
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
        self.ensure_blob();
        let is_bool = self.param_is_bool(slot, true, param_index);
        let wire = if is_bool {
            EditValue::Bool(value >= 0.5)
        } else {
            EditValue::Float(value)
        };
        if self.send_if_extra(slot, true, param_index, wire)? {
            return Ok(());
        }
        if is_bool {
            return self.set_param_bool(slot, true, param_index, value >= 0.5);
        }
        let value = self.clamp_param(slot, true, param_index, value as f64) as f32;
        let txn = self.bump_txn();
        let body = edit::set_paired_value(slot, param_index, value, txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Set a parameter to a [`ParamValue`] as read back off the device, dispatching on its type.
    ///
    /// The typed setters each hard-code one wire type, which is right when the caller knows what it
    /// is editing (a knob, an enum selector). Replaying a *captured* block is the other case: its
    /// params are a mix of floats, enum ints and bools, and sending a bool as a float is how you
    /// turn `TempoSync1 = false` into `0.0` and get an edit the pedal either refuses or misapplies.
    pub fn set_param_value(
        &mut self,
        slot: i64,
        paired: bool,
        param_index: i64,
        value: ParamValue,
    ) -> crate::Result<()> {
        match value {
            ParamValue::Float(v) if paired => self.set_paired_param(slot, param_index, v),
            ParamValue::Float(v) => self.set_param(slot, param_index, v),
            ParamValue::Int(v) => self.set_param_enum(slot, paired, param_index, v),
            ParamValue::Bool(v) => self.set_param_bool(slot, paired, param_index, v),
        }
    }

    /// Send a **bool** parameter as a MessagePack bool — the only wire type the device accepts for
    /// a switch. See [`Self::param_is_bool`] for why the other setters route here.
    pub fn set_param_bool(
        &mut self,
        slot: i64,
        paired: bool,
        param_index: i64,
        value: bool,
    ) -> crate::Result<()> {
        let txn = self.bump_txn();
        let model_sel = if paired {
            edit::MODEL_PAIRED
        } else {
            edit::MODEL_MAIN
        };
        let body = edit::set_value_on(slot, model_sel, param_index, EditValue::Bool(value), txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Set a block's **Trails** switch — the delay/reverb tail that keeps ringing after the block
    /// is bypassed or the preset changes.
    ///
    /// Trails is the one value these blocks carry past the end of their symbol's param list, so it
    /// has no ordinary param index and op 30 refuses every write addressed by one. HX Edit reaches
    /// it by flipping target key 29 to `false`, which switches key 28 to indexing the block's extra
    /// values instead — and there Trails is `0`. See [`edit::set_value_flagged`].
    pub fn set_trails(&mut self, slot: i64, on: bool) -> crate::Result<()> {
        let txn = self.bump_txn();
        let body =
            edit::set_value_flagged(slot, edit::MODEL_MAIN, false, 0, EditValue::Bool(on), txn);
        self.send_edit(body)?;
        Ok(())
    }

    /// Make sure a preset blob is on hand before an edit that has to know what it is editing —
    /// which parameters are switches, what ranges they declare, which slot holds a split node.
    ///
    /// All of that reads `last_raw`, and a one-shot CLI invocation connects and edits without ever
    /// having read anything, so every such check silently answered "no" and the edit went out with
    /// the wrong wire type. Costs one ~3 KB read, once per session: after the first read (which the
    /// GUI does at connect) this is free. Best-effort — a failure here must not fail the edit.
    fn ensure_blob(&mut self) {
        if self.last_raw.is_none()
            && let Err(e) = self.read_preset_raw()
        {
            tracing::debug!(error = %e, "no preset blob for the pre-edit checks; sending as asked");
        }
    }

    /// Does this param currently read as a bool? Answered from the **device's own last blob**, not
    /// the reference data, so it works on a clean clone with no `.models` imported.
    ///
    /// A switch has exactly one acceptable wire type. Confirmed on hardware (HX Stomp, fw 3.71,
    /// 2026-08-02): `TempoSync1` takes `Bool(true)` and refuses both `Int(1)` and `Float(1.0)` with
    /// device code `-3`. The typed setters below each hard-code a type, so a caller that guesses
    /// wrong gets a guaranteed refusal — the GUI's switch control routed through
    /// [`Self::set_param_enum`] and so could never toggle anything.
    fn param_is_bool(&self, slot: i64, paired: bool, param_index: i64) -> bool {
        self.param_meta_of(slot, paired, param_index)
            .is_some_and(|(is_bool, _)| is_bool)
    }

    /// `(is_bool, extra_index)` for a param, read out of the device's own last blob. `extra_index`
    /// is `Some` for a value past the model's symbol list, which op 30 reaches through key 29
    /// `false` rather than by param index — see [`EditorParam::extra_index`].
    fn param_meta_of(
        &self,
        slot: i64,
        paired: bool,
        param_index: i64,
    ) -> Option<(bool, Option<i64>)> {
        self.last_raw
            .as_ref()
            .and_then(|raw| self.catalog.load_preset(raw).ok())
            .and_then(|p| {
                p.block(slot).and_then(|b| {
                    let params = if paired { &b.paired_params } else { &b.params };
                    params
                        .iter()
                        .find(|q| q.index as i64 == param_index)
                        .map(|q| (matches!(q.value, ParamValue::Bool(_)), q.extra_index))
                })
            })
    }

    /// Send a param that lives past the model's symbol list, if this is one. Returns `true` when it
    /// handled the write, so the ordinary setters can fall through when it isn't.
    fn send_if_extra(
        &mut self,
        slot: i64,
        paired: bool,
        param_index: i64,
        value: EditValue,
    ) -> crate::Result<bool> {
        let Some((_, Some(extra))) = self.param_meta_of(slot, paired, param_index) else {
            return Ok(false);
        };
        let txn = self.bump_txn();
        let model_sel = if paired {
            edit::MODEL_PAIRED
        } else {
            edit::MODEL_MAIN
        };
        let body = edit::set_value_flagged(slot, model_sel, false, extra, value, txn);
        self.send_edit(body)?;
        Ok(true)
    }

    /// Set an **integer/enum** parameter (e.g. the cab `Mic` selector) by its param index. `paired`
    /// targets the block's cab/IR sub-model (`26:1`) rather than the main model. The value is the
    /// option index, sent on the wire as an int (not a float).
    ///
    /// A param the blob reports as a **bool** is redirected to [`Self::set_param_bool`]: an int is
    /// refused there, and a `0`/`1` from a switch is unambiguous.
    pub fn set_param_enum(
        &mut self,
        slot: i64,
        paired: bool,
        param_index: i64,
        value: i64,
    ) -> crate::Result<()> {
        self.ensure_blob();
        let is_bool = self.param_is_bool(slot, paired, param_index);
        let wire = if is_bool {
            EditValue::Bool(value != 0)
        } else {
            EditValue::Int(value)
        };
        if self.send_if_extra(slot, paired, param_index, wire)? {
            return Ok(());
        }
        if is_bool {
            return self.set_param_bool(slot, paired, param_index, value != 0);
        }
        let value = self
            .clamp_param(slot, paired, param_index, value as f64)
            .round() as i64;
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
        // decode fails, back off and re-read; transient interleaving clears on the retry.
        // Consume any pending `goto_preset` expectation: this read is the one that confirms it, and
        // taking it here bounds the extra re-reads to a single call. If the user moves the pedal by
        // hand in the meantime the expectation simply never matches, costs this one read its retries,
        // and is gone.
        let expect = self.expect_identity.take();
        let mut last_err: Option<crate::Error> = None;
        for attempt in 0..3 {
            match self.read_preset_inner() {
                Ok((payload, info, settled)) => {
                    // Two ways the blob and the identity can disagree. `settled == false` means the
                    // identity moved *across* the stream, so the blob belongs to neither preset.
                    // `stale` is the other direction, and the one a field log caught: after a preset
                    // change the device serves the **new** preset's stream while still reporting the
                    // **old** identity — before *and* after the stream, so asking twice can't see it.
                    // Only the address we asked `goto_preset` for can. [solid — 2026-07-30 Floor log:
                    // a 8118-byte Pull Me Under stream reported as `WATERS IN HELL #56` on both
                    // reads, correct 370 ms later]
                    let stale = !identity_confirms(expect, info.as_ref());
                    // Re-read rather than decode it — but only while attempts remain, so a device
                    // the user is actively scrolling still yields *something* rather than an error.
                    if (!settled || stale) && attempt < 2 {
                        tracing::debug!(
                            attempt,
                            settled,
                            stale,
                            want = ?expect,
                            got = ?info.as_ref().map(|i| (i.bank, i.index)),
                            "preset identity doesn't match the blob yet; re-reading"
                        );
                        self.backoff_before_retry(attempt);
                        continue;
                    }
                    match self.catalog.load_preset(&payload) {
                        Ok(mut preset) => {
                            // The blob's `10 → 8` is the snapshot that was *stored* with the preset, not
                            // the one the pedal is on: an HX Stomp parked on SNAPSHOT 3 reported 0.
                            // The read-info reply's key 92 *is* the live value (same key the snapshot
                            // status-push uses), so prefer it and keep the stored one only as a
                            // fallback for offline decodes. See docs/protocol.md.
                            if let Some(live) = info.as_ref().and_then(|i| i.snapshot) {
                                if preset.active_snapshot != Some(live) {
                                    tracing::debug!(
                                        stored = ?preset.active_snapshot,
                                        live,
                                        "preset blob's stored snapshot disagrees with the device; using the device's"
                                    );
                                }
                                preset.active_snapshot = Some(live);
                            }
                            self.last_info = info.clone();
                            preset.current = info;
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
                    }
                }
                Err(e) => last_err = Some(e),
            }
            // Log the error itself, not just that there was one: this warn is usually all a remote
            // tester's log gives us, and "failed" alone can't distinguish a decode fault from the
            // device having stopped answering.
            tracing::warn!(
                attempt,
                error = last_err.as_ref().map(|e| e.to_string()).unwrap_or_default(),
                "preset read/decode failed; backing off and retrying"
            );
            self.backoff_before_retry(attempt);
        }
        Err(last_err.expect("loop runs at least once"))
    }

    /// Pause, then clear the wire, before re-attempting a failed or unsettled read.
    ///
    /// The pause is the point. A failed read usually means the device is *busy*, not that a frame
    /// was dropped — loading a preset reconfigures both DSPs, and the tester's Helix Floor stopped
    /// answering entirely during it. Re-issuing immediately (as this did) put a fresh ~7.5 KB
    /// stream request into a unit that was already behind. Backing off first gives it room to
    /// finish; the drain then clears whatever it queued while we waited.
    fn backoff_before_retry(&mut self, attempt: usize) {
        let pause = std::time::Duration::from_millis(match attempt {
            0 => 150,
            1 => 400,
            _ => 800,
        });
        std::thread::sleep(pause);
        self.transport
            .drain_wire(std::time::Duration::from_millis(60), 256);
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
        // Unlike [`Self::read_preset`], which may decode a provenance-ambiguous blob rather than
        // show the user nothing, this one is the input to a **read-modify-write**: every op-21 path
        // (`set_node_pos`, `delete_block`, `reorder_block`, `move_block_to_row`, `insert_block`)
        // reads here, edits the tree, and writes it straight back to whatever preset the device is
        // sitting on now. If the identity moved across the read, the blob belongs to neither preset
        // and writing it back overwrites the current one with someone else's signal chain — or with
        // an empty one. So retry, and fail rather than guess. [2026-08-01: 21 "provenance is
        // ambiguous" warnings across the field logs, and a `fretwireTest3` resave that came back
        // with no blocks at all.]
        for attempt in 0..3 {
            let (raw, info, settled) = self.read_preset_inner()?;
            if settled {
                self.last_raw = Some(raw.clone());
                // A settled read establishes the identity as firmly as `read_preset` does, so
                // publish it: callers that only want the bytes (`dump-raw`) can still say which
                // preset they got, and `last_identity`'s contract is "as fresh as the last read".
                self.last_info = info;
                return Ok(raw);
            }
            tracing::debug!(
                attempt,
                got = ?info.as_ref().map(|i| (i.bank, i.index)),
                "raw read straddled a preset change; re-reading before any write"
            );
            self.backoff_before_retry(attempt);
        }
        // Don't leave a stale blob behind for a caller that falls back to it.
        self.last_raw = None;
        Err(fretwire_data::Error::Stream(
            "the preset changed under every read attempt — refusing to edit a blob whose \
             provenance is ambiguous (try again once the device settles)"
                .to_string(),
        )
        .into())
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
        let (_, start, _) = self.read_preset_inner()?;

        let mut presets = Vec::with_capacity(total);
        for (done, (index, listed_name)) in listing.iter().enumerate() {
            let index = *index as i64;
            self.goto_preset(0, index)?;
            let (raw, info, _) = self.read_preset_inner()?;
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
    /// Returns `(payload, identity, settled)`. `settled` is false when the device's reported
    /// identity changed between the start and the end of the stream — the blob may then belong to
    /// either preset, so a caller that needs coherent state should re-read.
    ///
    /// The per-chunk decision is [`classify_chunk`].
    fn read_preset_inner(
        &mut self,
    ) -> crate::Result<(Vec<u8>, Option<fretwire_data::stream::PresetInfo>, bool)> {
        // Start aligned: clear any frames left on the wire from a prior edit's fire-and-forget
        // follow-up or, crucially, the device's **unsolicited state pushes** (a footswitch bypass or
        // panel knob/snapshot change). Mid-session those would otherwise be mis-matched as this
        // read's first reply and desync the whole sequence into a bulk-IN timeout. At connect the
        // wire is already quiet, so this just costs one short read.
        let started = std::time::Instant::now();
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
        // then desynced the wire. Live evidence: every truncated read the tester captured landed on a
        // multiple of 256 while every good read ended mid-chunk. With the declared length we skip a
        // premature short/empty chunk and keep reading until the payload is actually whole; the
        // heuristic still governs when the envelope length can't be read (fallback below).
        let mut payload = first.body.clone();
        let full_chunk = first.body.len();
        let target = fretwire_data::stream::declared_stream_len(&first.body);
        let mut empties = 0usize;
        for _ in 0..stream_request_cap(target) {
            if target.is_some_and(|t| payload.len() >= t) {
                break; // whole declared payload is in hand
            }
            // Bound the read in *wall-clock*, not just in chunks. The request cap bounds how many
            // requests we make, but each one can burn the full bulk-IN timeout against a device
            // that is enumerated yet no longer answering — for a ~7.4 KB preset that is ~36 × 3 s,
            // and the tester's freeze measured 121 s inside a single attempt, with three attempts
            // behind it. That is the "GUI froze" report: the pedal was already gone and we spent
            // minutes discovering it one timeout at a time. [solid — 2026-07-30 Floor session]
            if started.elapsed() > READ_DEADLINE {
                return Err(fretwire_data::Error::Stream(format!(
                    "preset read gave up after {:?} with {} of {} bytes — the device stopped \
                     answering mid-stream",
                    started.elapsed(),
                    payload.len(),
                    target.map_or("?".to_string(), |t| t.to_string()),
                ))
                .into());
            }
            let chunk = self.edit_request(cmd::CHUNK, Vec::new())?;
            let n = chunk.body.len();
            tracing::debug!(arg = chunk.arg, body = n, "chunk reply");
            match classify_chunk(n, full_chunk, payload.len(), target) {
                ChunkVerdict::Skip => {
                    empties += 1;
                    // Expected, not alarming: an empty body here is the device's `cmd 0x08`
                    // flow-control credit — the same frame it interleaves during an op-21 write —
                    // landing between two stream chunks. Skipping it is the whole point; the run is
                    // still bounded below in case the device goes quiet for real.
                    tracing::debug!(
                        got = payload.len(),
                        want = target,
                        empties,
                        "credit frame between stream chunks — skipping, continuing read",
                    );
                    // Bound consecutive empties so a wedged device still errors out.
                    if empties >= 8 {
                        break;
                    }
                }
                ChunkVerdict::Keep => {
                    payload.extend_from_slice(&chunk.body);
                    if n < full_chunk {
                        // Also expected: the device sometimes splits one 256-byte chunk across two
                        // frames, and the halves always sum back to 256 (207+49, 46+210, 12+244,
                        // 251+5 in the field logs). Every read that logged this still reassembled
                        // to exactly its declared length, so it is fragmentation, not truncation —
                        // keep it and move on. Kept at debug so it doesn't drown a real anomaly.
                        tracing::debug!(
                            got = payload.len(),
                            want = target,
                            len = n,
                            "stream chunk arrived fragmented — keeping it, continuing read",
                        );
                    } else {
                        empties = 0;
                    }
                }
                ChunkVerdict::Last => {
                    payload.extend_from_slice(&chunk.body);
                    break;
                }
            }
        }

        self.transport.drain(); // clear any batched epilogue frames
        tracing::info!(
            bytes = payload.len(),
            declared = target,
            "reassembled preset stream",
        );

        // A short payload is a failed read, not a preset. Every exit from the loop above except
        // "the declared payload is whole" lands here — the request cap, the consecutive-empties
        // guard — and each one used to fall straight through into the decoder, which then blamed
        // whichever envelope key the missing tail happened to contain: the tester's recurring
        // "envelope key 104 missing or not bytes", raised against a stream that was simply cut off.
        // Erroring here says what actually happened and lets `read_preset`'s retry have another go,
        // which is all a truncated read ever needed. [solid — 2026-08-02, `fretwire39`]
        if let Some(t) = target
            && payload.len() < t
        {
            return Err(fretwire_data::Error::Stream(format!(
                "preset read ended {} bytes short of the declared {t} — the device stopped \
                 answering mid-stream",
                t - payload.len(),
            ))
            .into());
        }

        // Re-ask who we're on. The op-23 identity **lags the blob by one preset**: the first read
        // after a preset change serves the new preset's stream under the *previous* preset's
        // identity. The tester's 2026-07-26 session shows it unambiguously — 19 of the 21 distinct stream
        // lengths were reported under exactly two consecutive identities, and the later of the two
        // is the one every subsequent stable read keeps. Left uncorrected this mislabels the header
        // and, since the snapshot comes from the same reply (key 92), paints the previous preset's
        // active snapshot.
        //
        // Asking again *after* the stream gives a fresher answer without touching the proven
        // open/prep/info/stream sequence. A mismatch means the device moved under us, so the blob's
        // provenance is unknown — report it and let `read_preset` decide. Failure here is
        // non-fatal: fall back to the pre-stream identity rather than turn a good read into an
        // error.
        // Log the outcome either way: this is the one step that adds a wire operation to a sequence
        // that was byte-for-byte HX Edit's, so "it silently fell back" and "it worked" must be
        // distinguishable in a field log.
        let txn = self.bump_txn();
        let after = match self.edit_request_txn(
            cmd::STREAM,
            Tlv::command(op::PARAM_SET, edit::read_info(txn)).to_bytes(),
            txn,
        ) {
            Ok(r) => {
                let parsed = fretwire_data::stream::parse_preset_info(&r.body);
                if parsed.is_none() {
                    tracing::warn!(
                        body = r.body.len(),
                        "post-stream read-info did not parse — falling back to the pre-stream identity"
                    );
                }
                parsed
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "post-stream read-info failed — falling back to the pre-stream identity \
                     (the preset name and active snapshot may lag by one preset change)"
                );
                None
            }
        };
        tracing::debug!(?after, "post-stream read-info reply");
        let settled = match (&preset_info, &after) {
            (Some(before), Some(after))
                if before.index != after.index || before.bank != after.bank =>
            {
                tracing::warn!(
                    before = ?(before.bank, before.index, &before.name),
                    after = ?(after.bank, after.index, &after.name),
                    "preset identity moved across the stream read — blob provenance is ambiguous",
                );
                false
            }
            _ => true,
        };
        // Prefer the post-stream identity when we got one: it is never staler than the pre-stream
        // one, and when they agree the choice is moot.
        Ok((payload, after.or(preset_info), settled))
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
        let payload = self.list_presets_raw(bank)?;
        // The browse numbers presets **globally** (`bank * setlist_size + slot`) — a TEMPLATES
        // (bank 7) listing comes back starting at 896 = 7 × 128 — whereas a preset's own identity
        // (`PresetInfo::index`) is the bank-relative slot, and that is what `goto_preset` /
        // `save_preset` take. Normalise here so callers only ever see one numbering: passing a
        // global index through as a slot is how `goto_preset(7, 906)` reached the device and
        // locked it up.
        let base = bank * self.device().setlist_stride();
        let raw = fretwire_data::stream::parse_preset_list(&payload)?;
        let (out, reordered) = normalise_preset_list(raw, base);
        tracing::debug!(
            bank,
            base,
            n = out.len(),
            reordered,
            "preset list normalised to slots"
        );
        Ok(out)
    }

    /// The **raw reassembled preset-list stream** for `bank`, undecoded. Diagnostic hook: the
    /// browse's index numbering has not fully reconciled with the device's own (a listing has been
    /// seen offset from the same device's `.hxb` backup), and a captured stream is what settles it.
    pub fn list_presets_raw(&mut self, bank: i64) -> crate::Result<Vec<u8>> {
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

        // The list stream carries the same `marker/type/len` prefix as a preset stream — a bank of
        // 128 on a Stomp declares 3259 and reassembles to 3267 — so it gets the same treatment:
        // the declared length decides when the stream is done, a mid-stream empty reply is a credit
        // frame to skip rather than a terminator, a short non-empty chunk is payload, and running
        // out of requests is an error instead of a shorter list. On the old "stop at the first
        // empty or short chunk" rule one interleaved credit frame silently truncated the setlist,
        // and a truncated listing is not a cosmetic problem: the browse indices feed `goto`.
        // [2026-08-02, same failure the preset read hit in `fretwire39`]
        let mut payload = first.body.clone();
        let full_chunk = first.body.len();
        let target = fretwire_data::stream::declared_stream_len(&first.body);
        let mut empties = 0usize;
        for _ in 0..stream_request_cap(target) {
            if target.is_some_and(|t| payload.len() >= t) {
                break;
            }
            let chunk = self.channel_request(chan, cmd::CHUNK, Vec::new())?;
            let n = chunk.body.len();
            tracing::debug!(arg = chunk.arg, body = n, "list chunk");
            match classify_chunk(n, full_chunk, payload.len(), target) {
                ChunkVerdict::Skip => {
                    empties += 1;
                    if empties >= 8 {
                        break;
                    }
                }
                ChunkVerdict::Keep => {
                    payload.extend_from_slice(&chunk.body);
                    if n >= full_chunk {
                        empties = 0;
                    }
                }
                ChunkVerdict::Last => {
                    payload.extend_from_slice(&chunk.body);
                    break;
                }
            }
        }
        self.transport.drain();
        tracing::info!(
            bytes = payload.len(),
            declared = target,
            bank,
            "reassembled preset-list stream"
        );
        if let Some(t) = target
            && payload.len() < t
        {
            return Err(fretwire_data::Error::Stream(format!(
                "preset-list read ended {} bytes short of the declared {t} — the device stopped \
                 answering mid-stream",
                t - payload.len(),
            ))
            .into());
        }
        Ok(payload)
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
        if self.device_lost {
            // Nothing is reading the other end. Each close frame would burn the full write timeout
            // before failing (three channels — six seconds of teardown on a pedal that is going to
            // be power-cycled anyway), and the panel unlocks on the power cycle regardless.
            tracing::info!("skipping session close — the device stopped responding");
            return Ok(());
        }
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

/// How many chunk requests a paginated stream of `target` bytes is allowed to take.
///
/// A pure loop bound, so a garbage declared length or a device that never terminates can't spin
/// forever — the real timeout is `READ_DEADLINE`. It counts **requests**, and it has to be sized
/// against a *fragment* rather than a whole chunk: the device splits chunks as it pleases (207+49,
/// 46+210, 42+172, 84+130 in the field logs), and every split costs one more request for the same
/// payload. The old bound — one request per whole chunk plus eight spare — could not absorb that,
/// and `fretwire39` is what it cost: chunk #0 arrived 214 bytes long, so the cap came out at
/// 7055/214 + 8 = 40, the read fragmented twelve times, and 40 requests fetched 6366 of 7055 bytes.
/// The read then returned that truncated blob as a success and the decoder blamed the envelope.
///
/// 32 bytes is comfortably below the smallest fragment ever recorded (42), which puts a 7 KB preset
/// at 236 requests where a healthy read of it needs 33.
///
/// [solid — 2026-08-02, `fretwire39` Floor session]
fn stream_request_cap(target: Option<usize>) -> usize {
    /// Assumed floor on a productive fragment. Not a wire constant — a bound.
    const MIN_FRAGMENT: usize = 32;
    target.map_or(4096, |t| t / MIN_FRAGMENT + 16)
}

/// What to do with one paginated-stream chunk reply. See [`classify_chunk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkVerdict {
    /// Real payload, and the stream continues.
    Keep,
    /// Real payload, and this reply ends the stream.
    Last,
    /// Not payload — discard it and keep reading.
    Skip,
}

/// Decide what a chunk reply of `n` bytes means, given the chunk size established by chunk #0
/// (`full`), how much payload is already in hand (`have`), and the length the stream's envelope
/// declared (`target`, `None` when it couldn't be read).
///
/// The declared length is the authority for "done". The older rule — "the first chunk shorter than
/// chunk #0 ends the stream" — is only a heuristic: on the Floor an **empty** chunk reply can
/// arrive mid-stream (a batched keepalive/state-push landing in a chunk slot), and treating it as
/// the terminator truncated the payload at an exact chunk boundary and then desynced the wire.
/// Every truncated read the tester captured landed on a multiple of 256 while every good read ended
/// mid-chunk. So an empty reply before the declared end is [`ChunkVerdict::Skip`] — dropped, not
/// appended. A short but *non-empty* chunk is real payload and is always kept; discarding it would
/// silently corrupt the blob. Without a declared length the short-chunk heuristic still governs.
fn classify_chunk(n: usize, full: usize, have: usize, target: Option<usize>) -> ChunkVerdict {
    match target {
        // No declared length: fall back to "a short chunk ends the stream".
        None => {
            if n < full {
                ChunkVerdict::Last
            } else {
                ChunkVerdict::Keep
            }
        }
        Some(t) => {
            if n == 0 && have < t {
                ChunkVerdict::Skip
            } else if have + n >= t {
                ChunkVerdict::Last // the declared payload is now whole
            } else {
                // Still short of the declared length — keep reading, even if this chunk was
                // shorter than chunk #0. With a declared length the short-chunk heuristic is not
                // just unnecessary, it is actively wrong: acting on it here is what truncated
                // reads at an exact chunk boundary.
                ChunkVerdict::Keep
            }
        }
    }
}

/// Turn a browse listing's **global** indices into bank-relative slots, and put the list in slot
/// order. Returns the list and whether the device's own order needed changing.
///
/// Two separate corrections, both load-bearing:
///
/// * **Numbering.** The browse numbers presets globally (`bank * stride + slot`) while a preset's
///   own identity, `goto_preset` and `save_preset` all use the bank-relative slot. Passing a global
///   index through as a slot is how `goto_preset(7, 906)` reached the device and locked it up.
/// * **Order.** The device does not emit the listing in slot order. A preset the user has *moved*
///   keeps its old position in the stream while carrying its new index — the tester's 2026-07-29
///   eight-bank dump emits bank 0's slot 68 at stream position 101 and bank 1's slot 95 at position
///   84, with all 1024 entries otherwise in order. [solid] Callers render this array positionally,
///   so a moved preset would draw in the wrong row under a correct number.
fn normalise_preset_list(raw: Vec<(u16, String)>, base: i64) -> (Vec<(u16, String)>, bool) {
    let mut out: Vec<(u16, String)> = raw
        .into_iter()
        .map(|(global, name)| ((global as i64 - base).max(0) as u16, name))
        .collect();
    let reordered = out.windows(2).any(|w| w[0].0 > w[1].0);
    // Stable, so if the device ever does repeat a slot the duplicates keep their relative order
    // rather than being shuffled arbitrarily.
    out.sort_by_key(|&(slot, _)| slot);
    (out, reordered)
}

#[cfg(test)]
mod preset_list_tests {
    use super::normalise_preset_list;

    fn named(entries: &[(u16, &str)]) -> Vec<(u16, String)> {
        entries
            .iter()
            .map(|(i, n)| (*i, (*n).to_string()))
            .collect()
    }

    #[test]
    fn global_indices_become_bank_relative_slots() {
        // TEMPLATES (bank 7) lists from 896 = 7 × 128.
        let (out, reordered) = normalise_preset_list(
            named(&[(896, "Quick Start"), (897, "Parallel Spans")]),
            7 * 128,
        );
        assert_eq!(out, named(&[(0, "Quick Start"), (1, "Parallel Spans")]));
        assert!(!reordered, "an in-order listing is left alone");
    }

    #[test]
    fn a_moved_preset_is_put_back_in_slot_order() {
        // The shape bank 0 of the tester's Floor really sends: slot 68 arrives late, after 100.
        let (out, reordered) = normalise_preset_list(
            named(&[
                (67, "BMBLFOOT PRINCE"),
                (69, "SHEEHAN PEARCE"),
                (100, "REUTER LEAD"),
                (68, "InSTANtgH0St/24"),
            ]),
            0,
        );
        assert_eq!(
            out,
            named(&[
                (67, "BMBLFOOT PRINCE"),
                (68, "InSTANtgH0St/24"),
                (69, "SHEEHAN PEARCE"),
                (100, "REUTER LEAD"),
            ]),
        );
        assert!(reordered, "the device's order differed and we say so");
    }

    #[test]
    fn an_index_below_the_bank_base_clamps_rather_than_wrapping() {
        // Defensive: `as u16` on a negative would wrap to ~65k and address nothing.
        let (out, _) = normalise_preset_list(named(&[(5, "Stray")]), 7 * 128);
        assert_eq!(out, named(&[(0, "Stray")]));
    }
}

#[cfg(test)]
mod chunk_tests {
    use super::{ChunkVerdict::*, classify_chunk, stream_request_cap};

    const FULL: usize = 256;

    #[test]
    fn a_full_chunk_mid_stream_continues() {
        assert_eq!(classify_chunk(FULL, FULL, 512, Some(7398)), Keep);
    }

    #[test]
    fn the_chunk_that_completes_the_declared_length_ends_the_stream() {
        // 7398 = 28 full chunks (7168) + 230.
        assert_eq!(classify_chunk(230, FULL, 7168, Some(7398)), Last);
    }

    /// The Floor's mid-stream empty replies. Regression: these used to be appended to the payload
    /// *before* being classified as spurious — harmless only because they were zero-length. They
    /// must never contribute bytes, and must not end the stream.
    #[test]
    fn an_empty_chunk_before_the_declared_end_is_dropped_not_appended() {
        // The tester's 2026-07-26 session: got=2560 want=7527, got=3072 want=7508, got=256 want=7193 —
        // every one an exact multiple of the chunk size, i.e. a zero-length reply.
        for (have, want) in [(2560, 7527), (3072, 7508), (4352, 7344), (256, 7193)] {
            assert_eq!(classify_chunk(0, FULL, have, Some(want)), Skip);
        }
    }

    /// The other half of that fix: a short chunk carrying real bytes must be kept — and must not
    /// end the stream — when it lands before the declared end. Dropping it would corrupt the blob;
    /// stopping on it would truncate the blob, which is the original bug the declared length was
    /// introduced to kill.
    #[test]
    fn a_short_nonempty_chunk_before_the_declared_end_keeps_reading() {
        assert_eq!(classify_chunk(100, FULL, 1000, Some(7398)), Keep);
    }

    /// An empty reply that arrives once the declared payload is already whole is the terminator,
    /// not a spurious frame — otherwise the loop would keep asking a finished stream for more.
    #[test]
    fn an_empty_chunk_at_the_declared_end_terminates() {
        assert_eq!(classify_chunk(0, FULL, 7398, Some(7398)), Last);
    }

    #[test]
    fn without_a_declared_length_a_short_chunk_still_ends_the_stream() {
        assert_eq!(classify_chunk(FULL, FULL, 512, None), Keep);
        assert_eq!(classify_chunk(12, FULL, 512, None), Last);
        assert_eq!(classify_chunk(0, FULL, 512, None), Last);
    }

    /// The read that truncated in `fretwire39`, replayed. Every reply the Floor actually sent is
    /// real payload — `classify_chunk` keeps all of them — so nothing about the *decision* was
    /// wrong. The read ran out of **requests**: chunk #0 came back 214 bytes instead of 256, and
    /// the old cap of `declared / chunk_0 + 8` gave 40 slots for a stream that fragmented twelve
    /// times. The 40th reply left 689 bytes still on the device, and the truncated blob went to the
    /// decoder as if it were whole.
    #[test]
    fn a_fragmented_read_does_not_run_out_of_requests() {
        // The reply sizes `fretwire39` logged at 14:10:08, chunk #0 first. 214-byte chunks, split
        // into 42+172 and 84+130 whenever the device felt like it.
        const REPLIES: [usize; 41] = [
            214, 214, 214, 214, 214, 42, 172, 84, 214, 214, 214, 214, 214, 214, 214, 42, 172, 84,
            130, 126, 214, 214, 214, 42, 214, 42, 172, 84, 214, 42, 214, 42, 214, 42, 214, 42, 214,
            42, 214, 42, 214,
        ];
        const DECLARED: usize = 7055;
        let got: usize = REPLIES.iter().sum();
        assert_eq!(got, 6366, "what the device managed to hand over");
        assert!(got < DECLARED, "and it was 689 bytes short");

        // Not one of them was misread — the loop simply stopped asking.
        let mut have = REPLIES[0];
        for &n in &REPLIES[1..] {
            assert_eq!(classify_chunk(n, REPLIES[0], have, Some(DECLARED)), Keep);
            have += n;
        }

        assert_eq!(DECLARED / REPLIES[0] + 8, REPLIES.len() - 1, "the old cap");
        // The new one absorbs the recorded run with room to spare, and would still cover the whole
        // stream arriving as fragments smaller than any the device has ever sent.
        assert!(stream_request_cap(Some(DECLARED)) > REPLIES.len());
        assert!(stream_request_cap(Some(DECLARED)) >= DECLARED.div_ceil(42));
    }
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
    use super::{edit, edit_op_txn, identity_confirms, op_name, reject_hint, reply_txn};
    use fretwire_data::stream::{PresetInfo, parse_edit_rejection};

    fn info(bank: i64, index: i64, name: &str) -> PresetInfo {
        PresetInfo {
            bank,
            index,
            name: name.to_string(),
            snapshot: Some(0),
        }
    }

    /// The longest run of consecutive chunks drawing no flow-control credit — the same rule
    /// `write_preset` applies incrementally, restated over a whole recorded trace so the threshold
    /// can be checked against real transfers.
    fn longest_silence(per_chunk: &[usize]) -> usize {
        let (mut run, mut worst) = (0usize, 0usize);
        for &c in per_chunk {
            run = if c == 0 { run + 1 } else { 0 };
            worst = worst.max(run);
        }
        worst
    }

    #[test]
    fn three_quiet_chunks_separates_the_recorded_writes() {
        // Per-chunk flow-control credits counted off real whole-preset writes: each entry is how
        // many credits arrived in that chunk's window. `write_preset` stops after three consecutive
        // zeroes; these are the traces that threshold has to sit between.

        // An HX Stomp credits essentially every chunk, and its presets fit in five. Measured
        // 2026-08-01 on a 2211-byte write that landed: credits 1,2,4,5,6 cumulative.
        assert_eq!(longest_silence(&[1, 1, 2, 1, 1]), 0);

        // A Helix Floor write that stalls: credits for chunks 1 and 2, then flat. Every 2026-08-01
        // abort traced this and stopped at chunk 5 — `sent=2480`, i.e. 5 × 496, our own constant
        // rather than anything the device chose.
        let floor_stalled = [1, 1, 0, 0, 0];
        assert_eq!(longest_silence(&floor_stalled), 3);

        // And the same 6816-byte preset, same Floor, minutes apart in `fretwire24.log`: all 14
        // chunks credited and the transfer completed. So a large Floor write is possible, and the
        // threshold has to clear this trace by a wide margin or it would abort a healthy one.
        let floor_completed = [3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];
        assert_eq!(floor_completed.len(), 14);
        assert_eq!(longest_silence(&floor_completed), 0);

        // A device that batches hard — a couple of chunks unanswered, then a burst — was never dead
        // either. The even earlier cumulative-deficit rule aborted on exactly this shape
        // (2026-07-31: a 6-chunk Stomp write that had already sent every byte, sent=2688 total=2688).
        assert_eq!(longest_silence(&[1, 0, 0, 2, 0, 0, 3]), 2);

        assert_eq!(longest_silence(&[]), 0);
    }

    #[test]
    fn the_codes_we_have_pinned_down_get_a_plain_language_gloss() {
        // -306 is only ever "out of DSP" on a model swap; the same number from another op means
        // something we have not established, so it stays bare rather than guessing at the user.
        let dsp = reject_hint(Some(edit::OP_SWAP_MODEL), -306);
        assert!(dsp.contains("not enough DSP"), "{dsp}");
        assert_eq!(reject_hint(Some(edit::OP_MOVE_BLOCK), -306), "");
        assert_eq!(reject_hint(Some(edit::OP_SWAP_MODEL), -21), "");

        // -3 is the parameter-write refusal whatever op carries it.
        assert!(reject_hint(Some(edit::OP_SET_VALUE), -3).contains("wrong value type"));
        assert!(reject_hint(None, -3).contains("wrong value type"));
    }

    #[test]
    fn a_refused_edit_is_read_out_of_the_ack_the_device_sends() {
        // Verbatim off the wire (2026-07-30 Floor log): the reply to a paired `add_block`, twice.
        // `{102: 44, 103: 255, 104: {111: -21}}` — the device refused, applied nothing, and said so
        // in the one field we were treating as a don't-care.
        let refused = [
            0, 0, 6, 0, 12, 0, 0, 0, // TLV header
            0x83, 102, 0xcd, 0, 44, 103, 0xcc, 255, 104, 0x81, 111, 0xeb,
        ];
        assert_eq!(parse_edit_rejection(&refused), Some((44, -21)));

        // The two shapes that are *not* refusals, from the same session: a bare ack with 104 nil,
        // and a payload reply echoing the parameter the device just applied.
        let acked = [
            0, 0, 6, 0, 9, 0, 0, 0, //
            0x83, 102, 0xcd, 0, 12, 103, 1, 104, 0xc0,
        ];
        assert_eq!(parse_edit_rejection(&acked), None);
        let echoed = [
            0, 0, 6, 0, 23, 0, 0, 0, //
            0x83, 102, 0xcd, 0, 71, 103, 0, 104, 0x85, 98, 1, 29, 0xc3, 26, 0, 28, 0, 119, 0xca,
            63, 122, 225, 72,
        ];
        assert_eq!(parse_edit_rejection(&echoed), None);
        // Several ops ACK with nothing at all (the save is one); that is not a refusal either.
        assert_eq!(parse_edit_rejection(&[]), None);
    }

    #[test]
    fn an_outgoing_edit_body_names_its_own_op_and_transaction() {
        // So the log line can say which command a refusal belongs to — the 2026-07-30 rejection
        // took frame-size arithmetic to identify precisely because it couldn't.
        let body = fretwire_protocol::edit::add_block(1, 14, -1, 44);
        assert_eq!(edit_op_txn(&body), (Some(39), Some(44)));
        assert_eq!(op_name(39), "block add");

        let body = fretwire_protocol::edit::save_preset(3, 0, "fretwireTest1", 128);
        assert_eq!(edit_op_txn(&body), (Some(71), Some(128)));
        assert_eq!(op_name(71), "save");

        assert_eq!(edit_op_txn(&[]), (None, None));
    }

    #[test]
    fn a_paired_add_never_puts_the_cab_in_the_add_command() {
        // The device refuses op 39 carrying a cab, so `Session::add_block` sends the amp bare and
        // pairs with op 40. Guard the builder call the same way: whatever `paired_index` the picker
        // hands us, the add body must be the unpaired one (and byte-identical to it).
        let bare = fretwire_protocol::edit::add_block(1, 14, -1, 44);
        let paired = fretwire_protocol::edit::add_block(1, 14, 691, 44);
        assert_ne!(
            bare, paired,
            "the builder still encodes the pair when asked"
        );
        // 691 is `HD2_CabMicIr_4x12BlackbackH30`, the cab `amp.models` links to Brit P75 — every
        // amp's linked cab index is >= 256, which is why every paired add grew the frame to 56.
        assert_eq!(paired.len(), bare.len() + 2);
    }

    #[test]
    fn a_lagging_identity_does_not_confirm_the_preset_we_asked_for() {
        // The 2026-07-30 Floor log, verbatim: goto FACTORY 1 #45, and the device serves Pull Me
        // Under's 8118-byte stream while still calling itself WATERS IN HELL #56 — on the read
        // before the stream *and* the one after, so comparing those two can't catch it.
        let want = Some((0, 45));
        assert!(!identity_confirms(
            want,
            Some(&info(0, 56, "WATERS IN HELL"))
        ));
        assert!(identity_confirms(want, Some(&info(0, 45, "Pull Me Under"))));

        // The same session's other case: the index had caught up but the bank had not, so checking
        // the index alone would have called this settled.
        assert!(!identity_confirms(
            Some((0, 3)),
            Some(&info(1, 3, "Cali Rectifire"))
        ));

        // No pending goto → nothing to confirm against; a missing identity is only a failure when
        // we were waiting on a specific one.
        assert!(identity_confirms(None, None));
        assert!(identity_confirms(
            None,
            Some(&info(0, 56, "WATERS IN HELL"))
        ));
        assert!(!identity_confirms(want, None));
    }

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
