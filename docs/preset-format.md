# Preset MessagePack format (device stream)

When a preset is opened, the device streams it as MessagePack (see `docs/protocol.md` for the
transport). `fretwire_data::stream::PresetStream::parse` decodes a reassembled stream. Validated
against `captures/preset1_stream.msgpack.bin` (HX Stomp, fw v3.71). Integer keys below are
**observed**; names are inferred (often by correlation with the `.hlx` JSON schema).

## Envelope
Reassembled stream = `{102: <u32>, 103: 0, 104: <blob>}` (MessagePack, root ~8 bytes into the
stream after a `marker/type/len` header). Key **104** is a `str`/`bin` blob = the preset.

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

## Preset map (integer keys)
| key | value | meaning (inferred) |
|----:|-------|--------------------|
| 7 | Map `{36: "P33", 35: 58720256, 37: "v3.71-32-g1039661"}` | device info: **36 = model code** (`P33` = HX Stomp family), 35 = version (0x03800000), 37 = firmware string |
| 0 | Map `{21: split, 22: Array[20]}` | **block slots** of the first DSP (`21` = split type) — see below |
| 1 | Map `{21: split, 22: Array[20]}` — or nil | **the second DSP's slot array**, same shape as key `0`. **nil on the HX Stomp** (one DSP), populated on the Helix Floor. [solid — 2026-07-22, Floor captures cross-checked against a `.hxb` backup] |
| 2 | Map `{0: Array[13], 1: Array[13×Array[7]]}` | snapshot/controller matrices (13 = snapshots? all zero here) |
| 3 | Map `{7: 0, 8: Array[5]}` | **footswitch / stomp layout** — bound blocks only; see below |
| 4 | Array[10] (all nil) | **parameter-controller** assignments (separate from `3 → 8`; empty here) |
| 5 | Map{15} `{16: f32, 45..56: …, 30, 134}` | **preset-level settings**. Byte-identical across all three captures (all at defaults), so the fields aren't separable yet; `16` = f32 80 is most likely the preset tempo. [hypothesis] |
| 6 | Map{2} `{98: <slot>, 26: 0}` | **the focused block** — key `98` is the same slot number the edit commands address, and it differs per capture (5, 7, 12), matching the block last selected. [solid] |
| 10 | Map{6} `{6,7,8, 9: 20, 10: Array[n], 13: Array[20]}` | **snapshots**: `10` is the snapshot array (3 on a Stomp, 8 on a Floor), `9` = the slot count, `13` = a per-slot array. Each snapshot is `{0: enabled, 1: Array[11], 2: Array[64], 3: Array[20], 4: name, 5: f32 tempo, 12, 14}` — note `3` is **per-slot state**, one entry per block slot. [solid] |

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
- `24` = `{25: model/DSP id, 26: secondary id, …}` (metadata).
- `11` = `{2: count, 3: count, 4: [values]}` — **the ordered param vector** (msgpack `float32`
  for knobs, `int`/`bool` for enums/switches), in `.models` param order.
- `12` = a second (usually empty) param bank; `9` = flag.

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
  name (matches `.models` `name`); for a type-2 node, `5` is the footswitch **label** (e.g.
  `"OD Sw"`), `6` is *not* a model id (won't resolve), and `11 → 9` = `{28,29,41}` (the same
  param-descriptor shape used in the assignment table — see below). [solid — verified live]
- `14` = user label string; `13` = has-label flag (e.g. a `Harmonic Tremolo` renamed `Tremolo`).

### Controllers / footswitch assignments (`4`), snapshots (`10`/`2`) [partial — 2026-06-24]
- **`4` = `Array[10]`** — **parameter**-controller assignment table. A populated entry is `Array[1]`
  of `{0:0, 1: Map{9}}` where the inner map is `{0:<controller#>, 1:<type>, 4:<min>, 5:<target slot>,
  6:{28:<param idx>, 29, 41}, 7:<max>, …}` — i.e. *controller N drives slot/param*. Example: a
  Dual-Amp preset's entry `[7]` = controller 7 → slot 15 param 0 (an amp drive switch on a footswitch).
  The controller-number → physical-control (which footswitch / EXP) mapping is **not yet decoded** —
  HX's internal controller-ID scheme; needs a diff experiment or docs.
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
- [ ] Decode path key `11 → 6` → `category` (UI grouping only — no longer needed for identity).
      Computed value, not a binary table; needs the serializer decoded or a device
      preset with amp/preamp blocks to sample more values.
- [ ] Link a (block, param) to its wire handle (`8X 62 [block] NN`) so edits can target it.
      (Slot `24 → 25` is a per-block runtime handle — a lead for this work.)
