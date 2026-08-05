# Runbook: one HX Edit capture session

Everything currently blocked on watching HX Edit do something, in one sitting. Ordered by what it
unblocks, so stopping early still leaves the valuable ones done.

Read `README.md` first for the Wireshark setup. **On a Helix Floor the device is VID `0x0E41` /
PID `0x4248`** (the Stomp is `0x4246`) — the USBPcap interface number will differ from the one the
README names, so find it by filtering `usb.idVendor == 0x0e41` and reading `usb.device_address`.

General rules, same as always:

- One action per capture, 1–2 seconds around it, stopped immediately.
- Name the file after exactly what was clicked.
- Copy `_TEMPLATE.md` alongside each one and fill it in. The labels are what make the bytes mean
  anything; an unlabelled capture is close to worthless.
- Where a step says **dump**, that's `fretwire dump-raw <name>.bin` on the Linux side afterwards,
  with the same base name. A capture shows the *command*, a dump shows the *result* — several of
  these need both.

---

## A. The node move ⭐ highest priority

**Unblocks:** the op-21 whole-preset write lockup, open since Round 21, and the only thing on record
that has wedged the pedal hard enough to need a reboot. Ending each unit on a short USB packet took
it from 68% of writes to 12% (Round 25) — so it is much rarer now, and the last 12% still needs
this capture.

We know HX Edit sends bare op-21 whole-preset writes — all 43 existing captures were swept for it on
2026-08-02. What we have never seen is HX Edit doing the **node** move specifically, which is the one
that locks up. We want its packetisation byte-for-byte: chunk sizes, pacing, how it handles
flow-control credits.

Start from a preset with a parallel path and at least one block on the lower row.

1. `node_move_split_right_one.pcapng` — drag the **split** one column right. Pause. Nothing else.
2. `node_move_split_back.pcapng` — drag it back to where it was.
3. `node_move_mixer_left_one.pcapng` — drag the **mixer** one column left.
4. `node_move_mixer_back.pcapng` — and back.

In the notes, record the column each node started and ended on, counting the same way the UI draws
it. If HX Edit *refuses* any of these, that is just as useful — say so and note the exact wording.

## B. Mono ↔ stereo on an existing block ⭐

**Unblocks:** letting the editor switch a block's variant. 153 models ship both; we read the variant
and cost it correctly, and the swap works on the wire, but the GUI's picker collapses to one entry
per model and only ever offers the variant the block already has. Before building a toggle we should
know what HX Edit actually sends — plain op 40 to the other symbol index, or something else.

1. `variant_delay_mono_to_stereo.pcapng` — take a **mono** delay and make it stereo, however HX Edit
   exposes that. **Write down where the control is** — that's half the answer.
2. `variant_delay_stereo_to_mono.pcapng` — and back.
3. `variant_mod_mono_to_stereo.pcapng` — same on a modulation block, to check it isn't per-category.

Dump before and after the first one (`variant_before.bin` / `variant_after.bin`) so we can diff what
changed in the block record besides the model ref.

## B2. The output block's **destination** — the one thing blocking DSP2 presets ⭐

**Unblocks:** building a preset that uses Path 2 (DSP2) at all. Everything else about DSP2 is
already solved — slots are global (`dsp*20+index`), edits to a DSP2 block are byte-identical to a
DSP1 one, an "empty" DSP2 still carries its full 20-slot array so the grid is already there, and
`add_block` derives the DSP from the slot. What is missing is the *routing*: nothing feeds Path 2
until Path 1's output is pointed at it.

That selector is **not a parameter**. On the output node (slot 9) the params array holds exactly
`pan` and `gain`; the destination is a sibling field, content key **`6`** — and the input node's
source is key **`5`**. Ordinary `set_value` addresses params by index in the model's symbol order,
so it cannot reach either one. Guessing the write is the sort of thing that hung the pedal once
already (an out-of-range int on a head selector), so this wants a capture rather than a probe.

The backup already gives us the *values*: across its 363 presets, `dsp0.outputA.@output == 2`
means Path 2 is fed (126 presets vs 5 where it isn't), while `== 1` is the ordinary output (202 of
229 unused). So we need the write shape, not the meaning.

