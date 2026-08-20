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

> **Confirmed on hardware 2026-07-29** by the tester's `pullmeunder` dump — the first preset we
> have with more than one block on a parallel path. It puts six on DSP2's row B (wire slots
> 33..=38 → columns 3..=8) and two on DSP1's (11/12 → columns 1/2), all inside the 8-column grid,
> and its snapshots only read as coherent scenes under this mapping. The `slot − 10` arithmetic
> holds per DSP. See "A split can span both DSPs" below for the part it does *not* settle.

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
hardware involved — the same failure mode the tester saw. Both facts are pinned by tests
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

## Second hardware round (2026-07-29): eight bank listings + a multi-row-B preset

The tester sent a `dump-list` of all eight banks and a `dump-raw` of `Pull Me Under` (FACTORY 1
slot 45), which between them close two of the three questions that were open.

### Preset numbering is settled  [solid]

All eight listings parse: 128 entries each, 1024 total, every index decoding to its own bank under
`global = bank × 128 + slot`. Cross-checked against the same unit's 2026-07-22 `.hxb` backup, the
listings agree on **1021 of 1024** slots — and the three exceptions are not a numbering error at
all but presets the tester has since *moved*:

| bank | preset | backup slot | device slot |
|---|---|---|---|
| 0 FACTORY 1 | `InSTANtgH0St/24` | 101 | 68 |
| 0 FACTORY 1 | `Parallel Muffs` | 108 | 107 |
| 1 FACTORY 2 | `BAS:FunkIfIKnow` | 84 | 95 |

A `difflib` sequence diff of each bank against the backup reports exactly one insert + one delete
per move and "equal" for every other run. USER 1 (63/63), USER 2 (1/1) and TEMPLATES (43/43) match
the backup **exactly** — those are the banks a base offset would have exposed, so the earlier
"index drift" is now conclusively closed as the tester's own device state, not our parser.

> This retires the *numbering* half of the `FRETWIRE_SETLISTS=1` gate, and the gate was split to
> match: **browsing setlists is now ungated**, while a flash write into a setlist the device isn't
> in still needs the flag. The remaining reason is the Floor lockups, still unexplained (see the
> INCIDENT entries in `STATUS.md`) — nothing here bears on those, and the cross-setlist *write* path
> has still never run against a Floor.

### The device does not list presets in slot order  [RETRACTED 2026-08-19 — it does]

> **This section was wrong, and the "fix" it describes was the bug.** Kept in full because the
> observation underneath it is correct and still load-bearing; only the inference was backwards.
> The correction is below.

The observation, which stands: on the tester's Floor, three rows arrive out of step with their own
map key — bank 0 emits key 68 at stream position 101 and key 107 at position 108, bank 1 emits key
95 at position 84. The other banks arrive strictly ascending, which is why it went unnoticed.

The inference, which does not: this read the **key** as the preset's current index and the stream
position as stale, on the strength of a `difflib` diff against the same unit's 2026-07-22 `.hxb`
backup — assuming the tester had moved those presets *after* taking it. `Session::list_presets_in`
was changed to sort by the key.

It is the other way round. **The array position is the slot; the key is the preset's index before
it was last reordered.** Three independent sources agree, and the moves predate the backup:

