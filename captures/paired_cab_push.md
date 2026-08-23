# Capture: paired-cab status push (key 26)

- **pcap file:** none — taken with `fretwire watch --secs 75`, which decodes the pushes directly.
- **date:** 2026-08-23
- **context:** HX Stomp, fw 3.80, preset `[19] PrincesSM7`. Slot 5 is `HD2_AmpUSPrincess` with
  `1x12 US Deluxe` paired. Knobs turned on the **pedal itself**, not in the editor.

## Action performed
- **What:** swept the cab's **Distance** knob, then the amp's **Mid** knob.
- **Block / slot:** slot 5, one amp+cab block — so both knobs live in the same slot.
- **Parameters:** cab `Distance` (paired index 2) and amp `Mid` (main index 2). Deliberately the
  pair that collides: they share a slot *and* an index.

## What came back

153 `Param` pushes, every one of them `param: 2` — 123 from the cab, 30 from the amp:

```
[16.81s] Param { slot: 5, param: 2, value: Float(2.25),       extra: false, paired: true  }   cab Distance
[56.46s] Param { slot: 5, param: 2, value: Float(0.66999996), extra: false, paired: false }   amp Mid
```

## Decoded meaning

**Key 26 is the only thing that separates them.** Same slot, same index, same `extra` flag; the
sub-model selector is the whole difference — `1` the paired cab, `0` the block's own model, exactly
as it is on the edit side (`edit::MODEL_PAIRED` / `MODEL_MAIN`).

The value ranges corroborate which is which: `Distance` is in inches over 1..12, `Mid` is normalized
0..1 (0.67 here, which `pull` renders as the 6.8 on the pedal's screen).

This is the capture that was missing when key 26 was first decoded — the decode was inferred from
the edit map's shape, since no capture we held moved a cab parameter from the pedal's own panel.
It is now measured. A decoder that drops key 26 delivers this cab sweep to the amp's Mid, which is
precisely how the bug was reported. [solid — issue #11]
