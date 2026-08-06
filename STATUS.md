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

**FIRST HARDWARE RUN on a real Helix Floor (2026-07-24, a contributor):** fretwire ran
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

**Second hardware run (2026-07-26).** The reassembly fix holds: every read in the tester's logs now
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
   inside `split_pos..mixer_pos` and no column leaves the grid. The `bmblfoot.bin` dump the tester
   sent (2026-07-26) did **not** close it — it parses fine but it is the restored original (6
   blocks, serial, one DSP, no Y-split at all); the multi-row-B version he had been looking at was
   his own edit, reverted by the restore. **Closed 2026-07-29 by `pullmeunder.bin`** — see below.
2. **Setlists implemented.** The Floor has eight (Factory 1/2, User 1-5, Templates); we only ever
   browsed bank 0, so a unit in User 1 listed Factory 1's names — the tester's "the list on the left
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

## INCIDENT 2026-07-26: contributor's Helix Floor locked up, presets lost

**What happened.** The tester opened the **TEMPLATES** setlist in the new build, picked a preset,
and the Helix locked up hard enough to need a reboot. He then ran a footswitch factory reset
(7+8) to recover, after which FACTORY 1 held an older preset set, all USER setlists were empty, and
the unit showed "003 new preset" — persisting across reboots. He has Thursday's `.hxb` backup and is
restoring from it; the backup file parses cleanly here (363 presets across 8 setlists).

**The lockup is ours.** The preset-list browse numbers presets **globally**
(`bank * setlist_size + slot`) while `goto_preset`/`save_preset`/`rename_preset` take the
**bank-relative** slot, and the GUI passed the browse number straight through. Selecting a template
sent `goto_preset(bank = 7, preset = 906)` — 906 = 7x128+10, far past the end of a 128-slot setlist.
Confirmed against his own backup: browse index 906 is exactly bank 7 slot 10, "Wet-Dry-Wet Amps".
**Bank 0 hid this completely**, since there global == relative, which is why every earlier test
passed.

**The preset loss is very probably not ours, but that is not the same as proven.** Audited every
flash-write path: `save_preset` (op 71), `rename_preset` (op 6) and `restore_preset` are reachable
only from explicit button presses; `write_preset` (op 21) targets the edit buffer, not flash.
Browsing and navigating never write, and no path we have can empty a setlist. The observed pattern
(factory list rolled back + user setlists cleared + survives reboot) is what a device-level factory
restore does. But we did wedge the device, and a hard lockup is not a risk-free state to leave
firmware in — so "we sent no write command" is the strongest claim supportable.

**Fixed.** `Device::setlist_size` (128 on the Floor); `list_presets_in` normalises the browse's
global index to a slot; `Session::check_preset_addr` rejects an out-of-range bank or slot in
goto/save/rename **before it reaches the wire**.

**Withheld — narrowed 2026-07-29.** Cross-setlist *browsing* is now **on**: the numbering that
motivated the gate is settled (see the 2026-07-29 round below — 1024 slots verified against the
unit's own `.hxb`), and browsing writes nothing. What stays behind `FRETWIRE_SETLISTS=1` is the one
irreversible action: **a flash write into a setlist the device isn't in**
(`commands::cross_setlist_write_enabled`, enforced in `check_cross_setlist_write` against
`Session::last_identity()` — the device's own reported bank, not anything the frontend tracks).
Save As greys out with a reason while browsing elsewhere; plain Save always targets the preset's own
bank and is unaffected. Lift it once a Floor gets through a session without locking up.

**RESOLVED 2026-07-26 (evening) — the "index drift" was his device, not our parser.** The offset
against his `.hxb` (+1, then +9) was real but it was *content* drift: his FACTORY 1 had diverged from
the July-22 backup, which he spotted himself ("the Fact 1 list looks suspiciously like a 3.7-era
list"). Three independent checks after he restored from that backup:

- **read-info indices match the backup exactly** on all 20 presets appearing in his session log —
  3 Cali Rectifire, 15 Moo)))n Jump, 46 Stone Cold Loco, 67 BMBLFOOT PRINCE, 78 BILLY KASTODON, ...
- **The TEMPLATES browse listing matches exactly** — `TemplateWTF.png` lists 896 Quick Start → 906
  Wet-Dry-Wet Amps against the backup's bank 7 slots 0 → 10. Same code path, same session, **no**
  offset, which is what clears `parse_preset_list` and the `bank * 128` normalisation.
- The offset only ever appeared in bank 0, and only *before* the restore.

So the numbering is understood and the blocker is lifted. Cross-setlist browsing stays gated for now
regardless — the lockups below are the reason, and they happen in FACTORY 1 with no setlist switching
involved.

**Confirmed exhaustively 2026-07-29.** He sent `dump-list` output for all eight banks. All 1024
entries parse, every index decodes to its own bank under `global = bank × 128 + slot`, and against
the July-22 `.hxb` they agree on **1021 of 1024** — the three exceptions being presets he has since
*moved* (`InSTANtgH0St/24` 101→68, `Parallel Muffs` 108→107, `BAS:FunkIfIKnow` 84→95), each showing
up in a sequence diff as one insert + one delete with every other run equal. USER 1 (63/63), USER 2
(1/1) and TEMPLATES (43/43) match slot-for-slot. Nothing left open here.

**The header/highlight mismatch had a second, real cause** — see the identity lag below. That is
fixed independently.

## INCIDENT 2026-07-26 (evening): two more lockups, no setlist involved

Reading only — 66 preset reads, 14 browse listings, **zero writes** in the whole session — the Floor
stopped responding twice, in FACTORY 1, on ordinary preset changes. Nothing we sent could alter
stored presets, which further supports the preset loss above not being ours.

**Read amplification (fixed).** The GUI refreshed on *every* push batch. One preset change emits a
flurry of pushes over ~1 s and the heartbeat delivers them in 250 ms batches, so a single knob turn
cost **3.1 full preset streams plus a preset-list re-read** — ~530 KB across 21 preset changes —
fired at a unit still reconfiguring both DSPs. Both lockups show the same shape: a read completes,
then ~3 s later **`read-open` itself** fails, i.e. the device had already gone quiet before we
noticed; the old code then retried immediately, adding load. Fixed by coalescing pushes (300 ms
quiet, 1.2 s cap), re-listing the browse only when the *bank* changes, applying bypass pushes in
place with no read at all, and backing off (150/400/800 ms) before a retry.

**Not proven.** Read pressure is a hypothesis for the lockups, not a demonstrated cause — the two
Wireshark captures he sent are empty (both zips contain a single empty directory). A real pcap of a
freeze is the outstanding ask; it reproduces for him.

**Identity lags the blob by one preset (fixed).** The first read after a preset change serves the new
preset's stream under the *previous* preset's identity — 19 of 21 distinct stream lengths in his log
are reported under exactly two consecutive identities. Since the live snapshot rides the same reply
(key 92), this also painted the previous preset's snapshot. `read_preset_inner` now re-issues op 23
after the stream and reports whether the identity moved; `read_preset` re-reads when it did. See
`docs/protocol.md`.

**Stream reassembly (hardened).** A spurious mid-stream chunk was appended to the payload *before*
being classified as spurious — safe only because every observed case was zero-length. The decision is
now `session::classify_chunk`, unit-tested: empty replies before the declared end are dropped, short
*non-empty* chunks are kept and do not terminate the read.

## Third round (2026-07-29): eight bank listings + `pullmeunder.bin`

The tester answered both outstanding asks in one email. Full write-up in `docs/helix-floor.md`;
the short version:

- **Preset numbering: closed.** 1024 entries across all eight banks, 1021 matching the `.hxb`
  exactly, the three exceptions being presets he moved. See the RESOLVED block above.
- **Row-B rendering: closed.** `pullmeunder` (FACTORY 1 slot 45) is the multi-row-B preset we asked
  for — six blocks on DSP2's row B (wire 33..=38 → columns 3..=8) plus two on DSP1's (11/12 →
  columns 1/2). The `slot − 10` column mapping holds per DSP and everything stays inside the
  8-column grid.
- **Bug found and fixed: the browse listing is not sorted.** A moved preset keeps its old *stream
  position* while carrying its new *index*, so the sidebar drew three presets in the wrong row under
  correct numbers. `Session::list_presets_in` now sorts after normalising
  (`normalise_preset_list`, unit-tested) and logs `reordered=true` when the device's order differed.
- **Bug found and fixed: the snapshot bypass matrix is a flat 40-entry array** indexed by wire slot,
  not one 20-entry array per DSP. `show-preset`'s scene diagnosis was indexing by the per-DSP index,
  so every DSP2 block reported DSP1's state — all eight snapshots looked alike and none matched the
  live scene. Fixed; the snapshots now read as coherent scenes (Intro = clean delay/verb on, gain
  path off; Solo = the reverse) with exactly one match. Regression test in `multi_dsp.rs` uses a
  synthetic preset with different states at wire 13 and wire 33, which per-DSP indexing cannot pass.
- **Second independent confirmation of the key-92 fix.** `pullmeunder`'s blob stores active snapshot
  **4**; its live scene is unambiguously snapshot **0**. Same disagreement the `Dual Amp` capture
  showed, on a different preset and a different device state.
- **New, unresolved: a Y-split can span both DSPs.** `pullmeunder` splits after a common Volume+Comp
  on DSP1 and rejoins at the end of DSP2. Both DSPs report `is_split()`, and the bracket ends are
  split across them — DSP1 `split=2, mixer=0`, DSP2 `split=0, mixer=9` — so the
  "common-before / path A / common-after" rule and the `split_pos ≤ column < mixer_pos` invariant
  do not hold for such a preset. Tagged `[hypothesis]` off one sample. **Deliberately not acted
  on**: guessing ahead of data is what produced the original row-B bug. Needs a screenshot of this
  preset's grid in HX Edit, or a second cross-DSP preset.

**Gate split as a result.** Setlist browsing + `goto` now ship ungated; only cross-setlist flash
writes still need `FRETWIRE_SETLISTS=1`. See the Withheld note above.

