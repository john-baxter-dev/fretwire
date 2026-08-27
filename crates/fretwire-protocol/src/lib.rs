//! HX MI_00 wire protocol: framing, channels, and the TLV body layer.
//!
//! Recovered from USB captures
//! (`docs/protocol.md`). Transport is plain libusb **bulk**
//! transfers on EP 0x01 (OUT) / 0x81 (IN), strict request/response.

mod body;
pub mod edit;
mod frame;
pub mod session;
pub mod settings;

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
/// USB Product ID for the HX Effects, read off a contributor's unit with `lsusb`
/// (2026-08-22, issue #10): `ID 0e41:4245 Line6, Inc. HX Effects`.
///
/// That descriptor line is the whole of what we know — no traffic from one has been seen, and the
/// HX Effects is the family member least like the others (it runs the HX effects but no amp or cab
/// models, and its preset layout is its own), so nothing below is filled in from a sibling.
pub const PID_HX_EFFECTS: u16 = 0x4245;
/// USB Product ID for the POD Go, from a contributor's capture of POD Go Edit's startup
/// (2026-08-25, issue #15): `ID 0e41:4247`.
///
/// The control protocol is the HX family's, verified against that capture: the frame codec, the
/// three channels and their ids, and the session handshake are byte-identical, the MessagePack
/// envelope and op numbers match (76 meters, 22 preset read, 24 settings, 33 read-switch), and the
/// paged preset stream reassembles into the same `l6-helix` structure. What differs is **data, not
/// protocol** — the POD Go has its own symbol table (`PodGo.sym`, 627 entries against the Helix's
/// 833), so a block's model reference (`24 -> 25`) means something else on this device.
/// See `docs/pod-go.md`.
pub const PID_POD_GO: u16 = 0x4247;
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
    /// Someone has run the editor against one and reported back, but the device is not yet fully
    /// described — so some field below is still honestly unknown. Sits between the other two on
    /// purpose: "a user has this working" is real information, and calling it [`Support::Untested`]
    /// understates it, while [`Support::Verified`] promises a device we can describe completely.
    ///
    /// It does **not** imply we hold no traffic. The POD Go sits here with its reads *and* writes
    /// reconciled byte-for-byte against captures from a real unit; what it lacks is a
    /// `preset_device_id`, which only a backup file carries. The tier tracks how completely we know
    /// the device, not how much evidence we have.
    Reported,
    /// Only the USB IDs are known. The device is in the HX family and very probably speaks the
    /// same protocol, but nothing has been checked against real traffic from one.
    Untested,
}

impl Support {
    /// A short caveat to show the user, or `None` when there is nothing to warn about.
    pub fn caveat(self) -> Option<&'static str> {
        match self {
            Support::Verified => None,
            Support::Reported => Some("reported working; some device details are still unknown"),
            Support::Untested => Some("untested — its protocol is assumed to match the HX family"),
        }
    }
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
    /// How many presets the pedal's own screen puts in one **bank** — the `3` in the HX Stomp's
    /// `01A`/`01B`/`01C`/`02A`. Slot `n` is then bank `n / this + 1`, letter `'A' + n % this`.
    ///
    /// `None` where we have not seen the device's screen, and the editor then numbers presets by
    /// slot instead. That is the conservative direction on purpose: this label's whole job is to
    /// match what is written on the hardware, and a label that confidently disagrees with the panel
    /// is worse than an honest slot number — it is the same failure as listing presets in an order
    /// the pedal doesn't use.
    ///
    /// **Bank size is not the whole story.** Which of the two forms the pedal shows —  `01A` or the
    /// flat `000` — is a global setting on the device, so this label can be right about the banking
    /// and still not match a panel set to the other form. [2026-08-21 XL owner report]
    ///
    /// **The setting does not reach the wire** [solid — 2026-08-21, HX Stomp]. Flipping it on a
    /// live unit and re-reading left both streams we take byte-identical: the bank-0 browse listing
    /// (3267 bytes, same md5) and the loaded preset's stream (2388 bytes, same md5, same slot). So
    /// it cannot be detected from a listing or a preset — it lives in the globals, which op 24 reads
    /// and op 25 writes (see `docs/protocol.md`). **It is setting id 27** [2026-08-22] — `true` for
    /// the flat form, `false` for the banked one. So the pedal supplies the default and the GUI's
    /// override (`ui/src/lib/numbering.svelte.js`) only overrules it once the user sets one; this
    /// still renders the banked form where no device has been read, that being how the pedal ships.
    pub presets_per_bank: Option<usize>,
    /// How much of the above is confirmed from real traffic.
    pub support: Support,
}

