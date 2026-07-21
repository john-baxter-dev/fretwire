# fretwire frontend (Svelte)

The web UI for the Tauri desktop app. It builds into `../dist`, which the Rust crate embeds at
compile time (see `../tauri.conf.json`).

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
npm run build                       # → ../dist
cargo run -p fretwire-tauri               # from the repo root, runs against real hardware
```

In the real Tauri webview the mock is bundled but never used — `window.__TAURI_INTERNALS__` is
present, so `ipc.js` routes to the real backend.
