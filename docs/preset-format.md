# Preset MessagePack format (device stream)

When a preset is opened, the device streams it as MessagePack (see `docs/protocol.md` for the
transport). `fretwire_data::stream::PresetStream::parse` decodes a reassembled stream. Validated
against `captures/preset1_stream.msgpack.bin` (HX Stomp, fw 3.80 — the `v3.71` in its key-37 stamp
is a build id, not the firmware it ran; see below). Integer keys below are
**observed**; names are inferred (often by correlation with the `.hlx` JSON schema).

## Envelope
Reassembled stream = `{102: <u32>, 103: 0, 104: <blob>}` (MessagePack, root ~8 bytes into the
stream after a `marker/type/len` header). Key **104** is a `str`/`bin` blob = the preset.

> **Byte 3 of the header is volatile — don't diff on it.** [solid — 2026-08-02] The high byte of
> `type` changes between reads of an *unchanged* preset: twelve consecutive `dump-raw` runs on one
> Stomp preset split into two groups differing at offset 3 alone (`0x00` / `0x28`), and the field
> dumps a tester sent as three presets turned out to be one preset three times, differing at offset
> 3 and nowhere else (`0x00` / `0x28` / `0x10`). Everything from offset 8 was byte-identical. The
> parser skips the header, so this reaches nobody but a person running `cmp` — but it makes two
> dumps of the same preset look different and, worse, made three dumps of one preset look like
> three presets. Compare with `fretwire diff-stream`, which walks the tree, or from offset 8.

## Blob = a flat sequence of 3 MessagePack values
1. `str "l6-helix\0"` — magic.
2. `str` — the **offset table**, 48 bytes = 12 little-endian `u32`s. See below.
3. `Map` — **the preset** (integer-keyed).

