# USB Capture Procedure

Goal: record short, **single-action** USB sessions between HX Edit and the HX Stomp so we can
isolate each message type on the MI_00 control interface.

Device: Line 6 HX Stomp — **VID 0x0E41 / PID 0x4246**.

## One-time setup
1. Close HX Edit.
2. Launch **Wireshark** (as Administrator — USBPcap needs it).
3. **The Stomp is on `USBPcap5`** (identified 2026-06-21). Note: this interface also carries
   constant traffic from other devices and the Stomp's own audio streaming — we filter that
   out in analysis (see below), so don't worry about the noise while capturing.

## Per-capture loop (do this for each labeled action)
1. Have HX Edit open and **idle** on the relevant screen.
2. Start capture on the Stomp's USBPcap interface.
3. Perform **exactly one** action (see the list in `../ROADMAP.md` Phase 1). Keep it minimal —
   e.g. turn a single knob a known amount, or toggle one block's bypass.
4. Stop capture immediately.
5. Save as `NN-<short-action>.pcapng` (e.g. `03-amp-drive-up.pcapng`).
6. Copy `_TEMPLATE.md` to `NN-<short-action>.md` and fill in exactly what you did
   (which block, which param, before/after displayed value). The labels are what let us
   correlate bytes → meaning.

## Tips
- Smaller is better: 1–2 seconds of capture around a single action beats a long recording.
- Capture the **startup handshake** separately: start capture first, *then* launch HX Edit.
- Don't worry about the audio interface (MI_01) traffic — we'll filter to MI_00 in analysis.
- A useful Wireshark display filter once we know the device address: `usb.device_address == N`.

## Isolating the Stomp's control traffic (analysis)
`USBPcap5` is noisy (other devices + the Stomp's isochronous audio). To get down to just the
MI_00 control channel:
1. Find the Stomp's address: `Statistics → Conversations → USB`, or filter the descriptor
   packet `usb.idVendor == 0x0e41` and read its `usb.device_address`.
2. Then apply: `usb.device_address == N && usb.transfer_type != 0x01`
   (`0x01` = isochronous = audio; control/bulk/interrupt are what we want).
3. The MI_00 control endpoints are bulk or interrupt — those carry the protocol.
