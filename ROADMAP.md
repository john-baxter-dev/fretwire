# fretwire — Roadmap

Target: **HX Stomp** (VID 0x0E41 / PID 0x4246). Goal: **full GUI editor on Linux**.
Strategy: recover the MI_00 USB control protocol by observing traffic to and from the device;
read the model/preset/control data from the user's own installed copy at runtime (nothing is
redistributed); build a libusb transport + greenfield GUI.

**Implementation language: Rust.** USB via `nusb` (pure-Rust, cross-platform, async) — no
libusb C dependency, clean on Linux; falls back fine for dev on Windows. Workspace layout:

| crate        | role |
|--------------|------|
| `fretwire-data`    | parse shipped JSON (`.models`, catalog, controls, `.hlx` presets) into typed structs |
| `fretwire-protocol`| MI_00 wire message types + encode/decode codec (filled in during Phase 2/3) |
| `fretwire-usb`     | `nusb` transport: enumerate, claim MI_00, bulk/interrupt I/O |
| `fretwire-core`    | device session API (connect, sync, param set, preset load/save, snapshots, tuner) |
| `fretwire-cli`     | command-line validator/driver |
| `fretwire-tauri`   | GUI — **Tauri 2** (WebKitGTK) + Svelte |

## Phase 0 — Recon  ✅ (done 2026-06-21)
- [x] Locate the device; set up USBPcap + Wireshark.
- [x] Inventory `res/` — found complete model/preset/UI data as JSON/XML.
- [x] Identify device VID/PID and the MI_00 vendor control interface.
- [x] Set up project dir + notes.
- [x] Scaffold the Rust workspace (5 crates) — compiles clean.
- [x] `fretwire-data`: parse every shipped `.models` + `.hlx`; presets round-trip losslessly (3 tests green).
- [x] `fretwire-usb`: `nusb` enumeration works — `fretwire detect` reports the unit present.

## Phase 1 — Capture infrastructure
- [ ] Determine which USBPcap root hub the HX Stomp is on; build a capture filter.
- [ ] Establish a repeatable capture procedure (start cap → do ONE known action → stop).
- [ ] Capture the **startup handshake** (launch HX Edit with device connected).
- [ ] Capture a set of **labeled single-action** sessions:
      - one parameter tweak (e.g. amp Drive) — note before/after value
      - block on/off (bypass)
      - model swap in one block
      - preset change (select another preset)
      - snapshot change
      - tuner on/off
      - receive a full preset (open editor on a preset)
      - send/save a preset to the device
- [ ] Store each as `captures/NN-<action>.pcapng` + a `.md` describing the exact action.

## Phase 2 — Protocol decode  (in progress)
- [x] Identify endpoints — **interrupt EP 0x01 OUT / 0x81 IN**, 16-byte base frames, addr 8.
- [x] Reusable extractor `tools/dump-control.ps1`; living spec `docs/protocol.md`.
- [x] First framing pass: 3 channels w/ swapping ids + per-channel seq; edit channel `ed03/8010`;
      inner **opcode 0x0006** + u32 length prefix; reverb-block handle `83 66 cd 03`.
- [x] Disambiguate bypass: differing bytes were a transaction counter → bypass is toggle-class.
- [x] Handle != block address; block id is the 3rd byte of `8X 62 [id] NN` (reverb 07, tremolo 04).
- [x] **Parameter values = big-endian f32** (Mix 100%→`3f800000`, 0%→`00000000`).
- [x] Op class byte after `83 66 cd`: `03` toggle, `04` set-value.
- [x] Analyze `startup.pcapng` — HX Edit opens multiple channels (ef03/ed03/f003) each with
      SESSION_OPEN resource ids; the handshake is byte-stable across runs.
- [x] Transport confirmed **bulk**, not interrupt. `arg` field (bytes 12–15) is a u32 offset,
      **not a checksum** → likely no per-packet checksum.
- [x] **Handle discovery resolved**: opening a preset streams full state as **MessagePack** on
      the edit channel (cmd 0x04→0x0c→0x08, 272-byte chunks). Block/param handles come from there
      — no need to compute them. Blob saved: `captures/preset1_stream.msgpack.bin`. See `docs/protocol.md`.
- [x] Parse the preset stream with `rmpv` — `fretwire_data::stream::PresetStream` decodes envelope →
      `l6-helix` blob → integer-keyed preset map (device info, 20 block slots, paths, snapshots).
      Tests + `docs/preset-format.md`. (codec also fixed: `len` is u16, handles 272-byte chunks.)
