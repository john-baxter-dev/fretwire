# Helix Rack — device survey

What we know about the **Helix Rack**, from an owner's unit on Linux (2026-10-14, issue #24,
firmware 3.80.0 build `7d01f5e`, fretwire 0.6.0 unmodified). Like the LT survey, this rests on no
USB captures and no device backup: everything below was read by fretwire itself over `MI_00`.

**Bottom line: the Rack needs no protocol change.** The handshake, the preset read, the snapshot
decode, the setlist and preset-list browses, the settings dump and the teardown all ran unmodified
against the entry that had been in the table, untested, since 2026-09-01.

**Nothing was written to this device.** That is why the entry is `Support::Reported` and not
`Support::Verified`, the same as the LT.

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

The one real flag left is DSP1's slot 12 at column 2, left of a split at column 4. HX Edit's tone
JSON for the preset puts it there too (`@path 1, @position 1`), so the factory preset really does
store a path-B block before its split.

**Snapshot "MISMATCH" on US Double Nrm:** the blob's stored active index is 2, while the live scene,
and the device's own read-info (`snapshot: Some(0)`), both say 0. The stored index is whatever was
active when the preset was last saved. The live readout is consistent, so this is not a bug.

## Setlists  [arity solid; names not corroborated]

Banks 0..7 each returned a listing and bank 8 was refused with code `-3`. Bank 0 holds 128 factory
amp presets (`US Double Nrm`, `Essex A30`, `Brit Plexi Brt`, …). That makes **eight banks of 128**,
the Floor's and the LT's layout, so `DEVICES` uses the Floor's setlist names with the same caveat as
the LT: only the arity and bank 0 were observed.

The survey's `fretwire setlists` printed one setlist named `Presets`. That was the old entry's
fallback (`setlists: None`), not the device.

Bank 0 is **not** the same list as `FACTORY 1` in our Floor backup (2026-07-22): 105 of 128 slots
differ, and 23 names appear only on the Rack and 23 only on the Floor, not counting `No:thing` vs
`No:Thing` (Plush Garden is index 53 here and 47 there). The
listing also says 36 entries carry a stored key other than their slot, and the owner has not
reordered anything. Both are unexplained so far. They could be a different factory set, or a
quirk of how the factory bank is stored.

## Preset numbering  [solid]

Read off the unit: **`01A`, `01B`, `01C`, `01D`, `02A`…**, four to a bank, so `presets_per_bank:
Some(4)` and 128 slots run `01A`-`32D`. This is the first bank size read off any Floor-class device.
The Floor's and the LT's stay `None`: it is likely the same on both, but nobody has read it off one.

## Global settings

`settings-dump` answered 177 of 261 ids. The dump is attached to issue #24 and has not been mapped
against `settings::SETTINGS` yet.

## Not observed

- **`preset_device_id`.** No backup from a Rack.
- **Every write path.**
- **Topology `AB`'s key-21 value.** The Rack's Plush Garden *is* an `AB` sample on DSP2: `pull`
  drew a DSP2 bracket, so the value there is non-zero. But `pull` doesn't print it, and the raw dump
  attached was taken on a different preset. A `fretwire dump-raw` with Plush Garden loaded closes
  the open question in `tone::topology_code` (see `docs/helix-capture-round.md`).

## Reproducing

```
fretwire detect                 # Helix Rack: present (reported working, unverified)
fretwire connect                # handshake OK — device reports "P21"
fretwire pull                   # the loaded preset, blocks and params resolved
fretwire setlists               # 8 setlists
fretwire presets 0              # 128 factory presets
fretwire dump-raw rack.raw      # then: fretwire tree rack.raw
```