**Still outstanding:** a real pcap of a Floor freeze (his two zips were empty), and one clean
read-only Floor session exercising the setlist picker — nothing on that path has run against
hardware since any of the fixes.

## Fourth round (2026-07-30): the lockups are ours, and they are fixed

Eleven screenshots, four `RUST_LOG` captures, a photo of the pedal's screen, and a saved preset
blob. Two of the four sessions ended with the Floor dropping off USB. Full write-up in
`docs/helix-floor.md`; the short version:

- **Root cause found: the device does not range-check parameter writes.** A `Heads 1-2` selector
  (integer enum, `min: 0, max: 3`) on a legacy DL4 delay was sent `77`. The write was ACKed, and
  the pedal then stopped answering and reset off the bus. This **falsifies a written assumption**
  in `editor.rs` that "the device clamps, so an off estimate only mis-scales the slider, not the
  sent value" — the comment has been corrected. Out-of-range integers are not survivable.
- **Three compounding causes, fixed at each layer.** (1) `param_meta_from` keyed its table by the
  `.models` `symbolicID` while blocks look up by the *variant-stripped* id, so the eight legacy DL4
  delays — the only models the data names solely in suffixed form — resolved to no metadata at all.
  (2) With no `max`, the editor invented a `0..=127` slider; on a `0..=3` param that is ~97 %
  illegal travel. (3) Nothing below the UI checked. Now: the meta table aliases the stripped base,
  unranged integers render read-only instead of guessed, and `Session::clamp_param` bounds every
  write. With (1) fixed the param is a four-option dropdown, so the crash value is unreachable
  rather than merely clamped. Regression test: `legacy_dl4_ranges_survive_the_variant_suffix`.
- **First direct check of our routing grid against the pedal's own display.** A photo of
  `15D RC REINCARNATION` on the Floor's screen agrees with our render on every structural point —
  eight blocks on path 1, one parallel block under the later chain, path 2 empty.
- **Listing order confirmed live.** `bank=0 … reordered=true`, `bank=2 … reordered=false` — exactly
  what the 2026-07-29 dump predicted. The sort in `list_presets_in` does real work on this hardware.
- **Logging gap closed.** `preset read/decode failed` now logs the error, not just the attempt
  number; without it a remote log can't separate a decode fault from a device that has stopped
  answering.
- **The cross-DSP split `[hypothesis]` is CLOSED, in our favour.** A photo of `Pull Me Under` on the
  pedal's own screen shows path 1's two rows each leaving into path 2, which then merges — so path 1
  genuinely has a split and no mixer, path 2 a mixer and no split, and the `0` really does mean
  "this DSP doesn't hold that end of the bracket". Our render matches block for block, including
  bypass states and row-B columns on both DSPs. Two topologies are now confirmed against hardware
  (bracket on one DSP: RC Reincarnation; bracket across both: Pull Me Under).
- **`Waters in Hell` — the last unexplained render — is CLOSED, also in our favour.** Its DSP2 draws
  a one-column split/mixer bracket at columns 1–2 containing nothing while that DSP's row-B blocks
  sit at columns 6–7 outside it, which looked impossible. The pedal photo shows the hardware doing
  exactly that: **a mixer can sit to the left of blocks on its own row B**, and the device papers
  over it with a wrap-around return line. Measured against the screen's own column pitch, every one
  of our nodes lands where the device puts it. So `split_pos ≤ column < mixer_pos` was never an
  invariant. **Every render the tester has sent is now confirmed correct; no routing-layout bug is
  open.** Table of all eleven renders in `docs/helix-floor.md`.
- **The freeze was partly ours: reads had no wall-clock bound — fixed.** The chunk loop capped
  request *count*, not time, so a still-enumerated but unresponsive pedal cost ~36 × the 3 s
  bulk-IN timeout per attempt, times three attempts. The measured gap inside one attempt was
  **121 s** — that is the "GUI froze" report. `READ_DEADLINE` (10 s; a healthy full read is ~20 ms)
  now bounds it, so a dead device errors in ~30 s instead of hanging for ~6 minutes.

The two earlier "contributor's Floor locked up" incidents were never explained. This round gives a
mechanism that fits them, but does not prove it — the first of this round's two lockups has the same
signature with no echoed edit body, so which write did it is unproven.

## Fifth round (2026-07-30, evening): the first clean Floor sessions

Two full GUI sessions, one before the fixes and one after. **Neither locked up, and both closed
cleanly.** ~9 minutes of connected time, zero `ERROR` lines, one `WARN` — the already-understood
benign `empty chunk before declared stream end`, which the skip logic absorbed. This is the first
time a Floor has held a session start to finish.

Neither log fired `clamp_param` or `READ_DEADLINE`: he did not touch a legacy DL4 delay and nothing
wedged. The lockup fixes are **unexercised, not confirmed**.

Two new bugs came out of the material, both fixed:

- **The op-23 identity can lag past the stream, not just up to it.** We already knew the identity
  lags the blob by one preset, and mitigated it by re-asking *after* the stream. The log shows that
  is not enough: an 8118-byte `Pull Me Under` stream was reported as `WATERS IN HELL #56` on the
  read before it **and** the one after, so `before == after` and the settled-check passed a blob
  labelled with the wrong preset. It self-corrected 370 ms later only because the GUI read again.
  The earlier session shows the fields lagging *independently* — `bank: 1, index: 3, name: "Cali
  Rectifire"`, where the name is FACTORY **1** slot 3 and only the bank was stale — so the reply
  can't be treated as atomically old-or-new. Since the active snapshot rides the same reply (key
  92), a stale identity also paints the previous preset's snapshot onto the new chain. **Fix:**
  `goto_preset` records the address it asked for and `read_preset` re-reads until the device agrees.
  Comparing two device answers can't see this; comparing against the address we chose can.
  Regression test: `identity_confirms`, built from both log cases.
- **The status line never refreshed.** `Connected — N blocks` is set once at connect and left alone,
  so it describes whatever preset was loaded then, forever. Both screenshot rounds caught it, each
  captioning a preset with the *other* preset's block count. `Saved to slot 7.` is the same bug and
  worse — it sat on screen over a preset in a different setlist. It now re-states itself whenever
  the loaded preset's identity moves, from whichever path moved it.

## Sixth round (2026-07-30, later): a preset built and saved on the Floor

Still the pre-fix build, so it tests none of the above. It tests **writing**, for the first time in
the field: 7 minutes, zero `ERROR`, browse out of FACTORY 1 into USER 2, build a preset in slot 0,
`save` — and the identity comes back `bank: 3, index: 0, name: "fretwireTest1"`. He mailed the blob
and our own decoder reads it cleanly (`Brit P75 Nrm` + `2x12 Silver Bell`, all params, snapshots
consistent), matching his screenshot. The **browse/write setlist split** is confirmed working: he
browsed and loaded across setlists and saved in place. Cross-setlist *writing* is still gated.

Two more bugs, both fixed:

- **We never checked whether the device accepted an edit.** Envelope key 103 is the reply's kind,
  not a don't-care: `0` = payload, `1` = ack, **`255` = refused**, with `104: {111: code}`. Two
  commands in this session came back `{103:255, 104:{111:-21}}` and the preset stream was
  byte-identical across both — nothing applied — while `send_edit` logged the reply at `DEBUG` and
  returned `Ok`, so the GUI reported success. Now `Error::Rejected`. The log also couldn't say
  *which* command was refused, so `send_edit` logs the op and txn it sent.
- **Adding an amp with its cab has never worked.** The refused command was `add_block` (op 39)
  carrying a `paired_index` — i.e. every pick from the synthetic **Amp+Cab** category, since each
  amp's linked cab sits at `Helix.sym` 687–829 and needs a `uint16`, which is exactly the 2 bytes
  separating the refused 56-byte frames from the 52-byte frames of the two adds that worked. The
  saved preset is the proof: amp and cab as two separate, *unpaired* blocks — the fallback after
  two refusals. **Fix:** add the amp bare, then op-40 the cab onto it, which is HX Edit's order and
  the byte-exact path the capture tests cover. **Confirmed working on hardware 2026-07-31.**

## Seventh round (2026-07-31): the freeze reproduced, and it is the whole-preset write

**The first reproducible lockup, with a log through it.** Build a parallel path, then drag the mixer
(the ⋈ join) to a column between two blocks: the pedal stops responding and needs a power cycle. He
hit it twice, either side of a reboot, on the same action.

Both freezes are a **whole-preset write (op 21) that the device stopped accepting mid-transfer**, and
the mechanism is ours:

- Each 496-byte data frame earns an empty `cmd 0x08` frame back. That is a **flow-control credit**,
  not decoration — a healthy write earns ≈1 per chunk and never runs more than one behind. We were
  treating them as noise: glance at the wire for 5 ms, discard whatever showed up, send the next
  chunk regardless.
- In both freezes the credits went to a flat zero — after chunk 1 in one, after chunk 7 in the other
  — and we pushed four more chunks (≈2 KB) into a device that had already stopped reading.
- Then we hung. **`Transport::send` had no timeout at all**: the last line of both logs is
  `Submitted URB … on ep 1` with no completion. Once the device stops draining its OUT endpoint an
  unbounded `bulk_out` blocks forever, so the editor wedged along with the pedal.

Fixes:

- `Transport::send` races a **2 s `WRITE_TIMEOUT`**, mirroring what `recv_timeout` already did for
  reads. A stalled endpoint is now `Error::Timeout`, not an unkillable hang.
- `write_preset` **waits for each chunk's credit** (250 ms) instead of glancing for 5 ms, tracks the
  running deficit, and aborts with `Error::WriteStalled` once it runs more than 2 chunks ahead —
  which on both field traces is the frame *before* the one we blocked on. It also drops the read
  cache, since the edit buffer is then half-written.

Honest limit: this stops fretwire hanging and stops it emptying the rest of a blob into a wedged
pedal. It does not stop the pedal freezing — see below.

## Eighth round (2026-07-31, same night): the fix holds, the freeze is not ours to pace away

He rebuilt on the fixed code and ran the same action. Two results.