1. `route_path1_output_to_path2.pcapng` — on a serial single-path preset, set **Path 1's output** to
   Path 2. Just that.
2. `route_path1_output_to_multi.pcapng` — and back to Multi/main.
3. `route_path2_input_source.pcapng` — change **Path 2's input** source, wherever HX Edit puts it.

Dump before and after #1 (`route_before.bin` / `route_after.bin`) — the diff should show key `6`
on slot 9 changing, and confirms whether anything else moves with it.

## C. A two-cab (`Cab › Dual`) block

**Unblocks:** dual-cab support, which is currently absent and probably reads back as half of itself.

There are 46 `HD2_CabMicIr_*WithPan` symbols and the pedal refuses an in-place swap to any of them
(`-306`, sometimes `-21`), so HX Edit must create them some other way. We have never decoded a real
one — our only "dual" fixture is a dual *amp*.

1. **`dualcab.bin` — the dump matters most.** Build a preset with a `Cab › Dual` block in HX Edit,
   save it, then dump it from Linux. Even with no capture at all this tells us where the second
   model ref lives.
2. `dualcab_create.pcapng` — turn a single cab into a dual one.
3. `dualcab_pan_left.pcapng` — move **cab A's** Pan.
4. `dualcab_swap_second.pcapng` — change which cab is in the **second** slot.

Note in the template which visual half of the block each action touched — A vs B is the whole
question.

## D. Global settings — op-25 id space

**Unblocks:** a Global Settings pane. We can already write settings and cannot read any back.

Full recipe is in `_TODO-global-settings.md`; it hasn't changed. Six small captures, Input Z first,
and the one that matters most is `global_settings_pane_open.pcapng` — **just opening the pane**, for
the read-side traffic. Without that the GUI can set values it can never display.

## E. A block with two or more values past its symbol list — *only if you happen to hit one*

Key 29 solved the single trailing extra (Trails on a delay/reverb, `Mic` on a legacy cab). A block
carrying **two** or more would stay read-only in the editor, because we have no evidence for what the
second index means.

Checked 2026-08-02: **no such block exists in any dump we hold.** Every one has at most one extra,
and that one is now addressable. So there is nothing to go looking for — but if the editor ever
shows a value greyed out as *"read-only — no confirmed address"*, that is the case, and a capture of
HX Edit changing that value closes it. Worth knowing about, not worth hunting for.

---

## Not a capture — but do these, they're cheap

**The DSP meter screenshot.** Load `somehinged3` in HX Edit and photograph its DSP readout. Ours
says 72.7% and the editor now reports the headroom that actually goes with it (~2.3%, against the
measured ~75 ceiling) instead of pretending the budget is 100. That part is settled — a census of
458 real DSPs pins the ceiling and rules out the routing nodes as the missing quarter, see
`docs/protocol.md`. What is left is cosmetic, and this screenshot decides it in thirty seconds:

- HX Edit reads **~97%** → it displays `blocks ÷ 75`, and we can show the same number users see in
  HX Edit rather than a raw sum plus a ceiling.
- HX Edit reads **~72.7%** → it shows the raw sum like we do, and the ceiling stays as-is.

No longer urgent — nothing about fit checking depends on it — but still cheap and still worth it.

**A screenshot of the mono/stereo control** wherever HX Edit puts it, from step B. Worth having on
its own even if the capture is messy.

## Doesn't need Windows at all

Listed so nobody spends session time on them:

- **Type-49 pushes** (`{98: slot}` — pedal-side model changes not reaching our UI): `fretwire watch`
  on Linux while changing a block's model with the joystick.
- ~~A `-306` ladder on a serial preset~~ — **done, from the `.hxb` backup instead** (2026-08-04).
  458 real DSPs say the ceiling is the same serial or parallel, so the split and mixer are not
  billed against it. See `docs/protocol.md`.
- **Why the filters went dead** (Round 26 — the answer is probably level, not routing). Put an
  envelope filter in path B, play it, then raise the split's `Balance B` or the filter's own
  Sensitivity and play again. If it wakes up, that closes two evenings of "no worky" for good.
- **Anything about audio.** Captures show commands, not sound.
