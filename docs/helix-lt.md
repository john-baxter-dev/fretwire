# Helix LT — device survey

What we know about the **Helix LT**, from a user's unit on Linux (2026-08-18, firmware string
`a7e2585`). Unlike the Floor survey, this one rests on **no USB captures and no device backup**:
everything below was read by fretwire itself over `MI_00`, with this PR's device entry applied.

**Bottom line: the LT needs no protocol change.** The handshake, the preset read, the snapshot
decode, and the setlist and preset-list browses all work unmodified once the PID is in the table.
The catalogue resolved every block and parameter of the loaded preset by name.

**Nothing has been written to this device** — no edit, no save, no backup, no restore. That is why
the entry is `Support::Untested` despite the reads reconciling cleanly.

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

Key `35` is recorded raw on purpose. `docs/helix-floor.md` notes `device_version = 0x03800000` for
firmware 3.82, which would make `0x03710020` read as ~3.71 — but that encoding has never been
pinned down against two known firmware versions, so this is data, not a conclusion.

## Preset model  [solid]

Reading the loaded preset (8326 bytes, declared length matched):

- **Both DSPs.** Preset key `1` is populated and blocks came back in slots 21–28 as well as 3–6,
  i.e. the global `slot = dsp * 20 + index` numbering the Floor established. The unit reported
  DSP1 71.0% used and DSP2 43.0%.
- **8 snapshots**, `SNAPSHOT 1`…`SNAPSHOT 8`, and the stored active index agreed with the live
  scene.
- Every one of the 8 blocks resolved to a `.models` definition by name, with device-ordered named
  parameters — no unmatched model, no unmatched parameter key.

## Setlists  [arity solid; names not corroborated]

Browsing each bank in turn:

| bank | result |
|---|---|
| 0 | 128 presets — `US Double Nrm`, `Essex A30`, `Brit Plexi Brt` (factory) |
| 1 | 128 presets |
| 2 | 128 presets — the user's own presets; `read-info` on the loaded preset reported `bank: 2, index: 0, name: "WIP"` |
| 6 | 128 presets |
| 7 | 128 presets — `Quick Start`, `Parallel Spans`, `SNP:4-Amp Spill` (templates) |
| 8 | refused, code `-3` |

So: **eight banks of 128**, bank 0 factory and bank 7 templates — the Floor's layout, and
`setlist_stride()`'s 128 fallback was already correct for this device.

The names in `DEVICES` are therefore the Floor's. Worth being explicit about the difference in
evidence: the Floor's names came from the eight `L6Setlist` streams of a real `.hxb`, whereas here
only the *arity* and the *character of the two end banks* were observed. If an LT backup ever turns
up and disagrees, this is the field to fix.

## Not observed

- **`preset_device_id`.** The handshake identity reply carries no `0x0021xxxx` device id, and the
  wire preset stream has no such field — the Floor's `0x210001` came from a `.hxb`. Left `None`
  rather than copied across.
- **Every write path.** Nothing was sent to this unit beyond reads and the handshake.

## Reproducing

With the LT connected and the udev rule installed:

```
fretwire detect                 # Helix LT: present (untested device)
fretwire connect                # handshake OK — device reports "P21"
fretwire pull                   # the preset above, blocks and params resolved
fretwire setlists               # 8 setlists
fretwire presets 7              # the templates bank
fretwire dump-raw lt.raw        # then: fretwire tree lt.raw
```
