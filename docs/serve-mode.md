# Serve mode — the editor over HTTP, for headless machines

Status: **serve mode works on loopback** (2026-09-01; a live browser-to-pedal session confirmed the same day) and **files cross the seam as bytes** (2026-09-02) — the §1 lift landed 2026-08-31
(`fretwire-commands`), and `fretwire-serve` (§2) plus the ipc.js HTTP transport are built and
verified end-to-end; IRs and exports cross the seam as bytes (§3). A bind beyond loopback takes a
bearer token (§4, 2026-09-02; plain HTTP on a trusted network, SSH tunnel otherwise). Nice-to-
haves left: the server-side directory browser (§3). Opened 2026-08-23; this doc is the survey that should stop the next person re-deriving it.

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

### 1. Lift the command layer out of `fretwire-tauri` — DONE (2026-08-31)

Moved `commands.rs` + `dto.rs` into the transport-neutral **`fretwire-commands`** crate (a
`default-members` member, so the offline suite covers it). `fretwire-tauri` keeps one-line
`#[tauri::command]` wrappers over it; the server becomes a second consumer of the same layer — and
an MCP server would be a third, which was the strongest argument for doing this lift at all.

By the time it landed the surface had grown to **65** commands (the counts below were measured at
61). As predicted, almost all of it was mechanical. The two non-mechanical parts, as built:

- `AppState` (the `Arc<Mutex<Option<Session>>>`, the clipboards, the `cancel_export` flag) moved
  as-is; only its owner changed.
- The event sink is `fretwire_commands::events::EventSink`, taken as a parameter by
  `spawn_heartbeat` and `export_setlists` (the only two producers). Each event's wire name and
  JSON payload are defined once, in `events::Event`, so transports cannot drift; `fretwire-tauri`
  adapts it with a `TauriSink(AppHandle)` newtype. The heartbeat's behaviour is unchanged — the
  `LOST_AFTER_BEATS` giving-up logic and the poll-under-lock-then-release-before-emitting ordering
  both exist for reasons recorded in its doc comment, and a server has the same failure mode.

### 2. A `fretwire-serve` binary — DONE (2026-09-01)

Built as surveyed, axum + rust-embed:

- static file serving for the built `dist/` (shared with `fretwire-tauri`; embedded in release —
  one static binary — read from disk in debug)
- `POST /invoke/{command}` carrying the JSON args, mirroring Tauri's shape — including the
  **camelCase argument names** the frontend sends (`paramIndex`, `modelIndex`), which Tauri's
  macro normally converts. The dispatcher (`fretwire_commands::dispatch`, an explicit 65-arm
  match) lives in `fretwire-commands` rather than the server so the offline suite covers it on a
  clean clone, and a future MCP server reuses it. Errors cross as plain strings with a 500,
  matching how Tauri rejects an `invoke`.
- a WebSocket at `/events` for the three events, framed `{"event", "payload"}` — the client wraps
  them in the `{event, payload, id}` envelope the handlers expect
- transport selection: the daemon injects `<script>window.__FRETWIRE_SERVE__={}</script>` into
  `index.html`'s `<head>` (inline, so it runs before Vite's deferred module scripts), and
  `ipc.js` routes to `lib/serve.js` when it sees the marker — the same dist runs under Tauri,
  serve, and the `npm run dev` mock
- clean teardown on SIGINT/SIGTERM (the session close the GUI does on exit), and the heartbeat
  spawned at startup exactly as the GUI's setup hook does
- an **idle close** (added 2026-09-01): the session survives editor disconnects — a refresh or a
  Wi-Fi blip must not cost the undo history — but after 5 minutes with no lease holder the daemon
  closes it cleanly, so a closed tab doesn't leave the USB interface claimed all night (an
  unclean host shutdown with a session open is the state that leaves the pedal needing a power
  cycle). Guarded by a lease generation counter so a stale timer never closes a session an
  editor came back to. The page's side of the same coin: on startup the UI asks `is_connected`
  and re-attaches to a live session automatically (`connect` is an idempotent re-read), so a
  reload lands back in the editor — under Tauri and the mock that check is always false and
  nothing changes.

It is a separate crate, out of `default-members`, exactly as `fretwire-tauri` is — a built
frontend must never become a prerequisite for `cargo build` (see the `default-members` comment in
the root `Cargo.toml`). No WebKitGTK, no Tauri, none of the Pi GPU risk.

### 3. Files across the seam — DONE (2026-09-02), one server-path case left by design

Every file command took a `path: String` and did `std::fs` on the *serving* machine. Under Tauri
the distinction never existed (one disk); serve mode made it visible, and for most flows the
user's files are on the laptop, not the Pi. Settled 2026-09-01 and landed 2026-09-02, per flow:

- **IRs — client-side.** An HX IR is 2048 samples (~KB), so the file rides inside the invoke:
  `ir_upload_inline` takes the WAV as base64 plus a name (there is no path to derive one from);
  `ir_export_inline` returns `{name, wav_base64}` and the browser saves it as a download. Same
  parser, same 48 kHz rule, same error text as the path pair — `ir_write`/`ir_wav` are the shared
  bodies in `fretwire-commands`.
