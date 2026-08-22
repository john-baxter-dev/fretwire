//! `fretwire` — command-line driver for the fretwire stack.
//!
//! Commands are defined with `clap`'s derive API, so `--help` is generated from the definitions
//! below rather than maintained by hand, and every numeric argument either parses or errors — a
//! bad one used to fall back to a default, which on `save`/`rename` meant silently writing to
//! bank 0 instead of the bank you typed.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

/// Bypass state in **pedal** semantics, which are inverted from "block enabled".
///
/// A plain `bool` positional can't express this: clap reads a bare `bool` as a flag, and the old
/// hand-rolled parser treated *any* unrecognised word — and a missing argument — as `off`, so a
/// typo silently did the opposite of what was asked.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OnOff {
    /// Engage bypass: the block goes OFF, as on the pedal.
    #[value(alias = "true", alias = "1")]
    On,
    /// Release bypass: the block is active.
    #[value(alias = "false", alias = "0")]
    Off,
}

/// Which end of a controller assignment's travel is being set.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
enum TravelEnd {
    /// The heel / off end (op 65).
    Min,
    /// The toe / on end (op 66).
    Max,
}

/// Which signal row a block is being moved to.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Row {
    /// The parallel path (row B).
    #[value(name = "p", alias = "parallel", alias = "1")]
    Parallel,
    /// The series path (row A).
    #[value(name = "s", alias = "series", alias = "0")]
    Series,
}

/// Independent Linux editor for the Line 6 HX Stomp / Helix Floor.
///
/// Offline commands work anywhere; **live** commands need the pedal on USB (see `install-udev`
/// if they fail with a permissions error). Commands marked ⚠ write to the device's flash.
#[derive(Parser)]
// `--version` carries the commit too, so a bug report that quotes it identifies the build exactly.
#[command(name = "fretwire", version = fretwire_core::BUILD_BANNER)]
struct Cli {
    /// Defaults to `detect` when omitted, so a bare `fretwire` still reports what's plugged in.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    // ---- offline ----
    /// List any HX devices present on USB (enumeration only — claims nothing).
    Detect,
    /// Decode a reassembled preset stream from a file and print it as a block/param tree.
    ShowPreset {
        /// A file written by `dump-raw`.
        stream: String,
    },
    /// Decode the MessagePack body of a captured edit command.
    DecodeEdit {
        /// Hex bytes of the edit body, e.g. `8366cd03f2...`.
        hex: String,
    },
    /// Print the integer-key paths that differ between two saved streams.
    DiffStream { a: String, b: String },
    /// Print the MessagePack structure of a saved raw stream (RE exploration).
    Tree {
        stream: String,
        /// How many levels deep to descend.
        #[arg(default_value_t = 4)]
        depth: usize,
    },
    /// Inspect an HX Edit `.hxb` device backup offline. Reads only.
    ShowBackup {
        backup: String,
        /// Also list every preset in every setlist.
        #[arg(long)]
        presets: bool,
    },
    /// List what an export file contains (setlists, slots + names), to pick a restore.
    BackupShow { backup: String },
    /// Import Line 6's reference data from your own HX Edit install.
    ///
    /// The source is either an HX Edit installer (.exe/.msi/.pkg/.dmg, unpacked with 7z) or a
    /// directory of already-extracted files (e.g. an install's `res/` folder, which needs no 7z).
    /// We redistribute nothing — the data goes Line 6 → user → tool.
    ImportData { source: String },
    /// Install the udev rule granting your user access to the pedal's USB node.
    ///
    /// Without it every live command needs root.
    InstallUdev {
        /// Print the rule to stdout instead of installing it.
        #[arg(long)]
        print: bool,
    },

    // ---- live: read / navigate ----
    /// Open a session and complete the handshake, then hang up.
    Connect,
    /// Connect and immediately tear down — isolates the teardown so you can confirm the pedal
    /// returns to standalone (no "panel lock") after our software lets go.
    Disconnect,
    /// Hold a session and print the device's unsolicited status pushes as you touch the pedal —
    /// footswitches, snapshot/preset changes, panel knobs. The tool for finding out what the
    /// hardware actually sends for a change we don't follow yet. Ctrl-C to stop.
    Watch {
        /// Seconds to watch before hanging up.
        #[arg(long, default_value_t = 60)]
        secs: u64,
    },
    /// Read the currently-loaded preset and print it, with the snapshot diagnosis.
    Pull,
    /// List one setlist's presets with their indices. Reads only.
    Presets {
        /// Defaults to the setlist the device is currently sitting in.
        bank: Option<i64>,
    },
    /// Name each setlist the connected device has, with the bank index `presets`/`goto` take.
    Setlists,
    /// Navigate to a preset. Changes the device's active preset.
    Goto {
        preset: i64,
        #[arg(default_value_t = 0)]
        bank: i64,
    },
    /// Switch the active snapshot (0-based, as `pull` lists them).
    Snapshot { index: i64 },
    /// Save the raw reassembled preset-list stream for a setlist. Reads only, diagnostic.
    DumpList { bank: i64, out: String },
    /// Save the raw reassembled preset stream to a file (for diffing states).
    DumpRaw { out: String },
    /// Read the preset stored in a slot **without loading it** (op 4). Reads only — the pedal stays
    /// on whatever preset it is showing, and any pending edits survive.
    ReadSlot {
        bank: i64,
        slot: i64,
        /// Write the raw document here instead of summarising it.
        #[arg(long)]
        out: Option<String>,
    },

    // ---- live: edit the buffer (reversible by reloading the preset) ----
    /// Set a block's bypass, in pedal semantics: `on` engages bypass (block OFF).
    Bypass { slot: i64, state: OnOff },
    /// Set a parameter by its index in the model's `Helix.sym` order.
    Set {
        slot: i64,
        param_index: i64,
        value: f32,
    },
    /// Turn a delay/reverb's Trails on or off (the tail that rings on after bypass).
    Trails { slot: i64, state: OnOff },
    /// Set a parameter on the block's paired cab/IR (amp+cab blocks).
    ///
    /// Param indices are in the cab's own namespace: mic=0, position=1, distance=2, angle=3.
    SetCab {
        slot: i64,
        param_index: i64,
        value: f32,
    },
    /// Swap a block's model. `model-index` is the `Helix.sym` index.
    Swap {
        slot: i64,
        model_index: i64,
        /// Paired cab/IR index; -1 for none.
        #[arg(default_value_t = -1)]
        paired_index: i64,
    },
    /// Add a block. `model-index` is the `Helix.sym` index.
    AddBlock {
        slot: i64,
        model_index: i64,
        /// Paired cab/IR index; -1 for none.
        #[arg(default_value_t = -1)]
        paired_index: i64,
    },
    /// Delete a block (op 28 surgical delete — preserves the other blocks' footswitch layout).
    DeleteBlock { slot: i64 },
    /// Move a block. The destination slot encodes the row: a parallel slot index moves it to row B.
    Move { src_slot: i64, dst_slot: i64 },
    /// Position-aware cross-row move.
    MoveToRow {
        src_slot: i64,
        row: Row,
        /// Insertion index among the target row's blocks, or `end`.
        #[arg(default_value = "end")]
        pos: String,
    },
    /// Move a block into the common (pre-split) section, just before the split.
    BeforeSplit { src_slot: i64 },
    /// Move the split (⋔) or mixer (⋉) node to a signal-flow column. Goes through the **op-21
    /// whole-preset write** — the operation that has produced every device lockup on record — so
    /// this exists mainly to reproduce one from the CLI with a log attached.
    NodePos {
        /// `split` or `mixer`.
        which: String,
        /// Target column. The split must stay left of every row-B block; the mixer right of them.
        pos: i64,
    },
    /// Retype the parallel split node (op 40). Only meaningful on a split preset.
    SplitType {
        /// `y`, `ab`, `xover`, `dyn`, or a raw model index.
        which: String,
    },
    /// Rename a snapshot of the current preset (0-based index).
    RenameSnapshot { index: i64, name: String },

