//! HX MI_00 wire protocol: framing, channels, and the TLV body layer.
//!
//! Recovered from USB captures
//! (`docs/protocol.md`). Transport is plain libusb **bulk**
//! transfers on EP 0x01 (OUT) / 0x81 (IN), strict request/response.

mod body;
pub mod edit;
mod frame;
pub mod session;

pub use body::{TLV_MARKER_CMD, TLV_MARKER_REPLY, Tlv};
pub use edit::{EditBody, EditValue};
pub use frame::{Frame, MAGIC, MAGIC_HANDSHAKE};

/// USB Vendor ID for Line 6.
pub const VID_LINE6: u16 = 0x0E41;
/// USB Product ID for the HX Stomp.
pub const PID_HX_STOMP: u16 = 0x4246;
/// USB Product ID for the HX Stomp XL (same protocol).
pub const PID_HX_STOMP_XL: u16 = 0x4253;
/// USB Product ID for the Helix Floor. Confirmed from a contributor's descriptor capture
/// (fw 3.82, 2026-07-22): the vendor control interface, its bulk endpoints and their 512-byte
/// max packet size are identical to the Stomp's.
///
/// The protocol on that pipe is **verified** on this device too — the handshake is byte-identical
/// and every builder in [`edit`] reproduces the Floor's own wire bytes exactly, including edits to
/// blocks on the second DSP (addressed by the same bare slot integer: `slot = dsp * 20 + index`).
/// See `docs/helix-floor.md` and [`DEVICES`].
pub const PID_HELIX_FLOOR: u16 = 0x4248;
/// USB Product ID for the Helix LT, read off a physical unit on Linux (2026-08-18):
/// USB product string `HELIX`, `bcdDevice 0x0200`, and the same six-interface layout the
/// Floor has, interface 0 being the vendor control channel.
///
/// The unit identifies itself as `P21` — the Floor's model code — and every read path
/// reconciles against it unchanged. See `docs/helix-lt.md`.
pub const PID_HELIX_LT: u16 = 0x424A;
/// Interface number of the vendor-specific control channel.
pub const CONTROL_INTERFACE: u8 = 0x00;
/// Bulk OUT endpoint (host → device).
pub const EP_OUT: u8 = 0x01;
/// Bulk IN endpoint (device → host).
pub const EP_IN: u8 = 0x81;

/// How much of a device we've actually confirmed, as opposed to inferred from the family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Wire traffic from this exact device has been observed and reconciled against our builders.
    Verified,
    /// Only the USB IDs are known. The device is in the HX family and very probably speaks the
    /// same protocol, but nothing has been checked against real traffic from one.
    Untested,
}

/// Static facts about one HX-family device.
///
/// Replaces scattered per-device constants: the fields here are exactly what differs between
/// devices, so device-specific behaviour reads a [`Device`] rather than branching on a PID.
/// `None` means **we don't know**, not "not applicable" — nothing here is guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Device {
    /// USB Product ID (with [`VID_LINE6`]).
    pub pid: u16,
    /// Human name, as Line 6 markets it.
    pub name: &'static str,
    /// Model code the device stamps into a preset at key `7 → 36` (e.g. `"P33"`).
    pub model_code: Option<&'static str>,
    /// The `device` field of a preset written by this unit (e.g. `0x210006`).
    pub preset_device_id: Option<u32>,
    /// Populated DSPs — i.e. how many slot groups its presets use. Governs the wire slot range:
    /// slots run `0 .. dsps * 20` (see `fretwire_data::stream::DSP_SLOT_STRIDE`).
    pub dsps: Option<usize>,
    /// Snapshots per preset.
    pub snapshots: Option<usize>,
    /// The device's **setlists**, in bank order — the `bank` of `goto_preset`/`save_preset` and of
    /// a preset's identity (`PresetInfo::bank`, key `107`) is an index into this list. The HX Stomp
    /// has a single flat list; the Helix Floor has eight.
    pub setlists: Option<&'static [&'static str]>,
    /// Preset slots **per setlist**. This is the stride between setlists in the preset-list
    /// browse's *global* numbering: a browse entry's index is `bank * setlist_size + slot`, while
    /// a preset's own identity (`PresetInfo::index`) is the bank-relative `slot`. Confusing the
    /// two sends an out-of-range preset number to the device.
    /// [solid for the Floor — its `.hxb` holds exactly 128 slots in each of the 8 setlists, and a
    /// browse of TEMPLATES (bank 7) returned indices starting at 896 = 7 × 128]
    pub setlist_size: Option<usize>,
    /// How much of the above is confirmed from real traffic.
    pub support: Support,
}

