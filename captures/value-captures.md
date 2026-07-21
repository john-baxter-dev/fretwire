# Capture notes — value-encoding set (tremolo)

Four captures taken 2026-06-21, all on the **tremolo** block. Full decode in `docs/protocol.md`.

| file | action | key result |
|------|--------|-----------|
| `toggle_tremolo_on_off.pcapng` | bypass toggle | handle `82 62 04 3b`; block id **04** (reverb was 07) → `83 66 cd 03` is NOT the block addr |
| `set_tremolo_mix_to_100.pcapng` | drag Mix → 100% | value stream ends at `3f800000` = **1.0** (big-endian f32) |
| `toggle_tremolo_on_set_mix_to_0.pcapng` | toggle on, drag Mix → 0% | value stream ends at `00000000` = **0.0** |
| `startup.pcapng` | launch HX Edit | **analyzed**: per-channel SESSION_OPEN (ef03→ed03→f003), identity query per channel (→ `"P33Main"`/`"P33"` + ver), then meters + preset stream. See `docs/protocol.md` §Session handshake |

## Headline findings
- **Parameter values = 32-bit big-endian IEEE-754 float.** 1.0=`3f800000`, 0.5=`3f000000`,
  0.2=`3e4ccccd`, 0.0=`00000000`. Matches `.models` min/max (0.0–1.0). Knob drags stream every
  intermediate value as its own opcode-0x0006 frame.
- **Op class** is the byte after `83 66 cd`: `03` = toggle/bypass (short, no value),
  `04` = set value (long, trailing f32).
- **Block id** = 3rd byte of the `8X 62 [id] NN` target handle (reverb 07, tremolo 04).
- Re-extract any of these: `tools/dump-control.ps1 captures/<file>.pcapng`.
