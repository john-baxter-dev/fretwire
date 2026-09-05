# HX Stomp MI_00 Control Protocol — living spec

Status: **early.** Derived from USB captures (see `captures/`).
Notation: bytes shown as hex; multi-byte integers appear **little-endian** unless noted.
Each claim is tagged **[solid]** (directly observed, repeatable) or **[hypothesis]** (best
current guess, needs a disambiguating capture).

## Transport [solid]
- Device address 8 = HX Stomp (VID 0x0E41 / PID 0x4246).
- Control channel = **USB bulk transfers**, endpoint **0x01 (OUT)** and **0x81 (IN)**, on the
  vendor-specific interface 0. (Wireshark `usb.transfer_type==0x03` = **Bulk**; an earlier
  draft here mislabeled it "interrupt" — the bytes were always right, only the name was wrong.)
- Strict **request/response**: one bulk OUT then one bulk IN before the next command.
- Messages are framed; base frame is **16 bytes**, larger when carrying a body.

> **Frame format** (src/dst u16 LE, `cmd` field, the `arg`/offset field at bytes 12–15, TLV body),
> derived from the captures. The notes below are the original capture-derived analysis.

## Frame layout [solid — validated byte-exact in `fretwire_protocol::frame`, `tests/real_frames.rs`]
```
offset  size  field   notes
0       2     len     u16 LE = 8 + significant-body length (excludes trailing zero padding).
                      16B→0x08, 36B→0x19, 40B→0x1d; stream chunks use the high byte (272B→0x0108).
2       1     00
3       1     magic   0x18 normal; 0x28 only on the first HANDSHAKE packet.
4       2     src     host channel id (LE).
6       2     dst     device channel id (LE). src/dst swap by direction.
8       1     00
9       1     seq     per-channel sequence counter, +1 each frame.
10      1     00
11      1     cmd     0x02 open, 0x04 data/open-resource, 0x08 chunk, 0x0c stream, 0x10 idle.
12      4     arg     u32 LE — per-(channel,direction) **ACK-style stream offset**: a running
                      count of *significant* payload bytes (the L-field body, excluding zero
                      padding) the side has received on that channel. Each `cmd=0x08` chunk
                      request advances OUT `arg` by exactly **256** (one page) — see the `arg`
                      analysis below; this is likely how the device pages the preset stream. NOT
                      a checksum.
16      ..    body    significant bytes (a TLV on data frames); frame zero-padded to 4 bytes.
```
There is **no per-packet checksum**. (An earlier draft guessed offset 10 = a "type/message-class"
field and offset 12 = a "check"; full-frame captures show offset 10 is just padding and 12–15 is
the `arg` offset above. The `cmd` byte at offset 11 — not a 16-bit type — distinguishes idle vs
data vs chunk.)

## Channels [solid]
Three logical channels run concurrently, each with an independent `seq`:
| device-side id | host-side id | observed role |
|----------------|--------------|---------------|
| `ed03`         | `8010`       | **edit channel** — block/param edits + preset stream (identity query op 0x06) |
| `f003`         | `0210`       | status/meters (identity query op 0x04; echoes edit handles) |
| `ef03`         | `0110`       | primary/identity (`"P33Main"`; identity query op 0x05; opened first) |

At idle every channel just trades 16-byte keepalives (`type=0x0010`, incrementing `seq`).

## Session handshake & channel setup [solid] — from `startup.pcapng`
On launch the host brings up all three channels in order **`ef03` → `ed03` → `f003`**, each with the
same two-step open, then loads device identity and the current preset:
```
per channel:  OUT 00 10 00 00  (type 0x0002, SESSION_OPEN)  ->  IN 00 02 00 00   (ack)
then a query: OUT 01 00 0X 00 01 00 00 00 0X 00 00 00       ->  IN .. 0X "P33"/"P33Main" + ver
```
- The open exchange is `00100000` → `00020000` (the `0x0010` keepalive value used as the open
  request; reply `0x0002`). This is the documented SESSION_OPEN, run **once per channel**.
- The identity query's opcode differs by channel (**`ef03`→0x0005, `ed03`→0x0006, `f003`→0x0004**);
  the reply carries the model code string — `ef03` returns **`"P33Main"`**, `ed03`/`f003` return
  **`"P33"`** — plus `0x03800000` (matches preset key 35). `P33` = HX Stomp. That word is **not the
  running firmware version**, despite reading like 3.80: the same value sits in a *3.82* Floor's
  backup header. See "Neither `7 → 37` nor `7 → 35`…" in `docs/preset-format.md`.
- **Model codes seen so far:** `P33` HX Stomp, `P21` Helix Floor, **`P36` HX Stomp XL**. The XL's is
  [solid — 2026-08-21]: an owner's handshake returned `"P36Main"` and a preset read off the same
  pedal carried `P36` at key `7 → 36`, two independent paths agreeing. It is the only field a bug
  report (rather than a capture) has been able to fill in for that device.
- After setup, the **edit channel** (`ed03/8010`) runs meter/state queries and then streams the
  **current preset** (frame 4272 IN begins `da 0a … 6c 36 2d 68 65 6c 69 78` = "l6-helix") — the
  same paged MessagePack mechanism as "Preset open" below.

**Status pushes carry keys 29 *and* 26 too, and both matter.** A panel change mirrors back the same
map an op-30 edit sends — `{98: slot, 29: by_index, 26: model_sel, 28: index, 119: value}` — so a
push names its index space the same way an edit does, and a slot can hold **three** spaces that all
start at 0:

| key | value | key 28 indexes |
| --- | --- | --- |
| `29` | `true` (default) | the selected model's param list |
| `29` | `false` | the block's **extra** values (`Trails`, a legacy cab's mic index) |
| `26` | `0` (default) | the block's own model |
| `26` | `1` | the block's **paired cab/IR** |

Dropping either one silently drives a different control. Key 29 was the first half: a trails toggle
arrived as the model's param 0, so toggling Trails on the pedal swept the *Time* slider. [solid —
`dynamic_ambience_trails_on_off.pcapng` against `dynamic_ambience_mix_modify.pcapng`; issue #5]

Key 26 was the second: on an amp+cab block the cab's `Distance` is paired param **2** and the amp's
`Mid` is main param **2**, so a mic-distance push arrived as a Mid change and moving mic distance
appeared to move the amp's Mid. The write direction was never affected — `edit::set_paired_value`
has always sent `26:1`, byte-exact against the cab captures — only the read-back.

Both forms are captured off one slot: sweeping the cab's `Distance` and then the amp's `Mid` on an
amp+cab block sent 153 pushes, every one `28:2`, differing only in key 26 — 123 with `26:1` (values
in inches, 1..12) and 30 with `26:0` (normalized 0..1). [solid — HX Stomp fw 3.80, 2026-08-23,
`captures/paired_cab_push.md`; issue #11]

**Unifying insight:** those queries use the **same MessagePack envelope as edits**. The meter query
`83 66 cd 03 e8 64 4c 65 80` = `{102: 0x03e8, 100: 76, 101: {}}` — key 102 a counter, key 100 the
operation/resource id, key 101 the target (`{}` = "query all"; `{98: slot, …}` = a block). Editing
is the same shape (see "Edit body" below); the *parameter* within a block is selected by target key
28 = its param index.

## The `arg` stream offset — measured [solid] — from `launch_hx_modify_param_close_Hx.pcapng`
`arg` (header bytes 12–15) is **per (channel, direction)** and behaves like a TCP ACK: it counts the
**significant** payload bytes (the `len`/L-field body, *not* the zero-padded frame length) received
on that channel. Measured deltas on the edit channel (`1080↔03ed`):
- The paged preset read: each OUT `cmd=0x08` chunk request advances OUT `arg` by **exactly 256**
  (`0x1193 → 0x1293 → 0x1393 → …`), while the device's IN `arg` stays pinned at `0x250`. The host is
  declaring "I've consumed up to page N; send N+1." **Likely how the device seeks the next page.**
- For request/reply edits, OUT `arg` jumps by the *significant* byte count of the **reply** it last
  received (e.g. +0x4c after a 76-significant-byte reply), confirming the "bytes received" reading
  (earlier off-by-ones were padded vs significant length).

**★ Predicted first-contact bug:** `fretwire_core::Session::read_preset()` sends `arg = 0` on every chunk
request. If the device pages by `arg` (plausible, per the +256 pattern) rather than purely by its own
state, `pull` will re-read page 0 or stall. **Ready fix:** track a per-channel offset and advance the
`cmd=0x08`/`cmd=0x0c` requests by the significant bytes received (256 per full page). Try `arg=0`
first (current code); if the stream won't advance, switch to the tracked offset. Edits *may* tolerate
`arg=0` since they're self-contained; the paged read is the risky one. (Open: whether the device
*validates* OUT `arg` exactly or treats it as advisory — only hardware will say.)

## Connect flow — newer captures differ from our builder [hypothesis] — from the `launch_hx_*` captures
Our `fretwire_protocol::session::device_handshake()` was reconstructed from `startup.pcapng`. The newer
full-session captures (device now enumerates at **address 9**) show HX Edit doing a **two-phase
connect**:
1. **Probe:** open `ef03` only (`cmd=0x02`, `0x28` magic), one identity query **op 0x0005** → reply
   `"P33Main"`, one chunk read, then **close `ef03`** with a bare `cmd=0x02` (the same teardown frame
   as on exit). This is a throwaway identity probe.
