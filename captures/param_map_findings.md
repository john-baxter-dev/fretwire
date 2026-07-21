# Param-map experiment — results (2026-06-22)

Goal (from `docs/next-steps.md` Track 1): determine whether parameter editing **generalizes** — i.e.
whether a parameter's wire selector is computable, or needs a capture per param.

**Result: H2 (best case) — the selector is the parameter's INDEX in the model's `Helix.sym` device
order.** Editing is computable from shipped data; no per-parameter captures are needed.

## Captures (one parameter change each)
`bucket_brigade_mix_modify`, `70s_chorus_mix_modify_and_enable`, `dynamic_plate_mix_modify`,
`dynamic_ambience_mix_modify`, `dynamic_ambience_predelay_modify`, `dynamic_ambience_lowcut_modify`,
`dynamic_ambience_level_modify`, `dynamic_ambience_trails_on_off`, `dynamic_ambience_8m_10m_12m`.

Decode any edit body with: `cargo run -p fretwire-cli -- decode-edit <hex>` (body = the bytes after the
8-byte TLV header `01 00 06 00 <ilen u32>`; extract with `tools/dump-control.ps1`).

## Same param (Mix) across models → key 28 = Mix's index in each model
| model | slot | key 28 | Mix's index in `Helix.sym` order | match |
|-------|-----:|-------:|----------------------------------|:----:|
| Bucket Brigade (mono) | 2 | 3 | `Time,Feedback,Noise,Mix(3)…` | ✓ |
| 70s Chorus (mono)     | 3 | 4 | `ChorusIntensity,Mode,VibratoRate,VibratoDepth,Mix(4)…` | ✓ |
| Dynamic Plate         | 6 | 5 | (reverb, Mix at 5) | ✓ |
| Dynamic Ambience      | 7 | 5 | `RoomSize,PreDelay,Damping,Diffusion,EarlyLateBlend,Mix(5)…` | ✓ |

(Same param, different index per model — exactly because its position differs. Value always at key 119.)

## Different params, same block (Dynamic Ambience, slot 7) → key 28 tracks index
order: `RoomSize(0),PreDelay(1),Damping(2),Diffusion(3),EarlyLateBlend(4),Mix(5),LowCut(6),HighCut(7),Level(8)`
| param | key 28 | index | match |
|-------|-------:|------:|:----:|
| PreDelay | 1 | 1 | ✓ |
| Mix | 5 | 5 | ✓ |
| LowCut | 6 | 6 | ✓ |
| Level | 8 | 8 | ✓ |

## Envelope corrections (vs the earlier 2-sample guess)
- **Key 102** = a whole u16 running counter (Mix later shows `0x05xx`), not op-high-byte + counter.
- **Key 100** = the operation: 41 = bypass, 30 = set-value.
- **Value** = target key 119 (float32 knobs; int enums; bool switches).

## Edge case (not yet resolved)
`@trails` toggle and the `8m/10m/12m` tempo-sync both decode with **key 28 = 0** (not a real index)
and aren't in the block's main param list — switch/transport params use a different addressing. All
continuous/knob params follow the index rule.

## Consequence
`fretwire_protocol::edit::set_value(slot, param_index, value, txn)` and
`fretwire_core::EditorBlock::set_param_by_name(name, value, txn)` build byte-exact set-value commands for
any knob parameter, using only shipped data. Parameter control is no longer gated on protocol
decoding — only on the live transport (Linux).
