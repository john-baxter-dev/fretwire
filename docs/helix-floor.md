# Helix Floor — device survey

What we know about extending fretwire to the **Helix Floor**, from a contributor's USB captures and
device backup (firmware **3.82**, captured 2026-07-22 on Windows/USBPcap).

**Status after capture 5:** the handshake is byte-identical to the Stomp's, our existing parser
reads Floor presets, every edit builder we have (`set_value`, `bypass`, `swap_model`,
`begin_structural`, `save_preset`) is **byte-exact against the Floor's own wire traffic**, and the
last open question — how an edit addresses a block on the *second* DSP — is **answered**: slot
numbers are **global**, `slot = dsp * 20 + index`. See "DSP2 addressing" below.

No protocol change is needed for this device, read or write. What remains is entirely in our own
preset *model*: slot **type 7** and preset key **1** (second DSP).

The source files are contributor-supplied and contain their personal presets, so they stay out of
git (`.gitignore`: `/captures/helix-floor/`, `*.hxb`). This document is the distilled result.

**Bottom line: no further captures are needed for Floor support.** The data layer covers the Floor
completely, the USB plumbing is identical to the Stomp's, and the protocol is fully accounted for.

As of 2026-07-23 the read/edit path is **implemented**: both DSPs are enumerated, slots are global,
`type 7` blocks are handled, and `Transport::open` matches the Floor's PID. Still to do: the routing
*planner* and the GUI grid, which both assume one DSP. And it has never been run against physical
hardware — see "Next steps".

## USB identity  [solid]

| | HX Stomp | Helix Floor |
|---|---|---|
| VID / PID | `0x0E41` / `0x4246` | `0x0E41` / **`0x4248`** |
| `bcdDevice` | — | `0x0200` |
| Device class | — | `0xEF` (misc / IAD) |

Config descriptor (304 bytes, 6 interfaces) — read directly out of the capture:

| iface | alt | class | endpoints |
|---|---|---|---|
| 0 | 0 | **Vendor (`0xFF`)** | **bulk IN `0x81`, bulk OUT `0x01`, 512-byte max packet** |
| 1 | 0 | Audio control | — |
| 2 | 0/1 | Audio streaming | iso OUT `0x03`, 224 B |
| 3 | 0/1 | Audio streaming | iso IN `0x83`, 224 B |
| 4 | 0 | Audio / MIDI | bulk OUT `0x04`, bulk IN `0x84` |
| 5 | 0 | HID | int IN `0x85`, 8 B, interval 8 |

Interface 0 is the MI_00 control channel, and it is **byte-for-byte the same shape as the Stomp's**
— same interface number, same endpoint addresses, same max packet size. `CONTROL_INTERFACE`,
`EP_IN` and `EP_OUT` in `fretwire-protocol` need no change.

`PID_HELIX_FLOOR` is now defined in `fretwire-protocol`, and the udev rule covers `4248`. Nothing
*matches* on that PID yet — `Transport::open` still finds only the Stomp. Opening a Floor and
speaking Stomp-shaped commands to it is not something to do on a hunch; see "Next steps".

## Preset / data-model identity  [solid]

The setlist and preset JSON carry a numeric `device` field:

| device | value | snapshots | DSPs |
|---|---|---|---|
| Helix Floor | `2162689` = **`0x210001`** | 8 (`snapshot0`…`snapshot7`) | `dsp0` **and** `dsp1` both populated |
| HX Stomp | `2162694` = **`0x210006`** | 3 (`snapshot0`…`snapshot2`) | `dsp0` only; `dsp1` empty |

`device_version` = `0x03800000` for fw 3.82.

Worth knowing: the reference data we already import ships `default_preset.hlx` and
`empty_preset.hlx` that are **already `device 0x210001` — Helix Floor presets**, with all eight
snapshots. Only `default_preset_hxs.hlx` is the Stomp. HX Edit's data has always described the
Floor; we simply never read it that way.

### The catalog already covers the Floor  [solid]

Checked every block in all 363 non-empty presets of the contributor's backup against our imported
`.models` catalog:

