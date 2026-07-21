# TODO capture: global settings (Input Z / impedance, pad, output levels) — op-25 id space

**Goal:** map the **op-25 global-settings id space** so the GUI can expose Global Settings →
Ins/Outs: **Input Z (impedance)**, guitar pad, output level switches (instrument/line), phones
level, and whatever else lives there. Everything per-preset on the input/output *blocks* (gate,
threshold, decay, level, pan) is already decoded and editable — this is only the **global** side.

## What we already know [solid]
- The write op: **op 25** `{102: txn, 100: 25, 101: {118: <setting id>, 119: <value>}}` — plain
  edit-channel envelope, not block-addressed. From `switch_input_gate_and_guitar_pad.pcapng`.
- One id mapped: **118: 134** — a 3-state input setting cycled `0 → 1 → 2` in that capture
  (recorded alongside the guitar pad toggle; which UI control it is exactly needs re-checking
  against what was clicked).
- Plumbing exists: `fretwire_protocol::edit::set_setting`, `Session::set_setting(id, value)`, CLI probe.
- **Unknown:** the read side. HX Edit must fetch current global values at startup/on opening the
  Global Settings pane — either in the connect sequence we already replay or via a dedicated read
  op. Without it the GUI can write settings but can't display them.

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

## Deliverables
- `docs/protocol.md`: a global-settings id table (`118 → setting, values → options`) tagged [solid].
- `fretwire_core`: typed wrappers over `set_setting` + the read path; GUI Ins/Outs panel after that.

## Safety
Op 25 writes global config, not flash/firmware — same risk class as preset edits (recoverable by
setting it back / factory-reset globals). Still: one setting at a time, note the original value
before cycling, restore it after each capture.
