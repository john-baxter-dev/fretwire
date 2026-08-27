# The paired-cab / block-class sweep (2026-08-26, HX Stomp, fw 3.71)

Nine reassembled preset streams, each the edit buffer after `fretwire swap 1 <model> [paired]` on
the same base preset — the device's own answer to "what does this combination store". No capture
rig involved: our own verified swap op did the writing and `dump-raw` the reading, edit buffer
only, discarded afterwards (the slot re-read byte-identical).

The sweep settled the two questions blocking `.hxb`/`.hlx` amp+cab import (`fretwire_data::tone`):

1. **There is no cab-family mapping.** Legacy `HD2_Cab…` and new `HD2_CabMicIr_…` are both
   ordinary `Helix.sym` entries; a paired cab stores whichever family the preset uses. The
   supposed tone↔wire family mismatch was two observations from different presets.
2. **The class byte (content key `9`) is per-model, not per-`@type`** — and these files are its
   measurement. See `block_class` in `tone.rs` for the full table.

| file | swap | classes/counts it pins |
|---|---|---|
| `pairing_amp_legacy_cab` | `swap 1 41 49` | class **18**; bank 12 = 5 sym params + `@mic` appended, counts `2`=6 `3`=5 |
| `pairing_amp_micir_cab` | `swap 1 41 709` | class **33**; bank 12 = 7 of 8 sym params (`IrData` dropped), counts 7/7 |
| `pairing_dual_legacy_cab` | `swap 1 49 50` | class **16**; both banks in the legacy layout |
| `pairing_dual_micir_cab` | `swap 1 733 726` | class **32**; `WithPan` symbols, 9 of 10 stored |
| `class_legacy_cab_standalone` | `swap 1 49` | class **15**; bank 11 = 6 values, counts 6/5 |
| `class_micir_cab_standalone` | `swap 1 709` | class **31**; bank 11 = 7 values, counts 7/7 |
| `class_ir_mono` | `swap 1 149` | class **19**, counts 5/5; content key **27** = the referenced IR's UUID, NUL-terminated, in place of bank 12 |
| `class_ir_dual` | `swap 1 708` | class **21**, counts 14/14; key 27 = the two slot UUIDs concatenated |
| `class_ir_unreferenced` | `swap 1 149` + Index → empty slot | key 27 = `"\0"` — what a tone with no `@uuid` maps to |
| `class_synth` | `swap 1 377` | class **23**, counts 20/20 |

Also measured, not kept as files: amp alone = 17, preamp (`swap 1 186`) = **17**. The looper's
class **22** and shape came from the Sultans Floor stream instead — a device-written looper the
same unit's `.hxb` also holds as tone.

Model indices: 41 `HD2_AmpUSDoubleNrm`, 49 `HD2_Cab1x12USDeluxe`, 50 `HD2_Cab1x15TucknGo`,
149 `HD2_ImpulseResponse1024Mono`, 377 `HD2_Synth3NoteGeneratorMono`,
708 `HD2_ImpulseResponse1024DualStereo`, 709 `HD2_CabMicIr_1x12USDeluxe`,
733 `HD2_CabMicIr_4x12CaliV30WithPan`, 726 `HD2_CabMicIr_2x12JazzRivetWithPan`.

Every swapped block carries the model's factory defaults — the device reset them on swap, which
is what makes these usable as encoding oracles: the expected values are printable from the
reference data alone. `tests/paired_blocks.rs` replays the pairings from hand-written tone JSON
and compares against slot 1 of these files byte-for-byte.