**The abort works.** The mixer move produced exactly the intended failure — `device stopped
acknowledging mid-write — aborting the transfer sent=2480 total=6817 credits=2 chunks=5` — and the
app stayed alive instead of hanging on a URB that never completes. The healthy write in the same
session (the rotary drag) now runs at **deficit 0 for all 14 chunks**, where before the fix the same
write ran a chunk behind; waiting for credits measurably improved the good path too.

**The freeze is not a pacing race.** [solid] That was the hypothesis last round and it is wrong. With
the host waiting properly for every credit, the device still dies in the same place — credits stop
after chunk 2, at 2,480 of 6,817 bytes, the same signature as the pre-fix trace. Whatever kills it,
outrunning it is not the trigger. It is ~1 KB into the blob when it goes, so it is reacting to bytes
it has already consumed, not to the assembled preset. Still open. [hypothesis]

**One more bug, found in the aftermath and fixed.** Once the pedal was wedged, the GUI heartbeat kept
beating into it every 250 ms — each beat now burning the full 2 s write timeout, each holding the
session lock, so the whole UI was stuck behind it — for the ~50 s until he disconnected by hand. Then
`close()` spent 6 s more sending teardown frames to a device that was not listening. The heartbeat
now gives up after 3 consecutive failures (or immediately on a latched `Session::device_lost`), drops
the session, and emits `device-lost`; the frontend falls back to the disconnected view with
"power-cycle the HX device, then reconnect". `close()` skips the wire entirely when the device is
gone.

## Ninth round (2026-07-31): **a real bug — we were corrupting the preset's offset table**

> **Correction (thirteenth round):** this was written up as *the* root cause of the lockups. It is
> not. The offset table was genuinely corrupt and is genuinely fixed, but a mixer drag still wedges
> a pedal on the fixed build. See the thirteenth round.

He sent the `dump-raw` of `fretwireTest2`, and the bug was reproducible offline in minutes.

**The preset header is an offset table, not the "fixed header/uuid, meaning TBD" we had it down as.**
48 bytes = 12 LE `u32`s: slot 0 is the preset map's offset, slots 1–9 are the offsets of individual
top-level entries, and slots 10 and 11 are the blob's total length. The device seeks with it instead
of walking the MessagePack.

`to_blob()` re-encodes the map with rmpv, which writes integers **minimally**, and the device does
not — it emits `d1 00 00` for a zero all over the preset. So our re-serialization of an *untouched*
preset comes out **117–216 bytes shorter**, with everything after the first such integer shifted
left. And we copied the header across verbatim. On `fretwireTest2`:

| header slot | points at, in the device's blob | …applied to ours |
|---|---|---|
| 5 (949) | `02 82` — key 2, a map | `02 82` ✓ (before the first shift) |
| 6 (1391) | `05 de` — key 5, a map | `cd 01 13` — **mid-value** |
| 7 (1814) | `06 82` — key 6, a map | `00 00 00` — **mid-value** |
| 10/11 (7004) | the blob's length | **216 bytes past the end of our 6788-byte blob** |

That is what killed the pedal: it was told its data was 7004 bytes with sections at those offsets,
handed 6788 shifted bytes, and followed a pointer off the end. It stopped draining its endpoint
~1 KB in, which is exactly where the credits stopped in all three field traces.

**This was never a Floor bug, and never about the mixer.** Every one of the four presets we have —
including the two HX Stomp captures — round-trips wrong, by 117–216 bytes. Any op-21 write we have
ever sent carried a corrupt offset table. The mixer drag was just the first edit with no surgical op
behind it, so it was the first to *need* op 21.

**Fix:** `to_blob()` rebuilds the table against the bytes it actually emits. Byte-identity with the
device's encoding isn't reachable through rmpv and isn't needed; self-consistency is. Verified on all
four captures, before and after the mutation that froze the unit — total-length slot correct, every
interior offset landing exactly on a top-level entry, nothing past the end.

The test that was supposed to catch this asserted the header survived **unchanged** — i.e. it
asserted the bug — and its byte-identity check was a `eprintln!` with the comment "(Not required —
the device parses msgpack.)" It now checks the invariant that matters.

**Still needs hardware.** The offline evidence is strong but nobody has yet moved a mixer on a real
pedal with this build.

## Tenth round (2026-07-31): `fretwire12.log` — still pre-fix, and the parallel path is silent

The log ends 30 seconds before `813cae3` was committed, so it is the old build.

**Three op-21 writes completed on it** — 6809 bytes each, deficit 0, no freeze. So a corrupt offset
table is not *always* fatal; it depends where the bad offsets land. Every trace so far splits by
size: 6809-byte writes complete, 6817-byte writes freeze. The 8 bytes are the mixer-move mutation.
Correlation, not mechanism.

**New symptom, open:** a block on the parallel row makes no sound. Mixer enabled, levels sane,
reproduced with two different models. The same drag worked in `fretwire8.log`, so it is a regression
within the session rather than something that never worked.

Leading hypothesis: the three whole-preset writes earlier in that session damaged the stored preset.
On this preset the stale offsets addressed keys **5, 6 and 10** — settings, focused block, and the
snapshots, whose per-snapshot `3` field is per-slot state for every block. Garbage there yields a
correct-looking chain with wrong per-block state. He saved afterwards, so it is in flash.
[hypothesis] The alternative — our row-B routing being wrong — isn't excluded, but has to explain
`fretwire8.log` working.

**Next:** on the fixed build, rebuild the preset from scratch in a fresh slot rather than trying to
repair `fretwireTest2`. Also decoded this round: preset keys 5, 6 and 10, previously "TBD" — see
`docs/preset-format.md`.

## Eleventh round (2026-07-31): offset fix is in; the parallel path is still silent

First session on `813cae3` (`fretwire14.log`). Fresh preset built from scratch, **no op-21 writes at
all**, zero errors, clean close — and a block moved to the parallel row still makes no sound.

**The corruption hypothesis is withdrawn.** No whole-preset write happened in that session, so
nothing we sent carried an offset table; the row moves are surgical and the device performs them
itself. This is its own bug.

**What a parallel path requires** (diffing the split captures against the serial ones — new, and
worth having regardless): the split and mixer nodes are **always present**, at slot indices 10 and
19, even on a serial preset, already carrying the Y-split model and the mixer model. Exactly five
fields differ: DSP group key `21`, and `18` (enabled) + `13` (column) on each of the two nodes. So
going parallel means **enabling two nodes that already exist and giving them columns** — and none of
those five fields has a known surgical op.

`move_block_to_row` sends op 43 and nothing else, resting on a doc comment that claims "the device
activates/retires the split as needed". That claim has never been checked against a dump, and it is
now the prime suspect.

**This cannot be settled from a log** — logs carry ops and sizes, not preset contents. One `dump-raw`
taken straight after a drag answers all five fields at once. Asked for; nothing further to do until
it lands.

## Twelfth round (2026-07-31): the dump clears our code

`fretwireTest3.bin`, dumped straight after a drag to the parallel row. **The device sets all five
topology fields itself** on the op-43 move: `key21=1`, both nodes `18: true`, columns 2 and 9, block
at slot 12 — structurally identical to the device-authored `split_preset` fixture. `move_block_to_row`
sending op 43 alone is correct and the doc comment I had flagged as the prime suspect was right.

The node parameters are sane too: Split Y at `Balance A/B = 0.5/0.5` (both `.models` defaults), mixer
at `A Level 0 dB`, `B Level 0 dB`, master `Level +3 dB`. Nothing mutes path B; the one non-default is
*louder*. Also decoded on the way: the split/mixer param layouts, and that `Enabled` is content key
`18` rather than an entry in the param array — see `docs/preset-format.md`.

**So the preset we produce is correct and this is not a data bug we can see.** What the chain actually
is, though: path A is `amp → cab → reverb` and path B is a **bare distortion with no cab**, summed at
equal level. A cab is a steep low-pass, so path B is thin fizz under a cab'd amp — a plausible
non-routing reason for "no rotary goodness" / "no delay sounds".

**Next, and it needs no dump:** mute path A (`A Level` → −60 dB) and listen. Audible ⇒ routing works
and it is a mix/placement issue (put the cab before the split, or raise `B Level`). Still silent ⇒ the
routing really is dead and the cause is outside the preset data, which would be a new lead.

## Thirteenth round (2026-07-31): the freeze survives the offset fix

Reproduced by the maintainer on an **HX Stomp**, on the fixed build, dragging the mixer to sit just
after the amp on the A path. Pedal wedged; handshake failed until it was power-cycled.

- **The offset table was not the cause.** Real bug, really fixed, wrong verdict — corrected in the
  ninth round above. What the fix bought is containment: the write now aborts instead of hanging the
  editor.
- **Not Floor-specific.** Same failure on a Stomp. Every lockup on record is an op-21 whole-preset
  write; nothing else has ever done it.
- **Next hypothesis:** our re-encoding. `to_blob` rebuilds the map with rmpv (minimal integers) while
  the device writes `d1 00 00` freely, so our blob is 117–216 bytes shorter for the same tree. We
  assumed the device just parses MessagePack; it demonstrably does more than that. [hypothesis]
- **The experiment:** `fretwire write-roundtrip` — read a preset and write it straight back unchanged.
  No mutation, no geometry. If it wedges, op-21 is unsafe for any preset and the fix is to splice the
  changed value into the device's own bytes rather than re-encode. If it survives, the mixer
  *position* is the problem and `set_node_pos`'s guards are the place to look.
- `FRETWIRE_DUMP_WRITES=<dir>` now saves the exact blob before it is sent, so whichever way it goes
  the bytes are on disk.

## Fourteenth round (2026-07-31): `write-roundtrip` survives — the encoding is exonerated

On a Stomp, **serial** preset: the device accepted a 2167-byte blob where it had sent 2303 (our
minimal integer encoding, 136 bytes shorter) and re-served the preset intact. No freeze. So the
thirteenth round's hypothesis is wrong — re-encoding is not what wedges the pedal, and splicing into
the device's own bytes is not the fix.