    // ---- live: writes and probes ----
    /// ⚠ PERSISTENT WRITE. Save the current edit buffer over a preset slot.
    Save {
        slot: i64,
        name: String,
        #[arg(default_value_t = 0)]
        bank: i64,
    },
    /// ⚠ PERSISTENT WRITE. Rename a preset in flash, name-only (op 6).
    ///
    /// Changes only the stored name — does NOT commit the edit buffer, so pending edits stay
    /// unsaved.
    Rename {
        slot: i64,
        name: String,
        #[arg(default_value_t = 0)]
        bank: i64,
    },
    /// Export a setlist's presets to a file. Reads only.
    ///
    /// This is a **setlist export**, not a device backup: it captures presets and nothing else —
    /// no global settings, no IRs. Restoring from it will not make a wiped pedal whole. Flash is
    /// never written, but the active-preset cursor sweeps the setlist (it is put back afterwards).
    ///
    /// Was called `backup`, which overpromised; the old name still works.
    #[command(alias = "backup")]
    ExportSetlist {
        out: String,
        /// Which setlist to export (see `setlists`). Ignored with `--all`.
        #[arg(long, default_value_t = 0)]
        bank: i64,
        /// Export every setlist the device has. On a Helix Floor that is 1024 presets — expect it
        /// to take the best part of an hour, and the pedal to step through all of them.
        #[arg(long)]
        all: bool,
    },
    /// ⚠ PERSISTENT WRITE. Restore one preset from an export file into a setlist slot.
    Restore {
        backup: String,
        index: i64,
        /// Target slot; defaults to the backup index.
        slot: Option<i64>,
        /// Which setlist the preset was exported from, and the one it goes back to.
        #[arg(long, default_value_t = 0)]
        bank: i64,
    },
    /// PROBE: read the preset and write it back unchanged via op 21, then re-read.
    ///
    /// Safe — touches the edit buffer only (reversible by reloading) and changes nothing.
    WriteRoundtrip,
    /// Set a global/input setting (op 25). The id space is only partly mapped — a live RE probe.
    ///
    /// Known: id 134 = 3-state input setting (0/1/2).
    Setting { id: i64, value: i64 },
    /// List the device's user IR slots — index, name and blob checksum. **Reads only.**
    IrList {
        /// Show empty slots too, instead of only the ones holding an IR.
        #[arg(long)]
        all: bool,
    },
    /// Show one IR slot's metadata (op 12). **Reads only.**
    IrInfo { slot: i64 },
    /// Download an IR slot to a 32-bit float, 48 kHz mono WAV. **Reads only.**
    IrExport {
        slot: i64,
        /// Where to write it. Defaults to the slot's own name in the working directory.
        out: Option<std::path::PathBuf>,
    },
    /// Download every populated IR slot into a directory. **Reads only.**
    IrExportAll { dir: std::path::PathBuf },
    /// Upload a WAV into a user IR slot (op 9 + op 13). **WRITES DEVICE FLASH.**
    ///
    /// User data, not firmware, so the risk class is `save`'s — but unlike a preset edit it does
    /// not sit in the edit buffer and cannot be undone by reloading. Export the slot first if it
    /// holds anything you want to keep.
    ///
    /// The file is resampled to nothing: it is truncated or zero-padded to 2048 samples — the
    /// longest the device stores — and its first channel is taken. A file that is not already
    /// 48 kHz is refused unless --force.
    IrUpload {
        slot: i64,
        wav: std::path::PathBuf,
        /// Name to store, up to 31 characters. Defaults to the file's stem.
        #[arg(long)]
        name: Option<String>,
        /// Replace an IR already in the slot.
        #[arg(long)]
        overwrite: bool,
        /// Upload anyway when the file's sample rate is not 48 kHz.
        #[arg(long)]
        force: bool,
    },
    /// Empty a user IR slot (op 15). **WRITES DEVICE FLASH.**
    ///
    /// Export it first if its contents matter — there is no undo.
    IrDelete { slot: i64 },
    /// Rename the IR in a slot (op 10). **WRITES DEVICE FLASH** — the name only.
    IrRename { slot: i64, name: String },
    /// PROBE: send an arbitrary opcode inside an IR session with a `{112: slot}` target.
    ///
    /// For mapping the IR op family — ops 9/11/12/13 are decoded, the rest are not. Prints
    /// whatever comes back, refusals included. Only send this at a slot you can afford to lose.
    IrProbe {
        op: i64,
        slot: i64,
        /// Add the `101: 2` companion key that ops 11 and 13 carry.
        #[arg(long)]
        kind: bool,
        /// Select the slot (op 12) before sending, as the blob stream requires.
        #[arg(long)]
        select: bool,
    },
    /// Read one device setting by id (op 24). **Reads only.**
    SettingGet { id: i64 },
    /// Set a device setting, matching the type the device already holds (op 24 then 25).
    ///
    /// Unlike `setting`, which always writes an integer, this reads the current value first — so it
    /// can write the float and bool settings too.
    SettingSet { id: i64, value: f64 },
    /// Dump the device-setting id space (op 24). **Reads only.**
    ///
    /// Write it to a file, change one thing on the pedal, dump again and diff: the id that moved
    /// is the setting you touched. That is how this namespace gets mapped.
    SettingsDump {
        /// Highest id to try.
        #[arg(long, default_value_t = 260)]
        max: i64,
        /// Write to this file instead of stdout.
        out: Option<std::path::PathBuf>,
    },
    /// Diff two `settings-dump` files and name the ids that changed. Offline.
    SettingsDiff {
        a: std::path::PathBuf,
        b: std::path::PathBuf,
    },
    /// Send an arbitrary edit op, for decoding ops we do not yet know. **Probe only.**
    ///
    /// `--set` takes `key=value` pairs for the target map, repeatable. Values parse as bool, then
    /// integer, then float, then string: `--set 102=1 --set 66=16711683 --set 109=Lead`.
    ///
    /// **This wedges pedals.** Op 58 with `{102:1, 66:255}` stopped an HX Stomp draining its bulk
    /// OUT endpoint and cost a power cycle, moments after the same op had *accepted* a shorter body
    /// (see `docs/safety.md`). Send one op with one body and then look at the device; never sweep.
    /// A power cycle discards the whole edit buffer, so save or abandon anything that matters
    /// first.
    ///
    /// A `-3` refusal means the op exists and the body is wrong, which is the result you want.
    ProbeEdit {
        #[arg(long)]
        op: i64,
        #[arg(long = "set", value_parser = parse_kv)]
        set: Vec<(i64, rmpv::Value)>,
    },
    /// Ask the device what a footswitch carries (op 33). The number is **one-based**: 1 = FS1.
    ReadSwitch { switch: i64 },
    /// Ask the device what drives one parameter (op 36).
    ReadAssign {
        slot: i64,
        param: i64,
        /// Read the paired cab's parameter namespace instead of the block's own.
        #[arg(long)]
        cab: bool,
    },
    /// Put a block's bypass on a footswitch (op 56). The switch number is **zero-based**: 0 = FS1.
    ///
    /// Edit-buffer only — reload the preset to undo, `save` to keep.
    AssignBypass { slot: i64, switch: i64 },
    /// Take a block's bypass off a footswitch (op 57). Zero-based, like `assign-bypass`.
    UnassignBypass { slot: i64, switch: i64 },
    /// Put a parameter under a controller (op 37).
    ///
    /// `source` is the controller ordinal: 0 none (removes it), 1-2 expression pedals,
    /// 3-7 footswitches (3 = FS1), 8 MIDI, 9 snapshots. Edit-buffer only.
    AssignParam {
        slot: i64,
        param: i64,
        source: i64,
        /// Assign the paired cab's parameter instead of the block's own.
        #[arg(long)]
        cab: bool,
    },
    /// Set one end of an existing assignment's travel (ops 65/66).
    ///
    /// The value is in the parameter's own units, the same ones `set` takes.
    AssignTravel {
        slot: i64,
        param: i64,
        /// Which end to move.
        end: TravelEnd,
        value: f32,
        /// Address the paired cab's parameter.
        #[arg(long)]
        cab: bool,
    },
    /// Send one hand-built edit-channel frame and print the reply.
    ///
    /// For poking the live sequence without recompiling. All three arguments are hex.
    Probe {
        cmd_hex: String,
        arg_hex: String,
        body_hex: String,
    },
}

