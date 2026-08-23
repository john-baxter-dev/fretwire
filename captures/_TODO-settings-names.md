# Settings names — every label in the table needs reading off a menu

**Goal:** confirm, at the pedal, that each of the 27 names in `fretwire_protocol::settings` is what
the pedal's Global Settings screens actually say — and capture the menu order for the five groups
`MENU_ORDER` doesn't cover yet. One walk through the menus does all of it.

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

### Preferences (3)

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `73` | Snapshot edits | — | | |
| `81` | Bypass type | — | | |
| `127` | Auto In-Z | [XL + owner] | | |

<details><summary>value labels for Preferences</summary>

- `73` Snapshot edits: choice — `0` Recall, `1` Discard
- `81` Bypass type: flag — true `DSP bypass`, false `Analog bypass`
- `127` Auto In-Z: choice — **values unnamed**

</details>

### MIDI (2)

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `9` | MIDI base channel | — | | |
| `11` | MIDI over USB | — | | |

<details><summary>value labels for MIDI</summary>

- `9` MIDI base channel: choice — `0`..`15`, shown one-based
- `11` MIDI over USB: flag — true `On`, false `Off`

</details>

### Tempo (2)

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `14` | Tempo select | — | | |
| `16` | Tempo | — | | |

<details><summary>value labels for Tempo</summary>

- `14` Tempo select: choice — `0` Per snapshot, `1` Per preset, `2` Global
- `16` Tempo: number — BPM

</details>

### Displays (1)

| id | fretwire says | read off | what the pedal shows | actual section |
|---|---|---|---|---|
| `27` | Preset numbering | — | | |

<details><summary>value labels for Displays</summary>

- `27` Preset numbering: flag — true `000-127`, false `01A-32D`

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

**`127` — which value is `First`?** Set Auto In-Z to `First`, `setting-get 127`, switch it to
`Enabled`, read it again. Two integers and `Kind::Choice(&[])` can finally be filled in.

```
0 =
1 =
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

**Are `MIDI` and `Tempo` separate pages?** `GROUPS` claims six sections — Global EQ, Ins/Outs,
Tempo, MIDI, Preferences, Displays — and those headings were never checked against the pedal either.

```
answer:
```

**Anything in the menus with no row here?** A setting on the screen that never appears in the panel
is an unmapped id, and a dump-change-dump on the spot names it. 138 of the 166 answering ids are
still unidentified, so this is the likeliest place to find several at once.

```
answer:
```

## Menu order, section by section

`MENU_ORDER` currently covers **Ins/Outs only**, from the XL. One entry per line in screen order,
**including entries fretwire has no id for** — a gap in the list is itself a finding, and it is what
tells us an id is missing rather than merely unnamed.

Ins/Outs is prefilled with the XL's order as a candidate: correct it if a Stomp differs.

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
(127 Auto In-Z is the tenth item on an XL; 73 and 81 are unplaced)
```

```
MIDI
```

```
Tempo
```

```
Displays
```

```
Global EQ
```

## What happens with the answers

- Wording and section corrections go into `SETTINGS` — and if the casing turns out consistent on the
  pedal, the table's split between `Input Level` and `Snapshot edits` gets normalised in the same
  pass.
- Each completed section becomes ids appended to `MENU_ORDER`, which is all the panel needs to match
  the pedal group by group. `menu_rank` already puts anything unplaced last, so a partial answer is
  useful on its own.
- `127`'s two integers turn `Kind::Choice(&[])` into a named pair, and `a_choice_may_be_empty` needs
  a different id to point at — `docs/protocol.md` notes that empty is deliberate and must stay legal.
- Anything found on screen with no id is a new sweep target.

Delete this file once it's answered, the way the other `_TODO-` sheets go.