### The header is an offset table, not a uuid  [solid — 2026-07-31]
Every slot is a byte offset into the blob (offset 0 = the blob's first byte). On all four presets
captured so far:

| slot | value | points at |
|---:|---|---|
| 0 | 61 | the **preset map**'s first byte (always 61: 10-byte magic + 3-byte `str16` marker + 48-byte header) |
| 1–9 | varies | the first byte of one **top-level preset entry** — the key byte, not its value. The slot→key mapping is a fixed permutation (`0,1,3,4,2,5,6,7,10` on a Floor preset), so slot 8 = 62 addresses whichever key the map happens to serialize first |
| 10, 11 | blob length | the blob's **total size**, twice |

The device seeks with this table rather than walking the MessagePack, which makes it load-bearing on
write. **This cost us two device lockups.** `to_blob()` re-encodes the map with rmpv, which writes
integers minimally where the device does not — the device emits `d1 00 00` (int16 zero) and `cc 00`
freely — so an untouched preset re-serializes **117–216 bytes shorter**, with everything after the
first such integer shifted left. We copied the header verbatim across that shift, so the device
followed offsets that no longer began anything and a declared total length past the end of the
buffer it had been given. It stopped draining its endpoint mid-transfer and needed a power cycle.

`to_blob()` now rebuilds the table against the bytes it actually emits (`classify_header` records
what each slot addressed at parse time; the offsets are re-derived on write). Byte-identity with the
device's own encoding is not achievable through rmpv and is not required — **self-consistency is**.

**Which sections the stale offsets pointed at matters.** On `fretwireTest2` the three wrong interior
slots addressed keys **5, 6 and 10** — preset settings, the focused block, and *the snapshots*, whose
`3` field is per-slot state for every block. A device reading those from wrong offsets keeps a
plausible-looking block layout while its per-snapshot block state is garbage, which is a route to
"the chain looks right and makes no sound". The offsets before the first shifted integer (the slot
arrays, key 0/1) stayed correct, which is why the block list always survived. [hypothesis]

### Serial vs parallel: what actually differs  [solid — 2026-07-31]
The split (kind 2) and mixer (kind 3) nodes are **always present**, at slot-array indices **10 and
19**, even on a fully serial preset. Only four fields distinguish the two topologies:

| field | serial | parallel |
|---|---|---|
| DSP group key `21` (split type) | `0` | non-zero (`1` on both Stomp captures; `2`/`3` seen on a Floor) |
| split node `20 → 18` | `false` | **`true`** |
| split node `20 → 15 → 13` (column) | `0` | the split's column (2, 5) |
| mixer node `20 → 18` | `false` | **`true`** |
| mixer node `20 → 17 → 13` (column) | `0` | the mixer's column (7, 9) |

Everything else is identical — a serial preset already carries the Y-split model (`15 → 8 = 257`) and
the mixer model (`17 → 8 = 151`), both with `10 => true`. So "make this preset parallel" is not about
creating nodes; it is about **enabling the two that are already there and giving them columns**.

None of those five fields has a known surgical op — the column is documented as op-21-only. **The
device sets all five itself on an op-43 move into a row-B slot** — verified 2026-07-31 against a dump
taken straight after a drag, which came back structurally identical to a device-authored parallel
preset (`key21=1`, both nodes `18: true`, columns 2 and 9, block at slot 12). So `move_block_to_row`
sending op 43 alone is correct, and the doc comment claiming the device activates the split was right.

### The split and mixer node parameters  [solid — 2026-07-31]
Their param arrays live at `20 → <holder> → 7 → 4` (holder `15` for the split, `17` for the mixer),
and the **`Enabled` param is not in that array** — it is the node content's key `18`. So the stored
array is always one shorter than the model's param list:

| node | model | stored params |
|---|---|---|
| Split Y | 257 `HD2_AppDSPFlowSplitY` | `Balance A` (def 0.5), `Balance B` (def 0.5), `bypass` |
| Split A/B | 256 `HD2_AppDSPFlowSplitAB` | `Route To` (def 0.5), `bypass` |
| Mixer | 151 `HD2_AppDSPFlowJoin` | `A Level` (def 0, −60..12 dB), `A Pan` (0.5), `B Level` (0), `B Pan` (0.5), `B Polarity` (false), `Level` (0) |

The bool in each array (`bypass` / `B Polarity`) pins the alignment, so the mapping is not guesswork.

## The host side: `tone` JSON ↔ this format  [solid — 2026-08-25]

An `.hlx` preset file and every slot of an `.hxb` backup carry the same preset as a **`tone`
object** — named models, named parameters, `@path`/`@position` instead of slot numbers. That is
the format HX Edit saves and shares, so converting it to this one is what a restore or an `.hlx`
import needs. `fretwire_data::tone` does it; the reconciliation is below and in that module.

The check is a preset held in **both** forms — a contributor's Floor backup and a wire dump of the
same slot off the same unit — so this is measured, not inferred. All 15 blocks, both DSPs, both
split topologies and all 106 parameter values agree.

| tone | wire |
|---|---|
| `@model` + `@stereo` | the device symbol's index in `Helix.sym` (`24 → 25`) |
| named parameters | `11 → 4`, in that symbol's parameter order |
| `@path`, `@position` | slot index = `@path × 10 + @position + 1` |
| `@enabled` | content key `10` |
| `@mic`, `@trails` | one value appended past the symbol's parameters |
| `@type` | content key `9` (table above) |
| `global.@topologyN` | DSP group key `21` |
| `footswitch.dspN.blockM` | key `3 → 8`, transposed: `@fs_index` − 1 is the array position |
| `@fs_label` / `@fs_ledcolor` / `@fs_enabled` | `11 → 5` (NUL-terminated) / `11 → 6` / `11 → 7` |
| `@fs_customlabel` / `@fs_momentary` | `14` (NUL-terminated, `13` says whether there is one) / `12` |
| `@fs_customcolor` | `16`, a palette index, with `15` saying whether there is one |
| `snapshotN.@name` / `@tempo` / `@valid` | snapshot keys `4` (NUL-terminated) / `5` / `0` |
| `snapshotN.@pedalstate` / `@ledcolor` / `@custom_name` | snapshot keys `11` / `12` / `14` |
| `snapshotN.blocks.dspN.<name>` | snapshot key `3[wire slot][1]` |

Two things here are counter-intuitive enough to be worth stating on their own.

**`@stereo` is written only when the model has both variants.** 153 of the 680 symbols do; the rest
are `Stereo`-only (36, including every reverb), `Mono`-only (12) or unsuffixed (479, the amps and
cabs). So an absent `@stereo` means *the variant that exists*, not "Mono" — reading it as Mono
makes `HD2_ReverbHall` unresolvable, and reading it as a default would pick a symbol with a
different parameter order.

**Whether a split or mixer is a real branch point comes from the topology string, not from the
node's own `@enabled`.** Both nodes of a bracket that spans two DSPs report `@enabled: true` on
both, while the device has DSP1's join and DSP2's split *inactive* (`20 → 18` false, column 0) —
because the split opens on DSP1 (`SAB`) and the join closes on DSP2 (`ABJ`). The node's `@enabled`
is its own bypass and lands on the holder's key `10`.

**An amp+cab block's `@cab` is a sibling reference, not a model.** `@cab: "cab0"` names another
entry of the same `dspN` object, which holds the cab's own `@model`, `@mic` and parameters. The
pair is one block on the wire, with the cab at `24 → 26` and its parameters in bank `12`. A dual
cab (`@type` 4) is the same shape with a cab as the main model. **Which symbol the index names
stopped being an open question on 2026-08-26**: legacy `HD2_Cab…` and new `HD2_CabMicIr_…` are
both ordinary `Helix.sym` entries and a paired cab stores whichever family the preset uses — the
supposed tone↔wire family mismatch was two observations from different presets (a 3.80-built pair
uses a new cab; the backup's `HD2_Cab…` siblings are genuinely legacy cabs). Measured by pairing
every combination on a live HX Stomp — `captures/pairing_sweep.md`.

**The tone's parameter-name spelling drifts by HX Edit era.** One backup stores the same legacy
cab's high cut as `HighCut` 142 times and `High Cut` 26, with a stray `Low Cut` and
`Early Reflections`. Name lookups in `fretwire_data::tone` match with spaces stripped and case
folded for this reason.

**A footswitch binding stays in the layout when `@fs_enabled` is false**, carrying `11 → 7` false —
that is a block assigned to a switch and not currently answering to it, not an unbound switch. One
switch can hold several bindings, as an array, ordered with the `@fs_primary` one first and each
entry's position recorded at `10`. [solid — the oracle preset has both cases]

**Path B's input and output nodes live inside the structural slots**: the split slot's key `14` is
the tone's `inputB` and the mixer slot's key `16` is `outputB`, the same shape as slot 0 (`inputA`)
and slot 9 (`outputA`). All four store a **ragged prefix** of their symbol's parameter list — DSP1's
input keeps 3 of 8, DSP2's keeps none — and the rule behind the prefix length is not known, which is
why `tone` leaves those four alone.

## Preset map (integer keys)
| key | value | meaning (inferred) |
|----:|-------|--------------------|
| 7 | Map `{36: "P33", 35: 58720256, 37: "v3.71-32-g1039661"}` | device info: **36 = model code** (`P33` = HX Stomp family), 35 = version (0x03800000), 37 = build stamp — *not* the running firmware, see below |
| 0 | Map `{21: split, 22: Array[20]}` | **block slots** of the first DSP (`21` = split type) — see below |
| 1 | Map `{21: split, 22: Array[20]}` — or nil | **the second DSP's slot array**, same shape as key `0`. **nil on the HX Stomp** (one DSP), populated on the Helix Floor. [solid — 2026-07-22, Floor captures cross-checked against a `.hxb` backup] |
| 2 | Map `{0: Array[13], 1: Array[13×Array[7]]}` | snapshot/controller matrices (13 = snapshots? all zero here) |
| 3 | Map `{7: 0, 8: Array[5]}` | **footswitch / stomp layout** — bound blocks only; see below |
| 4 | Array[10] (all nil) | **parameter-controller** assignments (separate from `3 → 8`; empty here). Ten because this is a Stomp — the length is `footswitches + 5`, so an XL holds 13 |
| 5 | Map{15} `{16: f32, 45..56: …, 30, 134}` | **preset-level settings**. Byte-identical across all three captures (all at defaults), so the fields aren't separable yet; `16` = f32 80 is most likely the preset tempo. [hypothesis] |
| 6 | Map{2} `{98: <slot>, 26: 0}` | **the focused block** — key `98` is the same slot number the edit commands address, and it differs per capture (5, 7, 12), matching the block last selected. [solid] |
| 10 | Map{6} `{6,7,8, 9: 20, 10: Array[n], 13: Array[20]}` | **snapshots**: `10` is the snapshot array (3 on a Stomp, 8 on a Floor), `9` = the slot count, `13` = a per-slot array. Each snapshot is `{0: enabled, 1: Array[11], 2: Array[64], 3: Array[20], 4: name, 5: f32 tempo, 12, 14}` — note `3` is **per-slot state**, one entry per block slot. [solid] |

### Neither `7 → 37` nor `7 → 35` is the pedal's firmware version  [solid — 2026-08-21]
Both look like one. Neither is, and labelling either "fw" tells the user their pedal is on a version
it isn't — reported as exactly that in issue #4.

| | HX Stomp (on **3.80**) | HX Stomp XL (on **3.80.0**) | Helix Floor (on **3.82**) |
|---|---|---|---|
| key `7 → 37` | `v3.71-32-g1039661` | `v3.71-32-g1039661` | `7d01f5e` |
| key `7 → 35` | `0x03800000` | — | `0x03800020` |
| `.hxb` header `0x1c` | — | — | `0x03800000` |

**Key 37** is a build id. A single pedal refutes the firmware reading — a Stomp on 3.80 stamps
`v3.71` — and the XL only adds that it is not per-unit. Read the suffix literally and the
contradiction dissolves: `-32-g1039661` is `git describe` for *32 commits past a tag named `v3.71`*,
so it names a build of something inside the firmware image whose last tag was 3.71, not a release.
The Floor's bare sha is the same thing with no tag behind it. [hypothesis]

**Key 35** is not the fallback: `0x03800000` appears on this 3.80 Stomp *and* in a 3.82 Floor's
backup header, so it did not move across those releases. A format revision that reads "3.80" is the
likely story [hypothesis] — and the reason it went unchallenged is that it agreed with one pedal by
coincidence.

So **no field we have decoded reports the version on the pedal's boot screen**: the live identity
reply carries that same `0x03800000`. Worth revisiting if op 25 (globals) is ever decoded.

Exposed as `PresetStream::build_stamp()` / `EditorPreset::build_stamp`, deliberately not `firmware`.

### Block slots (`0 → 22`, Array of 20 — and `1 → 22` for the second DSP)
Each slot is `{19: type, 20: content}`:
- `type 8` + `nil` → **empty slot**.
- `type 6` + `Map{5}` → **populated effect block** (5 params). 6 such blocks in this preset,
  matching the names seen in the stream (Bucket Brigade, Harmonic Tremolo, Tremolo, 70s Chorus,
  Dynamic Hall, …).
- `type 7` + `Map{4}` → **a Looper block**. Same *idea* as type 6 but a **different content shape**
  — see below. Not device-specific: it simply never appeared in the Stomp fixtures (none of them
  contain a Looper), and turned up in the Helix Floor captures.
  **[solid — 2026-07-22, cross-checked against a `.hxb` backup]**
- other types (`0,1,2,3`) → structural slots (input/split/join/output).

**The array is 20 slots on both devices.** The Helix Floor does not widen it — it uses a *second*
array at preset key `1` for DSP2. So a full block enumeration must walk **both** `0 → 22` and
`1 → 22`; reading only key `0` silently drops every DSP2 block (verified: a Floor preset whose
Looper lives on DSP2 came back one block short until key `1` was walked).

#### The wire slot number is **global** across DSPs  [solid — 2026-07-23]

Edit ops address a block by key `98` = a **single** slot integer with no DSP qualifier, and that
integer spans both arrays:

```
wire_slot = dsp * 20 + index_in_that_dsp's_array
```

so DSP1 is slots **0–19** and DSP2 is slots **20–39**. This is device-independent framing: the Stomp
simply never exceeds 19 because its key `1` is nil.

Established from a Helix Floor capture of five DSP2 blocks being edited in HX Edit (`WinCap5`,
`FACTORY 1` `12B` "Pull Me Under"). Each `op 78 {98: n}` / `op 30 {98: n, 28: p, 119: v}` pair
resolves under this rule to a block whose stored value for param `p` is one UI increment from the
first value on the wire — five independent matches across five models and three different parameter
scales, including correctly picking the branch-B one of two identical `HD2_DelaySimpleDelayMono`
blocks (index 17, not 7). It is also consistent with every earlier capture, whose slots were all
< 20 and all resolved to DSP1. Full working in `docs/helix-floor.md`.

Independently corroborated *inside* the preset: the **footswitch layout** (`3 → 8 → … → 11 → 8`)
numbers its targets the same way. In that Floor preset FS4/FS5 point at slots 27/28 and FS10/FS11 at
37/38, and each entry's name matches the model found at that **global** slot — including telling the
two identical `Simple Delay` blocks apart as 27 and 37. Two different tables, one numbering.

**Consequence:** `fretwire_protocol::edit` needs no change for multi-DSP devices. The `(dsp, index)`
pair is an internal detail of preset traversal; flatten it with the formula above before it reaches
the wire. Implemented in `fretwire_data::stream` as `wire_slot(dsp, index)` /
`split_wire_slot(slot)`, with `DSP_SLOT_STRIDE = 20`.

The block content maps hold the per-param **values** (seen as msgpack `float32`, e.g. `ca 3f800000`
= 1.0) — the same big-endian f32s we set via op-0x06. Mapping these slots/params to the wire
**handles** (`8X 62 [block] NN`) is the remaining correlation (next step).

### Block content (`0 → 22 → [i] → 20`, the param values)
A `type 6` block content is `Map{5}`:
- `10` = **`enabled` bool** (`true` = block active, `false` = bypassed). **[solid — verified live
  2026-06-23]** by toggling a block and diffing the stream (`fretwire diff-stream`). The block bypass
  state lives **here**, not in the `24` metadata. (An earlier draft wrongly read `24 → 23`.)
- `24` = `{23: has-paired-cab, 25: model index, 26: paired index}` (metadata). **`23` is the
  pairing flag**, not a bypass: it is `true` on exactly the blocks whose `26` is not `-1`, on every
  amp in every fixture held. [solid — 2026-08-25]
- `11` = `{2: stored, 3: from-symbol, 4: [values]}` — **the ordered param vector** (msgpack
  `float32` for knobs, `int`/`bool` for enums/switches), in `Helix.sym` device order.
  **The two counts are different numbers and say so:** `3` is the model symbol's own parameter
  count and `2` is how many values are stored, and they differ by exactly one on the blocks that
  append a trailing extra — a cab's **mic** and a delay/reverb's **trails** switch. So a cab reads
  `{2: 6, 3: 5}` and a Simple Delay `{2: 7, 3: 6}`. [solid — 2026-08-25, 24 distinct symbols]
- `12` = the **paired model's** param vector, same `{2, 3, 4}` shape — the cab's parameters on an
  amp+cab or the second cab's on a dual, empty on everything else, laid out exactly as that
  model's own bank `11` would be. Two family-specific rules, both measured live
  (`captures/pairing_sweep.md`): a **new** (`HD2_CabMicIr_…`) cab lists `Mic` first and does not
  store the symbol's trailing `IrData` (7 of 8; 9 of 10 on a `WithPan`), while a **legacy**
  (`HD2_Cab…`) cab appends its mic after the symbol's five (`{2: 6, 3: 5}`). [solid — 2026-08-26]
- `27` = on an **IR block only**, in place of bank `12`: the referenced IR's UUID as a
  NUL-terminated hex string — the tone's `@uuid`, and how the device re-matches an IR by content
  when slots have moved. A dual IR concatenates its two UUIDs into the one string; an IR block
  aimed at an empty slot stores `"\0"`. [solid — 2026-08-26, live]
- `9` = the **block class**, a small fixed number per kind of block. It tracks the tone's `@type`
  *and* the model family — the device tells the two cab generations apart, and a dual IR is its
  own class. Every row measured against device-written bytes; the bolded ones by the 2026-08-26
  live sweep on an HX Stomp (`captures/pairing_sweep.md`):

  | `9` | tone `@type` | what |
  |---:|---:|---|
  | 1 | 0 | any ordinary effect — EQ, comp, dist, mod, wah, vol/pan |
  | 8 | 7 | delay and reverb, i.e. exactly the trails-capable blocks |
  | 15 | 2 | a legacy cab on its own |
  | **16** | 4 | a dual **legacy** cab (`26` = the second cab) |
  | 17 | 1 | an amp or preamp on its own (`26` = −1) |
  | **18** | 3 | an amp + **legacy** cab pair |
  | **19** | 5 | an IR block (mono) |
  | **21** | 5 | a **dual** IR block |
  | **22** | 6 | the looper — a different slot kind (7), model index at key `8` |
  | **23** | 8 | a synth block |
  | **31** | 2 | a **new** (`CabMicIr`) cab on its own |
  | **32** | 4 | a dual **new** cab (two `WithPan` symbols) |
  | 33 | 3 | an amp + **new** cab pair (`26` = the cab, `23` = true) |

  The pattern — 15..19 consecutive, +16 where the new cab engine is involved — is descriptive,
  not assumed: each cell is its own measurement. `fretwire_data::tone` builds all of these now;
  the only remaining per-block refusal is a mixed-family dual cab, which no device-written preset
  has ever shown.

### Block content for `type 7` (Looper) — a different shape
A `type 7` slot's content is `Map{4}` and does **not** follow the type-6 layout:
- `8` = **model index** into `Helix.sym` — directly, *not* nested under `24 → 25`
  (e.g. `153` = `HD2_LooperStereo`).
- `10` = **`enabled` bool** — same key as type 6.
- `7` = `{2: count, 3: count, 4: [values]}` — the ordered param vector, at key **`7`**, not `11`
  (e.g. `[0.0, 0.0, 20.0, 20000.0]` = `HD2_Looper`'s `Playback, Overdub, lowCut, highCut`).
- `9` = a secondary id/flag.

This is the same `{8: model, 10: enabled, 7: params}` shape the **structural** nodes use inside
their sub-maps (`15`/`17`), so type 7 reads more like a structural node than an effect block.

### Signal path (`3 → 8`, the block identities)
`Array[5]`, one entry per path position (`nil` = empty). Each populated position is `Array[1]`
of a `Map{7}` node:
- `11` = `{0: <node type>, 5: "<name>", 6: <model id>, 8: <slot>, …}`. **`11 → 0` = node type:
  `1` = DSP block, `2` = controller/footswitch node.** For a type-1 block, `5` = model display
  name (matches `.models` `name`); for a type-2 node, `5` is the **parameter's** name, `6` is *not*
  a model id (won't resolve), and `11 → 9` = `{28,29,41}` (the same param-descriptor shape used in
  the assignment table — see below). [solid — verified live]
  > Refined 2026-08-22 by *making* one: op 37 (`docs/protocol.md`) assigned `Mix` on slot 16 to FS1,
  > and the layout gained `{10: 0, 11: {0: 2, 5: "Mix\0", 7: false, 8: 16, 9: {…}}}`. So a type-2
  > entry is a **parameter controller**, and key 5 names the parameter — the earlier `"OD Sw"`
  > reading was a switch *parameter*, not a free-text label. A parameter assignment therefore appears
  > **here as well as** in key 4; a bypass appears only here, as type 1. `loaded_blocks` drops type-2
  > entries before enriching a block's `footswitch`, so a block with only a knob on FS1 is not badged
  > as being on FS1.
- `14` = custom label string; `13` = has-label flag (e.g. a `Harmonic Tremolo` renamed `Tremolo`).
  `16` = custom LED colour; `15` = has-colour flag — the same value-plus-gate shape. Op 33 mirrors
  `14` as its `109` and `16` as its top-level `66` exactly when the gate is true [solid — flag-flip
  experiments through the op-21 write path, live HX Stomp, 2026-08-27]. `16`'s value is
  `@fs_customcolor`'s **palette index** (1–10), not `0xRRGGBB` `[hypothesis` — never observed on a
  wire document, but HX Edit backups report `@fs_ledcolor` as raw RGB and `@fs_customcolor` as a
  small index, and both come from reading the device`]`. Which index is which colour is unmapped.

### Controllers / footswitch assignments (`4`), snapshots (`10`/`2`) [solid — corrected 2026-08-21]
- **`4` = `Array[footswitches + 5]`** — **parameter**-controller assignment table, **indexed by
  source ordinal**:
  one position per physical control, `nil` where that control drives nothing. A populated entry is an
  `Array` of `{0:<place in table>, 1: Map}` — **one item per assignment on that source**, so a control
  driving two things has two items (observed: EXP1 with two, `captures/xl_exp_bypass.msgpack.bin`).
  A target slot **need not still hold a block** — the same capture has an EXP1 entry aimed at an
  empty slot 1, so a consumer must treat the slot lookup as fallible. The inner map is
  `{0:<source>, 1:<value type>, 2:<min>, 3:<max>, 5:<target slot>, 6:{28:<path>, 29:<param idx>, 41}, …}`.
- **The parameter index is `6 → 29`; `6 → 28` is the model path.** This is the reverse of the op-37
  *request* that creates an assignment, where 28 carries the index — and reading the request's shape
  here reported parameter 0 for every assignment in every preset, because the path is 0 throughout.
  [solid — assigning `Mix` (index 2) to FS2 on a Stomp and diffing gives `6: {28: 0, 29: 2}`;
  `captures/assign_two_footswitches.msgpack.bin`, pinned by `fretwire-data/tests/assignments.rs`.]
  The bug hid for as long as it did because an assignment onto parameter 0 reads correctly either way.
- **The travel ends are keys `2` and `3`, not `4`/`7`** (which are `0` on every sample held). They are
  in the parameter's own raw units and follow its type: `false`/`true` for a switch, `0`/`1` for a
  0..1 knob, `0`/`8` for a delay time. [solid — three samples]
- **Source ordinal → physical control — the table is the device's size, not a fixed ten.**
  [corrected 2026-08-25, issue #13] The layout is: `0` none, `1`/`2` the expression inputs, **one
  entry per footswitch from 3**, then MIDI, then snapshots. So

  | | HX Stomp (5 switches) | HX Stomp XL (8) |
  |---|---|---|
  | footswitches | 3..=7 | 3..=10 |
  | MIDI | 8 | 11 |
  | snapshots | 9 | 12 |
  | `Array` length | 10 | 13 |

  **`length == footswitches + 5` on all ten streams we hold** — six Stomp captures at 5 and 10,
  four XL captures at 8 and 13. [solid]

  **FS1 = 3 on both**, established by diffing a front-panel assignment and again by writing one with
  op 37. The run's far end is now observed too: an XL owner assigned a Stupor OD's `Drive` to FS6
  from the front panel and it filed itself at **ordinal 8** — the index a Stomp calls MIDI.
  [solid — `captures/xl_assign_param_fs6.msgpack.bin`, pinned by
  `fretwire-core/tests/controller_table.rs`]

  **The Floor's switch run starts at 6, not 3** (2026-08-26). Its table is `Array[20]` and the one
  ordinal↔layout pair held — the oracle preset's `Route To` at ordinal **13**, sitting at layout
  position **7** (switch 8) — puts `switch = ordinal − 5` where the Stomp and XL are
  `ordinal − 2`. Three more leading sources than a Stomp is exactly what a Floor has that a Stomp
  does not (a third expression pedal plus the two Variax knobs), but that reading of ordinals
  3..=5 is [hypothesis]; the offset itself is [solid for that one pair] and is what
  `fretwire_data::tone` uses when writing a 20-entry table.

  That observation is what retired the old reading. This table was documented as a flat ten with
  8 = MIDI and 9 = snapshots, which was an HX Stomp's shape mistaken for the format's, and it held
  because every capture came off a Stomp. On a Stomp the numbers are unchanged; above five switches
  everything above the run moves.

  **Ordinals 1 and 2 are EXP1 and EXP2** — read off an XL 2026-08-25, retiring a `[unverified]` that
  stood for want of an expression pedal. The owner put one block's bypass on EXP1 and another's on
  EXP2 and read the table back: ordinal 1 targets the block he assigned to EXP1, ordinal 2 the one
  he assigned to EXP2. The slots are what make it a check rather than a restatement of the label —
  a swap would have shown the other block. [solid — owner report, issue #13]

  **The far end of the footswitch run is observed too.** The same preset put a parameter under
  **FS8**, which came back as ordinal **10** — exactly `SOURCE_FS1 + 8 - 1` on an eight-switch
  device, and a second independent confirmation of the length formula after FS6 → 8.
  [solid — owner report, issue #13]

  **MIDI is 11 and snapshots is 12 on an XL** — the last two entries of this table to be read rather
  than computed. One preset put a Teemah's `Gain` under a MIDI CC and a Stupor OD's `Drive` under
  Snapshots; they came back at indices 11 and 12 of a 13-long table, with inner key `0` echoing the
  ordinal in each. So the eight-switch shape is observed end to end and the formula describes two
  pedals rather than fitting one. On a **Stomp** the pair is 8 and 9 — ordinal 9 was accepted by op
  37 and filed at index 9, and `tonepush` names both — but neither has been read off that panel, so
  it is the XL that carries this. [solid on an XL — owner report, issue #13, 2026-08-25,
  `captures/xl_assign_midi_and_snapshots.msgpack.bin`, pinned by
  `fretwire-core/tests/controller_table.rs`]

  **One past the end is silently ignored** — ordinal 10 on a Stomp was accepted and did nothing —
  and **the device does not range-check this**, so a caller must. `Session::assign_param` bounds it
  against the loaded preset's own count. [solid — 2026-08-22]
- **`6 → 28` is the sub-model selector, not a path.** `0` is the block's own model and `1` its
  paired cab, exactly like key `26` on the edit ops. It read as a constant `0` for as long as every
  sample was a main-model parameter; assigning a **cab** parameter puts a `1` there. This matters
  for naming: a cab's parameter 1 is `Position` where the amp's is `Bass`, so an assignment decoded
  against the wrong list names a real parameter that isn't the one being driven.
  [solid — verified live 2026-08-22; `Assignment::paired()`]
- **Only parameter controllers live in key `4`.** Assigning a block's **bypass** to a footswitch
  does not touch this table — that is recorded in `3 → 8` as a type-1 node, which we already read.
  [solid — reconfirmed by construction 2026-08-22: op 56 changed `3 → 8[0]` and nothing else.]
- **Snapshots remember each controller's value** at `10 → 10[N] → 2[<entry>][2]`, one per snapshot:
  `false` while nothing is assigned, the parameter's value once something is. Removing the
  assignment leaves the number behind. Worth knowing for two reasons — it is most of the diff noise
  when you assign something, and the same field correlates with the op-4 nil-slot puzzle
  (`docs/protocol.md`).
  [solid — assigning a Simple Delay's bypass to FS1 leaves key `4` entirely `nil`;
  `captures/assign_bypass_on_fs1.msgpack.bin`.]

  **A bypass on an *expression pedal* does reach key 4** — the "probably" above resolved on
  2026-08-25. An XL owner put one block's bypass on EXP1 and another's on EXP2, and both landed here
  as ordinary key-4 entries carrying a target slot (`5`) and **no parameter reference** (`6`), which
  is the same test that already separates a bypass entry from a parameter one. So the destination is
  chosen by the *source*, not by what is being driven: a bypass goes to `3 → 8` when a footswitch
  drives it and to key `4` when an expression pedal does. `tonepush`'s wah auto-engage example is
  this, not a different feature. [solid — owner report, issue #13]

  **Which opcode writes that is still unread.** Ops 56/57 take a plain switch index and nothing we
  hold shows one accepting an expression input; the assignment above was made on the pedal's own
  panel, so it settles the *document* and not the request that produces it.
- **Key `1` is not parameter-vs-bypass** [solid as a refutation]. `tonepush` documents it as
  "4 a parameter, 0 a bypass"; every assignment we have captured is a parameter and two of the three
  carry `0`. To tell the two apart, test for the presence of key `6` (the parameter reference).

  **Under a MIDI source, key `1` is the CC number** [solid — issue #13, 2026-08-25]. A `Gain` put
  under `CC5` gives `1: 5` at ordinal 11, while the Snapshots entry in the same preset drives an
  equally continuous `Drive` and gives `1: 0`. Same value type, different key — so this field is
  read **against the source**, which is what `K_ASSIGN_CC` (key `71`) already says of the op-37
  request that writes one.

  Off a MIDI source the "target's **value type**" reading — `0` on the continuous parameters
  (`Time`, `Mix`, `Drive`), `4` on the boolean one (`OD Switch`) — held for four samples and then
  **failed on the fifth**: every row of the Floor oracle preset stores `1: 4`, including two
  continuous wah/volume `Pedal`s (2026-08-26). So off a MIDI source the field's meaning is
  genuinely open; `4` is what most device-written rows hold, and what `fretwire_data::tone`
  writes.
- Worked example: the Dual-Amp preset's entry `[7]` is controller 7 → slot 15 **param 9**, the
  Grammatico GSG's `OD Switch`, swept `false`→`true`. (Previously recorded here as "param 0, an amp
  drive switch" — the description was right, the index was the bug above.)
- **`3 → 8` is the footswitch / stomp layout, not the signal path.** Each array position is a
  footswitch; a populated entry names the block whose **bypass** that switch toggles (with its slot
  at `11 → 8`). Empty position = unbound switch (FS3 is the global tap/tuner). **A block's bypass
  footswitch = its layout position + 1** (FS1 = `[0]`). **[solid — proven twice by controlled diff:**
  (1) swapping FS1↔FS2 changed *only* `/3/8[0]` and `/3/8[1]`; (2) binding a block to FS1 on an
  unbound preset flipped `/3/8[0]` from `nil` → that block's node and *nothing else*.] So `3 → 8`
  lists **only footswitch-bound blocks**, in FS order — it is empty when nothing's on a switch (a
  preset meant to be driven by snapshots/preset-changes), which is why block enumeration must come
  from the slot array, not here. Key `4` is the **separate** parameter-controller table.
- **Snapshots: key `10`.** `8` = stored active index, `9` = slot count (20), `10` = `Array` of
  snapshot objects, `13` = `Array[20]` of per-slot flags (all `true` in every fixture).
  Each snapshot object is `{0: in-use, 1: Array[11], 2: Array[64], 3: Array[20], 4: name,
  5: tempo?, 12, 14}`:
  - **`3` = the bypass matrix [solid]** — one `[_, enabled]` pair per slot; `enabled` is the
    inverse of a block's `bypassed`. The first element of the pair is `false` throughout and
    discriminates nothing. Proven against `preset1_stream`: its live blocks (2/3/4/7 bypassed,
    5/6 active) are exactly snapshot 0's row, and `8` reports 0. Parsed by
    `PresetStream::snapshot_details`.
    - **Indexed by wire slot (`dsp × 20 + index`), as one flat array across the whole device**
      — `Array[20]` on the Stomp but **`Array[40]` on a two-DSP Floor**, not one array per DSP.
      [solid — the tester's `pullmeunder` Floor dump, 2026-07-29.] Reading it at the per-DSP index
      makes every DSP2 block report DSP1's state, which is silent and looks like "all the
      snapshots are the same"; it is what `show-preset`'s scene diagnosis did until 2026-07-29.
  - **`8` is not reliably the *live* snapshot.** `dual_amp_stream` stores `8 = 1`, yet its live
    block state matches snapshot **0** (snapshots 1/2 there are pristine "everything on"). Both
    facts are locked in by tests. This is the standing lead on the GUI highlighting the wrong
    snapshot on hardware. Deriving the live snapshot by matching the matrix against live block
    state is a candidate fix but ambiguous when two snapshots hold identical scenes — which
    `preset1_stream`'s snapshots 1 and 2 do.
  - `2` = `Array[64]` of `[bool, int, nil]`, one per controller/param slot — the per-snapshot
    **parameter** values. Almost entirely `[false, 64, nil]` in the fixtures (64 reads as a
    midpoint default), so the encoding of an actually-varying value is still TBD: it needs a
    capture of one knob moved between two snapshots.
  - `1` = `Array[11]` of `[13, false, [0; 7]]` — uniform in every fixture, purpose unknown.

### Block enumeration: the slot array is authoritative  [solid]
**Blocks are enumerated from the slot array `0 → 22`, not the signal path.** Each kind-6 slot is a
self-contained block: model identity at `24 → 25` (→ `Helix.sym` index), enabled at `10`, params at
`11 → 4`, and — for amp+cab blocks — a paired cab/IR index at `24 → 26` with its params at `12 → 4`.
This needs **no name strings and no signal path**, so it decodes serial *and* split presets, and
recovers blocks the path omits. (`fretwire_core::editor::Catalog::load_preset` does exactly this.)

The signal path (`3 → 8`) is **unreliable as a block list**: freshly-built and split presets leave
it all-`nil`, and even the factory capture lists only 4 of its 6 blocks there. We earlier over-fit
to that one capture. The path is now used only to *enrich* a block with its footswitch binding,
controller-node kind, and user label **when present** (keyed by slot via `11 → 8`).

### Split / parallel topology  [solid]
- **`<dsp> → 21`** is the split flag: `0` = serial, **non-zero = split, and the value is the split
  *type***. The Stomp only ever uses `1`, so this was originally recorded as a `0`/`1` flag; the
  Floor uses `2` and `3` for its other split types, and a DSP's two paths can differ within one
  preset. Across the five presets we can check, `21 == 0` holds exactly when that DSP's row-B slots
  are empty. Each DSP carries its own flag. [solid — 2026-07-23]
- **Slot index encodes the row:** slots 0–15 = main (top) row, 16+ = the parallel (B) row.
  Proven by moving one block to path B and diffing: it relocated `0 → 22[6] → [16]`, `0 → 21`
  flipped `0 → 1`, and the pre-existing **kind-2 (split)** and **kind-3 (mixer)** nodes activated
  (`18: false → true`, with routing values at `13`). [boundary slot 16 = hypothesis, 1 sample]

### Param vector ↔ model params
The slot's value vector (`11 → 4`) is the model's parameters in order, excluding the structural
`@enabled/@stereo` params (the `enabled`/bypass flag lives at content key `20 → 10`). Verified by matching
device values to model defaults exactly — e.g. Dynamic Hall `Decay=4, Predelay=0.05, Damping=3720,
Motion=0.29`; Harmonic Tremolo `BassFreq=500, TrebFreq=700`.

**The param order is the resolved symbol's own list** (`Helix.sym[24→25]`). Since the index gives
the *exact* device symbol — including the `Mono`/`Stereo` variant — there's no guessing: a mono
block uses the mono order (Harmonic Tremolo index 318 → `…Mono`; index 8 is `SyncSelect1`=6, **not**
`.models`' `Spread`). This replaced the old `resolve_order` count-heuristic, which mis-guessed
variants whose lengths tie (it labeled Bucket Brigade *Stereo* and the reverb *Mono*; the index
says **Mono** and **Stereo** respectively). `resolve_order` is retained for length-based fallback.

**Trailing `Trails` switch:** time-based fx carry one extra value the symbol doesn't list. The
slot distinguishes it: `11 → 2` = array length, `11 → 3` = real param count (e.g. the reverb is
`2:13, 3:12`). A lone trailing extra is named `Trails`; further extras are positional (`#i`).

### Model identity: the `Helix.sym` index  [solid]
A block's model is identified by **`24 → 25`, an index into `Helix.sym`'s array order** (833
entries, with `Mono`/`Stereo` suffixes). `Helix.sym[idx]` gives the device symbol; strip the
suffix for the `symbolicID`, then resolve display name + category via
`ModelDefs::id_by_symbolic_id`. Amp+cab blocks carry a second index at `24 → 26` (the cab/IR; `-1`
= none). Verified two ways: a hand-built preset (591 → US Princess amp, 80 → Simple Delay, by the
user's own hands) and the factory capture (76 → Bucket Brigade Mono, 318 → Harmonic Tremolo, 610 →
Dynamic Hall Stereo, 709 → 1×12 US Deluxe cab). tests/`correlate_modelid.rs`.

> **Correction:** we earlier called `24 → 25` "a runtime DSP handle, not a model index" — wrong.
> It looked non-indexing only because it was tested against `HelixModelDefs.bin` (681 models); it
> indexes the **833-symbol `Helix.sym`** instead. There *is* a numeric model id after all.

Two related facts still hold:
- **`symbolicID` is the only globally-unique *string* key** (681/681); display names collide (163:
  cab mic/pan variants, amp vs preamp, legacy/modern delays, per-device I/O). It's how we turn a
  stripped symbol into a name.
- **Path `11 → 6` encodes *category*, not model** (Harmonic Tremolo & 70s Chorus both `1037` =
  category 8). Now moot for identity, which comes from `24 → 25`.

**Resolution strategy (shipped data only):** look the block's `name` up in `HelixModelDefs.bin`
via `ModelDefs::resolve`. **★ Use the param count — which we already parse from the value vector —
not the category.** Full analysis (`tools/analyze-name-collisions.js`, over all 681 models):

**Category is no longer needed to disambiguate identity.** The `24 → 25` index resolves the *exact*
symbol (amp vs preamp vs a specific cab are distinct `Helix.sym` entries), so the old name-collision
problem — which needed category to break ties — doesn't arise. Decoding `11 → 6` → `category` is now
only a nicety (e.g. grouping in a UI), not a correctness blocker.

For a **name-only resolver** (no `Helix.sym` index at hand — analysis in
`tools/analyze-name-collisions.js`), the collisions break down as:

| disambiguator | colliding names resolved |
|---|---|
| name alone | 517 / 681 unique already |
| **+ param count** | **150 of the 164 collisions** (incl. cab mic/pan variants and 97 of 108 amp/preamp pairs) |
| + param count + **category** | 11 more (amp/preamp pairs with *equal* param counts — see below) |
| neither | 3 residual (the `2x12 Match H30`/`G25` cab pair — a duplicate-name **defect in Line 6's own data** — and per-device `Input`/`Output`, resolved by device context) |

So for that fallback **category (`11 → 6`) would matter for only 11 of 681 models** — the amp/preamp
pairs whose amp and preamp variants happen to declare the same number of params: `Cali Bass`,
`G Cougar 800`, `Line 6 2204 Mod`, `Busy One Ch1/Ch2/Jump`, `Del Sol 300`, `Woody Blue`, `Agua 51`,
`SVT-4 Pro`, `Line 6 Clarity`.

**Why `11 → 6` stays undecoded (2026-06-24):** the three observed
values (Mod `1037`, Delay `67840`, Reverb `1049600`) are **not a simple lookup table** — they look
computed by the preset serializer. (The value `1037` is also the Windows LCID for Hebrew/Israel
(`0x40D`) — a coincidence, not a model/category table.) With only 3 category samples the encoding
isn't derivable from data; decoding it needs a device preset stream containing amp/preamp blocks to
sample more values.
Moot for identity now that the `24 → 25` index resolves it exactly.

(For reference, the test blocks' true table indices are Bucket Brigade 264, 70s Chorus 422,
Harmonic Tremolo 441, Dynamic Hall 640 — reached by name, not by any preset field.)

## What an empty preset looks like  [solid — HX Stomp, 2026-08-26]

Read off two never-used slots with `read-slot` (op 4, non-destructive — the pedal stays where it
is), and diffed against a populated one. An empty preset is 2254 bytes and reads:

| Path | Empty |
| --- | --- |
| `0 → 22[n] → 19` / `→ 20` | kind `8`, content `nil`, for all 20 slots |
| `4` | `Array[10]`, every entry `nil` — no controller assignments |
| `3 → 8` | `Array[5]`, every entry `nil` — no footswitch bindings |
| `10 → 10[i] → 4` | `SNAPSHOT 1`, `SNAPSHOT 2`, `SNAPSHOT 3` (a Stomp has three) |
| `10 → 10[i] → 0` | `false` — the snapshot "in use" flag |
| `0 → 21` | serial |

**There is no canonical blank blob.** Two untouched slots off the same pedal are not byte-identical:
they disagree on the input node's param `[2]` (`0 → 22[0] → 20 → 7 → 4`), the output node's param
`[5]` (`0 → 22[19] → …`), and key-`5` flags `49`/`56`. They also carry the build stamp of whatever
firmware last wrote them (`7 → 37`), which is *not* the running firmware. So "clear a preset" cannot
mean "write a known-empty document" — it means "remove what the user put in", which is what
`Session::clear_preset` does.

### What a block takes with it when deleted  [solid — HX Stomp, 2026-08-26]

Op 28 (delete block, preceded by the op-78 structural marker) is not just a slot wipe:

- A **parameter assignment** on the deleted block — key `4[ordinal]`, made here with op 37 source 1
  (EXP1) — reads back `nil` afterwards. No op-37-source-0 pass is needed to clean up.
- Its **footswitch bypass binding** — key `3 → 8[switch]` — goes too, as
  [`Session::delete_block`] already documented.
- A **split preset collapses to serial on its own** once the last row-B block is deleted (`0 → 21`
  clears, and the kind-2/kind-3 nodes with it). Nothing has to move the split or mixer node home.

What survives an emptied chain, and therefore has to be reset by name if you want it gone:

- **Snapshot names** (`10 → 10[i] → 4`) — per-preset text no block owns. Op 89 renames them.
- The preset **tempo** (`5 → 16`), the **footswitch page** (`3 → 7`), and the **focused slot**
  (`6 → 98`). `clear_preset` leaves all three alone: the first two are per-preset settings with no
  agreed-on blank value (see above), and the third is UI state.

## Status
- [x] Reassembly → envelope → blob → magic/header/preset map (parser + tests in `fretwire-data`).
- [x] Typed model: `PresetStream::{device_model, firmware, blocks, effect_blocks, loaded_blocks,
      footswitch_layout, is_split}`.
- [x] **Blocks enumerated from the slot array `0 → 22`** (kind 6) — serial *and* split presets,
      including blocks not on any footswitch (`loaded_blocks`).
- [x] **Model identity = `24 → 25` index into `Helix.sym`** → exact symbol (with Mono/Stereo) →
      `symbolicID` → name/category. Amp+cab pair via `24 → 26`. Param order is the symbol's own.
- [x] **Split topology decoded:** `0 → 21` split flag; row from slot index (0–15 main, 16+ row B);
      kind-2 split / kind-3 mixer nodes. Verified by a controlled serial↔split diff.
- [x] Name + param-count collision analysis (`tools/analyze-name-collisions.js`): 150/164 colliding
      names resolvable without category — a fallback design for paths where the `Helix.sym` index
      isn't available (the index supersedes it in code).
- [x] **Empty-preset shape and what a delete takes with it** — see the section above; the basis for
      `Session::clear_preset`.
- [ ] Decode path key `11 → 6` → `category` (UI grouping only — no longer needed for identity).
      Computed value, not a binary table; needs the serializer decoded or a device
      preset with amp/preamp blocks to sample more values.
- [ ] Link a (block, param) to its wire handle (`8X 62 [block] NN`) so edits can target it.
      (Slot `24 → 25` is a per-block runtime handle — a lead for this work.)