* **HX Edit's own listing of the same unit** (`WinCap5.pcapng`, the Windows capture the tester sent
  on 2026-07-23), reassembled whole with `tools/extract-preset-stream.py --list`. Compared
  entry-by-entry against his `.hxb`: in **stream order** all 128 rows of FACTORY 1 and all 128 of
  FACTORY 2 name the preset the backup holds at that position. Sorted by key they do not — 36 rows
  of FACTORY 1 and 12 of FACTORY 2 carry a key that is not their slot (34 displaced by the 68 → 101
  move, 2 by the 107/108 swap, and 12 by FACTORY 2's 95 → 84 move). HX Edit renders the list
  positionally and ignores the key. Pinned by `crates/fretwire-data/tests/preset_list_order.rs`,
  which skips when the contributor's files are absent.
* **The device's own op-23 identity.** All 38 bank-0 identities across the field logs match the
  backup slot, i.e. the position. `BULB RHYTHM` reports index 71 live; its key is 72, and the
  pre-sort screenshot (`from_redditor/bmblfootPrince01.png`) labels that row `072`.
* **The shape of the anomaly is a move, not a renumbering.** Bank 0's second anomaly is a bare
  adjacent transposition — `…106, 108, 107, 109…`, one click of knob 1. The first is the same
  operation applied 33 times: key 68 lifted out and reinserted after key 101, everything it passed
  shifted one place. Keys stay welded to their preset while the order moves around them.

There is no "UI order" field to consult, either: across 236 clean rows the inner map is
`{109: name, 123: false, 124: false, 125: 0}` — the order *is* the array order.

**What the key is.** The preset's index before it was reordered [solid], stable across a week,
many sessions, power cycles and HX Edit's own backup run. Most likely its physical storage slot,
with the displayed order a permutation table laid over it — reordering then costs a table write
instead of relocating a ~7 KB blob [hypothesis]. Against that reading: no command on this protocol
accepts the key as an address, so it may just be an implementation detail leaking into the wire
format. Three cheap experiments would settle it, all on a device with reordered presets: move one
preset twice and dump the listing after each (physical slot → the key never changes; stale index →
it follows); save a preset after moving it and re-dump; restore a backup over a reordered setlist
and see whether the keys come back identity-mapped.

**Fixed 2026-08-19.** `parse_preset_list` numbers rows by their position in the stream and returns
`PresetListEntry { slot, key, name }`; the sort is gone. The key never leaves `fretwire-core` — it
feeds the `reordered` flag in the `preset listing parsed` log line and a footnote on
`fretwire presets`, and nothing else, so it cannot be mistaken for an address again. Reported by an
HX Stomp XL user who reorders presets on the pedal; the mechanism is device-independent.

### The snapshot bypass matrix is one flat 40-entry array  [solid] — FIXED

`10 → 10 → [i] → 3` spans the **whole device**, indexed by wire slot (`dsp × 20 + index`), not one
20-entry array per DSP. `pullmeunder` reports `block_enabled.len() == 40`.

`show-preset`'s snapshot diagnosis was indexing it by the per-DSP index, so every DSP2 block
silently reported DSP1's state — which made all eight of this preset's snapshots look nearly
identical and the live scene match none of them. Indexed by wire slot they resolve cleanly:

```
[0] Intro   1+ 2+ 3+ 4+ 5+ 11- 12+ 27+ 28+ 33+ 34+ 35+ 36- 37- 38-   <- matches the live scene
[2] Solo    1+ 2- 3+ 4- 5+ 11- 12+ 27- 28- 33+ 34+ 35+ 36+ 37+ 38+
```

— i.e. Intro runs the clean path's delay/reverb (27/28) with the gain path's (37/38) off, and Solo
the reverse. Musically coherent, and unique: exactly one snapshot matches.

**This is also a second, independent confirmation of the key-92 finding.** The blob stores active
index **4**; the live scene is unambiguously snapshot **0**. Different preset, different device
state, same disagreement the `Dual Amp` capture showed — so overriding the blob with the op-23
reply's key 92 is right.

### A split can span both DSPs  [hypothesis]

`pullmeunder` is one Y-loop stretched across the whole unit: common Volume + Deluxe Comp, then a
clean leg (Jazz Rivet 120 → 70s Chorus → cab, continuing onto DSP2 as Clean Delay + Clean Verb) in
parallel with a gain leg (Weeper → Scream 808 on DSP1's row B, continuing as Cali Rectifire → Cali
Q → 4×12 → Gain → Gain Delay + Gain Verb on DSP2's row B), rejoining at the end.

Both DSPs report `is_split() == true`, and the bracket ends split across them:

| DSP | split pos (kind 2) | mixer pos (kind 3) |
|---|---|---|
| 1 | 2 | **0** |
| 2 | **0** | 9 |

The `0` appears on whichever side does not hold that end of the bracket. So the
"common-before / path A / common-after" rule on `structural_node_pos` — which assumes both ends are
on the same DSP — does **not** hold here, and neither does the
`split_pos ≤ column < mixer_pos` invariant asserted by `row_b_cells_sit_inside_the_split_bracket`
(all our fixtures are single-DSP-bracket presets, so it still passes).

Tagged `[hypothesis]`: one preset, and a `0` could equally be an absent-value default rather than a
sentinel. **Nothing has been changed in the grid code on the strength of it** — reasoning ahead of
data is what produced the original row-B column bug. What would settle it is a screenshot of this
preset's routing grid in HX Edit next to ours, or a second cross-DSP preset to compare.

## Third hardware round (2026-07-30): two lockups, both root-caused

Nine emails: eleven GUI screenshots, four `RUST_LOG` captures, one photo of the pedal's own screen,
and one saved preset blob. Two sessions ended with the Floor dropping off USB.

### The device does **not** range-check parameter writes  [solid]

This is the important one, and it invalidates an assumption the code had written down.

Both lockups have the same shape: an edit is ACKed, the next read fails three times with growing
gaps, and then every URB comes back `No such device` — the pedal has reset itself off the bus. The
second is pinned exactly, because its ACK echoes the edit body back:

```
2:04:35.2281  edit ACK  {102: 65, 103: 0, 104: {98: 22, 29: true, 26: 0, 28: 3, 119: 31}}
2:04:39.0749  edit ACK  {102: 71, 103: 0, 104: {98: 22, 29: true, 26: 0, 28: 2, 119: 77}}
2:04:42.1061  WARN preset read/decode failed … attempt=0
2:04:45.3478  WARN preset read/decode failed … attempt=1
2:05:08.0670  WARN preset read/decode failed … attempt=2      <- 23 s gap; the pedal is gone
```

Wire slot 22 in that preset (`Massif`, and its saved copy `Massif2`, which is the attached blob) is
`HD2_DL4Multihead`. Param 2 is `Heads 1-2` and param 3 is `Heads 3-4` — **integer enums with
`min: 0, max: 3`**. They were sent `77` and `31`.

So an out-of-range integer does not get clamped, ignored, or NAKed. It is ACKed, and then the DSP
goes down hard enough to take the USB device with it. Integer params index tables in the firmware,
which is the obvious way for this to end in an out-of-bounds read.

The first lockup has the identical signature (edit → three failed reads → `No such device`), but its
ACK carries no echoed body, so *which* write did it is unproven. Same shape, weaker evidence.

### Why an out-of-range value was sendable at all  [fixed]

Three compounding causes, fixed at each layer:

1. **The metadata was missing.** `param_meta_from` keyed the table by the `.models` `symbolicID`,
   but a block looks meta up by its **variant-stripped** id (`load_preset` runs the device symbol
   through `split_variant` first). Eight models — the legacy DL4 delays, and only those — are named
   in the reference data solely in their suffixed form (`HD2_DL4MultiheadStereo`), so they resolved
   to *no metadata at all*: no range, no `value_type`, no enum labels. The table now also aliases
   the stripped base, never overwriting an exact entry.
2. **The editor invented a range.** With no `max`, `ParamPanel.svelte` fell back to a `0..=127`
   span for integer params. On a `0..=3` selector that is ~97 % illegal travel. An integer with no
   declared range is now shown read-only rather than guessed at — a value we cannot bound is one we
   have no business sending.
3. **Nothing checked below the UI.** `Session::clamp_param` now bounds every parameter write by the
   model's declared range, on the float, paired and enum paths alike. It costs a decode we already
   pay for edit labeling. A guard that only holds while every model resolves is not a guard.

With (1) fixed, `Heads 1-2` picks up its `heads12` control from `HelixControls.json` and renders as
a four-option dropdown (`1-2 Off / HD1 On / 1-2 On / HD2 On`), so the crash value is now unreachable
by construction rather than merely clamped.

Regression test: `editor::tests::legacy_dl4_ranges_survive_the_variant_suffix`.

### Our grid matches the pedal's own screen  [solid]

The photo of `15D RC REINCARNATION` on the Floor's display, next to our render of the same preset,
agrees on every structural point: eight blocks on path 1, exactly one block on the parallel row
positioned under the later part of the chain (we place it at wire slot 16 → row B column 6), and
**path 2 empty** (we show DSP 2 at 0.0 %). This is the first direct check of our routing grid
against the device's own drawing of it, rather than against another decode of the same blob.

### The cross-DSP split is real, and we render it correctly  [solid]

**A photo of `Pull Me Under` on the pedal's own screen settles the 2026-07-29 `[hypothesis]`, and
in our favour.** The Floor draws four rows: path 1 row A (5 blocks) and path 1 row B (2 blocks)
each ending in a *route-to-path-2* arrow, then path 2 row A and path 2 row B, merging into the
output at the right. So path 1 genuinely **has a split and no mixer**, and path 2 genuinely **has a
mixer and no split** — the `0` really is "this DSP does not hold that end of the bracket", exactly
as the hypothesis guessed, and not an absent-value default.

Our render agrees block for block: 5 on DSP1 row A, 2 on DSP1 row B (`Weeper` bypassed), `Clean
Delay`/`Clean Verb` at DSP2 columns 7–8, `Cali Rectifire`/`Cali Q`/`4x12` at DSP2 row B columns 3–5
with `Gain`/`Gain Delay`/`Gain Verb` bypassed at 6–8, split on DSP1, mixer on DSP2.

The only cosmetic gap: we terminate DSP1's row A with an `OUT` cap, where the device shows an arrow
meaning "into path 2". Ours isn't wrong so much as less informative.

> **Correction.** An earlier draft of this section read the same screenshots as showing that *every*
> preset whose row B continues onto DSP2 renders a broken bracket. That was wrong: it treated "no
> mixer on DSP1" as evidence of a missing node, when the hardware shows there is genuinely no mixer
> there. `Pull Me Under` and `AUS Flood` are correct renders — and so, it turned out a day later, is
> `Waters in Hell` (below).

Node positions read off the eleven renders:

| Preset | DSP1 split / mixer | DSP2 split / mixer | row B (DSP1 / DSP2) |
|---|---|---|---|
| RC Reincarnation #59 | 5 / 7 | — (DSP2 empty) | 16 / — |
| Massif2 #4 | 0 / 7 | — | 11–14 / — |
| Trademark #41 | 1 / **none** | **none / none** | 12 / 33,34,36,37,38 |
| Justice Fo Y'all #44 | 1 / **none** | **none / none** | 16 / 33,34,35,38 |
| AUS Flood #42 | 3 / **none** | none / 7 | 14 / 35,36 |
| Pull Me Under #45 | 1 / **none** | none / 9 | 11,12 / 33–38 |
| Waters in Hell #56 | 6 / 7 | **1 / 2** | 17 / 36,37 |

Two shapes are now confirmed against the device's own display: bracket entirely on DSP1
(RC Reincarnation) and bracket split across DSPs (Pull Me Under). `AUS Flood` is the same shape as
the latter and needs no separate confirmation.

`Trademark` and `Justice Fo Y'all` draw **no mixer on either DSP**. Given what the Pull Me Under
photo establishes — that a path's rows can leave without merging — this is plausibly correct too:
path 2's two rows can end at different physical outputs and never join. Unverified, but no longer
suspicious on its face.

#### `Waters in Hell` is correct too — the empty bracket is real  [solid]

This was the last render we could not explain: DSP2 draws a split at column 1 and a mixer at column
2 — a one-column bracket containing nothing — while that DSP's own row-B blocks sit at columns 6 and
7, outside it. A bracket that excludes every block on its own row looked impossible, and it read as
the `split_pos ≤ column < mixer_pos` invariant failing in the wild.

The pedal photo (2026-07-30) says otherwise. Measuring the screen against its own column pitch
(~212 px, origin at the input) puts every node exactly where we draw it:

| | device | ours |
|---|---|---|
| DSP2 split | between col 1 and 2 | between col 1 and 2 |
| DSP2 mixer | between col 2 and 3 | between col 2 and 3 |
| DSP2 row A | cols 4, 5 (`Parametric` ×2) | cols 4, 5 |
| DSP2 row B | cols 6, 7 (`Simple Delay`, `Spring`) | cols 6, 7 |
| DSP1 split | between col 6 and 7 (after `5150`) | between col 6 and 7 |
| DSP1 mixer | after the cabs | after col 7 |

So the invariant is simply not one. **The mixer can sit to the *left* of blocks on its own row B**,
and the device handles it by drawing a wrap-around return line: row B runs right past the mixer
column to the end of the lane, turns up, and runs back left to the mixer. Two horizontal mid-lane
segments in the photo — one from the split wrapping left into the start of row B, one from the mixer
running right to meet row B's tail — are that wrap, and they are what make the bracket look empty.

We render the nodes at their true columns and don't draw the wrap. That is a legibility gap, not a
correctness one, and the same one we already have at DSP1's `OUT` cap.

> **Every render the tester has sent is now confirmed correct.** No routing-layout bug remains open.
> The `[hypothesis]` that opened this section — that a `0` node means "not on this DSP" — is closed
> in our favour on two independent presets.

### The freeze is partly ours: reads had no wall-clock bound  [fixed]

The chunk loop bounded the number of requests but not the time. Against a device that is still
enumerated but no longer answering, each request burns the full 3 s bulk-IN timeout, so a ~7.4 KB
preset costs ~36 × 3 s — and `read_preset` retries three times on top. The measured gap inside a
single attempt was **121 s**, which is what the tester experienced as "the GUI froze": the pedal was
already gone and we spent minutes discovering it one timeout at a time.

`read_preset_inner` now carries a `READ_DEADLINE` (10 s — a healthy full read is ~20 ms, so this can
only fire on a device that has genuinely stopped) and returns a message naming how far the stream
got. Worst case for a dead device drops from ~6 minutes to ~30 s, with an error instead of a hang.

### Listing order confirmed live  [solid]

`preset list normalised to slots bank=0 base=0 n=128 reordered=true` appears in his log, with
`bank=2 … reordered=false`. That is exactly the prediction from the 2026-07-29 dump — Factory 1
carries moved presets whose stream position no longer matches their index, and the user setlists do
not. The sort in `list_presets_in` is doing real work on this hardware.

### Logging gap closed

`preset read/decode failed` logged the attempt number but not the error, so the only line that
matters in a remote tester's log couldn't distinguish a decode fault from a device that had stopped
answering. It now includes the error.

## Fifth round (2026-07-30, evening): two clean sessions, two new bugs

Two full GUI sessions on the Floor, one on the pre-fix build and one on the build carrying the
clamp/deadline fixes. **Neither locked up.** Both closed cleanly (`session closed — pedal returned
to standalone`), and across ~9 minutes of connected time the pre-fix log carries *zero* `WARN` and
*zero* `ERROR` lines while the post-fix log carries one — the already-understood benign
`empty chunk before declared stream end … empties=1`, which the skip logic absorbed (`reassembled
preset stream bytes=8118 declared=8118` immediately after).

This is the first Floor session to survive start to finish. It does not yet clear the
`FRETWIRE_SETLISTS` gate — no cross-setlist write was attempted — but it removes the standing doubt
about whether a Floor can hold a session at all.

Neither log fired `clamp_param` or `READ_DEADLINE`, which is expected: he did not touch a legacy DL4
delay, and nothing wedged. The fixes are unexercised, not disproven.

### The op-23 identity can lag *past* the stream, not just up to it  [solid] [fixed]

The known behaviour was that the identity lags the blob by one preset, and the mitigation was to
re-ask **after** the stream on the assumption that the second answer is fresher. The evening log
shows the assumption is too weak. Selecting `Pull Me Under` (FACTORY 1 #45) from `WATERS IN HELL`:

```
03:44:27.790  read-info … PresetInfo { bank: 0, index: 56, name: "WATERS IN HELL", snapshot: Some(0) }
03:44:27.890  reassembled preset stream bytes=8118 declared=8118     ← Pull Me Under's size, not #56's 7402
03:44:27.899  post-stream read-info reply after=…{ bank: 0, index: 56, name: "WATERS IN HELL" }
...
03:44:28.165  read-info … PresetInfo { bank: 0, index: 45, name: "Pull Me Under", snapshot: Some(0) }
```

Both identity reads are stale, so `before == after` and the existing settled-check passes a blob
labelled with the wrong preset. It corrected itself 370 ms later only because the GUI happened to
read again. The earlier session has the same fault with the fields lagging *independently* — a read
reported `bank: 1, index: 3, name: "Cali Rectifire"`, where the name belongs to FACTORY **1** slot 3
and only the bank was stale — which rules out treating the reply as atomically old-or-new.

The consequence is not cosmetic. The active snapshot comes from the same reply (key 92), so a stale
identity paints the previous preset's snapshot onto the new preset's chain, and the header and
sidebar highlight point at a preset the user is not editing.

**Fix:** `goto_preset` now records the `(bank, index)` it asked for, and `read_preset` re-reads until
the reported identity matches it. Comparing two device answers cannot detect this; comparing against
the address *we chose* can. The expectation is consumed by the first `read_preset` after the goto, so
a user turning the knob on the pedal costs one read its retries and nothing after that.
`identity_confirms` carries the regression test, built from both log cases.

### The status line never refreshes  [fixed]

`Connected — N blocks. Session held for editing.` is set once at connect and then left alone, so it
describes whatever preset happened to be loaded at the time forever after. Both rounds of
screenshots caught it: `Waters in Hell` (9 blocks) captioned "15 blocks" on the earlier build, and
`Pull Me Under` (15 blocks) captioned "9 blocks" on the later one — the counts are each correct for
the *other* preset. `Saved to slot 7.` has the same problem and is worse, since it sat on screen
over a preset in a different setlist.

The status line now re-states itself whenever the loaded preset's identity changes, from whichever
path moved it. Transient messages still persist while you stay on the preset they describe.

## Sixth round (2026-07-30, later): the first preset built and saved on the Floor

Still the pre-fix build ("here we go before we do another pull"), so nothing here tests the identity
or status-line fixes above. What it does test is **writing**, for the first time in the field:

```
03:56:12  connect → Pull Me Under (FACTORY 1 #45), 8118 bytes
03:56:19  browse bank 3 (USER 2) → 3347-byte listing
03:56:22  identity moves to bank 3 #0 "New Preset", 6690 bytes
   ...    add / undo / add amp / add cab / set params
04:00:14  edit ACK reply=[]  →  identity now bank 3 #0 "fretwireTest1"
04:03:28  session closed — pedal returned to standalone
```

7 minutes, zero `ERROR`, two `WARN` (both the benign `empty chunk before declared stream end`, both
absorbed). The saved preset came back as an attachment and our own decoder reads it cleanly:
`Brit P75 Nrm` in slot 1 with all twelve params, `2x12 Silver Bell` in slot 2 with six, snapshots
consistent — matching his screenshot of the same preset exactly. The **cross-setlist browse-and-load
split** (browsing and loading out of another setlist is allowed, writing across is not) is confirmed
working: he was in FACTORY 1, browsed and loaded out of USER 2, and saved in place there.

### The device refuses commands and we said "done" anyway  [solid] [fixed]

Twice in the middle of that session the device answered an edit with

```
edit ACK reply=[…, 131, 102, 205,0,44, 103, 204,255, 104, 129, 111, 235]   → {102:44, 103:255, 104:{111:-21}}
edit ACK reply=[…, 131, 102, 205,0,60, 103, 204,255, 104, 129, 111, 235]   → {102:60, 103:255, 104:{111:-21}}
```

and the preset stream stayed 6778 bytes across both — nothing was applied. `send_edit` logged the
reply at `DEBUG` and returned `Ok`, so the GUI announced both edits as done. See
`docs/protocol.md` for the key-103 status decode; `Error::Rejected` now carries it out.

The log also could not say *which* command was refused — we logged only what came back — so the op
had to be recovered from the frame length. `send_edit` now logs the op and transaction it sent.

### What was refused: adding an amp **with its cab**  [solid] [fixed]

The two refused frames are 56 bytes. Frames are zero-padded to 4 bytes, so that is a 53–56 byte
frame; `add_block` builds 51 with both model indices as fixints and 53 with a `uint16` paired index,
and no other builder in the session's vocabulary lands in that window. Every amp in `amp.models`
links to a cab at `Helix.sym` index **687–829** (Brit P75 → `HD2_CabMicIr_4x12BlackbackH30`, 691), so
*every* pick from the synthetic **Amp+Cab** category produces exactly that 53-byte body.

The behaviour confirms it: after two refusals he added the amp alone (index 14) and a plain cab
(index 58) as separate blocks, and that is what the saved preset contains — two blocks, both
`23:false, 26:-1`, no pairing. The Amp+Cab category has never worked on hardware.

**Fix:** `Session::add_block` sends op 39 with no cab and follows it with op 40 carrying the pair,
which is HX Edit's own order and the op-40-with-pair path the capture tests already cover byte-exact.
**Confirmed working on hardware 2026-07-31** — picking an amp from the Amp+Cab category now lands
both blocks, paired.

### Smaller things, noted not acted on

- `drained stale wire frames at connect` fires on every read-open, not just at connect. The message
  is wrong; the behaviour is right.
- The device pushes unsolicited `cmd=4` frames on both `0x03f0` and `0x03ed` around a save
  (`body=31`, `body=17`, then `body=41` after the list refresh). We skip them as non-replies while
  waiting on a request, and re-read anyway, so nothing is lost — but they look like "stored" /
  "list changed" notifications and would be a cheaper way to know a write landed.

## Seventh round (2026-07-31): the freeze, reproduced with a log through it

Three logs and four screenshots, on one preset (`fretwireTest2`, USER 2 slot 1: Deluxe C… → split →
Rotary Drive on row B → Placater → 8x10 → Double T…). The tester's own account:

> I dragged the rotary down, worked a treat, then when I dragged the end of the loop to be between
> the amp and the cab, then it froze the unit […] when I rebooted the unit, and relaunched the GUI,
> it hadn't saved when I dragged the rotary down to the loop. So I recreated that, and it defaults
> the mixer to the endpoint of the DSP. So when I tried to place the mixer to between the amp and
> the cab, it froze the unit again(!)

Two freezes, same action, either side of a power cycle. This is the first lockup we have a log
through — and unlike the July 26 incidents, nothing is ambiguous about it.

### Both freezes are an op-21 write the device stopped accepting  [solid] [partly fixed]

The mixer move has no surgical op (the position is the node holder's key 13), so it goes through the
**whole-preset write**: ~14 frames of 496 bytes. Counting the device's empty `cmd 0x08` replies per
chunk across the three writes in these logs:

| write | outcome | credits per chunk | worst deficit |
|---|---|---|---|
| log 8 #1 (rotary → row B) | landed, 6809/6809 | `0 3 1 1 1 1 2 1 1 1 1 1 1 1` | **1** |
| log 8 #2 (mixer move) | froze at 2480/6809 | `2 0 0 0 0` | 3 |
| log 9 (mixer move, post-reboot) | froze at 5456/6809 | `1 2 1 1 1 1 1 0 0 0 0` | 3 |

Those replies are flow control, which we had documented as something to "drain". A healthy write
earns about one per chunk and never runs more than one ahead. Both freezes go to a flat zero and stay
there, while we kept sending — four more chunks, ≈2 KB, into a device that had stopped reading.

Then the editor hung with it. The last line of both logs is `Submitted URB … on ep 1` with no
completion: `Transport::send` was an **unbounded** `block_on(bulk_out(…))`, so a device that stops
draining its OUT endpoint blocks the host forever. The IN path had raced a timer since the beginning;
the OUT path never did.

**Fixes:** `Transport::send` now races a 2 s `WRITE_TIMEOUT`; `write_preset` waits 250 ms for each
chunk's credit, tracks the deficit, and aborts with `Error::WriteStalled` once it is more than 2
chunks ahead — on both traces that is the frame *before* the one we blocked on — dropping the read
cache on the way out, since the edit buffer is half-written at that point.

**What this does not fix:** the pedal still freezes, as far as we know. This stops fretwire hanging
and stops it pushing the rest of a preset into a wedged unit. The pacing change may also prevent the
freeze — the two traces dying at different chunks (2 and 8) on the same action looks like a race, not
a byte the firmware rejects — but that is inference. Needs hardware.

### Retest ask

The sixth round's Amp+Cab fix is **confirmed working on hardware** (2026-07-31), so that one is
closed. The mixer retest came back the same night — see below.

## Eighth round (2026-07-31, same night): the abort holds; the freeze is not ours

Same action, rebuilt on the fixed code. `fretwire10.log`, three screenshots.

### The guard did its job  [solid]

```
ERROR device stopped acknowledging mid-write — aborting the transfer
      sent=2480 total=6817 credits=2 chunks=5
```

No hang. The app stayed responsive through the failure instead of blocking forever on a URB that
never completes, and the two writes in this session read cleanly off the log:

| write | outcome | credits per chunk | worst deficit |
|---|---|---|---|
| rotary → row B | landed, 6809/6809 | `1 2 1 1 1 1 1 1 1 1 1 1 1 1` | **0** |
| mixer move | aborted at 2480/6817 | `1 1 0 0 0` | 3 |

The healthy write is now at deficit **0** across all fourteen chunks, against a worst of 1 on the
same write before the fix — waiting for credits improved the good path as well as catching the bad
one.

### The freeze is not a pacing race  [solid] — corrects the seventh round

Last round's guess was that outrunning the credits might be what wedged the device. It isn't. With
the host waiting properly for every one, the same action kills it at the same place: credits stop
after chunk 2, abort at 2,480 of 6,817. The credits are how you *detect* a wedged device, not how you
avoid wedging one.

What is left is content. The device is ~1 KB into the blob when it dies, so it is reacting to bytes
it has already consumed rather than to a finished preset it has parsed. We still cannot reproduce it
offline — there has never been a dump of `fretwireTest2`.

### The aftermath was its own bug  [solid] [fixed]

After the abort the log runs another 50 seconds of:

```
ERROR bulk OUT timed out — the device stopped draining its endpoint
```

every ~2.25 s, twenty times, then three more at exactly 2.0 s and a close. That is the GUI heartbeat
(250 ms) beating into a dead pedal, each beat now burning the full 2 s write timeout — and since the
beat holds the session lock, the entire UI was stuck behind it until he disconnected by hand. The
final three are `close()`, sending teardown frames on all three channels to a device that was not
listening.

**Fix:** the heartbeat gives up after 3 consecutive failures — or immediately when
`Session::device_lost` is latched, which a stalled OUT endpoint does on the first miss — drops the
session, and emits `device-lost`; the frontend falls back to the disconnected view with "power-cycle
the HX device, then reconnect". `close()` skips the wire entirely once the device is gone, so
teardown is instant instead of 6 s.

### Retest ask

1. A `dump-raw` of `fretwireTest2` **before** the mixer drag, plus which column he drops the mixer
   on. That lets the killing op-21 body be rebuilt offline and diffed against the rotary-move body
   that writes fine — the only way to get at the content question without another lockup.

## Ninth round (2026-07-31): a real bug — the offset table we were invalidating

> **Correction (twelfth round, below):** filed at the time as *the* root cause of the lockups. It
> was not — a mixer drag still wedges a pedal with this fixed. Real bug, real fix, wrong verdict.

He sent `dump-raw` of `fretwireTest2` (7023 bytes, serial, 5 blocks — the state before the drag).
That was enough; the rest was offline.

### The header is an offset table  [solid]

What `docs/preset-format.md` called "a fixed header/uuid (kept verbatim, meaning TBD)" is 48 bytes =
12 little-endian `u32` offsets into the blob. Slot 0 is the preset map's offset (always 61), slots
1–9 address individual top-level entries, slots 10 and 11 are the blob's total length. Confirmed on
all four presets we have: slot 0 matched the map offset every time, the last slot matched the blob
length every time, and every interior slot landed exactly on a `<key><value>` boundary.

### We were shifting the bytes out from under it  [solid] [fixed]

rmpv encodes integers minimally; the device does not (`d1 00 00` for zero, everywhere). So our
re-encode of an **unmodified** preset is 117–216 bytes shorter, and everything past the first such
integer moves. We copied the table verbatim. On `fretwireTest2`:

```
header slot 5  (949)  dev: 02 82 ...   ours: 02 82 ...     ok — before the first shift
header slot 6 (1391)  dev: 05 de ...   ours: cd 01 13 ...  mid-value
header slot 7 (1814)  dev: 06 82 ...   ours: 00 00 00 ...  mid-value
header slot 10 (7004) = the blob's length   →  our blob is 6788 bytes
```

The device was told "7004 bytes, sections here", handed 6788 shifted bytes, and followed a pointer
216 bytes past the end of its buffer. It stopped reading ~1 KB in — which is where the flow-control
credits stopped in all three field traces.

**Not a Floor bug and not about the mixer.** Both HX Stomp captures round-trip just as wrong. Every
op-21 write fretwire has ever sent carried a corrupt table; the mixer drag was simply the first edit
with no surgical op behind it, so it was the first one that had to use op 21.

**Fix:** `to_blob()` rebuilds the table from the bytes it emits. Self-consistency, not byte-identity
with the device — the latter isn't reachable through rmpv and isn't what the device needs. Verified
across all four captures, with and without the mixer mutation.

The regression test that should have caught this asserted the header came back **unchanged**, which
is the bug restated as a requirement; its byte-identity check was an `eprintln!` annotated
"(Not required — the device parses msgpack.)" Both now assert the real invariant.

### Retest ask

1. The mixer drag, once more. This is the first build where the blob we send describes itself
   correctly. If it still freezes, the offset table was not the whole story and I want the log.

## Tenth round (2026-07-31): `fretwire12.log` — pre-fix, and a new symptom

**This log predates the offset-table fix.** It ends 06:05:36Z; `813cae3` was committed 06:06:06Z,
thirty seconds later. Everything below is the old build.

### Three op-21 writes completed  [solid] — narrows the previous claim

```
write-preset sent bytes=6809 acked=false
write-preset sent bytes=6809 acked=true
write-preset sent bytes=6809 acked=true
```

All fourteen chunks, **deficit 0 throughout**, no freeze, ~9 minutes, clean close. So a corrupt
offset table is *not* always fatal — the device survives it when the bad offsets happen to land
somewhere it tolerates. Across every trace so far the split is by size: **6809-byte writes complete,
6817-byte writes freeze** (fw8, fw9, fw10 all froze at 6817; fw10 and fw12 completed at 6809). The
8-byte difference is the mixer-move mutation. That is a correlation, not a mechanism.

### The parallel path is silent  [open]

The tester, on the preset built in this session:

> fretwireTest2h shows that the mixer is not disabled levels are good, but no rotary goodness […]
> I'm gonna try changing it to an obvious delay […] saved, but no delay sounds.

So: block sits on row B, mixer enabled, levels sane, **no audio from the parallel path** — confirmed
twice, with two different models. Note he reported the *same* drag working earlier ("dragged the
rotary down, worked a treat", `fretwire8.log`), so this is a regression within the session, not a
feature that never worked.

**Leading hypothesis: the earlier op-21 writes damaged the stored preset.** The three whole-preset
writes at 05:57:20, 05:57:32 and 05:58:15 all carried the stale offset table, and on this preset the
wrong slots addressed keys **5, 6 and 10** — preset settings, focused block, and the snapshots, whose
per-snapshot `3` field is per-slot state for every block. Garbage there gives a correct-looking chain
with wrong per-block state. He then saved (op 71) at 05:58:30, 06:03:20 and 06:04:49, so **the damage
is in flash**. The drag at 06:04:46 was op 43 — surgical, computed by the device from its own already
damaged state. [hypothesis]

The alternative — that our row-B routing is simply wrong — is not excluded, but it has to explain why
the same action worked in `fretwire8.log`.

**Cheap test:** on the fixed build, build the preset again *from scratch* in a fresh slot. Repairing
`fretwireTest2` is not worth it; if its stored state is damaged, every later edit inherits it.

### Minor, recovered on its own

Right after a model swap (op 40, 05:57:47) three consecutive reads failed with
`envelope key 104 missing or not bytes`, backing off and retrying, then succeeded. The retry loop did
its job; worth watching whether a swap needs a settle delay before the read.

## Eleventh round (2026-07-31): `fretwire14.log` — offset fix is in, parallel path still silent

First session on `813cae3`. Fresh preset (`fretwireTest3`), built from scratch: op-39 adds and op-71
saves, then two op-43 row moves. **No op-21 writes at all**, zero errors, clean close.

That is decisive on one point: **the silent parallel path is not the offset-table bug.** No
whole-preset write happened, so nothing we sent could have carried a stale table; the moves are
surgical and the device performs them itself. The corruption hypothesis from the tenth round is
withdrawn.

### What a parallel path actually requires  [solid]

Diffing the two split captures against the two serial ones (see `docs/preset-format.md`): the split
and mixer nodes are **always present**, at slot indices 10 and 19, on serial presets too. Five fields
differ, and no more:

| field | serial | parallel |
|---|---|---|
| DSP group `21` | 0 | non-zero |
| split `20 → 18` | false | **true** |
| split `20 → 15 → 13` (column) | 0 | 2 or 5 |
| mixer `20 → 18` | false | **true** |
| mixer `20 → 17 → 13` (column) | 0 | 7 or 9 |

A serial preset already carries the Y-split model (257) and the mixer model (151). So going parallel
is **enabling two nodes that already exist and giving them columns** — and none of those five fields
has a known surgical op.

`move_block_to_row` sends op 43 and nothing else, on the strength of a doc comment asserting "the
device activates/retires the split as needed". That assertion has never been checked against a dump.

### Why this can't be settled from a log

The logs carry frame sizes, ops and identities — no preset contents. Both readings survive it:

- the device sets all five fields and something *else* is silencing path B (mixer B-level, or the
  block landing at a column outside the split→mixer bracket); or
- the device sets some but not all, and the row-B slot is simply not in the signal path.

There is indirect evidence for the first — `set_node_pos` refuses unless `dsp_is_split` is true, and
it ran (and froze) on `fretwireTest2` in the sixth round, so key `21` was non-zero there after a
row-B move. But that is one preset, inferred, and it does not cover the other four fields.

### Retest ask — one artifact settles it

On the fixed build: drag a block to the parallel row, then **close the GUI** and

```
cargo run -p fretwire-cli -- dump-raw parallel-after.bin
```

That dump answers all five fields at once, and tells us whether this is a missing activation on our
side or a routing detail on the device's.

## Twelfth round (2026-07-31): the dump clears our code — the preset is correct

`fretwireTest3.bin`, taken straight after a drag to the parallel row, plus `fretwire15.log` (op 78 +
op 43, then op 71 save — nothing else, no errors).

### The device sets all five topology fields  [solid] — suspect exonerated

| field | after our op-43 drag | device-authored `split_preset` |
|---|---|---|
| DSP group `21` | **1** | 1 |
| split `18` / column | **true / 2** | true / 2 |
| mixer `18` / column | **true / 9** | true / 9 |
| row-B block | **slot 12** | slot 12 |

Identical. `move_block_to_row` sending op 43 alone is correct, and the doc comment I flagged as the
prime suspect was right after all. `show-preset` agrees: *"split (parallel) topology"*, block at slot
12 marked `(row B)`, enabled in the live scene and in all eight snapshots.

### The node parameters are sane too

Resolved against the catalog (`Enabled` is content key `18`, not a param, so the stored arrays are
one shorter than the model's list — the bool in each pins the alignment):

```
SPLIT  Split Y   Balance A = 0.5   Balance B = 0.5   bypass = false      (both at the .models default)
MIXER  Mixer     A Level = 0 dB    A Pan = 0.5       B Level = 0 dB
                 B Pan = 0.5       B Polarity = false  Level = +3 dB
```

Nothing here mutes path B. The only deviation from default is the mixer's master `Level` at +3 dB,
which is *louder*. `B Level` is at unity.

**So the preset we produce is correct**, and this is not a data bug we can see. Every field we can
compare matches a working device-authored preset.

### What the chain actually is

```
10 Band Graphic (common)  →  ⋔  →  A: Line 6 2204 Mod → 2x12 Match H30 → Cave
                                   B: Alpaca Rouge
                                                        →  ⋉  →  out
```

Path B is a **bare distortion with no cab**, summed against a full amp→cab→reverb path. A cab is a
steep low-pass; without one, path B is thin fizz sitting under a cab'd amp at equal mixer level. That
is a plausible reason for "no rotary goodness" / "no delay sounds" that is not a routing failure —
and it is consistent with him hearing nothing useful from a rotary and a delay placed the same way.

### The test that separates the two, and needs no dump

Mute path A at the mixer — set `A Level` to −60 dB — and listen.

- **Path B audible** → routing works; this is a mix/placement issue, and the fix is musical (put the
  cab before the split so both paths share it, or raise `B Level`).
- **Still silent** → the routing really is dead, and the cause is somewhere we cannot see in the
  preset data, which would be a genuinely new lead.

## Thirteenth round (2026-07-31): the freeze survives the offset fix — reproduced on a Stomp

Reproduced by the maintainer on an **HX Stomp**, on the build with the offset-table fix in it,
dragging the mixer to sit right after the "US Princess" amp (before the reverb) on the A path. The
pedal wedged; the next connect could not complete a handshake until it was power-cycled.

Two things follow.

**The offset table was not the cause.** It was a real bug and it is really fixed — the blob we send
now describes itself correctly, verified on every capture. But a self-consistent blob still wedges
the device, so the ninth round's verdict was wrong and is corrected there. What the fix did buy is
that the failure is now *contained*: the write aborts instead of hanging the editor.

**It is not Floor-specific.** Same failure on a Stomp, same operation. Every lockup we have on record
is an op-21 whole-preset write, and nothing else has ever done it.

### The next hypothesis: our re-encoding itself  [hypothesis]

`to_blob` rebuilds the preset map with rmpv, which writes integers minimally; the device writes
plenty of `d1 00 00` (int16 zero). Our blob is therefore 117–216 bytes shorter than the device's for
the *same* preset — a different byte sequence describing the same tree. We have been assuming the
device just parses MessagePack. It demonstrably does more than that (the offset table), so it may
also care about widths, or about the exact length it was told to expect.

### The experiment that separates encoding from geometry

`fretwire write-roundtrip` reads the current preset and writes it straight back **unchanged** via
op 21. No mutation, no geometry change — the only difference between what the device sent and what it
gets back is our re-encoding.

- **It wedges** → the op-21 path is unsafe for *any* preset, the mixer position is irrelevant, and the
  fix is to stop re-encoding: splice the mutated value into the device's own bytes and leave the rest
  untouched.
- **It survives** → the encoding is fine and something about the mixer *position* is what the device
  cannot take, which points back at `set_node_pos`'s guards.

Run it on a scratch preset, with `FRETWIRE_DUMP_WRITES=<dir>` set so the exact blob is on disk either
way. Worst case is a power cycle, which this operation already costs.

## Fourteenth round (2026-07-31): `write-roundtrip` survives — encoding is not the problem

Run on an HX Stomp with `FRETWIRE_DUMP_WRITES` set. Preset `ClaudeTest`, **serial**, 2 blocks.

```
reassembled preset stream bytes=2303 declared=2303
dumped the op-21 blob before sending  bytes=2167
write-preset sent bytes=2188 acked=false
reassembled preset stream bytes=2303 declared=2303   <- re-read, preset intact
session closed — pedal returned to standalone
```

**No freeze.** The device accepted a 2167-byte blob where it had sent 2303 — our minimal integer
encoding, 136 bytes shorter — and re-served the preset correctly. So the thirteenth round's
hypothesis is wrong: the re-encoding is not what wedges the pedal, and splicing into the device's own
bytes is not the fix.

### The offset-table fix, verified on hardware  [solid]

First check of a real emitted blob against a real device. The dump:

```
offset table: 61, 96, 514, 516, 527, 539, 662, 707, 62, 713, 2167, 2167
   slot 0  = 61          the preset map
   slot 10/11 = 2167     exactly the blob length
   interior → 00 82 | 01 c0 | 03 82 | 04 9a | 02 82 | 05 8f | 06 82 | 07 83 | 0a 86
```

Every interior offset lands on a `<key><value>` boundary and nothing points past the end. The ninth
round's fix does what it claims.

### Two honest limits on this result

1. **The preset was serial.** Every lockup on record has been on a **split** preset. This run does
   not touch the failing configuration.
2. **A no-op write cannot prove the device *applied* it.** The blob was unchanged, so an identical
   re-read is equally consistent with the device having ignored it (`acked=false`). What this does
   prove is that the transport survives an op-21 carrying our encoding — and the failure we are
   chasing is a transport-level wedge, so that is the relevant half.

### Next: the same probe on a split preset

That is the one variable between this run and every freeze. Load a **parallel** preset — no drag, no
mutation — and run `write-roundtrip` again.

- **Freezes** → op-21 is unsafe on a split preset *regardless* of what changed, which puts the split
  and mixer node structures in the frame rather than the position value.
- **Survives** → the transport is fine in both topologies and it is specifically the **mixer position
  value** that kills it, which points squarely at `set_node_pos` and its guards.

## Fifteenth round (2026-07-31): eleven op-21 writes on hardware, no freeze

Run directly against an HX Stomp by the maintainer's agent, on the build with the guard fix
(`0732954`). A new `fretwire node-pos <split|mixer> <column>` CLI command drives `set_node_pos` — the
exact call the GUI's mixer drag makes — so the failing operation can be reproduced without a mouse.

| probe | result |
|---|---|
| `write-roundtrip`, **serial** preset | survived, re-read intact |
| `move-to-row 6 p` (serial → split, op 43) | survived, split created |
| `write-roundtrip`, **split** preset | survived, re-read intact |
| `node-pos mixer 6` (mixer past the only A block) | survived, **applied**: column 9 → 6 |
| `node-pos mixer 6` with a reverb at column 7 — mixer lands *between* two A blocks, the maintainer's exact failing shape | survived, applied |
| 8 × `set_node_pos` back to back **in one held session** | all 8 survived and applied |

Eleven whole-preset writes, none of which wedged anything, all verified applied by re-reading the
column back. The op-21 path works.

### So what was freezing?

The maintainer's last freeze was at 00:03:22 local; the guard fix landed at 00:07:13 — four minutes
later. That freeze is the one whose log reads `sent=2688 total=2688 credits=3 chunks=6`: **every byte
had already gone out**, and the old cumulative-deficit guard aborted anyway, which skips the
terminating `cmd 0x08` and leaves the device holding a complete transfer it is never told has ended.
The guard added to prevent lockups was, in that case, causing one.

Two distinct failure modes, then:

1. **Pre-guard** (`fretwire8/9.log`): the device genuinely stopped crediting mid-transfer and the
   unbounded `bulk_out` hung the host forever. Why it stopped is still unexplained.
2. **Post-guard, pre-`0732954`**: the guard misfiring on a completed transfer and withholding the
   terminator. Fixed.

### The limit on this result

Every probe here was a **2.2–2.4 KB Stomp preset — 5 chunks**. The Floor freezes were **6.8 KB, 14
chunks**. That is nearly three times the transfer and a different device, so this does not close mode
1; it only shows the path is sound at this size. A Floor retest on the current build is still the
thing that would settle it.

## Round 16 (2026-08-01): the edit ACKs were mostly not ACKs

Six logs (`fretwire16`–`21`) and two more `.bin`s. The freezes are not the interesting part of this
batch — the reply correlation is.

### We took credit frames as edit acknowledgements

`send_edit` matched a reply as "the next non-keepalive frame on the edit channel". The device also
sends empty `cmd 0x08` credit frames there, and after a browse read, leftover chunks of the finished
stream. Counting every `edit ACK` line in the field logs — 353 of them, ops
20/28/30/39/40/41/43/71/78 — against the transaction each one was answering:

| | count |
|---|---:|
| echoed the txn we sent | 233 |
| empty body (a credit frame) | 86 |
| echoed an **earlier** txn (lag 1–5) | 50 |
| cross-stream (an op-20 reply holding preset-list bytes) | 1 |

By op, the structural path is the one that never worked:

| op | correlated | total |
|---|---:|---:|
| 43 `move_block` | **0** | 21 |
| 71 `save_preset` | **0** | 21 |
| 78 `begin_structural` | 6 | 24 |
| 30 `set_value` | 115 | 183 |
| 40 `swap_model` | 46 | 67 |

Both failure shapes are harmful. An empty frame accepted as an ACK means we report an edit applied
on the strength of a frame that says nothing about it. And once a stray frame is consumed, every
later reply is one behind — so a refusal is attributed to the wrong command, and the
`sent_txn == txn` check then quietly *suppresses* the rejection rather than reporting it.

`fretwire19.log` catches the end state: an op-20 select whose reply body is preset-*list* text, then
every subsequent request timing out for ~60 s while the pedal itself stays healthy and answers its
own keepalives. That is exactly the tester's "it locks up the UI, but strangely, not the unit — the
UI pretends to let you do something, but nah."

Fixed by correlating on the txn echoed at key 102. Verified on a Stomp: 46 consecutive op-40 swaps
and ops 30/39/41/43/71 all matched their own transaction. Note **save does have a real ACK** —
`{102: txn, 103: 0, 104: nil}` — arriving just after the credit frame we had been mistaking for it.

This was also floated as an explanation for the op-21 freezes, on the reading that a structural drag
is `op 78 → op 43 → op 21` and 78/43 are the two ops we almost never correlated. **Refuted
2026-08-02:** that sequence appears in none of the 43 captures. `move_EQ_right_two_slots` carries a
bare op-21 and nothing else, `one_by_one_move_all_blocks_one_right` is `78,43` eleven times with no
op-21 anywhere, and `move_simple_eq_to_parallel_path` is `43,23`. HX Edit's whole-preset write is
unbracketed, like ours.

### The duplicate cabs are HX Edit's `Cab › Dual`

Reported from the field: the Cab (Mic+IR) list shows every cab twice and the second copy will not
load — the block reverts, or stays empty if it was empty.

The 46 `HD2_CabMicIr_*WithPan` symbols are the **dual** cab model (two cabs, per-cab pan), and
`HX_ModelCatalog.json` groups them under `Cab › Dual` while the plain symbols sit in `Cab › Single`.
Both carry the same display `name`, so a flat per-category listing shows 92 rows for 46 cabs. The
pedal refuses an in-place swap to one. Two codes turn up — **`-306`** most of the time and `-21`
in some states (both seen on the Stomp; the Floor log `fretwire18.log` has six op-40 `-21`
refusals from the tester working through the cab list). The refusal is the invariant, not the code.

They are now excluded from the swap list — editing a dual cab needs two model refs and the pan
params, which is a feature, not this fix. Name resolution is deliberately untouched so a preset that
already contains one still reads back with its own name and params. All 46 remaining entries were
swapped onto a real block on hardware: **0 refused**.

One genuine duplicate label survives: `HD2_CabMicIr_2x12MatchG25` and `..._2x12MatchH30` are
different cabs that `HelixModelDefs.bin` gives the same `name`, "2x12 Match H30".
`HX_ModelCatalog.json` has the right one ("2x12 Match G25"), so preferring catalog names would fix
it. Line 6's data, not ours.

### The 3 Osc Synth was filed under a category of its own

`HD2_SynthSubtractive` is the only model the shipped `.models` files put in category 5, so the
picker grew a "Synth" entry containing exactly one model while the **Pitch/Synth** list — where HX
Edit files it (`Pitch/Synth › Stereo`) and where anyone would look — did not have it. Category 5 now
folds into 7. Swapping a block to it works on hardware, so the reported "loading a synth locks up
the UI" is consistent with the ACK desync above rather than anything about the synth.

### Not yet closed

`fretwire17.log` froze genuinely mid-write — `sent=2480 total=6911`, credits flat from chunk 3 of
14 — but the log line reads `deficit=`, so it is the pre-`0732954` build. It needs re-running on the
current one before it says anything new.

### A read-modify-write could run on the wrong preset

`read_preset` already handles the case where the preset identity moves across a stream read: it
retries, and only as a last resort decodes the blob anyway, on the grounds that showing a stale view
beats showing nothing. `read_preset_raw` did not — it called the same inner read and discarded the
flag. And that is the function every op-21 whole-preset write reads from (`set_node_pos`,
`delete_block`, `reorder_block`, `move_block_to_row`, `insert_block`): read the blob, edit the tree,
write it straight back to whatever preset the pedal is on *now*.

So a structural edit made while the device was mid preset-change would write a blob belonging to
neither preset over the current one. These logs carry 21 `provenance is ambiguous` warnings, and the
tester's resaved `fretwireTest3` came back with **no blocks at all** — matching his note that the
preset contents disappeared and he then saved over them. `read_preset_raw` now retries and errors
rather than guessing. `backup_setlist` already had an equivalent identity guard, so backups were
never exposed.

### Making the next log readable

`nusb`'s per-URB debug lines are **94% of a bug-report log by volume** — 7.2 MB of one 7.7 MB
session — and they bury the protocol lines a report is actually about. Both binaries now damp `nusb`
to `warn` unless `RUST_LOG` names it explicitly. Measured on hardware, the same `pull` drops from
407 KB to 8 KB; `RUST_LOG=debug,nusb=debug` still gets the URBs back when a transport question needs
them.

Two read-path warnings were downgraded to debug after checking they are benign, so that a WARN in
the next round means something:

- **empty chunk mid-stream** — the device's `cmd 0x08` flow-control credit, the same frame it
  interleaves during an op-21 write, landing between two stream chunks.
- **short chunk mid-stream** — a 256-byte chunk arriving as two frames. The halves always sum back
  to 256 (207+49, 46+210, 12+244, 251+5 across these logs) and every read that logged it still
  reassembled to exactly its declared length. Fragmentation, not truncation.

### Loose ends closed before handing the build back

- **Every edit op re-checked on hardware, timed.** ops 6/20/25/28/30/39/41/43/71/78/88/89 each
  matched their own transaction, 1–229 ms, no skipped frames. This mattered because op 28
  (`delete_block`) never correlated once in the field logs (3 samples, 2 empty + 1 lagged), which
  would have meant delete now burning the full 3 s match window and failing. It correlates fine —
  those samples were victims of the desync, not evidence that op 28 is different.
- **A refused edit no longer eats the undo timeline.** `edit_begin` truncated the redo branch and
  set a `pending` label; if the edit then failed, `edit_commit` never ran and both were left behind.
  That path was rarely taken while refusals were being swallowed — now that they surface, every one
  hits it. Truncation moved to `edit_commit`, `Session::edit_abort()` added, and both GUI helpers
  (`mutate_edit`, `returning_edit`) call it on the error path. Verified live: two edits, one undo,
  then a refused swap — timeline still `["Loaded", "Set 0.4", "Set 0.6"]` with the redo intact.
- **The GUI already handles a surfaced refusal correctly**: `apply()` leaves `preset` untouched and
  raises a toast, which matches the device, since a refused edit changes nothing.
- **`set_setting` is refused** (`op 25`, code `-3`) — found only because refusals now surface. Not
  in the GUI's command surface, CLI-only, so it is a lead rather than a regression.

## Round 17 (2026-08-01, evening): the abort was ours, and a 14-chunk Floor write does work

`fretwire22b`, `23`, `24` — the first logs from a build carrying the ACK-correlation, provenance and
history fixes (and the nusb damping: three sessions in 267 KB total, against 7.7 MB for one before).

**The ACK fix works.** ops 78/43/41/30/20/71 all echo their own transaction in these logs.

**It did not stop the freezes**, so the "structural edit never really acknowledged" hypothesis from
round 16 is dead. Four more lockups here, and every one of them aborted at exactly `sent=2480`:

| log | preset | credits per chunk | outcome |
|---|---|---|---|
| 22b | 6844 B | 1, 2, –, –, – | abort at chunk 5 |
| 23 | 6844 B | 1, 1, –, –, – | abort at chunk 5 |
| 24 #0 | 6816 B | 1, 1, –, –, – | abort at chunk 5 |
| **24 #1** | **6816 B** | **3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1** | **all 14 chunks, completed** |
| 24 #2 | 6816 B | 1, 1, –, –, – | abort at chunk 5 |

2480 is 5 × 496 — **our** number, `MAX_SILENT_CHUNKS = 3` firing on the third quiet chunk. The
device never chose it. And write #1 settles the question the guard was hiding: **the same Floor
writes the same 6816-byte preset over all 14 chunks and completes it.** So a large op-21 write is
not inherently beyond the Floor, and a three-chunk silence is not a death certificate.

The guard is removed. Its stated job — stop feeding a device that is already gone — belongs to
`WRITE_TIMEOUT`, which fails a send in 2 s once the pedal stops draining, and which did not exist
when the guard was written. What the guard added was a guess about credit patterns, and the guess is
wrong. Credit *pacing* stays (outrunning the device is a real failure mode); only the abort goes,
with a 30 s wall-clock backstop so nothing can loop forever.

An HX Stomp is unaffected: it credits every chunk (1,2,4,5,6 on a 5-chunk write), and six writes
spread across 90 reads in one held session all landed.

### The open question, and one lead

What separates a Floor write that credits from one that doesn't is **not yet known**. The one thing
that separates them in the data is the edit channel's running `arg` offset at the moment the write
starts:

| arg at write start | outcome |
|---:|---|
| 22079 | completed |
| 47311 | abort |
| 94976 | abort |
| 112962 | abort |
| 220292 | abort |

The completed write was the first one after a reconnect — which matches the tester's own note that
"after a reboot/reconnect, the mixer could be moved between cab and reverb / saved". That is a
**lead, not a finding**: n = 5, and it is confounded with session length. An attempt to reproduce it
on a Stomp by driving `arg` up over 90 reads in one session changed nothing — but Stomp presets are
5 chunks, so that test never reaches the region where the Floor fails.

**The next Floor round should answer it directly:** with the guard gone, does a stalling write now
recover and finish, or does `WRITE_TIMEOUT` fire? And does an op-21 write immediately after connect
behave differently from one late in a session?


## Round 18 (2026-08-01, later): the experiment ran, and the answer is no

`fretwire26.log` — the tester on a build with the credit guard removed, dragging a block into the
loop on a rebuilt preset (`Cali400test1`).

**The guard was not what stopped those writes.** The write goes quiet after chunk 2 exactly as
before, reaches three silent chunks at chunk 5 — and with nothing to stop it, pushes on. The next
send times out after the full two seconds: the device has stopped draining its endpoint altogether.
It does not recover. So aborting never caused the lockup, and the pedal is already gone by the time
the credits stop.

The guard is restored. Its value is not prevention, it is a legible failure: `sent`/`total`/
`credits` after ~0.75 s rather than a bare USB write timeout after 2 s. It cannot fire on a healthy
transfer — a Stomp credits every chunk, and the one Floor write on record that completed credited
all fourteen.

### Where that leaves the freeze

Six Floor whole-preset writes are now on record:

| log | arg at start | total | chunks sent | outcome |
|---|---:|---:|---:|---|
| 24 | 22079 | 6816 | 14 | **completed** |
| 24 | 47311 | 6816 | 5 | wedged |
| 23 | 94976 | 6844 | 5 | wedged |
| 24 | 112962 | 6816 | 5 | wedged |
| 26 | 203731 | 6883 | 5 | wedged |
| 22b | 220292 | 6844 | 5 | wedged |

The mechanism is now clear even if the cause isn't: the device credits a chunk or two, stops
consuming, keeps accepting into a buffer for another three chunks, and then blocks. Five chunks is
~2560 bytes on the wire including framing, which is a plausible receive buffer.

**The only thing separating the six** is the edit channel's running `arg` offset when the write
begins — the single completed write started at 22079, every failure at 47311 or above, and the
completed one was the first write after a reconnect. That matches the tester's own observation that
a reconnect let a mixer move through. It stays a **lead, not a finding**: n = 6, `arg` is confounded
with session length, and driving `arg` up over 90 reads in one Stomp session reproduced nothing
(though a Stomp write is 5 chunks, so it never reaches the region where the Floor dies).


## Round 19 (2026-08-01, night): nine writes, and one variable separates them

`fretwire27.log` adds three more Floor writes, and one of them is the cleanest control yet: **two
writes of the same preset, in the same session, seven minutes apart — the early one completed all 14
chunks, the late one wedged at 5.** No reconnect between them, nothing else different.

Every recorded Floor whole-preset write, by the edit channel's running `arg` offset when it started:

| arg | outcome |
|---:|---|
| 22079 | completed |
| 42264 | completed |
| 47311 | wedged |
| 72189 | wedged |
| 93106 | wedged |
| 94976 | wedged |
| 112962 | wedged |
| 203731 | wedged |
| 220292 | wedged |

Nine for nine, split between 42264 and 47311. `arg` is a running count of the bytes we have
*received* on that channel, so it climbs ~7 KB per preset read and tracks how much a session has
done — which means it is also a proxy for session age, and the two cannot be separated from the logs
alone.

So: **`FRETWIRE_WRITE_ARG=<n>`** sends the write's chunks with a fixed `arg` instead of the channel
cursor. Off unless set; verified on a Stomp that the default path is byte-identical to before and
the override reaches the wire.

    # late in a session, on the drag that reliably wedges it:
    FRETWIRE_WRITE_ARG=0 RUST_LOG=debug cargo run -p fretwire-tauri 2>&1 | tee log.txt

If the write completes where it otherwise wedges, the device is doing something with that field and
we have the bug. If it wedges anyway, `arg` was only ever a proxy for session age and the cause is
elsewhere — which is equally worth knowing, and rules out the one lead we have.

Unchanged across all of these: the stall is always at `sent=2480` (5 × 496), on presets from 6816 to
8430 bytes, and the device stops draining entirely one chunk later.


## Round 20 (2026-08-01, late): `arg` is refuted, and the stall is decided at chunk one

Eight more Floor writes across `fretwire30/32/33/35`, every one of them with `FRETWIRE_WRITE_ARG`
pinning the field the previous round suspected — first to `0`, then to `1`.

| log | write | cursor at start | forced arg | total | chunks | outcome |
|---|---:|---:|---:|---:|---:|---|
| 30 | 1 | 50635 | 1 | 6883 | 14 | **completed** |
| 30 | 2 | 97969 | 1 | 6883 | 14 | **completed** |
| 30 | 3 | 297136 | 1 | 6845 | 5 | wedged |
| 32 | 1 | 277759 | 1 | 6826 | 5 | wedged |
| 33 | 1 | 29397 | 1 | 6845 | 5 | wedged |
| 35 | 1 | 134312 | 1 | 6883 | 5 | wedged |
| 35 | 2 | 14361 | 1 | 6883 | 14 | **completed** |
| 35 | 3 | 615032 | 1 | 6841 | 5 | wedged |

**`arg` is not the cause. [solid]** Holding it constant changed nothing, and the ordering it was
supposed to explain is gone: log 33 wedged at cursor 29397 while log 30's first write completed at
50635. The nine-for-nine split of round 19 was session age wearing the cursor as a costume.

### What the credits say instead

Re-reading **every** recorded Floor write — 21 of them across `fretwire12`…`35`, 13 wedged — the
credit count separates them without a single exception:

| | credits delivered | chunk 3 credited? |
|---|---|---|
| completed (8 writes) | climbs at every chunk, to 14–19; `silent` never reaches 1 | yes, 8/8 |
| wedged (13 writes) | **stops at 2 or 3** | never, 13/13 |

So the device is not being outrun and does not degrade across the transfer. It stops dead after two
or three chunks, and everything after that goes into an endpoint that has already stopped. The
familiar "2480 of N bytes" is our own guard's stop point (5 × 496), not the device's.

First-chunk credit *latency* is a good tell but not a rule: 4–8 ms on all eight completed writes and
32–198 ms on twelve of the thirteen wedged ones — `fretwire24`'s third write is the exception,
credited in 3 ms and then dead after the second. `write_preset` now reports `first_credit_ms` on
every write, but the credit ceiling is the reliable signal.

### What it isn't

Nothing about the bytes explains it. In `fretwire35` the **same paste of the same 6883 bytes**
wedged the pedal, and then completed 43 seconds later in the same GUI session after a power cycle.
`fretwire24` does the same trick over three writes of one preset: wedged, power cycle, completed,
then wedged again 56 s later. Preset size doesn't separate the groups (6883 appears in both), nor
does the preceding op sequence (a write after one bypass wedged; a write after twenty parameter sets
completed).

That leaves device-side state we cannot see from here. The next step is bytes, not more inference:
`FRETWIRE_DUMP_WRITES=<dir>` saves the exact blob before the first frame goes out, so a stalling
write and a succeeding one can be diffed offline instead of compared by size.

    FRETWIRE_DUMP_WRITES=~/fretwire-writes RUST_LOG=debug cargo run -p fretwire-tauri 2>&1 | tee log.txt


## Round 20b: a preset size that made a preset unreadable

Separately, the same session turned up a decode bug with nothing to do with the Floor: three
consecutive reads of one preset reassembled 6794/6794 bytes and all three failed with
`envelope key 104 missing or not bytes`, and the tester saw the preset-list spelling of it
(`envelope key 104 is not an array`) at launch.

The stream is `marker:u16, type:u16, len:u32 (LE)` and then the MessagePack envelope, and
`locate_root` found the root by scanning for the value that consumed the most input. The length is
little-endian, so **its low byte sits immediately in front of the real root** — and when that byte
is itself a container marker satisfied by the three remaining length bytes plus one more value, the
decoder swallows the whole envelope as that container's last element. It ends where the real root
ends but starts four bytes earlier, so it consumes more and wins the scan, yielding
`{26: 0, 0: <the real envelope>}` — no key 104.

Exactly two length values in 256 do this: low byte `0x82` (fixmap, 2 pairs) and `0x94` (fixarray, 4
elements). A 6794-byte stream declares 6786 = `0x1A82`. Every read of a preset that size failed, and
would have kept failing.

Callers now scan with `locate_root_where`, which only accepts a candidate carrying the key that
caller needs.

The log corpus confirms the prediction exactly. Across 874 reads in every log we have, 99 distinct
stream sizes appear; **the only two that ever failed to decode are the two this predicts** — 7068
bytes (declared `0x1B94`, three reads, `fretwire12`, 2026-07-31) and 6794 bytes (declared `0x1A82`,
three reads, `fretwire35`, 2026-08-01) — plus three more reads of those same sizes. Nine reads in
874, all nine accounted for, and no decode failure anywhere else. [solid — verified against all 256
low bytes in a test, and against every read the testers have logged]

## Round 21 (2026-08-02): our write is a run of maximum-size USB packets

Four more Floor sessions (`fretwire37`–`40`), six op-21 writes, five wedged. Same signature as
always: two or three credits, then nothing, then `bulk OUT timed out — the device stopped draining
its endpoint`. The tester's summary of when it happens has not varied in a week — *"moving the
mixer"*, *"dragging the endpoint of the loop"*, *"ok here we go, will the endpoint dump core? yup,
it certainly will"*.

### It is one gesture, and it is the only op-21 in the app

Worth stating plainly, because it narrows the whole problem. From the GUI, `write_preset` is
reachable from exactly three places: `set_node_pos` (drag the split ⋔ or mixer ⋉ to a new column),
undo/redo, and a backup restore. Every write in these four logs is preceded 10–100 ms earlier by a
complete preset read, which is `set_node_pos`'s own read-modify-write — undo replays a stored blob
and reads nothing first. So all six were the loop-endpoint drag. Every other edit the tester made
across four sessions — model swaps, bypasses, parameter sweeps, block moves into and out of the
parallel path, saves — is surgical, and none of it has ever wedged a pedal.

### What the op history says, and what it doesn't

Tabulating all 21 recorded writes against the ops that preceded them in the same power-on:

| ops before the write | completed | wedged |
|---|---:|---:|
| an op-43 `move_block` somewhere earlier | 2 | 9 |
| no op-43 | 7 | 3 |

Suggestive, and it is not the answer: `fretwire38` wedged on an op history of exactly `[78, 43]`
and `fretwire8` completed on exactly `[78, 43]`. Same ops, same order, opposite outcomes — the same
wall the blob bytes hit.

### The packets

`wMaxPacketSize` on the pedal's bulk endpoints is **512**. A frame is a 16-byte header plus its
body, so our 496-byte chunk body is a packet of *exactly* the maximum size, and we sent nothing but
those. A bulk transfer built only from maximum-size packets has no short packet to terminate it.

HX Edit never does this. Its unit is 512 payload bytes split into a 496-byte frame and a 16-byte
frame, so every unit closes on a 32-byte packet, and the credit comes back after the pair. Both
captures carrying a bulk upload agree — the op-21 write in `move_EQ_right_two_slots`
(`496,16,496,16,496,16,8,496,8,8,496,16,423` for a 2991-byte TLV) and the fifteen 496+16 pairs of
`import_ir`. Measured on a Stomp, ours was `512 512 512 512 512 224` where HX Edit sends
`512 32 512 32 512 32 512 32 512 32 144`.

This is the first candidate that fits every fact the blob theories could not:

* **The bytes never mattered.** The same 6883 bytes wedged and then completed after a power cycle.
  Packetisation is identical either way; what differs is how much room the endpoint had left.
* **It dies two or three units in, always.** That is a receive path filling up, not a parser
  choking — and it is why chunk 3 is never credited in any of the thirteen wedged writes.
* **Session age looked like a predictor and then didn't.** The `arg` cursor separated the first six
  writes perfectly and fell apart when it was pinned (Round 20). Undrained endpoint slack is the
  thing session age was actually standing in for.
* **A power cycle is the only reliable cure**, and it is what resets an endpoint.
* **The Stomp mostly survives it** — a different USB stack with more slack, and it still completes
  every write on both cadences.

`Session::write_preset` now sends 496 + 16 per credit. It round-trips clean on a Stomp, which proves
only that it isn't a regression: the Stomp completed writes before the change too. **[hypothesis]**
until a Floor runs a build with it. If the endpoint drag stops killing the pedal, that is the answer.

One thing that sharpens it: **packetisation is now the only known difference between our op-21 and
HX Edit's.** Scanning the ops in all 43 captures on 2026-08-02 killed the standing suspicion that
HX Edit brackets its whole-preset write with `op 78 → op 43`. It does not — `move_EQ_right_two_slots`
is a bare op-21 with nothing before it, `one_by_one_move_all_blocks_one_right` is `78,43` eleven
times and never reaches op-21, and `move_simple_eq_to_parallel_path` is `43,23`. Same op, same
`{110: blob}` envelope, same terminator. The blob is our minimal re-encode rather than the device's
own bytes — shorter, offset table rebuilt to match, and exonerated on hardware in Round 14 — so what
is left differing is the packet boundaries.

## Round 21b: the "envelope key 104" errors are truncated reads

The tester has been reporting `envelope key 104 missing or not bytes` on and off for days —
at launch, after a save, once "apart from fat fingering the space bar". Two of them, in `fretwire12`
and `fretwire35`, were the false-root bug of Round 20b. The rest are something else, and
`fretwire39` caught one whole:

```
stream-start reply (chunk #0) arg=1887 body=214
stream chunk arrived fragmented — keeping it, continuing read got=1112 want=7055 len=42
... twelve more fragments ...
reassembled preset stream bytes=6366 declared=7055
```

Six hundred and eighty-nine bytes short, reported as a successful read, handed to the decoder, which
blamed whichever envelope key the missing tail happened to contain. The device still had more to
send — the next two frames in the log are `cmd=4 body=42` and `cmd=4 body=172`, discarded as
non-replies — and the session fell over half a minute later.

The read loop was bounded by `declared / chunk_0_size + 8` requests. Chunk #0 came back **214** bytes
rather than 256, giving a cap of 40, and the device fragmented the stream twelve times (42+172,
84+130, and so on, each split costing one more request for the same payload). The 40th request was
the cap. A bound sized against a whole chunk cannot survive a device that fragments freely.

Sized against a fragment now, and — the part that turns a recoverable hiccup into a bogus decode
error — **a short payload is an error rather than a preset**, so the existing retry gets its go.
[solid]

## Round 22: what the mixer column means is still unknown [refuted]

The first version of this section claimed that a block on row B past the mixer column is off the
signal path and goes silent, and that this explained the tester's repeated "no worky" reports. It is
wrong, and it was refuted within the hour by the next dump he sent.

`somehinged3_var1.bin` has the bracket at **split before column 1, mixer before column 3**, and its
two loop blocks moved out to **columns 3 and 4 — both past the mixer**. `somehinged2.log` shows the
moves going out as two clean `78 → 43` pairs with two saves and not one error, and he checked by ear:

> moved the loop fx elements, and they work even though they're "outside" the loop. saved

So whatever the node holder's key 13 encodes, it is **not** "signal stops here", and the bracket we
draw from it is not the boundary of the parallel path. The wire drawing was mistaken for a fact
about audio.

What actually silences a block dragged into the loop is still open. The baseline dumps rule out the
obvious rewrite of the theory too: `somehinged`, `somehinged2` and `midhinged` all have the mixer
before column 6 with both loop blocks at columns 1 and 2 — comfortably *inside* the bracket — and
that is the session where he reported loop blocks going silent over and over. Inside and outside
both work, and both fail. The mixer's own level/pan on the B leg has not been looked at yet and is
the obvious next suspect.

Two things that are worth keeping from the round:

* **The device is looser than our enclosure guard.** `Session::set_node_pos` and the UI both refuse
  to leave a loop block outside the bracket; op 43 will happily move one out there and the pedal
  saves and plays it. Our guard is what stopped him putting the mixer between blocks 1 and 2 — twice
  — so it is a candidate for removal, pending a hardware test of a mixer position left of a B block.
* `show-preset` now prints each DSP's bracket and flags loop blocks outside it, because working this
  out from a dump previously meant decoding key 13 by hand.

### The Round-21b fix is confirmed on hardware
`envelope key 104` appears five times in the chat during the `fretwire42` session, and never again
after he pulled the fix: zero decode failures in `fretwire45`, against six `preset read/decode
failed` in `fretwire42`. Truncated reads are done.

The write lockup is not. He wedged the pedal once on the new build, moving the mixer — an op-21 —
but that session's log has not arrived, so the packetisation hypothesis of Round 21 is still open.

## Round 23: the mixer is a block, and it is not the culprit either

The B-leg level/pan the last round nominated as the next suspect is now readable, and in the
tester's own dump it is innocent.

The routing nodes are ordinary blocks with a model and a stored param array; `show-preset` prints
them now instead of making you decode key 15/17 by hand. From `somehinged3_var1.bin`:

```
DSP1 ⋔ split before col 1  slot 10  [HD2_AppDSPFlowSplitY]
     [ 0] BalanceA       = 0.5
     [ 1] BalanceB       = 0.5
     [ 2] bypass         = false

DSP1 ⋉ mixer before col 3  slot 19  [HD2_AppDSPFlowJoin]
     [ 0] A Level        = 0      [ 3] B Pan          = 0.5
     [ 1] A Pan          = 0.5    [ 4] B Polarity     = false
     [ 2] B Level        = 0      [ 5] Level          = 3
```

Dead centre, both legs at unity, polarity off. Nothing here mutes a leg, so **the mixer's levels do
not explain the silent blocks** — the third theory in a row to die on his dumps, and the one that
leaves the least behind. The split's `BalanceA`/`BalanceB` are the only other routing knobs there
are, and they are centred too.

What the round does settle is that he could never have checked this himself: the mixer glyph has
always been clickable, but nothing said so, and the CLI never showed the values at all. Both fixed.

### `-306` is out of DSP [solid]
Two `-306` refusals in `somehinged3.log`, and with the target map in the log this time they were
finally diagnosable — see `docs/protocol.md`. The short version: op 40 refuses a swap the DSP budget
cannot fit, and **our meter reads about a quarter low**, so his 72.7% was effectively full. He was
not doing anything strange; the two models he picked were just too big for what was left.

This also means the first hypothesis — that op 40 cannot cross a model category — is dead, along
with the reflex behind it. Three of the last four "the pedal is being weird" findings turned out to
be fretwire mis-modelling the pedal, and the fourth was fretwire hiding what the pedal reported.

### The bool fix is confirmed on hardware
`somehinged3var3.log` has `{98:2, 29:true, 26:0, 28:9, 119: Bool(true)}` — `TempoSync1` on the trem,
sent as a MessagePack bool — accepted, code 0. Zero op-30 `-3` in either of the two newest logs. On
the older build the same gesture is the `-3` he hit twice in chat, once on a delay and once on a
reverb:

> ooh, turned on trails, pedal refuse the para change (op30) device code -3

Trails specifically is the key-29 case, fixed separately; the switch had gone read-only in the build
he was testing (`somehingeddelaytrails.png` shows it greyed), which is what he was reporting.

## Round 24: a loop block left of the split has nothing feeding it [refuted — see Round 26]

The evening's last three sessions finally produced a mechanism for the silent loop blocks, and it
is the one geometric claim that survives — the *other* side of the bracket from the one Round 22
demolished.

`somehinged3var5.log` has our own warning in it, fired once:

```
WARN moving this node leaves row-B blocks outside the bracket — strays=[3] pos=4 kind=2
```

He dragged the split to column 4. The loop block at column 3 was left to its **left**. What he
reported over the next ten minutes:

> tronup works / no wait, it doesn't / but helio does, weird
> ok, the helio works, but it's hella quiet …
> the tron still won't work, hold on, will try another filter
> OK, so now filters won't work - I've tried 3 different ones

> **Refuted the next day — see Round 26. The position was a coincidence; all three "dead" models
> were envelope filters and the live one was a reverb. Read this round for the evidence, not the
> conclusion.**

The two blocks were on opposite sides of the split. Heliosphere at column 4 was inside the bracket
and audible; Tron Up at column 3 was outside it and dead, and stayed dead through three model swaps.
The swaps are in the log — `HD2_FM4ObiWah`, `HD2_FM4QFilter`, `HD2_FilterMysterFilterMono`, all
three ACKed, all three inaudible. A cell left of the split has nothing feeding it, because the
signal has not branched yet.

**This does not resurrect Round 22.** Past the mixer he verified by ear that blocks still play, and
that stands. The bracket is asymmetric: the right-hand side is cosmetic, the left-hand side is not.

Two things fall out of it:

* **"hella quiet" is not a bug.** Split Y's `Balance A`/`Balance B` are both at their 0.5 default, so
  each leg is attenuated and the mixer sums them back — normal Helix behaviour, and against a dry
  amp path a parallel delay is meant to sit back. He also had the mixer's output `Level` at +3 dB
  and both leg levels at 0 dB, all within a hair of factory.
* **Our warning was right and invisible.** It went to a log file. The chain now marks an unfed loop
  block with an amber dashed border and a "⚠ no feed" badge, and the drag caption for the split says
  which blocks a drop would strand. Nothing is refused — the pedal takes the arrangement, and
  out-guarding it is what caused the last two mistakes.

To confirm: drag the split back left of the stranded block and the filter should come alive with no
other change. One gesture, unambiguous.

### Edit ACKs carry their target now
Matching "I tried three different filters" to the wire took hand-decoding the MessagePack the device
echoes in each op-40 reply. `send_edit` logged the target only on refusal; it logs it on success too
now, so a session log can be read back as a list of what was actually done.

## Round 25: the short-packet fix cut the op-21 lockup to a fifth, and one credit tells you [solid]

`fretwire43` opens with a `Compiling` line — the tester pulled and rebuilt mid-evening, which
splits every recorded write into before and after `80ee812` ("End every op-21 unit on a short USB
packet"). Same pedal, same presets, the same few hours:

| | writes | wedged | |
|---|---:|---:|---|
| before `80ee812` (`fretwire12`–`42`) | 31 | 21 | **68%** |
| after `80ee812` (`fretwire43`–`51`, `somehinged*`) | 26 | 3 | **12%** |

So ending each 512-byte unit on a short packet was the single biggest win the write path has had,
and it is **not a cure** — 12% still wedge, with the identical signature. The hypothesis in
`write_preset`'s loop is confirmed as a cause and refuted as *the* cause.

### The credit ceiling separates all 51 writes with nothing in between

Counting every op-21 write we hold, on both devices:

| | credits received | first-credit latency |
|---|---|---|
| completed (29) | **14–19** — every chunk credited | 4.6–8.2 ms |
| wedged (22) | **1–3, never more** | 22–255 ms (one outlier at 2.8 ms) |

No write on either side of that gap. The device does not degrade under load and is not being
outrun: it stops dead after two or three chunks, and everything sent afterwards goes into an
endpoint that has already stopped draining. First-credit latency is a good tell with one standing
exception (`fretwire24`'s third write, credited in 2.8 ms, dead after the second); the credit
ceiling has none.

### The guard was three times too patient

Sharper still: **across all 29 completed writes, not one chunk ever went uncredited** — `silent`
never reached 1 — and **all 22 wedged writes reached it.** One silent chunk is the whole signal, so
`MAX_SILENT_CHUNKS` is now 1 rather than 3.

That matters because the old value no longer fired at all. In the newest logs the device wedges
after 2–3 chunks, so waiting for a third silent chunk hands the next blocking `send_frame` the
chance to time out first — which is exactly how `fretwire48` and `fretwire51` failed: a bare
`bulk OUT timed out`, then a second one, then the keepalive dropping the session, four seconds of
nothing, and none of the diagnostics the guard exists to print. It now reports
`sent`/`total`/`credits`/`first_credit_ms` in about 250 ms.

This does not save the pedal — Round-21 work (`fretwire26`) already established that aborting is
not what wedges it and that it is wedged by the time we notice. It converts a four-second freeze
into an immediate, accurate message that says a power cycle is needed and that flash was untouched.

## Round 26: it was never the split — envelope filters need level [solid]

Round 24 said a row-B block left of the split has nothing feeding it. The tester's next session put
the split between columns 4 and 5, leaving Heliosphere at column 4 outside the bracket on the left,
and our new badge duly lit up:

> a-hah!, helio, which is on L4, now has an amber warning notice "no feed"
> and interestingly, even though the Helio is "out of the loop", it still works
> so the UI logic says "NOPE", but Helix say "This is fine"

That is a direct counter-example, and it came with the explanation attached. Sorting the two
sessions by *what the blocks were* rather than where they sat:

| block | family | what he heard |
|---|---|---|
| Tron Up (`HD2_FM4TronUp`) | envelope filter — Freq, Q, **Range** | dead |
| Obi Wah, Q Filter, Mystery Filter | envelope filters — **Sensitivity**, Attack, Release | dead |
| Heliosphere, Ping Pong, Adriatic, Dual Delay | reverb / delay | audible, "hella quiet" |

Every model he called dead sweeps on input level; every model he called quiet passes signal
regardless. Split Y sits at `Balance A`/`Balance B` 0.5 by default, so each leg runs about 6 dB
down before the mixer sums them — enough that an envelope filter in path B may never open, wherever
in path B it sits. Position correlated with effect type by accident across two sessions.

The badge is gone. A row-B block outside the bracket is still worth marking, because HX Edit cannot
draw that layout at all — its path B always spans exactly the split→mixer span — but it is now a
muted grey "outside path B" note whose tooltip says the pedal keeps it and still plays it, not an
amber warning claiming it is dead. **Third time the same lesson: do not out-guard the pedal.**

To settle the audio question with no Windows and no capture: put an envelope filter in path B, play,
then raise `Balance B` (or the filter's own Sensitivity) and play again. If it comes alive, level is
the whole story.
