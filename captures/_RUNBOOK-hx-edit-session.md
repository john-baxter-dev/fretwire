# Runbook: one HX Edit capture session

Everything currently blocked on watching HX Edit do something, in one sitting. Ordered by what it
unblocks, so stopping early still leaves the valuable ones done.

**A and B are the two to make sure of** (2026-08-25). A is not even a capture — one backup plus a
few `dump-raw`s afterwards — and it closes the whole remaining gap in `.hxb`/`.hlx` import. B is the
only thing that safely opens footswitch colours, which have already cost one power cycle to guess at.
**F needs a two-DSP unit** (Floor or LT) and cannot be done on a Stomp; **G is obsolete** — that
area moved to Linux when the read op turned up.

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

## Checklist

Tick as you go. Sections below have the detail and the reasoning; this is the thing to have open.

**In HX Edit (Windows)**

- [ ] **Take the whole-device `.hxb` backup.** ⭐⭐ — §A
- [ ] Write down the preset slot number holding an **Amp+Cab** block ⭐ — §A
- [ ] …and one holding a **Cab › Dual**, an **IR** block, a **Looper**, a **synth** block — §A
- [ ] `fs_colour_set.pcapng` — one switch's ring colour, note the colour's exact name ⭐ — §B
- [ ] `fs_colour_set_second.pcapng` — a different colour, same switch ⭐ — §B
- [ ] `fs_label_set.pcapng` — custom name on that switch, note the exact text — §B
- [ ] `fs_label_clear.pcapng` — clear it again — §B
- [ ] `node_move_split_right_one.pcapng` / `_back` — drag the split a column, and back — §C
- [ ] `node_move_mixer_left_one.pcapng` / `_back` — same for the mixer — §C
- [ ] `variant_delay_mono_to_stereo.pcapng` / `_stereo_to_mono` — note where the control is — §D
- [ ] `variant_mod_mono_to_stereo.pcapng` — same on a modulation block — §D
- [ ] `dualcab_create.pcapng`, `dualcab_pan_left.pcapng`, `dualcab_swap_second.pcapng` — §E
- [ ] Photo: the **footswitch colour picker open**, showing its option list — §B
- [ ] Photo: the **mono/stereo control**, wherever HX Edit puts it — §D
- [ ] Photo: the **DSP meter** on a busy preset — settles whose number we display

**Back on Linux** — these pair with the backup and are where its value is realised

- [ ] `fretwire goto <slot>` + `fretwire dump-raw ampcab.bin` for the Amp+Cab slot ⭐ — §A
- [ ] …and one dump each for the dual-cab, IR, looper and synth slots — §A
- [ ] `fs_before.bin` / `fs_after.bin` around the colour change — §B
- [ ] `variant_before.bin` / `variant_after.bin` around the first variant change — §D

**Per capture, every time:** one action, 1–2 s around it, stop, and fill in a copy of
`_TEMPLATE.md`. An unlabelled capture is close to worthless.

**Skip:** §F (needs a two-DSP unit — a Stomp has no Path 2) and §G (obsolete; that work moved to a
Linux `settings-dump` → change one menu item → `settings-dump` → `settings-diff` loop).

---

## A. The `.hxb` backup, and the Linux dumps that pair with it ⭐⭐ cheapest, highest value

**Unblocks:** every remaining refusal in the `tone` → wire conversion, which is what stands between
us and restoring a preset from a backup or importing a shared `.hlx`.

This is not a capture. It is **one backup in HX Edit plus a few `dump-raw`s on Linux afterwards**,
and it is worth more than anything else on this page because of how the conversion is verified: a
preset held in *both* forms — host-side `tone` JSON from the backup, and the device's own bytes from
a dump — is an oracle that settles an encoding question outright. That is how the whole mapping was
proved (see `docs/preset-format.md`), using a contributor's Floor backup that happened to overlap a
capture. Doing it deliberately, on a unit we have, closes the rest.

1. **Take the backup.** Whatever HX Edit calls "create/save a device backup" — the whole unit, one
   `.hxb`. Keep it out of git (`captures/` ignores `*.hxb`; it is your own preset data).
2. **While you are in there, write down which preset slots contain each of these**, because that is
   the part that cannot be recovered later:
   - an **Amp+Cab** block ⭐ — the one that matters most, see below
   - a **Cab › Dual** block (also section E)
   - an **IR** block
   - a **Looper**
   - a **3 Note Generator** or other synth block
3. **Back on Linux**, for each slot noted: `fretwire goto <slot>` then
   `fretwire dump-raw ampcab.bin` (name it after the block it is for).

**Why Amp+Cab is the one to make sure of.** An Amp+Cab block is one block on the wire, with the cab's
index at `24 → 26` and its parameters in bank `12`. The `tone` names the paired cab with a plain
`HD2_Cab…` symbol; the one paired dump we hold stores a `HD2_CabMicIr_…` one, a different family with
a different parameter list (`Mic` first instead of appended, and the trailing `IrData` not stored).
Nothing pins the two together, so amp+cab blocks are **refused** rather than converted — about a
fifth of a factory setlist, and it is how most people build a preset. One backup-plus-dump pair ends
that.

The other four are the same trick applied to the block classes still marked unknown (`@type` 4, 5,
6, 8 in `fretwire_data::tone`). Each pair settles one, and the refusal messages name what is missing.

## B. Footswitch ring colour and custom label ⭐

**Unblocks:** picking a colour and a name per footswitch in the editor — explicitly one of the few
things HX Edit still does that we cannot.