/// Every HX device fretwire knows about.
///
/// The HX Stomp and Helix Floor are both [`Support::Verified`] — the handshake is byte-identical
/// between them and every edit builder reproduces both devices' own wire bytes.
///
/// Two are [`Support::Reported`], for different reasons, which is the point of the tier being
/// evidential rather than a ranking: the **Helix LT** was surveyed on real hardware (reads and
/// browses reconcile; no edit has been sent to one), while the **HX Stomp XL** is known through its
/// owner — what they can run, read off the panel and paste back. Neither has a capture reconciled
/// against our builders, and in both cases every field we did not observe stays `None` rather than
/// being copied from a sibling: the LT reports the Floor's `P21` and still does not inherit its
/// `preset_device_id`, and the XL's own `P36` says nothing about its DSP or snapshot counts.
///
/// The **HX Effects** reached [`Support::Reported`] the same way, by outcome alone: its `lsusb`
/// line arrived first and made it findable, and an owner has since said it works. That report is
/// the whole of the evidence — no capture, no session, no panel readings — so every other field is
/// still empty.
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
        // 126 = 42 banks of three, which is what the three footswitches select.
        // [solid — read off the pedal's screen, 2026-08-20]
        presets_per_bank: Some(3),
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
        // Unknown. 128 divides by 4 and by 8 and the Floor has eight preset footswitches, so a
        // guess here has two plausible answers and no evidence — and the wrong one mislabels every
        // preset on the unit. Needs one look at a Floor's screen.
        presets_per_bank: None,
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
        // Unknown: the survey read presets and browsed setlists but never says how the LT's screen
        // groups them, and the Floor's is unknown too, so there is nothing to inherit. Presets fall
        // back to slot numbers until someone reads it off the unit. See `docs/helix-lt.md`.
        presets_per_bank: None,
        // Handshake, preset read, setlist and preset-list browse are all reconciled against a
        // physical LT, but no edit has ever been sent to one. That is more than `Untested` ("only
        // the USB IDs are known") describes — the reads are real traffic, checked against real
        // parsers — and less than `Verified`, which means the *builders* have been reconciled too.
        support: Support::Reported,
    },
    Device {
        pid: PID_HX_STOMP_XL,
        name: "HX Stomp XL",
        // `P36`, seen twice on one owner's unit: the handshake identity reply came back `"P36Main"`
        // (a live device answering us, not a spec sheet) and a preset read off the same pedal
        // carried `P36` at key `7 → 36`. The two paths are independent, so this is the one field a
        // bug report has been able to settle. [solid — 2026-08-21, owner report, issue #4]
        model_code: Some("P36"),
        // Still unknown: nothing we have read exposes it. The model code above comes from an
        // identity string and a preset stamp; this is a different field in the backup header.
        preset_device_id: None,
        dsps: None,
        snapshots: None,
        setlists: None,
        // 32 banks of 4 is 128 presets, which the owner reads off the panel as `01A`-`32D`. Equal
        // to the `setlist_stride` fallback, so it changes no addressing — it records the reading.
        // Whether the XL has *several* such setlists is still unknown, which is why `setlists`
        // above stays `None`. [2026-08-21 owner report]
        setlist_size: Some(128),
        // Four, not the Stomp's three, and not a guess from the footswitch count either — the owner
        // read `A`/`B`/`C`/`D` off the pedal's own screen. [2026-08-21 owner report]
        presets_per_bank: Some(4),
        // An owner ran the 0.2.x editor against one over several sessions — browsing, editing,
        // exporting — and the bugs they filed were device-independent and reproduced on an
        // HX Stomp. That is enough to say it works, and (with the banking above) not enough to fill
        // in the rest. [2026-08-20/21, issues #2 and #3]
        support: Support::Reported,
    },
    Device {
        pid: PID_HX_EFFECTS,
        name: "HX Effects",
        // Nothing but the USB ID has been seen. The HX Effects is an effects-only unit — no amps,
        // no cabs — so even the fields that look safe to copy from a Stomp are not: its model code,
        // preset geometry and snapshot count are all unobserved.
        model_code: None,
        preset_device_id: None,
        dsps: None,
        snapshots: None,
        setlists: None,
        setlist_size: None,
        presets_per_bank: None,
        // An owner says it works. Unlike the XL there are no panel readings behind that — nothing
        // was read off a screen, nothing was pasted back — so it fills in none of the fields above.
        // It is still `Reported` rather than `Untested`: "a user has this working" is real
        // information about the protocol, confirmed by outcome. [2026-08-24 owner report]
        support: Support::Reported,
    },
    Device {
        pid: PID_POD_GO,
        name: "POD Go",
        // `P34`, from two independent paths in one capture — the handshake identity reply returns
        // "P34Main" on the primary channel, and the preset the device then streamed carries
        // `7 -> 36` = "P34\0". The same two-source standard the XL's `P36` was accepted on.
        // [solid — 2026-08-25, issue #15 capture]
        model_code: Some("P34"),
        // Not the `0x02500000` the identity reply carries beside the model code — that is the
        // field matching preset key `35` (the Stomp's reads `0x03800000`), not this one. The
        // Stomp's and Floor's `preset_device_id` came from `.hxb` backup headers, and we have no
        // backup from a POD Go.
        preset_device_id: None,
        // One. The streamed preset populates group key `0` only — key `1` is nil — and every block
        // sits in slots 1..10, inside the first group's stride. This is the same reasoning that put
        // the LT on two, run the other way. [solid — 2026-08-25 capture]
        dsps: Some(1),
        // Four, against the Stomp's three: the preset's snapshot table holds exactly four entries,
        // named SNAPSHOT 1..4. Read out of the preset itself, not from the footswitch count.
        // [solid — 2026-08-25 capture]
        snapshots: Some(4),
        // More than one — POD Go Edit's startup asks for preset info with `107` (bank) = 1, which
        // the Stomp's single flat list would never do — but the capture never names them or shows
        // how many there are, so listing any would be invention.
        setlists: None,
        setlist_size: None,
        // Unknown: nobody has read a POD Go's screen. Left `None` so presets are numbered by slot
        // rather than mislabelled, as on the Floor.
        presets_per_bank: None,
        // Both halves are now reconciled against a real unit. Reads: the frame codec, channel ids,
        // handshake and paged preset stream all parse unchanged, and the op numbers match. Writes:
        // the contributor captured a parameter change, a bypass toggle and a block model swap on
        // named slots, and the `edit` builders — written entirely from HX Stomp traffic — reproduce
        // all three byte-for-byte (`tests/pod_go_writes.rs`); the bypass body differs from the
        // Stomp's captured one in exactly one byte, the slot number. They have also driven a POD Go
        // from the editor in both directions.
        //
        // `Reported` rather than `Verified` on one count only: `preset_device_id` is unknown,
        // because that field has only ever come from a `.hxb` backup header and nobody has sent a
        // POD Go backup. The remaining gaps are *geometry*, not protocol — the fixed chain's
        // add/move/delete semantics and the footswitch mapping. [solid — 2026-08-26, issue #15]
        support: Support::Reported,
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

    /// The two things setting 27 (`Preset Number`) is called on **this** device's screen, as
    /// `(flat, banked)` — `("000-125", "01A-42C")` on an HX Stomp, `("000-127", "01A-32D")` on an
    /// XL.
    ///
    /// The setting is one id with one meaning, but its menu spells out the pedal's preset range, so
    /// the text differs with the preset count and `settings::SETTINGS` — a flat table with no notion
    /// of the device — cannot hold both. Deriving them here is what keeps that table honest: the
    /// counts are already measured per device, so the labels follow rather than being a second set
    /// of facts to keep in sync.
    ///
    /// `None` unless both counts are known, which is the same conservative rule
    /// [`Self::presets_per_bank`] states: a label that confidently disagrees with the panel is worse
    /// than the generic one. That is the Floor and the LT today — 128 slots each, bank size unknown.
    ///
    /// **Every string this produces has been read off a screen, with one deliberate exception** —
    /// Stomp `000-125` and `01A-42C` [2026-08-24], XL `01A-32D` [2026-08-21]. The exception is the
    /// XL's flat form: its menu draws `000-128`, which is a **firmware bug** — that unit's presets
    /// stop at 127 [owner, 2026-08-24] — so this derives the truthful `000-127` and the editor
    /// disagrees with the pedal's screen by one character. See the note on setting 27.
    ///
    /// The Stomp's banked form is the one worth naming: it was
    /// *derived here first* and confirmed on the pedal afterwards, so the arithmetic has been
    /// checked against hardware rather than merely trusted. That is the reason to keep deriving it
    /// for a device whose counts are known, and the reason to keep returning `None` for one whose
    /// counts are not.
    pub fn preset_numbering_labels(&self) -> Option<(String, String)> {
        let (size, per_bank) = (self.setlist_size?, self.presets_per_bank?);
        // A bank size that doesn't divide the list, or runs past `Z`, means one of the two counts is
        // wrong; say nothing rather than draw a label off it.
        if per_bank == 0 || per_bank > 26 || size % per_bank != 0 {
            return None;
        }
        let last_letter = (b'A' + per_bank as u8 - 1) as char;
        Some((
            format!("000-{}", size - 1),
            format!("01A-{:02}{last_letter}", size / per_bank),
        ))
    }

    /// How the pedal's own screen writes preset `slot` — `01A`, `01B`, `01C`, `02A`, … — or `None`
    /// on a device whose banking we have not seen ([`Device::presets_per_bank`]).
    ///
    /// Banks are 1-based and zero-padded to **two** digits, matching the panel — the Stomp's 126
    /// presets are 42 banks, so two is all it ever needs. The letter runs `A..` within the bank, so
    /// a Stomp's slot 24 is `09A`: bank 9, first of three.
    ///
    /// This is a **label, not an address** — the same distinction the browse listing's map key
    /// turned out to need. `goto_preset`/`save_preset` take the slot; nothing takes this.
    pub fn preset_label(&self, slot: i64) -> Option<String> {
        let per = self.presets_per_bank?;
        let slot = usize::try_from(slot).ok()?;
        // 26 letters is not a real constraint (no HX device banks more than a handful per bank),
        // but a table typo that said 40 shouldn't produce `001{`.
        let letter = char::from_digit((slot % per) as u32 + 10, 36)?.to_ascii_uppercase();
        Some(format!("{:02}{letter}", slot / per + 1))
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
