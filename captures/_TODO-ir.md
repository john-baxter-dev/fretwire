# IR (impulse response) management — DECODED AND IMPLEMENTED (2026-08-22)

Two captures landed (`import_ir.pcapng`, `export_ir.pcapng`), the transaction shape was decoded
2026-06-28, and the whole family has now been **driven against a live HX Stomp**. Decode them with
`tools/decode-edits.py <cap> OUT` (the envelopes carry TLV opcode `0x02`, *not* the `0x06` every
block edit uses). HX Edit sends these on the **primary** channel; we send them on the **edit**
channel and the device answers exactly the same, which is the same substitution the preset listing
already makes. What we know:

**Every IR transaction is bracketed by a session open/close:** op **255** `{}` (open) … op **254** `{}`
(close) — the same 255/254 pair the preset browse uses.

**Upload / import — op 9** (`{102:txn, 100:9, 101:{…}}`), target keys:
- `112` = **IR slot index** (0-based; was 1 in the capture).
- `113` = a **u32 checksum of the IR data — SOLVED (2026-07-22) [solid]**. It is the **sum of the
  8192-byte blob read as 2048 little-endian u32 words, truncated to 32 bits** — *not* a CRC:
  ```python
  checksum = sum(struct.unpack("<2048I", blob)) & 0xffffffff   # 0xc0a076ed ✓
  ```
  Verified against `import_ir.pcapng` after reassembling the op-9 upload (16 × 496-byte host→dev
  PRIMARY chunks; same chunking as the op-21 preset write). crc32, crc32-inverted, adler32, byte-sum,
  big-endian sum and xor were all checked and all differ. The earlier "crc32 didn't match" note was
  right about crc32 and wrong to assume a CRC.
- `109` = **name**, a 32-byte NUL-padded string (capture: `"G12-65 212 C Hi-Gn 421+57 Celes"`).
- `114=1, 115=3, 123=false, 124=false, 125=0` = **format/flags**, constant across this one capture —
  meaning unknown (channels? sample format? normalize?). Need captures of *different* IRs to interpret.
- `110` = the **IR audio blob**. The op-9 TLV declared **8259 bytes** total → the blob is **8192 bytes
  = 2048 × float32**, little-endian PCM (a full HD2_ImpulseResponse2048). First samples form a real
  impulse transient. Sent inline in one envelope (no chunking needed at this size).

Then op **13** `{101:2}` = **commit** the write (kind 2, same kind value the preset/IR streams use).

**Download / export** — op **12** `{112:slot}` (select the slot) then op **11** `{112:slot, 101:2}`
(start the read stream); the IR streams back paged like the preset read (`cmd 0x0c` pagination).

## Still needed
- **Delete / rename / reorder** — never captured, and **two reconstructions have now failed**:
  op 10 with a `{112: slot}` target is refused with code `-3`, and writing an "empty" record via
  op 9 does not clear a slot (the device overrides `114`/`115`). Until one of these is captured a
  slot can be overwritten but not emptied. A single HX Edit capture of a delete settles it; probing
  opcodes at a live pedal is the expensive way round.
- **Captures of different IR lengths/formats** (a 1024-sample IR, a stereo IR) to finish `115` and
  `123/124/125`. `114` and `115` are no longer constants — see below.
- `ir-assign-to-block` — how a preset's IR block references a user slot vs a built-in cab IR.

## Implemented and verified live (2026-08-22)
Everything above the "still needed" line is **built and driven against a real HX Stomp**:
`fretwire_protocol::edit::{ir_session_begin,ir_select,ir_stream,ir_commit,ir_upload,ir_checksum}`
(byte-exact against these captures), `fretwire_data::ir` (records, WAV in and out),
`Session::{ir_info,ir_directory,ir_export,ir_upload}`, and the CLI's `ir-list` / `ir-info` /
`ir-export` / `ir-export-all` / `ir-upload`.

Three findings from doing it, all in `docs/protocol.md`:
- **The store is verbatim.** A blob read off the pedal is byte-identical to the one this capture
  recorded HX Edit uploading, and a slot written from that file reads back identical again. An IR
  round-trips bit-exact — nothing is resampled or normalised.
- **`114` is the occupancy flag** (`1` full, `0` empty), and `115` follows it (`3` / `1`). They only
  looked like constants because every sample here came from a populated slot. A zero-filled slot
  with no name still reports `114: 1` — a *silent IR*, not an empty slot.
- **Op 9's reply is `103: 1`**, with the real completion arriving afterwards as a status push. The
  ack is not the verdict; re-reading the slot's checksum is.

---

## Original capture spec (kept for reference)

# TODO capture: IR (impulse response) management

Goal: decode how HX Edit **uploads and manages user impulse responses** on the HX Stomp's IR slots
(the device holds ~128 user IR slots, separate from the built-in cab mic IRs in `cabmicirs.models`).
This is a **new protocol area** — we've never captured it. Likely a binary-upload stream (an IR is a
short `.wav`), distinct from the preset/edit ops, possibly on its own channel or a large chunked
transfer like the preset blob (op 21 / key 110).

## Captures to record (HX Edit open + idle, one action each)
1. **`ir-import.pcapng`** — import a small `.wav` IR into an **empty** IR slot. Note the slot index
   and the file (size, sample rate, length). A short IR keeps the capture small.
2. **`ir-rename.pcapng`** — rename an IR slot.
3. **`ir-move.pcapng`** — move/reorder an IR to a different slot.
4. **`ir-delete.pcapng`** — clear an IR slot.
5. **`ir-assign-to-block.pcapng`** — assign a user IR to an IR block in a preset (so we learn how a
   block references an IR slot vs a built-in cab IR).

For each: the IR **slot index**, the action, and (for import) the source file's size/format.

## What to look for / hypotheses
- Decode with `tools/pcap-frames.py`; watch for a **large chunked upload** (like the op-21 preset
  write: a `{…: <blob>}` envelope streamed in 256-byte chunks with periodic ACKs) carrying the IR
  audio data, plus a small command framing the slot + name + format.
- The IR data may be sent raw, resampled, or wrapped — compare the on-wire byte count to the source
  `.wav` to tell. Note the device's fixed IR length (HX IRs are a fixed sample count, e.g. 1024/2048
  — see `HD2_ImpulseResponse1024/2048` in `fixed.models`).
- **Safety:** IR upload is normal user data (not firmware) — low risk — but it *writes device flash*,
  so treat like `save_preset`: back up, use an empty slot to test.

Once decoded: `edit::ir_*` builders + `Session` methods + GUI IR-slot management; wire into
Phase 7. Until then, IR management is the one device capability with **no** decode at all.
