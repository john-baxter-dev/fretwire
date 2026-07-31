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

### Known lockup: the whole-preset write (op 21)
A chunked op-21 write can wedge the device — reproduced twice in the field 2026-07-31 by dragging the
mixer node, needing a power cycle each time. It is 🟡 (transient, flash untouched, no data lost), but
it is the one operation we know can take the pedal out mid-session.

The device grants one flow-control credit per 496-byte chunk; **outrunning them is not safe**. Since
2026-07-31 `write_preset` waits for each credit and aborts once it is more than 2 chunks ahead, and
`Transport::send` has a 2 s timeout so a stalled endpoint can no longer hang the host. If you touch
that path, keep both properties. Never make the OUT path unbounded again — a device that stops
draining its endpoint will otherwise block the editor forever.

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
