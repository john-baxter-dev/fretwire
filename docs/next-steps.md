# Next steps — from offline basecamp to live HX Stomp control

Status (2026-06-22): the wire protocol is decoded and the offline model + tooling are built and
tested (`STATUS.md`). Nothing talks to a real device yet. This doc is the concrete path forward,
in priority order. **Track 1 needs no Linux and no new code — do it first.**

---

## Track 1 — Crack the parameter-map ✅ DONE (2026-06-22)

**Result: best case (H2).** A parameter is selected on the wire by its **index in the model's
`Helix.sym` device order** (edit target key 28). Verified across 4 models / 6 params — see
`captures/param_map_findings.md`. **Parameter editing is computable from shipped data; no per-param
captures needed.** `fretwire_protocol::edit::set_value()` and `fretwire_core::EditorBlock::set_param_by_name()`
build byte-exact set-value commands today. Only edge case left: switch/transport params (`@trails`,
tempo-sync) use a different addressing (key 28 = 0) — minor.

This was the pivotal de-risking experiment, and it came out the good way. The remaining gate is
purely the **live transport (Track 2)** — which needs Linux + the pedal.

<details><summary>Original experiment plan (for reference / to extend to switch params)</summary>

The pivotal question was whether the param key generalizes. It does (key 28 = param index). To
extend coverage to switch/enum params, capture a few of those and decode with `fretwire decode-edit`,
comparing key 28 / key 119 type against the model's param list.
</details>

### The hypotheses to distinguish
- **H2 (best):** the value-key / `100` tag is a function of the **parameter index** in the model's
  param list → param addressing is *computable from shipped data*; almost no further captures needed.
- **H1 (good):** key is per-parameter but **model-independent** (Mix is always 119) → capture once
  per distinct parameter; many are shared across models.
- **H3 (hard):** keys are arbitrary per (model, param) → many more captures needed.

### Procedure
1. Launch HX Edit (Windows) with the HX Stomp connected; start USBPcap on its root hub (see
   `ROADMAP.md` Phase 1 / the existing captures for the filter).
2. Make **one parameter change per capture**, and record it using `captures/_TEMPLATE.md`. Suggested
   set (chosen to test the hypotheses):
   - **Same block, several params** — Harmonic Tremolo: `Speed`, `Intensity`, `Level`, `Mix`
     (model indices 0, 1, 6, 7). Reveals whether tag/key tracks param index.
   - **Same param, different models** — `Mix` on Bucket Brigade, on Dynamic Hall, on 70s Chorus.
     Reveals whether a given param's key is model-independent.
   - **A switch/enum param** (e.g. a Wave Shape) and a **frequency param** — to see non-float encodings.
3. Extract the edit body bytes from each capture with `tools/dump-control.ps1 <pcap>` (the `data`
   after the 8-byte TLV header: `01 00 06 00 <ilen u32> …`).
4. Decode each with the tool we built:
   ```
   cargo run -p fretwire-cli -- decode-edit <hex of the body>
   ```
   Record the `prop tag` (key 100), the `value` key, and the slot for each.

### What to look for
- Tabulate `(param name, model index, tag 100, value key)`. If tag or key == a simple function of
  the model index (or the device `Helix.sym` index), **H2 holds** — wire it into `fretwire-core` and
  parameter editing is essentially solved offline.
- If `Mix` is `119` on every model → **H1**; start a `param → key` table.
- Capture the descriptor keys too (Mix carried `29/26/28`) — see if they're constant or vary.

Feed results back into `docs/protocol.md` (the "envelope key 100 = property/param id" open item) and,
if H1/H2, extend `fretwire_protocol::edit` with a `set_value(slot, param, value)` builder.

---

## Track 2 — Linux bring-up — READ PATH DONE (2026-06-23)

`detect → connect → pull` all work against real hardware on Linux. `pull` reads the loaded preset
**non-destructively** (decoded to named blocks, repeatable). The **write path (`bypass`/`set`) is the
remaining live work** — see the bottom of this section. **Windows can't run these** — the Line 6
driver owns the vendor interface; Linux's generic stack lets `nusb` claim it (after a udev rule).

### What first contact taught us (all resolved)
- **udev:** the raw USB node needs `70-hxstomp.rules` with **lower-case** `ATTR{idVendor}=="0e41"`,
  `ATTR{idProduct}=="4246"`, `TAG+="uaccess"`. Default systemd `uaccess` only tags `/dev/snd/*`, not
  the vendor interface, so without the rule you get `EACCES`.
- **Response matching:** a reply matches a request by **channel (`dst == our src`) + seq**. The status
  channel `f003` streams unsolicited meters that interleave and must be skipped; one bulk-IN transfer
  can also concatenate several frames. `fretwire_usb::Transport::request` now handles both.
- **`arg` (frame offset 12):** a per-channel running counter = **sum of received body lengths** on the
  channel. Edit-channel base after the handshake is `0x1009`; the stream advances it `+256`/chunk.