- [x] Typed device-preset model (`PresetStream::{device_model, firmware, blocks, path_blocks}`)
      + **device blocks resolve to `.models` defs by name** (4/4 in test preset). `docs/preset-format.md`.
- [x] Path↔slot pairing verified (path key 11→8 = slot index); param vector aligns with `.models`
      param order (confirmed by default matches). Reading params by (block, index) works end-to-end.
- [x] Model identity → `symbolicID`: there is **no numeric model id** (path 11→6 = category,
      slot 24→25 = runtime handle; neither indexes the 681-model `HelixModelDefs.bin`). Canonical
      id is `symbolicID` (unique 681/681); resolve a block by name(+category) via
      `ModelDefs::resolve`. Effect blocks unambiguous by name; amp/cab variants need category,
      which is the only undecoded bit (path 11→6 → category; needs more presets). See
      `docs/preset-format.md`, tests/`correlate_modelid.rs`.
- [x] Edit body is **MessagePack** (`fretwire_protocol::edit::EditBody`): `{102: counter, 100: op,
      101: {98: slot, 28: param_index, 119: value}}`. Block slot = key 98; bypass = bool at key 59;
      **param selected by its index (key 28) in the model's `Helix.sym` order → editing is computable
      from shipped data** (verified 4 models/6 params, `captures/param_map_findings.md`). Builders
      `edit::bypass`/`set_value` generate byte-exact commands. Switch/transport params (key 28=0) TBD.

## Phase 4 — Linux transport (proof of life)
- [x] **`fretwire-protocol` codec** built + tested: `Frame` encode/decode (exact bytes), `Tlv` body,
      BE-f32 value helpers, channel/cmd/op constants. 7 golden tests against real captured frames
      (round-trip byte-exact) + byte-exact generation of the validated 5-packet handshake.
- [ ] libusb prototype: claim MI_00, replay the handshake, read a reply.
- [ ] Implement parameter read/write; verify against the physical unit.
- [ ] Implement preset get/set; round-trip a `.hlx`.

## Phase 5 — Core library
- [x] Data layer: parse `.models` / catalog / controls / presets + device preset stream into a
      state model (`fretwire-data`).
- [~] Editor model (`fretwire_core::editor`): preset stream → typed `EditorPreset`/`EditorBlock` with
      resolved model id + category, device-ordered named params + values, and byte-exact edit
      command generation (bypass). **Live** session (connect/sync over `fretwire-usb`, param read/write,
      preset get/set, snapshots, tuner) still to come.