**The offset-table fix is now verified on hardware.** The dumped blob's table: slot 0 = 61, slots
10/11 = 2167 = exactly the blob length, every interior offset landing on a `<key><value>` boundary,
nothing past the end.

Two limits, stated plainly: the preset was **serial** and every lockup has been on a **split** preset,
so this does not cover the failing case; and a no-op write cannot prove the device *applied* it
(`acked=false`), only that the transport survived it — which is the half that matters for a wedge.

**Next:** the same probe on a **split** preset, no mutation. Freeze ⇒ op-21 is unsafe on split
presets generally, and the split/mixer node structures are the suspects. Survive ⇒ it is specifically
the mixer *position value*, and `set_node_pos`'s guards are where to look.

## Fifteenth round (2026-07-31): eleven op-21 writes on hardware, no freeze

Driven straight against a Stomp on the guard-fixed build, using a new `fretwire node-pos
<split|mixer> <column>` command that makes the same `set_node_pos` call the GUI's mixer drag does.
`write-roundtrip` on a serial preset, a serial→split move, `write-roundtrip` on the split preset, a
mixer move past the only A block, the same move landing *between* two A blocks (the exact failing
shape), and finally **8 `set_node_pos` calls back to back in one held session** — eleven whole-preset
writes, all survived, all verified applied by reading the column back.

**The last freeze was the guard's fault.** It logged `sent=2688 total=2688 credits=3 chunks=6` —
every byte already sent — and the old cumulative-deficit rule aborted anyway, skipping the
terminating `cmd 0x08` and leaving the device holding a transfer it was never told had ended. Fixed
in `0732954`, four minutes after that log.

So there were two failure modes: the pre-guard one (device genuinely stopped crediting, unbounded
`bulk_out` hung the host — still unexplained) and the guard misfiring (fixed).

**Limit:** every probe was a 2.2–2.4 KB Stomp preset, 5 chunks. The Floor freezes were 6.8 KB and 14
chunks. This does not close the first mode — a Floor retest on the current build would.

## Sixteenth round (2026-08-01): **we were reading the wrong frame as every edit's ACK**

Six more logs (`fretwire16`–`21`) and two `.bin`s. The headline is not in the freezes — it is that
our edit ACKs were largely not ACKs at all.

**The bug.** `send_edit` took "the next non-keepalive frame on the edit channel" as the reply. The
device also puts empty `cmd 0x08` credit frames on that channel, and leftover chunks of a finished
browse stream. Across the field logs, of 353 edit ACKs only **233 actually echoed the transaction we
sent**: 86 had an empty body, 50 echoed an *earlier* txn (lag 1–5), and one op-20 reply carried
preset-*list* bytes. Per op the structural path was worst — **op 43 `move_block` never once
correlated** (0 of 21), op 78 `begin_structural` 6 of 24, and op 71 `save_preset` 0 of 21. Two
consequences, both bad: an edit is reported applied on the strength of a frame that says nothing
about it, and once shifted, a refusal lands on the wrong command — where the `sent_txn == txn` guard
then *suppresses* it. `fretwire19.log` shows the end state: a select whose reply was list bytes,
then every request timing out while the pedal stayed perfectly healthy. That is the tester's "the UI
pretends to let you do something, but nah — and strangely, not the unit".

`send_edit` now correlates by the txn at key 102 (`edit_request_txn`, which already existed and was
used only for rename). Verified on a Stomp: 46 consecutive op-40 swaps plus ops 30/39/41/43/71 each
matched their own transaction. **Save has a real ACK** — `{102: txn, 103: 0, 104: nil}` — we were
just reading one frame too early and calling a credit frame the confirmation.

Whether this also explains the op-21 freezes is **[hypothesis]**: a structural drag is
`op 78 → op 43 → op 21`, and 78/43 are precisely the ops we almost never correlated, so the 14-chunk
write may have started against a device that had not acknowledged the structural edit. A Floor
retest would settle it.

**Two catalog bugs, both reported from the field and both reproduced offline.**

- *Every Cab (Mic+IR) listed twice, and the second copy would not load.* The 46
  `HD2_CabMicIr_*WithPan` symbols are HX Edit's **`Cab › Dual`** subcategory — a two-cab block with
  per-cab pan, not an alternative single cab. They share the plain cab's display name, so the picker
  showed 92 rows for 46 cabs, and the pedal refuses an in-place swap to one — **`-306`**, and `-21`
  in some states (both reproduced on a Stomp; the tester's Floor log shows `-21`). Selecting a duplicate did nothing and the block snapped back.
  They are now kept out of the swap list (name resolution is untouched, so a preset that already
  contains one still reads back correctly). 46 rows, and all 46 verified accepted on hardware.
- *A "Synth" category holding one model, and the 3 Osc Synth missing from Pitch/Synth.*
  `HD2_SynthSubtractive` is the only model the shipped `.models` put in category 5; HX Edit files it
  under **Pitch/Synth › Stereo**. Category 5 now folds into 7. Swapping a block to it works on
  hardware, so the reported "loading a synth locks up the UI" is consistent with the ACK desync
  above rather than anything synth-specific.

**Also confirmed, not fixed:** `HD2_CabMicIr_2x12MatchG25` displays as "2x12 Match H30" because
`HelixModelDefs.bin` — Line 6's own file — gives both cabs that `name`. `HX_ModelCatalog.json` has
it right ("2x12 Match G25"), so the fix is to prefer catalog names; left for later since it is one
cosmetic label and wiring in a new name source is its own change.

**A second real bug: `read_preset_raw` ignored the provenance check.** `read_preset` retries when the
preset identity moves across the stream read (`settled == false`) and, as a last resort, decodes the
blob anyway — fine, because the worst case is showing the user a stale view. But `read_preset_raw`
just called `read_preset_inner()` and threw the flag away, and *it* is the input to every op-21
read-modify-write (`set_node_pos`, `delete_block`, `reorder_block`, `move_block_to_row`,
`insert_block`). So a structural edit made while the device was mid preset-change would read a blob
belonging to neither preset, mutate it, and write it back over whatever the pedal is on now. 21
"provenance is ambiguous" warnings across these logs, and the tester's resaved `fretwireTest3` came
back **with no blocks at all** — which matches his note that the preset contents vanished and he
saved over them. It now retries and fails rather than guessing. (`backup_setlist` already had an
equivalent identity guard, so backups were never exposed to this.)

**Log noise.** `nusb`'s per-URB debug is **94% of a bug-report log by volume** (7.2 MB of one 7.7 MB
session) and buries the protocol lines. Both binaries now damp `nusb` to `warn` unless `RUST_LOG`
names it explicitly — measured on hardware: the same `pull` goes from 407 KB to 8 KB, and
`RUST_LOG=debug,nusb=debug` still gets the URBs back. Two read-path warnings were also downgraded to
debug after checking they are benign: the empty chunk is the device's `cmd 0x08` credit landing
between stream chunks, and the "short chunk" is a 256-byte chunk arriving as two frames — the halves
always sum to 256 (207+49, 46+210, 12+244, 251+5) and every read still reassembled to its declared
length.

**Also checked, no bug:** `spacedirt.bin` decodes correctly including its DSP2 blocks (slots 24–26 =
DSP2 indices 4–6, the documented `dsp × 20 + index` framing), and it contains a real
`HD2_CabMicIr_1x15AmpegB15WithPan` — confirming that dropping the dual cabs from the *picker* while
leaving name resolution alone was the right split.

**Still open from these logs:** `fretwire17.log` froze genuinely mid-write — `sent=2480 total=6911`,
credits flat from chunk 3 of 14 — but it is a `deficit=` log line, i.e. the pre-`0732954` build. The
tester needs the current build before that datapoint means anything.

## Seventeenth round (2026-08-01, late): **a preset size that made a preset unreadable**, and `arg` is dead

Six more logs (`fretwire30`–`35`) plus the chat. Two results.

**1. A real decode bug, and it was never flaky — just that size.** Three consecutive reads of one
preset reassembled 6794/6794 bytes and all three failed with `envelope key 104 missing or not
bytes`; the tester also saw its preset-list spelling (`... is not an array`) at launch. The stream
is `marker:u16, type:u16, len:u32 (LE)` then the MessagePack envelope, and `locate_root` picked the
root by scanning for whichever value consumed the most input. The length is little-endian, so **its
low byte sits directly in front of the real root** — and when that byte is itself a container marker
whose element count is satisfied by the three remaining length bytes plus one more value, the
decoder eats the whole envelope as that container's last element. It ends where the real root ends
and starts four bytes earlier, so it consumes *more* and wins, giving `{26: 0, 0: <envelope>}` with
no key 104. Exactly two lengths in 256 do it: low byte `0x82` (fixmap 2) and `0x94` (fixarray 4).
6794 declares 6786 = `0x1A82`. Callers now scan with `locate_root_where`, which only accepts a
candidate carrying the key that caller actually needs; a test walks all 256 low bytes. [solid]

**2. The `arg` lead is refuted.** Eight more Floor writes, all with `FRETWIRE_WRITE_ARG` pinning the
field — three completed, five wedged anyway, and one wedged at cursor 29397 while a completed one
started at 50635. Round 19's nine-for-nine split was session age wearing the cursor as a costume.
The probe is removed.

What replaced it is sharper, and it holds across **all 21** recorded Floor writes (`fretwire12`
onward, 13 wedged): **the credit count separates them without exception.** Every wedged write got 2
or 3 credits and not one more — chunk 3 is never credited in any of them — while every completed
write was credited at each chunk, climbing to 14–19 with `silent` never reaching 1. The device is
not being outrun and does not degrade over the transfer; it stops dead after two or three chunks,
and chunks four and five are us pushing into a stopped endpoint. That is why the tester always
reports the same "2480 of N bytes" — that is *our* guard's stop point, not the device's. (First-chunk
credit *latency* is a good tell — 4–8 ms completed vs 32–198 ms wedged — but 20/21, not a rule;
`write_preset` logs `first_credit_ms` for future reports.) Nothing about the bytes explains the
split: the **same paste of the same 6883 bytes** wedged the pedal and then completed 43 seconds
later in the same GUI session after a power cycle, and `fretwire24` wedged, recovered over a power
cycle, completed, then wedged again 56 s later on the same preset. Next step is bytes, not inference —
`FRETWIRE_DUMP_WRITES=<dir>` (already shipped) saves the blob before the first frame goes out.

