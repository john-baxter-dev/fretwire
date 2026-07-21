# Hardware Safety

> **Firmware operations are OUT OF SCOPE and must never be transmitted.** They are the only
> realistic way to brick an HX Stomp. Everything else in this project is recoverable.

## Risk by activity
| Activity | Risk | Notes |
|---|---|---|
| USB captures | 🟢 none | passive observation |
| Parsing data / building the codec | 🟢 none | offline |
| Live control: param set, bypass, preset load | 🟡 low | same message *class* HX Edit sends; worst case is transient — audio glitch, confused session, or a **power cycle**. Flash is untouched. |
| Persistent writes: save preset/setlist, global settings | 🟡 low (data, not device) | risks corrupting *your data*; recoverable by factory reset / restoring a backup |
| **Firmware update / flash / bootloader / DFU** | 🔴 **brick risk** | **do not capture, replay, or transmit. If seen in captures: document and avoid.** |

## Guardrails (follow before transmitting anything)
1. **Back up the device first** — full HX Edit backup (`.hxb`: all setlists/presets/IRs/globals).
   Makes any data mishap fully reversible.
2. **Read-only / transient first** — handshake, open preset, read state, live param tweaks are
   all non-persistent. Stay here a long time before any "save to device" path.
3. **Scratch preset** — do write experiments on one throwaway slot.
4. **Know the recovery path** — HX Stomp reflash via Line 6 Updater / safe-boot (footswitch-hold
   on power-up). Confirm it works *before* experimenting.
5. **Stable power/cable during writes** — direct USB port, no flaky hub; never interrupt power
   mid-write.
6. **Linux: claim only interface 0** (the vendor control interface). Leave the audio interface alone.

## Why the brick risk is low here
The control protocol we're implementing moves the same kinds of messages HX Edit exchanges
constantly. A malformed control frame, at worst, makes the device's parser reboot — recovered by
a power cycle. Persistent flash (firmware) is only written by the update path, which we
deliberately never touch. Protocol-decoding uncertainty means we still test incrementally:
back up, use a scratch preset, watch the unit after each new message type.