- **355 distinct models used → 0 missing.** Every one resolves in our shipped `.models`.
- **2,621 blocks / 19,377 named parameter keys → 0 unmatched.** Every parameter name on every
  block exists in that model's `params` list.
- The structural flow nodes resolve too: `HD2_AppDSPFlow1Input`, `…2Input`, `…Join`, `…Output`,
  `…SplitAB`, `…SplitY`, `…SplitXOver`.

The model namespace (`HD2_*`) is shared across the HX family — it is not per-device. So
`fretwire-data`, the `Helix.sym` parameter ordering, and therefore the **parameter-index
computation that `edit::set_value` depends on** should carry over to the Floor unchanged.

### Topology delta vs. the Stomp

- **Two DSPs.** Both `dsp0` and `dsp1` are populated; the Stomp only ever uses `dsp0`.
- **Block slots:** highest index observed `dsp0.block13`, `dsp1.block12` — so at least 14 slots per
  DSP, versus the Stomp's flat 20-slot array.
- **Two paths per DSP** (`@path` 0/1) → the four paths (1A/1B/2A/2B) users talk about.
- **8 snapshots**, versus 3.
- Per-DSP structural nodes: `inputA`, `inputB`, `split`, `join`, `outputA`, `outputB`, plus paired
  `cab0`/`cab1` nodes hanging off amp blocks (the same amp+cab pairing our preset doc describes).
- Floor-only `tone` sections we have no equivalent for: `footswitch`, `commandFS1`…`commandFS11`,
  `commandInst4`, `dt0`/`dt1`/`dtdual` (DT amp link), `powercab0`/`1`/`dual`, `variax`,
  `irUuidTable`.

## The `.hxb` backup container  [solid]

Fully decoded, and simple enough to support directly:

```
0x00  "AF6L"          magic (4 bytes)
0x04  u32             version = 1
0x08  u32             1100769   (payload-ish length; not yet pinned down)
0x10  u32             141       (= stream count 138 + 3?  unconfirmed)
0x18  u32             0x210001  device ID
0x1c  u32             0x03800000 device version
0x28  u32             unix timestamp of the backup
0x30  char[64]        user comment, NUL-padded
0x70  ...             payload
```

The payload is **concatenated raw zlib streams**, back to back, no index or length prefixes — you
just inflate one and start the next where it ended. The file we were sent has 138, then two `\0`
bytes:

1. `#0` — globals JSON (`DSP`, `EQ`, `L6Link`, `System`, `Tuner`).
2. `#1`–`#128` — 128 IR slots as RIFF WAV (32-bit float, 48 kHz, mono).
3. `#129` — `schema: "L6UMDArchive"`, a 1,173-entry model-usage table.
4. `#130`–`#137` — the 8 setlists, `schema: "L6Setlist"` version 2, each `data.presets` an array of
   exactly 128 preset objects (empty slots have no `tone` key). Names: `FACTORY 1`, `FACTORY 2`, …,
   `USER 1`…`USER 3`, etc.

8 × 128 = **1024 preset slots** (363 populated in this backup).

**Implemented 2026-07-26: `fretwire_data::hxb`** parses the container — header, the stream walk,
setlists, presets and IRs — and `fretwire show-backup <file.hxb> [--presets]` prints it. Verified
against the contributor's real backup: 138 streams, 128 IRs, and

```
  [0] FACTORY 1    128/128 slots used      [4] USER 3       0/128
  [1] FACTORY 2    128/128 slots used      [5] USER 4       0/128
  [2] USER 1        63/128 slots used      [6] USER 5       0/128
  [3] USER 2         1/128 slots used      [7] TEMPLATES   43/128
```

363 populated, matching the count above. **The setlist order in this file is the `bank` numbering**
— bank 2 is `USER 1` and holds `Sludge` at index 17, exactly what a live read-info reply reported
(`PresetInfo { bank: 2, index: 17, name: "Sludge" }`). Two independent sources, so `Device::setlists`
is now [solid] rather than a guess off the unit's menu.