/// Every HX device fretwire knows about.
///
/// The HX Stomp and Helix Floor are both [`Support::Verified`] — the handshake is byte-identical
/// between them and every edit builder reproduces both devices' own wire bytes. The Stomp XL is
/// listed for its PID only: we have no capture, no preset and no backup from one, so its model
/// code, preset device id, DSP and snapshot counts are honestly unknown rather than assumed to
/// match the Stomp's.
pub const DEVICES: &[Device] = &[
    Device {
        pid: PID_HX_STOMP,
        name: "HX Stomp",
        model_code: Some("P33"),
        preset_device_id: Some(0x0021_0006),
        dsps: Some(1),
        snapshots: Some(3),
        // One flat list of 126 presets — no setlist concept on the Stomp. [solid — `list_presets`
        // returns the lot and bank 0 is the only bank that answers]
        setlists: Some(&["Presets"]),
        // One setlist, so bank is always 0 and the stride is never applied. 126 is what a live
        // browse returned.
        setlist_size: Some(126),
        support: Support::Verified,
    },
    Device {
        pid: PID_HELIX_FLOOR,
        name: "Helix Floor",
        model_code: Some("P21"),
        preset_device_id: Some(0x0021_0001),
        dsps: Some(2),
        snapshots: Some(8),
        // The device's own names, in its own casing, in bank order. [solid — two independent
        // sources agree: a `.hxb` backup's eight `L6Setlist` streams carry these names in this
        // order (`fretwire_data::hxb`), and a live read-info reply reported
        // `PresetInfo { bank: 2, index: 17, name: "Sludge" }` for a preset the user had selected
        // in USER 1 — that backup's bank 2 is `USER 1` and holds `Sludge` at index 17]
        setlists: Some(&[
            "FACTORY 1",
            "FACTORY 2",
            "USER 1",
            "USER 2",
            "USER 3",
            "USER 4",
            "USER 5",
            "TEMPLATES",
        ]),
        setlist_size: Some(128),
        support: Support::Verified,
    },
    Device {
        pid: PID_HELIX_LT,
        name: "Helix LT",
        // The LT stamps the Floor's code: the handshake identity reply reports "P21" and a
        // pulled preset carries key `7 → 36` = "P21\0". `by_model_code("P21")` therefore
        // resolves to the Floor, which is listed first — they are one data class.
        model_code: Some("P21"),
        // Unknown, not copied across: the handshake carries no `0x0021xxxx` device id and
        // the wire preset stream has no such field. The Floor's value came from a `.hxb`,
        // and we have no backup from an LT.
        preset_device_id: None,
        // Both DSPs — a pulled preset populates key `1` and holds blocks in slots 21..28
        // (the unit reported DSP1 71.0% / DSP2 43.0%).
        dsps: Some(2),
        // The pulled preset carries SNAPSHOT 1..SNAPSHOT 8.
        snapshots: Some(8),
        // Banks 0..7 each list 128 presets and bank 8 is refused (code -3). Bank 0 holds the
        // factory amp presets and bank 7 the templates ("Quick Start", "Parallel Spans",
        // "SNP:4-Amp Spill") — the Floor's layout, so the Floor's names are used. Unlike the
        // Floor's, these names are not corroborated by a backup; only the arity and the two
        // end banks were observed.
        setlists: Some(&[
            "FACTORY 1",
            "FACTORY 2",
            "USER 1",
            "USER 2",
            "USER 3",
            "USER 4",
            "USER 5",
            "TEMPLATES",
        ]),
        setlist_size: Some(128),
        // Handshake, preset read, setlist and preset-list browse are all reconciled against a
        // physical LT, but no edit has ever been sent to one.
        support: Support::Untested,
    },
    Device {
        pid: PID_HX_STOMP_XL,
        name: "HX Stomp XL",
        model_code: None,
        preset_device_id: None,
        dsps: None,
        snapshots: None,
        setlists: None,
        setlist_size: None,
        support: Support::Untested,
    },
];