**Do not probe this.** `probe-edit --op 58` wedged a pedal on 2026-08-22 and cost a power cycle; all
five candidate ops (58-62) accept a bare `{102: switch}` and do nothing with it, so acceptance says
nothing. At current knowledge it is roughly one power cycle per guess. **This capture turns it from a
search into a confirmation**, which is the whole reason it is worth a session slot.

Reading is already done: op 33 returns the switch record, `109` is the label and `67[].66` the
assignment's colour as `0xRRGGBB`. The `tone` side carries the same fields (`@fs_ledcolor`,
`@fs_customlabel`) and now round-trips through a conversion. What is missing is only the **write**.

Start from a preset with at least one block bound to a footswitch.

1. `fs_colour_set.pcapng` — change one footswitch's **ring colour** to a specific option. Nothing
   else. **Write down which switch, and the exact name of the colour you picked** — the option list
   is worth as much as the bytes, and a photo of the picker open is ideal.
2. `fs_colour_set_second.pcapng` — a **different** colour on the **same** switch. The pair is what
   separates "which op" from "which field".
3. `fs_label_set.pcapng` — give that switch a **custom name**. Write down the exact text.
4. `fs_label_clear.pcapng` — clear it again.

Dump before and after #1 (`fs_before.bin` / `fs_after.bin`) so the result can be diffed as well as
the command. Top-level `66` on the switch record stays nil while the assignment's own colour is set,
so which of the two the write lands on is one of the open sub-questions.

## C. The node move ⭐ highest priority

**Unblocks:** the op-21 whole-preset write lockup, open since Round 21, and the only thing on record
that has wedged the pedal hard enough to need a reboot. Ending each unit on a short USB packet took
it from 68% of writes to **15%** (34 writes since, 5 wedged) — much rarer, not gone, and every one
of the recent ones was a **node move**, which is exactly what this section captures. The editor now
gives up at the first uncredited chunk and says so, so the failure is at least diagnosable; the
pedal still needs a power cycle afterwards.

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

## D. Mono ↔ stereo on an existing block ⭐

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

## E. A two-cab (`Cab › Dual`) block

**Unblocks:** dual-cab support, which is currently absent and probably reads back as half of itself.
Step 1 below is now part of section A — if you did that, the dump exists and only the captures here
are still open.

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

## F. The output block's **destination** — DSP2 routing — *needs a Floor or an LT, not a Stomp*

**Skip this on an HX Stomp.** The Stomp has one DSP (`Device::dsps == 1`, and key `1` of its presets
is nil), so there is no Path 2 to route to and no way to perform the action this section describes.
It stays here for whoever next has a two-DSP unit in front of them.

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

In practice: start from a plain serial preset with one path, click the **output block** at the right
end of Path 1, and change its destination from Multi (or whatever it is) to **Path 2**. HX Edit will
probably draw a second path as soon as you do — that is the moment we want on the wire. **Write down
where the control was and what the options were called**, same as step C; the option list is worth
as much as the bytes, and a photo of that dropdown open is ideal.

1. `route_path1_output_to_path2.pcapng` — set Path 1's output to Path 2. Just that.
2. `route_path1_output_to_multi.pcapng` — and back to Multi/main.
3. `route_path2_input_source.pcapng` — change **Path 2's input** source, wherever HX Edit puts it.

Dump before and after #1 (`route_before.bin` / `route_after.bin`) — the diff should show key `6`
on slot 9 changing, and confirms whether anything else moves with it.

If HX Edit turns out to send this as an op-21 whole-preset write rather than a small edit, say so —
that is the same operation as section A, and the two captures then answer each other.

## G. Global settings — op-25 id space — **obsolete, do not spend session time here**

This section said "we can already write settings and cannot read any back". That stopped being true
on 2026-08-22: the **read is op 24**, it had been sitting in the connect capture mis-labelled as a
"prepare step", and settings are a flat numbered namespace. A 601-id sweep takes 1.4 s.

So the whole area is now a **Linux** job with no Windows box in it — `settings-dump`, change one
thing on the pedal's own menus, `settings-dump` again, `settings-diff`. That loop found nineteen ids
in one sitting on 2026-08-25 and closed both of the empty menus; `_TODO-settings-names.md` carries
what is left of it. Still no capture required.

## H. A block with two or more values past its symbol list — *only if you happen to hit one*

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

**A screenshot of the mono/stereo control** wherever HX Edit puts it, from step C. Worth having on
its own even if the capture is messy.

## Doesn't need Windows at all

Listed so nobody spends session time on them:

- ~~**Type-49 pushes** — pedal-side changes not reaching our UI~~ — **found, 2026-08-05.** The
  decoding was fine; the frames were being thrown away before they got to it, by the code waiting
  for a reply to something else (`zadtheinhaler57` binned 111 of them in one session). Now they go
  back on the queue. **Please confirm on the next build:** press a footswitch to bypass a block and
  watch the GUI *without* touching it — it should follow within a second, and turning a knob on the
  pedal should move the matching parameter too. If it still needs a click to catch up, say so, and
  run once with `FRETWIRE_TRACE_STATUS=1` set so the log carries the push bytes.
- ~~A `-306` ladder on a serial preset~~ — **done, from the `.hxb` backup instead** (2026-08-04).
  458 real DSPs say the ceiling is the same serial or parallel, so the split and mixer are not
  billed against it. See `docs/protocol.md`.
- **Why the filters went dead** (Round 26 — the answer is probably level, not routing). Put an
  envelope filter in path B, play it, then raise the split's `Balance B` or the filter's own
  Sensitivity and play again. If it wakes up, that closes two evenings of "no worky" for good.
- **Anything about audio.** Captures show commands, not sound.
