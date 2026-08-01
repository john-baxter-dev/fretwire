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
#[command(name = "fretwire", version)]
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
    /// List what a `backup` file contains (indices + names), to pick a restore.
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

    // ---- live: edit the buffer (reversible by reloading the preset) ----
    /// Set a block's bypass, in pedal semantics: `on` engages bypass (block OFF).
    Bypass { slot: i64, state: OnOff },
    /// Set a parameter by its index in the model's `Helix.sym` order.
    Set {
        slot: i64,
        param_index: i64,
        value: f32,
    },
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
    /// Read every preset on the device into a JSON backup file.
    ///
    /// Reads only — flash is never written — but the active-preset cursor sweeps the setlist
    /// (it is put back afterwards).
    Backup { out: String },
    /// ⚠ PERSISTENT WRITE. Restore one preset from a backup file into a setlist slot.
    Restore {
        backup: String,
        index: i64,
        /// Target slot; defaults to the backup index.
        slot: Option<i64>,
    },
    /// PROBE: read the preset and write it back unchanged via op 21, then re-read.
    ///
    /// Safe — touches the edit buffer only (reversible by reloading) and changes nothing.
    WriteRoundtrip,
    /// Set a global/input setting (op 25). The id space is only partly mapped — a live RE probe.
    ///
    /// Known: id 134 = 3-state input setting (0/1/2).
    Setting { id: i64, value: i64 },
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

    match Cli::parse().command.unwrap_or(Command::Detect) {
        Command::Detect => match fretwire_usb::present_devices() {
            Ok(found) if found.is_empty() => println!("no HX device found"),
            Ok(found) => {
                for d in found {
                    let note = match d.support {
                        fretwire_usb::Support::Verified => String::new(),
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
            let presets = s.list_presets_in(bank)?;
            println!("{} presets in bank {bank}{label}:", presets.len());
            for (i, name) in &presets {
                println!("  [{i:>3}] {name}");
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
            // DSP-fit check: read the current preset and project the swap's load. A warning only —
            // the budget is unconfirmed and the device is the final arbiter (see DSP_BUDGET).
            let projected = match s.read_preset() {
                Ok(preset) => {
                    let old = preset
                        .blocks
                        .iter()
                        .find(|b| b.slot == slot)
                        .and_then(|b| b.dsp_load)
                        .unwrap_or(0.0);
                    let new = s
                        .catalog()
                        .model_load_by_index(index)
                        .map(|l| l + s.catalog().model_load_by_index(paired).unwrap_or(0.0));
                    new.map(|n| (preset.dsp_load, preset.dsp_load - old + n))
                }
                Err(e) => {
                    tracing::debug!("fit check skipped (read failed: {e})");
                    None
                }
            };
            if let Some((_cur, proj)) = projected
                && proj > fretwire_core::editor::DSP_BUDGET
            {
                eprintln!(
                    "⚠  projected DSP ~{proj:.1}% exceeds the ~{:.0}% budget — the device \
                    may reject this swap or drop a block.",
                    fretwire_core::editor::DSP_BUDGET
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
        Command::DumpRaw { out: path } => {
            let mut s = fretwire_core::Session::connect()?;
            let raw = s.read_preset_raw()?;
            std::fs::write(&path, &raw)?;
            println!("wrote {} bytes to {path}", raw.len());
        }
        Command::Backup { out: path } => {
            let mut s = fretwire_core::Session::connect()?;
            println!("backing up the setlist (the pedal will step through every preset)…");
            let backup = s.backup_setlist(|done, total, name| {
                println!("  [{done:>3}/{total}] {name}");
            })?;
            std::fs::write(&path, backup.to_json())?;
            println!("wrote {} presets to {path}", backup.presets.len());
        }
        Command::BackupShow { backup: path } => {
            let backup =
                fretwire_core::backup::Backup::from_json(&std::fs::read_to_string(&path)?)?;
            println!("{} — {} presets:", backup.device, backup.presets.len());
            for p in &backup.presets {
                println!("  [{:>3}] {}  ({} bytes)", p.index, p.name, p.raw.len());
            }
        }
        Command::Restore {
            backup: path,
            index,
            slot,
        } => {
            let slot = slot.unwrap_or(index);
            let backup =
                fretwire_core::backup::Backup::from_json(&std::fs::read_to_string(&path)?)?;
            let entry = backup.preset(index).ok_or_else(|| {
                anyhow::anyhow!(
                    "backup has no preset at index {index} (see: fretwire backup-show {path})"
                )
            })?;
            let mut s = fretwire_core::Session::connect()?;
            let current = s
                .list_presets()?
                .into_iter()
                .find(|(i, _)| *i as i64 == slot)
                .map(|(_, n)| n)
                .unwrap_or_else(|| "?".into());
            eprintln!(
                "⚠  PERSISTENT WRITE: restoring {:?} into slot {slot}, overwriting {current:?}.",
                entry.name
            );
            let preset = s.restore_preset(&entry.raw, slot, &entry.name)?;
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
fn print_preset(preset: &fretwire_core::EditorPreset) {
    match &preset.current {
        Some(i) => println!("Preset [{}] {}", i.index, i.name),
        None => println!("Preset — (current identity unknown)"),
    }
    println!(
        "device {} firmware {}",
        preset.device_model.as_deref().unwrap_or("?"),
        preset.firmware.as_deref().unwrap_or("?")
    );
    let topo = if preset.split() {
        "split (parallel)"
    } else {
        "serial"
    };
    // A two-DSP device budgets each DSP separately, so report them separately.
    let load = match preset.dsp_load_by_dsp().as_slice() {
        [(_, one)] => format!("DSP {one:.1}% used ({:.1}% free)", (100.0 - one).max(0.0)),
        many => many
            .iter()
            .map(|(d, l)| format!("DSP{} {l:.1}%", d + 1))
            .collect::<Vec<_>>()
            .join(" · "),
    };
    println!(
        "{} block(s) · {topo} topology · {load}",
        preset.blocks.len()
    );
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
        let dsp = b
            .dsp_load
            .map(|l| format!("  ({l:.1}% DSP)"))
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
        for p in &b.params {
            println!(
                "       [{:>2}] {:<14} = {}",
                p.index,
                p.name,
                fmt_value(p.value)
            );
        }
        if let Some(cab) = &b.paired_model_name {
            println!("       + cab: {cab}");
            for p in &b.paired_params {
                println!(
                    "       [{:>2}] {:<14} = {}",
                    p.index,
                    p.name,
                    fmt_value(p.value)
                );
            }
        }
    }
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
                .map(|p| format!(" param {p}"))
                .unwrap_or_default();
            let slot = a
                .target_slot
                .map(|s| format!("slot {s}"))
                .unwrap_or_else(|| "?".into());
            println!("  controller {} -> {}{}", a.controller, slot, param);
        }
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
}
