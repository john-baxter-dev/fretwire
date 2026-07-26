# Status & Handoff

_Snapshot: 2026-07-05. Target: an independent Linux editor for the HX Stomp, in Rust._

## GUI direction change (2026-07-05): migrating to Tauri
The iced GUI is capped by its renderer: it's on tiny-skia (wgpu is ruled out by EGL/dmabuf driver
issues here) and tiny-skia **can't stroke paths**, so the routing UI can't draw wires/branches. A
spike (`crates/fretwire-tauri`) confirmed **Tauri (system WebKitGTK 4.1) is the way forward**: it links,
runs stably with `WEBKIT_DISABLE_DMABUF_RENDERER=1` (baked into `main()`), and renders SVG wires +
the split/rejoin branch on the real 2-row grid — reusing `fretwire-core` unchanged via `#[command]`s
(`detect`, `pull`). The Rust core (protocol/transport/decode) is untouched.

**Update (2026-07-06): the Tauri GUI is now at feature parity with iced**, all verified live —
connect/disconnect (clean teardown), preset browser (load/save/Save-As/rename), chain + block
selection, param editing (sliders/enums/switches + paired cab), bypass, model swap (DSP-fit
greyout), add/delete, snapshots, split-type + split/mixer node editing, live-follow
(footswitch/snapshot/preset pushes; needs the `core:event` capability), greyed bypassed blocks, and
the **interactive routing grid** (drag blocks across a 2-row × N-col grid of slots;
`PresetStream::grid()` + `Session::place_block`). The iced GUI (`fretwire-gui`) was **removed 2026-07-21**;
the sections below describing it are kept as history.
See ROADMAP for remaining follow-ups.

**Routing flexibility (2026-07-06, verified live):** the grid now covers the full parallel-path
lifecycle — **serial→split creation by drag** (the split/mixer node slots exist in the 20-slot array
even on serial presets [solid, preset1 fixture], so the empty B row is revealed during a drag and one
`place_block` into it makes the device activate the split; moving the last B block back retires it)
and **movable split/join nodes** (drag ⋔/⋉ to a new signal column — the position is the node
holder's key 13, written via the op-21 whole-preset write; `PresetStream::set_node_pos` +
`Session::set_node_pos` with enclosure guards; first live op-21 write of a *mutated* blob, device
honors it verbatim [solid]).

**Editor depth (2026-07-06 evening, mock-verified; live pass pending):** **undo/redo + a labeled
edit-history timeline with A/B compare** (blob snapshots restored via the op-21 edit-buffer write;
`Session::{edit_begin,edit_commit,history_jump}`; HistoryPane UI; labels name real blocks/params),
**input/output node editing** (slots 0/9: gate/threshold/decay, level/pan — plain set-value on the
node's slot [solid, input-gate capture]; io.models meta now bundled), category-colored blocks,
wheel-nudge sliders, click-empty-cell add (`add_block_at`, guarded), in-app dialogs/toasts, Save As
with a full setlist slot picker. Global settings (Input Z/impedance, pad, output levels) are **not
decoded** — see `captures/_TODO-global-settings.md`.

**Preset backup/restore (2026-07-07, offline+mock verified; live pass pending):** the proven op-21
write unlocked Phase 7 — `Session::backup_setlist` sweeps the setlist (goto → read each preset's raw
stream, op-23 identity cross-checked per slot, cursor put back at the end; reads only) into an
`fretwire-backup` JSON file (`fretwire_core::backup`, hex-encoded raw streams — the parseable form; the writable
blob is derived at restore via `parse → to_blob`). `Session::restore_preset` = goto slot → op-21
edit-buffer write → op-71 save (persistent). CLI: `fretwire backup <out.json>`, `fretwire restore <file> <index>
[slot]`, `fretwire backup-show <file>` (offline). GUI: Backup…/Restore… in the preset sidebar — progress
overlay (`backup-progress` events), restore dialog with source-entry + target-slot pickers showing
exactly what gets overwritten. **Live caveat:** op-21 has only been proven with mutated blobs of the
*current* preset; a foreign blob (restore) is the same mechanism but `[hypothesis]` until the first
live restore test. Backup itself is pure reads — safe to run first.

**Settled read-back after structural edits [solid — live 2026-07-20]:** a model swap updated the
device and the chain, but the param panel kept showing the *previous* block until you clicked a
second time. Cause: op-40/op-39 ACK once the device has taken the new **model reference**, then
rewrite the block's **param area** a moment later — so the read-back decoded the new model's identity
against the old model's values (`editor::build_block` names params from the new `Helix.sym` order).
The second click only "fixed" it because its read landed after the device had settled. Fix:
`Session::read_preset_settled(slot)` re-reads until the decoded block stops changing (40 ms apart, 4
attempts, then return the last read rather than erroring); used by the Tauri `swap_model` /
`add_block_at` commands (now `returning_edit`) and `Session::add_block_append`. Also fixes the undo
timeline — `edit_commit()` snapshots `last_raw`, so a swap could previously record a **mid-apply
blob** that undo/redo would replay. See `docs/protocol.md` "The ACK precedes the param rewrite".

**Editor round (2026-07-08, offline+mock verified; live pass pending):**
- **Live-follow bypass fix:** footswitch pushes now overlay onto the re-read preset in the GUI (the
  device's readable stream lags its own push — same reason the snapshot handler trusts the push).
- **Segmented params:** cab mic `Angle` is a float on the wire (0–45) but HX Edit renders it as two
  positions — `HelixControls.json` `controlType:"segmented"` without `isDiscrete`, stop spacing from
  `displayToWidgetScale`. Now `ParamMeta::stops` → 0°/45° buttons instead of a slider (data-driven;
  it's the only such control in today's data).
- **Change cab on amp+cab combos:** "Change cab ▾" in the param panel — same op-40 `swap_model`
  re-sending the block's own `model_index` with the new `paired_index`. **[solid — live
  2026-07-09]: a same-model swap keeps the amp's knob values** (a *different*-model swap resets
  params [solid]); the new cab arrives with its own factory defaults.
- **Wire key 23 = paired-model-active flag [solid — live 2026-07-09]:** op-40/op-39's model map
  (`{23, 25: model, 26: paired}`) had `23` hardcoded `false` (pinned from a paired=-1 capture) —
  the device then stores the `26` cab index but **never instantiates the cab** (empty key-12 param
  vector, no cab in the path; the GUI showed "+ cab" with no cab pane). Real preset blobs carry
  `23: true` on amp+cab blocks. Builders now send `23 = (paired_index >= 0)`; the device
  initializes the cab's params itself. Diagnosed by diffing a healthy fixture block against a live
  dump of the broken state. (Also decoded in passing: `delete_cab.pcapng` → **op 28 `{98:slot}` =
  remove paired cab** — unimplemented, cheap follow-up.)
- **Smooth param ramping [live-testable]:** sliders/wheel now stream `preview_param` /
  `preview_paired_param` (fire-and-forget op-30, no history, no re-read; ~60 ms latest-wins pump in
  ParamPanel) during the drag, so audio ramps like HX Edit; release still lands the ordinary
  history-tracked commit.
- **Amp+Cab picker category:** synthetic category id 100 — amps paired with their `amp.models`
  `ircablink` cab (combined DSP cost, `ModelChoice::default_paired_index` → add/swap). Note "4x10
  US Super" & friends live in **"Cab (Mic+IR)"** (category 19), not legacy "Cab" (2) — both listed.
- **Dirty indicator:** `Session::saved_cursor` vs history cursor (`dirty()`), stamped into
  `PresetDto` — "● edited" by the preset name + a dot in the sidebar; save/undo-to-saved clears it.
  Tracks GUI (history-bracketed) edits; device-panel edits don't register.