**Panel knobs now move the UI (2026-08-02).** The tester has twice noted that turning a knob on the
pedal doesn't show up in the editor. Nothing logged undecoded pushes, so no log we had could say
what a knob emits. `fretwire watch` + `FRETWIRE_TRACE_STATUS=1` answered it in one capture: a panel
parameter change is **push type 30**, carrying the *same* `{98: slot, 28: index, 119: value}` triple
the op-30 `set_value` edit sends — the device mirrors panel edits in the vocabulary it accepts them
in. Sweeping the Drive knob of a US Princess in slot 5 produced fifteen pushes with slot 5, index 0
and a descending f32. Parsed into `StatusPush::Param`, forwarded as a `Param` DTO, and applied in
place by the GUI like a bypass mirror (no re-read — the push carries the value, and a sweep pushes
~15 updates a second). Byte-exact test from the captured frame. Type 22 also streams continuously
while idle, so it stays `Other`. [solid]

**Fixed — the real reason the editor stopped following the hardware (2026-08-02).** Chasing the knob
feature turned up something bigger: **the status channel stops delivering pushes after ~4 KiB.**
Four captures, 4075 / 4075 / 4075 / 4040 bytes from frame counts of 179 / 191 / 195 / 386 — a byte
ceiling, not a timeout (4075 + 21 = 4096, the body of the next frame it declined to send). After it,
only empty keepalives: footswitches, knobs and preset changes all stop reaching the host until the
session is reopened. An idle session never reaches it (2037 bytes in 75 s), so it only bites a
session someone is using — exactly the tester's report, and *not* specific to knobs.

The window is re-opened the way a paged read pulls its next chunk: a `cmd 0x08` carrying the offset
advanced by the bytes just received. With it, a 300 s capture delivered **23457 bytes over 1117
frames with pushes still arriving at 299.9 s**, against a ceiling of 4075 without it — and a
re-verified 200 s run on the shipped code passed 9660 bytes with zero warnings. Status channel only,
which keeps the extra frame off the edit channel. Refuted on the way: advancing the idle beat's
`arg` without the request, which changed nothing. [solid]

**Also from these logs:** op 30 refused with code `-3` five times (parameter sets the device threw
out; the UI stayed healthy and surfaced them, which is the intended behavior). Blocks moved into the
send/return loop sometimes only sound with the preceding block enabled — the tester's own later
sessions attribute much of this to the loop endpoint still sitting before DSP1 OUT, so signal level,
not routing. Unchased.

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

## Eighteenth round (2026-08-02): **our op-21 write never terminates a USB transfer**

Four more Floor logs (`fretwire37`–`40`), the `calitest2|3|4` dumps, and the chat. Three results.

**1. The lockup is one gesture, and we may finally have the mechanism.** Six op-21 writes across the
four sessions, five wedged, all six the same user action: dragging the loop endpoint (the split ⋔ /
mixer ⋉ node). That is the only gesture in the GUI that reaches `write_preset` — `set_node_pos` does
a read-modify-write, and every write in these logs is preceded 10–100 ms earlier by a full preset
read, which undo and restore would not do. Everything else the tester did for four hours — model
swaps, bypasses, parameter sweeps, block drags into and out of the parallel path, saves — is
surgical and has never wedged a pedal.

The mechanism, and it is not in the blob: **`wMaxPacketSize` on the bulk endpoints is 512, a frame
is a 16-byte header plus its body, and our 496-byte chunk body is a packet of exactly the maximum
size.** We sent nothing but those, so the transfer had no short packet to terminate it. HX Edit
never does this — its unit is 512 payload bytes split 496 + 16, closing each unit on a 32-byte
packet, in both captures that carry a bulk upload (`move_EQ_right_two_slots`, `import_ir`). Measured
on the Stomp, ours was `512 512 512 512 512 224` against HX Edit's
`512 32 512 32 512 32 512 32 512 32 144`.

It fits what the blob theories could not: the same bytes wedging and then completing after a power
cycle, death at two or three units every single time, session age looking like a predictor and then
failing when `arg` was pinned, a power cycle being the only cure, and the Stomp shrugging it off.
Fixed, and it round-trips clean on the Stomp — which proves only that it is not a regression, since
the Stomp completed writes before it too. **[hypothesis] until a Floor runs it.**

Scanning the ops in all 43 captures the same day also killed a standing suspect: HX Edit does *not*
bracket its whole-preset write with `op 78 → op 43`. `move_EQ_right_two_slots` is a bare op-21,
`one_by_one_move_all_blocks_one_right` is `78,43` eleven times and never reaches op-21. Same op,
same envelope, same terminator. The blob itself is our own minimal re-encode rather than the
device's bytes back (shorter, with the header offset table rebuilt to match — pinned by tests and
exonerated on hardware in the fourteenth round), so **packetisation is the only known difference
left in how the write is *sent*.**

**2. The recurring "envelope key 104" errors are truncated reads.** `fretwire39` caught one whole:
6366 of a declared 7055 bytes, logged as a success and handed to the decoder, which blamed whichever
envelope key the missing tail contained. The read loop was capped at `declared / chunk_0 + 8`
requests; chunk #0 arrived 214 bytes long, giving 40, and the device fragmented the stream twelve
times, each split costing another request. The cap is sized against a fragment now, and a short
payload is an error instead of a preset, so the existing retry gets its go. [solid]

**3. The tester's build predates the push-window fix** — zero status pushes in all four logs. The
panel-follows-UI work and the 4 KiB ceiling fix are both still unseen in the field.

**Also, from a 90-second `watch` on the Stomp:** snapshot switching pushes exactly as documented
(42, then a type-49 bypass mirror per changed block, then 23, then 46 — seven switches, seven times),
so nothing was missing there. Two refinements fell out: **type 41's key 70 is the 0-based
footswitch** (FS1 → 0, FS2 → 1, matched against the type-49 slot on four presses), which retires the
old guess that its key 66 is a state bitmask and hands the "assign a block to a footswitch" request
its wire format; and **type 23 rides every snapshot switch** with a constant `{23: 0}`. Both stay
undecoded in code — acting on them would double-apply what types 49 and 42/46 already say.

**Side effect worth watching in the field.** `read_preset_raw` now publishes the identity it settled
on, which it never did before, and two GUI callers read that identity:

* `copy_preset` labels the clipboard with `last_identity()` *after* a raw read. Previously that name
  came from the last `read_preset` instead — so copying right after navigating **on the pedal** put
  the previous preset's name on the Paste button. That matches the field report of "copied
  cali400test … 'cali400test1' still shows on the Paste button". Consistent with it, not proven: the
  exact sequence hasn't been replayed.
* `check_cross_setlist_write` compares the target bank against `last_identity()` and **passes when
  there is no identity at all**. Every op-21 path reads raw, so on those paths the guard was
  previously inert; now it engages, using an identity `read_preset_raw` only publishes once it has
  settled. Safer, and a live behaviour change — a save that crosses setlists will now be refused
  where it used to go through (`FRETWIRE_SETLISTS=1` is the escape hatch).

## Nineteenth round (2026-08-02 evening): **a switch takes a bool, and nothing else**

`fretwire42`/`43` (pre-pull) and `fretwire45` (first Floor session on the new build), three preset
dumps, three screenshots, and a long chat log of the tester filling all eight blocks and dragging
things around to see what breaks. Four results.

**1. The truncated-read fix holds on the Floor.** `envelope key 104` appears five times in the chat
during the `fretwire42` session and never again after he pulled — zero decode failures in
`fretwire45`, against six `preset read/decode failed` in `fretwire42`. The Round-18 fix is confirmed
in the field. The write lockup is *not*: he wedged the pedal once on the new build moving the mixer,
which is an op-21, but that session's log (`fretwire46`) hasn't arrived, so the packetisation
hypothesis is still open.

**2. Every on/off switch in the GUI was a guaranteed refusal.** He hit it on a reverb's `Trails`:
"pedal refuse the para change (op30) device code -3". Reproduced on the Stomp and separated into two
distinct causes.

The first is ours. Key 119 is **not coerced** — a switch takes a MessagePack bool and refuses an int
or a float carrying the same 0/1 with `-3`. Measured on `HD2_DelayBucketBrigade`'s `TempoSync1`:
`Float(1.0)` → `-3`, `Int(1)` → `-3`, `Bool(true)` → accepted, reads back `true`. The GUI's switch
control routed through `set_param_enum`, which sends an int, so no switch in the editor had ever
worked. `Session` now reads the param's type out of the device's own last blob — no reference data
needed — and coerces; verified live, `set 2 7 0` flips `TempoSync1` where it used to be refused.

That fix needed a second one. `param_is_bool`, `clamp_param` and the split-bypass redirect all read
`last_raw`, and a one-shot CLI invocation connects and edits without ever having read anything, so
all three silently answered "no". `ensure_blob()` does one ~3 KB read on the first edit of a session
(free thereafter, and best-effort — a failure there must not fail the edit).

**3. `Trails` is genuinely unreachable, and that is a protocol fact, not a bug.** It is refused as a
bool too — as an int and a float as well. It is the one value a delay/reverb sends **past the end of
its symbol's param list**, and key 28 is an index into that list, so nothing addresses it. Same for
the mic index on a legacy cab. HX Edit shows a Trails switch, so it reaches it another way (op 25
`setting` is the obvious suspect; no capture of a Trails change exists yet). `EditorParam::settable`
is `false` for these and the GUI shows the value with no control, rather than a switch that can only
fail.

