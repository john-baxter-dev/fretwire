# POD Go — device survey

What we know about the **POD Go**, from one contributor capture of POD Go Edit's startup
(2026-08-25, issue #15) plus the reference data shipped in POD Go Edit v2.50. Like the LT survey
this rests on no device backup; unlike it, **fretwire has never talked to a POD Go** — every claim
below comes from decoding somebody else's session, not from driving the pedal.

**Bottom line: the protocol needs no change; the *data* does.** The frame codec, the channel ids,
the handshake, the MessagePack envelope, the op numbers and the paged preset stream are all the HX
family's, byte for byte. What is different is that the POD Go indexes **its own symbol table**, so a
preset decoded against `Helix.sym` resolves every block to the wrong model.

## USB identity  [solid]

| | HX Stomp | POD Go |
|---|---|---|
| VID / PID | `0x0E41` / `0x4246` | `0x0E41` / **`0x4247`** |
| `bcdDevice` | — | `0x0200` |

Read from the device descriptor in the capture. The PID slots between the HX Effects (`0x4245`) and
the Helix Floor (`0x4248`).

## The protocol is the HX family's  [solid — 2026-08-25 capture]

Every one of our existing decoders parsed the capture **unmodified**:

| layer | result |
|---|---|
| Frame codec | `tools/pcap-frames.py` decoded every frame — same 16-byte header, `0x18`/`0x28` magic, same `cmd` bytes, same `arg` semantics |
| Channels | the same three, with the same literal ids (`ef03`/`ed03`/`f003` ↔ `0110`/`8010`/`0210`) |
| Handshake | byte-identical: `00100000` → `00020000` per channel, then the same per-channel identity opcodes `0x05` / `0x06` / `0x04` |
| Envelope | the same `{102: counter, 100: op, 101: target}` |
| Ops | 76 meters, 22 preset read, 23, 24 settings, 33 read-switch, 1 preset info, 13, 99, 254 — and it queries *the same setting ids* (14, 73) at startup |
| Preset stream | same paged `cmd=0x08` / 256-byte flow control; `tools/extract-preset-stream.py` reassembled it into a valid 4406-byte `l6-helix` stream |

The only host-side difference worth noting: POD Go Edit skips one `cmd=0x08` on the primary channel
that HX Edit sends. That is the editor's behaviour, not the device's.

### Identity  [solid]

The primary channel's identity reply returns **`"P34Main"`**, and the preset the device then
streamed carries `7 → 36` = `"P34\0"` — two independent paths agreeing, the same standard the XL's
`P36` was accepted on. Beside the model code sits `0x02500000`, matching preset key `35` and the
editor's own v2.50 version string (the Stomp's reads `0x03800000`).

## Preset structure  [solid]

Same top-level key set as the Stomp's (`0`–`7`, `10`), with one addition: **key `12` = `Array[128]`**,
unknown. Blocks, slots, params, bypass, snapshots and the footswitch/EXP assignment tables all decode
with no parser change.

Geometry read off the capture's preset:

| | value | evidence |
|---|---|---|
| DSPs | **1** | group key `0` populated, key `1` nil, all blocks in slots 1..10 |
| Snapshots | **4** | the snapshot table holds four entries, named `SNAPSHOT 1`..`4` |
| Setlists | **>1** | the host asks for preset info with `107` (bank) = 1; the capture never names them |

## The model index space is the POD Go's own  [solid]

A block's model is an index at `content → 24 → 25` into the device's symbol table. The POD Go's is
`PodGo.sym` — **627 entries against the Helix's 833** — and it is not a reordering we can compute:
three models sit exactly +8 from their `Helix.sym` entry, but Simple EQ (473 vs 132) and Fassel
(239 vs 269) break that in both directions.

The preset conveniently embeds its own model names in the footswitch section, which gave ground
truth to check against. Decoded against `Helix.sym`, then against `PodGo.sym`:

| slot | preset's own label | ref | `Helix.sym[ref]` | `PodGo.sym[ref]` |
|---|---|---|---|---|
| 1 | Volume Pedal | 224 | `HD2_PreampTweedBluesNrm` | `HD2_VolPanVolStereo` ✓ |
| 2 | Fassel | 239 | `HD2_ReverbCaveStereo` | `HD2_WahFasselStereo` ✓ |
| 3 | Scream 808 | 93 | `HD2_DelaySweepEchoStereo` | `HD2_DistScream808Mono` ✓ |
| 4 | Minotaur | 92 | `HD2_DelaySimpleDelayStereo` | `HD2_DistMinotaurMono` ✓ |
| 5 | — | 119 | `HD2_CompressorDeluxeCompMono` | `HD2_FXLoopMono1` |
| 6 | — | 21 | `HD2_AmpEssexA30` | `HD2_AmpEssexA30` |
| 7 | — | 125 | `HD2_CompressorLAStudioCompStereo` | `HD2_ImpulseResponse1024Mono` |
| 8 | Simple EQ | 473 | `HD2_MM4TriChorus` | `HD2_EQ_STATIC_Simple3BandStereo` ✓ |
| 9 | Transistor Tape | 86 | `HD2_DelayElephantManStereo` | `HD2_DelayTransistorTapeStereo` ✓ |
| 10 | Glitz | 421 | `HD2_DM4FacialFuzz` | `HD2_ReverbGlitzStereo` ✓ |

