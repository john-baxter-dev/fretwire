# Helix Rack — device survey

What we know about the **Helix Rack**, from an owner's unit on Linux (2026-10-14, issue #24,
firmware 3.80.0 build `7d01f5e`, fretwire 0.6.0 unmodified). Like the LT survey, this rests on no
USB captures: everything below was read by fretwire itself over `MI_00`, apart from the device id,
which came from the owner's HX Edit backup.

**Bottom line: the Rack needs no protocol change.** The handshake, the preset read, the snapshot
decode, the setlist and preset-list browses, the settings dump and the teardown all ran unmodified
against the entry that had been in the table, untested, since 2026-09-01.

**Edits work** (third round, 2026-10-14): two parameter sets, one on each DSP, a bypass and a
revert, each confirmed on the unit's screen and by a re-read. That makes the entry
`Support::Verified` without an HX Edit capture from a Rack: it reports `P21`, so it takes the
Floor's builders, which were checked byte-for-byte against a Floor's own traffic, and the Rack
accepted them.

## USB identity  [solid]

`lsusb -d 0e41:` reads `ID 0e41:4249 Line6, Inc. HELIX`. That is the PID taken from the Linux
kernel's quirk table, now confirmed on a unit, and the same product string the LT carries.
`claim_interface(0)` succeeded on the first try.

## Identity and firmware  [solid]

The handshake identity reply reports **`P21`**, the Floor's model code, as the LT's does. A pulled
preset agrees, at key `7`:

| key | value |
|---|---|
| `36` | `"P21\0"` |
| `37` | `"7d01f5e\0"` |
| `35` | `58720288` = `0x03800020` |

Key `35` is recorded raw, as it was for the LT (`0x03710020`). This one comes with a known firmware
version (3.80.0), which is a first. It still doesn't pin the encoding: `docs/helix-floor.md` has a
Floor on 3.82 reporting `0x03800000` in the identity reply. That is a different field, but it's
enough to stop us reading the top bytes as the version without more samples.

## Preset model  [solid]

- **Both DSPs.** Plush Garden (bank 0 index 53) held blocks in slots 35..38; the unit reported
  DSP1 71.8% and DSP2 74.5%.
- **8 snapshots**, `SNAPSHOT 1`…`SNAPSHOT 8`.
- Every block resolved by name, with named parameters.