The tests use a **synthetic** `.hxb` built in-test, not the contributor's file — that backup is
personal device data and stays out of git.

**Reading only.** A preset inside a `.hxb` is a `tone` **JSON** object, not the MessagePack blob the
wire exchanges, so restoring one to the device needs a JSON→blob conversion that doesn't exist yet.

Note the globals JSON contains keys for *other* devices in the family (`P33FS3Function`,
`P36LastHomeView`) — it's a family-wide schema, not a Floor-specific one.

## What the captures don't have  [solid]

**Neither capture contains a single MI_00 control frame.** `tools/pcap-frames.py` returns 0 frames
on both, and that's correct, not a parser bug — the vendor interface (bulk `0x81`/`0x01`) has **no
traffic whatsoever** in either file.

The reason: **HX Edit was never running.** The contributor powered the unit on and operated the
front panel. The device does not broadcast parameter edits over the vendor channel unless an editor
has opened a session and subscribed; with no host talking to it, the control pipe stays silent.

What *is* in the captures, in full:

- Interface 4 (USB-MIDI), bulk `0x84` — a program-change burst per preset change, and nothing else:
  ```
  0b b0 00 00   CC#0   Bank Select MSB = 0
  0b b0 20 01   CC#32  Bank Select LSB = 1
  0c c0 50 00   Program Change 0x50
  ```
  Capture 1 shows one such burst (PC `0x50` = 80, one detent right). Capture 2 shows two (PC `0x4F`
  then `0x4E`, two detents left) — exactly matching the contributor's notes.
- Interface 5 (HID), int `0x85` — 12,776 / 16,330 polls, payload constant `00 00`. No signal.
- **The six knob sweeps, the joystick moves, and the model changes produced zero USB bytes.**

Useful side finding: preset selection *is* observable without the vendor channel at all, via
those Bank Select + PC messages on the MIDI interface.

## DSP2 = "Path 2", and how to spot it  [solid]

`dsp0` is **Path 1** and `dsp1` is **Path 2** — the two rows HX Edit draws. Confirmed by the
routing: across the 156 backup presets that use `dsp1`, the dominant wiring (128 of them) is
`dsp0.outputA @output: 2` feeding `dsp1.inputA @input: 0`, i.e. Path 1 → Path 2. Each DSP has its
own A/B branch on top of that (`@path` 0/1 plus its own `split`/`join` nodes), which is where the
familiar Path 1A/1B/2A/2B come from.

So **"is the second DSP used?" = "is there anything on Path 2?"** in HX Edit's flow view.

Two cautions when picking a test preset:
- **DSP2 use is common, not exotic** — 156 of the 363 populated presets in the backup touch `dsp1`.
- **A lone Looper is a weak signal.** 10 of those 156 have *only* an auto-placed `HD2_Looper` on
  `dsp1` (this is exactly the "Jail Breaker" case). Prefer a preset with several real Path 2 blocks.

### Preset numbering

Preset display labels are **4 per bank** with letters `A`–`D`: `bank = index/4 + 1`,
`letter = "ABCD"[index%4]` (globals `System.PresetNumbering: false`). Verified twice against the
contributor's own notes: `BAS:ParallelFuzz` at `FACTORY 2` index 80 → **21A** (they wrote "21a
BASSParallelFuzz"), and `Roundabout` at index 78 → **20C**, two detents left of 21A, matching their
second capture's description.

### Verbatim instructions for the contributor  [done — this produced `WinCap5`]

Kept as a record of what was asked, and as a template if another device ever needs the same
treatment. Don't ask anyone to identify "DSP2" or "Path 2" — name the block. In `FACTORY 1` `12B`
"Pull Me Under" every Path 2 block has a name that appears nowhere else in the preset:

> 1. Start the capture, then open **FACTORY 1, preset 12B "Pull Me Under"** in HX Edit.
> 2. Find the block named **`Cali Rectifire`** (it's an amp) and sweep its **`Drive`** knob from
>    minimum to maximum.
> 3. Then find the block named **`Hall`** (a reverb) and sweep its **`Decay`** knob the same way.
> 4. Stop the capture. Don't save.

`Cali Rectifire` and `Hall` are both on DSP2 — `Cali Rectifire` on its B branch, `Hall` on its A
branch — so between them they cover both sub-paths. Step 3 is optional but cheap.

*Outcome:* the contributor did both and then some, touching five DSP2 blocks across both branches
(they grabbed neighbouring knobs — `Mid` rather than `Drive`, `Predelay`/`LowCut` rather than
`Decay` — which cost nothing, since the block identity comes from the slot number and the parameter
identity from `28` + the stored value). The extra blocks turned one confirmation into five.

**Blocks to avoid naming:** `Simple Delay` appears **twice** in this preset (once on each of DSP2's
branches), so it's ambiguous. Everything on Path 1 (`Volume`, `Weeper`, `Deluxe Comp`, `Scream 808`,
`Jazz Rivet 120`, `70s Chorus`, `2x12 Jazz Rivet`) is the wrong DSP and tells us nothing new.

Full block list, for reference:

| DSP1 (Path 1) | DSP2 (Path 2) |
|---|---|
| Volume · Weeper · Deluxe Comp · Scream 808 · Jazz Rivet 120 · 70s Chorus · 2x12 Jazz Rivet | **Cali Rectifire** · Cali Q Graphic · 4x12 Cali V30 · Gain · Simple Delay ×2 · **Hall** · Plate |

### Best preset to request for the untested topologies

**Ask for `FACTORY 1` preset `12B` — "Pull Me Under".** One preset covers both gaps at once:
Path 1 has 7 blocks (2 on the B branch, `SplitAB`) and Path 2 has 8 blocks (6 on the B branch,
`SplitY`) — so it exercises DSP2, parallel paths on *both* DSPs, and both split types.

Fallbacks if that one won't open: `FACTORY 1` `22C` "RHETT'S ALLROUND" (9 blocks / 4 on B,
`SplitY`; 11 blocks / 3 on B, `SplitY`) or `FACTORY 1` `12A` "Justice Fo Y'all" (8 blocks / 1 on B,
`SplitAB`; 7 blocks / 4 on B, `SplitY`) — which sits in the same bank, one preset to the left.

## Live session captures (`WinCap3` / `Wincap4`, 2026-07-22)  [solid]

The second pair of captures **does** have HX Edit connected: 3,703 and 2,927 MI_00 frames, all three
channels (primary `ef03`, edit `ed03`, status `f003`) — the same channel IDs as the Stomp.

### The handshake is byte-identical to the Stomp's

All ten host→device bring-up frames from `device_handshake()` appear **verbatim** in both Floor
captures — the exact hex strings asserted in `tests/handshake_fidelity.rs`, matched byte for byte:

```
0c0000280110ef03000000020001002100100000   primary F1   ✓ both captures
110000180110ef030002000400100000010005000100000005000000   primary F2   ✓
…all 10 frames, primary + edit + status                   ✓
```

**`session::device_handshake()` needs no change for the Helix Floor.** Only the device's *replies*
differ, and only in the identity payload.

> **Trap:** the handshake's `arg` is `0x21000100`, which looks like the Floor's device ID
> (`0x210001`) shifted. It isn't — the **HX Stomp sends the identical constant**. It's a fixed
> protocol value, not a device identifier. Don't read device identity out of it.

### Identity reply (primary, cmd 0x04)

Both devices return 32 bytes: an 8-byte TLV header, an 11-byte NUL-padded model string, then 3×u32.

| | HX Stomp | Helix Floor |
|---|---|---|
| model string | `"P33Main"` | **`"P21"`** |
| u32 A | `0x003873D7` | `0x0030128B` |
| u32 B | `0x03800000` | `0x03800020` |
| u32 C | `0x10396611` | `0x07D01F5E` |

In the preset stream, key `7` reads `{36: "P21\0", 35: 58720288 (0x03800020), 37: "7d01f5e\0"}` —
so the Floor's **model code is `P21`** (Stomp: `P33`). Note key `37`, the "firmware string", is a
bare build sha on the Floor (`7d01f5e`) where the Stomp gives `v3.71-32-g1039661`; our CLI prints
`firmware 7d01f5e`, which is faithful, not a decode error. That sha is also u32 C above
(`0x07D01F5E` → the digits `7d01f5e`) and the value at offset `0x20` of the `.hxb` header.