All seven labelled blocks resolve exactly against `PodGo.sym`, and the three unlabelled ones then
read as a coherent POD Go fixed chain (FX loop, amp, IR). Against `Helix.sym` the DSP figure came
out at a nonsensical 120.6%; against `PodGo.sym` it is 33.7%.

The models themselves are shared with the Helix data — same names, and the same parameter lists —
so this is one model universe, numbered differently per device.

## POD Go Edit's reference data

`res/` is a one-to-one analogue of HX Edit's, same formats under different names:

| POD Go Edit | HX Edit |
|---|---|
| `PodGo.sym` (627) | `Helix.sym` (833) |
| `PodGoModelDefs.bin` | `HelixModelDefs.bin` |
| `PGControls.json` | `HelixControls.json` |
| `PGModelCatalog.json` / `.bin` | `HX_ModelCatalog.json` / `.bin` |
| `default_preset_p34.hlx` | `default_preset.hlx` |
| `amp.models`, `cab.models`, … | *identical names* |

Because the `.models` names collide, a POD Go import lands in **`<data-dir>/pod-go/`** rather than on
top of an existing HX install. `fretwire_core::import::DataFamily` holds the mapping;
`Catalog::load_for_model("P34")` selects it.

### The two vendors disagree on the `Mono`/`Stereo` suffix  [solid]

`HelixModelDefs.bin` **strips** the suffix from its `symbolicID`s (`HD2_DistScream808`);
`PodGoModelDefs.bin` **keeps** it (`HD2_DistScream808Mono`). Both devices' *symbol tables* carry the
suffixed form, so `ModelDefs::id_by_symbolic_id` now tries the symbol as given and then its stripped
base. That is strictly better on both — 748/833 Helix symbols resolve where stripping alone got 740,
and 537/627 POD Go symbols where stripping alone got 372.

## What is still unknown

- **Writes.** No byte has ever been sent to a POD Go and none of the `edit` builders has been
  reconciled against one. The capture is POD Go Edit's startup — read traffic only.
- **Setlist geometry.** How many setlists, how big, and how the panel numbers presets.
- **Footswitch layout.** We map a block to a switch by layout position + 1, which is the Stomp's
  rule; it currently reports things like `FS9` on a four-switch pedal, so the POD Go's mapping is
  its own and unmeasured.
- **Fixed topology.** The POD Go's chain has dedicated wah / volume / amp / cab / EQ positions
  rather than free blocks, so add / move / delete-block semantics need their own work. `insert_block`
  now refuses a slot outside the HX row windows instead of panicking on one (the POD Go's ten blocks
  are one row, so slots 9 and 10 hold blocks where the HX has bounding nodes).
- **Preset key `12`** (`Array[128]`).
- `preset_device_id`, which on the Stomp and Floor came from a `.hxb` header we do not have.

## The write path is the same too  [solid — 2026-08-26 captures]

The contributor sent the three captures asked for. Every builder in `fretwire_protocol::edit`, all
of them written from HX Stomp traffic, reproduces the POD Go's bytes **exactly** and unchanged
(`crates/fretwire-protocol/tests/pod_go_writes.rs`):

| edit | op | body | builder |
|---|---|---|---|
| slot 6 param 0 (Amp → Lead Gain) | 30 | `{98: 6, 29: true, 26: 0, 28: 0, 119: v}` | `set_value` |
| bypass slot 9 (Reverb, Dynamic Room) | 41 | `{98: 9, 59: false}` | `bypass` |
| slot 8 Elephant Man → Adriatic Delay | 40 | `{98: 8, 100: {23: false, 25: 75, 26: -1}}` | `swap_model` |

The sharpest evidence is the bypass body, against the HX Stomp's captured one in
`fretwire-protocol/tests/golden.rs`:

```
HX Stomp  8366cd03f1 6429 6582 62 07 3bc2
POD Go    8366cd03f1 6429 6582 62 09 3bc2
```

One byte apart — the slot number. (The matching transaction counter is coincidence; both captures
happened to sit at 1009.)

After a model swap POD Go Edit re-reads the block's footswitch (op 33) and then the whole preset
(ops 23, 22) — the same refresh HX Edit performs, and the same three builders.

## The Mono/Stereo suffix, again — and it cost whole categories  [solid]

The two editors disagree on whether a `symbolicID` keeps its `Mono`/`Stereo` suffix, and the split
is near-total in both directions:

| | keyed by base | keyed by full suffixed symbol |
|---|---|---|
| `HelixModelDefs.bin` | 344 | 8 (the DL4 legacy delays) |
| `PodGoModelDefs.bin` | **0** | **180** |

The model picker looked models up by the stripped base, so on a POD Go all 180 suffixed models
disappeared — every Wah and Reverb (both categories are entirely `…Stereo`), plus EQ, Volume/Pan,
IR, Looper and most of Delay. Six categories gone and Delay down to 10 entries. Looking the symbol
up **as the table spells it** restores them, and picks up the eight DL4 delays that were quietly
missing from the *HX* picker too:

| category | POD Go before | after | HX before | after |
|---|---|---|---|---|
| Wah | — | 11 | 11 | 11 |
| Reverb | — | 23 | 25 | 25 |
| Delay | 10 | 43 | 40 | **48** |

### Captures that would still help

- A `.hxb` backup from a POD Go — the one thing that would fill in `preset_device_id`, the only
  field keeping the device at `Reported` rather than `Verified`.
- Anything that names the setlists, or shows how the panel numbers presets.
- A footswitch assignment, to replace the Stomp's layout rule we currently apply blind.
