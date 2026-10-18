# Helix LT — device survey

What we know about the **Helix LT**, from a user's unit on Linux (2026-08-18, firmware string
`a7e2585`). That survey rested on **no USB captures and no device backup**: it was read by
fretwire itself over `MI_00`, with this PR's device entry applied.

A second owner (2026-10-18, fw 3.80, HX Edit 3.82, Windows) then sent a **backup and three HX
Edit captures** from part of the capture round (`docs/helix-capture-round.md`: A1, A2, B1, B2).
Their results are in [Owner captures (2026-10-18)](#owner-captures-2026-10-18) below; the
`.pcapng` files, the backup and the reassembled streams stay local (`captures/helix-lt/`, ignored).

**Bottom line: the LT needs no protocol change.** The handshake, the preset read, the snapshot
decode, and the setlist and preset-list browses all work unmodified once the PID is in the table.
The catalogue resolved every block and parameter of the loaded preset by name.

**Nothing has been written to this device** — no edit, no save, no backup, no restore. That is why
the entry is `Support::Reported` rather than `Support::Verified`: the reads are real traffic checked
against real parsers, but `Verified` means the *edit builders* have been reconciled against a unit's
own bytes too, and here they have not. (The PR proposed `Support::Untested`; that tier means "only
the USB IDs are known", which undersells this survey. `Reported` was added on 2026-08-20.)

## USB identity  [solid]

| | Helix Floor | Helix LT |
|---|---|---|
| VID / PID | `0x0E41` / `0x4248` | `0x0E41` / **`0x424A`** |
| `bcdDevice` | `0x0200` | `0x0200` |
| product string | — | `HELIX` |

Interface layout, read from Linux sysfs — the same six-interface shape the Floor has, with the
vendor control channel at interface 0:

| iface | class | kernel driver |
|---|---|---|
| 0 | **Vendor (`0xFF`)** | none — this is `MI_00` |
| 1 | Audio control | `snd-usb-audio` |
| 2 | Audio streaming | `snd-usb-audio` |
| 3 | Audio streaming | `snd-usb-audio` |
| 4 | Audio / MIDI | `snd-usb-audio` |
| 5 | HID | `usbhid` |

`CONTROL_INTERFACE`, `EP_IN` and `EP_OUT` needed no change, and `claim_interface(0)` succeeded on
the first try.

## Identity and firmware  [solid]

The handshake identity reply reports **`P21`** — the Floor's model code, not a code of its own. The
pulled preset agrees, at key `7`:

| key | value |
|---|---|
| `36` | `"P21\0"` |
| `37` | `"a7e2585\0"` |
| `35` | `57737248` = `0x03710020` |

Key `35` is recorded raw on purpose. The second LT, on a stated **fw 3.80**, carries `0x03800020`
with build `7d01f5e`, so the high half reads as the version (`0x0380` → 3.80, `0x0371` → 3.71)
[hypothesis: two points, one per firmware]. The low `0x0020` is not a patch level — the Stomp's
identity reply gives `0x03800000` on 3.80 — and the `.hxb` header of the same LT says `0x03800000`.
The Floor's dump has the same `0x03800020` and the same build `7d01f5e` as this 3.80 LT, so the
"3.82" recorded for it in `docs/helix-floor.md` was most likely HX Edit's version, not the
firmware's [hypothesis].

## Preset model  [solid]

Reading the loaded preset (8326 bytes, declared length matched):

- **Both DSPs.** Preset key `1` is populated and blocks came back in slots **≥ 20** as well as in
  the first group, i.e. the global `slot = dsp * 20 + index` numbering the Floor established. The
  unit reported DSP1 71.0% used and DSP2 43.0%.
- **8 snapshots**, `SNAPSHOT 1`…`SNAPSHOT 8`, and the stored active index agreed with the live
  scene.
- Every block resolved to a `.models` definition by name, with device-ordered named parameters —
  no unmatched model, no unmatched parameter key.

> **Maintainer note (2026-08-20).** The survey as submitted gave two different block counts — a
> slot list of `21–28` plus `3–6` (twelve slots) in one bullet and "all 8 blocks" in the next — and
> the question went unanswered, so the exact figure is not recorded here. Nothing rests on it:
> `dsps: Some(2)` needs only that *some* block occupies a slot ≥ 20, which both readings agree on,
> and which preset key `1` being populated says independently. If you have an LT, `fretwire pull`
> prints the slot list and settles it.

## Setlists  [arity solid; end-bank names solid, USER names not corroborated]

Browsing banks 0, 1, 2, 6, 7 and 8 (3–5 were not reported, and it is not recorded whether they were
browsed and dropped in editing or never browsed):

| bank | result |
|---|---|
| 0 | 128 presets — `US Double Nrm`, `Essex A30`, `Brit Plexi Brt` (factory) |
| 1 | 128 presets |
| 2 | 128 presets — the user's own presets; `read-info` on the loaded preset reported `bank: 2, index: 0, name: "WIP"` |
| 6 | 128 presets |
| 7 | 128 presets — `Quick Start`, `Parallel Spans`, `SNP:4-Amp Spill` (templates) |
| 8 | refused, code `-3` |

So: **eight banks of 128**, bank 0 factory and bank 7 templates — the Floor's layout. Bank 8 being
refused bounds the arity at eight on its own, so the three unreported banks do not weaken that.
These browses ran with this PR's entry applied, i.e. against its explicit `setlist_size: Some(128)`;
they corroborate that value rather than exercising `setlist_stride()`'s fallback, which never ran.

The names in `DEVICES` are therefore the Floor's. Worth being explicit about the difference in
evidence: the Floor's names came from the eight `L6Setlist` streams of a real `.hxb`, whereas here
only the *arity* and the *character of the two end banks* were observed. If an LT backup ever turns
up and disagrees, this is the field to fix.

**Update (2026-10-18):** an LT backup turned up. It has eight setlists of 128, and `FACTORY 1`,
`FACTORY 2` and `TEMPLATES` match. Its owner had renamed banks 2–6, so it can't confirm or rule out
`USER 1`…`USER 5`.

## Owner captures (2026-10-18)

### Device facts  [solid]

| | value | source |
|---|---|---|
| `preset_device_id` | **`0x00210004`** (Floor `0x00210001`) | `.hxb` header |
| `presets_per_bank` | **4** | the unit's screen showed `21D` with numbering set to `01A`-`32D` |
| setlists | `FACTORY 1`, `FACTORY 2`, …, `TEMPLATES` | `.hxb`; banks 2–6 renamed by the owner, so the `USER n` defaults are still the Floor's |

Both are now in `DEVICES`. The Floor's bank size is still unread, so it does not inherit the `4`.

### Factory presets read over USB (A1)  [solid]

Five factory/template presets were loaded and reassembled; every one parsed, and the preset key
`21` topology codes match `tone::topology_code` and the Floor backup's topologies:

| preset | DSP 1 | DSP 2 |
|---|---|---|
| Plush Garden | `2` SAB | `4` AB |
| GARBAGE BASS | `4` AB | `0` A |
| 4 Tone Switcher | `4` AB | `4` AB |
| Stereo Mixer | `4` AB | `4` AB |
| Pull Me Under | `2` SAB | `3` ABJ |

Plush Garden matches the Rack's dump of it except for float rounding, including the "before-split!"
diagnostic explained in `docs/helix-rack.md`. On fw 3.80 the factory presets sit in different slots
from the Floor's backup (Pull Me Under at FACTORY 2 21D); Trademark and Unicorns Forever were not
found.

### Input and output routing: op 42  [solid]

Setting Path 2's Input to Guitar (B1) and Path 1's Output To through Path 2A → 2B → 2A+B → 2A (B2)
each sent one **op 42**:

```
{102: txn, 100: 42, 101: {98: 20, 51: 2}}      input node, slot 20, input_type 2 = Guitar
{102: txn, 100: 42, 101: {98: 9,  52: 3}}      output node, slot 9, output_type 3 = Path 2B
```

Key `51` is an input's source and `52` an output's destination, both indexes into
`HelixControls.json`'s `input_type` / `output_type` lists. Each was echoed on the status channel as
type `27` with the same map. Builders: `edit::set_input`, `edit::set_output`.

**Path 2's input does not change by itself.** With Path 1 sending to Path 2A, no push named slot 20,
and HX Edit still shows Path 2's input as `None`, with an arrow marking the feed from Path 1.

**Path 2B must exist first.** HX Edit greys out `Path 2B` and `Path 2A+B` until Path 2's row B has
an input of its own (op 46, below).

### Removing a split: op 46  [solid for this one transition]

To give Path 2B an input, the owner added a Simple EQ on Path 2, dragged it to row B (op 39, then
op 43 `21 → 31`), and dragged the Split down. That last step sent `{98: 30}` as **op 46**, with no
op-21 write. The re-read preset differs in six paths:

```
/1/21                 1 -> 3      DSP 2: SABJ -> ABJ
/1/22[10]/20/15/13    1 -> 0      the split's position
/1/22[10]/20/15/10    true -> false
/1/22[10]/20/18       true -> false
/6/98, /6/26                      focus moved to slot 30
```

The device then pushed `{98: 30, 51: 0}`: slot 30 is now Path 2B's input, starting at None. Builder:
`edit::remove_split`. Not yet seen: op 46 on DSP 1, from `SAB`, and the inverse (dragging the
split back up).

## Not observed

- **Every write path but op 42 and op 46.** Set-value, bypass, swap, add, delete and save have not
  been compared against an LT's own bytes, so the entry stays `Reported`.

## Reproducing

With the LT connected and the udev rule installed:

```
fretwire detect                 # Helix LT: present (reported working, unverified)
fretwire connect                # handshake OK — device reports "P21"
fretwire pull                   # the preset above, blocks and params resolved
fretwire setlists               # 8 setlists
fretwire presets 7              # the templates bank
fretwire dump-raw lt.raw        # then: fretwire tree lt.raw
```