**4. A row-B theory that lasted an hour.** Ten-plus times he dragged a block onto the lower row and
it went silent. The bracket we draw stops at the mixer column, so cells past it look disconnected,
and I stopped offering them as drop targets. The next dump refuted it: `somehinged3_var1.bin` has
the mixer before column 3 with both loop blocks moved out to columns 3 and 4, `somehinged2.log`
shows the moves as two clean `78 → 43` pairs with two saves and no errors, and he checked by ear —
they play. The baseline dumps kill the inverse reading too: `somehinged`/`somehinged2`/`midhinged`
all have both loop blocks *inside* the bracket, and that is the session he spent reporting silent
loop blocks. Reverted; **what silences a block in the loop is still open**, and the mixer's own B-leg
level/pan is the next suspect.

Two things survive it. The device is **looser than our own enclosure guard** — op 43 moves a loop
block outside the bracket and the pedal keeps it — and that guard is what blocked him from putting
the mixer between blocks 1 and 2, twice. Worth a hardware test before removing. And `show-preset`
now prints each DSP's bracket with the loop blocks' columns, which is how this got settled in one
command instead of by hand-decoding key 13.

**5. Trails works — key 29 is an addressing mode, not a flag.** Sean's first report off the new
build was that the Trails switch had gone. It had: I had just made it read-only on the finding that
op 30 refuses it as bool, int and float alike. That finding was right and the conclusion was wrong.
`captures/dynamic_ambience_trails_on_off.pcapng` had the answer all along —

    Mix:    {98: 7, 29: true,  26: 0, 28: 5, 119: 0.5}
    Trails: {98: 7, 29: false, 26: 0, 28: 0, 119: <bool>}

— six toggles, all the same shape. **Key 29 chooses what key 28 indexes**: `true` = the param's
place in the model's symbol order, `false` = the block's *extra* values, where the lone trailing one
is `0`. Confirmed live: `{98:2, 29:false, 26:0, 28:0, 119:true}` turns a Bucket Brigade's trails on
and it reads back `true`. `EditorParam::extra_index` carries it, the ordinary setters route through
it, and the switch is back in the GUI. A block with *two* values past its symbol list stays
unaddressable — no evidence for what the second index would be.

**6. Our enclosure guard was stricter than the pedal, and it was the real obstacle.** Three times in
one evening the tester couldn't place a node — the mixer between blocks 1 and 2 (twice) and the
split after block 3 — because `set_node_pos` and the UI both required the bracket to keep enclosing
the occupied B row. Op 43 does not care: it moved his loop blocks out past the mixer, the pedal
saved them and they play. Relaxed to the one structural rule that is actually ours to keep (split
left of mixer, inside the grid); leaving blocks outside the bracket now logs a warning instead of
refusing.

**Not a bug: the three dumps.** `somehinged`, `somehinged2` and `midhinged` are byte-identical apart
from byte 3 (the volatile header byte) — three captures of one preset. `dump-raw` reads whatever is
loaded, and it now prints which preset that was, which is the fix from the round before.

## Twentieth round (2026-08-02 late): **`-306` is out of DSP, and the mixer is innocent**

`somehinged3.log` / `somehinged3var3.log`, two screenshots, and the chat from the session where he
methodically moved every loop block two columns right and tried to reposition the split and mixer
around them. Four results, two of them things we had been wrong about for weeks.

**1. `-306` on op 40 means the DSP is full.** It looked like a property of the model — a Room reverb
refusing to become a Euclidean Delay, a Bleat Chop Trem refusing to become an Elephant Man. It is
the preset's total load and nothing else. Same preset, same slot, same target model on the Stomp:
refused at 71.8% used, accepted at 65.3%. A ladder of targets brackets the ceiling between a landing
total of 74.9% (accepted) and 75.3% (refused), so **the device fills up at about 75% on our meter**
and "28% free" can mean no room at all. The meter sums the blocks' `load` and counts nothing else;
the missing quarter is probably the fixed input/output/split/mixer nodes, unconfirmed. His DSP1 was
at 72.7% — effectively full — so both his refusals were ordinary.

Eight hardware probes were needed to kill the first theory, that op 40 cannot cross a model
category. It can: tremolo→delay, reverb→delay and delay→reverb all work with DSP free, and the one
category-preserving swap that failed (70s Chorus Mono→Stereo) failed on capacity like the rest.
`send_edit` now glosses the code instead of guarding against it — the pedal decides what fits.

The rejection log with the target map, added the round before, is the only reason this was
diagnosable at all.

**2. The mixer is a block, and its levels are not the answer.** Round 19 nominated the B-leg
level/pan as the next suspect for the silent blocks. It is readable now — the split and mixer carry
a model and a stored param array like any block, and `show-preset` prints them instead of making you
decode key 15/17 by hand. In `somehinged3_var1.bin` the mixer (`HD2_AppDSPFlowJoin`) is A Level 0,
A Pan 0.5, B Level 0, B Pan 0.5, B Polarity off, and the split (`HD2_AppDSPFlowSplitY`) is
BalanceA/BalanceB 0.5. Unity and centred. **Third theory dead**, and this one leaves nothing behind:
those six values and the split's two are all the routing knobs there are.

What it does expose is that he could never have checked: the mixer glyph has always been clickable
and nothing said so. It has a tooltip now, and the CLI prints the values.

**3. The bool fix is confirmed in the field.** `somehinged3var3.log` has
`{98:2, 29:true, 26:0, 28:9, 119: Bool(true)}` — `TempoSync1` sent as a MessagePack bool — accepted,
code 0, and zero op-30 `-3` across both new logs. On the older build the same gesture is the `-3` he
hit twice in chat, on a delay and on a reverb. Trails specifically is the key-29 case from result 5
of the last round; `somehingeddelaytrails.png` shows it greyed out, which is exactly the read-only
state that fix removes.

**4. Confirmed: our node guard is what blocked him, not the rendering.** His own diagnosis was that
the drop targets were being covered by the loop blocks beneath them — "there's no vertical target
... because the two loop elements are beneath it". Nothing was covering anything; the UI never drew
a target, because the mixer's range started at `max(last B column + 1, split + 1)`. With loop blocks
at columns 1 and 2 that is column 3, and he wanted column 2. Fixed the round before, unpushed at the
time he hit it.

**Not a bug: the doubled writes.** Every discrete param change appears twice in the logs with
consecutive transaction ids and identical bodies (`59`/`60`, `72`/`73`, …). That is `preview_param`
streaming the gesture and `set_param` committing it — by design, and the commit is what earns the
history entry and the re-read.

**Open.** Still no explanation for a block going silent in the loop; the routing knobs are now
excluded, so the next place to look is the per-snapshot bypass mask (every block he called silent is
`-` in the active snapshot of the dump he sent, which may be cause or may be coincidence). Still no
`fretwire46`, so the op-21 packetisation hypothesis is untested. One cosmetic cost worth knowing:
every committed edit triggers a full ~7 KB preset re-read — 309 chunk round-trips for 12 edits in
`somehinged3var3.log`.

## Twenty-first round (2026-08-02 night): **the silent loop block, found** [result 1 refuted — see the twenty-second round]

Three more sessions off the pushed build (`somehinged3var4/5/5a.log`, one screenshot). Zero
refusals, zero decode failures, Trails working — and the mystery that has run since the Floor
arrived is answered by a warning we were already printing.

**1. A loop block left of the split has nothing feeding it.** `somehinged3var5.log`:

    WARN moving this node leaves row-B blocks outside the bracket — strays=[3] pos=4 kind=2

He dragged the split to column 4 and left the loop block at column 3 outside it, on the left. Then:
"tronup works / no wait, it doesn't / but helio does, weird". The two loop blocks were on opposite
sides of the split — Heliosphere at column 4 inside the bracket and audible, Tron Up at column 3
outside it and dead. He swapped three filters into that slot chasing it (`HD2_FM4ObiWah`,
`HD2_FM4QFilter`, `HD2_FilterMysterFilterMono`, all in the log, all ACKed) and none made a sound.
The signal has not branched yet at that column, so the cell has no feed. [hypothesis — one session,
but it explains every "no worky" on record that Round 22 did not]

This does **not** undo Round 22. Past the mixer he verified by ear that blocks still play. The
bracket is asymmetric: the right side is cosmetic, the left side is not.

**2. Our warning was right and nobody could see it.** It went to a log file. The chain now gives an
unfed loop block an amber dashed border and a "⚠ no feed" badge with the reason on hover, and the
split's drag caption says a drop will strand the blocks it lands right of. Still nothing refused —
the pedal accepts the arrangement, and out-guarding it is what caused the last two mistakes.

**3. "Hella quiet" is not a bug.** Split Y's `Balance A`/`Balance B` are both at the 0.5 factory
default, so each leg is attenuated and the mixer sums them; a parallel delay against a dry amp path
is meant to sit back. His mixer is 0 dB on both legs with +3 dB output — essentially factory.

**4. Edit ACKs log their target.** Matching "I tried three different filters" to the wire meant
hand-decoding the MessagePack in each op-40 reply, because `send_edit` logged the target only on
refusal. It logs it on success too now, so a session log reads back as a list of what was done.

**Open.** `captures/_RUNBOOK-hx-edit-session.md` collects everything now blocked on an HX Edit
capture — the node move (the op-21 lockup), how HX Edit switches a block mono↔stereo, a `Cab › Dual`
block, and the op-25 global-settings ids — plus the two things that need only a screenshot.
Confirmation for result 1 is one gesture: drag the split back left of the stranded block
and the filter should come alive with nothing else changed. Still no `fretwire46`, so the op-21
packetisation hypothesis remains untested.


## Repo map
`crates/` (fretwire-data, fretwire-protocol, fretwire-usb, fretwire-core, fretwire-cli,
fretwire-tauri) · `docs/` (protocol, preset-format, safety, next-steps) · `captures/` (pcaps + notes
+ reassembled blob) · `tools/` · `ROADMAP.md`.

## Twenty-second round (2026-08-03): **the packetisation fix is measured, and the split was a red herring**

Four sessions plus an archive of dumps and screenshots (`fretwire48`–`51`, `somehinged4`). The
tester rebuilt partway through the run, which turned the whole log pile into a controlled
before/after.