/// The log filter, with `nusb` damped unless the user asked for it by name.
///
/// `RUST_LOG=debug` turns on nusb's per-URB tracing, which is **94% of a bug-report log** by volume
/// (7.2 MB of a Floor session's 7.7 MB) and buries the protocol lines a report is actually about.
/// An explicit `nusb=…` directive still wins, so `RUST_LOG=debug,nusb=debug` gets the URBs back.
fn log_filter() -> tracing_subscriber::EnvFilter {
    match std::env::var("RUST_LOG") {
        Ok(v) if !v.is_empty() => tracing_subscriber::EnvFilter::new(if v.contains("nusb") {
            v
        } else {
            format!("{v},nusb=warn")
        }),
        _ => tracing_subscriber::EnvFilter::new("info,nusb=warn"),
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(log_filter())
        .init();
    // After `parse`, not before: `--version` and `--help` exit inside it, and neither wants a log
    // line on top of its output.
    let cli = Cli::parse();
    // First line of every real run, so a pasted log says which build produced it.
    tracing::info!(
        version = fretwire_core::VERSION,
        commit = fretwire_core::BUILD_ID,
        "fretwire-cli starting"
    );

    match cli.command.unwrap_or(Command::Detect) {
        Command::Detect => match fretwire_usb::present_devices() {
            Ok(found) if found.is_empty() => println!("no HX device found"),
            Ok(found) => {
                for d in found {
                    let note = match d.support {
                        fretwire_usb::Support::Verified => String::new(),
                        fretwire_usb::Support::Reported => " (reported working, unverified)".into(),
                        fretwire_usb::Support::Untested => " (untested device)".into(),
                    };
                    println!("{}: present{note}", d.name);
                }
            }
            Err(e) => println!("usb error: {e}"),
        },
        Command::ShowPreset { stream } => show_preset(&stream)?,
        Command::DecodeEdit { hex } => decode_edit(&hex)?,
        // ---- live device commands (need Linux + the pedal) ----
        Command::Connect => {
            let s = fretwire_core::Session::connect()?;
            let d = s.device();
            println!(
                "connected to {} — interface claimed and handshake completed",
                d.name
            );
            if let (Some(dsps), Some(snaps)) = (d.dsps, d.snapshots) {
                println!("  {dsps} DSP(s), {snaps} snapshots per preset");
            }
            let _s = s;
            // `_s` drops here, which runs the clean session teardown (see `disconnect`).
        }
        Command::Disconnect => {
            // Connect then immediately tear down — isolates the teardown so you can confirm the
            // pedal returns to standalone (no "panel lock") after our software lets go.
            let mut s = fretwire_core::Session::connect()?;
            s.close()?;
            println!("disconnected — session-close sent on all channels; pedal back to standalone");
        }
        Command::Watch { secs } => {
            // Same 250 ms beat the GUI's heartbeat uses, so what shows up here is exactly what the
            // GUI would see. `FRETWIRE_TRACE_STATUS=1` additionally logs every frame body, decoded
            // or not — that is the setting for identifying a push we don't parse yet.
            let mut s = fretwire_core::Session::connect()?;
            println!("watching for {secs}s — touch the pedal (footswitch, snapshot, knob)…");
            let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
            let start = std::time::Instant::now();
            let mut seen = 0usize;
            let mut idle = 0usize;
            while std::time::Instant::now() < until {
                for p in s.poll_events()? {
                    // The idle mirror arrives ~3 times a second and says nothing changed, so it is
                    // counted rather than printed — otherwise it buries the one event you started
                    // this command to see. The count still earns its place: a live channel that
                    // goes quiet is what the ~4 KiB push-window stall looked like, and "0 idle"
                    // tells you the difference between a still pedal and a dead channel.
                    if matches!(p, fretwire_data::stream::StatusPush::Idle) {
                        idle += 1;
                        continue;
                    }
                    seen += 1;
                    println!("  [{:>6.2}s] {p:?}", start.elapsed().as_secs_f32());
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            println!("{seen} push(es) seen ({idle} idle mirrors, not shown)");
            s.close()?;
        }
        Command::Pull => {
            // Read the currently-loaded preset live and print it, including the snapshot
            // diagnosis — so comparing what the pedal's screen shows against what the preset
            // stores is a single command with no file juggling.
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.read_preset()?;
            print_preset(&preset);
            if let Some(raw) = s.last_raw() {
                let raw = raw.to_vec();
                print_snapshot_diagnosis(&raw);
            }
            // The authority: key 92 of the read-info reply, which is what `active_snapshot` above
            // now reflects. Printed alongside so a mismatch with the blob is visible at a glance.
            match preset.current.as_ref().and_then(|c| c.snapshot) {
                Some(live) => println!("  => device reports live snapshot {live} (key 92)"),
                None => println!("  => device reported no live snapshot (key 92 absent)"),
            }
        }
        Command::Bypass { slot, state } => {
            let bypassed = state == OnOff::On;
            let mut s = fretwire_core::Session::connect()?;
            s.set_enabled(slot, !bypassed)?;
            println!(
                "slot {slot} bypass -> {}  (block {})",
                if bypassed { "on" } else { "off" },
                if bypassed { "off" } else { "on" }
            );
        }
        Command::Set {
            slot,
            param_index,
            value,
        } => {
            let mut s = fretwire_core::Session::connect()?;
            s.set_param(slot, param_index, value)?;
            println!("slot {slot} param[{param_index}] -> {value}");
        }
        Command::Trails { slot, state } => {
            let on = state == OnOff::On;
            let mut s = fretwire_core::Session::connect()?;
            s.set_trails(slot, on)?;
            println!("slot {slot} trails -> {}", if on { "on" } else { "off" });
        }
        Command::SetCab {
            slot,
            param_index,
            value,
        } => {
            let mut s = fretwire_core::Session::connect()?;
            s.set_paired_param(slot, param_index, value)?;
            println!("slot {slot} cab param[{param_index}] -> {value}");
        }
        Command::Presets { bank } => {
            let mut s = fretwire_core::Session::connect()?;
            let bank = match bank {
                Some(b) => b,
                None => s.read_preset()?.current.map(|c| c.bank).unwrap_or(0),
            };
            let names = s.device().setlist_names();
            let label = names
                .get(bank as usize)
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            let presets = s.list_preset_entries_in(bank)?;
            println!("{} presets in bank {bank}{label}:", presets.len());
            let device = s.device();
            for p in &presets {
                // Slot first — it is what `goto`/`save` take — then how the pedal's own screen
                // writes it, where we know this device's banking. Reports paste this listing, so
                // having both means "the numbers don't line up" can be checked against the panel
                // without a second round trip.
                match device.preset_label(p.slot as i64) {
                    Some(l) => println!("  [{:>3}] {l}  {}", p.slot, p.name),
                    None => println!("  [{:>3}] {}", p.slot, p.name),
                }
            }
            // A row's stored key is the preset's index before it was last moved on the pedal. It
            // addresses nothing, but a disagreement is worth saying out loud: this is the device
            // state that used to make the editor list presets in the wrong order, and it is the
            // first thing to check when a remote report says the numbers don't line up.
            let base = bank * s.device().setlist_stride();
            let moved = presets.iter().filter(|p| p.key_disagrees(base)).count();
            if moved > 0 {
                println!(
                    "  note: {moved} of {} entries carry a stored key other than their slot — \
                     presets in this setlist have been reordered on the device. The slots above \
                     are the ones the pedal shows.",
                    presets.len()
                );
            }
        }
        Command::ShowBackup {
            backup: path,
            presets: verbose,
        } => {
            let bytes = std::fs::read(&path)?;
            let b = fretwire_data::hxb::Hxb::parse(&bytes)?;
            let device = fretwire_usb::DEVICES
                .iter()
                .find(|d| d.preset_device_id == Some(b.device_id))
                .map(|d| d.name)
                .unwrap_or("unknown device");
            println!("{path}");
            println!(
                "  device {device} ({:#010x}), fw {:#010x}, {} streams",
                b.device_id,
                b.device_version,
                b.streams.len()
            );
            if !b.comment.is_empty() {
                println!("  comment: {}", b.comment);
            }
            println!("  {} impulse responses", b.impulse_responses().len());
            for s in b.setlists() {
                println!(
                    "  [{}] {:<12} {}/{} slots used",
                    s.bank,
                    s.name,
                    s.populated(),
                    s.presets.len()
                );
                if verbose {
                    for p in s.presets.iter().flatten() {
                        println!("        [{:>3}] {}", p.index, p.name);
                    }
                }
            }
        }
        Command::DumpList { bank, out: path } => {
            let mut s = fretwire_core::Session::connect()?;
            let raw = s.list_presets_raw(bank)?;
            std::fs::write(&path, &raw)?;
            println!("wrote {} bytes of bank {bank}'s list to {path}", raw.len());
            for (i, name) in s.list_presets_in(bank)?.iter().take(8) {
                println!("  [{i:>3}] {name}");
            }
        }
        Command::Setlists => {
            // Name each setlist the connected device has, with the bank index `presets`/`goto` take.
            let mut s = fretwire_core::Session::connect()?;
            let current = s.read_preset()?.current.map(|c| c.bank);
            let names = s.device().setlist_names();
            println!("{} setlist(s) on the {}:", names.len(), s.device().name);
            for (i, name) in names.iter().enumerate() {
                let here = if Some(i as i64) == current {
                    "  <- current"
                } else {
                    ""
                };
                println!("  [{i}] {name}{here}");
            }
        }
        Command::Goto { preset, bank } => {
            let mut s = fretwire_core::Session::connect()?;
            s.goto_preset(bank, preset)?;
            println!("selected bank {bank} preset {preset}");
        }
        Command::Snapshot { index } => {
            let mut s = fretwire_core::Session::connect()?;
            s.set_snapshot(index)?;
            println!("switched to snapshot {index}");
        }
        // Re-reads after, since positions shift.
        Command::Move {
            src_slot: src,
            dst_slot: dst,
        } => {
            let mut s = fretwire_core::Session::connect()?;
            s.move_block(src, dst)?;
            let preset = s.read_preset()?;
            println!("moved slot {src} -> {dst}");
            print_preset(&preset);
        }
        // Re-reads to show the new block's default params.
        Command::AddBlock {
            slot,
            model_index: index,
            paired_index: paired,
        } => {
            let mut s = fretwire_core::Session::connect()?;
            s.add_block(slot, index, paired)?;
            let preset = s.read_preset()?;
            println!("added model index {index} at slot {slot}");
            print_preset(&preset);
        }
        Command::WriteRoundtrip => {
            // PROBE: read the preset and write it back unchanged via op 21, then re-read. Safe — it
            // touches the edit buffer only (reversible by reloading) and changes nothing. The first
            // hardware test of the whole-preset-write path.
            eprintln!(
                "op-21 write probe: re-writing the current preset UNCHANGED (edit buffer only,"
            );
            eprintln!(
                "reversible by reloading the preset). Watch RUST_LOG=trace for the chunk frames."
            );
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.rewrite_preset_unchanged()?;
            println!("round-trip complete — re-read preset:");
            print_preset(&preset);
        }
        // op 78 begin-structural first, as HX Edit does. Edit buffer only; reload to undo.
        Command::DeleteBlock { slot } => {
            eprintln!(
                "deleting block at slot {slot} via op-28 (surgical; keeps footswitch layout)."
            );
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.delete_block(slot)?;
            println!("deleted slot {slot}:");
            print_preset(&preset);
        }
        Command::MoveToRow {
            src_slot: src,
            row,
            pos,
        } => {
            let par = row == Row::Parallel;
            let pos = match pos.as_str() {
                "end" => usize::MAX,
                n => n.parse().map_err(|e| {
                    anyhow::anyhow!("bad pos {n:?}: {e} (expected a number or 'end')")
                })?,
            };
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.move_block_to_row(src, par, pos)?;
            println!(
                "moved slot {src} to the {} row at {}:",
                if par { "parallel" } else { "series" },
                // `end` is carried as usize::MAX; printing that raw is just noise.
                if pos == usize::MAX {
                    "the end".to_string()
                } else {
                    format!("pos {pos}")
                }
            );
            print_preset(&preset);
        }
        Command::BeforeSplit { src_slot: src } => {
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.move_before_split(src)?;
            println!("moved slot {src} before the split:");
            print_preset(&preset);
        }
        Command::NodePos { which, pos } => {
            use fretwire_core::fretwire_data::stream::slot_kind;
            let kind = match which.to_ascii_lowercase().as_str() {
                "split" => slot_kind::SPLIT,
                "mixer" | "join" => slot_kind::MIXER,
                other => anyhow::bail!("unknown node {other:?} — use `split` or `mixer`"),
            };
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.set_node_pos(0, kind, pos)?;
            println!("moved the {which} node to column {pos}:");
            print_preset(&preset);
        }
        Command::SplitType { which } => {
            let index: i64 = match which.as_str() {
                "y" | "Y" => 257,
                "ab" | "AB" => 256,
                "xover" | "crossover" => 258,
                "dyn" | "dynamic" => 563,
                other => other.parse().map_err(|_| {
                    anyhow::anyhow!("usage: fretwire split-type <y|ab|xover|dyn|index>")
                })?,
            };
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.read_preset()?;
            // DSP 0's split node — the only one a single-DSP device has. A two-DSP device has one
            // per DSP; retyping the second would need a `--dsp` flag.
            let slot = preset
                .split_node()
                .map(|n| n.slot)
                .ok_or_else(|| anyhow::anyhow!("preset is not split — no split node to retype"))?;
            let preset = s.set_split_type(slot, index)?;
            println!("split type -> {index} (slot {slot}):");
            print_preset(&preset);
        }
        Command::Rename { slot, name, bank } => {
            let mut s = fretwire_core::Session::connect()?;
            s.rename_preset(bank, slot, &name)?;
            println!(
                "renamed bank {bank} slot {slot} to {name:?} (name-only; edit buffer not saved)"
            );
        }
        Command::Swap {
            slot,
            model_index: index,
            paired_index: paired,
        } => {
            let mut s = fretwire_core::Session::connect()?;
            // DSP-fit check: read the current preset and project the swap's load. Against **this
            // block's DSP**, not the whole preset — each DSP is budgeted on its own, so summing
            // both on a Floor warned about presets that fit fine and stayed quiet about ones that
            // didn't. A warning only; the device is the final arbiter (see DSP_CEILING).
            let projected = match s.read_preset() {
                Ok(preset) => {
                    let block = preset.blocks.iter().find(|b| b.slot == slot);
                    let dsp = block.map(|b| b.dsp).unwrap_or(0);
                    let cur = preset.dsp_load_on(dsp);
                    let old = block.and_then(|b| b.dsp_load).unwrap_or(0.0);
                    let new = s
                        .catalog()
                        .model_load_by_index(index)
                        .map(|l| l + s.catalog().model_load_by_index(paired).unwrap_or(0.0));
                    new.map(|n| (cur, cur - old + n))
                }
                Err(e) => {
                    tracing::debug!("fit check skipped (read failed: {e})");
                    None
                }
            };
            if let Some((_cur, proj)) = projected
                && proj > fretwire_core::editor::DSP_CEILING
            {
                eprintln!(
                    "⚠  projected DSP ~{:.0}% of capacity [{proj:.1} of ~{:.0} raw] — past what \
                     this pedal accepts, so expect a `-306` refusal.",
                    fretwire_core::editor::dsp_percent(proj),
                    fretwire_core::editor::DSP_CEILING
                );
            }
            s.swap_model(slot, index, paired)?;
            let delta = projected
                .map(|(cur, proj)| format!("  (DSP ~{cur:.1}% -> ~{proj:.1}%)"))
                .unwrap_or_default();
            println!(
                "slot {slot} -> model index {index}{}{delta}",
                if paired >= 0 {
                    format!(" (paired cab {paired})")
                } else {
                    String::new()
                }
            );
        }
        Command::RenameSnapshot { index, name } => {
            let mut s = fretwire_core::Session::connect()?;
            s.rename_snapshot(index, &name)?;
            println!("snapshot {index} renamed to {name:?}");
        }
        Command::Setting { id, value } => {
            let mut s = fretwire_core::Session::connect()?;
            s.set_setting(id, value)?;
            println!("setting {id} -> {value}  (op 25; id space partly mapped)");
        }
        Command::Save { slot, name, bank } => {
            eprintln!("⚠  PERSISTENT WRITE: overwriting bank {bank} slot {slot} with the current");
            eprintln!("   edit buffer as {name:?}. Back up first; use a scratch slot to test.");
            let mut s = fretwire_core::Session::connect()?;
            s.save_preset(bank, slot, &name)?;
            println!("saved current edit buffer to bank {bank} slot {slot} as {name:?}");
        }
        Command::ReadSlot { bank, slot, out } => {
            let mut s = fretwire_core::Session::connect()?;
            let Some(raw) = s.read_preset_at(bank, slot)? else {
                println!("bank {bank} slot {slot}: device answered nil — no document to stream");
                s.close()?;
                return Ok(());
            };
            println!("read {} bytes from bank {bank} slot {slot}", raw.len());
            match out {
                Some(path) => {
                    std::fs::write(&path, &raw)?;
                    println!("wrote {path}");
                }
                None => {
                    let ps = fretwire_data::stream::PresetStream::parse(&raw)?;
                    println!("  blocks: {}", ps.loaded_blocks().len());
                }
            }
            // Prove the read was non-destructive: whatever the pedal was showing, it still is.
            if let Some(id) = s.read_identity()? {
                println!(
                    "  pedal still on: [{}] {} (bank {})",
                    id.index, id.name, id.bank
                );
            }
            s.close()?;
        }
        Command::DumpRaw { out: path } => {
            let mut s = fretwire_core::Session::connect()?;
            let raw = s.read_preset_raw()?;
            std::fs::write(&path, &raw)?;
            // Say *which* preset came out. This dumps whatever is loaded, and the filename is no
            // evidence of that: a tester meaning to capture three presets sent three dumps of one,
            // and it took a byte-level diff to notice (2026-08-02). Navigate with `goto` first.
            let who = s
                .last_identity()
                .map(|i| format!("{} (bank {}, slot {})", i.name, i.bank, i.index))
                .unwrap_or_else(|| "unknown — the identity read failed".to_string());
            println!("wrote {} bytes to {path}", raw.len());
            println!("  preset: {who}");
        }
        Command::ExportSetlist {
            out: path,
            bank,
            all,
        } => {
            let mut s = fretwire_core::Session::connect()?;
            let names = s.device().setlist_names();
            let banks: Vec<i64> = if all {
                (0..names.len() as i64).collect()
            } else {
                vec![bank]
            };
            println!(
                "exporting {} (the pedal will step through every preset)…",
                banks
                    .iter()
                    .map(|b| names.get(*b as usize).copied().unwrap_or("Presets"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let backup = s.export_setlists(&banks, |p| {
                println!("  [{:>4}/{}] {}: {}", p.done, p.total, p.setlist, p.name);
                true
            })?;
            std::fs::write(&path, backup.to_json())?;
            println!("wrote {} presets to {path}", backup.presets.len());
        }
        Command::BackupShow { backup: path } => {
            let backup =
                fretwire_core::backup::Backup::from_json(&std::fs::read_to_string(&path)?)?;
            println!("{} — {} presets:", backup.device, backup.presets.len());
            for bank in backup.banks() {
                // A v1 file records no setlist names, and multi-setlist files are the only ones
                // where the heading earns its line.
                let label = backup.setlist_name(bank).unwrap_or("Presets");
                println!("  {label} (bank {bank}):");
                for p in backup.presets.iter().filter(|p| p.bank == bank) {
                    println!("    [{:>3}] {}  ({} bytes)", p.index, p.name, p.raw.len());
                }
            }
        }
        Command::Restore {
            backup: path,
            index,
            slot,
            bank,
        } => {
            let slot = slot.unwrap_or(index);
            let backup =
                fretwire_core::backup::Backup::from_json(&std::fs::read_to_string(&path)?)?;
            let entry = backup.preset(bank, index).ok_or_else(|| {
                anyhow::anyhow!(
                    "backup has no preset at bank {bank} index {index} \
                     (see: fretwire backup-show {path})"
                )
            })?;
            let mut s = fretwire_core::Session::connect()?;
            let current = s
                .list_presets_in(bank)?
                .into_iter()
                .find(|(i, _)| *i as i64 == slot)
                .map(|(_, n)| n)
                .unwrap_or_else(|| "?".into());
            eprintln!(
                "⚠  PERSISTENT WRITE: restoring {:?} into slot {slot}, overwriting {current:?}.",
                entry.name
            );
            let preset = s.restore_preset(&entry.raw, bank, slot, &entry.name)?;
            println!(
                "restored {:?} to slot {slot}; device now shows:",
                entry.name
            );
            print_preset(&preset);
        }
        Command::Tree {
            stream: path,
            depth,
        } => {
            let ps = fretwire_data::stream::PresetStream::parse(&std::fs::read(&path)?)?;
            println!("{}", fretwire_data::stream::summarize(&ps.preset, depth));
        }
        Command::DiffStream { a, b } => diff_stream(&a, &b)?,
        Command::IrList { all } => {
            let mut s = fretwire_core::Session::connect()?;
            // The directory is one request and carries each slot's stored hash; the sweep is 128
            // and is only worth it when the empty slots are the point.
            let slots = if all { s.ir_scan()? } else { s.ir_directory()? };
            let shown: Vec<_> = slots.iter().filter(|i| all || i.is_used()).collect();
            if shown.is_empty() {
                println!("no IRs loaded ({} slots read)", slots.len());
            }
            // The two listings answer with different fields — the directory carries each slot's
            // stored hash but no checksum or length, the sweep the reverse — so every column is
            // optional and blank where that listing has nothing to say.
            for i in shown {
                let len = match i.stored_samples() {
                    0 => String::new(),
                    n => format!("{n} smp"),
                };
                let sum = i
                    .checksum
                    .map_or_else(String::new, |c| format!("{c:#010x}"));
                // A populated slot whose samples are all zero is silence, not emptiness, and the
                // two look identical in every other column.
                let silent = if i.is_used() && i.checksum == Some(0) {
                    "  (silent)"
                } else {
                    ""
                };
                println!(
                    "{:>3}  {:<32} {len:>9}  {sum:<10}  {}{silent}",
                    i.index,
                    i.display_name(),
                    i.md5.as_deref().unwrap_or("")
                );
            }
        }
        Command::IrInfo { slot } => {
            let mut s = fretwire_core::Session::connect()?;
            match s.ir_info(slot)? {
                Some(i) => println!("{i:#?}"),
                None => println!("IR slot {slot}: no decodable reply"),
            }
        }
        Command::IrExport { slot, out } => {
            let mut s = fretwire_core::Session::connect()?;
            match s.ir_export(slot)? {
                Some((info, blob)) => {
                    let path = out.unwrap_or_else(|| ir_filename(&info));
                    std::fs::write(&path, fretwire_data::ir::to_wav(&blob))?;
                    println!(
                        "IR {slot} \"{}\" -> {} ({} samples, peak {:.3})",
                        info.name,
                        path.display(),
                        fretwire_data::ir::IR_SAMPLES,
                        fretwire_data::ir::peak(&blob)
                    );
                }
                None => println!("IR slot {slot} is empty"),
            }
        }
        Command::IrExportAll { dir } => {
            let mut s = fretwire_core::Session::connect()?;
            std::fs::create_dir_all(&dir)?;
            let slots = s.ir_directory()?;
            let used: Vec<i64> = slots
                .iter()
                .filter(|i| i.is_used())
                .map(|i| i.index)
                .collect();
            println!("{} IR(s) to export", used.len());
            for slot in used {
                match s.ir_export(slot)? {
                    Some((info, blob)) => {
                        let path = dir.join(ir_filename(&info));
                        std::fs::write(&path, fretwire_data::ir::to_wav(&blob))?;
                        println!("  {slot:>3}  {}", path.display());
                    }
                    // The directory said it was used, so this is worth a line rather than silence.
                    None => println!("  {slot:>3}  vanished between the listing and the read"),
                }
            }
        }
        Command::IrUpload {
            slot,
            wav,
            name,
            overwrite,
            force,
        } => {
            let bytes = std::fs::read(&wav)?;
            let (blob, rate) = fretwire_data::ir::from_wav(&bytes)
                .map_err(|e| anyhow::anyhow!("{}: {e}", wav.display()))?;
            if rate != fretwire_data::ir::IR_SAMPLE_RATE && !force {
                anyhow::bail!(
                    "{} is {rate} Hz; the device runs at {} Hz and nothing here resamples, so it \
                     would play short and bright. Convert it first, or pass --force",
                    wav.display(),
                    fretwire_data::ir::IR_SAMPLE_RATE
                );
            }
            let name = name.unwrap_or_else(|| {
                wav.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            let mut s = fretwire_core::Session::connect()?;
            let landed = s.ir_upload(slot, &name, &blob, overwrite)?;
            println!(
                "IR {slot} <- {} as \"{}\" (checksum {:#010x}, peak {:.3})",
                wav.display(),
                landed.name,
                landed.checksum.unwrap_or(0),
                fretwire_data::ir::peak(&blob)
            );
        }
        Command::IrDelete { slot } => {
            let mut s = fretwire_core::Session::connect()?;
            let before = s.ir_info(slot)?;
            if let Some(b) = &before
                && b.is_used()
            {
                println!("emptying slot {slot}, which held \"{}\"", b.display_name());
            }
            s.ir_delete(slot)?;
            println!("IR {slot} emptied");
        }
        Command::IrRename { slot, name } => {
            let mut s = fretwire_core::Session::connect()?;
            let landed = s.ir_rename(slot, &name)?;
            println!("IR {slot} is now \"{}\"", landed.name);
        }
        Command::IrProbe {
            op,
            slot,
            kind,
            select,
        } => {
            let mut s = fretwire_core::Session::connect()?;
            match s.ir_probe(op, slot, kind, select)? {
                Some(v) => println!("op {op} slot {slot} -> {v}"),
                None => println!("op {op} slot {slot} -> no decodable reply"),
            }
        }
        Command::SettingGet { id } => {
            let mut s = fretwire_core::Session::connect()?;
            match s.read_setting(id)? {
                // The type is not cosmetic: a write whose value type differs from what the device
                // already holds is refused with -3, so the probe has to show it.
                Some(v) => println!("{id} = {v}  [{}]{}", value_type(&v), setting_gloss(id)),
                None => println!("{id}: no value (the device does not implement it)"),
            }
        }
        Command::SettingSet { id, value } => {
            let mut s = fretwire_core::Session::connect()?;
            match s.set_setting_num(id, value)? {
                Some(v) => println!("{id} = {v}  [{}]{}", value_type(&v), setting_gloss(id)),
                None => println!("{id}: wrote, but the device reports no value back"),
            }
        }
        Command::SettingsDump { max, out } => {
            let mut s = fretwire_core::Session::connect()?;
            let found = s.scan_settings(0..=max);
            let mut text = String::new();
            for (id, v) in &found {
                text.push_str(&format!("{id}\t{v}\n"));
            }
            match out {
                Some(path) => {
                    std::fs::write(&path, &text)?;
                    println!(
                        "{} of {} ids answered -> {}",
                        found.len(),
                        max + 1,
                        path.display()
                    );
                }
                None => {
                    print!("{text}");
                    println!("# {} of {} ids answered", found.len(), max + 1);
                }
            }
        }
        Command::SettingsDiff { a, b } => {
            let read = |p: &std::path::Path| -> anyhow::Result<Vec<(String, String)>> {
                Ok(std::fs::read_to_string(p)?
                    .lines()
                    .filter(|l| !l.starts_with('#') && !l.is_empty())
                    .filter_map(|l| l.split_once('\t'))
                    .map(|(i, v)| (i.to_string(), v.to_string()))
                    .collect())
            };
            let (before, after) = (read(&a)?, read(&b)?);
            let lookup: std::collections::HashMap<_, _> = before.iter().cloned().collect();
            let mut changed = 0;
            for (id, now) in &after {
                match lookup.get(id) {
                    Some(was) if was != now => {
                        changed += 1;
                        let n: i64 = id.parse().unwrap_or(-1);
                        println!("{id}: {was} -> {now}{}", setting_gloss(n));
                    }
                    None => {
                        changed += 1;
                        println!("{id}: (absent) -> {now}");
                    }
                    _ => {}
                }
            }
            if changed == 0 {
                println!("no setting changed between the two dumps");
            }
        }
        Command::ProbeEdit { op, set } => {
            let mut s = fretwire_core::Session::connect()?;
            let target: Vec<(rmpv::Value, rmpv::Value)> = set
                .into_iter()
                .map(|(k, v)| (rmpv::Value::from(k), v))
                .collect();
            println!("op {op} target {target:?}");
            match s.probe_edit(op, target) {
                Ok(Some(v)) => println!("  accepted: {v}"),
                Ok(None) => println!("  accepted, empty reply"),
                Err(e) => println!("  refused: {e}"),
            }
        }
        Command::ReadSwitch { switch } => {
            let mut s = fretwire_core::Session::connect()?;
            match s.read_switch(switch)? {
                Some(v) => println!("FS{switch}: {v}"),
                None => println!("FS{switch}: no decodable reply"),
            }
        }
        Command::ReadAssign { slot, param, cab } => {
            let mut s = fretwire_core::Session::connect()?;
            match s.read_assignment(slot, cab, param)? {
                Some(v) => println!(
                    "slot {slot} param {param}{}: {v}",
                    if cab { " (cab)" } else { "" }
                ),
                None => println!("slot {slot} param {param}: no decodable reply"),
            }
        }
        Command::AssignBypass { slot, switch } => {
            let mut s = fretwire_core::Session::connect()?;
            s.assign_bypass_to_switch(slot, switch)?;
            // Read back rather than announce: this is the same immediate re-read the GUI's command
            // layer does, so if the device ever ACKed ahead of rewriting the document the CLI would
            // show it too, instead of leaving it for someone to find in the UI.
            let p = s.read_preset()?;
            let on = p
                .blocks
                .iter()
                .find(|b| b.slot == slot)
                .map(|b| b.footswitch);
            println!(
                "slot {slot} bypass -> FS{} (reads back as {:?})",
                switch + 1,
                on
            );
        }
        Command::UnassignBypass { slot, switch } => {
            let mut s = fretwire_core::Session::connect()?;
            s.unassign_bypass_from_switch(slot, switch)?;
            println!("slot {slot} bypass off FS{}", switch + 1);
        }
        Command::AssignParam {
            slot,
            param,
            source,
            cab,
        } => {
            let mut s = fretwire_core::Session::connect()?;
            s.assign_param(slot, cab, param, source)?;
            let p = s.read_preset()?;
            let found = p
                .assignments
                .iter()
                .find(|a| a.target_slot == Some(slot) && a.param_index == Some(param));
            println!(
                "slot {slot} param {param} -> source {source} (reads back as {:?})",
                found.map(|a| a.controller)
            );
        }
        Command::AssignTravel {
            slot,
            param,
            end,
            value,
            cab,
        } => {
            let mut s = fretwire_core::Session::connect()?;
            s.set_assign_travel(slot, cab, param, end == TravelEnd::Max, value)?;
            println!("slot {slot} param {param} {end:?} = {value}");
        }
        Command::Probe {
            cmd_hex,
            arg_hex,
            body_hex,
        } => {
            let cmd_b = u8::from_str_radix(&strip_hex(&cmd_hex), 16)
                .map_err(|e| anyhow::anyhow!("bad cmd hex {cmd_hex:?}: {e}"))?;
            let arg = u32::from_str_radix(&strip_hex(&arg_hex), 16)
                .map_err(|e| anyhow::anyhow!("bad arg hex {arg_hex:?}: {e}"))?;
            let body = parse_hex(&body_hex)?;
            use fretwire_core::fretwire_protocol::{Frame, channel};
            let (src, dst) = channel::EDIT;
            let mut s = fretwire_core::Session::connect()?;
            let reply = s.request(&Frame::new(src, dst, 0, cmd_b, arg, body))?;
            println!(
                "reply cmd={:#04x} arg={:#010x} dst={:#06x} body={:02x?}",
                reply.cmd, reply.arg, reply.dst, reply.body
            );
        }
        Command::ImportData { source } => import_data(&source)?,
        Command::InstallUdev { print } => {
            if print {
                print!("{UDEV_RULE}");
            } else {
                install_udev()?;
            }
        }
    }
    Ok(())
}

/// The udev rule, embedded at build time from the canonical copy in `packaging/` so `install-udev`
/// works from an installed binary (no dependency on the source tree at runtime).
const UDEV_RULE: &str = include_str!("../../../packaging/70-hxstomp.rules");
const UDEV_RULE_PATH: &str = "/etc/udev/rules.d/70-hxstomp.rules";

/// Install the udev rule that grants the logged-in user access to the HX Stomp's raw USB node,
/// then reload udev so it takes effect on the next replug. Writes directly when run as root;
/// otherwise re-runs the privileged steps through `sudo`. Falls back to printing manual
/// instructions if the rule can't be installed automatically.
fn install_udev() -> Result<()> {
    use std::path::Path;
    let target = Path::new(UDEV_RULE_PATH);

    // Try a direct write first — succeeds when already root, and is the only step that needs it.
    match std::fs::write(target, UDEV_RULE) {
        Ok(()) => {
            println!("wrote {UDEV_RULE_PATH}");
            reload_udev(run_status);
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            println!("Installing the udev rule needs root — running the install via sudo.");
            println!("(You may be prompted for your password.)\n");
            if let Err(err) = install_udev_via_sudo() {
                eprintln!("\nCouldn't install automatically: {err}\n");
                print_udev_manual();
                return Err(err);
            }
        }
        Err(e) => return Err(e.into()),
    }

    println!(
        "\n\u{2713} udev rule installed. Unplug and replug your HX Stomp for it to take effect."
    );
    Ok(())
}

/// Perform the privileged install in one `sudo` shell: stage the rule to a user-owned temp file,
/// then `install` it into place and reload udev. One password prompt for the whole sequence.
fn install_udev_via_sudo() -> Result<()> {
    use std::process::Command;
    let tmp = std::env::temp_dir().join(format!("70-hxstomp-{}.rules", std::process::id()));
    std::fs::write(&tmp, UDEV_RULE)?;
    let script = format!(
        "install -m 0644 {tmp} {target} && udevadm control --reload && udevadm trigger",
        tmp = shell_quote(&tmp.to_string_lossy()),
        target = shell_quote(UDEV_RULE_PATH),
    );
    let status = Command::new("sudo")
        .arg("sh")
        .arg("-c")
        .arg(&script)
        .status();
    let _ = std::fs::remove_file(&tmp);
    match status {
        Ok(st) if st.success() => Ok(()),
        Ok(st) => anyhow::bail!("sudo install failed (exit {st})"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("`sudo` not found on PATH")
        }
        Err(e) => Err(e.into()),
    }
}

/// Reload udev's rules and re-trigger, via `runner` (so the root path and tests can vary it).
/// Best-effort: a failed reload is a warning, not fatal — the file is already written and a
/// replug (or manual reload) will pick it up.
fn reload_udev(mut runner: impl FnMut(&str, &[&str]) -> std::io::Result<std::process::ExitStatus>) {
    for args in [["control", "--reload"], ["trigger", ""]] {
        let args: Vec<&str> = args.iter().copied().filter(|a| !a.is_empty()).collect();
        match runner("udevadm", &args) {
            Ok(st) if st.success() => {}
            Ok(st) => eprintln!("⚠  `udevadm {}` exited {st}", args.join(" ")),
            Err(e) => eprintln!("⚠  couldn't run `udevadm {}`: {e}", args.join(" ")),
        }
    }
}

fn run_status(prog: &str, args: &[&str]) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new(prog).args(args).status()
}

/// Minimal single-quote shell escaping for embedding a path in the `sudo sh -c` script.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn print_udev_manual() {
    println!("Install it manually:");
    println!("  sudo tee {UDEV_RULE_PATH} > /dev/null <<'EOF'");
    print!("{UDEV_RULE}");
    println!("EOF");
    println!("  sudo udevadm control --reload && sudo udevadm trigger");
    println!("Then unplug and replug your HX Stomp.");
}

/// Import Line 6's reference data into the local data dir from a **user-supplied** source (an HX
/// Edit installer, or a directory of already-extracted files). The mechanics live in
/// `fretwire_core::import` so the GUI's first-run screen can offer the same thing; this just prints.
fn import_data(source: &str) -> Result<()> {
    let summary = fretwire_core::import::import_from(std::path::Path::new(source))?;
    println!(
        "imported {} reference file(s) → {}",
        summary.copied,
        summary.dest.display()
    );
    for name in &summary.missing {
        eprintln!("⚠  {name} missing — model names/ordering won't be available");
    }
    println!("(set $FRETWIRE_DATA_DIR to override that location)");
    println!("the tool now loads model names, DSP loads and param ranges from here at runtime.");
    Ok(())
}

/// Strip non-hex-digit characters from a string (spaces, `0x`, commas).
fn strip_hex(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_hexdigit()).collect()
}

/// Parse a loose hex string (spaces/separators allowed) into bytes.
fn parse_hex(hex: &str) -> Result<Vec<u8>> {
    let clean = strip_hex(hex);
    anyhow::ensure!(
        clean.len().is_multiple_of(2),
        "hex has an odd number of digits"
    );
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).map_err(Into::into))
        .collect()
}

/// Decode a single edit-command body (the MessagePack `data` after the TLV header) from hex.
/// Handy for labeling single-knob captures: dump the body bytes, paste here, read the param key.
fn decode_edit(hex: &str) -> Result<()> {
    let bytes = parse_hex(hex)?;

    use fretwire_core::fretwire_protocol::edit::{OP_BYPASS, OP_SET_VALUE};
    let e = fretwire_core::fretwire_protocol::EditBody::parse(&bytes)?;
    let op = match e.op {
        OP_BYPASS => "bypass".to_string(),
        OP_SET_VALUE => "set-value".to_string(),
        n => format!("op {n} (unknown)"),
    };
    println!("op        : {op}  (envelope key 100 = {})", e.op);
    println!("txn       : 0x{:04x}  (key 102, running counter)", e.txn);
    println!("slot      : {:?}  (target key 98)", e.slot);
    if let Some(i) = e.param_index {
        println!("param idx : {i}  (target key 28 = index in the model's device param order)");
    }
    match e.value {
        fretwire_core::fretwire_protocol::EditValue::Bool(b) => println!("value     : {b}"),
        fretwire_core::fretwire_protocol::EditValue::Float(f) => println!("value     : {f}"),
        fretwire_core::fretwire_protocol::EditValue::Int(i) => println!("value     : {i}"),
        fretwire_core::fretwire_protocol::EditValue::None => println!("value     : (none found)"),
    }
    println!("\nfull decoded map: {:?}", e.raw);
    Ok(())
}

/// Find and print the paths in the preset tree that differ between two saved raw streams. Used to
/// decode which key encodes a given device state (e.g. block bypass).
fn diff_stream(a_path: &str, b_path: &str) -> Result<()> {
    use fretwire_data::stream::PresetStream;
    let a = PresetStream::parse(&std::fs::read(a_path)?)?;
    let b = PresetStream::parse(&std::fs::read(b_path)?)?;
    let mut diffs = Vec::new();
    diff_values(&a.preset, &b.preset, String::new(), &mut diffs);
    if diffs.is_empty() {
        println!("no differences in the preset tree");
    } else {
        println!("{} differing path(s):", diffs.len());
        for d in diffs {
            println!("  {d}");
        }
    }
    Ok(())
}

/// Recursively compare two MessagePack values, recording `path: a -> b` for each leaf difference.
fn diff_values(a: &rmpv::Value, b: &rmpv::Value, path: String, out: &mut Vec<String>) {
    use rmpv::Value::*;
    match (a, b) {
        (Map(am), Map(bm)) => {
            let key = |m: &Vec<(rmpv::Value, rmpv::Value)>, k: &rmpv::Value| {
                m.iter().find(|(mk, _)| mk == k).map(|(_, v)| v.clone())
            };
            for (k, av) in am {
                let kp = format!("{path}/{k}");
                match key(bm, k) {
                    Some(bv) => diff_values(av, &bv, kp, out),
                    None => out.push(format!("{kp}: {av} -> (absent)")),
                }
            }
            for (k, bv) in bm {
                if key(am, k).is_none() {
                    out.push(format!("{path}/{k}: (absent) -> {bv}"));
                }
            }
        }
        (Array(aa), Array(ba)) => {
            for (i, av) in aa.iter().enumerate() {
                match ba.get(i) {
                    Some(bv) => diff_values(av, bv, format!("{path}[{i}]"), out),
                    None => out.push(format!("{path}[{i}]: {av} -> (absent)")),
                }
            }
            for (i, item) in ba.iter().enumerate().skip(aa.len()) {
                out.push(format!("{path}[{i}]: (absent) -> {}", item));
            }
        }
        _ => {
            if a != b {
                out.push(format!("{path}: {a} -> {b}"));
            }
        }
    }
}

/// Decode a reassembled device preset stream (from a file) and print it as a block/param tree.
fn show_preset(path: &str) -> Result<()> {
    use fretwire_core::editor::Catalog;
    let stream = std::fs::read(path)?;
    let preset = Catalog::load()?.load_preset(&stream)?;
    print_preset(&preset);
    print_snapshot_diagnosis(&stream);
    Ok(())
}

/// Print each snapshot's stored scene next to the preset's **live** block state, and say whether
/// the stored active index agrees with it.
///
/// This exists to settle an open question (see `docs/helix-floor.md`): preset key `10 → 8` claims
/// an active snapshot, but in `dual_amp_stream` it names one whose scene is *not* what the preset
/// actually has loaded. We cannot tell from captures alone whether the index or the scene is the
/// truth — so dump a preset from a device parked on a *known* snapshot and read this off:
///
///   * "stored active index agrees" → key `8` is right; the GUI's current behaviour is correct.
///   * the scene-match line names the snapshot the pedal really shows → key `8` is unreliable and
///     matching the scene is the fix.
fn print_snapshot_diagnosis(stream: &[u8]) {
    use fretwire_data::stream::PresetStream;
    let Ok(ps) = PresetStream::parse(stream) else {
        return;
    };
    let snaps = ps.snapshot_details();
    if snaps.is_empty() {
        return;
    }
    // Index by the **wire slot**, not the per-DSP index: `block_enabled` is one flat array over the
    // device's whole slot space (40 entries on a two-DSP Floor — [solid], pullmeunder dump), so on
    // DSP2 the per-DSP index reads 20 slots too low and reports DSP1's scene for a DSP2 block.
    let live: Vec<(usize, bool)> = ps
        .blocks()
        .iter()
        .filter(|b| b.is_block())
        .filter_map(|b| b.bypassed.map(|byp| (b.wire_slot() as usize, !byp)))
        .collect();
    let matches: Vec<usize> = snaps
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            live.iter()
                .all(|&(slot, on)| s.block_enabled.get(slot) == Some(&on))
        })
        .map(|(i, _)| i)
        .collect();
    let stored = ps.snapshots().0;
    println!("\nsnapshots (stored-in-blob active index: {stored:?})");
    for (i, s) in snaps.iter().enumerate() {
        let on: Vec<String> = live
            .iter()
            .map(|&(slot, _)| {
                let enabled = s.block_enabled.get(slot).copied().unwrap_or(false);
                format!("{slot}{}", if enabled { "+" } else { "-" })
            })
            .collect();
        println!(
            "  [{i}] {:<14} {}{}",
            s.name,
            on.join(" "),
            if matches.contains(&i) {
                "   <- matches the live scene"
            } else {
                ""
            }
        );
    }
    let live_str: Vec<String> = live
        .iter()
        .map(|&(slot, on)| format!("{slot}{}", if on { "+" } else { "-" }))
        .collect();
    println!("  live       {}", live_str.join(" "));
    match (stored, matches.as_slice()) {
        (Some(a), m) if m.contains(&(a as usize)) => {
            println!("  => stored active index agrees with the live scene");
        }
        (Some(a), [only]) => {
            println!("  => MISMATCH: stored index says {a}, but the live scene is snapshot {only}");
        }
        (Some(a), []) => println!("  => the live scene matches no snapshot (stored index: {a})"),
        (Some(a), m) => println!("  => ambiguous: stored {a}, scene matches {m:?}"),
        (None, _) => println!("  => no stored active index"),
    }
}

/// Print an editor preset as a block/param tree.
fn print_params(params: &[fretwire_core::editor::EditorParam]) {
    for p in params {
        // Display name first, but keep the `symbolicID` visible where it differs — a dump is what
        // you read when working out what to address, and that is the name edits are keyed by.
        let label = match p.display_name() {
            d if d == p.name => p.name.clone(),
            d => format!("{d} ({})", p.name),
        };
        println!(
            "       [{:>2}] {:<14} = {}{}",
            p.index,
            label,
            fmt_param(p),
            if p.settable {
                ""
            } else {
                "   (read-only — no confirmed address)"
            }
        );
    }
}

/// The split and mixer are real blocks with their own model and parameters — the mixer is
/// `HD2_AppDSPFlowJoin`, whose A/B levels, pans and polarity decide what you actually hear out of
/// the parallel path. They are the first thing to check when a block on one leg goes silent, and
/// until now a dump never showed them: you had to read key 17 out of the raw MessagePack.
fn print_routing_nodes(preset: &fretwire_core::EditorPreset) {
    for d in &preset.dsps {
        for (glyph, what, node, pos) in [
            ("⋔", "split", &d.split_node, d.split_pos),
            ("⋉", "mixer", &d.mixer_node, d.mixer_pos),
        ] {
            let Some(n) = node else { continue };
            let at = pos.map(|p| format!(" before col {p}")).unwrap_or_default();
            println!(
                "\n  DSP{} {glyph} {what}{at}  slot {}  [{}]",
                d.dsp + 1,
                n.slot,
                n.symbolic_id.as_deref().unwrap_or("unresolved"),
            );
            if n.params.is_empty() {
                println!("       (no parameters)");
            }
            print_params(&n.params);
        }
    }
}

fn print_preset(preset: &fretwire_core::EditorPreset) {
    match &preset.current {
        Some(i) => println!("Preset [{}] {}", i.index, i.name),
        None => println!("Preset — (current identity unknown)"),
    }
    println!(
        "device {} preset build {}",
        preset.device_model.as_deref().unwrap_or("?"),
        preset.build_stamp.as_deref().unwrap_or("?")
    );
    let topo = if preset.split() {
        "split (parallel)"
    } else {
        "serial"
    };
    // A two-DSP device budgets each DSP separately, so report them separately. Percentages are of
    // *capacity* — DSP_CEILING reads 100% — with the raw sums kept in brackets, because the block
    // costs printed below and every load figure in the docs are on the raw scale.
    use fretwire_core::editor::{DSP_CEILING as CEIL, dsp_percent as pc};
    let load = match preset.dsp_load_by_dsp().as_slice() {
        [(d, one)] => format!(
            "DSP {:.1}% used · {:.1}% free  [{one:.1} of ~{CEIL:.0} raw]",
            preset.dsp_percent_on(*d),
            pc(preset.dsp_free_on(*d)),
        ),
        many => format!(
            "{}  [raw {} of ~{CEIL:.0}]",
            many.iter()
                .map(|(d, _)| format!(
                    "DSP{} {:.1}% · {:.1}% free",
                    d + 1,
                    preset.dsp_percent_on(*d),
                    pc(preset.dsp_free_on(*d))
                ))
                .collect::<Vec<_>>()
                .join(" · "),
            many.iter()
                .map(|(_, l)| format!("{l:.1}"))
                .collect::<Vec<_>>()
                .join(" · "),
        ),
    };
    println!(
        "{} block(s) · {topo} topology · {load}",
        preset.blocks.len()
    );
    // Where each DSP's bracket actually sits. "Is this block inside the parallel path?" is the
    // question behind most of the field reports about blocks going silent, and answering it from a
    // dump used to mean decoding key 13 by hand.
    for d in &preset.dsps {
        if let (Some(sp), Some(mp)) = (d.split_pos, d.mixer_pos) {
            let inside: Vec<String> = d
                .grid
                .iter()
                .filter(|c| c.row == 1 && c.occupied)
                .map(|c| {
                    let where_ = if c.column < sp {
                        " before-split!"
                    } else if c.column >= mp {
                        " past-mixer!"
                    } else {
                        ""
                    };
                    format!("slot {} @col {}{where_}", c.slot, c.column)
                })
                .collect();
            println!(
                "  DSP{} bracket: split before col {sp}, mixer before col {mp} → path B spans cols \
                 {sp}..={}  [{}]",
                d.dsp + 1,
                mp - 1,
                inside.join(", ")
            );
        }
    }
    for b in &preset.blocks {
        let label = b
            .user_label
            .as_deref()
            .map(|l| format!(" \"{l}\""))
            .unwrap_or_default();
        let bypass = match b.bypassed {
            Some(true) => "  [bypassed]",
            _ => "",
        };
        let variant = b.variant.map(|v| format!(" {v}")).unwrap_or_default();
        if b.is_controller {
            println!(
                "\n  slot {:<2} {} [footswitch/controller assignment]",
                b.slot, b.model_name
            );
            continue;
        }
        let row = if b.row == 1 { " (row B)" } else { "" };
        let fs = if b.footswitch > 0 {
            format!("FS{} · ", b.footswitch)
        } else {
            String::new()
        };
        // Same scale as the header's percentage: two figures in one listing that add up
        // differently is a trap. The raw sum is in the header's brackets for `.models` lookups.
        let dsp = b
            .dsp_load
            .map(|l| format!("  ({:.1}% DSP)", fretwire_core::editor::dsp_percent(l)))
            .unwrap_or_default();
        println!(
            "\n  {}slot {:<2}{} {}{}  [{}]{}{}{}",
            fs,
            b.slot,
            row,
            b.model_name,
            label,
            b.symbolic_id.as_deref().unwrap_or("unresolved"),
            variant,
            bypass,
            dsp,
        );
        print_params(&b.params);
        if let Some(cab) = &b.paired_model_name {
            println!("       + cab: {cab}");
            print_params(&b.paired_params);
        }
    }
    print_routing_nodes(preset);
    if !preset.snapshot_names.is_empty() {
        let active = preset
            .active_snapshot
            .map(|i| format!(" (active: {i})"))
            .unwrap_or_default();
        println!("\nsnapshots{active}: {}", preset.snapshot_names.join(", "));
    }
    if !preset.assignments.is_empty() {
        println!(
            "\n{} footswitch/controller assignment(s):",
            preset.assignments.len()
        );
        for a in &preset.assignments {
            let param = a
                .param_index
                .map(|p| {
                    // A cab parameter is numbered in the cab's own list, so say which list.
                    format!(" {}param {p}", if a.paired() { "cab " } else { "" })
                })
                .unwrap_or_default();
            let slot = a
                .target_slot
                .map(|s| format!("slot {s}"))
                .unwrap_or_else(|| "?".into());
            let travel = match (&a.min, &a.max) {
                (Some(lo), Some(hi)) => format!("  [{lo} -> {hi}]"),
                _ => String::new(),
            };
            println!(
                "  {} -> {}{}{}",
                source_name(a.controller),
                slot,
                param,
                travel
            );
        }
    }
}

/// Name the physical control an assignment's source ordinal refers to.
///
/// FS1 = 3 and FS2 = 4 are [solid] — each was assigned on a Stomp and the document diffed. The rest
/// are inferred from that run being consecutive and from `tonepush`'s notes putting EXP1 at 1, so
/// anything unproven prints as a bare ordinal rather than a guess with a confident label on it.
fn source_name(ordinal: i64) -> String {
    match ordinal {
        // Footswitches are the run we have proven, both by diffing a front-panel assignment and by
        // writing one: FS1 = 3, and the device answers switches 1-5 and refuses 6.
        n @ 3..=7 => format!("FS{}", n - 2),
        // These three names are `tonepush`'s. Ordinals 1, 2 and 9 do file themselves at their own
        // index here, but nothing on a Stomp proves *which* control 1 and 2 are — 3..=7 being the
        // footswitches simply leaves the two expression inputs. Named rather than numbered because
        // a bare "controller 1" tells a reader less, and the caveat lives in `docs/preset-format.md`.
        1 => "EXP1".into(),
        2 => "EXP2".into(),
        8 => "MIDI".into(),
        9 => "Snapshots".into(),
        n => format!("controller {n}"),
    }
}

/// A parameter as a human reads it, with the raw value kept alongside because that is what
/// `fretwire set` takes: `1.373 s  [1.3728]`. An enum shows its label, a plain number shows alone.
fn fmt_param(p: &fretwire_core::editor::EditorParam) -> String {
    use fretwire_data::stream::ParamValue::*;
    let raw = fmt_value(p.value);
    let pretty = match p.value {
        Float(f) => p.meta.format.as_ref().and_then(|nf| nf.display(f.into())),
        Int(i) => p.meta.enum_label(i).map(str::to_string),
        Bool(_) => None,
    };
    match pretty {
        Some(s) if s != raw => format!("{s:<12}  [{raw}]"),
        _ => raw,
    }
}

fn fmt_value(v: fretwire_data::stream::ParamValue) -> String {
    use fretwire_data::stream::ParamValue::*;
    match v {
        Float(f) => format!("{f}"),
        Int(i) => format!("{i}"),
        Bool(b) => format!("{b}"),
    }
}

/// A filesystem-safe filename for an exported IR: its slot number and its own name.
///
/// The number leads so a directory sorts the way the device does, and it disambiguates the two
/// slots that a user has, inevitably, given the same name.
fn ir_filename(info: &fretwire_data::ir::IrSlot) -> std::path::PathBuf {
    let safe: String = info
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_ .+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim();
    if safe.is_empty() {
        format!("ir-{:03}.wav", info.index).into()
    } else {
        format!("ir-{:03} {safe}.wav", info.index).into()
    }
}

/// Name a MessagePack value's type, for the settings probe.
fn value_type(v: &fretwire_core::fretwire_data::rmpv::Value) -> &'static str {
    use fretwire_core::fretwire_data::rmpv::Value;
    match v {
        Value::Boolean(_) => "bool",
        Value::Integer(n) if n.is_i64() => "int",
        Value::Integer(_) => "uint",
        Value::F32(_) => "f32",
        Value::F64(_) => "f64",
        Value::String(_) => "string",
        Value::Binary(_) => "bytes",
        Value::Array(_) => "array",
        Value::Map(_) => "map",
        Value::Nil => "nil",
        Value::Ext(..) => "ext",
    }
}

