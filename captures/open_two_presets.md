# Capture: open_two_presets_one_after_another

- **pcap:** `open_two_presets_one_after_another.pcapng` · **date:** 2026-06-21
- **action:** opened one preset in HX Edit, then opened a second.

## Result — the state-read exchange (resolves handle discovery)
Opening a preset streams its **full state as MessagePack** on the edit channel (`8010/03ed`):
- OUT op 0x06 `cmd=0x04` (open resource, handle `82 6b 00 6c`) → `cmd=0x0C` (start stream,
  reply = chunk #0) → `cmd=0x08` ×N (chunk reads). Replies are 272-byte frames (256B payload);
  a short read (<272) ends the stream. 20 full chunks + tail for the pair of presets.
- Reassembled preset 1 → `preset1_stream.msgpack.bin` (2804 B). Contains `"l6-helix"`, version
  `v3.71-32-g1039661`, block names (Bucket Brigade, Harmonic Tremolo, Tremolo, 70s Chorus,
  Dynamic Hall), SNAPSHOT names. MessagePack (`0xb2` fixstr, `0xda` str16); root not at offset 0.

## Next
- Parse the blob with `rmpv` (in `fretwire-data`/`fretwire-core`); map block/param entries → wire handles
  (`8X 62 [block] NN`) so editing can target them. Account for the per-chunk stream envelope.
- Re-extract: see the reassembly command in this turn's notes / `tools/` (to be scripted).