**1. Ending each op-21 unit on a short packet cut the lockup rate from 68% to 12%.** `fretwire43`
starts with a `Compiling` line — that is him picking up `80ee812`. Across every write we hold: 21 of
31 wedged before it, 3 of 26 after. Same pedal, same presets, the same evening. The hypothesis is
confirmed as *a* cause and refuted as *the* cause; the remaining 12% wedge with the identical
signature.

**2. One uncredited chunk is the entire signal, and our guard wanted three.** Over all 51 recorded
writes the split is total: the 29 that completed were credited at every single chunk (14–19 credits,
`silent` never once reaching 1); the 22 that wedged got 1–3 credits and never another. Nothing in
between. `MAX_SILENT_CHUNKS` is now 1.

That was not cosmetic. The device now wedges after 2–3 chunks, so waiting for a third silent one let
the next blocking send time out first — which is precisely how `fretwire48` and `fretwire51` failed:
`bulk OUT timed out`, again, then the keepalive dropping the session. Four seconds of nothing and
none of the numbers the guard exists to print. It now fires in ~250 ms and says a power cycle is
needed and that flash was untouched. It still cannot save the pedal; `fretwire26` settled that the
abort is not the cause and the device is gone before we notice.

**3. Round 21's result 1 was wrong: it was never the split.** He moved the split so Heliosphere sat
outside the bracket on the left, our new badge lit up — and he heard the block play anyway. "So the
UI logic says NOPE, but Helix say This is fine." Sorting both evenings by what the blocks *were*
instead of where they sat: every model he called dead is an envelope filter (Tron Up, Obi Wah, Q
Filter, Mystery Filter — all sweep on input level), and every model he called merely quiet is a
delay or reverb. Split Y's legs sit ~6 dB down at the 0.5 default, which is enough that an envelope
filter in path B may never open, wherever in path B it is. Position tracked effect type by accident,
twice.

The "⚠ no feed" badge is gone. A row-B block outside the bracket still gets a marker, because HX
Edit cannot draw that layout at all, but it is a muted "outside path B" note that says the pedal
keeps it and plays it. Third time for the same lesson — do not out-guard the pedal.

**4. The paste buffer is ours, not the pedal's.** He saw "somehinged2" stay on the Paste button
across a UI crash and a device reboot, wondered if it lived in firmware, and answered it himself: it
survives within a run and is gone after the app restarts. Worth knowing when reading his logs; not a
bug.

**Open.** Result 3 is one gesture from settled with no Windows involved: put an envelope filter in
path B, then raise `Balance B` or its Sensitivity and listen. `captures/_RUNBOOK-hx-edit-session.md`
is unchanged and still the list for the next HX Edit session — the node move stays top of it, since
12% of writes still wedge and we have never watched HX Edit perform that edit.

## Parameters read in real units (2026-08-03)

`HelixControls.json` was only being used for enum labels. Every continuous parameter showed its raw
DSP value, which is how the tester spent part of a session on an Adriatic Delay reading `1.3728`,
couldn't tell what it meant, and nearly filed it as broken:

> fiddled with the adriatic delay, the time was was too long, almost made me think it wasn't working

It was a 1.4-second delay. The same file already held the recipe to say so — a `dspToDisplayScale`,
range-switched `format` rules, and `formatUnits` templates — so `ParamMeta` now carries it and both
front ends apply it:

    [ 0] Time           = 1.373 s       [1.3728]
    [ 1] Feedback       = 50 %          [0.5]
    [ 5] Level          = +0.0 dB       [0]
    [ 9] SyncSelect1    = 1/4 Triplet   [6]

The CLI keeps the raw value in brackets because that is what `fretwire set` takes. The GUI formats
client-side, from rules sent in the param DTO, because a slider re-renders on every drag frame
before any value reaches Rust. Aliases are resolved (`time_ms_20_1800` → `time_ms`), ranges pick
their own units (ms under a second, seconds past it), and anything the reference data doesn't
describe falls back to the bare number.

## The DSP meter tells the truth about headroom (2026-08-04)

Round 20 measured where the pedal stops accepting blocks — **~75% on our meter, not 100%** — and
then nothing used the number. `DSP_BUDGET` was still the invented `100.0`, so `show-preset` printed
`72.7% used (27.3% free)` for a preset with room for nothing, the GUI's meter and its "does it fit"
greying agreed, and the swap fit-check warning never fired before a `-306` did. **The tool was
telling people they had a quarter of a DSP they did not have.** Fixed:

- `DSP_BUDGET = 100.0` → **`DSP_CEILING = 75.0`**, carrying its provenance (the Stomp ladder, and
  the Floor's `somehinged3` refusing `+6.02` at 72.7%, which brackets that device under 78.7 too).
- `EditorPreset::dsp_free_on` / `dsp_free_by_dsp` / `dsp_load_on` — headroom is `ceiling - load`,
  floored at zero, **per DSP**.
- CLI: `DSP 72.7% used · 2.3% free (the pedal refuses past ~75%)`.
- GUI: header and per-DSP headings show used *and* free; the ceiling ships in the preset DTO
  (`dsp_ceiling`) so the two Svelte copies of `const BUDGET = 100` are gone.
- The `swap` fit check compared the projected load against the **whole preset's** total. On a Floor
  that both warns about presets that fit and stays quiet about ones that don't — each DSP is
  budgeted on its own. Now checked against the target block's DSP.

**The missing quarter is a flat reserve, not the routing nodes.** The standing guess was the fixed
input/output/split/mixer nodes we never sum. `io.models` does price them (`HD2_AppDSPFlow1Input`
`10.99`, `HD2_AppDSPFlowOutput` `8.00`, Split Y `1.50`, Join `10.99`) and they are real slots in
the preset — `0`, `9`, `10`, `19` of each DSP's array, kinds `0`/`1`/`2`/`3` against an ordinary
block's `6`. But they do not explain the gap, and the arithmetic said so before the census did: the
ladder preset is parallel, so the four together are 31.48 and would put the ceiling at 68.5, where
73.3 was accepted. No subset lands on the ~25 the bracket demands.

**The census that settles it.** His `.hxb` backup has been sitting in `captures/helix-floor/` since
July and we had never used it as evidence: 363 presets over eight setlists, **including Line 6's own
two factory setlists** — 458 DSPs carrying blocks, every one a preset the hardware accepted. Summing
each DSP's block loads the way our meter does:

| slice | n | max load |
|---|---:|---:|
| everything | 458 | **74.84** |
| parallel DSPs | 151 | 74.84 |
| serial DSPs | 307 | 74.80 |
| DSP1 · DSP2 | 302 · 156 | 74.84 · 74.77 |

The wall is **flat**. Were the split and mixer billed here, serial presets would run to ~87 and
parallel ones stop at ~75; both stop at 74.8, a difference of 0.04, and the same holds across split
types. Nothing preset-dependent is being charged — the device keeps a quarter of each DSP and lets
blocks have the rest. 46 DSPs sit in the 70–74.84 band and **none above**, so Line 6 builds right up
to this number too. With the Stomp ladder (74.9 accepted, 75.3 refused) the ceiling is pinned to
**[74.9, 75.3)**, hence 75.0.

Also learned, independently useful: **each DSP has two inputs and two outputs** (`inputA`/`inputB`,
`outputA`/`outputB` in the `.hxb` tone), not one of each. The reproduction recipe for the census is
in `docs/protocol.md`.

Still open, and now purely cosmetic: what HX Edit puts on screen. If it displays `blocks ÷ 75` its
meter reads ~97% where ours reads 72.7% and we could show the same number instead of a measured
ceiling. One screenshot decides it; no fit check depends on the answer.

### The meter now reads 0–100 (2026-08-04)

With the ceiling pinned to [74.9, 75.3), the honest presentation is a percentage of *capacity*:
`editor::dsp_percent` divides by `DSP_CEILING`, so a full DSP reads 100% instead of 75%. The
tester's `somehinged3` goes from `72.7% used (27.3% free)` — which is what made his refusals look
like the pedal misbehaving — to **97.0% used · 3.0% free**.

- **GUI:** scaled everywhere and raw nowhere — header, per-DSP headings, the model picker's "DSP
  free", and each model's cost in the picker list. Scaling the model costs too is what keeps the
  arithmetic on screen self-consistent (a 5.6 block reads 7.5%, and 97.0 + 7.5 overflows, correctly).
- **CLI:** scaled too — **including each block's own cost**, so a listing's figures sum to its
  header. Two scales in one listing is a trap; the raw sum rides along in the header's brackets
  (`DSP 97.0% · 3.0% free  [raw 72.7 of ~75]`) for `.models` lookups and for the logs and docs that
  quote raw, which is all the anchor that was actually needed.
- Fit comparisons are unchanged and still in raw units. Scaling is presentation only.

**The picker's grey-out works now.** It was always there — models that don't fit are disabled — but
against a budget of 100 it effectively never fired: a preset at 72.7 looked like it had 27 free, so
everything passed. Against the real ceiling it greys the models the pedal would actually refuse.
Disabled entries now say why on hover ("Needs 7.5% DSP; only 3.0% is free…") and show their cost in
warning colour, since that number is the whole explanation.

## Twenty-third round (2026-08-04): **the stall guard works, and the picker fix arrived a day late**

`fretwire52.log` / `fretwire52a.log` plus two dumps, from the 08-04 build.

**1. The fail-fast write guard is confirmed in the field.** Both wedges were caught exactly as
designed — `silent=1`, 1536 of ~7100 bytes, ~250 ms after the last credit, with credits and
`first_credit_ms` in the record:

    write-preset chunk sent=1536 total=7120 credits=2 silent=1
    ERROR device stopped acknowledging mid-write — abandoning sent=1536 credits=2 chunks=3

On the old guard this was four seconds of bare `bulk OUT timed out` with nothing to diagnose. It
still does not *save* the pedal — the keepalive times out 2 s later and the session drops, as
documented — but the diagnosis is now free. Both wedges hit at chunk 3 of a node move.

**2. The wedge rate is not obviously improving.** 8 writes, 2 wedged (25%) this session; running
post-`80ee812` total 34 writes / 5 wedged (**15%**), against 68% before it. Small sample, and both
of tonight's were node moves — still the operation that does it.

**3. Three `-306` refusals on op 39 (add) that the new build would have prevented.** He tried to add
to slot 8 three times on `somehinged2_test`, which sits at **raw 69.7 — 92.9% of capacity, 5.3 raw
free**. Nothing was wrong; the model didn't fit. That is exactly what the picker now greys out, so
these logs are the last from a build that let him try.

**4. A third envelope-filter-in-path-B case.** `somehinged3_baseline_loop-no-worky.bin` has a
**Mystery Filter** (Sensitivity 5.2) at slot 15 in path B with `BalanceB` at 0.5, next to a Dual
Delay that works. Same shape as Round 26 both times. He has named it "baseline", so he is setting up
the controlled comparison himself.

### The write wedge has an early warning, and we were measuring the wrong chunk (2026-08-04)

Re-analysing every write on record with per-chunk timings — 39 of them — turned up a near-perfect
discriminator that is **not** the one we have been logging:

| the chunk-2 credit took | writes | outcome |
|---|---:|---|
| 5–8 ms | 19 | all completed |
| 23–253 ms | 19 | all wedged |
| 3 ms | 1 | wedged (`fretwire24`, the standing exception) |

**Chunk one's credit predicts nothing** — `first_credit_ms` is 2–7 ms on wedged and completed
writes alike. That field was added specifically to capture this signal, and it measures the wrong
chunk: the original analysis had found the right thing but computed it as the gap between the first
two chunk log lines, which is chunk *two*'s credit wait. Named for chunk one, implemented for chunk
one, and it has separated nothing ever since.

So the device **does** degrade before it dies — a full chunk before the silence — which refutes the
"it stops dead, it is never outrun" reading. Acting on it:

- `SLOW_CREDIT` (15 ms, in the empty gap between the two populations) marks a chunk as lagging.
- On a lagging chunk we **stand off `BACKOFF` (120 ms)** instead of pushing the next one in. This is
  the actual fix attempt: the theory is that feeding a receive path that has stopped keeping up is
  what finishes it, which is the one mechanism consistent with the short-packet fix helping, the
  blob never mattering, and a slow credit predicting death one chunk early.
- Two consecutive slow credits means backing off did not take, so we stop there rather than feed
  the silence — no completed write on record has even one slow credit, so this cannot misfire.
- `credit_ms` per chunk and `worst_credit_ms` on the summary line are now logged, so the next round
  says whether the back-off worked. A healthy write should show single digits.

**Unconfirmed by construction:** an HX Stomp has never wedged, so this cannot be tested here. It
ships as a hypothesis with the instrumentation to judge it.

### Twenty-fourth round (2026-08-05) — the back-off is refuted, and the panel mirror was in the bin

`fretwire55`/`56` and `zadtheinhaler57`/`58` are the first logs from the back-off build, and they
carry per-chunk `credit_ms` for every write. 20 writes, 3 wedged (**15%** — unchanged).

**1. The chunk-2 predictor is confirmed, exactly.** With real instrumentation instead of log-line
arithmetic the two populations do not overlap at all:

| chunk-2 credit | writes | outcome |
|---|---:|---|
| 1–3 ms | 17 | all completed |
| 28 / 32 / 94 ms | 3 | all wedged |

`first_credit_ms` is 0–5 ms in both groups and still separates nothing. Slow credits on the **final**
chunk are normal — 2–195 ms on writes that complete perfectly — so only non-final chunks count.

**2. Backing off does not rescue the write. Refuted, 3 for 3.** In every wedge the pedal went from
one slow credit straight to complete silence on the very next chunk, and **not one credit arrived
during the 120 ms pause** (`credits` is unchanged across it in all three). Whatever the slow credit
means, it is not congestion we can drain by waiting.

So the loop no longer pauses — it **stops at the first slow mid-transfer credit, with the next chunk
still in hand**, and `SLOW_CREDIT` moves 15 → 22 ms. The threshold has to clear the worst *non-final*
credit on a write that went on to complete, which is 16 ms (`fretwire56`, chunk 7 of 14, a blip the
device came straight back from); 22 sits between that and the slowest wedge, so no recorded healthy
write trips it. The write was lost either way — nothing with a slow chunk-2 credit has ever
completed — so the only thing that changes is that we stop pushing 512 more bytes into an endpoint
that has stopped draining. **Open:** every wedge to date has ended needing a power cycle, and
whether withholding that last chunk spares it is the one variable never tested.

**3. The status channel's panel mirror was being thrown away mid-request.** [solid] Sean has asked
three times why a footswitch press doesn't move the GUI, and why clicking *any* block in the GUI
then makes the pedal's real state appear. Root cause: `Transport::request_matching` **dropped** every
frame that wasn't the reply it was waiting for. `zadtheinhaler57` discarded **111** status frames in
one session — 49 of `body=21`, 23 of `body=35`, and so on, all `cmd 4` from `0x03F0`.

Everything downstream already worked: `parse_status_push` decodes type 49 to
`StatusPush::Bypass { slot, enabled }`, `dto.rs` maps it to `PushDto::Bypass`, and the GUI applies
it. The frames just never got there — they arrived while some other request was in flight and went
in the bin, and the GUI's next full re-read (triggered by the user's own click) is what finally
showed the truth. Non-reply frames are now put **back** on the pending queue in arrival order;
keepalives and empty bodies are still dropped, so the queue can't grow without bound.