### Our parser reads Floor presets  [solid]

`tools/extract-preset-stream.py` (new — reassembles chunked read-streams; self-checked by
regenerating `captures/preset1_stream.msgpack.bin` byte-identically from the Stomp capture)
recovered five preset streams, 7,277–7,545 bytes each versus the Stomp's ~2,800.

`fretwire show-preset` parses them **unmodified**: model names, parameter names and values, amp+cab
pairing, DSP-load percentages, footswitch bindings (`FS8`…`FS13`), and all **8 snapshots**
(`SNAPSHOT 1`…`SNAPSHOT 8` — the Stomp's 3-snapshot assumption did not get in the way).

Every value was cross-checked against the same presets in the `.hxb` backup, which is independent
ground truth. After the two fixes below, **all three presets reconcile exactly** — block for block,
parameter for parameter.

### Two parser gaps found by that cross-check  [both fixed 2026-07-23]

Both were small, precisely characterised, and **neither was really Floor-specific** — they were
general gaps the Floor happened to expose. Both are now implemented; full detail in
`docs/preset-format.md`.

1. **Slot `type 7` = a Looper block, and we skip it.** Enumeration only accepts `type 6`, so the
   Looper vanishes. Its content shape differs: model index at key **`8`** (not `24 → 25`), params at
   key **`7 → 4`** (not `11 → 4`), enabled at `10` (same). None of our Stomp fixtures contain a
   Looper, which is why this never surfaced — a Stomp preset with one would very likely hit it too.
2. **Preset key `1` is the second DSP's slot array, and we ignore it.** It has the same
   `{21: split, 22: Array[20]}` shape as key `0`; it is `nil` on the Stomp, which is why
   `docs/preset-format.md` recorded it as "`nil` | ?". The slot array is **still 20 entries** on the
   Floor — the device does not widen it, it adds a second one. Reading only key `0` silently drops
   every DSP2 block.

Worked example — factory preset **"Jail Breaker"** (`FACTORY 2`, index 72). The `.hxb` says
`dsp0: 8 blocks, dsp1: 1`. We reported 8. Walking key `1` finds the missing block: a
`HD2_LooperStereo` at DSP2 slot 8 — which is *also* a type-7 slot, so it needed both fixes.
Re-checked after the fix: that preset now reports **9** blocks, the ninth being
`6 Switch Looper [HD2_Looper] Stereo` at wire slot **28** (DSP2 index 8), matching the backup.

### Unrelated bug spotted in passing — FIXED (2026-07-25)

Legacy (non-`CabMicIr_*`) cab models mislabeled their parameter list. `HD2_Cab2x12MailC12Q` has
params `@mic, Distance, LowCut, HighCut, EarlyReflections, Level, @enabled`; we printed the five real
values correctly but labeled the trailing mic index as `Trails`. The Stomp's `CabMicIr_*` cabs label
fine, so this was a **legacy-cab-family issue, not a Floor issue** — any Stomp preset using an
old-style cab reproduced it. Fixed: `editor::name_params` takes the model category and names the
trailing extra `"Mic"` for cabs (categories 2/19), `"Trails"` only for time-based fx.

### The write path works on the Floor, byte-for-byte  [solid]

These captures **were** driven from HX Edit's UI, and they contain the full write path. Our existing
builders in `fretwire_protocol::edit` reproduce the Floor's bytes **exactly** — 9 of 9 ops checked
against the raw TLVs pulled from the captures:

| op | builder | example from the capture |
|---|---|---|
| 30 | `set_value` | slot 5, param 1 → `0.8`; and a full sweep `0.51 → 0.70` on slot 3, param 0 |
| 41 | `bypass` | slot 1 → off, slot 6 → on |
| 78 | `begin_structural` | slot 8, slot 5 |
| 40 | `swap_model` | slot 8 → `HD2_ReverbOctoStereo` (244); slot 5 → `HD2_TremoloHarmonicMono` (318); slot 5 → `HD2_AmpBritJ45Nrm` (12) paired with `HD2_Cab4x12Greenback20` (65) |
| 71 | `save_preset` | bank 1, slot 64, name `"Sultans"` — a real write to flash |
| 20 | select-preset | `{107: 1, 108: 72}` |

The envelope shapes are **identical to the Stomp's**, key for key:
`op 30` = `{98: slot, 29: true, 26: 0, 28: param_index, 119: value}`,
`op 41` = `{98: slot, 59: bool}`,
`op 40` = `{98: slot, 100: {23: stereo, 25: model_index, 26: paired_index}}`.

Combined with the byte-identical handshake, **the Floor needs no protocol changes at all** — read
*or* write. The remaining Floor work is purely in the preset *model* (slot type 7, DSP2) and in
device matching.

> Earlier revisions of this document claimed these captures were front-panel-driven and contained
> no writes. That was wrong — an inspection truncated at 40 of 63 envelopes, with every write op
> falling in the untruncated tail. The contributor's account was correct.

### The one real unknown left: how does an edit address a **DSP2** block?  [was open]

Every write we'd observed targeted a block by key `98` = **a bare slot number**, with no DSP
qualifier anywhere in the envelope (key `26` is already known — main vs. paired cab, `MODEL_MAIN`/
`MODEL_PAIRED`). But the Floor has **two slot arrays, both 20 entries long**.

"Jail Breaker" made the ambiguity concrete — it has a block at slot 8 on *each* DSP:

| | slot 8 |
|---|---|
| DSP1 (preset key `0`) | `HD2_ReverbRoomStereo` |
| DSP2 (preset key `1`) | `HD2_LooperStereo` (type 7) |

The capture's `op 78 {98: 8}` + `op 40 → model 244` swapped the **DSP1** Room reverb for
`HD2_ReverbOctoStereo`. So `98: 8` resolved to DSP1, and we had never seen a DSP2 block addressed at
all. Three hypotheses were on the table: global slot numbers; an elided extra key; or a context/mode
op selecting the active DSP. `WinCap5` settles it — **hypothesis 1**.

## DSP2 addressing: slot numbers are global  [solid — `WinCap5`, 2026-07-23]

**`slot = dsp * 20 + index`.** DSP1 occupies slots **0–19**, DSP2 slots **20–39**. Nothing else in
the envelope changes: `op 30`/`op 41`/`op 78` on a DSP2 block are byte-for-byte the same shape as on
a DSP1 block, `26` stays `0`, and no context op is involved. Op `33` remains unidentified but is
irrelevant to this — it never appears in this capture.

`WinCap5` is a single HX Edit session on `FACTORY 1` `12B` "Pull Me Under" (`op 20 {107: 0,
108: 45}`, closed by `op 71 … 'Pull Me Under\0'`). It touches **five DSP2 blocks and nothing else**,
and every `op 78` slot lands where the global rule predicts:

| wire `98` | = dsp·20+i | block at `1 → 22 → [i]` | swept param | first value on the wire | value stored in the preset |
|---|---|---|---|---|---|
| 28 | 1·20+8 | `HD2_ReverbHallStereo` | `28: 2` → `LowCut` | `84.0` | `83.0` |
| 28 | | | `28: 1` → `Predelay` | `0.1` | `0.1` |
| 33 | 1·20+13 | `HD2_AmpCaliRectifire` | `28: 2` → `Mid` | `0.41` | `0.4` |
| 34 | 1·20+14 | `HD2_CaliQMono` | `28: 1` → `240Hz` | `-3.0` | `-3.1` |
| 37 | 1·20+17 | `HD2_DelaySimpleDelayMono` | `28: 1` → `Feedback` | `0.2` | `0.19` |
| 38 | 1·20+18 | `HD2_ReverbPlateStereo` | — (`op 78` only) | — | — |

Each sweep's **first** value sits one UI increment off the value stored in the preset — five
independent confirmations, on five different models with five different parameter scales (dB, Hz,
0–1). No other assignment of slot numbers to blocks reproduces that.

Two corroborating details:

- **The duplicate is disambiguated.** This preset has `HD2_DelaySimpleDelayMono` twice on DSP2 — at
  index 7 (branch A) and index 17 (branch B). The wire says `37`, i.e. index 17, the branch-B one.
  A per-DSP scheme would have had to say `17` for both.
- **It is consistent with everything prior.** `Wincap4`'s slots were `1`, `3`, `5`, `6`, `8` — all
  < 20, all DSP1, including the Jail Breaker `98: 8` that resolved to DSP1's Room reverb. Under the
  global rule that's exactly right, so nothing has to be reinterpreted.

The 20-slot array itself is unchanged from the Stomp — the device does **not** widen it, it adds a
second one. Verified against all three tracked Stomp fixtures, which have the same 20 entries with
the same structural nodes at indices 0/9/10/19 and `dsp1 = nil`.

### What this means for us

Addressing does **not** need to become a `(dsp, slot)` pair on the wire — a single integer still
works, exactly as `fretwire_protocol::edit` already emits it. The `(dsp, index)` split is purely an
internal detail of walking the preset, and it converts to the wire slot with one multiply-add. That
removes the design question that was blocking "walk preset key `1`" — see `ROADMAP.md` Phase 9.

## Next steps

All three original blockers — "get a connected capture", "verify the handshake", "work out DSP2
addressing" — are **done**, and **no further captures are needed**.

The preset-model work is done too (2026-07-23): slot `type 7` is enumerated, both slot arrays are
walked, and slots are global throughout. Confirmed end-to-end against the captures — "Pull Me Under"
now decodes all 15 blocks across both DSPs with correct rows, per-DSP load and footswitch bindings,
where it used to show only DSP1's 7. `fretwire_protocol::edit` needed no change.

**Device matching is done too.** `fretwire_protocol::Device`/`DEVICES` describe each device (PID,
model code, preset `device` ID, DSP and snapshot counts, and a `Support` flag), and
`Transport::open` matches any of them rather than hardcoding the Stomp's PID. The Floor is listed as
`Verified`; `Session::device()` tells the layers above what they're talking to. **So a Floor will
now connect.** What remains:

1. **Teach `Session`'s routing planner about DSPs.** ✅ Done (2026-07-25) — `add_block_at`,
   `place_block`, `insert_block`, `reorder_block` and `set_node_pos` all plan in wire space and take
   the DSP from the slot; cross-DSP moves are rejected.
2. **Render two DSPs in the routing grid.** ✅ Done (2026-07-25) — `Chain.svelte` draws one grid per
   DSP from `PresetDto.dsps[]`, and the routing planner is DSP-aware (drag/insert/node-move on either
   DSP). The two-DSP browser mock ("Pull Me Under") makes it testable without hardware.
3. **`.hxb` reading** — ✅ Done (2026-07-26). `fretwire_data::hxb` + `fretwire show-backup`; see
   the container section above. Restoring *from* a `.hxb` is still open (the presets inside are
   `tone` JSON, not wire blobs).
4. **Fix the legacy-cab parameter labels** — ✅ Done (2026-07-25). The trailing extra value is now
   named `"Mic"` for cab categories (it's the mic index) and `"Trails"` only for time-based fx; see
   `editor::trailing_extra_name`.

## First hardware run (2026-07-26)  [solid]

fretwire has now been run against a physical Helix Floor by a contributor. It connects, handshakes
(`device reports "P21"`), reads presets across both DSPs, edits, and holds a session without
crashing. Three things came out of it.

### Preset-stream reassembly — FIXED

Reads truncated at a multiple of 256 whenever an empty chunk landed mid-stream and was mistaken for
the terminator. The stream envelope's declared length (`marker:u16, type:u16, len:u32le`, total =
`len + 8`) is authoritative; `fretwire_data::stream::declared_stream_len` now drives reassembly and
short chunks before that length are skipped rather than treated as EOF.

### Row-B grid columns — FIXED

Blocks on a parallel path rendered *outside* the Y-loop bracket. `dsp_grid` derived a row-B cell's
column as `slot − split_idx + 1`, where `split_idx` is the split node's slot-array index (always
10) — so row B was pinned to columns 2..=9 and never consulted the split node's signal-flow
position, which is where the glyphs are drawn. The 20-slot array is
`[0=in, 1..=8 row A, 9=out, 10=split, 11..=18 row B, 19=mixer]`: **both rows are 8 columns**, so a
row-B column is `slot − 10`, in the same absolute space as row A.