/// A short note on the settings whose meaning is known, appended to a printed value.
///
/// The namespace is flat and numbered, and almost all of it is still unmapped — this names the
/// handful that are pinned down so a dump is not 147 anonymous numbers.
/// A short name for a setting id, where we have one.
///
/// The table lives in `fretwire_protocol::settings` so the CLI and the GUI name ids identically —
/// they used to carry separate lists, which is how `201`-`203` stayed glossed as "global EQ" here
/// after the real EQ block turned out to be 190-200.
/// Parse a `key=value` probe field. Values take the first type that fits: bool, integer, float,
/// then string — so `66=16711683` is an int and `109=Lead` a string, which is what those two keys
/// actually hold. A key that needs a type this can't express is a reason to write a real builder.
fn parse_kv(arg: &str) -> Result<(i64, rmpv::Value), String> {
    let (k, v) = arg
        .split_once('=')
        .ok_or_else(|| format!("expected key=value, got {arg:?}"))?;
    let key: i64 = k
        .trim()
        .parse()
        .map_err(|_| format!("{k:?} is not an integer key"))?;
    let v = v.trim();
    let value = match v {
        "true" => rmpv::Value::Boolean(true),
        "false" => rmpv::Value::Boolean(false),
        "nil" => rmpv::Value::Nil,
        _ => {
            if let Ok(i) = v.parse::<i64>() {
                rmpv::Value::from(i)
            } else if let Ok(f) = v.parse::<f32>() {
                rmpv::Value::F32(f)
            } else {
                rmpv::Value::from(v)
            }
        }
    };
    Ok((key, value))
}

