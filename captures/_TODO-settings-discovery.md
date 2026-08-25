# Footswitches and EXP Pedals — two whole menus with no ids

**Goal:** name the settings ids behind the pedal's **Footswitches** and **EXP Pedals** sections.
Both are populated on the hardware and `fretwire_protocol::settings` has **zero** entries for either.

**Prize:** ~131 of the 166 ids that answer on an HX Stomp are still unidentified. These two sections
are the largest block of them whose *location is already known*, which is what makes this pass cheap:
you are not sweeping a namespace, you are walking a menu you can see.

## Why now

Until 2026-08-24 nobody had checked what sections the pedal's Global Settings actually has. It has
six — `Ins/Outs · Preferences · Footswitches · EXP Pedals · MIDI/Tempo · Displays` — confirmed on a
Stomp and an XL independently. Two of them were entirely absent from our table. They are declared
now, and empty, precisely so this file has somewhere to put the answers.

Every id found here is one the GUI stops showing as `Setting 143` and starts showing by name, and
one that becomes writable.

## The loop

Reads only. `settings-dump` is op 24 and touches nothing; **do not** use `probe-edit` here — see
`docs/safety.md` and the 2026-08-22 incident where op 58 wedged a pedal.

```
cargo run -p fretwire-cli -- settings-dump before.txt
#   change exactly ONE item on the pedal's own menus
cargo run -p fretwire-cli -- settings-dump after.txt
cargo run -p fretwire-cli -- settings-diff before.txt after.txt
```

`settings-diff` names the id that moved. Then `before.txt` becomes the new baseline — or just dump
again after each change and diff consecutive pairs.

**One item per diff.** Two changes between dumps gives two moved ids and no way to attribute them,
and re-deriving which was which costs more than the second dump saves.

## What to write down

The thing this project keeps getting wrong is recording *half* an observation. Id `127` sat for two
days with both its values seen and its menu text unrecorded, so which integer meant `First` was
unrecoverable and had to be read again. Id `127` before that carried a **name** — `Guitar In-Z` —
that no pedal has ever shown.

So for every item, capture all four:

1. the **exact menu text**, casing and spacing as drawn (`EXP/FS Tip`, not `Exp/FS tip`)
2. the **id** `settings-diff` named
3. the **value before and after**, each next to the menu text that was showing at the time
4. every **other option** the item offers, if you cycle through them

Item 3 is the one that gets skipped. `0 → 1` on its own is not a result.

## Footswitches

| menu text (exact) | id | value → what the screen said | other options |
|---|---|---|---|
| | | | |
| | | | |
| | | | |
| | | | |
| | | | |
| | | | |
| | | | |

## EXP Pedals

| menu text (exact) | id | value → what the screen said | other options |
|---|---|---|---|
| | | | |
| | | | |
| | | | |
| | | | |
| | | | |
| | | | |

## Menu order

`MENU_ORDER` wants the order the pedal lists these in, which is not the id order. Just number the
rows above top-to-bottom as they appear on screen, or list the ids here in menu order:

```
Footswitches:
EXP Pedals:
```

## Two things worth checking while you are in there

**Do `95`/`96`/`68`/`69` really live under Preferences?** They are `EXP/FS Tip`, `EXP/FS Ring`,
`Tip Polarity` and `Ring Polarity` — names that sound like they belong in the two sections above.
Confirmed under Preferences on 2026-08-24, so this is a re-check rather than an open question, but
these four are the likeliest place for that answer to have been wrong.

```
answer:
```

**Does anything in these sections refuse on a Stomp?** An id the XL has and the Stomp does not simply
refuses, and `scan_settings` treats a refusal as absence rather than an error. If a Footswitches item
is XL-only that is a real result — mark it `[XL]` when it lands in the table.

```
answer:
```

## When the answers land

Add each to `SETTINGS` **at its numeric position** — the table is id-ordered and searched by number;
menu order lives in `MENU_ORDER`. Pick `Kind::Flag` vs `Kind::Choice` by what the *wire* holds, which
the panel shows directly for an unidentified id: `false`/`true` is a flag, `0`/`1`/`2` is a choice.
A value you saw but whose menu text you did not record goes in as `Kind::Choice(&[])` — an empty
option list is legal and is how "observed, never explained" is written down. Do not fill it in from
memory; that is what this file exists to prevent.
