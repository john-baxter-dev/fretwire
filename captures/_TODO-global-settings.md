# Global / device settings — READ SIDE DECODED (2026-08-22), id space still to map

**Goal:** map the **op-25 setting id space** so the GUI can expose Global Settings → Ins/Outs:
**Input Z (impedance)**, guitar pad, output level switches, phones level, and whatever else lives
there. Everything per-preset on the input/output *blocks* is already decoded and editable — this is
only the **global** side.

## What we know [solid]
- **Op 24 reads: `{102:txn, 100:24, 101:{118:id}}`, and the reply carries the value at key 119.**
  This was the missing half, and it was hiding in plain sight — the connect capture sends
  `{118: 128}` and we had it written up as a "read-sequence prepare step". It is not a prepare
  step; settings are a flat numbered namespace and the handshake happens to fetch id 128.
- The write op: **op 25** `{102:txn, 100:25, 101:{118:id, 119:value}}`.
- **Settings are typed, and a write of the wrong type is refused with `-3`.** Tempo is an `f32`, so
  the old integer-only `set_setting` could never write it. `Session::set_setting_num` reads the
  current value first and sends back its type.
- **166 of ids 0..=260 answer on an HX Stomp.** Named so far: **16** tempo in BPM (`f32`), **28**
  current preset index (`int`), **192** global EQ low-peak gain, **201-203** global EQ.
- Plumbing: `edit::{read_setting,set_setting,set_setting_value}`,
  `Session::{read_setting,scan_settings,set_setting,set_setting_num}`, CLI `setting-get`,
  `setting-set`, `settings-dump`, `settings-diff`.

## Mapping the rest needs no capture at all
With a read op the Windows box is unnecessary. The loop is:

```
fretwire settings-dump before.txt      # 166 ids
# change exactly one thing on the pedal's own menus
fretwire settings-dump after.txt
fretwire settings-diff before.txt after.txt
```

The diff names the id. Verified end to end: with tempo moved 80 -> 132 the diff reported exactly
`16: 80 -> 132` out of 166 ids and nothing else.

**Do one setting at a time**, and note the original value so it can be put back. Priorities are the
ones the GUI wants first: **Input Z (impedance)**, guitar pad, main out level (instrument/line),
and the **preset numbering** flag (`01A` vs `000`) — that last one is the cheapest possible first
consumer, since it is confirmed absent from every preset stream we read and the GUI currently
carries a manual toggle for it.

## Safety
Op 25 writes global config, not flash/firmware — same risk class as preset edits, recoverable by
setting it back. Read before writing (which `set_setting_num` does anyway, since it needs the type).

---

## Original capture recipe (kept for reference — no longer the only route)

## Capture recipe (Windows box, `tools/dump-control.ps1` + HX Edit)
One capture per setting, changing **only that setting**, stepping through **all its values in
order** with a beat between clicks (id ↔ control mapping falls out of the value sequence). Name
each file after exactly what was clicked, e.g.:

1. `global_input_z_cycle.pcapng` — Input Z through every option (Auto, 22k, 32k, 70k, 90k, 136k,
   230k, 1M, 3.5M). **The impedance one — highest priority.**
2. `global_guitar_pad_on_off.pcapng` — pad on → off → on (isolated this time).
3. `global_output_level_inst_line.pcapng` — main out instrument ↔ line.
4. `global_phones_monitor_level.pcapng` — if it's a global (continuous → expect a 119 float).
5. `global_settings_pane_open.pcapng` — **just opening** the Global Settings pane after connect,
   for the read-side traffic (what op fetches current values?).
6. `global_restore_defaults.pcapng` — optional: factory-default globals; a bulk write would reveal
   many ids at once.

## Decode
`python3 tools/decode-edits.py <cap>` — expect op-25 envelopes; the id (118) per capture names the
setting, the 119 sequence maps values → menu options. For the pane-open capture, look for a
read/query op and its reply shape (may ride the PRIMARY channel like the preset list does).

## Follow-on
The id space is mapped enough to ship (27 named as of 2026-08-23) and the deliverables below are
done. What is **not** done is checking the names themselves against the pedal's screens — see
[`_TODO-settings-names.md`](_TODO-settings-names.md), which exists because id `127` carried an
invented name for a day.

## Deliverables
- `docs/protocol.md`: a global-settings id table (`118 → setting, values → options`) tagged [solid].
- `fretwire_core`: typed wrappers over `set_setting` + the read path; GUI Ins/Outs panel after that.

## Safety
Op 25 writes global config, not flash/firmware — same risk class as preset edits (recoverable by
setting it back / factory-reset globals). Still: one setting at a time, note the original value
before cycling, restore it after each capture.
