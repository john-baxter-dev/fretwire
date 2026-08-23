# Model icons

The GUI draws a small filled silhouette for every block: the pedal, amp head, cabinet or rack unit
that the model is modelled on. It is the fastest way to read a chain — a gold three-knob box, a big
silver fuzz, a green four-switch delay, a 4x12 — without reading eight labels.

Nothing here is Line 6's artwork. `HX_ModelCatalog.json` names a PNG per model
(`FX_HX_DIST_KinkyBoost.png`); those files are proprietary and are never read, copied or shipped.
Every icon is drawn from scratch as SVG at runtime, and encodes only silhouette, finish and control
layout — no logos, no lettering, no panel graphics.

## Where it lives

| File | Role |
| --- | --- |
| `ui/src/lib/icons/palette.js` | enclosure / panel / grille-cloth colours, named by finish |
| `ui/src/lib/icons/render.js` | draws a spec as SVG on a 32×32 grid |
| `ui/src/lib/icons/models.js` | the per-model table, the amp rules, the cab finishes |
| `ui/src/lib/icons/spec.js` | resolves a model to a spec; category icons |
| `ui/src/lib/icons/ModelIcon.svelte` | the component the chain and the picker use |
| `ui/src/lib/icons/mics.js` | microphone silhouettes, for the cab mic view |
| `ui/src/lib/CabMicView.svelte` | the cab's mic placement, drawn |

## How a model resolves

`iconSpec(symbolicId, category, name)` tries four things, most specific first:

1. **The curated table** (`MODELS` in `models.js`) — keyed by the `Helix.sym` base symbol. This is
   where "Scream 808 is a green three-knob overdrive" is recorded.
2. **Amps and cabs.** Amps match on the symbolic-id prefix (`HD2_AmpBritPlexi…`), longest first, so
   the `Nrm`/`Brt`/`Jump` variants share one entry; preamps are normalised to the amp symbol and
   drawn as a rack unit. Cabs are *derived*: the display name starts with the driver array
   (`4x12 Greenback 25`), which becomes the speaker grid, and a regex on the rest picks the finish.
3. **The effect family** — a keyword on the symbolic id. Any chorus that isn't in the table still
   gets a chorus icon, so new firmware models are never blank.
4. **The category icon.**

## Adding or correcting one

One line in `MODELS`:

```js
HD2_DistMinotaur: box(C.gold, 3, { led: "#ffd166" }),
```

The constructors are `box` / `wide` / `mini` (stompboxes), `rack`, `wah`, `reel`, `util`; the
shapes `render.js` knows are `stomp`, `stompWide`, `stompNarrow`, `round`, `wedge`, `wah`, `head`,
`combo`, `cab`, `rack`, `reel`, `mic`, `util`, `pedalboard`, `looper`, `rotary`, `spring`. Useful
spec fields: `knobs`, `layout` (`row` / `arc` / `grid`), `sw` (footswitches), `plate` (a contrasting
faceplate), `mark` (`window` / `stripe` / `bars`), `glyph` (a rack's lit display: `arch`, `room`,
`cave`, `plate`, `shimmer`, `gate`, `echo`, `particle`, `wave`), `led`, `speakers`, `mic`, `stack`.

## The cab mic view

A cab's parameters get a picture beside them: the microphone, how far off the grille it stands, where
across the cone it sits, and how far it is tilted. `ParamPanel` draws it for any param list carrying
both a `Mic` and a `Distance`, which is both shapes the device has — the paired cab on an amp+cab
block, and a standalone Cab block — without needing to know the category.

It sits in a flex row with the param grid rather than in a band above it: the drawing is wider than
it is tall and the grid is not, so stacking them left a strip of dead panel across the middle. The
grid takes the width the drawing doesn't, down to about 600px of panel, below which the drawing
wraps underneath and centres.

It is a *view* of those parameters, not a second source of truth: it reads the same array the grid
below edits and writes through the same callbacks, so the device's re-read still settles the value.
Dragging the mic sets Distance (horizontally) and Position (vertically), arrow keys do the same one
step at a time, and Angle keeps its segmented buttons in the grid — the drawing shows the tilt as an
arc at the capsule.

Same rule as the model icons: no artwork is read or shipped. The reference data names a mic only as a
string (`"57 Dynamic"`, `"4038 Ribbon"` — the `mic` and `cabMICir` discrete controls in
`HelixControls.json`), and `mics.js` turns the number in that string into proportions and a finish.
Unlisted mics fall back to their family — Dynamic, Ribbon or Condenser — so a firmware update adding
one still draws something.

