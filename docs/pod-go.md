# POD Go — device survey

What we know about the **POD Go**, from an owner's captures, backup and hardware reports
(2026-08-25 through 2026-08-28, issue #15) plus the reference data shipped in POD Go Edit v2.50.
The device is **`Support::Verified`**: every field in its `Device` entry is measured, and every
edit builder we have reproduces its captured bytes exactly. The owner has also driven a POD Go
from fretwire itself, both directions.

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
| Setlists | **2** | `Factory` and `User`, 128 slots each — named by the backup, sized by the wire (see below) |

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

- **Structural writes.** Parameter, bypass, model-swap and footswitch assignment are reconciled
  (below); what is *not* is anything that changes the chain's shape — add, move, delete. The
  POD Go's chain is **fixed** (see "The fixed chain" below), so those ops may not even exist for
  it in the form the HX uses; `insert_block` refuses a slot outside the HX row windows rather than
  guessing. A POD Go owner has replaced the fixed wah slot with a delay via the JSON
  export-edit-reimport route and the device accepted it, which we do not yet understand and should
  not lean on.
- **The IR block's sixth stored value.** Beside its five symbol parameters the wire stores one
  more (`6` beside `Index: 7` in the one sample held) whose rule can't be pinned from a single
  observation — 0-based mirror of Index? an `irUuidTable` position? Until a second IR preset is
  seen on the wire, `hxb-convert` refuses IR blocks rather than guess what lands in flash. The
  looper's slot shape is likewise unobserved (its HX cousin uses its own slot kind). Everything
  else converts — see "Backups convert" below.
- **Preset key `12`** (`Array[128]`).

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

## The backup — `.pgb` is `.hxb`, and it closed out `Verified`  [solid — 2026-08-27, issue #15]

POD Go Edit exports backups as `.pgb`. Same `AF6L` container as HX Edit's `.hxb` — and the file
whose payload did *not* start at the `.hxb`'s assumed fixed offset is what forced the container's
real structure out: a **tagged archive with a 36-byte-per-entry index table at the end**, fully
decoded in `fretwire-data/src/hxb.rs`. (The Floor's `.hxb` re-reads identically under the table —
its comment happens to be exactly the 64 bytes the old fixed-layout reading assumed.)

What the backup settled:

- **`preset_device_id` = `0x210007`**, twice over: the container header's device-id field and the
  `L6UMDArchive` section's `"device": 2162695`. That was the last unknown `Device` field — the
  POD Go is now `Support::Verified`.
- The header's device-version field reads `0x02500000`, matching the identity reply's — which
  confirms that reply's second word is the *version*, not the device id.
- **Two setlists, `Factory` and `User`, 128 slots each** (123 and 12 populated in this one), from
  the `SLNM` section, both setlists' own `meta.name`, and the owner's panel description agreeing.
- POD Go Edit writes **no comment** (`DESC`) section and stores **only the populated IR slots**
  (7 here), where HX Edit writes all 128.
- The tone JSON is the HX shape with the geometry differences already known from the wire: `dsp0`
  only, blocks `block0`..`block9`, four snapshots — and **no `@path`** on blocks (one row makes it
  meaningless), which is what `hxb-convert` trips on (see "still unknown").

## Setlists and banks  [solid — 2026-08-27 capture + owner report]

The panel calls the two setlists **Factory** and **User** — functionally identical, the names
describe pre-population — and numbers slots `01A`..`32D`, four to a bank
(`presets_per_bank: Some(4)` reproduces exactly that as `01A-32D`). On the wire nothing is new:

- The startup `read_info` answers `{107: 0, 108: 0, 109: "US Deluxe Nrm"}` — bank 0, slot 0,
  matching the backup's Factory 01A.
- Switching lists in the editor is just a **browse of the other bank** (op 1, `{107: 1}`), and the
  reply keys are global indices with the familiar 128 stride (first populated User entry arrives
  keyed 136 = 1 × 128 + 8). No mode-switch message exists.
- The owner reports the *pedal-side* bank switch emits USB-**MIDI** Bank Change + Program Change
  (which the capture, taken on the `MI_00` interface only, does not carry) and that the editor
  sends no equivalent — the two preset lists scroll independently until a load. [reported]

## The footswitch map  [solid — 2026-08-27 capture]

Assigning slot 6's bypass to the switch POD Go Edit calls **FS3** produced op 56 with
`{98: 6, 102: 2}`, and the follow-up read asked op 33 for `102: 3` — the HX Stomp's exact opcodes
**and** its off-by-one-on-purpose numbering (one-based to read, zero-based to write). Both are
golden-tested in `pod_go_writes.rs`. So the wire rule was never wrong — position + 1 *is* the
FS label for the stomp switches.

The `FS9`-on-a-small-pedal display was the **expression toe switch**: in the backup's tones the
wah and the volume pedal both carry `@fs_index: 9` (one enabled at a time — the toe toggles
between them), so wire layout position 8 is the toe switch, not a ninth stomp. The status push
after the assignment is the familiar type 31: `{98: slot, 70: switch, 79: assigned}`.

## The fixed chain  [reported — 2026-08-27, issue #15]

Per the owner, POD Go Edit and the pedal both enforce that every preset contains exactly one each
of **volume pedal, wah, FX loop (mono or stereo), amp, cab/IR**, plus **at least one EQ** — in any
order, leaving four freely assignable blocks. The editor implements this as fixed-type slots. The
community bypasses it by editing a preset's exported JSON and reimporting, with no widely-reported
ill effect; the restriction is assumed part DSP budget, part product segmentation. This is why
add/move/delete may be a different shape here than on the HX chain, and why we don't send them.

## The wire slot array  [solid — read from the 2026-08-27 capture's preset]

The POD Go's group-0 slot array holds **12 entries**, not the HX's 20:

```
[0]      kind 0   input node
[1]..[10] kind 6  the ten chain blocks
[11]     kind 1   output node
```

No split or mixer nodes — the chain is one row, structurally. The footswitch layout array
(`3 → 8`) is **9 positions**: 0..5 are the stomp switches FS1..FS6, position 8 is the expression
toe switch (see "The footswitch map"), and positions 6..7 have not been seen carrying anything.

## Backups convert  [solid — 2026-08-28]

`fretwire hxb-convert` turns a `.pgb` into restorable presets, verified against the two presets
held in **both** forms — the backup's tone JSON and the same unit's own wire stream ("US Deluxe
Nrm" and "AC30 Ambient"). Converting the tone reproduces the device's preset slot for slot:
every block, class, parameter value, the footswitch layout with its toe-switch pair, the
controller table and all four snapshot matrices (`fretwire-data/tests/pgb_to_wire.rs`). Of the
owner's 135 presets, 101 convert; the 34 refusals are exactly the IR and looper blocks above.

What the POD Go's tones do differently (each reconciled, not assumed):

- No `@path`; slot = `@position + 1`. Empty slots are written as bare `{"@position": n}` stubs
  where HX Edit omits the entry. The structural entries are `input`/`output`, not `inputA`/…
- Its **own `@type` vocabulary**: 0 effect/EQ/cab, 1 amp, **2 IR**, **4 looper**, **5 trails**
  (FX loop, delay, reverb) — against the HX's 5 = IR, 6 = looper, 7 = trails.
- Its **own class bytes** where the HX disagrees: EQ = 23 (HX: 1 — and 23 is the HX's *synth*),
  cab = 26 (HX: 31), FX loop = 9 (HX: 1), IR = 15 (HX: 19). Amps (17), plain effects (1) and
  delay/reverb (8) match. See `pod_go_block_class`.
- Controller rows stop at key 7 (no HX key 13), and snapshot matrices default **true** on the
  input/output cells where the HX holds false.
- **POD Go Edit rounds parameter values to three decimals in the backup** (`"Tone" : 0.270`
  against the wire's `0.26999998`) — so any restore from a `.pgb`, by anyone, is at that
  precision. The `.hxb` does not round.

### Captures that would still help

- **Anything showing add / move / delete from POD Go Edit**, if the editor can do them at all —
  or confirmation that it refuses. This is the one remaining write family.
- **A wire read of any second IR preset** (`dump-raw` once fretwire is connected, or a capture of
  POD Go Edit opening one) — two samples of the IR's sixth stored value would pin its rule and
  lift the one conversion refusal that matters (32 of the owner's presets carry an IR).
