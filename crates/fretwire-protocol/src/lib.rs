//! HX MI_00 wire protocol: framing, channels, and the TLV body layer.
//!
//! Recovered from USB captures
//! (`docs/protocol.md`). Transport is plain libusb **bulk**
//! transfers on EP 0x01 (OUT) / 0x81 (IN), strict request/response.

mod body;
pub mod edit;
mod frame;
pub mod session;

pub use body::{Tlv, TLV_MARKER_CMD, TLV_MARKER_REPLY};
pub use edit::{EditBody, EditValue};
pub use frame::{Frame, MAGIC, MAGIC_HANDSHAKE};

/// USB Vendor ID for Line 6.
pub const VID_LINE6: u16 = 0x0E41;
/// USB Product ID for the HX Stomp.
pub const PID_HX_STOMP: u16 = 0x4246;
/// USB Product ID for the HX Stomp XL (same protocol).
pub const PID_HX_STOMP_XL: u16 = 0x4253;
/// Interface number of the vendor-specific control channel.
pub const CONTROL_INTERFACE: u8 = 0x00;
/// Bulk OUT endpoint (host → device).
pub const EP_OUT: u8 = 0x01;
/// Bulk IN endpoint (device → host).
pub const EP_IN: u8 = 0x81;

/// Logical channels, identified by a (host `src`, device `dst`) u16 pair. `src`/`dst` swap by
/// direction on the wire. Names from observed roles.
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