The `pull` output flagged path-B blocks as "before-split!" and "past-mixer!" on both DSPs. Most of
that was the CLI misreading the preset, not a Rack problem. Plush Garden's bracket opens on DSP1 and
never closes (`SAB`, then `AB` on DSP2, per the Floor backup's copy of the same preset), and an
inactive node is written at column 0. The CLI read that 0 as a real position. The Floor's Pull Me
Under printed the same false "past-mixer!". Fixed in the same change.

**Topology `AB` is split type `4`** [solid]. The owner's `dump-raw` of Plush Garden reads key 21
= `2` on DSP1 (`SAB`, as the table already had) and `4` on DSP2, where the paths only pass through.
Converting the Floor backup's `tone` of the same preset now carries both
(`tests/tone_to_wire.rs`, `topology_ab_comes_across`). The owner also sent Pull Me Under: same
length as the Floor's capture, same structure, split types `2`/`3`, and its only differences are
34 float values one ULP apart (`0.14999998` against `0.15`).

The one real flag left is DSP1's slot 12 at column 2, left of a split at column 4. HX Edit's tone
JSON for the preset puts it there too (`@path 1, @position 1`), so the factory preset really does
store a path-B block before its split.

**Snapshot "MISMATCH" on US Double Nrm:** the blob's stored active index is 2, while the live scene,
and the device's own read-info (`snapshot: Some(0)`), both say 0. The stored index is whatever was
active when the preset was last saved. The live readout is consistent, so this is not a bug.

## Setlists  [solid]

Banks 0..7 each list 128 slots and bank 8 is refused with code `-3`: **eight banks of 128**, the
Floor's and the LT's layout. The names are the Floor's, confirmed by the owner's HX Edit setlist
pane: FACTORY 1, FACTORY 2, USER 1–5, TEMPLATES. (The first survey's `fretwire setlists` printed one
setlist named `Presets`. That was the old entry's fallback, not the device.)

After a factory reset (2026-10-14) the owner listed every bank: FACTORY 1 and 2 full (FACTORY 2
ends in 24 `New Preset` slots), USER 1–5 all `New Preset`, TEMPLATES 38 templates (`Quick Start`
… `FPS Video Game`) plus one stray user preset at slot 127.

**The factory order is not the Floor backup's.** Bank 0 matched the pre-reset listing exactly, and
it still differs from `FACTORY 1` in our Floor backup (2026-07-22) in 105 of 128 slots. Presets
that are in bank 0 on the Rack (Plush Garden at 53, Stone Cold Loco at 52) sit at other indices
there, and a run of the Floor's FACTORY 1 (Pull Me Under, AUS Flood, Lonely Hearts, the `SFX:`
presets) is in the Rack's FACTORY 2. The likeliest reading is that the Floor's factory banks were
last reset on an older firmware with a different factory lineup; firmware updates don't rewrite
presets. Unconfirmed.

**Stored keys other than the slot are factory state.** `presets` reports 36 such entries in
FACTORY 1 and 12 in FACTORY 2 on the freshly reset unit, so they are not evidence the owner moved
anything. The CLI note used to say "have been reordered on the device"; it now says they were moved
on the device or before it left the factory.

## Preset numbering  [solid]

Read off the unit: **`01A`, `01B`, `01C`, `01D`, `02A`…**, four to a bank, so `presets_per_bank:
Some(4)` and 128 slots run `01A`-`32D`. This is the first bank size read off any Floor-class device.
The Floor's and the LT's stay `None`: it is likely the same on both, but nobody has read it off one.

## Global settings

`settings-dump` answered 177 of 261 ids. The dump is attached to issue #24 and has not been mapped
against `settings::SETTINGS` yet.

## Device id  [solid]

The owner's HX Edit backup (2026-10-14) has `0x00210002` in its header's device field, beside the
Floor's `0x00210001`, with fw `0x03800000`. `show-backup` now names it a Helix Rack.

## Writes  [solid]

With Plush Garden loaded (`goto 53 0`), each step confirmed on the Rack's screen:

| command | result |
|---|---|
| `set 5 0 0.8` | Essex A30 Drive 4.5 → 8.0 |
| `bypass 4 off` | Valve Driver on |
| `set 35 0 0.6` | Revv Gen Red Drive 4.1 → 6.0, on DSP2 |
| `pull` | all three, and the bypass edit carried into snapshot 1's stored state |
| `revert` | back to the saved preset, confirmed by a second `pull` |

Not exercised: save, rename, add/delete/move, swap, footswitch assignment.

## The toe switch  [solid]

The thirteenth bypass-layout position is the expression pedal's toe switch: HX Edit's assign list
on the Rack shows Plush Garden's Fassel *and* both Volume blocks on "EXP Pedal Toe". fretwire
printed it as "FS13" and now uses HX Edit's name.

## The backup, against the Rack's own dumps  [solid]

The backup and the owner's `dump-raw`s of Plush Garden, Pull Me Under and US Double Nrm are the
same presets in both forms, off one unit. Converting each with `hxb-convert` onto another of the
three reproduces what the Rack stored, apart from float noise and the stored active snapshot. Four
things came out of making that true (`tests/tone_to_wire.rs`):

- **The input/output nodes**: stored only while routed (gate 3, pan/gain 2, nothing for None).
  See `docs/preset-format.md`.
- **HX Edit on a Mac writes whole-number knobs as JSON integers**: 10,878 values across all 307
  presets of this backup, against none in the Floor's Windows backup. Converted as written, they
  would have reached the device as integers. Numbers are now typed by `.models` `valueType`.
- **The preset tempo** is `5 → 16` and the **focused block** `6 → 98`, both from `global`.
- `hxb-convert` refused every Rack backup once the Rack had a device id: it compared device names,
  and a `P21` donor always reads as a Floor. It compares model codes now.

## Not observed

- **A capture of HX Edit talking to a Rack.** Not needed for `Verified` (see the top), but
  it would be the only way to spot a Rack-specific byte if one ever turned up.
- **Structural writes and saves** (see Writes).

## Reproducing

```
fretwire detect                 # Helix Rack: present
fretwire connect                # handshake OK — device reports "P21"
fretwire pull                   # the loaded preset, blocks and params resolved
fretwire setlists               # 8 setlists
fretwire presets 0              # 128 factory presets
fretwire dump-raw rack.raw      # then: fretwire tree rack.raw
```