Not unit-testable here — `Transport` wraps a live `nusb` interface and the crate has no fixture for
it — so this is verified by construction and by the field logs that show the frames existing.

**4. Field notes worth keeping.** Copy/paste of a preset across setlists carries footswitch
assignments (works). A cab switched to Cab › Mic+IR costs *less* DSP, not more. The `strays` warning
matches what he hears: a block left outside the split bracket goes quiet or drops low in the mix, so
that warning is earning its place. Minor UI nit: the block-grid scroll bar doesn't come back after
the window is narrowed again.

### The credit counter has never counted credits (2026-08-05)

Chasing why chunk 2 goes slow turned up something more basic: **`drain_collect` filters nothing**,
so the write loop's credit wait is satisfied by *any* frame — keepalives (`cmd 0x10`), status pushes
(`cmd 0x04` from `0x03F0`), anything. `credits` has always meant "frames seen", and it is the number
that decides how fast we push at the one endpoint that wedges when it is outrun.

It shows up as a surplus that cannot be explained away: **all 17 completed writes in the 08-05 logs
end with more credits than chunks sent** (+1 to +4). Two readings fit, with opposite consequences:

- **Strays** — non-credit traffic is landing in the wait, so we have been running ahead of a device
  that never acked those units. That is a mechanism for the wedge, and it fits its character:
  intermittent, indifferent to the blob, worse in a session someone is actively driving. It would
  also explain the slow chunk-2 credit — the wait at chunk 2 is the *chunk-1* credit finally
  arriving, because something else was counted in its place.
- **Per-frame credits** — a unit ships as two frames (496 + 16), so a device that sometimes acks the
  frame rather than the unit legitimately returns up to 28 credits for 14 chunks, and pacing is fine.

Both look identical in a log that records the count and never the frames, which is exactly why this
hid under a number we had been reading confidently for weeks. The loop now classifies every frame
(`real` = empty `cmd 0x08` on the edit channel vs `other`, plus `stray_src`/`stray_cmd` naming the
first non-credit frame counted as one) and reports `real_credits`/`stray_frames` per write.

**Pacing deliberately unchanged.** Demanding a strict credit the device might label differently
would fail closed at chunk one and take the tester's ability to save a preset with it — and a second
untested change on this path would make the next logs unreadable, since the stop-on-slow-credit
change is already in flight. One session of `real`/`other` settles which reading is right, and then
the guard moves with evidence behind it.

### Settled on hardware: the credit counter was counting panel pushes (2026-08-06)

Run against the HX Stomp with the classification from the previous section in place. **The strays
reading is right and the per-frame-credit reading is wrong.** Eight `write-roundtrip` writes across
two presets, every one of them exact:

    chunks=5  real_credits=5  stray_frames=3      (ClaudeTest, 2273 bytes)
    chunks=6  real_credits=6  stray_frames=1..3   (factory 0, 2767 bytes, split preset)

One `cmd 0x08` per 512-byte unit, never per frame. The three strays are named now:

    0x03f0  cmd 4  body 21    status-channel panel push
    0x03ed  cmd 4  body 17    the edit apply-ACK, {102: txn, 103: 1}
    0x03f0  cmd 8  body 0     the status channel's own page frame

The last is `cmd 0x08` on the *status* channel — so matching on opcode alone would still be wrong,
and the source check earns its place.

**Changed:** the wait is for a real credit; strays are set aside and returned to the queue when the
transfer ends; `slow`/`silent` both run off the strict count. Verified live — 8 writes, no aborts,
no false positive from `SLOW_CREDIT` (the only slow credits were final-chunk commits, 82–100 ms,
correctly ignored), plus four `node-pos` split moves — the op-21 node move that wedges Floors —
all of which completed and read back correctly.

**Two bugs fell out of it.** `acked` had been reporting **false on every write**: the apply-ACK
arrives mid-transfer, so the credit wait ate it and the closing sweep never saw it. It reads `true`
now. And two of the three strays are status-channel frames, so the write path had been swallowing
the panel mirror for the length of every save — the same loss as `request_matching`, by a different
route. Both are now returned to the queue.

**Also fixed, found live:** the requeue from the previous round had no ceiling, and a set-aside
frame is re-examined by *every* subsequent request until something drains it — the log showed one
frame (`got_seq=4`) rescanned ten times in a row. Past `MAX_SKIP` (64) that would starve a request
of its skip budget and fail a reply that had actually arrived. Capped at `MAX_PENDING` (32),
dropping oldest first, since these frames mirror current state and the newest are the ones worth
keeping.

**Still open:** whether this is *the* Floor wedge. A Stomp has never wedged, so it cannot be shown
here. What can be said is that the host can no longer get ahead of the device by mistaking a panel
push for permission to send — which was a real mechanism, and is now closed.
