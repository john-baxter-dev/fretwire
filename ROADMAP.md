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

**Still wanted (2026-08-21), for footswitch/controller assign** — each is one action in HX Edit,
start capture → do the single thing → stop:
- `assign_block_to_fs1.pcapng` — a preset with an unbound block; bind that block's bypass to FS1.
- `unassign_block_from_fs1.pcapng` — the reverse of the above, same preset.
- `move_block_fs1_to_fs2.pcapng` — a bound block moved between switches (separates "bind" from
  "reorder the layout").
- `assign_param_to_exp1.pcapng` — assign one parameter (e.g. a Wah position) to EXP1.
- `assign_same_param_to_fs4.pcapng` — the *same* parameter, same preset, to a footswitch instead.
  The pair is what makes the controller-number space readable by diff; a Helix Floor is the useful
  device here.

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
- [x] **Model icons** (2026-08-21): every block and picker row draws the hardware it models, as a
      generated SVG silhouette (`ui/src/lib/icons/`). Cabs derive their speaker grid from the driver
      array in the name; amps match on symbolic-id prefix; unlisted models fall back to their effect
      family, then the category. **Follow-up:** ~30 models are placeholders where the original
      wasn't identified — see the "Known guesses" table in `docs/icons.md`, correct opportunistically
      (one line each in `models.js`). Also open: the picker's category `<select>` is text-only (a
      native option can't hold an icon).
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

- [x] **A cleared footswitch label no longer comes back** (2026-08-22): key `14` keeps the last
      string written and key `13` is the has-label flag; we read 14 alone, so a Simple Delay bound to
      FS2 displayed as `"Tremolo"` — a name a different block had held on that switch. Found by
      smoke-testing the new `assign-bypass`.
- [x] **Show footswitch bindings** — DONE (2026-08-21): every block already carried `footswitch`
      (preset key `3 → 8`, layout position + 1, `0` = unbound) all the way to the DTO, and the GUI
      dropped it on the floor. Chain cells and the param panel now show an `FS<n>` badge. Read-only.
- [x] **Assign a block's bypass to a footswitch** — DONE (2026-08-22, verified live). It needed no
      captures in the end and no op-21 rewrite: **op 56 `{98: slot, 102: switch}`** binds it and
      **op 57** unbinds, both zero-based, both surgical. Sent on a preset with nothing bound, op 56
      added exactly one entry at `3 → 8[0]` and op 57 restored the document byte-for-byte. The
      opcodes came from `tonepush`'s macOS capture; the verification is ours.
      `edit::{assign_bypass_to_switch, unassign_bypass_from_switch}`, `Session` methods of the same
      name, CLI `assign-bypass` / `unassign-bypass`.
- [x] **Parameter controllers — reading** (EXP pedal / a switch driving a param — preset key `4`).
      **Unblocked 2026-08-21.** The diff experiment was run on a Stomp: assign a param to FS1, then a
      second to FS2, and diff the document each time. Key `4` is indexed by **source ordinal**
      (**FS1 = 3, FS2 = 4**; `tonepush` puts EXP1 at 1), the parameter index is `6 → 29` not `6 → 28`,
      and the travel is keys `2`/`3` not `4`/`7` — all three were being read wrong, so every
      assignment reported "param 0, 0 → 0". Fixed in `PresetStream::assignments`, pinned by
      `fretwire-data/tests/assignments.rs`, written up in `docs/preset-format.md`. `pull` now prints
      `FS1 -> slot 16 param 0  [0 -> 8]`.
- [x] **Parameter controllers — writing** — DONE (2026-08-22, verified live). **Op 37**
      `{98: slot, 26: paired, 28: param, 29: true, 74: source, 71: 4, 129: false}` puts a parameter
      under a controller, and the same op with `74: 0` removes it — there is no separate unassign.
      **Ops 65/66** move the Min/Max ends, in the parameter's own units. Assigning `Mix` to FS1
      landed the entry at **`/4[3]`**, confirming the source-ordinal indexing a second time and by a
      different route. `edit::{assign_param, set_assign_travel}`, CLI `assign-param` /
      `assign-travel`.
- [x] **Reading a footswitch and an assignment from the device** — **op 33** `{102: switch}`
      (one-based in, zero-based out) answers what a switch carries, its label, LED colour and
      latching type; **op 36** answers one parameter's assignment, or `104: nil`. Both verified live
      2026-08-22. Cross-checks rather than new capability — the document already carries both — but
      op 36's reply is byte-identical to the document's own entry, which makes it a cheap way to
      confirm a write landed.
