# fretwire

An independent, from-scratch **Linux editor for the Line 6 HX Stomp**, written in Rust.

fretwire talks to the pedal over its `MI_00` USB control interface (VID `0x0E41` / PID `0x4246`).
The wire protocol was recovered by **observing USB traffic to and from the device**; the model,
preset, and control **data is not included** — fretwire reads it at runtime from a copy you import
from your own HX Edit installation (see [below](#model-names-dsp-loads-and-param-ranges-optional)).

> ⚠️ **No warranty. Use at your own risk — see [Disclaimer](#disclaimer).** Firmware/flash/DFU
> operations are deliberately out of scope and never transmitted (`docs/safety.md`).

## Layout

| path | what |
|------|------|
| `crates/fretwire-data`     | parsers for the shipped JSON (`*.models`, catalog, controls, `.hlx` presets) |
| `crates/fretwire-protocol` | `MI_00` wire message types + codec |
| `crates/fretwire-usb`      | USB transport via `nusb` |
| `crates/fretwire-core`     | device session API |
| `crates/fretwire-cli`      | `fretwire` command-line driver |
| `crates/fretwire-tauri`    | the graphical editor — Tauri (WebKitGTK) + Svelte |
| `captures/`                | per-capture action notes + small preset-stream fixtures used by the tests |
| `docs/`, `ROADMAP.md`      | protocol notes, preset format, safety, and the plan |

## Quick start

The whole sequence, for a working graphical editor on a real pedal:

1. [Build](#build) the workspace (Rust 1.96+).
2. [Install the udev rule](#first-time-setup-talking-to-a-real-device) and replug the pedal.
3. [Import the reference data](#model-names-dsp-loads-and-param-ranges-optional) from your own HX
   Edit install (optional, but you want it — it's where model and parameter names come from).
4. [Build the frontend and run the GUI](#the-graphical-editor).

No pedal, no Rust, nothing installed? The whole UI runs in a browser against a
[mock device](#no-hardware-no-rust).

## Build

Needs **Rust 1.96 or newer** (`rustup update` if your distro's toolchain is older).

```
cargo build
cargo test
cargo run -p fretwire-cli -- detect                 # is an HX Stomp connected?
cargo run -p fretwire-cli -- show-preset <stream>   # decode a reassembled device preset stream
```

The CLI binary is `fretwire`. `show-preset` takes a reassembled preset MessagePack stream and prints
its blocks with resolved model ids, device param order (Mono/Stereo), and current values — offline.

`cargo build` deliberately skips the GUI, so it needs no system libraries beyond a Rust toolchain —
see [The graphical editor](#the-graphical-editor) for that.

## First-time setup (talking to a real device)

Linux's default `uaccess` tags only `/dev/snd/*`, not the HX Stomp's raw vendor USB node, so
without a udev rule every live command needs root. Install the rule once:

```
cargo run -p fretwire-cli -- install-udev   # writes /etc/udev/rules.d/70-hxstomp.rules, reloads udev
```

It writes directly when run as root, otherwise re-runs the privileged steps through `sudo`
(`install-udev --print` just emits the rule if you'd rather install it by hand; the canonical copy
lives in `packaging/70-hxstomp.rules`). **Unplug and replug the device** afterward. Then:

```
cargo run -p fretwire-cli -- detect       # HX Stomp: present
cargo run -p fretwire-cli -- pull         # read the loaded preset (non-destructive)
```

The rule covers both the HX Stomp (`0x4246`) and HX Stomp XL (`0x4253`).

### Model names, DSP loads, and param ranges (optional)

The wire protocol edits by raw parameter index, so the tool works with **no** Line 6 data at all —
you just get numeric indices instead of names. To get model/param names, the DSP meter, and control
ranges, import the reference data from **your own** HX Edit installer (nothing is redistributed):

```
cargo run -p fretwire-cli -- import-data <path-to-HX-Edit-installer>   # .exe/.msi/.pkg/.dmg (unpacked with 7z)
```

`import-data` also accepts a **directory** of already-extracted files — an HX Edit install's `res/`
folder, or an installer you unpacked by any means — which needs no `7z`:

```
cargo run -p fretwire-cli -- import-data /path/to/res     # a directory: no 7z required
```

It caches the files under `~/.local/share/fretwire/data` (`$FRETWIRE_DATA_DIR` overrides), and the
tool loads them from there at runtime. Builds ship **no** Line 6 data.

## The graphical editor

`fretwire-tauri` is the editor: a WebKitGTK window over a Svelte frontend, on the same
`fretwire-core` session the CLI uses. It's **not** part of `cargo build` — it needs system libraries
and a built frontend, so you opt into it.

**1. System libraries.** Tauri 2 needs WebKitGTK and a C toolchain:

```
# Debian / Ubuntu / Kubuntu
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
                 libssl-dev libayatana-appindicator3-dev librsvg2-dev
# Arch
sudo pacman -S webkit2gtk-4.1 base-devel curl wget file openssl librsvg libappindicator-gtk3 xdotool
# Fedora
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file librsvg2-devel \
                 libappindicator-gtk3-devel && sudo dnf group install "C Development Tools and Libraries"
```

You also need **Node 20+** for the frontend build.

**2. Build the frontend.** The Rust crate embeds `crates/fretwire-tauri/dist/` at compile time, and
that directory is a build artifact — it isn't in git. Build it first, or `cargo run -p fretwire-tauri`
will fail on a fresh clone:

```
cd crates/fretwire-tauri/ui
npm install
npm run build          # → ../dist
```

Re-run `npm run build` after any frontend change; the Rust side won't pick it up otherwise.

**3. Run it.**

```
cargo run -p fretwire-tauri
```

Connect, browse presets, edit blocks and parameters, drag blocks around the routing grid, manage
snapshots, save, back up and restore. It live-follows the hardware, so footswitch and panel changes
show up in the window. (The app forces WebKitGTK's non-dmabuf compositing path on Linux by default —
some GPU/compositor combinations hit a fatal Wayland protocol error otherwise. Set
`WEBKIT_DISABLE_DMABUF_RENDERER` yourself to override.)

### No hardware, no Rust

The frontend runs standalone in a browser against an in-memory **mock device** — no pedal, no Rust
toolchain, no Tauri, no system libraries:

```
cd crates/fretwire-tauri/ui
npm install
npm run dev            # → http://localhost:5173
```

When it can't find a Tauri runtime it routes every backend call to the mock, which implements the
full command surface: a setlist, the model catalog, split routing, and simulated live pushes from
the hardware. It's the easiest way to see what the editor actually does before installing anything.
See `crates/fretwire-tauri/ui/README.md`.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Disclaimer

fretwire is an **independent interoperability project** and is **not affiliated with, authorized,
endorsed, or sponsored by Line 6 or Yamaha Guitar Group**. "Line 6", "HX", "HX Stomp", and "HX Edit"
are trademarks of their respective owners and are used here only nominatively, to identify the
hardware fretwire interoperates with. No Line 6 software or data is included or distributed.

**THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED.** To the
maximum extent permitted by law, the authors and contributors accept **no liability** for any
damage, malfunction, data loss, or other harm to your device, your presets, or anything else
arising from the use of this software. **You assume all risk.** fretwire sends control-plane
messages of the same class the official editor uses and never transmits firmware/flash/DFU traffic
(`docs/safety.md`), but hardware interoperability recovered from observation is inherently uncertain
— **back up your device before writing to it, and understand the recovery path first.**