fn setting_gloss(id: i64) -> String {
    use fretwire_core::fretwire_protocol::settings::{Kind, by_id};
    let Some(s) = by_id(id) else {
        return String::new();
    };
    let detail = match s.kind {
        Kind::Flag { on, off } => format!(": true {on}, false {off}"),
        Kind::Choice(&[]) => String::new(),
        Kind::Choice(vs) => {
            let list: Vec<String> = vs.iter().map(|(v, n)| format!("{v} {n}")).collect();
            format!(": {}", list.join(", "))
        }
        Kind::Number { unit, off } => {
            let unit = if unit.is_empty() {
                String::new()
            } else {
                format!(", {unit}")
            };
            match off {
                Some(v) => format!("{unit}; {v} is off"),
                None => unit,
            }
        }
    };
    format!("  ({}{detail})", s.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_udev_rule_covers_both_devices() {
        // Lower-case vendor id is required for udev to match (see the rule's own comment).
        assert!(UDEV_RULE.contains(r#"ATTR{idVendor}=="0e41""#));
        assert!(UDEV_RULE.contains(r#"ATTR{idProduct}=="4246""#)); // HX Stomp
        assert!(UDEV_RULE.contains(r#"ATTR{idProduct}=="4253""#)); // HX Stomp XL
        assert!(UDEV_RULE.contains(r#"TAG+="uaccess""#));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("/tmp/plain"), "'/tmp/plain'");
        // An embedded quote must close, escape, and reopen so the shell sees one literal arg.
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn reload_udev_runs_control_then_trigger_and_survives_failure() {
        let mut calls: Vec<String> = Vec::new();
        reload_udev(|prog, args| {
            calls.push(format!("{prog} {}", args.join(" ")));
            // Simulate udevadm being absent — reload must warn, not panic.
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        });
        assert_eq!(calls, ["udevadm control --reload", "udevadm trigger"]);
    }

    #[test]
    fn an_exported_ir_is_named_for_its_slot_and_its_own_name() {
        let slot = |index, name: &str| fretwire_data::ir::IrSlot {
            index,
            checksum: None,
            name: name.to_string(),
            md5: None,
            length_mul: 1,
            length_exp: 3,
            flags: fretwire_data::ir::IrFlags::default(),
        };
        assert_eq!(
            ir_filename(&slot(7, "G12-65 212 C")).to_str().unwrap(),
            "ir-007 G12-65 212 C.wav"
        );
        // A name is a user string: it can hold separators, and a slash would write the file
        // somewhere else entirely.
        assert_eq!(
            ir_filename(&slot(3, "a/b:c*d")).to_str().unwrap(),
            "ir-003 a_b_c_d.wav"
        );
        // The slot number still identifies a nameless one.
        assert_eq!(ir_filename(&slot(12, "  ")).to_str().unwrap(), "ir-012.wav");
    }
}
