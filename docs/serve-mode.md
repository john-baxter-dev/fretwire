# Serve mode — the editor over HTTP, for headless machines

Status: **planned, not started** (opened 2026-08-23). No code yet. This doc is the survey that
should stop the next person re-deriving it.

It covers **two** requested features, because they turn out to rest on the same refactor: serving
the editor over HTTP to a headless machine, and exposing it over MCP to an LLM client. The lift
described in §1 is the shared foundation; see [A second consumer: MCP](#a-second-consumer-mcp).

## What was actually asked for

A Helix Floor owner on Reddit (2026-08-23) runs a **Raspberry Pi 5 inserted between path 1 and 2**
of the Floor, doing NAM captures with PiPedal over the same USB connection that carries audio — no
extra D/A–A/D conversion, no added latency. Every Linux machine they own is headless. What they
asked for, in their words, was a Raspberry Pi build; what they described wanting was *"running an
editor on that same machine that I can access from my laptop on the same network"*.

**Those are different things, and building the first does not deliver the second.** An
`aarch64` `.deb` of the Tauri GUI opens a WebKitGTK window on a machine with no display. The
request is for **serve mode**: `fretwire serve` on the Pi, browser on the laptop.

This reframing is the main finding. The arm64 build is downstream of it and much smaller —
see [Phase 8 in `ROADMAP.md`](../ROADMAP.md).

## Why this is closer than it looks

Three things already in the tree do most of the load-bearing work.

### 1. The UI already has a transport seam, with two implementations

`crates/fretwire-tauri/ui/src/lib/ipc.js` is the single point where the frontend reaches the
backend. Everything imports `invoke`/`listen` from there rather than from `@tauri-apps/api`
directly, and it already dispatches between **two** backends by sniffing for a Tauri runtime:

```js
export const IS_MOCK = !(typeof window !== "undefined" && "__TAURI_INTERNALS__" in window);
export const invoke = IS_MOCK ? mock.invoke : tauriInvoke;
export const listen = IS_MOCK ? mock.listen : tauriListen;
```

A third transport — HTTP to a daemon — is an insertion at exactly this file. Nothing else in the
UI has to know.

### 2. The UI is already proven in a plain remote browser

`npm run dev` serves the whole editor against the in-memory mock backend, in an ordinary browser,
with no Tauri and no Rust. That is the same rendering path a laptop browser would use against the
Pi. The unknown is the transport, not the UI.

### 3. The backend coupling to Tauri is thin

Measured 2026-08-23 against `crates/fretwire-tauri/src/commands.rs` (1434 lines, 61 commands):

| coupling | count | what it becomes |
|---|---|---|
| `State<'_, AppState>` | 55 | `&AppState` — mechanical |
| `tauri::AppHandle` | 2 | an event-sink trait or an mpsc channel |
| `app.emit(...)` sites | 3 | see below |

The entire event surface is three names, all consumed in `App.svelte`:

- `device-pushes` — device-originated changes (footswitch bypass, panel snapshot/preset switch)
- `device-lost` — the heartbeat gave up after `LOST_AFTER_BEATS` consecutive failures
- `backup-progress` — export sweep progress

61 commands is a big number but a shallow one. They are already documented as *"thin wrappers over
`fretwire_core::Session`"*, and they already run their blocking USB work off-thread via
`spawn_blocking`, which a server needs too.

## The work

### 1. Lift the command layer out of `fretwire-tauri`

Move `commands.rs` + `dto.rs` into a transport-neutral crate. `fretwire-tauri` keeps 61 one-line
`#[tauri::command]` wrappers over it; the server becomes a second consumer of the same layer — and
an MCP server would be a third, which is the strongest argument for doing this lift at all.

This is the bulk of the diff and almost all of it is mechanical. The two non-mechanical parts:

- `AppState` holds `Arc<Mutex<Option<Session>>>` plus the clipboards and the `cancel_export`
  flag. That struct moves as-is; only its *owner* changes.
- `spawn_heartbeat(app: tauri::AppHandle, ...)` takes the `AppHandle` purely to emit. It needs the
  event sink instead. Keep its behaviour exactly — the `LOST_AFTER_BEATS` giving-up logic and the
  poll-under-lock-then-release-before-emitting ordering both exist for reasons recorded in its
  doc comment, and a server has the same failure mode.

### 2. A `fretwire-serve` binary

- static file serving for the built `dist/`
- `POST /invoke/<command>` carrying the JSON args, mirroring Tauri's shape
- a WebSocket for the three events

**Keep it a separate crate, out of `default-members`,** exactly as `fretwire-tauri` is. Embedding
`dist/` into `fretwire-cli` would make a built frontend a prerequisite for `cargo build`, which is
the thing the workspace layout deliberately avoids (see the `default-members` comment in the root
`Cargo.toml`). With `dist/` embedded via `rust-embed`/`include_dir`, the deliverable is a single
static binary: `scp` it to the Pi and run it. No WebKitGTK, no Tauri, none of the Pi GPU risk.

### 3. The file-picker problem — the only real design work

`ipc.js` exposes `pickPath()`, which opens a **native** dialog (Tauri) or prompts for a typed path
(mock). Over a network neither is right: the paths that matter are on the *server*, and three
flows depend on them.

- first-run data import — locating an HX Edit installer or `res` folder
- backup export / restore
- IR export

Needs a small server-side directory browser. Typing absolute paths blind is a bad first-run
experience and first run is exactly when a new user is least able to guess what to type.

### 4. Auth, before it binds to anything but loopback

This grants write access to someone's guitar rig. Minimum bar:

- default to `127.0.0.1`; going wider is an explicit flag
- **check the `Origin` header** — a local HTTP server without one is reachable by any web page the
  laptop visits, via DNS rebinding, regardless of firewalls
- a token for the non-loopback case

## A second consumer: MCP

Asked for the same day (2026-08-23), independently: *"Would you consider adding MCP support? That
would enable all sorts of AI-assisted fun. CLI seems like it might be inefficient for that purpose
but I've not looked that closely."*

**Verdict: worth doing, offline-first, after the lift — not before.** Two unrelated requests landing
on the same refactor within a day of each other is the best evidence available that §1 is the right
shape.

### They are right about the CLI, and the reason is measurable

Every live CLI subcommand — about sixty of them — opens its own session:

```rust
let mut s = fretwire_core::Session::connect()?;
```

One handshake and one teardown per process invocation, and nothing carries across: not the goto
cursor, not the edit buffer. (`fretwire connect` is a *diagnostic* that connects, prints, and hangs
up — there is no daemon today.) An agent making forty parameter tweaks would pay forty handshakes.

An MCP server is a long-lived process, which fits `Session` and its 250 ms heartbeat **better** than
the CLI does. The instinct in the request is correct.

### Do not expose 61 tools

The command surface is GUI-shaped: the DTOs exist to *render*, not to reason over. Sixty-one tool
descriptions is a lot of context spent before the model does anything, and a `PresetDto` carrying
full parameter lists is bulky to hand back on every call. This wants a curated dozen or so, with
summarized views, rather than a mechanical translation of the existing surface.

### Offline first — most of the interesting part needs no pedal

This is the sequencing that matters. Explaining a preset, generating one, tone-matching, batch
renaming, diffing two — none of it requires the device. All of it runs against backup JSON plus the
catalog, and every piece is **already implemented offline**: `Catalog::load_preset`, and the
`backup-show` / `show-preset` / `tree` / `diff-stream` CLI commands.

Offline means zero device risk, no hardware needed, testable in CI, and useful to people who don't
own a unit. The live half is then a thin "push this to the device" step on top of it. Start there.

### Safety is a real constraint here, not a formality

`STATUS.md` records an incident on 2026-08-22 where a careful human probing op 58 wedged the pedal.
Writes are persistent (op-21 edit-buffer write + op-71 save), and restoring a *foreign* blob is
still tagged `[hypothesis]`. An LLM tool surface is a machine for doing the wrong thing confidently
and quickly.

So, as hard requirements rather than defaults to revisit:

- read-only tools by default; anything that writes is behind an explicit opt-in
- edit-buffer writes before persistent saves, so the escape hatch stays a power cycle
- back up before a write session
- **firmware / flash / bootloader / DFU never appears in the tool surface**, per `docs/safety.md` —
  it isn't reachable today and must not become reachable by way of a generic tool bridge

### Available today, with no code from us

`fretwire backup <out.json>` then pointing an agent at the file gets the offline half immediately —
`backup-show` is offline and the format is documented. Worth telling the requester, both because
it's a real answer and because it tests whether the use case survives contact with reality before
anyone builds a server for it.

### Open questions for MCP

- **What does "AI-assisted fun" mean concretely?** The answer decides offline vs. live, and the
  guess here is that it is mostly offline. Ask before scoping.
- **Catalog names reaching a cloud model.** Model and parameter *names* would go out in tool
  results. That is the user's own imported data, names only, nominative use — no data files leave
  the machine, and it is the same information the GUI already puts on screen. Judged fine, but
  recorded deliberately rather than by omission, because this project is careful about that line.

## Two deployment facts specific to this setup

### The udev rule does not work headless

`packaging/70-hxstomp.rules` grants access with `TAG+="uaccess"`, which is a *seat* mechanism:
systemd-logind grants the locally-seated user. **A Pi reached over SSH has no local session, so
`uaccess` grants nothing** and the daemon gets `EACCES`. The failure is silent about its cause and
a user has no chance of guessing it.

Serve mode needs a `GROUP=`-based variant of the rule. `install-udev` in
`crates/fretwire-cli/src/main.rs` embeds the canonical `packaging/` copy at build time and would
need the same choice. Note `UDEV_RULE.contains(r#"TAG+="uaccess""#)` is asserted in a test there.

### PiPedal coexistence should be fine — but confirm it

They are using the Helix as a USB **audio** interface at the same time. That is a different USB
interface number from the vendor control interface fretwire claims, and `fretwire-usb` already
handles a driver holding it:

```rust
// On Linux nothing should hold this vendor interface, but detach defensively if it does.
let iface = match dev.claim_interface(CONTROL_INTERFACE) {
    Ok(i) => i,
    Err(_) => dev.detach_and_claim_interface(CONTROL_INTERFACE)?,
};
```

So the mechanism is there. But `snd-usb-audio` binding the MIDI interface of the same device while
fretwire drives the control interface has **never been tested here**, and detaching the wrong
interface out from under a live audio path is not a friendly failure. Verify on hardware before
telling anyone it works.

## Their device is already supported

Worth saying plainly to the requester: the **Helix Floor** (PID `0x4248`) is in `DEVICES` and its
udev rule is marked *"verified: byte-identical handshake and edit path"* — see
[`docs/helix-floor.md`](helix-floor.md). The gap for them is headless access, not device support.

The standing Floor caveat still applies and is unrelated to any of this: grid/routing planning is
still DSP-0 only, and the routing view assumes one DSP × 2 rows. They said second-DSP routing is
critical for them. See Phase 9 in `ROADMAP.md` — reading and per-block edits are already
DSP-agnostic; the planning layer and the grid UI are not.

## Verified while surveying this

Cross-compilation of the CLI, on 2026-08-23, with no code changes:

```
cargo check --locked --target aarch64-unknown-linux-musl     -p fretwire-cli   ✅
cargo check --locked --target armv7-unknown-linux-musleabihf -p fretwire-cli   ✅
```

`nusb` 0.1.14 is pure Rust over usbfs with no `target_arch` gating outside a wasm branch, and
nothing in our crates is architecture-specific. This is a *check*, not a link, and no binary has
ever been run on ARM hardware.

## Open questions for serve mode

- **Does the heartbeat survive a slow network client?** It beats every 250 ms holding the session
  lock. That is tuned for a local webview; a WebSocket to a laptop is a different latency regime.
- **What happens with two browsers open?** Tauri has exactly one frontend by construction. A
  server does not, and `AppState`'s clipboards and undo history are single-editor assumptions.
  Simplest honest answer for v1: refuse a second session.
- **Should `fretwire-serve` and the GUI ship together?** They would share the lifted command layer
  but have separate binaries and separate packaging.
- **32-bit Pi OS.** `armv7` checks clean, but ROADMAP Phase 8 deliberately excludes it as an
  untestable support burden. Serve mode weakens that argument — a headless armv7 Pi *can* usefully
  run a server even if it can't usefully run the editor. Not decided.