- **Backup JSON — client-side restore and export-as-download, server path kept as an option.**
  `export_setlists_inline(banks)` runs the same sweep (same progress events, same cancel) and
  returns `{count, json}`; `backup_show_inline(json)` and `restore_preset_inline(json, …)` take
  the file's text. The daemon keeps no per-page file state, so the restore sends the text back
  along with the choice (a few MB at most — the `/invoke` route's body cap was raised from axum's
  2 MB default to 64 MB for exactly this). The export dialog under serve has a checkbox for
  **saving on the daemon's disk instead** (a backup that lives with the rig, cron-able), which is
  the one place the server-path variant is still reachable from the browser UI. The whole-device
  backup (2026-09-03) follows the same pattern: `backup_device_inline(banks, irs, settings)`
  returns `{count, irs, settings, json}`, `backup_info_inline(json)` the file's counts, and
  `restore_device_inline(json, presets, irs, settings)` the report — a device backup with the IR
  store is a couple of MB more, still well under the cap.
- **Data import — server-side, permanently.** The HX Edit installer is ~a gigabyte and `res/` is
  a folder tree; uploading that through a browser is clunky, and `fretwire import-data` over SSH
  already does it (the CLI and daemon share the data dir). This is the one flow that still reaches
  `pickPath()` under serve (`FirstRun.svelte`): a typed server-side path.

On the UI side `ipc.js` exports `INLINE_FILES` (`IS_SERVE || IS_MOCK`: "the user's files are on
this side of the seam"), `lib/files.js` holds the browser plumbing (`pickFile`, `saveFile`, the
base64 helpers — chunked; the Rust side decodes the standard padded alphabet), and the IR panel
and the two backup dialogs branch on it. The mock backend implements the `_inline` pair for real
(a genuine WAV out, the header read on the way in; export files parse back) so `npm run dev`
exercises the same UI path serve does. Tauri keeps the native picker and the path commands; all
70 commands are registered on every transport so the surface is one set.

The server-side **directory browser** is now a nice-to-have for the data import and the
backup-to-daemon path — typing an absolute path blind is a poor first-run experience, but the
SSH route covers it and nothing else needs it.

### 4. Auth — DONE (2026-09-02, confirmed live on the LAN): a bearer token beyond loopback, no TLS to start

This grants write access to someone's guitar rig. What stands, in layers:

- **Loopback by default, tokenless.** Only local processes reach the port; the supported remote
  path with no setup is an SSH tunnel (`ssh -L 8317:127.0.0.1:8317 <host>`).
- **`Host`/`Origin` always checked** (2026-09-01) — a local HTTP server without that is reachable
  by any web page the laptop visits, via DNS rebinding, regardless of firewalls — plus a
  `Content-Type: application/json` gate on invokes, which forms can't send.
- **A token for anything wider** (2026-09-02). Decided in discussion the same day: the **link is
  the credential** (a fragment, not a login page), and **no TLS in v1**.

How the token works:

- **Generated once**, 32 bytes from `/dev/urandom` as hex, kept in
  `~/.local/share/fretwire/serve-token` (mode 0600, beside the data dir so a data wipe never
  touches it; `--token-file` moves it). `--token` or `FRETWIRE_SERVE_TOKEN` override it — the env
  form is for a systemd unit's `Environment=`. Giving `--token` on loopback demands it there too.
- **Printed at startup** as `http://<host>:8317/#token=…`. The fragment never reaches the server,
  its logs, or a `Referer`; the page reads it once, keeps it in `localStorage` (per origin), and
  strips it from the address bar. With no stored token and the daemon's injected marker saying
  `auth: true`, the page asks for one up front (a paste box) rather than failing on its first
  call; a 401 on an invoke or a **4401** close on the event socket forgets the stored token and
  asks again, so a rotated token never leaves a page silently broken.
- **Carried** as `Authorization: Bearer` on every invoke and as `?token=` on the WebSocket
  handshake (browser JavaScript cannot set headers there; the daemon logs no query strings).
  Compared constant-time. Static assets stay open — the page has to load before it can read the
  fragment, and `index.html` holds nothing secret.
- **`Host` relaxes to "our port"** when a token is configured, and `Origin`, if present, must equal
  `Host`. The daemon cannot know which names a user legitimately types (an IP, `pi.local`, a DNS
  entry), and it doesn't need to: a DNS-rebinding page lands on *its* origin, whose `localStorage`
  holds no token, and gets a 401. The token is the defense; the strict loopback rule stays for a
  tokenless bind.

**No TLS, and why.** A self-signed certificate means browser warnings and certificate management
for every user, and it doesn't stop a hostile network from simply refusing the connection. The
honest statement, printed under the link, is that a LAN bind assumes a **trusted network** (home
Wi-Fi). For anything else: the SSH tunnel, Tailscale/WireGuard (encryption for free), or a reverse
proxy such as Caddy for real certificates. One consequence worth knowing: on a plain-HTTP LAN
address browsers withhold secure-context APIs (`crypto.randomUUID` among them); `serve.js`
already falls back where it matters.

