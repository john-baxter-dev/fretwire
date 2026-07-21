# Capture: capture1_toggle_reverb_block

- **pcap file:** `capture1_toggle_reverb_block.pcapng`
- **date:** 2026-06-21
- **HX Edit screen / context:** Edit view, reverb block selected

## Action performed
- **What:** pressed the block **bypass** switch (toggle)
- **Block / slot:** reverb block
- **Parameter:** bypass / `@enabled`
- **Notes:** The single intended toggle registered as **two** edit-channel commands
  (on→off→on), frames 2307 and 4851.

## Analysis notes
- Control channel: device addr 8, interrupt EP 0x01 OUT / 0x81 IN, 16-byte frames.
- Edit happens on channel `ed03/8010`, inner **opcode 0x0006**, `ilen=13`.
- Target handle (reverb block): **`83 66 cd 03`** (constant across both toggles).
- Only a transaction-counter-looking byte differed between the two toggles — bypass value
  encoding still ambiguous. See `docs/protocol.md` for the full decode + next steps.
- Re-extract anytime: `tools/dump-control.ps1 captures/capture1_toggle_reverb_block.pcapng`.