- [x] **Graceful disconnect / `Session::close()` — DONE (verified live 2026-06-25).** Connecting put
      the device into host-owned "edit mode" (front-panel page/preset arrows dead); dropping the
      interface — even a USB port reset — did **not** release it (it's firmware RAM, not USB state).
      The fix: send HX Edit's shutdown **session-close** (`cmd 0x02`, empty body) on each channel
      (status → edit → primary), **request/response with a ~150 ms settle before releasing the
      interface** (firing blind doesn't work). Runs on `Drop` so even Ctrl-C cleans up; `fretwire disconnect`
      exercises it. primary never acks (handshake diverges there) and the panel releases regardless,
      so the ack wait is capped (300 ms) to keep teardown fast. Decoded from `launch_hx_*_close*.pcapng`.
- [ ] CLI for validation (the GUI rides on top of this).
- [~] **DSP-fit / model availability.** HX Edit greys out models that won't fit the remaining DSP.
      **(1) per-model cost — SOLVED:** the `.models` files carry **`load`** (mono) and **`load_stereo`**
      (% of DSP budget); range 0.35–40.0 (amps ~28%, poly pitch/sustain cap at 40). Wired into
      `Catalog` (`bundled_loads`) → `EditorBlock.dsp_load` (incl. paired cab, by Mono/Stereo variant)
      → `EditorPreset.dsp_load`; `pull` shows "DSP X% used". **(3) current usage — SOLVED:** sum of
      block loads (computed locally — factory preset reads 71.8%, self-consistent). **(2) budget —
      likely 100% per DSP** (the 40-cap + busy-preset sums fit); still to confirm the exact HX Stomp
      cap and **validate our % against HX Edit's meter** on the same preset. **Availability:** the
      `devices` field (per-model device-id+firmware list; `None` = universal) gates which models a
      given unit even offers — **but on the HX Stomp this is moot:** the Stomp runs the *full* HX
      model library; its only constraint is DSP + the 8-block cap, so HX Edit's grey-out is
      **DSP-fit driven**, which we've built. The `devices` field is cross-product/firmware versioning
      (Helix Native + old-firmware gating), not a Stomp model restriction. **`swap` warns** when the
      projected load exceeds `editor::DSP_BUDGET` (~100%) via `Catalog::model_load_by_index`. **Live
      probe (2026-06-25):** the handshake identity reply carries the model string `"P33Main"` + ~12
      trailing bytes (serial/HW-rev/firmware/flags, undecoded) — **no `0x0021xxxx` device id**, so the
      `devices` ids can't be matched from the wire either. **Only loose end: confirm the budget value
      vs HX Edit's own DSP meter** (our two-amp preset reads 73.5% — does HX Edit agree?). Treat this
      thread as essentially resolved otherwise.

## Phase 6 — GUI  (the editor loop is done; remaining work is mostly UI polish)
- [x] Toolkit chosen: **iced 0.13** (tiny-skia software renderer).
- [x] Connect/disconnect, preset list + switching, current-preset identity (op 23).
- [x] Signal-chain view (boxes + wires, serial/parallel rows), click-to-select.
- [x] Live **bypass** + **param sliders** (release-to-commit), DSP meter.
- [x] **Model picker** with category selector + **DSP-fit grey-out** + live swap (same- and
      cross-category); split-type rides `swap_model`.
- [x] **Snapshots** (switch), **save-to-device** (two-click confirm), **⟳ Refresh**.
- [x] **Live-follow** of panel changes (footswitch bypass / snapshot / preset) via the
      status-channel state-push (`Session::poll_events`).
- [x] **Move** (op 43) + **add** (op 39) block — protocol + CLI verified live.
- [x] **Drag-and-drop to reorder** (serial chain) — DONE (2026-06-28): drop a block into a *gap*
      (insert, not replace). op 43 only relocates into an empty slot, so a reorder bubbles the block
      through a spare empty slot via single moves, each preceded by **op 78 begin-structural**
      (`edit::begin_structural`, byte-exact). Pure `plan_reorder` planner (unit-tested) +
      `Session::reorder_block`; GUI drop-into-gap with gap highlight. Serial presets only (errors on
      split). Verified live. **Next: parallel-path drag (Step 2, needs op-21 / split-node handling).**
- [x] **Add-block** — DONE (2026-06-29): "+ Add block" picker appends a model via the surgical op-39
      (`Session::add_block_append`, FS-safe); drag it into place. Verified live.
- [x] **Move to/from parallel (B) row** — DONE (2026-06-29): header button moves a block between
      series/parallel via `move_block` to a row-B slot (`Session::move_block_to_row`), creating/
      retiring the split; row-B derivation fixed (split-node divider). Verified live.
- [ ] **Cross-row drag** — drop a block directly onto row B (and reorder within B), reusing the
      gap-drag infra now that row B is modeled. Polish on the move-to-row buttons above.
- [x] **Delete block** — DECODED (2026-06-30): op 28 `{98:slot}` is a **surgical** delete (HX Edit
      optionally prefixes op 78; we mirror it) that **preserves the footswitch layout** — the old
      op-21 approach that wiped FS is no longer used. `edit::delete_block`, `Session::delete_block`,
      CLI `delete-block`, GUI **✕ Delete** on the selected block. Byte-exact-tested. *Pending live test.*
- [x] **Split/combine nodes & routing** — DECODED (2026-06-30): the split node's **type** is a
      `swap_model` (op 40) on the split slot to Helix.sym 256 (A/B) / 258 (Crossover) / 563 (Dynamic)
      — `cycle_through_split_types.pcapng`; the split's own params and the **mixer/join** node's A/B
      level/pan/polarity params (model 151 `HD2_AppDSPFlowJoin`) are ordinary set-values on the node's
      slot (10 / 19) — `adjust_A_B_level_and_pan_of_join.pcapng`. Surgical/FS-safe. `EditorPreset.
      {split_node,mixer_node}` + `PresetStream::structural_node`, `editor::SPLIT_TYPES`,
      `Session::set_split_type`, CLI `split-type`, GUI: split/mixer render as **selectable chips in the
      chain** (bracketing path B), selecting one edits its type/params in the normal param panel.
      Lane decode `[solid]`: bottom slots (11–18) = path B; the node's holder key `13` = its
      signal-flow column, so a top block at slot `s` is common-before (`s<split_pos`), path A
      (`split_pos≤s<mixer_pos`), or common-after (`s≥mixer_pos`). Fixtures `split_preset_stream`,
      `dual_amp_stream`. Verified live.
- [~] **Split-preset drag-routing** — position-aware cross-row/same-row moves with slot bubbling
      (`plan_row_insert` right-anchor for B, `plan_insert_right_end` left-anchor for common-before),
      an **op-43 overwrite guard** (`apply_row_moves` refuses moving onto an occupied slot — this had
      deleted a block), CLI `move-to-row`/`before-split`. Works for most regions but the **linear
      chain-with-gaps model is the wrong abstraction** for 2D routing: each region needs bespoke
      placement and some positions (e.g. end of path A, before the mixer) have no gap/slot. **NEXT:
      rewrite the chain view as the device's real 2-row × 8-col grid** (top slot = column, bottom slot
      = column + 9; split/mixer are *derived* markers). Every cell → one exact slot, one placement
      path, complete drop coverage — matches HX Edit and the hardware. This replaces the 5
      special-case move functions. **Superseded 2026-07-05** by the Tauri migration (below): the
      grid will be rebuilt in the webview, where SVG/CSS handle the wires+branches natively.
- [~] **GUI renderer → migrating to Tauri (WebKitGTK webview)** — DECIDED 2026-07-05 after a spike.
      **Why:** iced is stuck on the tiny-skia software renderer (wgpu is ruled out by EGL/dmabuf
      driver issues on this box), and tiny-skia **can't stroke paths** — so the routing UI can't draw
      wires/branches, the whole reason the chain uses plain widgets. The routing grid is inherently a
      drawing problem iced can't solve here. **Spike (`crates/fretwire-tauri`):** a minimal Tauri 2 app
      (static HTML frontend, no bundler) reusing `fretwire-core` unchanged via two `#[command]`s
      (`detect`, `pull`). Result — the `tauri`/`webkit2gtk`/`tao` tree links against system WebKitGTK
      4.1; the default dmabuf renderer hits the same fatal Wayland path as wgpu, **but
      `WEBKIT_DISABLE_DMABUF_RENDERER=1` (now baked into `main()`) runs stably** — the fallback wgpu
      never had. The webview renders **SVG stroked wires + the split/rejoin branch** cleanly (verified
      live 2026-07-05), laid out on the real 2-row × N-column grid (split/mixer get their own columns).
      The Rust core (protocol/transport/decode/`Session`/`Catalog`) is untouched and re-exposed as
      Tauri commands. **NEXT:** incremental port of the ~1900-line iced GUI to the webview — routing
      grid first (the motivation), then the param/model/save/rename panels. `fretwire-tauri` is a workspace
      member but excluded from `default-members` so `cargo test`/`build` stay off the WebKitGTK tree.
      **Progress:**
      - [x] Svelte + Vite frontend scaffold; SVG chain render on the real 2-row × N-col grid.
      - [x] Command layer (`commands.rs`) over the full `Session` surface + serde DTOs (`dto.rs`);
            session held in managed state (async + `spawn_blocking`; clean teardown on window close).
      - [x] Block/node **selection + live param editing** (bypass, sliders, enum dropdowns, on/off
            switches, paired cab params). Verified live.
      - [x] Model picker (swap) + add/delete block.
      - [x] Preset browser (list/switch) + save/Save-As/rename; snapshot switcher; split-type dropdown.
      - [x] Keepalive heartbeat + **live-follow** (footswitch bypass / panel snapshot+preset changes
            pushed to the UI via a `device-pushes` event; needs the `core:event` capability). Bypassed
            blocks render greyed in the chain.
      - [x] **Interactive routing grid** — the chain is a 2-row × N-col grid of draggable HTML cells
            (SVG wires behind); every slot is a drop target. `PresetStream::grid()` + `place_block`
            (one guarded op-43 to an exact slot; device recomputes split/mixer). Fixed-slot model, so
            drops target visible empty slots. **Tauri GUI is now at feature parity with iced.**
      - [x] **Serial→split creation via the grid** — DONE (2026-07-06, verified live): the split/
            mixer node slots exist even on serial presets [solid, preset1 fixture], so the empty B
            row is revealed while a drag is in flight (ghost bracket hint); one `place_block` into
            it and the device activates the split; last B block dragged back retires it.
      - [x] **Movable split/join nodes** — DONE (2026-07-06, verified live): drag ⋔/⋉ to a valid
            gap (drop zones show only the legal range). No surgical op exists — the node holder's
            key 13 is written via the **op-21 whole-preset write** (`PresetStream::set_node_pos` +
            `Session::set_node_pos`, guarded: bracket must enclose the occupied B row, split <
            mixer). First live op-21 write of a *mutated* blob — device honors it verbatim.
      - [x] **Insert-on-occupied-drop** — DONE (2026-07-06, verified live): dropping a block onto
            another inserts it before/after (by which half of the cell you drop on — glowing
            insertion bar), shifting neighbors. Same-row = `plan_reorder` bubble through a scratch
            slot; cross-row = `plan_row_insert` suffix shift. `Session::insert_block` +
            `insert_block` command. (Swap semantics were built first, then replaced per user
            feedback — insert matches HX Edit.) Off-by-one in the pos→final-index mapping caught by
            the mock smoke test and pinned with `insert_pos_tests`.
      - [x] **Trim trailing empty columns** — DONE (2026-07-06): at rest the grid ends one spare
            column past the last block (kept through the mixer column when split); every column
            reveals while a drag is in flight.
- [ ] **Multiple split points** — still future.
- [x] **Cab/IR param editing** — DECODED (2026-06-28): paired-cab params use sub-model selector
      `26:1` (main = `26:0`); index is positional in the cab namespace. `edit::{set_paired_value,
      set_value_on}`, `Session::set_paired_param`, CLI `set-cab`, GUI cab grid live-editable (float
      knobs). See `captures/_TODO-cab-params.md`. *Pending live test.*
- [x] **Enum param dropdowns** (incl. cab mic-select) — DONE (2026-06-28): `valueType:0` params get a
      `pick_list` of their labels (from `HelixControls.json[displayType].format`, `isDiscrete`); the
      selected index is sent as an int via `Session::set_param_enum` / `edit::set_value_on`. Generic —
      works for any discrete enum, not just mics. *Pending live test.*
- [x] **Absolute chain positions** — DONE by the Tauri routing grid (every cell = one exact slot,
      empty slots visible and droppable/clickable-to-add). Superseded the 2026-06-29 note.
- [x] **Undo/redo** — DONE (2026-07-06): exactly the op-21 blob-snapshot design — the command layer
      brackets every edit with `edit_begin(label)`/`edit_commit()`, snapshots come from the read
      cache (no extra USB), undo/redo write the prior blob back (edit buffer only). Header buttons +
      Ctrl+Z/Ctrl+Shift+Z/Ctrl+Y.
- [x] **Scrollable edit history with A/B compare** — DONE (2026-07-06): the undo stacks grew into a
      labeled timeline with a cursor (`Session::history_jump` = op-21 write of any entry; labels
      name real blocks/params, e.g. "Set Drive — Amp A"). HistoryPane: collapsible list, click to
      jump, mark two entries A/B, toggle between them by ear. Seeded with the "Loaded" state.
- [x] **Input/output node editing** — DONE (2026-07-06): slots 0/9 (gate/threshold/decay,
      level/pan) decoded [solid — io fixtures + input-gate capture: plain op-30 on the node slot];
      io.models meta bundled; IN/OUT glyphs in the grid open the param panel. **Global** settings
      (Input Z/impedance, pad, output level switches) still need a capture round — see
      `captures/_TODO-global-settings.md`.
- [x] **2026-07-08 editor round** (mock verified; cab paths live-verified 2026-07-09): segmented
      floats (cab mic Angle → 0°/45° buttons, `ParamMeta::stops` from `HelixControls.json` scale),
      **Change cab** on amp+cab combos ([solid]: same-model op-40 swap keeps amp params, new cab
      gets factory defaults), **Amp+Cab picker category** (synthetic id 100, `amp.models`
      `ircablink` defaults), **dirty/edited indicator** (`Session::saved_cursor`), **snapshot rename**
      (op 89, double-click the tab, undoable), **Spacebar bypass**, live-follow bypass overlay fix.
      Root-caused live: **wire key 23 = paired-model-active flag** — was hardcoded false, so paired
      swaps/adds stored the cab index without instantiating the cab; builders now mirror
      `paired_index >= 0` [solid]. Plus **smooth param ramping**: `preview_param`/`preview_paired_param`
      stream mid-drag values (no history/re-read); commit on release unchanged. `delete_cab.pcapng`
      decoded → op 28 `{98:slot}` = remove cab (not yet exposed in the GUI).
- [ ] Knob widget option, keyboard nav (beyond Space/Ctrl+Z), polish.

## Phase 7 — Preset & device management
- [x] **Setlist export** — BUILT (2026-07-07; multi-setlist, cancellable and renamed 2026-08-20;
      live-verified on an HX Stomp). `Session::export_setlists` walks each requested setlist (goto +
      raw read per slot, op-23 identity cross-check via `read_preset_confirmed`, cursor restored)
      into a `fretwire-backup` JSON file (`fretwire_core::backup` v2, hex-encoded raw streams).
      Reads only. CLI `export-setlist [--bank N|--all]` (alias `backup`) / `backup-show`; GUI
      **Export presets…** under the sidebar's ⋯ menu, with a scope choice, a whole-job progress bar
      and a Cancel — a Floor's eight setlists is 1024 presets and the better part of an hour, and a
      cancelled sweep still writes what it read. (Stores our own format, not `.hlx` — the
      wire↔`.hlx` key mapping isn't needed for round-tripping.)
      **Deliberately not called a backup**: see "Full device backup" below.
- [~] **Restore / preset write** — BUILT (2026-07-07; bank-aware 2026-08-20; **[hypothesis] until
      the first live test**): `Session::restore_preset` = goto bank/slot → op-21 edit-buffer write of
      the stored blob → op-71 save. The op-21 chunked write is live-proven for mutated *current*
      blobs (node moves); a foreign blob is the same mechanism, unverified. CLI
      `restore <file> <index> [slot] [--bank N]`; GUI Restore… with source + target pickers
      (overwrite always visible). Also gives duplicate/copy.
- [ ] **Full device backup** — the thing "Backup" used to imply and does not deliver. A restore that
      makes a wiped pedal whole needs three parts, and we have one:
      **presets** (done — setlist export above), **global / I/O settings** (op 25's id space is
      barely mapped — see below), and **IRs** (op 9/12 transaction only partly decoded — see below).
      Gated on those two, in that order; the naming stays honest until all three land, because a file
      called a backup gets trusted as one. `fretwire_data::hxb` already reads HX Edit's own `.hxb`,
      which is the reference for what a real one contains — and a plausible import path once the
      `tone` JSON → wire blob conversion exists.
- [ ] **IR management** — upload user impulse responses to the device's IR slots, rename/reorder/
      delete. **Transaction shape PARTIALLY DECODED** (2026-06-28, `captures/_TODO-ir.md`): PRIMARY
      channel, session op 255/254; **upload = op 9** `{112:slot, 113:u32 checksum, 109:name, 110:8192B
      blob (2048×f32), …flags}` + op 13 commit; **export = op 12/11**. Before implementing: reassemble
      the blob, **confirm the `113` checksum algorithm**, and decode the format flags (more captures).
- [ ] **Global / I/O settings** — extend op 25 (`{118:id, 119:value}`); map the id space (only id
      134 known; the preset-load pushes expose more `118/119` ids to catalog).
- [x] **Save As** (GUI, 2026-06-29) — write the edit buffer to a chosen slot under a new name (op 71);
      sidebar slot-pick + overwrite confirm. Verified live.
- [x] **Preset rename (name-only)** — DECODED (2026-06-30): op 6 `{107:bank,108:slot,109:name\0}` on
      the **primary** channel. Unlike save (op 71) it does **not** commit the edit buffer — the capture
      (`change_amp_drive_rename_..._name_sticks_change_doesnt`) proved a pending param edit didn't
      persist. `edit::rename_preset`, `Session::rename_preset`, CLI `rename`, GUI **Rename…** field
      (no confirm, HX Edit semantics). Byte-exact-tested. *Pending live test.*
- [ ] Copy/paste/duplicate **blocks** (read a block's content, `add_block` + `set_value`s).
- [x] **`.hxb` backup reading** (2026-07-26) — `fretwire_data::hxb` parses HX Edit's own backup
      container (AF6L header + concatenated raw zlib streams): globals, 128 IR slots, the model-usage
      table and the 8 setlists. CLI `show-backup <file.hxb> [--presets]`. Reading only — the presets
      inside are `tone` JSON, not wire blobs, so restoring *from* a `.hxb` still needs a JSON→blob
      conversion. Its setlist order is what promoted `Device::setlists` to [solid].

## Phase 7.5 — Tooling / developer experience
- [x] **Move the CLI to `clap`.** (2026-07-29) `fretwire-cli` hand-rolled a `match` over
      `std::env::args` for ~35 subcommands, with hand-maintained `eprintln!` help. Both motivating
      problems were observed rather than theoretical, and both are now structurally impossible:
      1. **The help drifted.** Editing one `eprintln!` during the 2026-07-26 session silently dropped
         `set` and `snapshot` from the listing. The migration found three *more* commands missing from
         it — `tree`, `move-to-row`, `before-split` — which nobody had noticed. `--help` is now
         generated from the command definitions, so it cannot disagree with them.
      2. **Silent bad-argument fallbacks.** `args.next().map(|s| s.parse().unwrap_or(0)).unwrap_or(0)`
         meant `fretwire goto 5 banana` quietly targeted bank 0 — and on `save`/`rename`, that was a
         persistent write to the wrong setlist. Every numeric argument now errors instead.
      `bypass`/`move-to-row` became `ValueEnum`s: the old parser read *any* unrecognised word, and a
      *missing* argument, as `off`/series — so a typo silently did the opposite of the request.
      Every documented invocation was smoke-tested unchanged (parsing happens before `connect`, so
      argument acceptance is checkable without hardware).

## Phase 8 — Publishing 
- [x] `fretwire import-data <installer>` — extract Line 6's reference data from the user's own HX Edit
      install (verified byte-identical vs the bundled copies).
- [x] The data flip: `Catalog::bundled()` (`include_bytes!`) → `Catalog::from_data_dir()` + no-data
      fallback; `git rm` the bundled data + `res-extracted/`; update tests. (2026-07-18)
- [x] First-run import in the GUI — `fretwire_core::import` + `FirstRun.svelte`, so a fresh install
      doesn't dead-end at "run `fretwire import-data`". (2026-07-21)
- [x] Packaging: `.deb`/`.rpm`/AppImage via `tauri build` (deb/rpm install the udev rule and ship the
      CLI), static musl CLI, GitHub Release on a `v*` tag. README + license/trademark notes.
      (2026-07-21)
- [ ] **TODO — publish to the AUR.** `packaging/PKGBUILD` is written but **not published or even
      built once**. Needs, in order:
      1. Local validation: copy it out of the tree (`makepkg` litters `src/`/`pkg/`), `updpkgsums`
         (needs `pacman-contrib`), then `makepkg -si`. This is the one package an Arch box can test
         end to end — the tag tarball, `npm ci`, both binaries, `check()`, the udev rule.
      2. Replace `sha256sums=('SKIP')` with the real hash (`updpkgsums` rewrites it). Publishing
         with `SKIP` means a tampered tarball can't be detected.
      3. Publish: AUR account + SSH key → `git clone ssh://aur@aur.archlinux.org/fretwire.git` →
         copy the PKGBUILD → `makepkg --printsrcinfo > .SRCINFO` (mandatory, push is rejected
         without it) → commit → push. Check the name is free first.
      Per release afterwards: bump `pkgver`, reset `pkgrel=1`, `updpkgsums`, regenerate `.SRCINFO`.
- [ ] Flathub — deferred. Needs a broad `--device=all` for USB, can't install a udev rule, and the
      sandbox complicates pointing at an HX Edit installer on the host. Revisit once there are users.
- [ ] **arm64 for the CLI** (`aarch64-unknown-linux-musl`) — a matrix entry on the existing musl job
      in `release.yml`, ~10 lines. `fretwire-cli` has no C dependencies (`nusb` is pure Rust), so it
      cross-compiles to a static binary cleanly. **Why:** a Raspberry Pi wired into a pedalboard doing
      preset switching / backup / restore with no screen is the real use case; Asahi Linux on Apple
      Silicon is a smaller second one. **Caveat:** untested — no arm64 hardware here, so label the
      asset as such in the release notes until someone confirms it runs.
- [ ] arm64 for the **GUI** — not planned. Feasible now that public repos get free `ubuntu-24.04-arm`
      runners (native build, no cross-compiling WebKitGTK), but it doubles the bundle matrix for a
      thin slice of users. Wait for someone to ask.
- [ ] Other architectures — **deliberately not doing**: i686 (dead on the desktop), armv7 / 32-bit Pi
      (won't run the editor usefully), RISC-V (no users). Untestable binaries are a support burden.

## Phase 9 — Other HX devices (Helix Floor first)
Opened 2026-07-22 by a contributor's Helix Floor captures + backup. Survey: **`docs/helix-floor.md`**.
The data layer already covers the Floor (355/355 models, 19,377/19,377 param keys resolve) and its
USB control interface is identical to the Stomp's, so this is mostly plumbing — *once* we can see a
real session.

- [x] Identify the device: PID `0x4248`, preset `device` ID `0x210001`, fw `0x03800000`. Constant +
      udev rule landed; no code matches on the PID yet.
- [x] Decode the `.hxb` backup container (header + concatenated raw zlib streams).
- [x] Get a capture with HX Edit connected (captures 3 & 4, 2026-07-22).
- [x] **Verify the handshake — it is byte-identical to the Stomp's.** All ten `device_handshake()`
      frames appear verbatim in both Floor captures. No change needed. Floor model code is `P21`.
- [x] Confirm our preset parser reads Floor streams — it does, unmodified, including all 8
      snapshots. Cross-checked against the `.hxb` backup as ground truth.
- [x] **Handle slot `type 7` (Looper)** in `fretwire_data::stream` enumeration. Different content
      shape: model index at key `8`, params at `7 → 4`, enabled at `10`. **Fixed the Stomp too** —
      a Floor capture's serial preset went from 8 blocks to 9 once its Looper stopped being skipped.
- [x] **Walk preset key `1` (the second DSP's slot array)** alongside key `0`. Blocks now carry
      `(dsp, index)` and flatten to the wire slot with `dsp * 20 + index`. `fretwire_protocol::edit`
      needed no change. `EditorPreset` gained a `DspView` per DSP (its own split/mixer/input/output
      nodes, grid and load); the flat accessors now mean DSP 0, which is what a one-DSP device has.
      Verified against a real Floor capture: "Pull Me Under" decodes all **15** blocks across both
      DSPs with the right rows and footswitch bindings (it used to show 7).
- [x] **Verify the write path — byte-exact, 9/9 ops.** Captures 3 & 4 are HX Edit-driven and carry
      the full write path; our existing `edit` builders reproduce the Floor's bytes exactly for
      `set_value`, `bypass`, `begin_structural`, `swap_model` (incl. a paired amp+cab swap),
      `save_preset` and select-preset. Envelope shapes are identical to the Stomp's. **No protocol
      change is needed for the Floor in either direction.**
- [x] **DSP2 addressing — solved (`WinCap5`, 2026-07-23). Wire slot numbers are global:
      `slot = dsp * 20 + index`**, so DSP1 is 0–19 and DSP2 is 20–39. There is no DSP field and none
      is needed. Confirmed by five DSP2 blocks edited in HX Edit on `FACTORY 1` `12B` "Pull Me
      Under", each sweep's first wire value landing one UI increment from that block's stored value —
      and consistent with every earlier capture (all slots < 20, all DSP1). The same capture also
      gives the read side of a parallel, dual-DSP preset. **No further Floor captures are needed.**
- [x] **Replaced the scattered PID constants with a device-descriptor type.**
      `fretwire_protocol::Device` + `DEVICES` carry PID, model code, preset `device` ID, DSP count,
      snapshot count and a `Support` flag; `Device::by_pid`/`by_model_code` do the lookups.
      `Transport::open` now matches **any** known device (verified ones first) and exposes which it
      opened via `Transport::device()` / `Session::device()`; `present_devices()` lists everything
      plugged in. The HX Stomp XL is listed as `Untested` with its unknown fields honestly `None` —
      we have no capture, preset or backup from one — and opening it logs a warning. Tests pin the
      invariants, including that every table entry has a matching udev rule.
- [ ] **Session grid/routing planning is still DSP-0 only.** `add_block_at`, `place_block`,
      `insert_block`, `reorder_block` and `set_node_pos` plan slot moves inside one 20-slot array
      and read it via `dsp_blocks(0)`/`dsp_grid(0)` — complete for the Stomp, needs a `dsp` argument
      for the Floor. Reading and per-block edits are already DSP-agnostic; only this layer is not.
- [ ] Grid/UI: the routing view assumes one DSP × 2 rows. The Floor needs 2 DSPs × 2 paths. The
      backend is ready — `PresetDto.dsps[]` carries each DSP's grid/nodes/load, and every cell and
      block is tagged with its `dsp`; the flat fields mirror `dsps[0]` so the current UI is
      unaffected until it's rewritten.
- [ ] `.hxb` import/restore — **independently useful, needs no device and no new captures**. Would
      give the Stomp backup-file interop too. Format is documented well enough to build against.

## Safety
See **`docs/safety.md`**. TL;DR: captures + offline work are zero-risk; live control is low-risk
(worst case = power cycle); **firmware/flash/bootloader/DFU is the only brick risk and is out of
scope — never transmit it**. Back up the device before any write experiments.

## Risks / unknowns
- Auth: HX Edit has `auth_*.xml` + an authentication log → confirm the device link itself
  doesn't require online auth (editing should be local; account is for licenses/marketplace).
- The wire preset format may be binary (not the JSON `.hlx`); captures will tell.
- Firmware update path is explicitly **out of scope** (risk of bricking) until late, if ever.
