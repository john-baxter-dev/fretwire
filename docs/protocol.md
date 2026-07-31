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
  **`"P33"`** — plus the firmware version `0x03800000` (matches preset key 35). `P33` = HX Stomp.
- After setup, the **edit channel** (`ed03/8010`) runs meter/state queries and then streams the
  **current preset** (frame 4272 IN begins `da 0a … 6c 36 2d 68 65 6c 69 78` = "l6-helix") — the
  same paged MessagePack mechanism as "Preset open" below.

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

## Global / input parameters use a different op [hypothesis] — from `switch_input_gate_and_guitar_pad.pcapng`
Block params address a slot (target key 98). **Global/input settings** (input gate, guitar pad) are
*not* slot-addressed. Two shapes seen in that capture:
- A **block switch** on slot 0 (Input): `{102: txn, 100: 30 (set), 101: {98:0, 29:true, 26:0, 28:0, 119:<bool>}}`
  — confirms the documented edge case that **switch params carry key 28 = 0** and a bool at key 119.
- A **global setting**: `{102: txn, 100: 25, 101: {118: <id>, 119: <value>}}` — a **new op (25)** with
  the target keyed by `{118: id, 119: value}` (e.g. id `134`), with no block slot. Decoding the key-118
  id space (which global maps to which id) is an open thread.

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
  is a constant descriptor on knob edits.
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
| `-21` | op 39 `add_block` carrying a `paired_index` | a paired model-ref is not accepted by add; pair with op 40 afterwards |
| `-3` | op 30 `set_value` writing a split node's `bypass` param | that parameter is not writable this way — bypass has its own op (41) |

The `-3` case is worth knowing about because `bypass` **is** a real entry in the split's stored param
array, and the four split models are the only ones in the whole catalog (4 of 681) that carry it. So
the editor will happily offer it as a knob; `Session::set_param` now recognises it and sends op 41
instead. [solid — 2026-07-31, two op-30 writes to a Split Y's bypass, both refused with `-3`]

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
| param/setting | `{105:22, 106:{…, 106:{118:id, 119:value}}}` | `118`/`119` (mostly global settings) |
| footswitch/scene state | `{105:41, 106:{70:_, 63:bool, 66:int}}` | (not decoded further) |

Observed live: switching a snapshot pushes the new index **and** a `type 49` bypass mirror for each
block the snapshot changed; HX Edit then re-reads the preset. So our editor: parse these into typed
events and (a) apply bypass mirrors in place, (b) on a snapshot/preset push, re-read to catch the
block/param changes. Parser: `fretwire_data::stream::parse_status_push` → `StatusPush` (byte-exact tests);
collected by `Session::poll_events` (same heartbeat as `keepalive`, but returns the pushes); the GUI
applies them on its tick (live-follow), no manual Refresh needed.

**Coalesce the pushes before reacting.** A single preset change emits a *flurry* — the preset push,
then the snapshot and per-block bypass mirrors as the new preset settles — spread over roughly a
second, and the heartbeat delivers them in 250 ms batches. Reading on every batch cost about three
full preset streams plus a preset-list re-read per knob turn (~530 KB across 21 preset changes in
the 2026-07-26 Floor session), aimed at a unit that was still reconfiguring both DSPs; it stopped
responding twice. The GUI now waits for ~300 ms of quiet (capped at 1.2 s) and reads once. Bypass
pushes are applied in place and trigger **no** read at all — the push fully describes the change,
and the device's readable stream lags its own push anyway.

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
| **24** | `{118: 128}` | read-sequence prepare (purpose TBD; replicated from capture) |
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
> it did, the blob can't be attributed to either preset and `Session::read_preset` re-reads. Asking
> again afterwards leaves the proven open/prep/info/stream sequence untouched.

> **The browse listing is numbered globally and is *not* sorted [solid].** A listing reply numbers
> presets `bank × setlist_size + slot` (a TEMPLATES listing on a Floor starts at 896 = 7 × 128),
> whereas a preset's own identity (key 108), `goto_preset` and `save_preset` all use the
> bank-relative slot — passing a global index through as a slot is what reached the device as
> `goto_preset(7, 906)` and locked it up.
>
> Separately, the entries **do not arrive in slot order**. A preset the user has *moved* keeps its
> old position in the stream while carrying its new index. In the tester's 2026-07-29 dump of all
> eight banks (1024 entries), bank 0 emits slot 68 at stream position 101 and bank 1 emits slot 95
> at position 84; the other six banks are strictly ascending, which is why it went unnoticed.
> `Session::list_presets_in` normalises to slots **and** sorts, since callers render the array
> positionally.
>
> Those same 1024 entries otherwise match the unit's own `.hxb` backup slot-for-slot — the three
> exceptions are exactly the three moved presets — which is what finally closed the "browse index
> drift" question as device state rather than a parser bug. See `docs/helix-floor.md`.

`open_two_presets_one_after_another.pcapng` is HX Edit **selecting** presets (op 20) — that's why an
earlier draft mistook op 20 for "open for read". The **non-destructive read** is what HX Edit does on
connect, decoded from `startup.pcapng` (`tools/pcap-frames.py`), on the edit channel (`8010/03ed`):
1. OUT op 0x06, `cmd=0x04` — body `read_open` (op 76, target `{}`): open the edit buffer.
2. OUT op 0x06, `cmd=0x0C` ×3 — `read_prep` (op 24), `read_info` (op 23), `stream_start` (op 22).
   The op-22 reply carries chunk #0; the op-23 reply carries the preset name (e.g. `"PrincesSM7"`).
3. OUT `cmd=0x08` (len 16) ×N — request subsequent chunks; each reply is a **272-byte** frame
   (16-byte header + 256 payload). A reply `< 272` bytes signals end of stream.

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