Two things in the drawing are conventions rather than data, and are worth knowing before trusting
them:

- **The radial scale is linear.** Position runs 0..1 (displayed 0..10, `Center`..`Edge`) and the view
  maps it straight onto the cone's radius. Whether the device's own mapping is linear is not
  something the reference data says.
- **"Cap edge" is drawn at Position's own default** (0.23 on the stock cabs), because that default is
  the classic just-off-the-dust-cap placement. That ties the tick to the data instead of to a radius
  the drawing invented, but it is an inference about what the default *means*, not a measurement of
  the modelled speaker.

The legacy cab family (category 2) has no Position or Angle at all — only Mic and Distance — so those
parts of the drawing are simply omitted for it.

## Reviewing the set

`render.js` is plain JS with no Svelte or DOM dependency, so a contact sheet of every icon can be
rendered outside the app — import `iconBody`, lay the specs out on a grid, and rasterise the SVG.
That is how the set is checked after a change.

## Known guesses — correct these as you come across them

Most icons key off hardware whose look is well known, so the finish and control layout are right.
These ones are placeholders: either the original wasn't identified, or the pedal is known but its
finish isn't. **None of them is wrong in a way that breaks anything** — each is a plausible box in
the right category — so they're worth fixing opportunistically rather than in one pass. Correcting
one is a single line in `MODELS`; delete its row here when you do.

| Model | Symbol | Placeholder | Note |
| --- | --- | --- | --- |
| Prize Drive | `HD2_DistPrizeDrive` | teal box, 3 knobs | original not identified |
| Pillars | `HD2_DistPillars` | blue box, 4 knobs | original not identified |
| KWB | `HD2_DistKWB` | charcoal box, 3 knobs | original not identified |
| Legendary Drive | `HD2_DistLegendaryDrive` | oxblood box, 3 knobs | original not identified |
| Ratatouille Dist | `HD2_DistRatatouilleDist` | charcoal box, 3 knobs | guessed a RAT relative (Vermin Dist is the RAT) |
| Vital Dist / Boost | `HD2_DistVital…` | crimson | original not identified |
| Dark Dove Fuzz | `HD2_DistDarkDoveFuzz` | big black box | original not identified |
| Ballistic Fuzz | `HD2_DistBallisticFuzz` | graphite box | original not identified |
| Wringer Fuzz | `HD2_DistWringerFuzz` | magenta box | original not identified |
| Thrifter Fuzz | `HD2_DistThrifterFuzz` | tan box | original not identified |
| Xenomorph Fuzz | `HD2_DistXenomorphFuzz` | green box, 4 knobs | original not identified |
| Clawthorn Drive | `HD2_DistClawthornDrive` | green box, 4 knobs | original not identified |
| Regal Bass DI | `HD2_DistRegalBassDI` | espresso wide box, 5 knobs | assumed a bass DI/preamp |
| Bronze Master | `Line6BronzeMaster` | tan box, 3 knobs | Line 6 original — finish invented |
| Killer Z | `KillerZ` | oxblood box, 4 knobs | Line 6 original — finish invented |
| Dhyana Drive | `HD2_DistDhyanaDrive` | indigo box, 4 knobs | Zendrive; colour unconfirmed |
| Deluxe Comp | `HD2_CompressorDeluxeComp` | blue box, 4 knobs | original not identified |
| Rochester Comp | `HD2_CompressorRochesterComp` | navy + silver plate | assumed Ampeg family |
| Pebble Phaser | `HD2_PhaserPebblePhaser` | steel box, 1 knob | assumed a Small Stone |
| Deluxe Phaser | `HD2_PhaserDeluxePhaser` | violet box, 4 knobs | original not identified |
| Dynamix Flanger | `HD2_FlangerDynamixFlanger` | blue wide box | original not identified |
| FlexoVibe | `VIC_FlexoVibe` | navy wide box | assumed a Uni-Vibe descendant |
| Mystery Filter | `HD2_FilterMysterFilter` | charcoal wide box | original not identified |
| Asheville Pattrn | `HD2_FilterAshevillePattrn` | indigo box, 5 knobs | original not identified |
| Weeper / Throaty / Conductor / Colorful / Vetta Wah | `HD2_Wah…` | treadle, assorted colours | shape is right, colours are guesses |
| Interstate Zed, Divided Duo, Cartographer, Mail Order Twin | `HD2_Amp…` | see the table | tolex + panel finish guessed |
| Del Sol 300, Busy One, Woody Blue | `HD2_Amp…` | bass heads | tolex + panel finish guessed |

Resolved so far: **Teemah!** — aqua enclosure, four knobs (2026-08-21).