- **Snapshot rename:** op 89 (already decoded) exposed as `rename_snapshot` command — double-click
  a snapshot tab. Undoable (blob timeline), dirties the preset.
- **Spacebar** toggles the selected block's bypass, like HX Edit.
All mock-verified end-to-end against the browser mock backend; 121 offline tests.

**Runtime data loading + data removed from git (2026-07-18):** the publishing blocker is fully
resolved. `Catalog::load()` reads the reference files from `fretwire_core::data_dir()` at runtime (the
`fretwire import-data` cache); the `include_bytes!` embed is behind the **default-off** `bundled-data`
feature, so a default `cargo build` ships **zero** proprietary data and errors with a pointer to
`fretwire import-data` if the cache is absent (`--features bundled-data` embeds a local copy, ~2.9 MB
larger, for a self-contained dev binary). `crates/fretwire-data/data/` + `res-extracted/` are `git rm`'d
and `.gitignore`d (local dev copies kept). Data-dependent tests are gated on the `have_bundled_data`
build cfg (set by `build.rs` when the `fretwire import-data` cache is present) → full suite on a dev box, skipped on
a clean clone (verified: hide the data dir → still builds, 72 tests pass). `from_data_dir` is
parity-tested against `bundled()`. **The repo is now publishable data-wise.**

**Mock backend (2026-07-06):** the Svelte frontend can be developed with no hardware, Rust, or Tauri
— `cd crates/fretwire-tauri/ui && npm install && npm run dev` runs it in a browser against an in-memory
mock device. `src/lib/ipc.js` is the seam (routes `invoke`/`listen` to the real Tauri API or the mock
based on whether `window.__TAURI_INTERNALS__` exists); `src/mock/backend.js` implements every command
in the `dto.rs` shapes (setlist + model catalog + split routing + live-follow via `window.fretwireMock`).
See `crates/fretwire-tauri/ui/README.md`. Keep the mock in sync when adding backend commands.

**First-run data import + distributable packages (2026-07-21):** the GUI no longer dead-ends on a
fresh install. `fretwire_core::import` now owns the import mechanics (moved out of the CLI), exposed
as the `data_status`/`import_data` Tauri commands and a `FirstRun.svelte` screen with a native file
picker (`tauri-plugin-dialog`) — choose an HX Edit installer or an extracted `res/` folder, or skip
and edit with numeric names. Verified live: an empty data dir shows the setup screen, a populated one
goes straight to the editor. Packaging is wired up too — `tauri build` emits `.deb`/`.rpm`/AppImage
(`bundle.active`, icons generated from `packaging/icon.svg`), the deb/rpm **install the udev rule**
and ship the CLI alongside `fretwire-gui`, and `.github/workflows/release.yml` builds them plus a
static musl CLI on a `v*` tag. An AUR `PKGBUILD` is in `packaging/`. Verified by building a real
`.deb` and inspecting it: `Depends: libwebkit2gtk-4.1-0, libgtk-3-0`, rule at
`/usr/lib/udev/rules.d/70-hxstomp.rules`, both binaries in `/usr/bin`.

## TL;DR
**The protocol is essentially fully decoded, and there's a working graphical editor**
(`fretwire-tauri`, Tauri + Svelte) on top of it — all verified live on the HX Stomp on Linux. Clean
build. The GUI does, live: **connect → see the current preset → browse/switch all 126 presets →
signal-chain view → select a block → bypass → param sliders (main + paired cab) + enum dropdowns →
swap model (any category, DSP-fit grey-out) → drag-to-reorder (insert into gaps) → add block →
move blocks to/from a parallel (B) row (create/retire the split) → snapshots → save to flash → clean
teardown**, and it **live-follows the hardware** (footswitch bypass / panel snapshot+preset changes
update the GUI via the status-channel state-push). The CLI (`fretwire`) exposes the same ops + probes.

Decoded ops (all byte-exact-tested vs captures, in `fretwire_protocol::edit` / `docs/protocol.md`):
set-value 30, bypass 41, select-preset 20, read 76/24/23/22, **read-info 23 = current preset
identity**, snapshot 88, rename-snapshot 89, save 71, **rename-preset 6** (name-only, primary
channel), setting 25, swap-model 40 (also split-type), **move-block 43**, **add-block 39**,
**delete-block 28** (surgical), **begin-structural 78**, **whole-preset write 21** (chunked,
verified live), browse/list 254/0/1, and the device→host **status-push** (`{105:type,106:payload}`).