2. **Real session:** bring up channels in order **`ed03` → `f003` → `ef03`** (note: edit first,
   primary *last* — different from `startup.pcapng`'s `ef03→ed03→f003`). Each channel's identity
   query uses a **different op**: `ed03`→`0x0006`, `f003`→`0x0004`, and **`ef03`→`0x0002`** (value 2)
   — *not* the `0x0005` our builder sends. The device still answers (`"P33"`).

Our builder's `ef03` op-0x0005 query is exactly what the **probe** does and the device clearly
honours it (it returns `"P33Main"`), so `connect` should still establish identity. But if first
contact stalls, this is the alternative to mirror: probe-then-open, edit-channel-first, primary
identity via op 0x0002. A strict request/response device likely tolerates the channel order; the
op/two-phase difference is the more likely culprit. (`tools/dump-control.ps1 -Address 9` +
`tools/dump-arg-deltas.ps1` reproduce these — both Windows-only, so this note is the handoff.)

## Session teardown — the "panel lock" fix [solid] — from the `*_close*.pcapng` captures
On exit, HX Edit **cleanly closes each channel** with a single **session-close** frame: `cmd=0x02`
(same opcode as the handshake open) but with an **empty body** (16-byte frame, `magic=0x18`). The
device acks each with the same opcode. Order is **status (`f003`) → edit (`ed03`) → primary (`ef03`)
last** (primary closed last, the reverse of how it leads on open):
```
OUT 1002→03f0 cmd=02 (len 16, no body)   ─┐ status close
OUT 1080→03ed cmd=02 (len 16, no body)    │ edit close
IN  03ed→1080 cmd=02 ack                  │
IN  03f0→1002 cmd=02 ack                  │
OUT 1001→03ef cmd=02 (len 16, no body)    │ primary close (last)
IN  03ef→1001 cmd=02 ack                 ─┘
```
Verified byte-identical in `launch_hx_modify_param_close_Hx.pcapng` (frames 9080–9092) and
`launch_hx_rename_snapshot_savepreset_switchsnapshot_closehx.pcapng` (frames 11621–11633). The same
bare `cmd=0x02` close also appears mid-startup as a **probe teardown** (open `ef03`, query identity,
close it, then bring up the real session) — so `cmd=0x02` is a generic *session-control* opcode:
**open** = `0x28` magic + body `00 10 00 00`; **close** = `0x18` magic + empty body.

**Why it matters:** if the host just drops the USB handle without sending these closes, the pedal
keeps believing an editor is attached — the front panel stays in the connected/locked state and
behaves wonky until power-cycled. This is the observed "panel lock." Implemented as
`fretwire_core::Session::close()` (also run from its `Drop`), builder opcode `fretwire_protocol::cmd::SESSION_CLOSE`.

## Device settings: op 24 reads, op 25 writes [solid — verified live on an HX Stomp 2026-08-22]

Settings are a **flat numbered namespace**, not a structured document. `op 24 {118: id}` reads one —
the reply carries the value at key `119` — and `op 25 {118: id, 119: value}` writes it. Both ride
the ordinary edit envelope and are not block-addressed.

**166 of ids 0..=600 answer on an HX Stomp** — nothing above 226 answers, so that is the ceiling
rather than an artefact of where we stopped looking.

**28 are identified.** All were read off a physical pedal by changing one setting on its own menus
and diffing two dumps: an HX Stomp [solid — 2026-08-22], plus the five marked **XL**, which came
from an HX Stomp XL owner working the same loop [solid — 2026-08-23, PR #11]. An id one unit has and
the other doesn't simply refuses on the other, and `scan_settings` reads a refusal as absence.

| id | setting | type | values |
|---|---|---|---|
| 2 | send/return L level | `bool` | `true` line, `false` instrument — **XL** |
| 3 | send/return R level | `bool` | `true` line, `false` instrument — **XL** |
| 9 | MIDI base channel | `int` | zero-based: `3` is channel 4 |
| 11 | MIDI over USB | `bool` | |
| 14 | tempo scope | `int` | `0` snapshot, `1` preset, `2` global |
| 16 | tempo, BPM | `f32` | |
| 27 | **preset numbering form** | `bool` | `true` flat `000`-`127`, `false` banked `01A`-`32D` |
| 28 | current preset index | `int` | |
| 31 | input level | `bool` | `true` line, `false` instrument |
| 73 | snapshot edits | `int` | `0` recall, `1` discard |
| 81 | bypass type | `bool` | `true` DSP, `false` analog |
| 94 | output level | `bool` | `true` line, `false` instrument |
| 127 | auto In-Z | `int` | `0` and `1` observed, neither named — see below |
| 153 | USB in 1/2 trim | num, dB | wire type not recorded — **XL** |
| 154 | return type | `int` | `0` return, `1` aux in — **XL** |
| 156 | volume controls | `int` | `1` phones, `2` main+HP |
| 158 | phones monitor | `int` | `1` main L/R, `2` send — **XL** |
| 190 | global EQ low frequency | `f32` Hz | |
| 191 | global EQ low Q | `f32` | |
| 192 | global EQ low gain | `f32` dB | |
| 193 | global EQ mid frequency | `f32` Hz | |
| 194 | global EQ mid Q | `f32` | |
| 195 | global EQ mid gain | `f32` dB | |
| 196 | global EQ high frequency | `f32` Hz | |
| 197 | global EQ high Q | `f32` | |
| 198 | global EQ high gain | `f32` dB | |
| 199 | global EQ low cut | `f32` Hz | `19.9` is off |
| 200 | global EQ high cut | `f32` Hz | `20100` is off |

**Factory defaults, for the EQ only** [solid — one HX Stomp, 2026-08-22]. The pedal resets a
setting when its knob is pushed in; these are what it reported once every Global EQ knob had been
pushed. `193` is the one that proves the gesture rather than the state — it had been left at 1900 by
hand and came back as 2000.

| id | default | | id | default |
|---|---|---|---|---|
| 190 low frequency | 110 Hz | | 196 high frequency | 8000 Hz |
| 191 low Q | 0.707 | | 197 high Q | 0.707 |
| 192 low gain | 0 dB | | 198 high gain | 0 dB |
| 193 mid frequency | 2000 Hz | | 199 low cut | 19.9 (off) |
| 194 mid Q | 0.707 | | 200 high cut | 20100 (off) |
| 195 mid gain | 0 dB | | | |

No default appears anywhere else we look: not in any shipped `.models` file, not in
`HelixControls.json`, and not in the protocol, which offers a value and neither a default nor a
range. So this is **one unit's** factory EQ, recorded with its provenance rather than asserted as
universal — a Floor or an LT may differ, the same caveat the Floor's setlist names carry. No other
id's default has been observed.

**`201` answers `nil`; `202` is `1` and `203` is `true`** — they are implemented and still
unidentified. Worth noting only because an id that answers with no value is a shape the sweep hadn't
turned up before.

**The EQ is `190`-`200`, complete and contiguous** [solid — 2026-08-22]: three bands of
frequency/Q/gain at 190-192, 193-195 and 196-198, then the two cuts. The last four were predicted
from the pattern and then confirmed on the pedal rather than left as an inference — `194` moved
0.707 → 3.5 for mid Q and `197` 0.707 → 0.1 for high Q, with `195`/`198` taking the gains.

**`201`-`203` are not identified.** An earlier note in this file glossed them as "global EQ", which
was a guess made before any of the above was measured; the EQ bands turned out to be 190-200, so the
gloss is withdrawn rather than kept as a maybe.

**`127` is `Auto In-Z`, and this file called it `guitar In-Z` for a day** [corrected 2026-08-23].
The pedal shows **Auto In-Z**, two-valued (`First` / `Enabled`), tenth under **Preferences**. The
wrong name did not come from the pedal or from the person at it — the thirty-eighth round's write-up
supplied a Helix setting that sounds like it, and put the row under **Ins/Outs** because that is
where a Helix keeps *Guitar In-Z*. An XL owner noticed the entry doesn't exist on that pedal, which
is what surfaced it. Name and group are both corrected.

Its **values are still unnamed**: `0` and `1` were observed with the menu entry not recorded either
side, so which one is `First` can't be recovered after the fact. One pass at the pedal, naming the
exact entry before and after, finishes it. (`156` was in the same state until its owner named both
positions: `1` leaves the main outputs alone.)

**Not in this namespace: the input noise gate.** It is per-preset — `noiseGate`/`threshold`/`decay`
on the fixed input node at slot 0 of each DSP, set with the ordinary set-value op. See
`PresetStream::io_node`.

**Op 24 was already in this codebase under the wrong name.** The connect capture sends `{118: 128}`
and we had it written up as `OP_READ_PREP`, a "read-sequence prepare step", because we only ever
replayed it. It is not a prepare step — the handshake is fetching setting 128 along the way. The
missing read side that made this whole area look capture-blocked was a call we had been making
since the first handshake.

### Two orders, on purpose
`fretwire_protocol::settings::SETTINGS` is kept **in id order** — it is searched by number as often
as it is read through — and the pedal's own menu order lives beside it as `MENU_ORDER`, a list of
ids. `settings_read` sorts its rows by `(menu_rank, id)`, so the panel shows Ins/Outs the way the
pedal's screen does while an id nobody has placed in a menu keeps its numeric position at the end.

### Settings are typed, and the type is enforced [solid]
A write whose value type differs from what the device already holds is refused with `-3`. Tempo is
an `f32`, so an integer `132` is rejected outright while `132.0` is taken — which is why
`Session::set_setting_num` reads the current value before writing and sends back its type.

### Mapping the id space needs no capture
Dump the space, change one thing on the pedal's own menus, dump again, diff. `fretwire
settings-dump` / `settings-diff`. A full 601-id sweep takes **1.4 s** — ~0.8 ms a read — so the loop
is bounded by how fast someone can work the pedal's menus, not by USB.

That is how the whole table above was built: **19 ids in eleven minutes, with no capture and no
disassembly.** The method's one rule is one change per cycle, since a diff naming two ids can't say
which did what — and in practice the noise is id 28 (the current preset index) and id 16 (tempo, if
anything tap-driven is running).

## Body / inner command [solid]
On a data frame the body (offset 16+) is:
```
offset  size  field    notes
0       2     flag     [solid] 01 00 on OUT (host command), 00 00 on the IN reply.
2       2     opcode   [solid] 0x0006 for both bypass and parameter-set edits.
4       4     ilen     [solid] inner data length, u32 LE (0x0d bypass, 0x17 param-set).
8       ilen  data     [solid] command payload; frame zero-padded to a 4-byte boundary.
```
The `data` field is itself **MessagePack** — see "Edit body is MessagePack" below.

### Parameter values are big-endian f32 [solid]
From the tremolo **Mix** sweeps (`set_tremolo_mix_to_100`, `toggle_tremolo_on_set_mix_to_0`):
the **last 4 bytes of a param-set `data` are an IEEE-754 float, big-endian**.
```
3f800000 = 1.0 (100%)   3f000000 = 0.5    3ecccccd = 0.4    3e4ccccd = 0.2    00000000 = 0.0
```
This matches the 0.0–1.0 `min`/`max` in the `.models` data. Dragging a knob streams every
intermediate value as its own opcode-0x0006 frame (hence dozens of frames per gesture).

### Edit body is MessagePack [solid] — supersedes the "opaque handle" reading
The inner `data` decodes cleanly as MessagePack (same encoding as the preset stream). What an
earlier draft read as an opaque handle `83 66 cd … / 8X 62 [slot] NN` is just msgpack bytes —
`0x62` is integer key 98, `0x82`/`0x85` are `fixmap{2}`/`fixmap{5}`, `0xca` is float32, `0xc3`/`0xc2`
are true/false. Proven in `fretwire-data/tests/edit_body_msgpack.rs`; modeled by `fretwire_protocol::edit::EditBody`.

Decoded structure (confirmed across many single-knob captures, `captures/param_map_findings.md`):
```
{ 102: <u16 counter>,        // a session-wide running transaction counter (whole 16-bit value)
  100: <op>,                 // 41 = bypass, 30 = set-value
  101: { 98: <slot>,         // block slot index
         // bypass:   59: <bool>                              (new bypass state)
         // set-value: 29: true, 26: 0, 28: <param_idx>, 119: <value> } }
```
- **Key 102** is a **whole u16 running counter** (NOT split into op+counter — the same param edited
  later shows `0x04xx`, `0x05xx`; an earlier 2-sample guess that the high byte was an op class was
  wrong). It increments per edit.
- **Key 100 = the operation**: `41` bypass, `30` set-value (same envelope is reused for the startup
  property queries, where 100 is the property id and 101 is `{}` = "query all").
- **Key 101 = the target.** Inner **key 98 = block slot** [solid].
- **★ Parameter selector = target key 28 = the parameter's INDEX in the model's device (`Helix.sym`)
  param order.** This is the key result: **parameter editing is computable from shipped data** — no
  per-param captures needed. Verified: Bucket Brigade Mix→3, 70s Chorus Mix→4, Dynamic Ambience
  PreDelay→1 / Mix→5 / LowCut→6 / Level→8 (each = its index in the block's mono/stereo `Helix.sym`
  list). The same param has different indices on different models, exactly as its position differs.
- **Value = target key 119** (float32 for knobs; int for enums; bool for switches). Key `29: true`
  is a constant descriptor on knob edits. The type is **not** coerced — sending the wrong one is
  refused outright; see "The value's wire type must match the parameter's type exactly" below.
- **★ Sub-model selector = target key 26** [solid]: `0` = the block's **main model** (amp/effect),
  `1` = the **paired cab/IR** fused into the same slot (amp+cab blocks). A cab-param edit is byte-for-byte
  a main edit with `26:1`; the param index (key 28) is then positional in the *cab's* `Helix.sym`
  order (mic=0, mic position=1, mic distance=2, mic angle=3, …). Decoded from the cab-mic/distance/
  angle captures + `captures/cab_mic_order.txt`. (Same key number as the model-ref's paired-index, a
  different context.) Builder `set_paired_value()` / general `set_value_on(slot, model_sel, …)`.
- **Edge case:** switch/transport-style params (a `@trails` toggle, a tempo-sync note) both decode
  with **key 28 = 0** rather than a real index — they use a different addressing (and aren't in the
  block's main param list). TBD; normal knob/continuous params follow the index rule above.
- Builders `fretwire_protocol::edit::bypass()` / `set_value()` regenerate these bytes exactly.
- The `f003` status channel echoes the same msgpack back (state mirror) — useful for reading state.

### An edit ACK must be matched by its transaction id, not by arrival order [solid] — 2026-08-01
"The next non-keepalive frame on the edit channel" is **not** the reply to the edit you just sent.
The device interleaves two other things on that channel: empty `cmd 0x08` credit frames, and — after
a browse/list read — leftover chunks of the previous stream. Take one of those as the ACK and two
things go wrong at once: the edit is reported applied on the strength of a frame that says nothing
about it, and every later reply is off by one, so a refusal is attributed to the wrong command. The
mismatch then *suppresses* the refusal check (`sent_txn == txn` fails), which is how an edit the
pedal threw away reached the GUI as a success.

Measured over the 2026-07-30/31 field logs — 353 edit ACKs across ops 20/28/30/39/40/41/43/71/78:

| | count | |
|---|---:|---|
| correlated (reply echoes the txn we sent) | 233 | |
| **empty body** | 86 | a `cmd 0x08` credit taken as an ACK |
| **echoes an earlier txn** | 50 | the desync, lag 1–5 |
| cross-stream | 1 | an op-20 reply carrying preset-*list* bytes |

Per op, the structural path was the worst affected: **op 43 `move_block` never once correlated**
(0 of 21) and op 78 `begin_structural` managed 6 of 24, while op 30 `set_value` — the slider drag —
ran about 63% correct. op 71 `save_preset` was 0 of 21: every save we have ever "confirmed" was
confirmed by a credit frame. Its real ACK is `{102: txn, 103: 0, 104: nil}` and arrives right after,
so it was being read one frame too early.

`Session::send_edit` now correlates every edit by the txn echoed at key 102
(`Transport::request_matching`), skipping credit and cross-stream frames instead of consuming them.
Verified on an HX Stomp 2026-08-01: 46 consecutive op-40 swaps plus ops 30/39/41/43/71 all matched
their own transaction, save included.

> This was floated as a mechanism for the op-21 write freezes on the reading that a structural drag
> is `op 78 → op 43 → op 21`. **That sequence is not in any capture** (checked 2026-08-02, all 43):
> `move_EQ_right_two_slots` is a bare op-21 with no bracket, `one_by_one_move_all_blocks_one_right`
> is `78,43` eleven times with no op-21, and `move_simple_eq_to_parallel_path` is `43,23`. HX Edit
> sends a whole-preset write on its own, exactly as we do. Dropped as a lead.
>
> Postscript (2026-08-31): the **POD Go** turned out to use a two-thirds version of it for real —
> POD Go Edit's move is `op 78 → op 21` (no op 43 at all; the whole rewritten document rides the
> op-21). See `docs/pod-go.md` § "Structural edits".

### The reply's key 103 is a status, and `255` means refused [solid] — from the 2026-07-30 Floor log
Every reply envelope is `{102: txn, 103: kind, 104: payload}`, and until now we read key 103 as a
don't-care (`103:_` in the notes below). It is the **kind of answer**, and one of its values is a
refusal:

| 103 | 104 | meaning |
|---|---|---|
| `0` | the payload | data reply — the read-info identity, a stream chunk, the echo of an applied edit |
| `1` | `nil` | plain ack (bypass, whole-preset write) |
| `255` | `{111: code}` | **the device refused the command** |

A refusal is silent in every other respect: no error frame, no state change, and the next read
returns the preset byte-identical. Observed twice in one session — `{102:44, 103:255, 104:{111:-21}}`
and the same again at `102:60` — while the 6778-byte stream stayed 6778 across both. The meaning of
`fretwire_data::stream::parse_edit_rejection` reads it, and `Session::send_edit` turns it into
`Error::Rejected` — before this, `send_edit` logged the reply and returned `Ok`, so the GUI reported
success for edits the pedal had thrown away.

Two refusal codes are observed so far. Both are refusals of an op that is *valid in general* but not
for that target, so the code looks like "wrong shape for this thing" rather than a transport error:

| code | seen on | meaning [hypothesis] |
|---:|---|---|
| `-21` | op 39 `add_block` carrying a `paired_index`; op 40 swapping to a `*WithPan` twin | a model-ref the op will not take for that target |
| `-3` | op 30 `set_value` writing a split node's `bypass` param; op 30 sending the **wrong wire type** for the param; op 30 addressing a param **past the model's symbol list** | that parameter is not writable *as asked* — wrong op, wrong type, or no such index |
| `-306` | op 40 swapping to a model the DSP has no room for; op 40 swapping any block to a `*WithPan` dual-cab twin | **out of DSP** — see below. The dual-cab twins are a separate case (a different block type, not an in-place swap target) that answers `-21` in some device states, so there treat the refusal, not the code, as the signal |

The `-3` case is worth knowing about because `bypass` **is** a real entry in the split's stored param
array, and the four split models are the only ones in the whole catalog (4 of 681) that carry it. So
the editor will happily offer it as a knob; `Session::set_param` now recognises it and sends op 41
instead. [solid — 2026-07-31, two op-30 writes to a Split Y's bypass, both refused with `-3`]

### `-306` on op 40 means the model does not fit in the DSP budget [solid]
The field logs made this look like a property of the *model* — a Room reverb refusing to become a
Euclidean Delay, a Bleat Chop Trem refusing to become an Elephant Man. It is a property of the
**preset's total DSP load** at the moment of the swap, and nothing else.

Measured on an HX Stomp (fw 3.80, 2026-08-02), same preset, same block, same target model, only the
free DSP changed:

| slot 4 → `HD2_DelayElephantManMono` | preset before | result |
|---|---:|---|
| as loaded | 71.8% | refused, `-306` |
| after freeing 6.5% elsewhere | 65.3% | **accepted** (→ 68.8%) |

A ladder of targets on one slot from a fixed baseline brackets the ceiling. The number is the total
the preset *would land on*, by our meter:

| landing total | result |
|---:|---|
| 73.3%, 73.8%, 74.4%, 74.9% | accepted |
| 75.3%, 75.6%, 76.5% | refused, `-306` |

So the device fills up at **~75% on our meter**, not 100%: "28% free" can mean no room at all. This
is `fretwire_core::editor::DSP_CEILING`, and it is what every fit check and headroom figure now
compares against; `EditorPreset::dsp_free_on` returns `DSP_CEILING - load`, never `100 - load`.

The ceiling holds on the Helix Floor too, from the other direction: the tester's `somehinged3` sits
at **72.7%** on DSP1 and refused both Elephant Man Mono (`load` 6.02) and Euclidean Delay (10.5),
which puts that device's ceiling under 78.7 — and above 72.7, since the preset itself loads and
plays. Two different devices, same neighbourhood. **The ceiling is per DSP**, not per preset: a
Floor preset can total 120% across two DSPs and be nowhere near a refusal.

#### The missing quarter is a flat reserve, not the routing nodes [solid — 2026-08-04]
The standing theory was the fixed nodes we never add in, and `io.models` does price them — so the
numbers were finally in hand to check it:

| node | `load` |
|---|---:|
| `HelixStomp_AppDSPFlowInput`, `…OutputMain`, `…OutputSend` | 10.99 each |
| `HD2_AppDSPFlow1Input` / `2Input` | 10.99 |
| `HD2_AppDSPFlowOutput` | 8.00 |
| `HD2_AppDSPFlowJoin` (mixer) | 10.99 |
| `HD2_AppDSPFlowSplitY` / `SplitAB` 1.50 · `SplitXOver` 2.27 · `SplitDyn` 3.50 | |

They are real nodes in a real preset, not bookkeeping: slots `0`, `9`, `10` and `19` of each DSP's
slot array are the input, output, split and mixer (`19 => 0/1/2/3` where an ordinary block is `6`),
each with its own params. **They are also not what we are missing**, and the arithmetic said so
before the census did: the ladder preset is parallel, so the four together are `10.99 + 8.00 + 1.50
+ 10.99 = 31.48`, which would put the ceiling at 68.5 — but 73.3 was *accepted*. No subset lands on
the ~25 the bracket demands.

**The census settles it.** The tester's `.hxb` backup holds 363 presets across eight setlists,
including Line 6's own two factory setlists — 458 DSPs carrying blocks, every one of them a preset
the hardware accepted and plays. Summing each DSP's block loads the way our meter does:

| slice | n | max load |
|---|---:|---:|
| everything | 458 | **74.84** |
| parallel DSPs | 151 | 74.84 |
| serial DSPs | 307 | 74.80 |
| DSP1 | 302 | 74.84 |
| DSP2 | 156 | 74.77 |

**The wall is flat.** If the split and mixer were billed against this budget, serial presets would
run to ~87 and parallel ones stop at ~75; instead both stop at 74.8, a difference of 0.04. The same
holds across split types and across the two DSPs. Nothing that varies with the preset is being
charged, so the missing ~25 is a fixed reserve — the device keeps a quarter of each DSP for itself
and lets blocks have the rest.

Two corollaries worth having. The distribution has 46 DSPs in the 70–74.84 band and **none above**,
so Line 6's own designers build right up to this number — the best available evidence that ~75 is
the real limit and not an artefact of our load table. And combined with the Stomp ladder (74.9
accepted, 75.3 refused) the ceiling is pinned to **[74.9, 75.3)**, which is why `DSP_CEILING` is
75.0.

Consequence for the code: we do **not** fold the node loads into the meter — there is now positive
evidence they do not belong there, and the raw block sum is quoted in every log and capture note on
record. The correction lives entirely in the ceiling.

**Two scales, deliberately.** Every load figure in this document, in the `.models` files and in the
tester's logs is *raw* — a percentage of a budget the hardware never hands over, which is why 72.7
read as "27% free" when it was nearly full. What a user is shown is `blocks ÷ 75 × 100`
(`editor::dsp_percent`), so the ceiling reads 100%. **Everything on a given screen is scaled**,
per-block costs and model-picker costs included, so the figures in a listing sum to its total. The
one raw number kept in front of a user is the CLI header's bracket — `DSP 97.0% · 3.0% free  [raw
72.7 of ~75]` — which is the anchor between a pasted log and the tables here. Fit comparisons stay
in raw units throughout; scaling is strictly presentation.

It remains a guess that this is also what HX Edit displays. One screenshot would confirm it
(`captures/_RUNBOOK-hx-edit-session.md`); nothing depends on the answer.

##### Reproducing the census
The `.hxb` payload is concatenated raw zlib streams; streams `130..=137` are the setlists, and each
preset's `tone` object carries `dsp0`/`dsp1`, each with `block0..7`, `cab0..`, `inputA`, `inputB`,
`outputA`, `outputB`, `split` and `join`. A DSP is parallel when any of its blocks has `@path == 1`;
a block's load is its `@model` at `load`/`load_stereo` per `@stereo`, and an amp's paired cab is a
sibling `cabN` entry (count it once — that is where our meter's fused amp+cab figure comes from).
Note **each DSP has two inputs and two outputs**, not one, which is worth knowing independently of
the DSP question.

Eight probes were needed to kill the first theory, that op 40 cannot cross a model *category*. It
cannot: tremolo→delay, reverb→delay and delay→reverb all succeed with DSP free, and the one
category-preserving swap that failed (70s Chorus Mono→Stereo) failed on capacity like the rest.

`Session::send_edit` glosses the code rather than guarding against it locally — the pedal decides
what fits, we only explain the answer. (Out-guarding the pedal is how the row-B stranding and node
enclosure mistakes happened; see `docs/helix-floor.md`.)

### The value's wire type must match the parameter's type exactly [solid]
Key 119 is not coerced. A **switch** takes a MessagePack bool and *only* a bool: an int or a float
carrying the same 0/1 is refused with `-3` and nothing is applied. Measured on an HX Stomp (fw 3.80,
2026-08-02) against `HD2_DelayBucketBrigade`'s `TempoSync1`:

| sent | result |
|---|---|
| `Float(1.0)` | refused, `-3` |
| `Int(1)` | refused, `-3` |
| `Bool(true)` | **accepted**, reads back `true` |

This is a footgun rather than an obstacle: nothing about the refusal says "wrong type", and a client
that picks one wire type per control (a float for every slider, an int for every switch) gets a
parameter class that can never be written. Ours did — the GUI's on/off switch sent an int, so every
switch in the editor was a guaranteed refusal until 2026-08-02. `Session` now reads the param's type
out of the device's own last preset blob and coerces, so the reference data isn't needed for it.

### Target key 29 chooses what key 28 indexes [solid]
Some blocks send **one more value than their symbol names** — `Trails` on a delay/reverb, the mic
index on a legacy (non-`CabMicIr_*`) cab. These have no position in the symbol's param order, so the
ordinary addressing cannot reach them and op 30 refuses every wire type with `-3`:

    Trails, index 8 of 9, HD2_DelayBucketBrigade, 29:true — Bool(true) -3, Int(1) -3, Float(1.0) -3
    TempoSync1, index 7, 29:true — Bool(true) OK

**Key 29 is the switch between two addressing modes.** `true` — every ordinary edit — means key 28
is the param's index in the model's `Helix.sym` order. `false` selects the block's *extra* values,
and there the lone trailing value is index `0`:

| what | body |
|---|---|
| Dynamic Ambience `Mix` (`dynamic_ambience_mix_modify`) | `{98: 7, 29: true, 26: 0, 28: 5, 119: 0.5}` |
| Dynamic Ambience `Trails` (`dynamic_ambience_trails_on_off`) | `{98: 7, 29: false, 26: 0, 28: 0, 119: <bool>}` |

Six trails toggles in that capture, all the same shape. Confirmed live on an HX Stomp (fw 3.80,
2026-08-02): `{98: 2, 29: false, 26: 0, 28: 0, 119: true}` turns a Bucket Brigade's trails on and it
reads back `true`. Builder `edit::set_value_flagged`; `Session::set_trails` and the CLI's `trails`
wrap it, and the ordinary setters route a param whose `extra_index` is set through the same path, so
the GUI's Trails switch works like any other.

A block with **two or more** values past its symbol list has never been seen, and there is no
evidence for what a second one's extras index would be — those stay unaddressable rather than
guessed (`EditorParam::settable == false`).

### op 39 will not add a paired cab; add then swap [solid] — same log
`add_block` (op 39) is refused whenever the model-ref carries a real `paired_index` — i.e. every pick
from the synthetic **Amp+Cab** category, since each amp's `amp.models` `ircablink` cab sits at
`Helix.sym` index 687–829. The refused frames are 56 bytes on the wire, which is op 39's 51-byte
frame plus the two bytes a `uint16` paired index costs; both successful adds in the same session
carried `26: -1`, and the preset the tester saved has the amp and a cab as two separate, unpaired
blocks — the fallback you make after the paired add refuses twice.

`swap_model` (op 40) *does* take a paired index — that path is byte-exact against the capture tests,
including a `uint16` index — so `Session::add_block` now adds the amp bare and pairs it with a
following op 40, which is the order HX Edit uses.

### The empty `cmd 0x08` frames during an op-21 write are flow-control credits [solid]
*2026-07-31 Floor logs — two lockups reproduced on one user action.*

Sending a whole preset (op 21) means ~14 OUT `cmd 0x04` data frames of 496 bytes. The device answers
each with an **empty `cmd 0x08` frame**, and that reply is a credit: it means "I consumed that one".

| write outcome | credits per chunk | worst deficit |
|---|---|---|
| completed (14 chunks) | `0 3 1 1 1 1 2 1 1 1 1 1 1 1` | **1** |
| froze after chunk 1 | `2 0 0 0 0` | 3 |
| froze after chunk 7 | `1 2 1 1 1 1 1 0 0 0 0` | 3 |
| completed, *after* the pacing fix | `1 2 1 1 1 1 1 1 1 1 1 1 1 1` | **0** |
| froze after chunk 2, *after* the pacing fix | `1 1 0 0 0` | 3 |

A healthy transfer never runs more than **one** chunk ahead of its credits, and none at all once the
host waits for them. Every lockup shows the credits stopping dead and the host running 3+ ahead — and
once the device stops draining its OUT endpoint, an unbounded `bulk_out` **never returns** (the
pre-fix logs end on `Submitted URB … on ep 1` with no completion). Wait for each chunk's credit;
abort the transfer while the pedal is still recoverable.

**Pacing is not the cause.** [solid] With the host waiting properly for every credit, the same action
still kills the device at the same place — 2,480 of 6,817 bytes, credits stopping after chunk 2. So
the credits are how you *detect* a wedged device, not how you avoid wedging one. What actually
triggers it is still open; the blob that does it is ~1 KB in by then, so the device is reacting to
something it has already consumed rather than to the finished preset. [hypothesis]

#### The credit **latency** predicts it a chunk early [solid]
*2026-08-05, `fretwire55`/`56` + `zadtheinhaler57`/`58` — 20 writes with per-chunk timings.*

The device doesn't stop dead; it slows down first, and one chunk is all the warning there is. The
**chunk-2** credit separates the outcomes with no overlap:

| chunk-2 credit | writes | outcome |
|---|---:|---|
| 1–3 ms | 17 | all completed |
| 28 / 32 / 94 ms | 3 | all wedged |

Two things make this easy to measure wrong. Chunk **one**'s credit predicts nothing (0–5 ms in both
groups) — an earlier analysis found the right signal but computed it from the gap between the first
two chunk log lines, which is chunk two's wait, and the field added to record it was then named and
implemented for chunk one. And a slow credit on the **final** chunk is normal: 2–195 ms on writes
that complete perfectly, because that is the device committing the preset. Only non-final chunks
count.

#### A write straight after a `goto` stalls deterministically — read first [solid]
*2026-08-26, live HX Stomp.*

`restore` (goto → op-21 write) stalled at the same place twice running — 512 of 2230 bytes,
credits stopping at 4 with a 69 ms first credit — while `write-roundtrip` (read → op-21 write of
the same-size blob) completed immediately after, on the same session pattern. The difference is
the **read**: every op-21 write that has ever succeeded ran on an edit buffer a read had opened
(op 76 leads the read sequence), and a `goto` evidently invalidates that state. Inserting a
`read_preset_raw()` between the goto and the write made the identical restore complete on the
first try, and the flash read-back matched the written document.

Unlike the wedges above, this stall is benign when guarded: the device stayed fully responsive,
the reload cleared the half-written buffer, and nothing reached flash. Whether any of the July
wedges had this cause is unchecked — those logs would have to be re-read with the goto/read
distinction in mind before claiming it.

Related and useful on its own: **a `goto` to the slot the device is already on is a real reload,
not a no-op** [solid — 2026-08-27, live HX Stomp: make an edit, re-select the same slot, and the
re-read buffer is byte-identical to the flash copy]. That is the whole implementation of
`Session::revert_preset` (CLI `revert`, the GUI's "Revert to saved") — the pedal's own
switch-away-and-back gesture to discard edits, minus the switch away.

**Backing off does not rescue it.** [solid] The obvious reading — the device has fallen behind, so
stop feeding it and it recovers — was shipped as a 120 ms stand-off and failed 3 for 3. Every wedge
went from one slow credit to total silence on the very next chunk, and not a single credit arrived
during the pause. So a slow credit is not a queue you can drain by waiting; it is the endpoint on
its way out. The useful response is to stop while the next chunk is still in hand, which at least
stops pushing bytes at a device that has stopped taking them.

#### One credit per unit, and our "credits" were never credits [solid]
*2026-08-06, HX Stomp, `write-roundtrip` at `ccfd3d2`, 8 writes on two presets.*

The host's credit wait accepted *any* frame — no filter on channel or opcode — so keepalives
(`cmd 0x10`) and status-channel pushes (`cmd 0x04` from `0x03F0`) satisfied it as readily as the
device's `cmd 0x08`, and the loop pushed the next 512 bytes on the strength of it. Every completed
write in the 08-05 logs ends with **more credits counted than chunks sent** (+1 to +4).

Measuring it directly settles what the surplus is. Classifying every frame in the wait gives
**5 chunks / 5 real credits / 3 strays** and **6 chunks / 6 real credits** across eight writes:
the device credits **exactly one `cmd 0x08` per 512-byte unit**, never per frame. The strays:

```
0x03f0  cmd 4  body 21    status-channel panel push
0x03ed  cmd 4  body 17    the edit apply-ACK, {102: txn, 103: 1}
0x03f0  cmd 8  body 0     the status channel's own page frame
```

That third one is `cmd 0x08` on the *status* channel, so matching on opcode alone is not enough —
the source check is doing real work. It also refutes the competing reading, that the device credits
each of the two frames a unit ships as (496 + 16), which would have made the surplus correct.

Two consequences beyond the pacing. The apply-ACK is a stray by this definition, so a write whose
ACK arrives mid-transfer was reporting `acked=false` while the device had plainly taken it — the
ACK has to be looked for in the set-aside pile as well as the closing sweep. And two of the three
strays are status-channel frames, which means the write path was swallowing the panel mirror for
the length of every save, the same data `request_matching` was discarding by a different route.

The wait is now for a real credit, strays are held and put back when the transfer ends, and both
guards run off the strict count. Whether this is *the* Floor wedge is still open — a Stomp has
never wedged, so it cannot be shown here — but the host can no longer get ahead of the device by
mistaking a panel push for permission to send.

### A credit unit is **512 payload bytes, sent as 496 + 16** [solid]
*2026-08-02, re-read of `move_EQ_right_two_slots.pcapng` and `import_ir.pcapng`.*

The credit is not per frame. HX Edit sends **512 payload bytes per credit**, and it splits them
across two frames — one of 496 bytes, then one of 16:

```
OUT  cmd=0x04  blen=496      OUT  cmd=0x04  blen=496
OUT  cmd=0x04  blen=16       OUT  cmd=0x04  blen=8      ← whatever is left of the 512
IN   cmd=0x08  blen=0        IN   cmd=0x08  blen=0      ← one credit for the pair
```

The reason is USB, not the protocol. The bulk endpoints declare `wMaxPacketSize` **512**, and a frame
is a 16-byte header plus its body — so a 496-byte body is a packet of *exactly* the maximum size.
A bulk transfer made only of maximum-size packets never terminates; it takes a short packet to close
it. Splitting 512 into 496 + 16 makes the second frame a 32-byte packet, and that is what ends each
unit. Both captures that carry a bulk upload do it without exception: the op-21 preset write
(`496,16,496,16,496,16,8,496,8,8,496,16,423` for a 2991-byte TLV) and the IR upload (fifteen 496+16
pairs). Nothing else in the protocol ever sends a 512-byte packet.

We sent 496-byte bodies back to back and so emitted an unbroken run of maximum-size packets —
measured on a Stomp, `512 512 512 512 512 224` where HX Edit would send
`512 32 512 32 512 32 512 32 512 32 144`. That is a candidate mechanism for the Floor's
"stopped draining its endpoint" lockup, which arrives two or three units in and cares nothing for
the blob. Fixed in `Session::write_preset`. [hypothesis for the lockup — the framing itself is
[solid]; a Floor run confirms or kills the connection]

### A refused *stream* looks exactly like a refused *edit* [solid]
*2026-08-02, HX Stomp.*

Asking for a setlist the device doesn't have does not produce a malformed stream. It produces a
complete, well-formed 20-byte one carrying the ordinary refusal envelope:

```
00 00 06 00  0c 00 00 00        marker/type/len
83 66 cd 00 03  67 cc ff  68 81 6f fd
   {102: 3, 103: 255, 104: {111: -3}}
```

Same `103: 255` / `104: {111: code}` shape an edit rejection uses, on the browse stream. We were
reporting it as `envelope key 104 is not an array` and, on the preset stream, `envelope key 104
missing or not bytes` — which sent us looking for a decoder bug and is very likely the launch-time
error the field kept reporting. Both readers check `parse_edit_rejection` before blaming themselves.

### Bypass is set-state, not a blind toggle [solid] — resolves prior "open"
The two frames of one tremolo bypass press carry `101 → 59: true` then `101 → 59: false` (wire bytes
`…3b c3` then `…3b c2`). So bypass writes an **explicit bool** at target key 59; it is not a toggle.
(The earlier reverb capture looked counter-only because we hadn't decoded the msgpack bool yet.)

## Structural edits: move / add / split-type [solid] — from the 2026-06-26 Windows captures
Decoded with `tools/pcap-frames.py` + a minimal msgpack reader. All ride the edit channel as the same
`{102:txn, 100:op, 101:target}` envelope. **Key result: blocks are manipulated surgically, not by a
full-preset rewrite** (HX Edit *can* rewrite the whole preset via **op 21** `{110: <~3 KB blob>}`,
seen on one multi-slot drag, but the surgical ops below express every structural change and are what
we implement):

| action | op (100) | target (101) | source capture |
|---|---|---|---|
| **move block** | **43** | `{75: src_slot, 76: dst_slot}` | `one_by_one_move…`, `move_simple_eq_to_parallel_path` |
| **add block** | **39** | `{98: slot, 99: {19: 6, 20: {24: {23:false, 25:model, 26:paired}, 9:1, 10:true}}}` | `add_simple_eq…` |
| **split type** | **40** | `{98: split_slot, 100: {23:false, 25:type_idx, 26:-1}}` — *same op/shape as swap-model* | `cycle_through_split_types` |
| **A/B join mixer** | **30** | plain set-value (`{98:slot, 28:idx, 119:val}`) | `adjust_A_B_level_and_pan_of_join` |

- **Move**: the **destination slot encodes the row** — `{75:2, 76:12}` moved a block from slot 2 to a
  parallel-path slot (row B). HX Edit re-reads (op 23/22) after a move; so do we.
- **Add**: `99` is the new block's spec — node kind 6 = a normal DSP block; `20` is the block content
  (`24` = the `{23,25,26}` model-ref, `10` = enabled). After adding, HX Edit issues `set_value`s for
  the new block's default params. The device fills defaults itself, so a bare add + re-read suffices.
- **Split type** is just **swap-model on the split slot** — no new op needed; the split "models" are
  `Helix.sym` entries (observed indices 256/258/563).
- **Split/join node position** (moving the ⋔/⋉ points along the top row) has **no surgical op** —
  the position is the node holder's **key 13** in the preset data (slot `20` content → sub-map
  15 (split) / 17 (mixer) → `13`; only the model holder carries it, the companion 14/16 sub-map
  doesn't). Written by mutating the preset and sending an **op-21 whole-preset write**; the device
  honors the written value verbatim (edit buffer only). Verified live 2026-07-06 via
  `Session::set_node_pos` (guards: bracket must enclose the occupied B row, split < mixer). [solid]
- **op 78** `{98:slot, 26:0}` precedes moves/add in some captures but **not** in
  `move_simple_eq_to_parallel` — so it's an optional preamble, not required for the op to take effect.
- **The ACK precedes the param rewrite [solid — live 2026-07-20]:** op 40 / op 39 are ACKed once the
  device has taken the new **model reference**, but it rewrites that block's **parameter area** a
  moment later. A read issued straight after the ACK therefore returns the new model's identity
  carrying the *outgoing* model's param values — the decoder names them against the new model's
  `Helix.sym` order (`editor::build_block`), so the chain shows the new block while the param panel
  still shows the one it replaced. Read-backs after a structural edit must **settle**:
  `Session::read_preset_settled(slot)` re-reads until the decoded block stops changing (40 ms apart,
  4 attempts). Comparing decodes needs no per-model knowledge, so it holds for any model pair — a
  param-count check would false-pass whenever two models declare the same number of params.

Builders: `fretwire_protocol::edit::{move_block, add_block}` (byte-exact tests vs the captures);
`Session::{move_block, add_block}`; CLI `move`/`add-block`. Split-type rides existing `swap_model`.

## Device state-pushes: the status channel is a state mirror [solid] — from the 2026-06-26 captures
When something changes **on the pedal** (footswitch, panel knob, snapshot/preset switch), the device
**pushes unsolicited frames on the status channel** (`f003 → host`), interleaved with the idle
keepalives. Shape: `{105: type, 106: payload}`. `type` (key 105) selects the notification; the change
is usually nested under an inner key `106`. Decoded:

| change | frame | extract |
|---|---|---|
| **bypass** (footswitch/panel) | `{105:49, 106:{82:_,68:_,121:_, 106:{98:slot, 59:enabled}}}` | slot + `enabled` (key 59) |
| **snapshot** | `{105:42 (and 46), 106:{92:index}}` | `92` = new snapshot index |
| **preset** | `{105:4 (and 8), 106:{…, 106:{107:bank, 108:index}}}` | `108` = new preset index |
| **panel parameter** | `{105:30, 106:{82:_,68:_,121:_, 106:{98:slot, 29:true, 26:_, 28:index, 119:value}}}` | slot + param index + value |
| global setting | `{105:22, 106:{…, 106:{118:id, 119:value}}}` | `118`/`119` (mostly global settings) |
| **idle mirror** | `{105:22, 106:{82:0, 68:10, 121:27, 106:nil}}` | none — `StatusPush::Idle`, sent continuously |
| footswitch press | `{105:41, 106:{70:fs_index, 63:bool, 66:int}}` | key 70 = **footswitch**, 63 = new state |
| snapshot committed | `{105:23, 106:{23:0}}` | none — payload is constant |
| block added | `{105:39, 106:{82:1, …, 106:{98:slot, 26:_}}}` | (not decoded further) |

Types 41, 23 and 39 arrive *alongside* pushes we already decode and carry nothing the editor needs:
a footswitch press emits type 49 (the bypass we use) plus a type 41, a snapshot switch emits type 42
and 46 plus a type 23, and a preset change emits type 4 **and** 8 plus a type 39 naming a slot. Left
undecoded deliberately — acting on them would double-apply what the decoded push already says. Note
the type numbers are the **edit op numbers** (30 = set value, 39 = add block, 41 = bypass): the
device mirrors panel actions in the same vocabulary it accepts commands in.

**Type 41's key 70 is the footswitch, not a bitmask.** [solid — 2026-08-02, HX Stomp, four presses]
An earlier reading had key 66 down as a state bitmask; it isn't one — across four presses it went
458496, 13055, 1037, 67840, which no four-block state fits. What *is* legible is key 70. On a preset
whose own layout is FS1 → slot 2 and FS2 → slot 4, pressing FS1 twice and FS2 twice gave:

| type 41 | the type 49 that followed |
|---|---|
| `{70: 0, 63: true,  66: 458496}` | `Bypass { slot: 2, enabled: true }` |
| `{70: 1, 63: true,  66: 13055}`  | `Bypass { slot: 4, enabled: true }` |
| `{70: 1, 63: false, 66: 1037}`   | `Bypass { slot: 4, enabled: false }` |
| `{70: 0, 63: false, 66: 67840}`  | `Bypass { slot: 2, enabled: false }` |

Key 70 tracks the **0-based footswitch** (FS1 → 0, FS2 → 1) and key 63 the state the press produced,
matching the type-49 mirror every time. That is the live FS → block mapping, which is what an
"assign block to a footswitch" feature would need.

**Key 66 is the footswitch's LED ring colour, `0xRRGGBB`** [solid — 2026-08-22, see "The footswitch
record" below]. It reads as a state bitmask only until you look at it as a colour:

| | engaged | bypassed |
|---|---|---|
| slot 2 | `458496` = `0x06FF00` | `67840` = `0x010900` |
| slot 4 | `13055` = `0x0032FF` | `1037` = `0x00040D` |

Same hue, roughly a sixteenth of the brightness — a lit ring and an unlit one. Slot 2 is green and
slot 4 blue, which is Helix's own category colouring. So the push is not just telling us the block's
new state, it is telling us **what the pedal's ring now looks like**, which is the one thing about a
press that a preset re-read cannot reproduce.

**Type 23 rides every snapshot switch**, exactly once, always `{23: 0}` — seven switches, seven
copies, no variation. Ordering is fixed: type 42, then a type 49 bypass mirror per changed block,
then type 23, then type 46. A constant payload says nothing about its meaning, so "snapshot
committed" is a guess from position alone. [solid on the shape and the ordering; the name is
[hypothesis]]
[observed 2026-08-02, HX Stomp]

**A panel parameter change (type 30) is the op-30 edit reflected back.** Its payload is the *same*
`{98: slot, 28: param index, 119: value}` triple `edit::set_value` sends, under the same op number —
the device mirrors panel edits in the vocabulary it accepts them in. Identified by sweeping the
Drive knob of a `HD2_AmpUSPrincess` in slot 5 and watching fifteen pushes arrive with slot 5,
index 0 and a descending f32. That is what lets the GUI follow a knob without re-reading the preset:
the push carries the value, so it is applied in place, exactly like a bypass mirror.
[solid — 2026-08-02, HX Stomp, `fretwire watch`; byte-exact test]

### The device's push window must be paged, or the channel goes dead [solid]

The device mirrors panel activity only until ~4 KiB of it is unacknowledged, then stops. Measured
four times before the cause was found:

| capture | mirror frames | bytes delivered before silence | wall clock |
|---|---:|---:|---:|
| idle + interaction | 179 | **4075** | 44.6 s |
| scripted interaction | 191 | **4075** | 44.9 s |
| interaction, `arg` advanced | 195 | **4075** | 47.5 s |
| knob sweeps every ~25 s | 386 | 4040 | 29.7 s |

Different frame counts, the same total — and 4075 + 21 = 4096, the body of the next frame it
declined to send. Afterwards the channel carries only empty `cmd 16` keepalives; footswitches, knobs
and preset changes stop reaching the host until the session is reopened. A session that stays *idle*
never reaches the ceiling (2037 bytes in 75 s) and keeps pushing to the end, which is why this hid
for so long: it only bites a session someone is actually using.

**The window is re-opened the same way a paged read pulls its next chunk** — a `cmd 0x08` on the
channel, carrying the offset advanced by the bytes just received. The device's own `arg` on these
frames stays pinned (at 521) exactly as it does mid-read, which is what suggested it.
`Session::poll_events` now sends one per tick on the status channel whenever bytes arrived.

| | mirror frames | bytes | pushes span |
|---|---:|---:|---|
| without the page request | 179–386 | 4075 | died at 30–47 s |
| **with it** | **1117** | **23457** | **4.3 s → 299.9 s (whole run)** |

Status channel only: that is where the pushes live and where every measurement was taken, and it
keeps the extra frame off the edit channel. In the verifying capture all 501 requests went to the
status channel anyway — the other two never leave unacknowledged bytes here, because their reads
consume and acknowledge their own frames.

**Refuted on the way:** advancing the idle beat's `arg` without sending the page request. It changes
nothing (4075 → 4040, inside the noise), so the ack is the request, not the cursor.
[solid — 2026-08-02, HX Stomp, `fretwire watch`]

Type 22 also arrives continuously while nothing is happening (`{82:0, 68:10, 121:27, 106:nil}`, 154
identical copies in a two-minute idle capture, 100 in 30 seconds of an untouched pedal). It must
never be read as a change — but it isn't "undecoded" either, and filing it there had a cost: it was
**100% of a debug session's push log**, ~3.3 lines a second once the push window is paged properly,
which would have buried the write-stall evidence in the next field log. It parses to
`StatusPush::Idle` now, distinct from `Other`, so logging the genuinely undecoded pushes stays worth
doing. `fretwire watch` counts them instead of printing them, and still reports the count — a live
channel going quiet is exactly what the ~4 KiB stall looked like, so "0 idle" and "75 idle" mean
very different things. The idle copy and the carrying one differ in their **outer** keys, which
looks like a sub-type: idle is `68:10, 121:27, 106:nil`, while a real notification is
`68:9, 121:25, 106:{118:id, 119:value}`. In a 90-second session, 492 idle copies and 4 carrying
`{118: 21, 119: 0..3}` — 1, 2 and 3 in that order while the tester was working the pedal's footswitch
modes before switching snapshots, which makes "id 21 = footswitch mode" a tempting read and an
unverified one; nobody recorded what was actually pressed. [2026-08-02, HX Stomp]

Observed live: switching a snapshot pushes the new index **and** a `type 49` bypass mirror for each
block the snapshot changed; HX Edit then re-reads the preset. So our editor: parse these into typed
events and (a) apply bypass mirrors in place, (b) on a snapshot/preset push, re-read to catch the
block/param changes. Parser: `fretwire_data::stream::parse_status_push` → `StatusPush` (byte-exact tests);
collected by `Session::poll_events` (same heartbeat as `keepalive`, but returns the pushes); the GUI
applies them on its tick (live-follow), no manual Refresh needed. **`fretwire watch`** holds a
session and prints the pushes as you touch the pedal, and `FRETWIRE_TRACE_STATUS=1` logs every
status frame body decoded or not — the pair that identified type 30.

**Coalesce the pushes before reacting.** A single preset change emits a *flurry* — the preset push,
then the snapshot and per-block bypass mirrors as the new preset settles — spread over roughly a
second, and the heartbeat delivers them in 250 ms batches. Reading on every batch cost about three
full preset streams plus a preset-list re-read per knob turn (~530 KB across 21 preset changes in
the 2026-07-26 Floor session), aimed at a unit that was still reconfiguring both DSPs; it stopped
responding twice. The GUI now waits for ~300 ms of quiet (capped at 1.2 s) and reads once. Bypass
pushes are applied in place and trigger **no** read at all — the push fully describes the change,
and the device's readable stream lags its own push anyway.

## Reading a preset without loading it — op 4 [solid — verified live 2026-08-21]

`{102: txn, 100: 4, 101: {107: bank, 108: slot, 101: 2}}` streams the document stored in a slot and
**does not select it**: the panel stays on whatever preset it was showing and the edit buffer keeps
any uncommitted change. Same channel, same prologue (op 254 browse-open, op 0 presets-open) and the
same chunked stream as the preset *listing* (op 1) — only the target differs, by naming a slot
alongside the bank. Built by `fretwire_protocol::edit::read_preset_at`, driven by
`Session::read_preset_at`.

**The document is byte-identical to a select-and-read of the same slot.** Reading bank 0 slot 19
both ways gives streams that differ only in the envelope's stream-kind marker (`e0 00` → `e0 28`)
and the echoed txn; `diff-stream` reports no differences in the preset tree.

This is what a setlist backup should use. `select` + read per slot has to load and settle each
preset, which is why a full sweep took tens of minutes — and it walks the user's pedal through all
128. **Measured on an HX Stomp: 126 presets in 10.7 s**, against a sweep that had to be given a
several-hundred-second timeout.

### An empty answer is not an error [solid]

Some slots answer `{102: txn, 103: 0, 104: nil}` — seventeen bytes, status **0**, no document. It has
to be told apart from a desynced read, because both fail to parse and only one is worth retrying:
`fretwire_data::stream::is_empty_slot_reply` does that, and the backup drops to select-and-read for
those slots rather than lose them. Fixture: `captures/empty_slot_reply.msgpack.bin`.

That shape is not op 4's own — **op 36 answers an unassigned parameter with exactly the same
`{102, 103: 0, 104: nil}`** (2026-08-22). It is the device's general "nothing here", so read it as an
answer, not a fault.

**Corrected 2026-08-22.** This was first written up as one odd slot (bank 0 slot 102), from a sample
of five neighbours. Enumerating the whole setlist found **twelve** of 126 — 102, 105, 108-111, 113,
114, 116, 121, 122, 124 — and a thirteenth, 117, that answered nil in one sweep and streamed normally
in the next. Every one is an empty `New Preset`; **no preset with a block in it has ever answered
nil**, across three full sweeps. Cold single-slot reads reproduce each one, so it is a property of
the slot rather than of sweep position or device fatigue.

What separates them is visible in the documents, once you have both: the nil slots differ from their
working same-size neighbours in **three bytes and nothing else**, all `false` where the working ones
hold `nil`, at `/10/10[N]/2[0][2]` for each of the three snapshots. That path is a snapshot's
remembered value for controller entry 0 — assigning a parameter writes the value there and removing
the assignment leaves it behind (see "Controller assignments" below). So the nil-answering slots are
the ones whose snapshot controller array was initialised `false` rather than left empty. Twelve of
twelve match; the flaky slot 117 also carries `false`, which is consistent with it being the marginal
case. **Whether that is the cause or merely a co-symptom is not established**, and nothing yet
explains why the firmware would decline to stream such a document. Correctness is unaffected either
way: the fallback reads them.

### Op 4 is unverified outside the Stomp

The backup assumes op 4 works, and clears the assumption on the first refusal — falling back to the
old sweep for the whole job rather than re-asking 128 times. That is there for the **Helix Floor**,
where nothing about this opcode has been tried.

Its inverse (**op 5** / **op 8**, write a document into a slot; **op 16**, empty one) is deliberately
not built: those are persistent writes and want their own captures first. See `docs/safety.md`.

## Controller assignments — writing them [solid — verified live on an HX Stomp 2026-08-22]

Reading was solved on 2026-08-21 (see `docs/preset-format.md`). Writing is seven opcodes, all of them
edit-buffer commands like any other block edit: they survive a save and nothing else.

**There are two mechanisms, and conflating them is the trap.** A block's **bypass** on a footswitch
and a **parameter** under a controller are stored in different places and written by different
opcodes.

| Op | Target | Does |
|---|---|---|
| 56 | `{98: slot, 102: switch}` | put a block's bypass on a footswitch |
| 57 | `{98: slot, 102: switch}` | take it off again |
| 37 | `{98: slot, 26: paired, 28: param, 29: true, 74: source, 71: 4, 129: false}` | put a parameter under a controller |
| 65 | `{98, 26, 28, 29: true, 119: value}` | that assignment's **Min** |
| 66 | same | its **Max** |
| 33 | `{102: switch}` | read a footswitch's configuration |
| 36 | `{98, 26, 28, 29: true}` | read a parameter's assignment |

The opcode numbers and shapes are `tonepush`'s, from a macOS HX Edit capture we do not have; each row
above was then sent to an HX Stomp and checked against the document it changed. Builders:
`fretwire_protocol::edit::{assign_bypass_to_switch, unassign_bypass_from_switch, assign_param,
set_assign_travel, read_switch, read_assignment}`.

### The footswitch number is one-based to read and zero-based to write [solid]

Op 33 takes Footswitch 1 as `1` and **answers `102: 0`**; ops 56 and 57 take the same switch as `0`.
Confirmed both ways: asking 33 for 1, 2, 3 answered 0, 1, 2, and `assign_bypass_to_switch(16, 0)`
landed on the layout's first position. The CLI exposes the wire numbering rather than papering over
it — `read-switch 1` and `assign-bypass 16 0` are the same switch.

### A bypass goes to the layout; a parameter goes to both [solid]

Sending **op 56** for slot 16 on a preset with nothing bound changed exactly one path in the
document:

```
/3/8[0]: nil -> [{10: 7, 11: {0: 1, 5: "Simple Delay\0", 6: 458496, 7: true, 8: 16}, ...}]
```

The controller table at key `4` was untouched — which independently confirms, by construction, what
the front-panel diff had already shown: **a footswitch bypass never enters key 4.** Op 57 put the
document back byte-for-byte.

Sending **op 37** for the same block's Mix (param 2) to source 3 changed nine paths, and the two that
matter are:

```
/4[3]:   nil -> [{0: 3, 1: 4, 2: 0, 3: 1, 4: 0, 5: 16, 6: {28: 0, 29: 2, 41: false}, 7: 0, 13: false}]
/3/8[0]: nil -> [{10: 0, 11: {0: 2, 5: "Mix\0", 6: 462860, 7: false, 8: 16, 2: 0, 9: {...}}, ...}]
```

So a **parameter** assignment appears in the footswitch layout *as well as* the controller table,
distinguished by the layout node's kind (`11 → 0`): **1 for a DSP block's bypass, 2 for a parameter
controller**, where key 5 is then the parameter's name rather than a model's. `loaded_blocks` already
filters kind 2 out of its footswitch enrichment, so a block with only a knob on FS1 is correctly not
badged as being on FS1 — that filter was written from a fixture and is now proven by construction.

The entry landed at `/4[3]` — **index 3, the FS1 ordinal** our own front-panel diff had established,
which is a second and independent confirmation of the source-ordinal indexing.

`tonepush`'s full list — 0 none, 1-2 expression pedals, 3-7 footswitches, 8 MIDI, 9 snapshots — is a
**five-switch device's** version of that space, and taking it as the format's cost us: an HX Stomp
XL has eight switches, a 13-entry table, and puts **FS6 at ordinal 8**. The run always starts at 3;
where it ends, and therefore where MIDI and snapshots sit, is `footswitches + 5` long.
See `docs/preset-format.md` for the table. [corrected 2026-08-25, issue #13]

### There is no separate "unassign parameter" opcode [solid]

Op 37 with `74: 0` is the removal. It leaves one thing behind: the snapshot's remembered value for
that controller entry (`/10/10[N]/2[0][2]`) keeps the number it was given instead of returning to
`false`. Everything else — the key-4 entry, the layout entry, the counter at `/10/8` — reverts. The
residue is harmless, and it is the same field the op-4 nil-slot correlation above turns on.

### Key 1 of a controller entry is the MIDI CC number

Our own samples carried both `0` and `4` here, and "parameter vs bypass" was **refuted** on
2026-08-21. `tonepush` reads it as the CC number, constant `4` under any source that has no CC to
give. The op-37 write is consistent: assigning to FS1 stored `1: 4`. That is corroboration, not
proof — telling the CC reading apart from a "value type" reading needs a MIDI-sourced sample, which a
Stomp cannot make on its own. `[hypothesis]`, but now the better-supported one.

### Op 36's reply is the document's own entry [solid]

`{102: txn, 103: 0, 104: <the same map key 4 stores>}`, and `104: nil` when nothing drives the
parameter. Since `PresetStream::assignments` already decodes that map from a document, op 36 is a
cross-check rather than a second decoder — useful for confirming a write landed without re-reading
the whole preset.

### Op 33's reply, on a switch with nothing customised [solid — live HX Stomp, 2026-08-22]

    FS1 -> {102: 0, 65: false, 109: nil, 66: nil, 67: nil}
    FS2 -> {102: 1, 65: false, 109: nil, 66: nil, 67: nil}
    FS3 -> {102: 2, 65: false, 109: nil, 66: nil, 67: nil}

Key `102` is the switch index, **zero-based**, so the one-based argument the CLI takes is ours and
not the wire's — that much is `[solid]`, confirmed across three switches.

`65` is the switch type — `true` momentary, `false` latching — mirroring layout-entry key `12`:
writing that key through op 21 flipped `65` to `true` and back on the next op-33 read, and the
switch then behaved momentary under a foot [solid — live HX Stomp 2026-09-03, `fretwire switch-type`]. `109` label and `66` colour were
settled the same way on 2026-08-27 (below). `67` is the switch's bindings. Before any of that
was measured, all four were `[hypothesis]`, and the note that follows is from then: `109`
carries a *name* elsewhere in this protocol (it is the IR name), which is a suggestive analogy and
not an observation. **An HX Stomp cannot set a custom switch label from its own panel** — that is an
HX Edit feature — so if `109` is the label, this device may never move it on its own, and the key
would have to be confirmed by writing it.

### The footswitch record, decoded [solid — live HX Stomp, 2026-08-22]

Binding a block to a switch and re-reading named three of the four unknowns in one step.

    FS1, Simple Delay on it:
    {102: 0, 65: false, 109: "Simple Delay\0", 66: nil,
     67: [{59: true, 68: 1, 66: 458496,
           69: {109: "Simple Delay\0", 98: 16, 29: false, 26: 0, 28: 0, 120: {56: 0, 51: 0}}}]}

| key | meaning | evidence |
|---|---|---|
| `102` | switch index, **zero-based** | constant across FS1-3 |
| `109` | the switch's **label** | `nil` → `"Simple Delay\0"` when a block was bound |
| `67` | the **assignments array** | `nil` → one entry naming slot 16, the bound block |
| `67[].66` | the assignment's **LED colour**, `0xRRGGBB` | see below |
| `67[].69.98` | the target **slot** | `16`, and `5` for a block at slot 5 |
| `67[].59` | enabled | `true` on a fresh bind |
| `65`, `66` (top level), `68`, `26`, `120` | unknown | never moved |

**The colour was proved by binding a second block of a different category.** A delay came back
`0x06FF00` and an amp `0xFF0003` — Helix's green and red. Two categories, two hues, one gesture
apart; the earlier "66 is a bitmask" reading in the status-push section is refuted by the same
finding.

### Where the label and the override colour live — and why op 56 never fills them [solid — live HX Stomp, 2026-08-27]

Op 33's `109` and top-level `66` are not device state: they mirror **two value-plus-gate pairs in
the preset document's layout entry** — `14` the label string gated by `13`, `16` the colour gated by
`15`. Proved by writing them: flipping `13` alone (a one-byte patch pushed through the op-21 write
path) turned `109` from `nil` into the `"\0"` key 14 held; putting `"C"` in `14` came back as
`109: "C\0"` (the firmware supplies the terminator); `16: 127` surfaced as `66: 127` only once `15`
was true. The mirroring is verbatim and the gates decide — a stale string behind a false gate is
invisible, which is the same rule `footswitch_layout` already applied to `14`/`13`.

That settles the earlier "`109` is set by the pedal and not by our op 56" puzzle: **the pedal never
fills the label in on its own** — not with time, not on an op-76 re-read, not on a snapshot change,
and not across a save and a reload from flash. The panel's bind gesture writes `13: true` +
`14: <block name>` itself; op 56 writes the virgin `false`/`"\0"` pair. So op 56 is not missing a
follow-up opcode — the difference between a panel bind and ours is exactly those two document keys,
and either can be produced deliberately.

Consequence: **custom footswitch labels and colours are writable today** through the document
(op-21) path, and `hxb-convert` carries a tone's `@fs_customlabel` and `@fs_customcolor` into
`14`/`13` and `16`/`15`. Ops 58-62 remain interesting only as the *incremental* edit route HX Edit
presumably uses. `16`'s unit is the **palette index**, not RGB [solid — writing 1 through 10 and
watching the ring gave ten distinct hues, where raw RGB 1–10 would all be near-black]. The index
space is the `footswitchLED` enum in Line 6's own `HelixControls.json` — 0 Auto Color, 1 White,
2 Red, 3 Dark Orange, 4 Light Orange, 5 Yellow, 6 Green, 7 Turquoise, 8 Blue, 9 Violet, 10 Pink,
11 Off — whose ordinals match both the wire values and every observed ring colour.
The same sweep showed the ring and scribble **repaint immediately** on an op-21 write — the custom
label and colour are fully live-editable, and `Session::{set_switch_label,set_switch_color}` (CLI
`switch-label` / `switch-color`, and the ✎ editor in the GUI's block panel) do exactly that.

### Not built yet

Ops **58-62** — momentary/latching, custom switch label, LED colour — are documented by `tonepush`
and untried here. Since 2026-08-27 they are also *unnecessary for storage*: the label and colour
live in the document (see above) and can be written through the op-21 path, and since 2026-09-03
so can the switch type (layout key `12`, `Session::set_switch_momentary`, CLI `switch-type`, the
Latching / Momentary pair in the GUI's switch editor), so these ops matter only as the
incremental edit route. Op **64** sets a *parameter's* MIDI CC, which is a different
mechanism from a bypass's (that rides op 37 with `95: 5`). None of them are needed for the
assignment itself.

## The user IR store — reading and writing it [solid — verified live on an HX Stomp 2026-08-22]

The 128 user impulse-response slots, the one device capability HX Edit had entirely to itself. All
of it rides the **browse envelope** (TLV opcode `0x02` `SESSION_OPEN`), not the `0x06` envelope
every block edit uses, and every transaction sits between an **op 255** open and an **op 254**
close. Decoded from `captures/{import,export}_ir.pcapng`; the builders in
`fretwire_protocol::edit` are byte-exact against those captures, and the whole family has now been
driven against a live pedal.

| action | op (100) | target (101) | reply |
|---|---|---|---|
| **session begin** | **255** | `{}` | `104: nil` |
| **upload** | **9** | `{112:slot, 113:checksum, 109:name, 114,115,123,124,125, 110:blob}` | `103: 1`, then a status push |
| **stream a blob** | **11** | `{112:slot, 101:2}` | the 8192-byte blob, paged |
| **select a slot** | **12** | `{112:slot}` | that slot's whole metadata record |
| **commit / list** | **13** | `{101:2}` | the directory, as an array of records |
| **rename** | **10** | `{112:slot, 109:name}` | status |
| **delete** | **15** | `{112:slot}` | status |
| **session end** | **254** | `{}` | `104: nil` |

**Two listings, and they answer with different fields.** Op **13** returns the whole directory in
one request, each entry carrying the slot's name and the **MD5 of its stored bytes** — but no
checksum and no length. Op **12** answers per slot with the checksum and the declared length but no
MD5, so enumerating that way costs 128 round trips and is only worth it when the *empty* slots
matter. `Session::ir_directory` is the first, `Session::ir_scan` the second.

### An IR is 2048 samples, stored verbatim [solid]
The blob is **8192 bytes = 2048 little-endian `f32`**, mono, at the device's 48 kHz. No header, no
rate, no channel count — the length is the format. And the device stores what it is given: a blob
read back off slot 0 is **byte-identical** to the one `import_ir.pcapng` recorded HX Edit uploading
in June, and a slot written from that file reads back byte-identical again. Nothing is resampled,
normalised or trimmed on the way in or out, so an IR round-trips bit-exact.

### The `113` checksum is a word sum, not a CRC [solid]
The blob read as 2048 little-endian `u32` words, summed and truncated to 32 bits:

```python
checksum = sum(struct.unpack("<2048I", blob)) & 0xffffffff   # 0xc0a076ed
```

crc32, crc32-inverted, adler32, byte-sum, big-endian sum and xor were all checked against the
capture and all differ. `fretwire_protocol::edit::ir_checksum`. The read path verifies it on every
export, so a torn reassembly is an error rather than a file that is quietly a few samples short.

### `114`/`115` declare the stored length [solid] — corrects two prior readings
The device stores **`114 x 256 x 2^115`** samples. With the multiplier pinned at 1, the exponent
alone selects 256, 512, 1024 or 2048:

| `114` | `115` | stored |
|---|---|---|
| 1 | 0 | 256 |
| 1 | 1 | 512 |
| 1 | 2 | 1024 |
| 1 | 3 | 2048 |
| 0 | – | nothing — an empty slot |

Two earlier readings here were wrong, and both for the same reason: every sample this project had
came from a **populated** slot, where the pair is a constant `1, 3`.

1. They were written up as constant "format flags". They are a length.
2. Then, after an empty slot turned up reporting `0, 1`, `114` was written up as an **occupancy
   flag**. It is not — the zero is a length of zero. `IrSlot::is_used` now asks whether the stored
   length is non-zero, which gives the same answer for a better reason.

A third reading died with them. Writing a record with `114: 0` under an empty name looked like a
free delete, and when the device answered `1, 3` anyway that was written up as "the device
maintains these and ignores the caller". Also wrong: they *are* caller input, and the device was
correcting an invalid declaration against the 2048 samples actually sent. The real delete is op 15.

**This field is a hazard, not a decoration.** Data shorter than the declared length is zero-padded
and harmless. Data **longer** than it wedges the device's transfer state machine badly enough to
need the power pulled. `edit::ir_length_code` derives the pair from the sample count and
`edit::ir_upload` refuses anything that is not a stored length, so a caller cannot state one that
disagrees with what it sends. [the table is `tonepush`'s, measured by content hash; the `1, 3` and
`0, 1` rows are confirmed on this device]

`123/124/125` are `false, false, 0` on every slot, empty or full — and are not IR-specific, since
preset list entries carry the same trio. Still unmapped.

### Key `104` is the MD5 of the stored bytes [solid]
Each op-13 directory entry carries a 33-byte `104`: the MD5 of the slot's sample bytes *after* the
device's zero-padding, lowercase hex plus a NUL. Confirmed against this device — a slot holding
2048 known samples reports the same digest Python's `hashlib` gives for those bytes.

That makes verifying an upload free and much stronger than the `113` word sum, which any reordering
of the samples collides with. `Session::ir_upload` checks both.

### Writing a slot is a flash write, and the reply arrives twice [solid]
Op 9 is too big for one frame (8259 bytes of TLV) and goes out on the **same paced bulk transfer as
the op-21 preset write** — 512 payload bytes per credit, each unit split 496 + 16. Both captures
that carry a bulk upload are one of each, which is why `Session::send_chunked_tlv` is one
implementation serving both.

Its immediate reply is `103: **1**`, not the usual `0`, and the real completion lands afterwards as
a **status-channel push** echoing the same transaction. So the ack is not the verdict — `ir_upload`
re-reads the slot and compares checksums instead.

Unlike every other edit in this project, this one **does not live in the edit buffer**: there is no
reload that undoes it. `ir_upload` refuses an occupied slot unless told to overwrite.

### Delete and rename [solid — verified live]
**Op 15** `{112: slot}` empties a slot. Afterwards it reports field-for-field identically to a slot
that has never been written — same zero length, same empty name, same flags. Emptying an already
empty slot is not an error.

**Op 10** `{112: slot, 109: name}` renames, taking the same 32-byte NUL-padded field an upload
carries. The stored MD5 is unchanged across a rename, which is what confirms it touches the name
and not the samples.

Both are small commands, both write flash, and both are followed by op 13 here to refresh the
directory. An earlier probe of op 10 drew `-3` only because it was sent as `{112: slot}` with no
name — the opcode was right and the target was short.

### Still not decoded: reorder, and how a block points at a slot
Two gaps are left in this family.

**Reorder.** Moving an IR between slots has never been captured, and may not exist as an opcode at
all — a reorder is expressible as delete plus upload.

**How an IR *block* references a user slot** rather than a built-in cab IR. Everything above is the
store; nothing here says what a preset's IR block puts in its model reference to name slot *n*. It
is the last piece between reading the store and editing a preset that uses it, and one capture of
assigning a user IR to an IR block answers it. Related: a `tone` IR block is `@type` 5 and carries a
`@uuid`, and the preset's `irUuidTable` maps slot number → uuid, so the host side addresses IRs by
uuid where the store addresses them by index.

## Favorites — list, read, save [solid — from `favorite_add_delete_backup.pcapng`, HX Stomp fw 3.80, 2026-09-04]
The store HX Edit's model picker shows as the **Favorites** category (id 23 in `HX_ModelCatalog.json`).
It is not in the op-24 settings namespace and not in any preset — a `backup-device` before and after
saving one on the owner's Stomp differed in nothing but Stomp Mode. It has its own three ops on the
**browse side** (`PRI` channel, same as the preset listing), plus one on the edit channel:

| op | channel | target | reply |
|---|---|---|---|
| **112** list | PRI, `cmd 0x0c` | `nil` | `[{118: index, 64: model, 105: paired cab or 65535, 109: name}]` |
| **113** read | PRI, `cmd 0x0c` | `{118: index}` | the record, below |
| **119** save | EDIT, `cmd 0x04` | `{98: block slot, 118: index, 31: true, 109: name}` | `nil`; then a **state push type 56** on the status channel carrying the new list entry |
| **45** block-as-record | EDIT, `cmd 0x0c` | `{98: block slot}` | `{13: slot, 24: record}` — the block in favorite form; HX Edit asks it right after a save |

`64`/`105`/`25`/`26` are **`Helix.sym` indices**: 591 = `HD2_AmpUSPrincess`, 709 =
`HD2_CabMicIr_1x12USDeluxe`, 636 = `VIC_DynPlateStereo`, checked against the file. HX Edit lists
(op 112) once at connect, between the preset listing (op 1) and the IR directory (op 13).

The record (op 113's `64`, op 45's `24`):
```
{19: 6, 28: <index>,
 20: {24: {23: <composite>, 25: <model>, 26: <paired cab, or -1>},
      9:  33 for the amp, 8 for the reverb — not the catalog category (Amp is 11, Reverb 6); unexplained
      10: true,
      11: {2: <values>, 3: <of which sym-listed>, 4: [values…]},      // the block
      12: {2: 7, 3: 7, 4: [Mic, Position, Distance, Angle, LowCut, HighCut, Level]}}}   // the paired cab, or {2:0,3:0,4:[]}
```
The value arrays are in the model's **`Helix.sym` parameter order** — the US Princess's twelve are
Drive, Bass, Mid, Treble, Presence, ChVol, Master, Sag, Hum, Ripple, Bias, BiasX exactly — with
the **switch parameters that the sym list omits appended after it as bools** (the Dynamic Plate's
`@trails` is the twelfth of `2: 12, 3: 11`). The cab's `IrData` is not carried. `{23, 25, 26}` is
the same triple op 109 sends as `{106, 64, 105}` (below). `35`/`32`/`33` came back nil.

Adding a favorite from fretwire is `add_block` with its model and cab, then one typed op 30 per
sym-listed value — typed, because a favorite carries ints for enum params (a cab's Mic) and the
device refuses a float there with -3 [live 2026-09-04: the first attempt did exactly that]. Verified
live: both favorites land with every value equal to the record.

Save (op 119) takes a **block in the current preset**, not a record: HX Edit saved slot 6 as index 1
named "Dynamic Plate". Whether an index that exists is overwritten or refused, and what `31: true`
means, was not exercised. **No delete was captured** — the capture's name says add+delete but only the
add is in it, so the delete op is still unknown. Neither is rename.

In HX Edit's `.hxb` each favorite is its own section, `F000`, `F001`, … (hex, like the IR slots,
only the ones that exist), `L6ModelFavorite` JSON in the `.hlx` tone dialect: `data.favorite.slot0`
is the block (`@model`, `@type`, `@enabled`, then params by name — the same values as the wire record),
`slot1` the paired cab for an amp, `data.meta.name` the name. `fretwire_data::hxb::Hxb::favorites`
reads them; `show-backup` lists them.

## User Defaults — op 109, one ask per (model, composite, cab kind) [solid — capture + live, 2026-09-04]
`op 109 {64: model, 106: composite, 105: cab kind}` on the browse side, reply `104: nil`. HX Edit
sends it **1162 times during a backup** — exactly the row count of the `.hxb`'s `UMDS`
(`L6UMDArchive`, "user model defaults") table for this Stomp, with 359 composite rows matching the
359 calls that carry `105`. `105` is not the paired cab: it is **48** (`HD2_Cab1x12Lead80`, the first
legacy cab) or **687** (`HD2_CabMicIr_2x12JazzRivet`, the first of the mic'd-cab range 687–829) — the
*kind* of cab the composite pairs, so an amp is asked three ways: alone, with a legacy cab, with a mic'd
one. It is also sent **once when a block is selected** (op 78 on slot 5 → op 109 for that block's model).
Every reply in the capture is nil, and the pedal had no user default saved. **Then the owner saved
one** (the US Princess block, ACTION → user default, on the pedal) and `fretwire probe-browse` asked
again the same evening:

| ask | reply |
|---|---|
| `{64: 591, 106: false}` | nil |
| `{64: 591, 106: true, 105: 48}` | nil |
| `{64: 591, 106: true, 105: 687}` | the record — `{19: 6, 28: 0, 20: {24: {23: true, 25: 591, 26: 709}, 9: 33, 10: true, 11: {…12 values}, 12: {…7 cab values}}}` |

So **op 109 reads the user default, nil means none is saved, and the key is the triple**: the block was
an amp with a mic'd cab when it was saved, and only that form of the model holds it. The record is the
same shape as a favorite's (op 113's `64`, with the cab under `12`). The backup's UMDS table is that
sweep written out, and a bare manifest on both donors because there was nothing to carry; what a row
with a default looks like in the file is still unseen. Not seen either: how one is written or cleared.
**The pedal applies a user default itself when a block is added over USB** [solid — live
2026-09-04]: with the US Princess default saved, `add_block` (op 39, no values in the spec, then the
cab swap) came back at Drive 0.42, Bass 0.21 … and the cab at Mic 2, Position 0.42 — the saved
record, not the factory values. So an editor need do nothing for user defaults beyond backing them
up. Also worth noting: op 39's add spec `{19: 6, 20: {24: {23, 25, 26}, 9, 10}}` is the favorite
record's top level minus the value lists `11`/`12` — the same structure with the values left out.
Whether op 39 accepts a spec *with* `11`/`12` (a favorite added in one op) is untested.
[hypothesis]

HX Edit's sweep is slow — 1162 asks over 37 s, against 1.95 s for the 126-preset op-4 sweep — but
that is its pacing, not the pedal's: `backup-device` asks 1414 (every `Helix.sym` model alone, every
amp and preamp with each cab kind, every cab with the legacy kind — a superset, and an ask the
device has no slot for answers nil, checked live) in about 3 s, one browse session.

## All global settings in one read — op 85 [solid — same capture]
`op 85 {}` on the edit channel answers a 724-byte map: `{0: [{150: v}…{164: v}], 1: [{190: v}…{203: v}],
2: [{0: v}…{136: v}], 3: [{0: v}…{10: v}], 4: nil}` — five groups in the order the `.hxb` writes `GLOB`
(**DSP, EQ, System, Tuner, L6Link**), each a list of one-entry `{id: value}` maps. HX Edit asks it
before the backup's IR read. Group 2's ids are the op-24 ids we already scan (19 Stomp Mode, 27
PresetNumbering, 86 tuner reference 440.0 …), and so are DSP (150–164) and EQ (190–203): a
`backup-device` from the same evening holds every id op 85 answered with a value, and lacks exactly
the eleven op 85 answered nil (150–152, 155, 159–164, 201). Same coverage. What op 85 adds is the
**grouping**, which the flat namespace cannot express, and one round trip in place of 166.

## Resolved vs. still open
- [x] Endpoints / framing / channels / sequence.
- [x] Value encoding = **big-endian f32**.
- [x] **Edit body is MessagePack** (`fretwire_protocol::edit::EditBody`). Envelope `{102: u16 counter,
      100: op, 101: target}`; **block slot at target key 98**; value at target key 119.
- [x] **★ Parameter editing is computable** — the param is selected by target key 28 = its index in
      the model's `Helix.sym` device order (verified across 4 models / 6 params). Builders
      `edit::set_value()` + `fretwire_core::EditorBlock::set_param_by_name()` generate byte-exact edits.
- [x] Op = envelope key 100 (41 bypass, 30 set-value). Key 102 is a whole u16 counter.
- [x] **Bypass = explicit bool at target key 59** (set-state, not a toggle).
- [x] Block id = preset slot index (verified vs `preset1_stream.msgpack.bin`).
- [ ] Switch/transport params (`@trails`, tempo-sync note) decode with key 28 = 0 — a different
      addressing, not yet decoded. Continuous/knob params follow the index rule above.
- [x] **Handshake / channel setup** (`startup.pcapng`): per-channel SESSION_OPEN
      (`00100000`→`00020000`) in order ef03→ed03→f003, then an identity query per channel
      (opcode 5/6/4 → `"P33Main"`/`"P33"` + ver `0x03800000`), then meters + preset stream.
- [x] **Frame header fully resolved** (validated byte-exact vs real frames, `tests/real_frames.rs`):
      `len` = 8 + significant-body; offset 12–15 `arg` is a u32 **per-channel running stream
      offset** (idle frames on a channel share it; paged chunk reads advance it by 256) — **not a
      checksum**. No per-packet checksum exists.

## Reading the current preset — MessagePack stream [solid: verified live on Linux 2026-06-23]
The MessagePack envelope keys 100/101 are an **operation + target**, and op 20 is **not** a read:

| op (key 100) | target (key 101) | meaning |
|---|---|---|
| **20** | `{107: bank, 108: preset}` | **SELECT PRESET** — loads it; **changes device state** |
| **76** | `{}` | open the current edit buffer for a (non-destructive) read |
| **24** | `{118: id}` | **read a device setting** — reply carries the value at key `119`. The connect sequence sends `{118: 128}`, which is why this was written up for months as a "read-sequence prepare step"; it is not one. See *Device settings* above |
| **23** | reply `{107:bank, 108:index, 109:name, 92:snapshot, 117:?, 83:[u32,0]}` | read-info: current preset identity **and the live active snapshot** (key 92 — the authority; the preset blob's own `10 → 8` is the *stored* one and can differ). [solid] |
| **23** | `nil` | read-sequence query — **reply carries the current preset identity** (see below) |
| **22** | `nil` | start the paged stream — reply = chunk #0 |

**Op 23 = current preset identity [solid: decoded from `startup.pcapng`].** The read-info reply
envelope is `{102:txn, 103:_, 104:{107:bank, 108:index, 109:name, 117:bool, …}}` — key **108 is the
setlist index** (matches op 20 / the preset list) and **109 is the name**. In the startup capture:
`{107:0, 108:20, 109:"Dual Amp"}`, and index 20 in the primary preset-list reply is indeed "Dual
Amp". This is how the host learns which preset is loaded at connect (the streamed preset blob itself
carries **no** index/name). Parsed by `fretwire_data::stream::parse_preset_info`; surfaced as
`EditorPreset::current` (filled by `Session::read_preset`). The same reply also rides every
`read_preset`, so a post-navigation read reports the device's authoritative current preset for free.

> **The op-23 identity lags the blob by one preset change [solid].** The first read after the preset
> changes serves the **new** preset's stream under the **previous** preset's identity; the next read
> reports both consistently. Evidence — the tester's Helix Floor session of 2026-07-26: 19 of the 21
> distinct stream lengths in that log were reported under exactly two *consecutive* identities, and
> in every case the later of the two is the one all subsequent stable reads keep (e.g. a 7233-byte
> stream reported first as `DUSTED` (index 53) and then three times as `BMBLFOOT PRINCE` (index 67),
> while `DUSTED`'s own stream is 7297 bytes).
>
> This matters beyond the displayed name: the **live snapshot rides the same reply** (key 92), so an
> uncorrected read paints the previous preset's active snapshot. `read_preset_inner` therefore
> re-issues op 23 **after** the stream and reports whether the identity moved across the read; when
> it did, the blob can't be attributed to either preset and the caller re-reads. Asking again
> afterwards leaves the proven open/prep/info/stream sequence untouched.
>
> That check catches the identity moving *across* a read, but not the other shape: an identity that
> is uniformly stale on both sides of the stream. Nothing in the reply can — only the address
> `goto_preset` asked for. `Session` remembers it (`expect_identity`) and discharges it in
> `read_preset_confirmed`, which every caller that must not misattribute a blob goes through:
> `read_preset_raw` (the read-modify-write input) and `backup_setlist` (which had been reading it
> raw, and aborted whole sweeps on the lag).

> **A browse row's position is the slot; its map key is not [solid — corrected 2026-08-19].** The
> listing arrives in slot order, always has, and needs no sorting: `parse_preset_list` numbers rows
> by their position in the stream and `Session::list_presets_in` hands them over as they came.
>
> Each row's MessagePack **map key** is a different number — the preset's index *before* it was last
> reordered on the pedal, globally numbered as `bank × setlist_size + slot` (a TEMPLATES listing on a
> Floor starts at 896 = 7 × 128). **No command accepts it as an address**: a preset's own identity
> (key 108), `goto_preset` and `save_preset` all take the bank-relative slot. Passing a global index
> through as a slot is what reached the device as `goto_preset(7, 906)` and locked it up.
>
> An earlier draft of this block read the disagreement the other way — "the entries do not arrive in
> slot order" — and had `list_presets_in` sort by the key. That took a correct list and shuffled it
> into the device's pre-reorder order. What settles it: the tester's 2026-07-29 dump of all eight
> banks (1024 entries) matches that unit's own `.hxb` backup **position**-for-position, and not at
> all by key; the three rows where the two numbers disagree are exactly his three moved presets. The
> full argument, and what the key probably is, are in `docs/helix-floor.md`.

`open_two_presets_one_after_another.pcapng` is HX Edit **selecting** presets (op 20) — that's why an
earlier draft mistook op 20 for "open for read". The **non-destructive read** is what HX Edit does on
connect, decoded from `startup.pcapng` (`tools/pcap-frames.py`), on the edit channel (`8010/03ed`):
1. OUT op 0x06, `cmd=0x04` — body `read_open` (op 76, target `{}`): open the edit buffer.
2. OUT op 0x06, `cmd=0x0C` ×3 — `read_prep` (op 24), `read_info` (op 23), `stream_start` (op 22).
   The op-22 reply carries chunk #0; the op-23 reply carries the preset name (e.g. `"PrincesSM7"`).
3. OUT `cmd=0x08` (len 16) ×N — request subsequent chunks; each reply is a **272-byte** frame
   (16-byte header + 256 payload). A reply `< 272` bytes signals end of stream.

> **The envelope's declared length is the authority, and it is checked in both directions [solid].**
> The reassembler keeps reading until `declared_stream_len` is satisfied rather than stopping at the
> first short chunk, and then refuses the blob if it did not land on that length. Under-length means
> the device stopped answering mid-stream. **Over-length means a frame that was not stream payload
> got spliced in** — a non-empty reply in a chunk slot is appended sight unseen, and everything
> after it shifts. Eight bytes (one stream prefix) of overshoot on an HX Stomp decoded into a preset
> the pedal did not have, whose phantom block then drew `-3` from the device when it was edited
> (2026-08-20). One trailing pad byte is tolerated because one capture really carries one — and only
> one: four of the five tracked captures reassemble to exactly their declared length, which is what
> keeps the tolerance at a byte rather than somewhere wide enough to re-admit a splice.
>
> **The browse listing is checked the same way** (2026-08-21). It shares the guard, because a
> listing that ran long parses just as readily as a preset does — into the wrong rows. A remote
> report of "rows 009-015 missing from the sidebar, 008 blank" was one of these, and browse
> positions are what `goto` addresses, so a bad listing is an addressing fault, not a cosmetic one.
>
> **A failed stream is retried, not surfaced** (2026-08-21). Both of these failures are transient —
> back off, drain, re-read and they clear. Every read path now does that; until 2026-08-21 the
> confirmed read used by the setlist export did not, so a single splice ended a sweep that had
> already read ninety presets correctly.

**The `arg` offset is a per-channel running sum of received body lengths.** Edit-channel base after
the handshake = `0x1009`; it then advances by each reply's body length (`+76` after read-open's reply,
… `+256` per stream chunk). Stamp each OUT frame with the current value. `fretwire_core::Session` tracks
this in an `arg: HashMap<channel, u32>` (`edit_request`). Builders: `fretwire_protocol::edit::{read_open,
read_prep,read_info,stream_start,select_preset}`.

**Payload is MessagePack.** Reassembling preset 1's chunks (`captures/preset1_stream.msgpack.bin`,
2804 bytes, `payload = chunk[16..]` concatenated) yields recognizable content: `"l6-helix"`,
version `"v3.71-32-g1039661"` (`0xb2` fixstr), block model names (`Bucket Brigade`,
`Harmonic Tremolo`, `Tremolo`, `70s Chorus`, `Dynamic Hall`), and `SNAPSHOT 1..N`.
The msgpack root is not at offset 0 (a few stream-envelope bytes lead it); locate it by marker.

**Correction (earlier draft was wrong):** the per-param wire handles are **NOT** carried in this
stream. Searched `preset1_stream.msgpack.bin` for the captured handle bytes (`82 62 04 3b`,
`85 62 04 1d`, `82 62 07 3b`, and the `62 [slot]` pair generally) → **zero occurrences**. What the
stream's *envelope* does carry is the edit **context root** `83 66 cd 04 …` (once, at byte 8) —
the per-preset/session context, not a per-param map.

So **handle discovery is by construction, not by lookup**: block id = preset slot index (which we
parse), and the parameter selector is a small constant tuple per param (bypass = `(0x82,0x3b)`;
others TBD via captures). The editor targets a block from its slot and edits with op 0x06
(+ BE f32); no handle table is streamed.