> **Still needs confirming on hardware.** The fix is forced by the 20-slot arithmetic and matches
> both split fixtures, but every fixture we have holds only *one* row-B block. A `dump-raw` of a
> Floor preset with several (e.g. `BMBLFOOT PRINCE`, row B at slots 13/14/15) would close it out.
> Those are Line 6 factory presets — keep any such dump local and gitignored, diagnosis only.

### Setlists — IMPLEMENTED

The Floor has eight setlists; we only ever browsed bank 0, so a unit sitting in User 1 listed
Factory 1's names. `edit::presets_stream` had the bank hardcoded to 0. Now parameterised, with
`Device::setlists` naming them and the sidebar picking between them (hidden on a one-setlist
device). Confirmed from traffic: `PresetInfo { bank: 2, index: 17, name: "Sludge" }` for a preset
the user had selected in **User 1** — so `Factory 1 = 0, Factory 2 = 1, User 1 = 2`. The rest of the
order is read off the unit's PRESETS menu [hypothesis].

### Active snapshot can be wrong — FIXED 2026-07-26

The GUI highlighted snapshot 5 while the unit was on snapshot 1. The decoder is *not* at fault: the
preset blob's key `10 → 8` is the snapshot that was **stored** with the preset (dual_amp's fixture
reads 1, having been saved on SNAPSHOT 2), and the device has a global snapshot-recall preference,
so the live selection can differ from the stored one. A panel-side change reaches us only as a
status push (type 42/46), which we already apply. What's missing is a way to *query* the live
snapshot on connect — that needs a capture of HX Edit connecting to a unit parked on a non-default
snapshot. `read_preset` now logs the stored value at debug level to help correlate.