**Structural editing (Step 2, 2026-06-29):** drag-to-reorder (op-43 bubble through empty slots),
add-block (op-39 append), move-to/from-parallel (op-43 to a row-B slot), **delete-block (op-28,
surgical)**, **parallel routing** (split-type swap + mixer/split node params). The **op-21 whole-preset write** is implemented and verified live
(`PresetStream::to_blob` + chunked `Session::write_preset`), but is now only needed for dense
rewrites (backup/restore) — **delete uses the surgical op 28**, which (like moves/adds) **preserves
the footswitch layout** (the device drops only the deleted block's own binding). op-21 remains the
one op that makes the device re-derive & wipe the whole FS layout, so it's reserved for restore.

**Delete + name-only rename — DONE (2026-06-30), pending live test.** Decoded from the four
`delete_*` / `change_amp_drive_rename_*` captures: **delete = op 28** `{98:slot}` (surgical, HX Edit
optionally prefixes op 78; we mirror that) — `edit::delete_block`, `Session::delete_block`, CLI
`delete-block`, GUI **✕ Delete** on the selected block. **Rename = op 6** `{107:bank,108:slot,
109:name\0}` on the **primary** channel — name-only, does **not** commit the edit buffer (the capture
proved a pending drive edit did not persist across the rename) — `edit::rename_preset`,
`Session::rename_preset`, CLI `rename`, GUI **Rename…** field (no confirm, HX Edit semantics). Both
byte-exact-tested.

**Parallel routing — DONE (2026-06-30), pending live test.** The split & mixer nodes (kinds 2 & 3,
slots 10 & 19) are now editable: **split type** = swap-model (op 40) to Helix.sym 256 A/B / 258
Crossover / 563 Dynamic (`cycle_through_split_types`); the split's own params and the mixer/join's
A/B **level/pan/polarity/level** (model 151 `HD2_AppDSPFlowJoin`) are plain set-values on the node's
slot (`adjust_A_B_level_and_pan_of_join`). `EditorPreset.{split_node,mixer_node}` +
`PresetStream::structural_node` expose them (outside `blocks`, so reorder/DSP/row logic is
untouched); `editor::SPLIT_TYPES`, `Session::set_split_type`, CLI `split-type`, and a GUI **Parallel
routing** panel (type dropdown + node param grids, reusing the standard param handlers via
`EditorPreset::block[_mut]`). All surgical/FS-safe.

**Cab/IR param editing — DONE (2026-06-28).** A paired-cab param edit is an ordinary set-value with
the **sub-model selector key `26:1`** (main model = `26:0`); the param index is positional in the
cab's namespace (mic=0, position=1, distance=2, angle=3, …), enums send `119` as int, knobs as f32.
Decoded from the cab captures; `edit::{set_paired_value,set_value_on}` (byte-exact), `Session::
set_paired_param`, CLI `set-cab`, and the GUI cab grid is live-editable (verified live). **Enum
params** (incl. the cab **Mic** selector) now render as **dropdowns**: `valueType:0` params get their
labels from `HelixControls.json[displayType].format` and send the chosen index as an int via
`Session::set_param_enum` — generic for any discrete enum. *Dropdown pending live test.*

**IR management — transaction shape PARTIALLY DECODED** (2026-06-28, `captures/import_ir`+`export_ir`,
notes in `captures/_TODO-ir.md`): PRIMARY channel, session op 255/254; **upload = op 9** (slot, u32
checksum, 32B name, 8192-byte = 2048×f32 blob, format flags) + op 13 commit; **export = op 12/11**,
paged. Before implementing the flash write: reassemble the blob, confirm the checksum algorithm,
decode the format flags (needs more captures).

**Remaining (see ROADMAP Phase 6–8):** mostly UI — drag-and-drop move, add affordance, split-type
dropdown; plus **backup/restore** (restore needs the op-21 serializer), **IR management** (see above),
and **publishing** (`fretwire import-data` done; the `include_bytes!` → `from_data_dir` flip **done**
2026-07-18 — `Catalog::load()` reads runtime data, `bundled-data` feature default-on for dev but a
release builds `--no-default-features` to ship data-free; only `git rm`-ing the bundled data +
packaging remain).

⚠️ **Protocol correction (2026-06-23):** op `100:20` with `{107,108}` is **SELECT PRESET** (changes
the active preset), *not* an "open for read". The earlier `read_preset` used it and was navigating
the device. The **non-destructive read** is a different op sequence — see below / `docs/protocol.md`.

## What works (tested)
- **`fretwire-protocol`** — frame codec (decode/encode byte-exact vs real full frames; header fully
  resolved — `arg` is a per-channel stream offset, no checksum), TLV body,
  big-endian f32 value helpers, `session::primary_handshake()`.
  **`edit::EditBody`** — the edit-command body is **MessagePack**; param selected by index (key 28).
  Parses + **builds** bypass and set-value commands (`edit::bypass`/`set_value`), byte-exact vs
  captures. 14 tests.
- **`fretwire-data`** — parses every shipped `.models`/`.hlx`; presets round-trip losslessly. Decodes
  the device's **MessagePack preset stream** into a typed model: `PresetStream::{device_model,
  firmware, blocks, path_blocks, loaded_blocks, is_split}`. **Blocks are enumerated from the slot
  array `0 → 22`** (not the footswitch layout `3 → 8`) → serial *and* split presets, all blocks.
  **Model identity = `24 → 25`, an index into `Helix.sym`** (833 symbols) → exact symbol (with
  Mono/Stereo) → `symbolicID` via `ModelDefs::id_by_symbolic_id`. Amp+cab pair via `24 → 26`.
  Per-block param values labeled in the symbol's own order. 25 tests.
- **`fretwire-core`** — **editor model** (`Catalog::load_preset`): composes the above into typed
  `EditorPreset`/`EditorBlock`s — resolved name + `symbolicID` + variant + category, **named**
  params with values, paired cab/IR + its params, bypass state, footswitch, row, split flag — and
  emits byte-exact edit commands (`set_enabled_edit`, `set_param_by_name`). Decodes a hand-built
  serial+split pair and the factory capture (6 blocks). 6 tests.
  **+ `Session`** (live, over `fretwire-usb`): `connect()` (handshake — **works**, device reports
  `"P33Main"`), `read_preset()` (**works** — non-destructive, reassembles ~2.8 KB → 4 named blocks,
  repeatable), `set_bypass()`/`set_param()` (**work** — edit ACK confirmed, value verified by
  read-back; arg-accounted via `edit_request`, with a best-effort `cmd 0x08` follow-up),
  **`close()`** (clean teardown; also runs on `Drop` — **the panel-lock fix, verified live**), raw
  `request()`.

## Recent finding (2026-06-24/25): session teardown = the "panel lock" fix (VERIFIED LIVE)
Captured HX Edit's shutdown (`captures/launch_hx_*_close*.pcapng`). On exit it sends a **session-close**
frame (`cmd=0x02`, empty body) on each channel in order **status → edit → primary**, each acked. Our
`Session` previously just dropped the USB handle, leaving the pedal in the editor-connected state —
the observed "device acts wonky / panel locked after our software controls it." **Fixed & verified on
hardware (2026-06-25):** `Session::close()` (+ a `Drop` impl) sends the teardown; `fretwire disconnect`
exercises it. **Key nuance found on hardware:** the close must be **request/response** (read each ack)
with a brief **settle (150 ms) before the interface is released** — firing the frames blind and
dropping the handle immediately does *not* release the panel. status + edit ack promptly; **primary
never acks** (our reconstructed handshake diverges on that channel) and the panel releases regardless,
so each ack wait is capped at 300 ms (`CLOSE_ACK_WAIT`) to keep every `Drop` teardown fast. Also
decoded `switch_input_gate_and_guitar_pad.pcapng`: global/input settings use a **new op 25** targeted
by `{118: id, 119: value}` (not a block slot) — see `docs/protocol.md`.
- **`fretwire-usb`** — `nusb` enumeration (`fretwire detect`) **+ `Transport`** (claim iface 0, bulk EP
  0x01/0x81). **Runs on hardware.** `request()` matches a reply by **channel (`dst==src`) + "not a
  keepalive" (`cmd != IDLE`)**, skipping status meters and interleaved keepalives, and splits
  **batched** bulk-IN reads. ⚠️ **The device does *not* echo our `seq`** — it runs its own per-channel
  counter; matching on seq only worked by luck in short CLI bursts and broke once the GUI's heartbeat
  desynced the counters (it rejected the device's real reply). Bulk reads are bounded by a timeout so
  a desync errors cleanly instead of hanging. Needs Linux — Windows can't claim the interface.

## Persistent session = keepalive heartbeat (2026-06-25, GUI live editing)
A session held open between edits (the GUI does this; the CLI never did — it ran in <1 s bursts)
**must be serviced on a heartbeat** or the device stops responding on the edit channel after a few
seconds idle (and its own queued keepalives pile up unread behind the next edit's reply).
`Session::keepalive()` sends an idle (`cmd 0x10`) on each channel + drains the device's queued
frames; the GUI ticks it ~4×/s while connected. Also: the post-edit `cmd 0x08` follow-up is
**fire-and-forget** (it gets no distinct reply, only keepalives), and `send_edit` drains stale frames
first so the edit's reply is the next one read. Verified live: idle-then-edit and repeated edits
both reliable.

## GUI — `fretwire-gui` (iced, software renderer) — REMOVED 2026-07-21
_History only: superseded by `fretwire-tauri` and deleted from the tree. Kept because the phase
notes below record what was verified live against the device._
A graphical editor (`fretwire`) on **iced 0.13** (tiny-skia software renderer — wgpu hit EGL/dmabuf
issues on Linux). Working: render a preset (offline file arg **or** live), **Connect & Pull** /
**Disconnect** (async `spawn_blocking`, no UI freeze), the keepalive heartbeat, and **live bypass
toggles** against a session held open in `Arc<Mutex<Option<Session>>>`. Block/param tree + DSP meter
shown. **Phase 4 (verified live): live param sliders.** Each float param with a known range renders
a slider; dragging updates the UI, **releasing** commits one `set_param` to the device (release-only
commit avoids flooding the edit channel mid-drag). Powered by new param-range metadata (below).
**Phase 5 (verified live): preset list + switching.** Connect also pulls the full preset list
(`list_presets`); a left sidebar lists all 126, clicking one runs `goto_preset` + re-reads the
preset so the whole tree/meter/sliders refresh. The active preset highlights; the list is disabled
and the heartbeat paused mid-navigation (no stacked `goto`s / lock collisions). Browsing then editing
works fine on the same held session.

**Phase 6 (verified live): the signal-chain view.** The blocks render as a horizontal path of
clickable boxes connected by wires (`IN ─[..]─[..]─ OUT`); split presets show row A and row B. The
selected block highlights; clicking one shows its params in a panel below the chain (two-column
sliders + bypass). Built from **plain iced widgets** (buttons + `horizontal_rule`), **not a canvas**:
the tiny-skia software renderer renders fills but does **not** stroke canvas paths, so wires and box
borders vanished — widgets render reliably and reserve their own height (no overlap). Controllers are
omitted from the chain; the parallel split is drawn as two full rows (exact split/merge point TBD).

**Phase 7 (verified live): model list + swap.** Selecting a block reveals a **Change model ▾** picker
listing every model in that block's category (`Catalog::models_in_category`), each with its DSP cost;
the current model is marked, and models that wouldn't fit the remaining DSP budget are greyed out
(disabled). Clicking one runs `swap_model` (preserving the paired cab via the new
`EditorBlock::paired_index`) and re-reads the preset. Candidates are de-duplicated per model and
**deterministically ordered**; models that share a display name (e.g. the amp vs preamp "EV Panama
Red", both in the amp category) are disambiguated with a type token — fixing a flicker where tied
names shuffled (HashMap order) and their DSP numbers appeared to change.

**Phase 9 (verified live): snapshots + save + refresh in the GUI, and txn-correlated reads.** The
GUI now switches snapshots, **saves to device** (overwrite-to-flash, two-click confirm), and has a
**⟳ Refresh** to re-read after panel-side changes. The active snapshot is tracked in app state (the
stored `active_snapshot` lags a live switch). The big reliability fix: **reply correlation by
transaction id**. On a held session the device interleaves keepalives and **state pushes** (footswitch
bypass, panel knobs); the old "next non-keepalive frame" match could grab one of those as a streaming
read's **chunk #0**, yielding a stream with no envelope ("key 104 missing"/"not an array") or a
timeout. Now `read_preset` and `list_presets` (1) drain at the start and (2) match each structured
step's reply by the **txn echoed in key 102** (`reply_txn` hand-reads the leading `102:<txn>` entry —
critical because the stream-start reply *is* chunk #0 with a truncated key-104 blob that won't fully
parse). `read_preset` also retries once on a decode failure (the raw pagination chunks carry no txn).
**Live-follow of panel changes (decoded + implemented 2026-06-26).** The **status channel is a
state-mirror push channel**: the device sends `{105:type, 106:payload}` unsolicited on panel changes —
bypass (`type 49 → {98:slot, 59:enabled}`), snapshot (`type 42 → {92:index}`), preset
(`type 4 → {108:index}`). Parsed by `fretwire_data::stream::parse_status_push` → `StatusPush` (byte-exact
tests); `Session::poll_events` runs the heartbeat and returns the pushes; the GUI applies them on its
tick — bypass mirrors in place, snapshot/preset trigger a re-read. So footswitch/panel changes now
update the GUI **without** the manual Refresh (kept as a fallback). See `docs/protocol.md` §Device
state-pushes. **Verified live.**

**Phase 8 (verified live): cross-category swaps.** The picker has a **Category** dropdown
(`Catalog::categories()` + `editor::category_name`), so a block can be swapped to *any* effect type,
not just its own — verified live (e.g. delay → reverb). The `.models` `category` field is its own
effect-type enum (1=Amp, 2=Cab, 3=Distortion, 4=Dynamics, 8=Modulation, 9=Delay, 10=Reverb,
13=Preamp, …), **distinct from `HX_ModelCatalog.json`** (where 1=Distortion). Same-category swaps
keep the paired cab; cross-category swaps drop it (`paired=-1`). **Add-a-block** (assign a model to
an empty slot) is likely a distinct op — capture-gated, alongside block-move.

**Structural edits decoded (2026-06-26, from the Windows captures).** Move = **op 43**
`{75:src, 76:dst}` (dst slot encodes the row → parallel path); add = **op 39**
`{98:slot, 99:{19:6, 20:{24:{23,25:model,26:paired}, 9:1, 10:true}}}`; split-type = **op 40**
(= existing `swap_model` on the split slot); the A/B join mixer is plain set-value. Builders
`edit::{move_block,add_block}` (byte-exact tests), `Session::{move_block,add_block}`, and CLI
`move`/`add-block` are in — **verified live**. HX Edit *can* also rewrite the whole preset via
op 21 `{110:blob}`, but the surgical ops cover everything. **Remaining: GUI drag-and-drop / add
affordance (UI only).** See `docs/protocol.md` §Structural edits.

**Current-preset identity (op 23, verified live).** The streamed preset blob carries no index/name;
the host learns the loaded preset from the **op-23 read-info** reply `{104:{107:bank, 108:index,
109:name, …}}` — which `read_preset` already issues (it was logging+discarding it). Now parsed by
`fretwire_data::stream::parse_preset_info` and surfaced as `EditorPreset::current`. The GUI highlights the
loaded preset on connect and trusts the device's read-info after navigation; the CLI `pull` prints
`Preset [N] Name`. Decoded from `startup.pcapng` via `tools/pcap-frames.py`; see `docs/protocol.md`.
**Next: the signal-chain canvas + the model list (swap by name).**

**Param range metadata (2026-06-25):** `EditorParam` now carries `meta: ParamMeta { min, max,
display_type }`, sourced from the `.models` files' per-param `min`/`max`/`displayType`. A param's
`symbolicID` *is* the name `Helix.sym` lists in the wire order, so the lookup is by name — verified
byte-for-byte (e.g. Harmonic Tremolo `Mix` → 0..1 `percent`, `BassFreq` → 40..2000 `frequency`).
`display_type` (`generic_knob`/`volume`/`off_on`/`sync_note`/…) is the widget hint for later phases.

## Protocol, in one paragraph
USB **bulk** EP 0x01/0x81, iface 0, libusb-style, strict request/response. 16-byte frame
(`len:u16, magic, src/dst u16 LE, seq, cmd, arg:u32`) + optional **TLV** body (`marker, type,
len, value`). Session = per-channel SESSION_OPEN + identity query (ef03→ed03→f003), then meters +
preset stream. **Open preset → paged MessagePack stream** carrying full state. **Edits are op 0x06
with a MessagePack body** `{102: counter, 100: op, 101: {98: slot, 28: param_index, 119: value}}` —
the param is selected by its **index in the model's device param order**, so editing is computable
from shipped data (bypass = bool @ key 59; knob value = BE f32 @ key 119). No per-packet checksum.
Full detail in `docs/protocol.md`, `docs/preset-format.md`.

## Key facts to remember
- Device: VID `0x0E41` / PID `0x4246`. Control on the vendor interface (MI_00).
- Parameter values are **big-endian f32**.
- Preset model: all blocks in slot array (0→22), identity = `24→25` Helix.sym index; key 3→8 is
  the footswitch layout (bound blocks only), not the signal path.
- **Model identity = `symbolicID`** (unique 681/681; no numeric model id). The `24→25` Helix.sym
  index resolves the *exact* symbol (incl. Mono/Stereo), so identity needs no name disambiguation.
  (Where only a name is available, a name + **param-count** tie-break would cover 150/164 collisions —
  analysis in `tools/analyze-name-collisions.js`; the index makes it unnecessary in code.)
- **Safety**: firmware/flash/DFU = the only brick risk, out of scope. Back up before any write.
  See `docs/safety.md`.

## Helix Floor — contributor captures (2026-07-22 … 07-23)
A Reddit contributor sent USB captures + a device backup from a **Helix Floor** (fw 3.82). Full
survey in **`docs/helix-floor.md`**. Headlines:
- **USB is a non-issue.** PID **`0x4248`** (Stomp `0x4246`); the vendor control interface is
  identical — iface 0, bulk IN `0x81` / OUT `0x01`, 512-byte packets. `PID_HELIX_FLOOR` and a udev
  rule are in; **nothing matches on it yet** (`Transport::open` is still Stomp-only, deliberately).
- **The data layer already covers the Floor.** All **355 models** and all **19,377 param keys**
  across the backup's 363 presets resolve against our imported catalog — 0 missing. `HD2_*` is a
  family-wide namespace, so `Helix.sym` param ordering (and thus `set_value`'s index math) carries
  over. HX Edit's shipped `default_preset.hlx`/`empty_preset.hlx` are *already* Floor presets
  (device `0x210001`, 8 snapshots); only `default_preset_hxs.hlx` is the Stomp (`0x210006`).
- **Topology delta:** 2 populated DSPs (Stomp: 1), ≥14 block slots/DSP, 2 paths per DSP, 8 snapshots
  (Stomp: 3), plus `footswitch`/`commandFS1..11`/`dt*`/`powercab*`/`variax` sections.
- **`.hxb` backup format decoded** — `"AF6L"` header + concatenated raw zlib streams (globals JSON,
  128 IR WAVs, a model-usage table, 8 × 128-slot setlist JSONs). Implementable as-is, and useful for
  the Stomp too.
- The **first** pair of captures contained zero MI_00 traffic (HX Edit wasn't running; the front
  panel alone emits nothing on the vendor pipe — only USB-MIDI Bank-Select+PC on preset change).

**Captures 3 & 4 (same day, HX Edit connected) — the blocker is cleared:**
- **The handshake is byte-identical to the Stomp's.** All ten host→device bring-up frames from
  `device_handshake()` appear verbatim in both captures — the exact hex asserted in
  `tests/handshake_fidelity.rs`. **No protocol change needed to open a Floor session.**
  ⚠ The handshake `arg` `0x21000100` *looks* like the Floor's device ID but the Stomp sends the same
  constant — it is not a device identifier.
- Floor **model code is `P21`** (Stomp `P33`); key `37` "firmware" is a bare build sha (`7d01f5e`).
- **Our parser reads Floor presets unmodified** — models, params, cab pairing, DSP load, footswitch
  bindings, and all **8 snapshots**. Verified against the same presets in the `.hxb` backup
  (independent ground truth): they reconcile exactly, once two gaps are fixed.
- **Two parser gaps, neither truly Floor-specific:** (1) slot **`type 7` = Looper** is skipped by
  enumeration (model idx at key `8`, params at `7 → 4`, not the type-6 shape) — no Stomp fixture has
  a Looper, so it never surfaced; (2) preset key **`1` is the second DSP's slot array** (same shape
  as key `0`, `nil` on the Stomp — hence the old "nil | ?" note). The array stays **20 slots**; the
  Floor adds a second one rather than widening it. `docs/preset-format.md` is corrected.
- New `tools/extract-preset-stream.py` reassembles chunked read-streams; self-checked by
  regenerating `captures/preset1_stream.msgpack.bin` byte-identically from the Stomp capture.
- **The write path is byte-exact on the Floor too.** These captures were HX Edit-driven and contain
  the full write path; our existing `fretwire_protocol::edit` builders reproduce the Floor's bytes
  exactly, **9/9 ops**: `set_value` (30), `bypass` (41), `begin_structural` (78), `swap_model` (40,
  incl. an amp+cab paired swap), `save_preset` (71, a real flash write), select-preset (20). Envelope
  shapes are key-for-key identical to the Stomp's. **So the Floor needs no protocol change at all —
  read or write.** Remaining work is the preset model (type 7, DSP2) and device matching.
- Unrelated bug found in passing: legacy (non-`CabMicIr_*`) cabs mislabel their trailing param as
  `Trails`. Affects the Stomp too.

**Capture 5 (2026-07-23) — the last open question is answered; no more captures needed:**
- **Wire slot numbers are global across DSPs: `slot = dsp * 20 + index`.** DSP1 is slots **0–19**,
  DSP2 slots **20–39**. There is no DSP field in the envelope and none is needed — `op 30`/`41`/`78`
  on a DSP2 block are byte-for-byte the same shape as on a DSP1 block, and no context op is involved.
- Evidence: one HX Edit session on `FACTORY 1` `12B` "Pull Me Under" touching **five DSP2 blocks**
  (`98` = 28, 33, 34, 37, 38). Under the rule each resolves to a block whose stored value for the
  swept param is one UI increment from the first value on the wire — five matches across five models
  and three parameter scales (dB, Hz, 0–1), including correctly picking the branch-B one of two
  identical `HD2_DelaySimpleDelayMono` blocks. Consistent with all earlier captures (slots all < 20,
  all DSP1), so nothing has to be reinterpreted.
- **This unblocks "walk preset key `1`".** `fretwire_protocol::edit` needs no change — it already
  takes a single slot integer, now well-defined for both DSPs. `(dsp, index)` is an internal
  traversal detail; `EditorPreset` and the routing grid are what have to stop assuming one 20-slot
  array.
- The same capture is also the read side of a **parallel, dual-DSP** preset — the other gap noted
  after captures 3 & 4. Both are now closed.

**Multi-DSP support implemented (2026-07-23):**
- `fretwire_data::stream` now walks **both** slot arrays and emits **global wire slots**
  (`wire_slot(dsp, index) = dsp * 20 + index`). `Block`/`LoadedBlock`/`GridCell` carry `dsp`;
  `dsp_blocks`/`dsp_grid`/`dsp_structural_node`/`dsp_io_node`/`dsp_is_split` are the per-DSP
  accessors, and the no-argument forms now mean DSP 0.
- **Slot `type 7` (Looper) is enumerated**, with its own content shape (model at key `8`, params at
  `7 → 4`). This fixed the Stomp path too — a Floor serial preset went 8 → 9 blocks.
- **`dsp_is_split` no longer tests `== 1`.** Non-zero means split and the value is the split *type*;
  the Floor uses 2 and 3. Checked against all five presets we have: `21 == 0` exactly when that
  DSP's row-B slots are empty.
- `EditorPreset` gained a **`DspView` per DSP** (own split/mixer/input/output nodes, grid, load).
  `split`/`split_node`/`input_node`/`grid`/… became accessors meaning DSP 0; `dsp_load_by_dsp()`
  reports each DSP's own budget. `EditorBlock` carries `dsp`, and `slot` is the global wire address —
  so **`fretwire_protocol::edit` needed no change at all**.
- GUI DTO is additive: `PresetDto.dsps[]` plus `dsp` on every block and grid cell; the flat fields
  still mirror DSP 0, so the Svelte UI is untouched and unaffected.
- **Verified against the real Floor captures:** "Pull Me Under" decodes all **15** blocks across both
  DSPs (was 7) with correct rows, per-DSP load (38.4% / 58.9%) and footswitch bindings — the FS
  labels only resolve because enrichment now matches on global slots. Matches the `.hxb` exactly.
- 16 new tests (10 in `fretwire-data`, 6 in `fretwire-core`) built on **synthetic** two-DSP presets,
  since the Floor captures can't be committed. Suite: **140 passing**.
- Still DSP-0 only: `Session`'s grid/routing *planning* methods (see the module note in
  `session.rs`) and the Svelte routing view.

**Device matching generalized (2026-07-23):**
- `fretwire_protocol::Device` + `DEVICES` replace the loose PID constants: PID, model code, preset
  `device` ID, DSP count, snapshot count, and a `Support` flag (`Verified` / `Untested`).
  `Device::by_pid` / `by_model_code` for lookups.
- **`Transport::open` is no longer Stomp-only.** It matches any device in the table, verified ones
  first, and `Transport::device()` / `Session::device()` say which was opened.
  `fretwire_usb::present_devices()` lists everything currently plugged in; `fretwire detect` prints
  them by name. Opening an untested device logs a warning rather than pretending it's known-good.
- The **HX Stomp XL** stays `Untested` with `None` for model code / device id / DSP / snapshot
  counts — we have no capture, preset or backup from one, and its fields must not be assumed to
  match the Stomp's just because the name is similar.
- `Session::device_matches_preset` compares the connected device's model code against the one
  stamped in the preset (`None` when either is unknown — not a mismatch, an unanswerable question).
  Surfaced to the GUI as `PresetDto.device_name` / `device_matches`.
- 8 new tests, including one asserting **every** table entry has a matching udev rule — a device
  without one silently fails with EACCES on a normal desktop. Suite: **148 passing**.

**FIRST HARDWARE RUN on a real Helix Floor (2026-07-24, the contributor "Sean"):** fretwire ran
against a physical Floor for the first time — via the Tauri GUI (`cargo tauri dev`). **It works.**
Interface claim, handshake, and the whole read path succeed; the GUI shows the live preset with the
correct **15-block** Pull Me Under decode, `device P21`, `fw 7d01f5e`, 8 snapshots, and combined DSP
%. **Writes work too** (he went past the read-only line): bypass toggles grey out *on the pedal*,
param edits (Hot Springs Dwell/Spring Count/Drip) move live, and **`save` persisted**. So on the
Floor: reads ✅, edits ✅, saves ✅, snapshots ✅, preset list ✅, device match ✅.

The one real defect: **preset-stream reassembly truncated intermittently and then wedged the wire**
(his "shits its pants and drops the connection"). Signature from his logs is unambiguous — **every
failed read reassembled to an exact multiple of 256; every good read ended mid-chunk.** The old
loop's rule was "the first chunk shorter than chunk #0 ends the stream", so a single **empty** chunk
reply mid-stream (a batched keepalive/state-push mistaken for a chunk, or a zero-length packet)
was read as the terminator → payload truncated at a 256 boundary → the unread tail desynced the next
transaction → cascade to `timed out waiting for a bulk IN`. Dual-DSP presets die sooner because
they span more chunks (more exposure), matching his observation exactly.

**Fix (2026-07-25):** the preset stream's envelope declares its own length
(`marker:u16,type:u16,len:u32(LE)` → stream is `len+8` bytes). `fretwire_data::stream::declared_stream_len`
reads it from chunk #0, and `read_preset` now makes that the authority for "done": it skips a
premature short/empty chunk and keeps reading until the declared payload is whole, falling back to
the short-chunk heuristic only when the length can't be read. Bounded against a garbage length /
non-terminating device. Also fixed: the handshake's model-string check was hard-coded to `"P33"`, so
every Floor connect logged a spurious `no model string seen` — it now keys off the connected
device's `model_code`. **Verified against the 3 tracked stream fixtures + unit tests; the on-hardware
confirmation is the next Floor run.** Suite: **152 passing**.

**GUI two-DSP routing grid (2026-07-25):** the grid used to render **only DSP1**; it now draws one
routing grid **per DSP** (both paths of each), stacked with a `DSP 1 / DSP 2` label + per-DSP load.
`Chain.svelte` takes a `dsp` prop (one `DspDto`) and `App.svelte` loops `preset.dsps`; a single-DSP
device / the mock passes no `dsp` and falls back to the flat DSP-0 fields (unchanged). The header
`DSP %` is now **per-DSP** (`38.4% · 58.9%`) instead of one summed >100% number, and — the
functional half — the model-picker / swap **DSP-fit greyout budgets against the block's own DSP
load**, not the combined total (which on the Floor exceeded the 100% budget and greyed out
everything). Structural-node selection (split/mixer/IO) now spans both DSPs too. The backend DTO
already carried all of this (`dsps[]` with per-DSP grid/nodes/load; blocks tagged with `dsp`), so
this was a frontend-only change; `npm run build` is clean.

**DSP-aware routing planner + two-DSP mock (2026-07-25):** the routing methods
(`add_block_at`/`place_block`/`insert_block`/`reorder_block`/`set_node_pos`) planned in local index
space against `dsp_blocks(0)`, so a drag or node-move on DSP2 hit the wrong grid. They now plan in
**global wire-slot space** (`dsp*20+index`) — each derives its DSP from the slot, reads that DSP's
blocks/grid via `Block::wire_slot()`, and feeds the base-agnostic `plan_*` helpers wire slots;
`place_block`/`insert_block` reject a cross-DSP move, and `set_node_pos` gained a `dsp` argument
(threaded from the UI). The browser **mock is now genuinely two-DSP** (stride-20 topology, a Floor
"Pull Me Under" demo preset at index 0), so the dual-grid path — render, drag, node-move — is
testable without hardware. New base-20 planner tests + end-to-end mock checks; suite **154**.
(`move_block_to_row`/`move_before_split` are legacy — CLI-only, not on the GUI grid path — but were
converted to wire space too on 2026-07-26, so they no longer silently plan against DSP 0.)

**Pre-build cleanup (2026-07-25):** the legacy-cab **`Trails` mislabel** is fixed (trailing extra is
`"Mic"` for cabs, `"Trails"` only for time-based fx — `editor::trailing_extra_name`), and the
dev-mode Svelte console warnings (Dialog role, ParamPanel labels, ModelPicker initial-value lint)
that cluttered the tester's logs are cleared. Suite **156**.

**Second hardware run (2026-07-26).** The reassembly fix holds: every read in Sean's logs now
reports `reassembled preset stream bytes=N declared=N`, including one that skipped a premature short
chunk mid-stream (`short chunk before declared stream end — skipping, continuing read got=7168
want=7373`) and completed correctly. No truncation, no wedged wire, no reconnect prompts, and the
GUI survived a long unattended session. DSP2 renders. Three new findings, all addressed:

1. **Row-B blocks rendered outside the Y-loop** — his "the y-loop renders the start of the loop on
   the right, and the mixer on the left". `dsp_grid` computed a row-B column as
   `slot − split_idx + 1` where `split_idx` is the split node's *slot-array index*, always 10 — so
   row B was pinned to columns 2..=9 and never consulted the split's signal-flow position, which is
   where the glyphs are drawn. Probing the fixtures pins the topology (slot 0 = kind 0 input, slot 9
   = kind 1 output): `[0=in, 1..=8 row A, 9=out, 10=split, 11..=18 row B, 19=mixer]` — **both rows
   are 8 columns**, so the column is `slot − 10`. Both split fixtures land their row-B block exactly
   at their split position under the new formula. Regression test asserts occupied row-B cells stay
   inside `split_pos..mixer_pos` and no column leaves the grid. *Every fixture we have has only one
   row-B block, so a `dump-raw` of a multi-B Floor preset (BMBLFOOT PRINCE, slots 13/14/15) is still
   wanted to close it out.*
2. **Setlists implemented.** The Floor has eight (Factory 1/2, User 1-5, Templates); we only ever
   browsed bank 0, so a unit in User 1 listed Factory 1's names — Sean's "the list on the left
   doesn't show the other user1 presets". Nearly everything was already plumbed (`PresetInfo.bank`,
   `goto/save/rename_preset(bank, …)`); the one defect was `edit::presets_stream` hardcoding the
   bank to 0. Now parameterised, `Device::setlists` names them, `Session::list_presets_in(bank)`,
   and the sidebar has a picker — **hidden on a one-setlist device**, as HX Edit does for the Stomp.
   Confirmed from traffic: `PresetInfo { bank: 2, index: 17, name: "Sludge" }` in User 1, so
   `Factory 1 = 0, Factory 2 = 1, User 1 = 2`; the rest of the order is off the unit's menu
   [hypothesis]. A panel-side preset change now also pulls the sidebar into the device's setlist.
3. **Wrong active snapshot — FIXED and confirmed on hardware (2026-07-26).** The preset blob's
   `10 → 8` is the snapshot **stored** with the preset, not the live one. Settled on our own HX
   Stomp: parked on SNAPSHOT 3, the blob reported **0**. Scene-matching (comparing the decoded
   per-snapshot bypass matrix against the live block state) couldn't decide it either — that
   preset's snapshots 2 and 3 held identical scenes, so the match was ambiguous. Both candidate
   fixes were therefore wrong.

   Decoding the **op-23 read-info reply** in full found the answer already on the wire:
   `{107:bank, 108:index, 109:name, 92:snapshot, 117:?, 83:[u32,0]}` — **key `92` is the live
   active snapshot**, the same key a snapshot status-push carries (`{105:42, 106:{92:n}}`). We
   parsed 107/108/109 and dropped the rest. In the `Dual Amp` capture key 92 = 0, matching that
   preset's live scene while its blob stores 1 — three independent signals agreeing.
   `PresetInfo::snapshot` carries it and `Session::read_preset` prefers it over the blob (blob kept
   as the fallback for offline decodes). The GUI needed no change: it reads `active_snapshot`, now
   correct at the source. Verified live via `fretwire pull`. Keys `117`/`83` still unidentified.

**Mock device modes (2026-07-26):** `fretwireMock.device("stomp"|"floor")` flips the browser mock
between a one-DSP/one-list HX Stomp and a two-DSP/eight-setlist Helix Floor, so the setlist picker's
presence and the dual-grid layout are both testable without hardware.

**CI was broken and had never run (2026-07-26).** The clippy step passed its `-p` flags *after* the
`--`, sending them to the clippy driver (`error: Unrecognized option: 'p'`) — and the workflow only
triggered on `master`/`main`, so with all Floor work on a feature branch it had never executed at
all. Fixed, plus: triggers on every branch, a `rustfmt` job, a **GUI clippy job** (plain
`cargo clippy` only checks `default-members`, which excludes `fretwire-tauri` — that blind spot is
how a live warning survived), and the global `RUSTFLAGS: -D warnings` dropped in favour of passing
`-- -D warnings` to clippy, so third-party dep warnings can't fail the build. `.githooks/pre-push`
runs the same gates locally (`git config core.hooksPath .githooks`).

**The post-edit re-pull: revisited 2026-07-26, staying as-is.** Audited the read counts rather than
guessing. The wrapper split is already correct — `mutate_edit` wraps only session methods that do
*not* read internally (`set_param`, `set_paired_param`, `set_param_enum`, `set_bypass`,
`rename_snapshot`: 0 internal reads each), and `returning_edit` wraps the ones that do, so nothing
double-reads. That leaves **one** full stream read per param commit, and two per structural edit —
one to plan against current state, one to pick up the device's recomputed routing. Both are
inherent, and the drag path (`preview_param`) already skips the commit read entirely.

Removing the param-commit read would mean patching the param into `last_raw` locally (there is no
`PresetStream` param setter yet) and would put undo/redo history — which snapshots that blob — at
risk for a latency win the user mostly can't feel, since dragging never hits it. Not worth it; this
is now a settled decision, not a deferral.

## Prioritized next steps
> **The path to live control is in `docs/next-steps.md`.** TL;DR: (1) **on Windows now** — capture a
> dozen single-knob edits, decode with `fretwire decode-edit`, find out if param keys generalize (the
> pivotal experiment); (2) **on Linux with the pedal** — write the `fretwire-usb` transport, replay the
> handshake, read a preset, toggle a bypass (proof of life). The items below are the open decode
> threads feeding that work.

1. **Model id correlation (RESOLVED).** A block's model is **`24 → 25`, an index into `Helix.sym`**
   (833 symbols) → exact device symbol (with Mono/Stereo) → strip suffix → `symbolicID` →
   name/category via `ModelDefs::id_by_symbolic_id`. Amp+cab pair via `24 → 26`. Verified by a
   hand-built preset (591 = US Princess amp) and the factory capture. **Correction:** the Windows
   session had called `24 → 25` a non-monotonic runtime handle — it only looked non-indexing because
   it was tested against `HelixModelDefs.bin`; it indexes `Helix.sym`. (A name + param-count
   tie-break covers 150/164 name collisions where no index is available — analysis in
   `tools/analyze-name-collisions.js`; the index makes it unnecessary in code.) Category decode
   (`11→6`) is now a UI nicety only, not an identity blocker — and is
   *computed*, not a binary table (the `1037` hit was the Windows LCID table, a
   coincidence). See `docs/preset-format.md`.
2. **Edit protocol (RESOLVED — it's MessagePack, and editing is computable).** Body =
   `{102: u16 counter, 100: op (41 bypass / 30 set), 101: {98: slot, …, 28: param_index, 119: value}}`.
   **★ The parameter is selected by its index (key 28) in the model's `Helix.sym` device order** —
   verified across 4 models / 6 params (`captures/param_map_findings.md`), so param editing needs no
   per-param captures. `edit::set_value()` + `EditorBlock::set_param_by_name()` build byte-exact
   commands. Remaining: switch/transport params (`@trails`, tempo-sync) use key 28 = 0 (different
   addressing, TBD). See `docs/protocol.md`.
3. **Live bring-up on Linux — READ path DONE (2026-06-23).** `detect`, `connect`, and `pull` work
   on real hardware. First-contact findings, all resolved:
   - **udev:** raw-USB access needs a rule (`70-hxstomp.rules`, `ATTR{idVendor}=="0e41"` — lower-case
     `i`); the default systemd `uaccess` only covers `/dev/snd/*`, not the vendor interface.
   - **Response matching:** match a reply by **channel (`dst==our src`) + seq**; the status channel
     (`f003`) streams unsolicited meters that must be skipped. A single bulk-IN can batch frames.
   - **`arg` (offset 12) RESOLVED:** a per-channel running counter = **sum of received body lengths**
     on that channel. Edit-channel base after the handshake = `0x1009`; the paged stream advances it
     `+256`/chunk. (`Session` tracks it in an `arg: HashMap`.)
   - **Non-destructive read RESOLVED:** **not** op 20. The connect-time read (decoded from
     `startup.pcapng`) is, on the edit channel: `cmd 0x04 op 76 {}` (open) → `cmd 0x0c op 24 {118:128}`
     → `cmd 0x0c op 23 nil` (reply = preset identity/name) → `cmd 0x0c op 22 nil` (stream-start, reply
     = chunk #0) → `cmd 0x08` ×N pagination until a short read. `edit::{read_open,read_prep,read_info,
     stream_start}` build these byte-exact.
   - **Tooling:** no `tshark` on the Linux box — `tools/pcap-frames.py` parses the USBPcap `.pcapng`
     captures into decoded control frames.
   - **Write path DONE:** edits ride `cmd 0x04` on the edit channel (`{102:txn,100:op,101:{…}}`),
     the device ACKs `{102,103:0,104:nil}`, then a best-effort `cmd 0x08` follow-up. **op 41 key 59 =
     ENABLED (true = on/enabled)**, i.e. the wire flag is the inverse of "bypassed". The CLI
     already inverts it, so `bypass <slot> on` **bypasses** the block (pedal semantics) — an
     earlier note here claimed the naming was backwards; it isn't. `set` verified live (Tremolo Mix 1.0→0.5→1.0 via read-back).
   - **Robustness DONE:** bulk reads are bounded by a cancellable 3 s timeout (race vs
     `futures-timer`, dropping the transfer cancels the URB — no leak). `connect()` drains stale
     wire frames and **retries** on a failed handshake (a prior session leaves the device unable to
     reply until the interface is released/reset — drop-and-reopen fixes it). Rapid back-to-back
     `pull`/edit cycles are now reliable (was: alternating timeouts).
   - **Bypass display FIXED:** block enabled/bypass is **content key `20→10`** (`enabled` bool, true =
     on; `bypassed = !enabled`) — found by `diff-stream` of enabled-vs-disabled captures; the old
     `24→23` was wrong. Verified live: `bypass 4 on` → `[bypassed]`, `off` → clear.
   - **CLI semantics FIXED:** `bypass <slot> on` now *engages* bypass (block off), matching pedal
     mental-model; the raw layer is `Session::set_enabled(slot, enabled)`.
   - **New tooling:** `fretwire dump-raw <file>` (save a live stream) + `fretwire diff-stream <a> <b>` (find which
     msgpack key changed between two device states — the RE workhorse for the next features).
   - **Preset browsing + navigation DONE:** `list_presets()` reads all 126 names (browse stream:
     op 254 → op 0 → op 1 → paginate, TLV opcode `0x0002`, key 109 = name). HX Edit runs this on the
     primary channel, but our reconstructed `device_handshake` doesn't leave primary browse-ready
     (it diverges — extra `cmd 0x08`/`0x02`, missing the second `op 0x0002` query); the **edit
     channel** serves the same resource and works, so `list_presets` uses it. `goto_preset()` = op 20
     SELECT on the edit channel (changes the active preset). Both verified live.
   **Next features:** snapshots, controllers, model swap, save-to-device (back up first), GUI.
   - **Bugfix (2026-06-25): a controller assigned to a block's param no longer hides the block.** A
     controller node (`11 → 0 == 2`) in the footswitch layout points at its target block's slot; the
     enrichment was matching it by slot and reclassifying the real kind-6 DSP block as a controller —
     so a parallel-path amp with a controller assigned vanished from `pull` and the DSP total. Fix:
     only DSP-type nodes (`!= 2`) enrich blocks; controllers stay in `assignments`. Regression test +
     verified live (two-amp split now shows both amps; DSP 40%→73.5%).
   - **Controller assignments DECODED + displayed:** the `[unresolved]` "OD Sw" was a
     **footswitch/controller node** (path node type `11 → 0 == 2`), not a DSP block. The assignment
     table (key `4`, Array[10]) is parsed (`PresetStream::assignments` → `EditorPreset.assignments`)
     and `pull` shows e.g. `controller 7 -> slot 15 param 0`. Source/type flags still raw.
   - **Snapshots (names + active) parsed:** key `10` (`8` = active index, `10` = snapshot maps,
     each `4` = name); `pull` shows e.g. `snapshots (active: 0): CLEAN, Just TS, LEAD`. The per-block
     snapshot value matrix (key `2` / key `10` sub-arrays) is still TODO. See `docs/preset-format.md`.
   - **Snapshot *switching* DONE + verified live (op 88):** `Session::set_snapshot(index)` /
     `fretwire snapshot <index>` — body `{102:txn, 100:88, 101:{92:index}}`, byte-exact vs the capture.
     0-based (snapshots 0–2 on the Stomp), matching `active_snapshot`/`snapshot_names` order.
   - **Save-to-device DONE + verified live (op 71):** `Session::save_preset(bank, slot, name)` /
     `fretwire save <slot> <name> [bank]` — body `{107:bank, 108:slot, 109:name\0}` (name NUL-terminated),
     byte-exact vs `launch_hx_*_savepreset_*.pcapng`. Saves the current edit buffer to a slot;
     **verified persisted across a power cycle** (`bank 0 + flat slot index` addresses correctly,
     same basis as `goto`). ⚠️ persistent flash write — overwrites the slot.
   - **Rename-snapshot BUILT (op 89):** `Session::rename_snapshot(index, name)` / `fretwire rename-snapshot
     <index> <name>` — `{92:index, 109:name\0}`, byte-exact vs the capture. (Awaiting a live confirm.)
   - **Global/input settings BUILT as a probe (op 25):** `Session::set_setting(id, value)` / `fretwire
     setting <id> <value>` — `{118:id, 119:value}`, not block-addressed, byte-exact vs
     `switch_input_gate_and_guitar_pad.pcapng`. **The id space is only partly mapped** — only id 134
     (a 3-state input setting, 0/1/2) is known; this is the wire primitive for mapping the rest live.
   - **Model swap — BUILT (op 40).** `Session::swap_model(slot, model_index, paired_index)` /
     `fretwire swap <slot> <model-index> [paired-index]`. Body `{98:slot, 100:{23:false, 25:index, 26:paired}}`
     — the inner `{23,25,26}` is the same model-ref shape as the preset's key `24` (25 = `Helix.sym`
     index, 26 = paired cab/IR, -1 = none). Byte-exact vs `model_swap_delay_then_reverb.pcapng` (two
     swaps: index 79 and 607). The device resets the block's params to the new model's defaults —
     confirmed by an on-device diff (slot 6 Simple Delay→Mod/Chorus Echo: only `24→25` + `11→2/3` +
     the `11→4` default array changed; identity = `24→25` confirmed a third time). **Verified live
     (2026-06-25):** `fretwire swap 6 79` / `6 80` / `6 607` all swap slot 6's model on the pedal, params
     reset to the new model's defaults. *Nicety TODO:* swap by model **name** (resolve name →
     `Helix.sym` index) instead of the raw index. **DSP-fit warning:** `fretwire swap` now reads the preset,
     projects the new load (`Catalog::model_load_by_index`) and **warns if it exceeds `DSP_BUDGET`
     (~100%, unconfirmed)** — a warning, not a hard block (device is the arbiter). Over-budget device
     behavior still untested. **Availability is moot on the Stomp** (it runs the full HX model set;
     grey-out = DSP-fit). The handshake identity reply has `"P33Main"` + 12 trailing bytes
     (serial/HW/firmware, undecoded) but **no `0x0021xxxx` device id**, so the `.models` `devices` ids
     (cross-product/firmware gating) can't be matched and aren't needed. Only open DSP item: confirm
     the budget value vs HX Edit's meter.
   - **DSP meter — per-model cost + live usage (2026-06-25).** The `.models` files carry **`load`**
     (mono) / **`load_stereo`** (% of DSP budget). `Catalog` bundles them → each `EditorBlock.dsp_load`
     (incl. paired cab, picked by Mono/Stereo variant) and `EditorPreset.dsp_load` (sum); `pull` shows
     **"DSP X% used (Y% free)"** + per-block %. Factory preset = 71.8% (amp 27.5%, stereo reverbs use
     `load_stereo`). Budget **likely 100%/DSP** — still to confirm the exact Stomp cap and validate our
     % against HX Edit's meter. Model **availability** (`.models` `devices` field) not yet used for
     grey-out. See ROADMAP Phase 5.
   - **Split-topology decode — RESOLVED (2026-06-24).** The fix was to stop trusting the signal
     path entirely: **enumerate blocks from the slot array `0 → 22`** (kind 6), identify each by
     its `24 → 25` `Helix.sym` index (no name strings needed). This decodes serial *and* split
     presets, and even recovered 2 off-path blocks the factory capture had been hiding (4 → 6
     blocks). Topology: `0 → 21` = split flag; row from slot index (0–15 main, 16+ row B); kind-2
     split + kind-3 mixer nodes. Proven by a controlled serial↔split diff on hand-built presets
     S/P. `pull`/`show-preset` now render full blocks + paired cab + row + topology for both.
     *Remaining nicety:* reconstruct exact signal flow through the split (mixer routing at node
     key `13`), and confirm the slot-16 row boundary on a >2-block split.
   Loose end: make `device_handshake` primary bring-up faithful (would also enable primary-channel
   browse).

## Repo map
`crates/` (fretwire-data, fretwire-protocol, fretwire-usb, fretwire-core, fretwire-cli,
fretwire-tauri) · `docs/` (protocol, preset-format, safety, next-steps) · `captures/` (pcaps + notes
+ reassembled blob) · `tools/` · `ROADMAP.md`.
