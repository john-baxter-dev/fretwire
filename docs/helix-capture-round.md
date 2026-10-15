# Helix LT — capture round (DSP2 routing and the rest)

Instructions for recording what HX Edit sends to a **Helix LT**, so fretwire can do the same.

**About an hour in total.** Most of that is setup; each capture takes under a minute.

Also, don't worry about it being perfect. Even if it's not, it'll still be helpful.

---

## Before you start

**What you need**

- A **Windows** PC with **HX Edit** installed. A Linux PC running Windows in a VM also works (see
  "Capturing from Linux" at the end). macOS won't work because it can't easily record USB traffic.
- Your Helix connected to that PC over USB.
- About 15 minutes for setup, then about 45 minutes for the captures.

**The rules (please read these)**

1. **Never press Save** unless a step tells you to. Only step **C2** saves anything, and it saves to
   an empty slot. If HX Edit asks *"save changes?"* at any other point, choose **Don't Save**.
2. **Don't touch firmware updates**, Line 6 Updater, or Global Settings during the session.
3. **Don't type anything private while a capture is running.** The recording can include other USB
   devices on the same port group, including your keyboard. The only thing you'll type during a
   capture is a preset name.
4. **If the Helix or HX Edit freezes**, stop the capture, save it anyway (it's still useful), then
   power-cycle the Helix. That's safe: nothing is written to the pedal until you save.
5. **If something on screen doesn't match these instructions**, don't improvise. Write down what you
   saw in your notes, skip that step and carry on. A note that says "HX Edit wouldn't let me do this"
   is a useful result in its own right.

---

## Setup (once)

### S1. Install Wireshark with USBPcap

1. Download Wireshark from <https://www.wireshark.org/download.html> (Windows x64 installer).
2. During install, **tick "Install USBPcap"** when the installer asks. It's a separate checkbox
   page near the end.
3. **Reboot.** USBPcap doesn't work until you do.

### S2. Find the USBPcap interface your Helix is on

1. Plug in the Helix, power it on and **open HX Edit** so it connects.
2. Open Wireshark. The start page lists interfaces named `USBPcap1`, `USBPcap2`, and so on.
3. Click the small **gear icon** next to `USBPcap1`. The dialog lists the USB devices attached to it.
   Look for **Line 6 / Helix**. If it isn't there, try `USBPcap2`, and so on. Note which one has it.
   - *Fallback:* if you can't tell, close HX Edit, start a capture on each `USBPcap` interface in
     turn, then open HX Edit. If it's the right one, you'll see a bunch of chatter start up.
4. Close the dialog.

### S3. Make a folder and a notes file

Make a folder called `helix-lt-captures`. Inside
it, create a text file **`notes.txt`** and put these at the top:

```
Device:            Helix LT
Helix firmware:    (on the Helix: the version shown in Global Settings, or HX Edit → Help → About)
HX Edit version:   (HX Edit → Help → About)
Windows version:
```

### S4. Back up the Helix first

In HX Edit, use **Backup** to save a full backup of the device into your folder. It's an `.hxb`
file. Name it `before.hxb`.

This backup does two jobs. It's your safety net, and it gives fretwire a copy of each preset in HX
Edit's own format to compare against what the pedal sends over USB.

---

## How every capture works

Every capture below is done the same way:

1. Get HX Edit into the **starting state** the step describes.
2. In Wireshark, double-click the `USBPcap` interface from S2. The capture starts.
3. **Wait 3 seconds.** Do **exactly** the action, nothing else. **Wait 3 seconds.**
4. Press the red **Stop** square in Wireshark.
5. **File → Save As**, keep the format as **pcapng**, and use the **exact file name** the step gives,
   for example `B4-move-block-path1-to-path2.pcapng`.
6. Add one line to `notes.txt`: the capture name, what you did, the values before and after, and
   anything HX Edit said or refused. Example:
   `B2: Output To on Path 1 went Multi → Path 2A → Path 2B → Path 2A+B → Path 2A. Path 2's input changed to "Path 1" by itself after the first one.`

Where a step says **"in this order, 3 seconds apart"**, do the changes one at a time in that exact
order and pause about 3 seconds between each. That lets each change be matched to its message.

**Leave HX Edit open and connected the whole time.** Don't close it between captures.

---

## Part A — Load some factory presets (no editing)

This one only *loads* presets. HX Edit reads each preset from the pedal as it opens it, and that's
the part we need. These particular presets route their signal in ways fretwire has never seen over
USB.

Look each one up **by name**. The location is given to help you find it. If a name isn't where it
should be (you may have overwritten it), look for it elsewhere. If it's gone, skip it and note that.

**A1 — `A1-load-presets.pcapng`.** Start the capture. Then, **in this order, about 5 seconds apart**,
load:

| # | Setlist | Location | Name |
|---|---|---|---|
| 1 | FACTORY 1 | 11B | **Trademark** |
| 2 | FACTORY 1 | 12D | **Plush Garden** |
| 3 | FACTORY 1 | 20B | **GARBAGE BASS** |
| 4 | FACTORY 2 | 12D | **Unicorns Forever** |
| 5 | TEMPLATES | 03D | **4 Tone Switcher** |
| 6 | TEMPLATES | 04D | **Stereo Mixer** |
| 7 | FACTORY 1 | 12B | **Pull Me Under** |

Stop and save. In `notes.txt`, list the seven names in the order you loaded them, with any you
skipped.

**A2 — no capture, just look.** With **Pull Me Under** loaded, look at the **Helix's own screen**
(not HX Edit) and write down exactly how it shows the preset number. For example `12B`, `045` or
`45`.

---

## Part B — Routing between Path 1 and Path 2 (the main event)

This is what fretwire can't do yet on a Helix: send Path 1 into Path 2, move blocks between the two
paths, and move a merge point across them. Everything in Part B happens in the edit buffer and is
**never saved**.

**Starting state for all of Part B:** find an **empty preset slot**, one named **`New Preset`**, in
one of your USER setlists, and load it. Write down which setlist and slot you used, because you'll
save into it in step C2. Do Part B, Part D and Part C in one go on that preset, without loading
anything else in between (except where step C3 says to).

> "Path 1" is the upper signal chain in HX Edit and "Path 2" the lower one. Each path has an
> **Input** block at its left end and an **Output** block at its right end. When a path splits, it
> gets a **Split** point where it divides into an upper and lower row, and a **Merge** point where
> they rejoin.

**B1 — `B1-path2-input.pcapng`.** Click **Path 2's Input** block. Write down what its **Input**
setting shows (probably `None`). Start the capture, change it to **Guitar**, stop and save.

**B2 — `B2-path1-output-to.pcapng`.** Click **Path 1's Output** block. Write down its **Output To**
value (probably `Multi`). Start the capture. Then, **in this order, 3 seconds apart**, set
**Output To** to:

1. `Path 2A`
2. `Path 2B`
3. `Path 2A+B`
4. `Path 2A`

Stop and save. **Then look at Path 2's Input block again** and write down what it says now. It may
have changed by itself, and that matters.

**B3 — `B3-path2-output-to.pcapng`.** Click **Path 2's Output** block. Start the capture. **In this
order, 3 seconds apart**, set **Output To** to `1/4"`, then back to `Multi`. Stop and save.

**B4 — `B4-add-two-blocks.pcapng`.** Start the capture. On **Path 1, upper row**, click the first
empty spot and add **Delay → Simple Delay**. Wait 3 seconds. Click the next empty spot to its right
and add **Reverb → Plate**. Stop and save.

**B5 — `B5-move-block-path1-to-path2.pcapng`.** Start the capture. **Drag Simple Delay** from Path 1
down onto the **first empty spot on Path 2's upper row**. Stop and save. Note where it landed. If
HX Edit won't allow the drop, note that.

**B6 — `B6-move-block-path2-to-path1.pcapng`.** Start the capture. Drag Simple Delay **back** to its
original spot on Path 1, left of Plate. Stop and save.

**B7 — `B7-make-parallel-path1.pcapng`.** Start the capture. **Drag Plate straight down** onto Path
1's **lower row**. Path 1 should now show a Split and a Merge. Stop and save.

**B8 — `B8-move-merge-to-path2.pcapng`.** Start the capture. Grab **Path 1's Merge point** and drag
it **down onto Path 2**. The goal is for Path 1's two rows to stay separate and rejoin on Path 2
instead. Stop and save. Describe what you see. If HX Edit won't let you, write "not possible" and
skip B9.

**B9 — `B9-move-merge-back.pcapng`.** Start the capture. Drag the Merge **back up** to Path 1, to
the right of Plate. Stop and save.

**B10 — `B10-path2-parallel.pcapng`.** Start the capture. Drag **Simple Delay** to the first empty
spot on **Path 2's upper row** (as in B5). Wait 3 seconds. Then drag it **down onto Path 2's lower
row**. Path 2 should now have its own Split and Merge. Stop and save.

Don't save the preset. Carry straight on with Part D.

---

## Part D — Ordinary edits

These are things fretwire already does on other devices. Recording them on yours checks that it
sends exactly what HX Edit sends. Still on the same preset, still **not saving**.

**D1 — `D1-knob.pcapng`.** Click **Plate**. Start the capture. Drag its **Mix** knob slowly from
wherever it is **all the way up**. Stop and save. Note the start value.

**D2 — `D2-bypass.pcapng`.** Start the capture. Turn **Plate** off with its power/bypass button.
Wait 3 seconds. Turn it back **on**. Stop and save.

**D3 — `D3-model-swap.pcapng`.** Start the capture. Change **Plate**'s model to **Reverb → Hall**.
Stop and save.

**D4 — `D4-add-amp-cab.pcapng`.** Start the capture. On **Path 1's upper row**, click an empty spot
and add **Amp+Cab → US Deluxe Nrm**. Stop and save.

**D5 — `D5-amp-swap.pcapng`.** Start the capture. Change that amp's model to **Essex A30** and keep
its cab. Stop and save.

**D6 — `D6-delete-block.pcapng`.** Start the capture. **Delete** the amp+cab block you just added.
Stop and save.

---

## Part C — Controller assignments, then the one save

**C1 — `C1-controllers.pcapng`.** Click **Hall** (the former Plate) and open **Controller Assign**
for its **Mix** parameter.

First, with no capture running, **write down the full list of controllers HX Edit offers, top to
bottom, exactly as shown**. For example `None, EXP 1, EXP 2, …, FS1, …, MIDI CC, Snapshots`.

Then start the capture and, **in this order, 3 seconds apart**, set the controller to each of these
that the list offers (skip any it doesn't):

1. `EXP 1`
2. `EXP 2`
3. `EXP 3`
4. each **Variax** knob it lists, one by one
5. `FS1`
6. `FS7`
7. **MIDI CC**, choosing **CC 20** if it lets you pick (otherwise whichever CC it picks; write down
   the number)

**Leave it on MIDI CC.** Stop and save.

**C2 — `C2-save.pcapng`.** Start the capture. **Save** the preset into the empty slot you started
from, named **`FRETWIRE TEST`**. Stop and save the capture. This is the only save in the whole
session.

**C3 — `C3-reload.pcapng`.** Load any other preset first, with no capture running. Then start the
capture, load **FRETWIRE TEST** again, wait 5 seconds, and stop and save.

**C4 — export.** In HX Edit, **export FRETWIRE TEST** as a preset file (`.hlx`) into your folder,
named `fretwire-test.hlx`.

Afterwards you can delete FRETWIRE TEST from the pedal, or keep it.

---

## Send it back

Zip the whole folder. It should contain:

- [ ] `notes.txt`, filled in
- [ ] `before.hxb`
- [ ] `A1` … `D6` and `C1` … `C3`, the `.pcapng` files (any you skipped are noted in `notes.txt`)
- [ ] `fretwire-test.hlx`

Send the zip to John however you've arranged. GitHub won't accept `.pcapng` attachments, but it
does accept `.zip`.

**What's in it, privacy-wise:** your preset names and settings (from the backup and the captures),
the Helix's serial/firmware details, and any other USB traffic on that port group during the
captures (see rule 3).

---

## Capturing from Linux (instead of Windows)

Only if you run HX Edit in a Windows VM with the Helix passed through to it. Capture on the Linux
**host**, not inside the VM:

```
sudo modprobe usbmon
lsusb | grep -i line6          # note the Bus number, e.g. "Bus 003"
sudo dumpcap -i usbmon3 -w B1-path2-input.pcapng   # usbmon<bus number>; Ctrl-C to stop
```

`dumpcap` comes with Wireshark. Everything else (steps, file names, notes) is the same.

---
---

## For reference, what each capture closes

| Capture | Open question it answers | Where it goes |
|---|---|---|
| `before.hxb` (LT) | the LT's `preset_device_id`, never on the wire (the Floor's came from its `.hxb`); and its setlist names | `fretwire_protocol::DEVICES`, `docs/helix-lt.md` |
| A1 + `before.hxb` | ~~**topology `AB`'s key-21 value**~~ — **answered: `4`**, from a Helix Rack's Plush Garden (issue #24, 2026-10-14). Originally: Trademark / Plush Garden are `SAB`/`AB`, GARBAGE BASS `AB`/`A`, Unicorns Forever `SABJ`/`AB`, 4 Tone Switcher and Stereo Mixer `AB`/`AB` (from the 2026-07-22 Floor backup). Each loaded stream pairs with its tone in the backup; Pull Me Under is the known control | `tone::topology_code` |
| A1 + `before.hxb` | ~~more tone+wire pairs for the **input/output nodes' ragged prefix** rule~~ — **answered**: stored only while routed, from a Helix Rack's backup and dumps (issue #24, 2026-10-14) | `tone::apply_io_nodes` |
| A2 | the LT's `presets_per_bank` (`None` today) | `DEVICES` |
| B1–B3 | how the **Input `@input` / Output To `@output`** selectors are written: plain op 30 on slot 0/9 (and 20/29), or something else; plus what the pedal does to Path 2's input on its own. `Output To` is `HD2_AppDSPFlowOutput` param 2, `output_type` enum (0 None, 1 Multi, 2 Path 2A, 3 Path 2B, 4 Path 2A+B, …) | new session op, GUI I/O node panel |
| B5, B6, B10 | **cross-DSP block move**: whether op 43 just takes `{75: <20, 76: ≥20}` | lifts `"can't move a block between DSPs"` in `Session::{place_block, insert_block}` |
| B7–B9 | **the cross-DSP bracket** (`SAB`/`ABJ`): whether HX Edit writes it surgically (ops 58–62?) or by op 21, and which keys change. Node position on one DSP is op 21 today (`set_node_pos`) | `Session` routing planning |
| D1–D6, B4, C2, C3 | the **LT write path** byte-for-byte (set_value, bypass, swap incl. paired amp+cab, add, delete, save, select). Already verified on the Floor (9/9). Matching on the LT moves it from `Reported` to `Verified` | `Support::Verified`, `docs/helix-lt.md` |
| C1 list + capture | the **Floor/LT source-ordinal table**: the `switch = ordinal − 5` offset and the hypothesis that 3..=5 are EXP 3 and the Variax knobs; FS7 checks the numbering past the Mode switch [hypothesis: FS6 is Mode on a Floor/LT — unverified here] | `tone.rs` `switch_offset`, `edit::source::Layout` |
| C1 (MIDI) + C3 + `fretwire-test.hlx` | **the MIDI-sourced controller row**: op 37's body for a MIDI source, plus one tone+wire pair carrying the CC number (none of the backup's 363 presets has a MIDI-sourced assignment) | `tone.rs` `midi_skipped` → carried |

