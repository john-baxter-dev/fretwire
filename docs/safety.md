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
A chunked op-21 write can wedge the device — reproduced three times in the field 2026-07-31 by
dragging the mixer node, needing a power cycle each time. It is 🟡 (transient, flash untouched, no
data lost), but it is the one operation we know can take the pedal out mid-session, and **it is not
fixed** — only contained.

The device grants one flow-control credit per 496-byte chunk. Since 2026-07-31 `write_preset` waits
for each credit and aborts once it runs more than 2 chunks ahead; `Transport::send` has a 2 s
timeout; the GUI heartbeat drops the session rather than beating into a dead device; and `close()`
skips the wire once `Session::device_lost` is latched. Keep all four if you touch these paths — in
particular **never make the OUT path unbounded again**, since a device that stops draining its
endpoint would otherwise block the editor forever.

Note that pacing was *not* the cause: with the host waiting properly for every credit the same action
still kills the unit at the same byte offset. Treat the credits as detection, not prevention.

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

## Probing an undecoded op wedges the device — assume it, don't hope [solid — 2026-08-22]

`fretwire probe-edit` sends an arbitrary op with an arbitrary body. It exists because reading a
refusal code is how the next op gets decoded, and every op reachable that way is edit-buffer only.
That does **not** make it safe to sweep.

**Op 58 with `{102: 1, 66: 255}` wedged an HX Stomp**: bulk OUT stopped draining, the device fell
off the bus, and it needed a power cycle. The same five ops sent a moment earlier with `{102: 1}`
alone had all answered `103: 0` — *accepted* — and changed nothing. So:

1. **An `accepted` reply from an undecoded op means nothing.** All five ops accepted a body they
   evidently did not act on. Acceptance is not evidence the body was understood, and it is not
   evidence the next body will be tolerated.
2. **One op, one body, then look at the device.** The failure above came from a loop over five ops.
   Worse, the loop *kept going* after the first timeout, sending four more commands to a pedal that
   had already stopped answering — which tells you nothing and delays noticing.
3. **Expect to lose the edit buffer.** A power cycle discards every unsaved binding, including any
   the user made themselves. Say so before probing, not after.
4. **Never probe while anything unsaved matters.** Reload or save first.

The guardrails below still held — no flash, no firmware, full recovery from a power cycle, and the
pedal came back unharmed. This is the failure mode the guardrails are *for*, and it is a normal
outcome of this kind of work rather than a surprise. Budget for it.

## Why the brick risk is low here
The control protocol we're implementing moves the same kinds of messages HX Edit exchanges
constantly. A malformed control frame, at worst, makes the device's parser reboot — recovered by
a power cycle. Persistent flash (firmware) is only written by the update path, which we
deliberately never touch. Protocol-decoding uncertainty means we still test incrementally:
back up, use a scratch preset, watch the unit after each new message type.