## A second consumer: MCP — landed 2026-09-02 (`crates/fretwire-mcp`)

**What was built.** A stdio MCP server over `fretwire-commands`, on the official `rmcp` SDK.
The surface is exactly the shape argued for below: **14 read tools**, +10 with `--allow-writes`,
+1 (`preset_save`) with `--allow-save`; ungated tools are absent from `tools/list`, not refused.
Results are text, not DTOs — `summary.rs` renders a preset as its blocks in signal order with
values the way HX Edit displays them (the DTO's format rules run forwards for display and
*backwards* for `param_set`, so an assistant says "6.5" or "450 ms", never a stored 0.65). The
offline half (`backup_list` / `backup_describe` / `backup_diff`, `catalog_categories` /
`catalog_models`, `data_status`) decodes export files through the catalog into the same DTO the
live path produces, so one summarizer serves both. The live half (`device_status` / `_connect` /
`_disconnect`, `preset_read`, `block_params`, `preset_list`, `setlists`, `backup_export`, then the
gated `preset_goto`, `block_bypass`, `param_set`, `block_add` / `_swap` / `_delete`,
`snapshot_select`, `undo` / `redo`, `preset_revert`, `preset_save`) wraps the command layer
directly, so the edit history, the heartbeat and every safety rule are the GUI's. Verified
2026-09-02 offline on a fixture export and live read-only against the HX Stomp.

**Not done, by choice:** the streamable-HTTP transport inside `fretwire-serve` (one process
owning the pedal, human in the browser while the assistant edits). It needs a second seat on the
single-editor lease and a "preset changed" broadcast for host-originated edits; the stdio binary
opens its own session and cannot run beside the GUI or daemon. Also open: a `model_params` tool
(a model's parameters before it is in a preset — needs a catalog accessor), and HX Edit `.hxb`
files, whose tones are JSON rather than preset streams.

The survey that led here, kept as the rationale:

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

### The update check runs on the daemon (2026-09-02)

The optional once-a-day release check (`fretwire_core::update`, README "Updates") is a backend
command like any other, so under serve mode it is the *daemon* that asks GitHub and the daemon's
preference file (`update-check.json` beside `serve-token`) that holds the answer — which is right,
since the daemon is what needs updating. The browser shows the same ask bar and badge, worded to
say the request comes from the machine running fretwire-serve, and the release link is a plain
anchor (Tauri's `open_url` is not in the dispatcher: a daemon must not `xdg-open` on the Pi). On
a headless box, `fretwire check-update --auto on|off` sets the preference without a browser.

### The udev rule does not work headless — RESOLVED 2026-08-26

*(Recorded here because this is where it was found; it blocked the **arm64 CLI** in ROADMAP Phase 8
just as hard.)*

`packaging/70-hxstomp.rules` granted access with `TAG+="uaccess"` alone, which is a *seat*
mechanism: systemd-logind grants the locally-seated user. **A Pi reached over SSH has no local
session, so `uaccess` grants nothing** and the daemon gets `EACCES`. The failure is silent about
its cause and a user has no chance of guessing it.

**Fixed:** every rule line now carries `GROUP="plugdev"` alongside `uaccess` (the OpenOCD pattern —
seat grant for desktops, group grant for headless). `plugdev` ships on Raspberry Pi OS and Debian;
`install-udev` runs `groupadd -f plugdev` so the assignment resolves everywhere else, and prints
the `usermod -aG plugdev` step, which stays manual because membership only starts on the next
login. Verified on a box *without* the group: udev logs "Failed to resolve group 'plugdev',
ignoring" per line and still applies `MODE` + `uaccess`, so the degradation is a log warning, not
a broken rule. The CLI test now asserts both grants on every rule line.

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

- **Does the heartbeat survive a slow network client?** ANSWERED (2026-09-01): yes, by
  construction. The 250 ms poll-under-lock loop runs entirely on the serving machine next to the
  USB device — the network sits only between browser and server. Delivery is decoupled through a
  broadcast channel; a WebSocket that can't keep up skips a burst (`Lagged` → continue) instead
  of back-pressuring the beat, which is fine because pushes are advisory live-follow and every
  mutation returns fresh authoritative state.
- **What happens with two browsers open?** ANSWERED (2026-09-01): refused, as proposed. The page
  generates a random client id; its WebSocket claims a single-editor lease. A second browser's
  socket is accepted then closed with code **4409** (so it can render "editor open elsewhere" and
  stop reconnecting), its invokes get HTTP 409, and the lease releases when the holder's socket
  closes — a page refresh reclaims it.
- **Should `fretwire-serve` and the GUI ship together?** They would share the lifted command layer
  but have separate binaries and separate packaging.
- **32-bit Pi OS.** `armv7` checks clean, but ROADMAP Phase 8 deliberately excludes it as an
  untestable support burden. Serve mode weakens that argument — a headless armv7 Pi *can* usefully
  run a server even if it can't usefully run the editor. Not decided.