impl Device {
    /// Look a device up by USB Product ID.
    pub fn by_pid(pid: u16) -> Option<&'static Device> {
        DEVICES.iter().find(|d| d.pid == pid)
    }

    /// Look a device up by the model code it stamps into presets (preset key `7 → 36`). Only
    /// matches devices whose code we actually know.
    pub fn by_model_code(code: &str) -> Option<&'static Device> {
        DEVICES.iter().find(|d| d.model_code == Some(code))
    }

    /// Slot groups to walk when enumerating this device's preset blocks. Falls back to `1` when
    /// the DSP count is unknown — the conservative choice, since a preset's own key `1` being nil
    /// is what actually decides it at parse time.
    pub fn dsp_count(&self) -> usize {
        self.dsps.unwrap_or(1)
    }

    /// Preset slots per setlist — the stride between setlists in the browse's global numbering.
    /// Falls back to 128 (the only multi-setlist layout we have measured) when unknown.
    pub fn setlist_stride(&self) -> i64 {
        self.setlist_size.unwrap_or(128) as i64
    }

    /// This device's setlist names, in bank order. Falls back to a single unnamed list when we
    /// don't know — the conservative choice: bank 0 is the only bank every HX device answers on.
    pub fn setlist_names(&self) -> &'static [&'static str] {
        self.setlists.unwrap_or(&["Presets"])
    }
}

/// Logical channels, multiplexed over the single bulk endpoint pair ([`EP_OUT`]/[`EP_IN`]) — the
/// channel lives in the frame header, not in USB.
///
/// Each is a stable pair of ids, one per side: **`(host_id, device_id)`**. Neither is inherently
/// source or destination; a frame's [`Frame::src`]/[`Frame::dst`] are these two in the order its
/// direction implies — host→device sends `(host, device)`, and its reply comes back
/// `(device, host)`. So destructuring as `let (src, dst) = channel::EDIT;` is right for an outgoing
/// frame only; going the other way means using the fields in reverse. Matching a reply is better
/// done off the request (`reply.dst == frame.src`, as `fretwire_usb::Transport::request` does) than
/// by reaching for these constants.
///
/// `.0` (the host id) doubles as the **key identifying the channel** in the per-channel sequence
/// and stream-offset counters that `fretwire_core::session::Session` keeps — a use where "src"
/// means nothing.
///
/// Names from observed roles.
pub mod channel {
    /// Primary/handshake channel — the documented handshake runs here.
    pub const PRIMARY: (u16, u16) = (0x1001, 0x03EF);
    /// Edit channel — block/parameter changes.
    pub const EDIT: (u16, u16) = (0x1080, 0x03ED);
    /// Status/meter channel.
    pub const STATUS: (u16, u16) = (0x1002, 0x03F0);
}

/// Command byte (`cmd`, header offset 11).
pub mod cmd {
    /// Session-control. **Open** uses the 0x28 magic + body `00 10 00 00`; **close** is the same
    /// opcode with an *empty* body (see [`SESSION_CLOSE`]). Both are acked with the same opcode.
    pub const HANDSHAKE: u8 = 0x02;
    /// Channel close at shutdown — same opcode as [`HANDSHAKE`], sent with no body. HX Edit sends
    /// one per channel on exit (status → edit → primary) to return the pedal to standalone mode;
    /// omitting it leaves the device in the "editor-connected" panel-locked state. See
    /// `docs/protocol.md` ("Session teardown").
    pub const SESSION_CLOSE: u8 = HANDSHAKE;
    pub const OPEN: u8 = 0x04; // open resource / data
    pub const CHUNK: u8 = 0x08; // chunk request / ack
    pub const STREAM: u8 = 0x0C; // paged stream
    pub const IDLE: u8 = 0x10; // keepalive
}

/// TLV `type` values (the sub-command inside a data frame's body).
pub mod op {
    /// Parameter set — value is the target handle followed by a big-endian `f32`.
    pub const PARAM_SET: u16 = 0x0006;
    /// Block bypass toggle (no explicit value; device flips state).
    pub const BYPASS: u16 = 0x0003;
    /// Session-open resource.
    pub const SESSION_OPEN: u16 = 0x0002;
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("buffer too short: need at least {need} bytes, got {got}")]
    Short { need: usize, got: usize },
    #[error("declared length {declared} exceeds available bytes {avail}")]
    BadLength { declared: usize, avail: usize },
    #[error("body too short to be a TLV: {0} bytes (need >= 8)")]
    NotTlv(usize),
    #[error("edit body: {0}")]
    Edit(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
pub(crate) fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