- **The read is op 76, NOT op 20.** op `100:20` `{107:bank,108:preset}` is **SELECT PRESET** (it
  changes the active preset — the old `read_preset` was doing this). The real, non-destructive read
  sequence (from `startup.pcapng`, via `tools/pcap-frames.py`) is on the edit channel:
  `cmd04 op76 {}` → `cmd0c op24 {118:128}` → `cmd0c op23 nil` → `cmd0c op22 nil` (stream-start) →
  `cmd08`×N pagination, short read = end. Builders: `fretwire_protocol::edit::{read_open,read_prep,
  read_info,stream_start}`.

### Setup
- udev rule for non-root access (else run with `sudo`):
  `SUBSYSTEM=="usb", ATTR{idVendor}=="0e41", ATTR{idProduct}=="4246", MODE="0660", TAG+="uaccess"`
  in `/etc/udev/rules.d/70-hxstomp.rules`, then `sudo udevadm control --reload && sudo udevadm trigger`.
- Build: `cargo build -p fretwire-cli`. Turn on frame logging for everything below: `export RUST_LOG=trace`.

### Bring-up sequence (each step gates the next)
1. **Enumerate:** `cargo run -p fretwire-cli -- detect` → "HX Stomp: present".
2. **Connect / handshake:** `cargo run -p fretwire-cli -- connect`. Watch the `trace` log: you should see
   the 5 handshake frames go out and replies come back (the 2nd reply contains `"P33…"`). *This is
   first contact — expect to iterate here.* If it hangs on the first `bulk_in`, the device wants a
   different first frame; compare the trace to `startup.pcapng` via `tools/dump-control.ps1`.
3. **Read a preset:** `cargo run -p fretwire-cli -- pull`. If `Session::read_preset` reassembles and
   `print_preset` shows the right blocks, the **read path is proven end-to-end**.
4. **Write — bypass:** `cargo run -p fretwire-cli -- bypass 4 on` (use a real slot from step 3). Watch the
   pedal's screen toggle. **Proof of life for the write path.**
5. **Write — parameter:** `cargo run -p fretwire-cli -- set <slot> <param-idx> <value>` (param-idx is the
   index shown in `pull`'s output, e.g. Mix). Watch the value change on the pedal.

### Remaining live work
Connect, read, and write are **working on hardware** (handshake → `"P33Main"`; non-destructive
preset read; `bypass`/`set` ACK-confirmed and verified by read-back, arg-accounted via
`edit_request`). What's left:
- **Teardown — DONE (verified live 2026-06-25):** `Session::close()`/`Drop` send the session-close
  on each channel (status → edit → primary) — the panel-lock fix. The close must be request/response
  (read each ack) with a ~150 ms settle before the interface is released; firing blind doesn't work.
  primary never acks (handshake diverges there) and the panel releases anyway, so the ack wait is
  capped at 300 ms. `fretwire disconnect` confirms the pedal returns to standalone, fast.
- **Test writes safely:** writes change device state. Use a scratch preset, back it up, watch the
  pedal. If the device ever stops acking edits on the edit channel, the matching loop spins to
  `MAX_SKIP` — switch that path to `send_frame` (fire-and-forget) if needed.
- **Validate amp/cab block resolution:** identity now comes from the `24 → 25` `Helix.sym` index
  (verified vs shipped data, not yet vs a live amp/cab read). **`fretwire pull` a preset containing an
  amp + cab block** and confirm `symbolic_id`/`variant`/paired-cab resolve correctly. Same read
  `dump-raw`'d gives more `11 → 6` samples — the only way to crack the category encoding for the 11
  amp/preamp pairs (a UI nicety now, not an identity blocker; listed in `docs/preset-format.md`).
  `fretwire dump-raw a.bin` on two presets that differ only by amp vs preamp, then `diff-stream`, isolates
  the `11 → 6` value.
- Diff live frames against the captures with `tools/pcap-frames.py <capture>.pcapng` (Linux; no
  tshark needed). The newest Windows captures enumerate at **USB address 9** (not 8).
- **New protocol threads** (latest Windows captures): the session teardown (`cmd=0x02`, empty body)
  and global/input settings via **op 25** `{118: id, 119: value}` (not block-slot addressed) — both
  documented in `docs/protocol.md`.

### Safety
**Out of scope / brick risk:** never send firmware/flash/DFU traffic. Back up presets first. See
`docs/safety.md`.

---

## Track 3 — Feature parity, then GUI (later)

After read + edit work: snapshots (preset key 2), controller assignments (key 4), model-swap,
preset save-to-device, tuner. Each is its own protocol sub-area, mostly undecoded — decode from
captures the same way. The GUI (toolkit TBD) rides on top of `fretwire-core` once the session API is real.

## TL;DR
1. ~~Crack the param-map~~ ✅ **done** — editing is computable (param = index, key 28).
2. ~~Write the transport/session~~ ✅ **built** (`Transport`, `Session`, CLI live commands) — compiles.
3. **The one remaining gate: reboot to Linux, plug in the pedal, run the Track 2 runbook above**
   (`detect` → `connect` → `pull` → `bypass` → `set`). First-contact debugging, not RE.
3. Everything else (snapshots, controllers, tuner, GUI) follows.