- [x] **Assignments in the GUI** — DONE (2026-08-22). The two mechanisms get two controls, because
      confusing them is the whole trap: the block header's `FS` badge became a **picker** for which
      footswitch toggles that block's bypass (one select, since re-sending op 56 *moves* a binding),
      and every parameter row grew a quiet `⇢` that opens a **Controlled by** source picker with
      Min/Max travel sliders in the parameter's own units. `PresetDto` carries `assignments`
      (source, resolved parameter name, travel) and `footswitch_count`; four commands wrap the
      session methods. The source list is built from `footswitch_count`, which comes off the preset's
      own layout, so a Floor will offer its own number without the UI being told about Floors. MIDI
      is left out — it needs a CC number, which is a separate opcode.
      Checked rather than assumed: these use the ordinary immediate re-read, **not**
      `read_preset_settled` — assign, remove and unassign all read back correctly on the next read,
      three rounds in a row, so the ACK-before-rewrite hazard that model swaps have does not apply.
- [ ] **Parameter controllers — what is left.**
      - Confirm **EXP1 = 1** and the ordinals past FS2. Needs an expression pedal; a Stomp's three
        switches leave most of the ID space unsampled, so a **Helix Floor** (8 switches, 2 pedals)
        remains the better instrument for the full map.
      - Decide **key `1`** positively. "4 a parameter, 0 a bypass" stays **refuted**. `tonepush`
        reads it as the **MIDI CC number**, a constant 4 under any source with no CC to give, and the
        op-37 write agrees (assigning to FS1 stored `1: 4`) — but that is corroboration, not proof,
        and telling it apart from a "value type" reading needs a MIDI-sourced sample, which a Stomp
        cannot make alone.
      - **Ops 58-62** (momentary/latching, custom switch label, LED colour) and **op 64** (a
        parameter's MIDI CC) are documented by `tonepush` and untried here. Not needed for the
        assignment itself.

- [ ] **Tempo-sync as one control** (issue #5) — HX Edit and the pedal both fold `TempoSync{n}` /
      `SyncSelect{n}` into the time knob: switch sync on and the knob becomes a note-value selector
      (`1/4`, `1/8 Dotted`, …). We list all three as separate rows instead. Everything needed to
      *render* it is already in hand — `sync_note` is a discrete control with its 19 labels, and the
      dropdown works today.
      **Blocked on evidence, not effort:** nothing in the shipped data says *which* param a sync pair
      governs. Checked and refuted 2026-08-21 — position doesn't encode it (`Level` immediately
      precedes `SyncSelect1` in 57 models), and `assign` is amp-knob ordering, not this (Dual Delay
      assigns 3/4/5/6 against syncs 1/2; several sync-bearing models have no `assign` at all). 107
      models carry a sync pair and 14 carry two, so a name heuristic would be guessing on ~14 models
      where guessing wrong silently reassigns a control. Look in `HelixModelDefs.bin` or
      `HX_ModelCatalog.json` for a stated grouping before writing UI.
      Available now without any of that: hide `Note Sync` while `Tempo Sync` is off — that pairing
      *is* unambiguous, being the same ordinal.
      (The note *values* were off by one until 2026-08-21 — a discrete control's labels span the
      param's `min..=max`, and `sync_note` starts at 1. Fixed for every enum, issue #8; see STATUS
      "thirty-first round". Unrelated to the grouping question above.)

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
- [x] **Fast setlist export** (2026-08-21). `export-setlist` used to `goto` each slot and read it
      back — loading, settling and confirming 128 presets, which took tens of minutes and walked the
      user's pedal through every one of them. **Op 4 reads a slot's document in place**, byte-identical
      to the loaded read and without moving the panel: **126 presets in 10.7 s** on a Stomp. Falls
      back to the old sweep if the device refuses op 4 (untried on a Floor), and for the odd slot that
      answers `104: nil`. See `docs/protocol.md`.
- [ ] **Full device backup** — the thing "Backup" used to imply and does not deliver. A restore that
      makes a wiped pedal whole needs three parts, and we have one:
      **presets** (done — setlist export above), **global / I/O settings** (op 25's id space is
      barely mapped — see below), and **IRs** (op 9/12 transaction only partly decoded — see below).
      Gated on those two, in that order; the naming stays honest until all three land, because a file
      called a backup gets trusted as one. `fretwire_data::hxb` already reads HX Edit's own `.hxb`,
      which is the reference for what a real one contains — and a plausible import path once the
      `tone` JSON → wire blob conversion exists.
      > Decoding op 25 buys one small thing early: **preset numbering**. Whether the pedal writes
      > `01A` or `000` is a global, and it is **confirmed absent from every stream we already read**
      > — flipping it on a live Stomp left the browse listing and the preset stream byte-identical
      > (2026-08-21, [solid]). So the GUI carries a manual toggle
      > (`ui/src/lib/numbering.svelte.js`) and op 25 is the *only* thing that can replace it with a
      > detected default. Small, but it is the cheapest possible first consumer of the globals
      > decode — one field, no restore semantics, no risk.
- [x] **IR management — read and write** (2026-08-22, verified live). The one device capability
      HX Edit had entirely to itself, and the reason a Linux user still needed a Windows box.
      `Session::{ir_info,ir_directory,ir_export,ir_upload}`; CLI `ir-list`, `ir-info`, `ir-export`,
      `ir-export-all`, `ir-upload`; builders byte-exact against `captures/{import,export}_ir.pcapng`.
      An IR **round-trips bit-exact** — a blob read off the pedal matches the one the June capture
      recorded HX Edit uploading, and a slot written back from that file matches again.
      The `113` checksum (a little-endian word sum, not a CRC) was solved 2026-07-22 and this line
      went on saying it was the blocker; it was not.
      See `docs/protocol.md` "The user IR store".
- [x] **IR delete and rename** (2026-08-22, verified live) — op **15** `{112:slot}` empties a slot
      (afterwards it reads field-for-field like one never written), op **10** `{112:slot, 109:name}`
      renames. Both from `tonepush`'s `PROTOCOL.md`, not from a capture.
- [ ] **IR management — what is left.** **Reorder** is undecoded and may not exist as an opcode
      (delete + upload expresses it). Also unfinished: how a preset's IR block **references** a user
      slot vs a built-in cab IR.
- [x] **GUI IR panel** (2026-08-22) — toolbar **IRs…** opens an overlay: per-slot export/rename/
      delete, upload with a native picker and a target-slot picker that says what each slot holds,
      an optional empty-slot view (128 requests vs the directory's one), and confirmations that name
      what is lost. Slots are shown one-based, as the pedal's own menus number them. Mock backend +
      `npm test` contract check; **not yet clicked through by hand**.
- [~] **Global / I/O settings** — the **read side is decoded** (2026-08-22, live): **op 24**
      `{118:id}` answers with the value at key `119`, and 166 of ids 0..=260 answer on a Stomp.
      Named: **16** tempo BPM (`f32`), **28** current preset index, **192**/**201-203** global EQ.
      Settings are **typed** and a wrong-typed write is refused `-3`, so `set_setting_num` reads
      before writing. Op 24 was already in the tree misnamed `OP_READ_PREP` — the handshake had been
      calling it since day one.
      **Mapping the rest needs no capture:** `settings-dump`, change one thing on the pedal,
      `settings-dump` again, `settings-diff`. Verified — a tempo move showed up as exactly one id
      out of 166. Priorities: Input Z, guitar pad, main out level, and the preset-numbering flag.
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

- [x] Identify the device: PID `0x4248`, preset `device` ID `0x210001`, version word `0x03800000`
      (*not* the firmware version — a 3.80 Stomp reports it too). Constant +
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
      plugged in. The HX Stomp XL is `Reported` (2026-08-20) — an owner has the editor working
      against one, and both bugs they filed reproduced on an HX Stomp — with its unknown fields
      still honestly `None`, because "a user says it works" fills in none of them. We have no
      capture, preset or backup from one, so opening it still logs a caveat. Tests pin the
      invariants, including that every table entry has a matching udev rule.
      **Still open on the XL:** how many setlists it has. Its banking was answered on 2026-08-21
      by the owner reading `01A`-`32D` off the panel, and its model code `P36` by their handshake log.
- [x] **Helix LT — PID `0x424A`, `Reported` (2026-08-22, PR #3).** Surveyed on a contributor's
      physical unit: handshake, preset read, snapshot decode and the setlist/preset browses all work
      **unmodified** once the PID is in the table, so the LT needs no protocol change. It stamps the
      Floor's `P21` and carries the Floor's geometry (2 DSPs, 8 snapshots, 8 banks of 128), so
      `by_model_code("P21")` keeps resolving to the Floor — they are one data class. Survey:
      **`docs/helix-lt.md`**.
      **Still open on the LT:** its `preset_device_id` (never on the wire; the Floor's came from a
      `.hxb`, and we have no LT backup), how its screen banks presets, and **every write path** —
      no edit has ever been sent to one, which is why it is not `Verified`.
- [x] **HX Effects — PID `0x4245`, `Untested` (2026-08-22, issue #10).** A contributor ran
      `lsusb` and sent the line: `ID 0e41:4245 Line6, Inc. HX Effects`. That is the whole of the
      evidence, so it is the table's first `Untested` entry — `detect` finds one, the udev rule
      covers it, opening it warns, and every other field is `None`. It is the family member least
      like the rest (effects only, no amps or cabs), so nothing is inherited from the Stomp.
      **Still open:** everything else. One `pull` from an owner would settle its model code and
      preset geometry.
- [x] **Global settings — op 24 reads, op 25 writes, 27 ids named (2026-08-22, and eight Ins/Outs
      ids from an HX Stomp XL owner on 2026-08-23).** The namespace is
      flat and numbered; a 601-id sweep costs 1.4 s, so `settings-dump` / `settings-diff` maps it
      with no capture at all. `fretwire_protocol::settings` is the shared table, the CLI has
      `setting-get`/`setting-set`, and the GUI has a **Globals** panel. Id 27 (preset numbering)
      retired the manual toggle it was blocking.
      The global EQ is fully mapped: `190`-`200`, three bands of frequency/Q/gain then the two
      cuts, and the GUI draws it as a response curve.
      **Still open:** 138 answering ids are unidentified; `127` (**Auto In-Z**, renamed from a
      mis-transcribed "Guitar In-Z" on 2026-08-23) has two observed values and neither is named;
      `201`-`203` are unknown (an earlier "global EQ" gloss was withdrawn). Writes are gated to
      identified ids only.

- [x] **The footswitch record is decoded (2026-08-22).** Op 33 returns
      `{102: switch (zero-based), 65: ?, 109: label, 66: ?, 67: [assignments]}`, and an assignment is
      `{59: enabled, 68: ?, 66: colour, 69: {109: name, 98: slot, 28: param, …}}`. **Key 66 is the
      LED ring colour, `0xRRGGBB`** — proved by binding two blocks of different categories, a delay
      coming back `0x06FF00` (green) and an amp `0xFF0003` (red). The same key in the type-41 status
      push is the ring's *current* colour, bright when engaged and ~1/16 brightness when bypassed,
      which refutes the "state bitmask" reading that section carried.
- [ ] **Custom footswitch colours and labels.** The feature: pick a ring colour and a name per
      footswitch in the GUI, the way HX Edit does, instead of inheriting the block's category colour
      and name. Wanted — it is one of the few things HX Edit still does that we cannot.

      **Reading is done** (above): op 33 returns the record, `109` is the label, `67` the assignments
      array, and `67[].66` the colour as `0xRRGGBB`. Top-level `66` stays `nil` while the
      assignment's own colour is set, so it is very likely the per-switch **override** — the field
      this feature writes. That read-back is what makes the write tractable: we can tell whether an
      attempt landed without re-reading the preset.

      **Writing is ops 58-62, and probing them is expensive.** They are documented by `tonepush` and
      untried here. `probe-edit --op 58 --set 102=1 --set 66=255` **wedged an HX Stomp** and cost a
      power cycle (2026-08-22; `docs/safety.md`). All five ops accept a bare `{102: switch}` and do
      nothing with it, so acceptance says nothing about whether the body was understood. At current
      knowledge that is roughly one power cycle per guess.

      **Do not resume by guessing bodies.** Get `tonepush`'s op documentation first, or capture HX
      Edit setting a colour on Windows — either turns this from a search into a confirmation. Then:
      one op, one body, look at the device, and nothing unsaved on the pedal.

      Open sub-questions: which of 58-62 is which; whether `65`/`68`/`26`/`120` matter; and whether
      the colour is written on the switch record or on the assignment inside it, since both carry a
      key 66. An HX Stomp cannot set a custom **label** from its own panel, so that half can only be
      confirmed by writing it — there is no read-only route to it.

      Once it lands, the GUI already has the pieces: the footswitch binding UI exists, and
      `Catalog::category_color` gives the inherited colour to show as the default a custom one
      departs from.
- [ ] **`assign-bypass` leaves the switch label unset where the pedal sets it.** A switch bound from
      the hardware carries `109: "<block name>\0"`; one bound by our op 56 carries `109: nil`, with
      the name only inside the assignment. Whether the pedal fills it in on its own schedule or op
      56 is missing a step is not settled. Low stakes — the pedal still shows a sensible label — but
      a switch we bind is not byte-identical to one it binds, which matters before anything reads
      that field.

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
