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
- [~] **Backup** — BUILT (2026-07-07, offline+mock verified; live pass pending): `Session::
      backup_setlist` sweeps the setlist (goto + raw read per slot, op-23 identity cross-check,
      cursor restored) into a `fretwire-backup` JSON file (`fretwire_core::backup`, hex-encoded raw streams).
      Reads only. CLI `backup`/`backup-show`; GUI Backup… with a progress overlay. (Stores our own
      format, not `.hlx` — the wire↔`.hlx` key mapping isn't needed for round-tripping.)
- [~] **Restore / preset write** — BUILT (2026-07-07; **[hypothesis] until the first live test**):
      `Session::restore_preset` = goto slot → op-21 edit-buffer write of the stored blob → op-71
      save. The op-21 chunked write is live-proven for mutated *current* blobs (node moves); a
      foreign blob is the same mechanism, unverified. CLI `restore <file> <index> [slot]`; GUI
      Restore… with source + target pickers (overwrite always visible). Also gives duplicate/copy.
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

## Phase 8 — Publishing 
- [x] `fretwire import-data <installer>` — extract Line 6's reference data from the user's own HX Edit
      install (verified byte-identical vs the bundled copies).
- [ ] The data flip: `Catalog::bundled()` (`include_bytes!`) → `Catalog::from_data_dir()` + no-data
      fallback; `git rm` the bundled data + `res-extracted/`; update tests.
- [ ] Packaging: prebuilt binaries, udev rule, AppImage/AUR; README + license/trademark notes.

## Safety
See **`docs/safety.md`**. TL;DR: captures + offline work are zero-risk; live control is low-risk
(worst case = power cycle); **firmware/flash/bootloader/DFU is the only brick risk and is out of
scope — never transmit it**. Back up the device before any write experiments.

## Risks / unknowns
- Auth: HX Edit has `auth_*.xml` + an authentication log → confirm the device link itself
  doesn't require online auth (editing should be local; account is for licenses/marketplace).
- The wire preset format may be binary (not the JSON `.hlx`); captures will tell.
- Firmware update path is explicitly **out of scope** (risk of bricking) until late, if ever.
