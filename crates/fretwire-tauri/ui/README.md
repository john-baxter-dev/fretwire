# fretwire frontend (Svelte)

The web UI for the Tauri desktop app. It builds into `../dist`, which the Rust crate embeds at
compile time (see `../tauri.conf.json`).

## Three ways to run it

| you want | command | what you get |
|---|---|---|
| iterate on the UI | `npm run dev` (here) | browser + mock device, hot reload, no Rust |
| UI against the real pedal | `npm run tauri:dev` (here) | real backend in a real window, hot reload |
| the shipped app | `npm run build` then `cargo build --release -p fretwire-tauri` | `dist/` embedded into the binary |

`npm run tauri:dev` starts this dev server itself (`beforeDevCommand` in `tauri.conf.json`) and
points the app at `http://localhost:5173` instead of the embedded `dist/`. It runs
`tauri dev -- --no-default-features` from the crate root — the flag matters, see "Building for the
real app" below. The port is pinned with `strictPort`, so if something else holds 5173 Vite fails
loudly rather than moving to 5174 and leaving the app staring at a dead URL.

## Working on the frontend without hardware (or a Rust toolchain)

You don't need an HX Stomp, Rust, or Tauri to develop the UI. Run it in a plain browser against the
**mock device backend**:

```sh
npm install
npm run dev          # → http://localhost:5173
```

Open that URL in a browser. When the app can't find a Tauri runtime (which is always the case in a
plain browser), it automatically routes every `invoke`/`listen` call to an in-memory mock device
instead of the real one. You'll see a console line confirming it:

> `[fretwire] no Tauri runtime — using the mock device backend.`

The mock implements **every backend command** and returns the exact same data shapes as the real
Rust backend, so the whole UI works: Connect, the preset browser, param/model editing, bypass,
add/delete, snapshots, the split routing grid + drag-to-place, and live-follow. It ships a small
setlist (a split "Dual Amp" preset plus several serial presets) and a model catalog. State is
in-memory and persists until you reload the page.

One caveat: a browser can't read arbitrary file paths, so **Restore… only works against a backup
made in the same session** (Backup… keeps it in memory, and also downloads the JSON so you can see
the real file shape).

### The first-run import screen

The app gates on whether Line 6's reference data has been imported (`data_status`). The mock reports
it as present so you land in the editor; to work on `FirstRun.svelte`, run `fretwireMock.needsData()`
in the console and reload. The mock's `import_data` fakes a successful import after a short delay
(and throws if you give it `/`, so the error state is reachable). A browser can't open a native file
picker, so `pickPath()` falls back to a prompt for a typed path.

### Choosing which device to mock

fretwire supports two units whose UI differs, so the mock can present as either:

```js
fretwireMock.device()          // what am I right now?
fretwireMock.device("floor")   // Helix Floor: two DSPs, eight setlists  (the default)
fretwireMock.device("stomp")   // HX Stomp: one DSP, one flat preset list
```

**Reload after switching** — it applies on the next connect. The choice is saved to `localStorage`,
so it survives the reload.

What actually changes:

| | HX Stomp | Helix Floor |
|---|---|---|
| Routing grids | 1 | 2, stacked with per-DSP load |
| Setlists | one flat list | 8 (Factory 1/2, User 1–5, Templates) |
| Setlist picker | **hidden** | shown in the sidebar |
| Snapshots | 3 | 8 |

The picker is hidden on a one-setlist device on purpose — HX Edit shows no setlist control for the
Stomp either. Floor mode leaves User 3–5 empty, which is what a stock unit looks like and exercises
the empty-list state.

> **The mock shows the setlist picker; the real app currently does not.** Against hardware it is
> gated behind `FRETWIRE_SETLISTS=1` because the browse's preset numbering isn't fully understood —
> it numbered a TEMPLATES preset 906 (global) where the device wanted slot 10, which locked a Helix
> Floor up. The mock keeps it enabled so the UI can still be worked on. Don't treat the mock as
> evidence of shipped behaviour here.

### Simulating live device pushes

To exercise live-follow (changes the UI mirrors when they originate on the hardware, e.g. a
footswitch), use the `window.fretwireMock` helper from the browser devtools console while a preset is open:

```js
fretwireMock.bypass(1, false)   // footswitch bypasses the block in slot 1 (enabled=false → bypassed)
fretwireMock.snapshot(2)        // panel switches to snapshot 3
fretwireMock.preset(1)          // panel switches to preset #1
fretwireMock.state()            // inspect the current in-memory preset
```

### Where the seam is

- `src/lib/ipc.js` — the single seam. Everything imports `invoke`/`listen` from here. It picks the
  real Tauri API or the mock based on whether `window.__TAURI_INTERNALS__` exists.
- `src/mock/backend.js` — the mock device: model catalog, presets, and one handler per command.

When adding a new backend command, add its handler to `src/mock/backend.js` so the mocked UI keeps
working. Keep the mock's return shapes in sync with `../src/dto.rs`.

## Building for the real app

```sh
npm run build                          # → ../dist
cargo run -p fretwire-tauri --release  # from the repo root, runs against real hardware
```

Build `../dist` first: the `custom-protocol` feature is on by default so that plain cargo builds
work, and that makes `../dist` a build-time requirement — the codegen panics if it is missing.

For hot-reloading UI work against the real backend, use `npm run tauri:dev` (from here), which is
`tauri dev -- --no-default-features`. Turning the feature off is what points the webview at this
dev server instead of the embedded `../dist`; a `tauri dev` *without* it would silently serve the
last built `dist/` and your edits would appear to do nothing.

In the real Tauri webview the mock is bundled but never used — `window.__TAURI_INTERNALS__` is
present, so `ipc.js` routes to the real backend.