**Decoding the snapshot bypass matrix (key `10 → 10 → [i] → 3`) turned this from a guess into a
measurement.** Each snapshot stores one `[_, enabled]` pair per slot — the scene it recalls. In
`preset1_stream` the live blocks match snapshot 0's row and key `8` says 0, so both agree. In
`dual_amp_stream` key `8` says **1**, but the live block state is snapshot **0**'s scene. So the
stored index and the stored scene genuinely disagree in a fixture we already had, offline, with no
hardware involved — the same failure mode Sean saw. Both facts are pinned by tests
(`snapshot_matrix_matches_the_live_block_state`, `dual_amp_stored_active_snapshot_disagrees_with_its_scene`).

**Resolved: the device tells us, and we were discarding it.** An HX Stomp parked on SNAPSHOT 3
reported a stored index of **0**, refuting "key `8` is the live snapshot" outright. Scene-matching
was ambiguous on that preset (snapshots 2 and 3 held identical scenes), so it can't carry the fix
either — but decoding the **op-23 read-info reply** in full showed the answer was already on the
wire:

```
104: {107: 0, 108: 20, 109: "Dual Amp\0", 117: true, 83: [8850, 0], 92: 0}
```

**Key `92` is the live active snapshot** — the same key a snapshot status-push carries
(`{105:42, 106:{92:n}}`). We parsed only 107/108/109 and dropped the rest. In the `Dual Amp`
capture key 92 reads 0, matching that preset's live block *scene*, while its blob stores 1 — three
independent signals now agreeing.

`PresetInfo::snapshot` carries it, and `Session::read_preset` overrides the blob's stored value with
it (falling back to the blob for offline decodes, which have no device to ask). Keys `117` (bool)
and `83` (`[u32, 0]`) in that reply are still unidentified.
