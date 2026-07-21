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
2. `str` — a fixed header/uuid (kept verbatim, meaning TBD).
3. `Map` — **the preset** (integer-keyed).

## Preset map (integer keys)
| key | value | meaning (inferred) |
|----:|-------|--------------------|
| 7 | Map `{36: "P33", 35: 58720256, 37: "v3.71-32-g1039661"}` | device info: **36 = model code** (`P33` = HX Stomp family), 35 = version (0x03800000), 37 = firmware string |
| 0 | Map `{21: 0, 22: Array[20]}` | **block slots** (`21` = split flag) — see below |
| 1 | nil | ? |
| 2 | Map `{0: Array[13], 1: Array[13×Array[7]]}` | snapshot/controller matrices (13 = snapshots? all zero here) |
| 3 | Map `{7: 0, 8: Array[5]}` | **footswitch / stomp layout** — bound blocks only; see below |
| 4 | Array[10] (all nil) | **parameter-controller** assignments (separate from `3 → 8`; empty here) |
| 5 | Map{15} | TBD |
| 6 | Map{2} | TBD |
| 10 | Map{6} | TBD |

### Block slots (`0 → 22`, Array of 20)
Each slot is `{19: type, 20: content}`:
- `type 8` + `nil` → **empty slot**.
- `type 6` + `Map{5}` → **populated effect block** (5 params). 6 such blocks in this preset,
  matching the names seen in the stream (Bucket Brigade, Harmonic Tremolo, Tremolo, 70s Chorus,
  Dynamic Hall, …).
- other types (`0,1,2,3`) → structural slots (input/split/join/output).

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
- **Snapshots:** names + active index in **key `10`** (`8` = active, `10` = `[{4: name, …}]`); the
  per-block snapshot value matrix is in key `2` (and `10`'s sub-arrays). Names/active parsed; values TBD.

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
- **`0 → 21`** is the split flag: `0` = serial, `1` = split (parallel rows).
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
