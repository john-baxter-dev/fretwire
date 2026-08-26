# Settings names — every label in the table needs reading off a menu

**Goal:** confirm, at the pedal, that each of the 53 names in `fretwire_protocol::settings` is what
the pedal's Global Settings screens actually say. One walk through the menus does all of it.

> **Updated 2026-08-25.** `_TODO-settings-discovery.md` is retired: an XL owner walked Footswitches,
> EXP Pedals, MIDI/Tempo and Displays with it and came back with **19 new ids** (PR #16), which is
> everything that sheet asked for bar the two questions folded in at the bottom of this one. The
> table went 34 → 53 and this file is the live one again.
>
> Nineteen of those 53 names have now been read off an **XL** and none off a Stomp, which is the
> asymmetry this sheet exists to close — see the `[XL]` marks below.

## Why this exists

Id `127` was called **Guitar In-Z** from 2026-08-22 to 2026-08-23 and the pedal has never shown that
name. `git log -S "guitar In-Z"` puts the whole history of the string in one commit — `07d20ad`,
where it appears already formed in `STATUS.md`, the CLI gloss and `docs/protocol.md` at once.
Nothing was mis-transcribed: a Helix setting that sounds like the real one got written down in place
of **Auto In-Z**, and the `Ins/Outs` group followed the invented name, because Ins/Outs is where a
Helix keeps *Guitar In-Z*.

The entry had carefully refused to name its two values — `Kind::Choice(&[])`, with a comment about
not labelling from memory — while carrying a name nobody had read off a menu. The rule this table
states about values was never applied to the `name` field. So:

> **Every name here that nobody has quoted off a menu is a guess.** Two of the four checked so far
> were wrong (`156` was "volume knob assignment"; `31`/`94` had the wrong casing), and both were
> caught by pedal owners, not by review.

An owner of a *different* pedal found the `127` error. That is not a review process, and this file
is the cheap fix for it.

## How to fill this in

Walk Global Settings top to bottom with this open. For each row:

- **exact match**, casing and spacing included → put `=` in *what the pedal shows*
- **anything else** → type the screen's exact wording
- **not on this pedal** → put `absent`
- **didn't get to it** → leave blank

Fill *actual section* only where it isn't the group already claimed on the left of the table.

Nothing here needs the pedal connected over USB. The three open questions at the bottom do:

```
cargo run -p fretwire-cli -- setting-get 127

cargo run -p fretwire-cli -- settings-dump before.txt
# change exactly one thing on the pedal
cargo run -p fretwire-cli -- settings-dump after.txt
cargo run -p fretwire-cli -- settings-diff before.txt after.txt
```

**read off** in the tables says where the current wording came from: `[XL]` is an id an HX Stomp XL
owner contributed and no Stomp has confirmed, `[XL name]` is an id both pedals have where the XL's
menu supplied the wording, `[XL + owner]` is `127`, corrected on 2026-08-23 from both directions at
once, and `—` is a name nobody has quoted from anything.

## Names to verify

### Ins/Outs (8)

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `2` | Send/Return L | [XL] | | |
| `3` | Send/Return R | [XL] | | |
| `31` | Input Level | [XL name] | | |
| `94` | Output Level | [XL name] | | |
| `153` | USB In 1/2 Trim | [XL] | | |
| `154` | Return Type | [XL] | | |
| `156` | Volume Controls | [XL name] | | |
| `158` | Phones Monitor | [XL] | | |

<details><summary>value labels for Ins/Outs</summary>

- `2` Send/Return L: flag — true `Line`, false `Instrument`
- `3` Send/Return R: flag — true `Line`, false `Instrument`
- `31` Input Level: flag — true `Line`, false `Instrument`
- `94` Output Level: flag — true `Line`, false `Instrument`
- `153` USB In 1/2 Trim: number — dB
- `154` Return Type: choice — `0` Return, `1` Aux In
- `156` Volume Controls: choice — `1` Phones, `2` Main+HP
- `158` Phones Monitor: choice — `1` Main L/R, `2` Send

</details>

### Preferences (11)

Section membership and order confirmed on a Stomp [2026-08-24]; the wording below is the XL's except
where noted, so the names still want a Stomp's screen against them.

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `81` | Bypass Type | [XL name] | | |
| `73` | Snapshot Edits | [XL name] | | |
| `65` | Tempo Pitch | [XL] | | |
| `95` | EXP/FS Tip | [XL] | | |
| `96` | EXP/FS Ring | [XL] | | |
| `68` | Tip Polarity | [XL] | | |
| `69` | Ring Polarity | [XL] | | |
| `27` | Preset Number | [XL name] | | |
| `103` | Snapshot Reselect | [XL] | | |
| `127` | Auto In-Z | **done** | Auto In-Z | Preferences |
| `136` | Link Dual Cabs | [XL] | | |

<details><summary>value labels for Preferences</summary>

- `81` Bypass Type: flag — true `DSP`, false `Analog`
- `73` Snapshot Edits: choice — `0` Recall, `1` Discard
- `65` Tempo Pitch: flag — true `Transpr`, false `Authentc` [sic — screen-width truncation]
- `95` EXP/FS Tip: flag — true `FS7`, false `EXP 1`
- `96` EXP/FS Ring: flag — true `FS8`, false `EXP 2`
- `68` Tip Polarity: choice — `0` Normal, `1` Inverted
- `69` Ring Polarity: choice — `0` Normal, `1` Inverted
- `27` Preset Number: flag — **device-dependent**, derived per unit from the preset count. A Stomp
  draws `000-125`/`01A-42C`, an XL `000-127`/`01A-32D`. Both Stomp forms read off the pedal
  [2026-08-24]. See `Device::preset_numbering_labels`.
- `103` Snapshot Reselect: choice — `0` Reload, `1` Toggle
- `127` Auto In-Z: choice — `0` First, `1` Enabled ✅ **read off a Stomp, 2026-08-24**
- `136` Link Dual Cabs: choice — `0` Off, `1` On

</details>

### Footswitches (9)

**All nine off an XL, 2026-08-25.** Two of them — `130` and `131` — name switches a three-switch
Stomp does not have, so the interesting answer here is not the wording but whether they answer at
all: an id the Stomp lacks refuses, and `scan_settings` reads a refusal as absence.

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `17` | Stomp Select | [XL] | | |
| `19` | Stomp Mode | [XL] | | |
| `18` | Preset Mode | [XL] | | |
| `67` | Snapsht Mode | [XL] | | |
| `20` | Up/Down Switches | [XL] | | |
| `117` | Swap Up/Down | [XL] | | |
| `129` | TAP Function | [XL] | | |
| `130` | FS7 Function | [XL] | | |
| `131` | FS8 Function | [XL] | | |

<details><summary>value labels for Footswitches</summary>

- `17` Stomp Select: choice — `0` Off, `1` Touch, `2` Press, `3` Both
- `19` Stomp Mode: choice — `0` 4 Swtch, `1` 6 Swtch
- `18` Preset Mode: choice — `0` Moment, `1` Latch
- `67` Snapsht Mode: choice — `0` Moment, `1` Latch, `2` Toggle
- `20` Up/Down Switches: choice — `0` Banks, `1` Preset, `2` Snapsht
- `117` Swap Up/Down: choice — `0` Off, `1` On
- `129` TAP Function: flag — true `AllBypas`, false `TAP/Tunr`. **The one to cycle**: `130`/`131`
  offer ten functions each and two of them are exactly these two strings, so if TAP has more than
  two settings the flag can't hold them.
- `130` FS7 Function / `131` FS8 Function: choice — `0` TAP/Tunr, `1` Stomp 7 / `Stomp 8`,
  `2` Bank Up, `3` Bank Dn, `4` PresetUp, `5` PresetDn, `6` SnpshtUp, `7` SnpshtDn, `8` AllBypas,
  `9` TogglEXP

</details>

### EXP Pedals (2)

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `66` | EXP 1 Position | [XL] | | |
| `71` | EXP 2 Position | [XL] | | |

<details><summary>value labels for EXP Pedals</summary>

- `66` EXP 1 Position / `71` EXP 2 Position: choice — `0` Snapsht, `1` Preset, `2` Global

</details>

### MIDI/Tempo (9)

**One section on the pedal, not two** — confirmed on a Stomp and an XL [2026-08-24]. These were
separate groups here until then.

The four original names were re-read off an XL on 2026-08-25 and **all four were wrong** — casing on
`9` and `14`, a different name on the other two (`11` is `USB MIDI`, `16` is `BPM`). `14`'s three
value labels changed with it. They are corrected below; nobody has re-checked any of this on a
Stomp, so where the two units disagree it is the XL's wording that is in the table.

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `9` | MIDI Base Channel | [XL] | | |
| `10` | MIDI Thru | [XL] | | |
| `13` | Rx MIDI Clock | [XL] | | |
| `76` | Tx MIDI Clock | [XL] | | |
| `14` | Tempo Select | [XL] | | |
| `16` | BPM | [XL] | | |
| `11` | USB MIDI | [XL] | | |
| `12` | MIDI PC Rx | [XL] | | |
| `77` | MIDI PC Tx | [XL] | | |
| `135` | Snapshot CC Send | [XL] | | |

<details><summary>value labels for MIDI/Tempo</summary>

- `9` MIDI Base Channel: choice — `0`..`15`, shown one-based
- `10` MIDI Thru: flag — true `On`, false `Off`
- `13` Rx MIDI Clock: choice — `0` Off, `1` MIDI, `2` USB, `3` Auto
- `76` Tx MIDI Clock: choice — `0` Off, `1` MIDI, `2` USB, `3` MIDI+USB
- `14` Tempo Select: choice — `0` Snapsht, `1` Preset, `2` Global
- `16` BPM: number — BPM
- `11` USB MIDI: flag — true `On`, false `Off`
- `12` MIDI PC Rx / `77` MIDI PC Tx: choice — `0` Off, `1` MIDI, `2` USB, `3` MIDI+USB
- `135` Snapshot CC Send: choice — `0` Off, `1` On

</details>

> **`135` has no place in `MENU_ORDER`.** Every other id in this section was placed; this one came
> without its row number, so it draws at the foot of MIDI/Tempo rather than where the pedal puts it.
> One line of the menu, and it is fixed — `only_the_listed_ids_are_unplaced` names it.

### Displays (2)

Id `27` was its only member until 2026-08-24, and moved to Preferences where the pedal actually
keeps it, leaving the section empty. The two ids below arrived from an XL on 2026-08-25.

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `25` | LED Rings | [XL] | | |
| `26` | Tap LED | [XL] | | |

<details><summary>value labels for Displays</summary>

- `25` LED Rings: flag — true `Dim/Brt`, false `Off/Brt`
- `26` Tap LED: flag — true `On`, false `Off`

</details>

### Global EQ (11)

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `190` | EQ low frequency | — | | |
| `191` | EQ low Q | — | | |
| `192` | EQ low gain | — | | |
| `193` | EQ mid frequency | — | | |
| `194` | EQ mid Q | — | | |
| `195` | EQ mid gain | — | | |
| `196` | EQ high frequency | — | | |
| `197` | EQ high Q | — | | |
| `198` | EQ high gain | — | | |
| `199` | EQ low cut | — | | |
| `200` | EQ high cut | — | | |

<details><summary>value labels for Global EQ</summary>

- `190` EQ low frequency: number — Hz
- `191` EQ low Q: number
- `192` EQ low gain: number — dB
- `193` EQ mid frequency: number — Hz
- `194` EQ mid Q: number
- `195` EQ mid gain: number — dB
- `196` EQ high frequency: number — Hz
- `197` EQ high Q: number
- `198` EQ high gain: number — dB
- `199` EQ low cut: number — Hz, `19.9` is off
- `200` EQ high cut: number — Hz, `20100` is off

</details>

## Open questions

**`127` — which value is `First`?** ✅ **Answered 2026-08-24, on a Stomp.**

```
0 = First
1 = Enabled
```

**Are the `Kind::Flag` entries really bools on the wire?** Raised by the #11 contributor, who notes
he may have copy-pasted the `Kind` from a neighbouring entry rather than reading it. `setting-get`
prints what the device holds, and the two are distinguishable: a bool prints as `true`/`false`, an
int as `0`/`1`.

**One of the five is answered — `154` is an int** [XL, 2026-08-25, PR #14]: `setting-get 154`
printed `154 = 1  [int]`, so `Return Type` is now `Kind::Choice`. The mapping it carried as a flag
was never in doubt — the XL owner reached it by diffing dumps, and a dump prints the value, so
`0` Return / `1` Aux In is what was seen either way. Only the *type* was transcribed wrong.

**The other four are still open**, and the same slip is as likely in each:

```
cargo run -p fretwire-cli -- setting-get 2     # Send/Return L
cargo run -p fretwire-cli -- setting-get 3     # Send/Return R
cargo run -p fretwire-cli -- setting-get 31    # Input Level
cargo run -p fretwire-cli -- setting-get 94    # Output Level
```

```
2   =
3   =
31  =
94  =
```

**Writes are safe either way** — `set_setting_num` reads the current value and matches the type
against what the device actually holds, so it never consults `Kind` and a wrong one cannot cause a
refusal. And the panel's flag control tests truthiness, so an int `0`/`1` renders correctly too.

**The failure that matters is a third state.** A setting with *three* options declared as a `Flag`
offers two in the dropdown and renders the third as if it were the second. Moving `154` to `Choice`
removes that failure for it specifically — an unlisted value displays as the bare number instead of
being clamped to a neighbour — but nobody has counted the options yet, so whether a third exists is
still unknown. Cycle it on the pedal and count:

```
154 option count:
```

The same question rides on the four above: a `Flag` that turns out to be an int is only cosmetic
until it turns out to have three states.

**`3` — does Send/Return R answer on a plain Stomp?** `setting-get 3`. A refusal is a real result,
not a failure: it would make the id XL-only, and the row simply won't appear on a Stomp.

```
answer:
```

**`153` — int or float on the wire?** `USB In 1/2 Trim` is stored as `Kind::Number` with the type
unrecorded. `setting-get 153` prints what the device holds.

```
answer:
```

**Are `MIDI` and `Tempo` separate pages?** ✅ **Answered 2026-08-24 — no, one section.** And the
section list `GROUPS` claimed was wrong in three ways. The pedal has six, in this order, confirmed on
a Stomp and an XL independently:

```
Ins/Outs · Preferences · Footswitches · EXP Pedals · MIDI/Tempo · Displays
```

`Footswitches` and `EXP Pedals` did not exist here at all, and `Global EQ` is **not** one of these
sections — it is a separate top-level menu on the pedal.

**Anything in the menus with no row here?** A setting on the screen that never appears in the panel
is an unmapped id, and a dump-change-dump on the spot names it. 138 of the 166 answering ids are
still unidentified, so this is the likeliest place to find several at once.

```
answer:
```

## Menu order, section by section

`MENU_ORDER` now covers **Ins/Outs and Preferences**, both from the XL. One entry per line in screen
order, **including entries fretwire has no id for** — a gap in the list is itself a finding, and it
is what tells us an id is missing rather than merely unnamed.

Both are prefilled with the XL's order as a candidate: correct them if a Stomp differs.

```
Ins/Outs
1. Input Level          (31)
2. Output Level         (94)
3. Send/Return L        (2)
4. Send/Return R        (3)
5. Return Type          (154)
6. USB In 1/2 Trim      (153)
7. Phones Monitor       (158)
8. Volume Controls      (156)
```

```
Preferences
 1. Bypass Type         (81)
 2. Snapshot Edits      (73)
 3. Tempo Pitch         (65)
 4. EXP/FS Tip          (95)
 5. EXP/FS Ring         (96)
 6. Tip Polarity        (68)
 7. Ring Polarity       (69)
 8. Preset Number       (27)
 9. Snapshot Reselect   (103)
10. Auto In-Z           (127)
11. Link Dual Cabs      (136)
```

```
Footswitches            ← read off an XL, 2026-08-25
1. Stomp Select         (17)
2. Stomp Mode           (19)
3. Preset Mode          (18)
4. Snapsht Mode         (67)
5. Up/Down Switches     (20)
6. Swap Up/Down         (117)
7. TAP Function         (129)
8. FS7 Function         (130)
9. FS8 Function         (131)
```

```
EXP Pedals              ← read off an XL, 2026-08-25
1. EXP 1 Position       (66)
2. EXP 2 Position       (71)
```

```
MIDI/Tempo              ← read off an XL, 2026-08-25
1. MIDI Base Channel    (9)
2. MIDI Thru            (10)
3. Rx MIDI Clock        (13)
4. Tx MIDI Clock        (76)
5. Tempo Select         (14)
6. BPM                  (16)
7. USB MIDI             (11)
8. MIDI PC Rx           (12)
9. MIDI PC Tx           (77)
?. Snapshot CC Send     (135)   ← position unread; unplaced in MENU_ORDER
```

```
Displays                ← read off an XL, 2026-08-25
1. LED Rings            (25)
2. Tap LED              (26)
```

Global EQ is a **separate top-level menu**, not a Global Settings section, so it has no place in this
list — its eleven ids sort ahead of everything by number and the panel gives them their own tab.

## What happens with the answers

- Wording and section corrections go into `SETTINGS` — and if the casing turns out consistent on the
  pedal, the table's split between `Input Level` and `Snapshot edits` gets normalised in the same
  pass.
- Each completed section becomes ids appended to `MENU_ORDER`, which is all the panel needs to match
  the pedal group by group. `menu_rank` already puts anything unplaced last, so a partial answer is
  useful on its own.
- ~~`127`'s two integers~~ ✅ done. The test that guarded the empty-choice case no longer points at
  an id at all: it states the invariant about the *shape*, so the next unexplained setting inherits
  it without anyone remembering to re-anchor it.
- Anything found on screen with no id is a new sweep target.

## Inherited from `_TODO-settings-discovery.md`

That sheet was retired on 2026-08-25 when PR #16 answered it. Three things it asked for came back
unanswered, and they are pedal-side questions, so they live here now.

**Does anything in these sections refuse on a Stomp?** Nineteen ids exist here entirely on an XL's
word. `130`/`131` are `FS7 Function`/`FS8 Function` and a Stomp has three switches, so at least
those two ought to refuse — but "ought to" is not an observation, and a refusal is a real result:
`scan_settings` reads it as absence rather than as an error, which is what lets one table serve both
pedals. `settings-dump` on a Stomp answers all nineteen at once.

```
answer:
```

**Do `95`/`96`/`68`/`69` really live under Preferences?** `EXP/FS Tip`, `EXP/FS Ring`, `Tip Polarity`
and `Ring Polarity` — names that sound like they belong under EXP Pedals, which now exists and holds
only two ids. Confirmed under Preferences on 2026-08-24, so this is a re-check rather than an open
question, and it is the likeliest place for that confirmation to have been wrong.

```
answer:
```

**Where does `135` sit in the MIDI/Tempo menu?** One row number, and `MENU_ORDER` is complete.

```
answer:
```

Delete this file once it's answered, the way the other `_TODO-` sheets go.
