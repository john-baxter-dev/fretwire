# Settings names — every label in the table needs reading off a menu

**Goal:** confirm, at the pedal, that each of the 34 names in `fretwire_protocol::settings` is what
the pedal's Global Settings screens actually say. One walk through the menus does all of it.

> **Updated 2026-08-24.** Three of the four open questions at the bottom are answered, the section
> headings have been read off the hardware, and the tables below now match the table as it stands.
> Finding *new* ids is a different loop and lives in **`_TODO-settings-discovery.md`** — that one is
> the higher-value use of pedal time now, since two whole sections have no entries at all.

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
- `154` Return Type: flag — true `Aux In`, false `Return`
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

### MIDI/Tempo (4)

**One section on the pedal, not two** — confirmed on a Stomp and an XL [2026-08-24]. These were
separate groups here until then.

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `9` | MIDI base channel | — | | |
| `11` | MIDI over USB | — | | |
| `14` | Tempo select | — | | |
| `16` | Tempo | — | | |

<details><summary>value labels for MIDI/Tempo</summary>

- `9` MIDI base channel: choice — `0`..`15`, shown one-based
- `11` MIDI over USB: flag — true `On`, false `Off`

- `14` Tempo select: choice — `0` Per snapshot, `1` Per preset, `2` Global
- `16` Tempo: number — BPM

</details>

### Displays (0)

**Empty.** Id `27` was its only member and moved to Preferences on 2026-08-24, where the pedal
actually keeps it. The section exists on the hardware, so whatever it holds is unidentified — see
`_TODO-settings-discovery.md`, which covers the other two empty sections the same way.

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

```
cargo run -p fretwire-cli -- setting-get 2     # Send/Return L
cargo run -p fretwire-cli -- setting-get 3     # Send/Return R
cargo run -p fretwire-cli -- setting-get 31    # Input Level
cargo run -p fretwire-cli -- setting-get 94    # Output Level
cargo run -p fretwire-cli -- setting-get 154   # Return Type
```

```
2   =
3   =
31  =
94  =
154 =
```

**Writes are safe either way** — `set_setting_num` reads the current value and matches the type
against what the device actually holds, so it never consults `Kind` and a wrong one cannot cause a
refusal. And the panel's flag control tests truthiness, so an int `0`/`1` renders correctly too.

**The failure that matters is a third state.** A setting with *three* options declared as a `Flag`
offers two in the dropdown and renders the third as if it were the second. `154` Return Type is the
one to watch — cycle it on the pedal and count the options:

```
154 option count:
```

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
Footswitches            ← no ids at all; see _TODO-settings-discovery.md
```

```
EXP Pedals              ← no ids at all; see _TODO-settings-discovery.md
```

```
MIDI/Tempo
(9 MIDI base channel, 11 MIDI over USB, 14 Tempo select, 16 Tempo — order unread)
```

```
Displays                ← no ids left; 27 moved to Preferences
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

Delete this file once it's answered, the way the other `_TODO-` sheets go.
