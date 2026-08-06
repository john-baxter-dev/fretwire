# Helix Floor — first hardware bring-up test

Everything in `docs/helix-floor.md` is verified against USB captures and a device backup, **offline**.
Nobody has run fretwire against a physical Helix Floor. This is the checklist for that first run.

Two things only hardware can answer:

1. Whether the udev rule + `nusb` can actually claim interface 0 on a Floor.
2. Whether the handshake completes against a **live** device rather than a recording.

Both are exactly where a Windows-recorded capture can mislead. Everything below is **read-only** —
no writes, no flash, nothing in the firmware/DFU space that carries brick risk (see `docs/safety.md`).

---

## Message to send the tester

Everything from here to the end of the section is meant to be copied verbatim.

> ### Setup (Linux, one time)
>
> ```
> git clone -b helix_floor https://github.com/john-baxter-dev/fretwire
> cd fretwire
> cargo build -p fretwire-cli --release
> ```
>
> ### 1. Import the reference data from your own HX Edit install
>
> fretwire ships none of Line 6's data — it reads the model and parameter tables from *your* HX Edit
> installation. **Nothing works without this step**, so do it first.
>
> Copy the `res` folder out of your Windows HX Edit installation directory onto the Linux box, then:
>
> ```
> ./target/release/fretwire import-data /path/to/res
> ```
>
> (An HX Edit installer `.exe`/`.msi` also works, but needs `7z` installed to unpack. The `res`
> folder is simpler.)
>
> ### 2. Install the USB rule, then unplug and replug the Helix
>
> ```
> ./target/release/fretwire install-udev
> ```
>
> The replug matters — the rule only applies when the device next enumerates.
>
> ### 3. The actual test — all read-only
>
> Run these **in order** and send me the output of each, including any errors:
>
> ```
> ./target/release/fretwire detect
> ./target/release/fretwire connect
> ./target/release/fretwire presets
> ./target/release/fretwire pull
> ```
>
> Have a preset with blocks on **both** paths loaded when you run `pull`.
> `FACTORY 1` `12B` "Pull Me Under" is ideal — that's the one we decoded from your captures, so I can
> compare the output against what we already know it should be.
>
> ### Please don't run these yet
>
> `save`, `write-roundtrip`, `delete-block`, or anything else that changes the device. Those write to
> it. Let's confirm reading works first; writes come later, and I'll want a fresh backup before them.
>
> ### If something goes wrong
>
> Any hang or error, this gives me what I need:
>
> ```
> RUST_LOG=trace ./target/release/fretwire connect 2>&1 | tail -50
> ```

---

## What each step proves

| Command | Expected | What it confirms |
|---|---|---|
| `detect` | `Helix Floor: present` | PID `0x4248` matches `fretwire_protocol::DEVICES`, and the udev rule took |
| `connect` | `connected to Helix Floor` + `2 DSP(s), 8 snapshots per preset` | The handshake completes live — the byte-identical claim holds off-capture |
| `presets` | a list of named presets (count unverified — the Stomp returns 126; the Floor holds 128 per setlist across 8 setlists, and we don't know whether the op returns one setlist or all) | Preset-list stream paging works at the Floor's larger scale |
| `pull` on `12B` | **15 blocks**, slots `1`–`12` and `27`–`38` | Global slot numbering (`dsp * 20 + index`) against real hardware, not a capture |

The `pull` output should match what we decoded offline, block for block:

```
DSP1: slots 1,2,3,4,5 (path A) and 11,12 (path B)
DSP2: slots 27,28 (path A) and 33,34,35,36,37,38 (path B)
per-DSP load: DSP1 38.4% · DSP2 58.9%
FS4→27, FS5→28, FS10→37, FS11→38
```

Any divergence there is the interesting result — it would mean the live read differs from the
recorded one, which nothing so far predicts.

## Failure modes worth expecting

- **Permission error on `connect`.** The udev rule didn't take — almost always a missing replug.
  `ls -l /dev/bus/usb/*/*` and check the Floor's node is group-writable / `uaccess`-tagged.
- **Handshake timeout.** `Session::connect` retries three times, dropping and re-opening the
  interface each time to clear stale device state. That path has never been exercised on this
  device. The `RUST_LOG=trace` output above is what diagnoses it.
- **`no reference data` from `connect`.** Step 1 was skipped or pointed at the wrong folder;
  `fretwire import-data` prints where it wrote and what it found.

## After it passes

Then, and only then, the write path — with a fresh `.hxb` backup taken first, and starting with
`write-roundtrip` (rewrites the current preset unchanged, edit buffer only, reversible by reloading
the preset) before anything that touches flash.
