//! USB transport for HX devices using `nusb` (pure-Rust, libusb-free).
//!
//! [`Transport`] opens the HX Stomp, claims the vendor control interface, and does strict
//! request/response bulk I/O on EP `0x01` (OUT) / `0x81` (IN). Frame framing lives in
//! `fretwire-protocol`; the stateful protocol (handshake, channels, preset stream, edits) lives in
//! `fretwire-core::session` on top of this.
//!
//! Cross-platform compile, but only usable where the OS lets us claim the interface — i.e. **Linux**
//! (on Windows the Line 6 driver owns interface 0).

use futures_lite::future::{self, block_on};
use fretwire_protocol::{Frame, CONTROL_INTERFACE, EP_IN, EP_OUT, PID_HX_STOMP, VID_LINE6};
use nusb::transfer::RequestBuffer;

/// Max bytes to request on a bulk IN. Frames are ≤272 bytes (16 header + 256 payload chunk);
/// 1024 leaves headroom.
const IN_BUF: usize = 1024;

/// How long to wait for a bulk IN before giving up. nusb 0.1 has no bulk timeout, so a missing
/// reply would block forever (and hold the interface). Normal replies are <10 ms and inter-frame
/// gaps well under a second; 3 s turns a desync into a clean error (and keeps connect-retry snappy).
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Max unsolicited frames to skip while waiting for a request's matching reply. The status
/// channel emits meters/keepalives that interleave with our solicited replies; this bounds how
/// many we'll discard before giving up so a chatty device can't hang us.
const MAX_SKIP: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no HX Stomp found on the USB bus")]
    NotFound,
    #[error("usb error: {0}")]
    Usb(#[from] nusb::Error),
    #[error("usb transfer error: {0}")]
    Transfer(#[from] nusb::transfer::TransferError),
    #[error("frame decode: {0}")]
    Frame(#[from] fretwire_protocol::Error),
    #[error("no reply matching channel {dst:#06x}/seq {seq} within {MAX_SKIP} frames")]
    Unmatched { dst: u16, seq: u8 },
    #[error("timed out waiting for a bulk IN")]
    Timeout,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Returns whether an HX Stomp is currently enumerated. A first, dependency-free smoke test that
/// `nusb` can see the device before we attempt to claim its interface.
pub fn hx_stomp_present() -> Result<bool> {
    let found = nusb::list_devices()?
        .any(|d| d.vendor_id() == VID_LINE6 && d.product_id() == PID_HX_STOMP);
    Ok(found)
}

/// A claimed bulk pipe to the HX Stomp's control interface.
pub struct Transport {
    iface: nusb::Interface,
    /// Frames decoded from a prior bulk IN but not yet consumed — a single IN transfer can
    /// concatenate several frames (the handshake batches 3 in one read).
    pending: std::collections::VecDeque<Frame>,
}

impl Transport {
    /// Find the HX Stomp, open it, and claim the control interface.
    pub fn open() -> Result<Transport> {
        let info = nusb::list_devices()?
            .find(|d| d.vendor_id() == VID_LINE6 && d.product_id() == PID_HX_STOMP)
            .ok_or(Error::NotFound)?;
        let device = info.open()?;
        // On Linux nothing should hold this vendor interface, but detach defensively if it does.
        let iface = match device.claim_interface(CONTROL_INTERFACE) {
            Ok(i) => i,
            Err(_) => device.detach_and_claim_interface(CONTROL_INTERFACE)?,
        };
        tracing::info!("claimed HX Stomp control interface {CONTROL_INTERFACE}");
        Ok(Transport { iface, pending: std::collections::VecDeque::new() })
    }

    /// Send raw bytes on the bulk OUT endpoint.
    pub fn send(&self, bytes: Vec<u8>) -> Result<()> {
        tracing::trace!(len = bytes.len(), "bulk OUT {:02x?}", bytes);
        block_on(self.iface.bulk_out(EP_OUT, bytes)).into_result()?;
        Ok(())
    }

    /// Read one bulk IN transfer (up to [`IN_BUF`] bytes), bounded by [`READ_TIMEOUT`].
    pub fn recv(&self) -> Result<Vec<u8>> {
        self.recv_timeout(READ_TIMEOUT)
    }

    /// Read one bulk IN transfer, bounded by `dur`. nusb 0.1 has no bulk timeout, so we race the
    /// transfer against a timer; on timeout the transfer future is dropped, which cancels the URB
    /// (no leak), and we return [`Error::Timeout`].
    fn recv_timeout(&self, dur: std::time::Duration) -> Result<Vec<u8>> {
        let transfer = self.iface.bulk_in(EP_IN, RequestBuffer::new(IN_BUF));
        let outcome = block_on(future::or(
            async move { Some(transfer.await.into_result()) },
            async move {
                futures_timer::Delay::new(dur).await;
                None
            },
        ));
        match outcome {
            Some(Ok(data)) => {
                tracing::trace!(len = data.len(), "bulk IN  {:02x?}", data);
                Ok(data)
            }
            Some(Err(e)) => Err(e.into()),
            None => Err(Error::Timeout),
        }
    }

    /// Pull the next frame, refilling from a bulk IN (which may batch several) when the buffer
    /// is empty. Bounds the refill read by `timeout`.
    fn next_frame_within(&mut self, timeout: std::time::Duration) -> Result<Frame> {
        if let Some(f) = self.pending.pop_front() {
            return Ok(f);
        }
        let raw = self.recv_timeout(timeout)?;
        let mut frames = Frame::decode_all(&raw)?.into_iter();
        let first = frames
            .next()
            .ok_or(Error::Frame(fretwire_protocol::Error::Short { need: 16, got: raw.len() }))?;
        self.pending.extend(frames);
        Ok(first)
    }

    /// Send a frame and return the reply that matches it: same channel (the device echoes our
    /// `src` back as the reply's `dst`) and same sequence number. Unsolicited frames on other
    /// channels (status meters, keepalives) are skipped — see [`MAX_SKIP`].
    pub fn request(&mut self, frame: &Frame) -> Result<Frame> {
        self.request_within(frame, READ_TIMEOUT)
    }

    /// Like [`Transport::request`] but bounds each bulk-IN read by `timeout` instead of the default
    /// [`READ_TIMEOUT`]. Used by session teardown, where one channel may legitimately never ack and
    /// we don't want to block the full 3 s on it.
    ///
    /// **Matching:** a reply is the next frame on our channel (`dst == frame.src`) that is **not a
    /// keepalive** (`cmd != IDLE`). The device does *not* echo our `seq` — it runs its own per-channel
    /// counter — so we cannot match on seq; and it interleaves `cmd 0x10` keepalives on every channel
    /// that must be skipped. Frames on other channels (status meters) are skipped too.
    pub fn request_within(&mut self, frame: &Frame, timeout: std::time::Duration) -> Result<Frame> {
        self.request_matching(frame, timeout, |_| true)
    }

    /// Like [`Transport::request_within`] but the caller supplies an extra `accept` predicate the
    /// reply must satisfy (on top of the channel + non-keepalive match). Used to correlate by the
    /// **transaction id echoed in the reply body** — the device interleaves its own state-push and
    /// keepalive frames on the edit channel, and a loose match can mistake one of those for the
    /// reply (e.g. as a preset stream's chunk #0). The txn check pins the right frame.
    pub fn request_matching(
        &mut self,
        frame: &Frame,
        timeout: std::time::Duration,
        accept: impl Fn(&Frame) -> bool,
    ) -> Result<Frame> {
        self.send(frame.encode())?;
        for _ in 0..MAX_SKIP {
            let reply = self.next_frame_within(timeout)?;
            if reply.dst == frame.src && reply.cmd != fretwire_protocol::cmd::IDLE && accept(&reply) {
                return Ok(reply);
            }
            tracing::debug!(
                want_dst = format_args!("{:#06x}", frame.src),
                got_dst = format_args!("{:#06x}", reply.dst), got_src = format_args!("{:#06x}", reply.src),
                got_seq = reply.seq, cmd = reply.cmd, body = reply.body.len(),
                "skipping non-reply frame",
            );
        }
        Err(Error::Unmatched { dst: frame.src, seq: frame.seq })
    }

    /// Discard any already-buffered frames (e.g. the device's post-transaction epilogue that got
    /// batched into a read). Non-blocking; returns how many frames were dropped. Fresh frames that
    /// arrive on the wire later are tolerated by [`Transport::request`]'s channel/seq matching.
    pub fn drain(&mut self) -> usize {
        let n = self.pending.len();
        if n > 0 {
            tracing::debug!(frames = n, "draining buffered frames");
            self.pending.clear();
        }
        n
    }

    /// Clear stale frames left on the wire by a previous session: read with a short per-frame
    /// timeout and discard, until the device goes quiet for `quiet` or `max_frames` are dropped
    /// (a bound, in case the device is mid-stream). Each timed-out read cancels its URB cleanly.
    /// Run right after claiming the interface so a fresh handshake starts aligned.
    pub fn drain_wire(&mut self, quiet: std::time::Duration, max_frames: usize) {
        self.pending.clear();
        let mut dropped = 0;
        while dropped < max_frames {
            match self.recv_timeout(quiet) {
                Ok(_) => dropped += 1,
                Err(_) => break, // quiet (timeout) or error — done
            }
        }
        if dropped > 0 {
            tracing::debug!(frames = dropped, "drained stale wire frames at connect");
        }
    }

    /// Like [`Transport::drain_wire`] but **returns** the drained frames instead of discarding them,
    /// so the caller can inspect device state-pushes (status-channel `{105,106}` mirrors) that arrive
    /// on a held session. Reads until the device goes quiet for `quiet` or `max_frames` are collected.
    pub fn drain_collect(&mut self, quiet: std::time::Duration, max_frames: usize) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Some(f) = self.pending.pop_front() {
            out.push(f);
            if out.len() >= max_frames {
                return out;
            }
        }
        while out.len() < max_frames {
            match self.next_frame_within(quiet) {
                Ok(f) => out.push(f),
                Err(_) => break, // quiet (timeout) or error — done
            }
        }
        out
    }

    /// Send a frame without waiting for a reply (fire-and-forget OUT).
    pub fn send_frame(&self, frame: &Frame) -> Result<()> {
        self.send(frame.encode())
    }
}
