//! `fretwire` — command-line driver for the fretwire stack.
//!
//! Early scaffold: enough to prove the workspace builds and that USB enumeration works.
//! Subcommands grow as the protocol comes online.

use anyhow::Result;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "detect".into());
    match cmd.as_str() {
        "detect" => match fretwire_usb::present_devices() {
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
        "show-preset" => {
            let path = args.next().ok_or_else(|| {
                anyhow::anyhow!("usage: fretwire show-preset <reassembled-stream.bin>")
            })?;
            show_preset(&path)?;
        }
        "decode-edit" => {
            let hex = args.next().ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: fretwire decode-edit <hex bytes of the edit body, e.g. 8366cd03f2...>"
                )
            })?;
            decode_edit(&hex)?;
        }
        // ---- live device commands (need Linux + the pedal) ----
        "connect" => {
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
        "disconnect" => {
            // Connect then immediately tear down — isolates the teardown so you can confirm the
            // pedal returns to standalone (no "panel lock") after our software lets go.
            let mut s = fretwire_core::Session::connect()?;
            s.close()?;
            println!("disconnected — session-close sent on all channels; pedal back to standalone");
        }
        "pull" => {
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
        "bypass" => {
            // Pedal semantics: `bypass <slot> on` engages bypass (block OFF); `off` activates it.
            let slot: i64 = next_num(&mut args, "slot")?;
            let bypassed = matches!(
                args.next().as_deref(),
                Some("on") | Some("true") | Some("1")
            );
            let mut s = fretwire_core::Session::connect()?;
            s.set_enabled(slot, !bypassed)?;
            println!(
                "slot {slot} bypass -> {}  (block {})",
                if bypassed { "on" } else { "off" },
                if bypassed { "off" } else { "on" }
            );
        }
        "set" => {
            let slot: i64 = next_num(&mut args, "slot")?;
            let param_index: i64 = next_num(&mut args, "param-index")?;
            let value: f32 = next_num(&mut args, "value")?;
            let mut s = fretwire_core::Session::connect()?;
            s.set_param(slot, param_index, value)?;
            println!("slot {slot} param[{param_index}] -> {value}");
        }
        "set-cab" => {
            // Edit a parameter on the block's paired cab/IR (amp+cab blocks). Param indices are in
            // the cab's own namespace: e.g. mic=0, mic position=1, mic distance=2, mic angle=3.
            let slot: i64 = next_num(&mut args, "slot")?;
            let param_index: i64 = next_num(&mut args, "param-index")?;
            let value: f32 = next_num(&mut args, "value")?;
            let mut s = fretwire_core::Session::connect()?;
            s.set_paired_param(slot, param_index, value)?;
            println!("slot {slot} cab param[{param_index}] -> {value}");
        }
        "presets" => {
            // List one setlist's presets with their indices (non-destructive). `presets [bank]`;
            // bank defaults to the setlist the device is currently sitting in.
            let mut s = fretwire_core::Session::connect()?;
            let bank = match args.next() {
                Some(a) => a.parse().unwrap_or(0),
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
        "show-backup" => {
            // Inspect an HX Edit `.hxb` device backup offline (no pedal). Reads only.
            let path = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: fretwire show-backup <backup.hxb>"))?;
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
            let verbose = args.next().as_deref() == Some("--presets");
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
        "dump-list" => {
            // Save the raw reassembled preset-list stream for a setlist (reads only). Diagnostic:
            // the browse's numbering hasn't fully reconciled with the device's own.
            let bank: i64 = next_num(&mut args, "bank")?;
            let path = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: fretwire dump-list <bank> <out.bin>"))?;
            let mut s = fretwire_core::Session::connect()?;
            let raw = s.list_presets_raw(bank)?;
            std::fs::write(&path, &raw)?;
            println!("wrote {} bytes of bank {bank}'s list to {path}", raw.len());
            for (i, name) in s.list_presets_in(bank)?.iter().take(8) {
                println!("  [{i:>3}] {name}");
            }
        }
        "setlists" => {
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
        "goto" => {
            // Navigate to a preset: `goto <preset> [bank]` (bank defaults to 0). Changes state.
            let preset: i64 = next_num(&mut args, "preset")?;
            let bank: i64 = args.next().map(|s| s.parse().unwrap_or(0)).unwrap_or(0);
            let mut s = fretwire_core::Session::connect()?;
            s.goto_preset(bank, preset)?;
            println!("selected bank {bank} preset {preset}");
        }
        "snapshot" => {
            // Switch the active snapshot: `snapshot <index>` (0-based, as `pull` lists them).
            let index: i64 = next_num(&mut args, "index")?;
            let mut s = fretwire_core::Session::connect()?;
            s.set_snapshot(index)?;
            println!("switched to snapshot {index}");
        }
        "move" => {
            // Move a block: `move <src-slot> <dst-slot>`. The dst slot encodes the row (a parallel
            // slot index moves it to row B). Re-reads after, since positions shift.
            let src: i64 = next_num(&mut args, "src-slot")?;
            let dst: i64 = next_num(&mut args, "dst-slot")?;
            let mut s = fretwire_core::Session::connect()?;
            s.move_block(src, dst)?;
            let preset = s.read_preset()?;
            println!("moved slot {src} -> {dst}");
            print_preset(&preset);
        }
        "add-block" => {
            // Add a block: `add-block <slot> <model-index> [paired-index]`. model-index = Helix.sym
            // index. paired defaults to -1 (no cab). Re-reads to show the new block's default params.
            let slot: i64 = next_num(&mut args, "slot")?;
            let index: i64 = next_num(&mut args, "model-index")?;
            let paired: i64 = args.next().map(|s| s.parse().unwrap_or(-1)).unwrap_or(-1);
            let mut s = fretwire_core::Session::connect()?;
            s.add_block(slot, index, paired)?;
            let preset = s.read_preset()?;
            println!("added model index {index} at slot {slot}");
            print_preset(&preset);
        }
        "write-roundtrip" => {
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
        "delete-block" => {
            // Delete a block: `delete-block <slot>` (op 28 surgical delete; op 78 begin-structural
            // first, as HX Edit does). Preserves the footswitch layout of the other blocks. Edit
            // buffer only; reload the preset to undo.
            let slot: i64 = next_num(&mut args, "slot")?;
            eprintln!(
                "deleting block at slot {slot} via op-28 (surgical; keeps footswitch layout)."
            );
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.delete_block(slot)?;
            println!("deleted slot {slot}:");
            print_preset(&preset);
        }
        "move-to-row" => {
            // Position-aware cross-row move: `move-to-row <src-slot> <p|s> <pos>` (p=parallel/B,
            // s=series/A; pos = insertion index among the target row's blocks, or 'end').
            let src: i64 = next_num(&mut args, "src-slot")?;
            let par = matches!(
                args.next().as_deref(),
                Some("p") | Some("parallel") | Some("1")
            );
            let pos = match args.next().as_deref() {
                Some("end") | None => usize::MAX,
                Some(n) => n.parse().unwrap_or(usize::MAX),
            };
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.move_block_to_row(src, par, pos)?;
            println!(
                "moved slot {src} to {} row at pos {pos}:",
                if par { "parallel" } else { "series" }
            );
            print_preset(&preset);
        }
        "before-split" => {
            // Move a block into the common (pre-split) section, just before the split: `before-split <src>`.
            let src: i64 = next_num(&mut args, "src-slot")?;
            let mut s = fretwire_core::Session::connect()?;
            let preset = s.move_before_split(src)?;
            println!("moved slot {src} before the split:");
            print_preset(&preset);
        }
        "split-type" => {
            // Change the parallel split node's type: `split-type <ab|xover|dyn>` (op 40 swap-model on
            // the split slot). Only meaningful on a split preset.
            let which = args.next().unwrap_or_default();
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
        "rename" => {
            // Name-only rename (op 6, primary channel): `rename <slot> <name> [bank]`. Changes only
            // the stored name — does NOT commit the edit buffer (any pending edits stay unsaved).
            let slot: i64 = next_num(&mut args, "slot")?;
            let name = args.next().ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: fretwire rename <slot> <name> [bank]  (name-only, doesn't save edits)"
                )
            })?;
            let bank: i64 = args.next().map(|s| s.parse().unwrap_or(0)).unwrap_or(0);
            let mut s = fretwire_core::Session::connect()?;
            s.rename_preset(bank, slot, &name)?;
            println!(
                "renamed bank {bank} slot {slot} to {name:?} (name-only; edit buffer not saved)"
            );
        }
        "swap" => {
            // Swap a block's model: `swap <slot> <model-index> [paired-index]`. model-index is the
            // Helix.sym index (as `pull` resolves identity from). paired defaults to -1 (no cab).
            let slot: i64 = next_num(&mut args, "slot")?;
            let index: i64 = next_num(&mut args, "model-index")?;
            let paired: i64 = args.next().map(|s| s.parse().unwrap_or(-1)).unwrap_or(-1);
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
        "rename-snapshot" => {
            // Rename a snapshot: `rename-snapshot <index> <name>` (0-based index).
            let index: i64 = next_num(&mut args, "index")?;
            let name = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: fretwire rename-snapshot <index> <name>"))?;
            let mut s = fretwire_core::Session::connect()?;
            s.rename_snapshot(index, &name)?;
            println!("snapshot {index} renamed to {name:?}");
        }
        "setting" => {
            // Set a global/input setting (op 25): `setting <id> <value>`. The id space is only
            // partly mapped — this is a live RE probe. Known: id 134 = 3-state input setting (0/1/2).
            let id: i64 = next_num(&mut args, "id")?;
            let value: i64 = next_num(&mut args, "value")?;
            let mut s = fretwire_core::Session::connect()?;
            s.set_setting(id, value)?;
            println!("setting {id} -> {value}  (op 25; id space partly mapped)");
        }
        "save" => {
            // PERSISTENT WRITE: save the current edit buffer to a preset slot.
            //   fretwire save <slot> <name> [bank]   (bank defaults to 0)
            let slot: i64 = next_num(&mut args, "slot")?;
            let name = args.next().ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: fretwire save <slot> <name> [bank]  (⚠ overwrites the slot)"
                )
            })?;
            let bank: i64 = args.next().map(|s| s.parse().unwrap_or(0)).unwrap_or(0);
            eprintln!("⚠  PERSISTENT WRITE: overwriting bank {bank} slot {slot} with the current");
            eprintln!("   edit buffer as {name:?}. Back up first; use a scratch slot to test.");
            let mut s = fretwire_core::Session::connect()?;
            s.save_preset(bank, slot, &name)?;
            println!("saved current edit buffer to bank {bank} slot {slot} as {name:?}");
        }
        "dump-raw" => {
            // Connect and save the raw reassembled preset stream to a file (for diffing states).
            let path = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: fretwire dump-raw <out.bin>"))?;
            let mut s = fretwire_core::Session::connect()?;
            let raw = s.read_preset_raw()?;
            std::fs::write(&path, &raw)?;
            println!("wrote {} bytes to {path}", raw.len());
        }
        "backup" => {
            // Read every preset on the device into a JSON backup file. Reads only — flash is
            // never written — but the active-preset cursor sweeps the setlist (it's put back).
            let path = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: fretwire backup <out.json>"))?;
            let mut s = fretwire_core::Session::connect()?;
            println!("backing up the setlist (the pedal will step through every preset)…");
            let backup = s.backup_setlist(|done, total, name| {
                println!("  [{done:>3}/{total}] {name}");
            })?;
            std::fs::write(&path, backup.to_json())?;
            println!("wrote {} presets to {path}", backup.presets.len());
        }
        "backup-show" => {
            // Offline: list what a backup file contains (indices + names), to pick a restore.
            let path = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: fretwire backup-show <backup.json>"))?;
            let backup =
                fretwire_core::backup::Backup::from_json(&std::fs::read_to_string(&path)?)?;
            println!("{} — {} presets:", backup.device, backup.presets.len());
            for p in &backup.presets {
                println!("  [{:>3}] {}  ({} bytes)", p.index, p.name, p.raw.len());
            }
        }
        "restore" => {
            // PERSISTENT WRITE: restore one preset from a backup file into a setlist slot.
            //   fretwire restore <backup.json> <backup-index> [target-slot]   (target defaults to index)
            let path = args.next().ok_or_else(|| {
                anyhow::anyhow!("usage: fretwire restore <backup.json> <backup-index> [target-slot]  (⚠ overwrites the slot)")
            })?;
            let index: i64 = next_num(&mut args, "backup-index")?;
            let slot: i64 = args
                .next()
                .map(|s| s.parse().unwrap_or(index))
                .unwrap_or(index);
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
        "tree" => {
            // Offline: print the MessagePack structure of a saved raw stream (RE exploration).
            let path = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: fretwire tree <stream.bin> [depth]"))?;
            let depth: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(4);
            let ps = fretwire_data::stream::PresetStream::parse(&std::fs::read(&path)?)?;
            println!("{}", fretwire_data::stream::summarize(&ps.preset, depth));
        }
        "diff-stream" => {
            // Offline: find the integer-key paths that differ between two saved streams.
            let a = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: fretwire diff-stream <a.bin> <b.bin>"))?;
            let b = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: fretwire diff-stream <a.bin> <b.bin>"))?;
            diff_stream(&a, &b)?;
        }
        "probe" => {
            // Send one hand-built edit-channel frame and print the reply. For poking the live
            // sequence (e.g. trying open-resource body variants) without recompiling.
            let cmd_b = u8::from_str_radix(&strip_hex(&args.next().unwrap_or_default()), 16)
                .map_err(|e| anyhow::anyhow!("bad cmd hex: {e}"))?;
            let arg =
                u32::from_str_radix(&strip_hex(&args.next().unwrap_or_default()), 16).unwrap_or(0);
            let body = parse_hex(&args.next().unwrap_or_default())?;
            use fretwire_core::fretwire_protocol::{Frame, channel};
            let (src, dst) = channel::EDIT;
            let mut s = fretwire_core::Session::connect()?;
            let reply = s.request(&Frame::new(src, dst, 0, cmd_b, arg, body))?;
            println!(
                "reply cmd={:#04x} arg={:#010x} dst={:#06x} body={:02x?}",
                reply.cmd, reply.arg, reply.dst, reply.body
            );
        }
        "import-data" => {
            // Import Line 6's reference data from a user-supplied HX Edit installer *or* an
            // already-extracted directory into the local data dir. We redistribute nothing — the
            // data goes Line6 -> user -> tool.
            let source = args.next().ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: fretwire import-data <source>\n  \
                     <source> = an HX Edit installer (.exe/.msi/.pkg/.dmg, unpacked with 7z) or a \
                     directory of already-extracted files (e.g. an install's `res/` folder; needs no 7z)"
                )
            })?;
            import_data(&source)?;
        }
        "install-udev" => {
            // Install the udev rule that grants the logged-in user access to the HX Stomp's USB
            // node (else every live command needs root). `--print` just emits the rule instead.
            if args.next().as_deref() == Some("--print") {
                print!("{UDEV_RULE}");
            } else {
                install_udev()?;
            }
        }
        other => {
            eprintln!("unknown command: {other}");
            eprintln!("usage: fretwire <command>");
            eprintln!(
                "  offline: detect | show-preset <stream.bin> | decode-edit <hex> | diff-stream <a.bin> <b.bin>"
            );
            eprintln!(
                "           show-backup <backup.hxb> [--presets]   (inspect an HX Edit device backup)"
            );
            eprintln!(
                "           import-data <installer|dir>   (reference data from your own install; dir needs no 7z)"
            );
            eprintln!(
                "           install-udev [--print]   (install the udev rule for non-root USB access)"
            );
            eprintln!("  live:    connect | disconnect | pull | presets [bank] | setlists");
            eprintln!(
                "           dump-list <bank> <out.bin>   (raw preset-list stream, diagnostic)"
            );
            eprintln!("           goto <preset> [bank]");
            eprintln!(
                "           bypass <slot> <on|off>   (on = bypassed / block off, as on the pedal)"
            );
            eprintln!("           set <slot> <param-idx> <value> | snapshot <index>");
            eprintln!(
                "           set-cab <slot> <param-idx> <value>   (edit the paired cab/IR's params)"
            );
            eprintln!(
                "           save <slot> <name> [bank]   (⚠ persistent write — overwrites the slot)"
            );
            eprintln!(
                "           swap <slot> <model-index> [paired-index] | rename-snapshot <index> <name>"
            );
            eprintln!(
                "           move <src-slot> <dst-slot> | add-block <slot> <model-index> [paired-index]"
            );
            eprintln!(
                "           write-roundtrip   (op-21 probe: rewrite preset unchanged) | delete-block <slot>"
            );
            eprintln!(
                "           rename <slot> <name> [bank]   (name-only, op 6 — does NOT save edits)"
            );
            eprintln!(
                "           split-type <y|ab|xover|dyn>   (retype the parallel split node, op 40)"
            );
            eprintln!("           setting <id> <value>  (op 25 probe)");
            eprintln!("           backup <out.json>   (read every preset to a file — reads only)");
            eprintln!(
                "           restore <backup.json> <index> [slot]   (⚠ persistent write — overwrites the slot)"
            );
            eprintln!("           backup-show <backup.json>   (offline: list a backup's contents)");
            eprintln!("  debug:   probe <cmd-hex> <arg-hex> <body-hex> | dump-raw <out.bin>");
        }
    }
    Ok(())
}

fn next_num<T: std::str::FromStr>(args: &mut impl Iterator<Item = String>, what: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    let s = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing argument: {what}"))?;
    s.parse::<T>()
        .map_err(|e| anyhow::anyhow!("bad {what} {s:?}: {e}"))
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
