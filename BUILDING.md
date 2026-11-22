# Snowfall Developer Guide 🛠️

Technical architecture, build instructions, and debugging tools for developers and contributors.

## Build and Run

Prerequisites: Rust toolchain (stable). Nothing else is needed on Windows. On Linux and macOS no system development packages are required either: the X11, D-Bus and Objective-C bindings are pure Rust and the libraries are loaded at run time. The update check uses the system `curl` there.

Commands to build and run:

    git clone <your-repo-url>
    cd Snowfall
    cargo run --release

Note: Always benchmark and test in `--release` mode. Debug builds introduce severe frame timing latency due to CPU-bound rendering and physics steps.
Compiled binary output: `target\release\snowfall.exe` on Windows, `target/release/snowfall` elsewhere.

The platform is chosen by the compile target, so the same command works everywhere. You can type-check another platform without its OS if the target is installed, for example `cargo check --target x86_64-unknown-linux-gnu` or `--target aarch64-apple-darwin`. This catches type errors, but not runtime behaviour: CI builds and tests on Windows, Linux and macOS.

## Architecture & How It Works

The code is split into platform-independent logic and one backend per OS:

- Independent: `main.rs` (the frame loop), `snow.rs` (flakes, piles, physics, drawing with [`tiny-skia`](https://crates.io/crates/tiny-skia)), `geometry.rs` (window rects, monitors, landing segments), `app.rs` (shared settings and commands), `config.rs`, `update.rs`.
- Platform: `src/sys/`. `sys/mod.rs` defines the `Backend` trait (overlay surface, window tracking, cursor, a few system services) and exports exactly one implementation as `PlatformBackend`. The frame loop only talks to that trait.

| Backend | Directory | Compiled for |
|---|---|---|
| Windows | `src/sys/windows/` | `cfg(windows)` |
| X11 | `src/sys/x11/` | Unix except macOS |
| macOS | `src/sys/macos/` | `cfg(target_os = "macos")` |

To add a platform, implement `Backend` in a new `src/sys/<name>/` and export it from `sys/mod.rs`. Coordinates handed to the loop are pixels with `y` down, relative to the virtual screen of the whole desktop.

1. Rendering & Compositing:
   - Every backend owns a single topmost, transparent, click-through window spanning the virtual desktop, plus a premultiplied BGRA buffer. Frames are rendered purely on the CPU by `tiny-skia` into that buffer.
   - Windows: `WS_EX_LAYERED | WS_EX_TRANSPARENT`, the buffer is a DIB section presented with `UpdateLayeredWindow`.
   - X11: an override-redirect window with a 32-bit ARGB visual and an empty SHAPE input region (needs a compositor). The buffer is a `memfd` shared with the server (MIT-SHM, `ShmPutImage`, drawing waits for `ShmCompletion`), or banded `PutImage` when that is unavailable.
   - macOS: a borderless `NSWindow` at the screen-saver level with a clear background; each frame becomes a `CGImage` set as the layer contents. The render loop pumps the `NSApplication` event queue itself, so it must run on the main thread.

2. Window Tracking & Hitboxes:
   - Windows: `EnumWindows`, `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`, kept fresh by `SetWinEventHook` (including move/size start and end for dragging).
   - X11: `_NET_CLIENT_LIST_STACKING` and other EWMH properties, structure events on the root window as the change signal, frame extents (and `_GTK_FRAME_EXTENTS` shadows) to get the visible outline. Dragging is guessed from frame configure events while the left button is held.
   - macOS: `CGWindowListCopyWindowInfo` polled at most every 50 ms (no change notifications exist, and no Screen Recording permission is needed because titles are not read). Dragging is guessed from the held left button plus a rect change of the window under the cursor.
   - The visible top edge of each window (clipped against overlapping windows above it in the Z-order) is resolved into one or more 1D landing line segments in `geometry.rs`.

3. Physics & Accumulation:
   - Settled flakes increment column height values in discrete segment arrays.
   - A local slope-relaxation pass simulates angle-of-repose physics, allowing drifts to slide naturally off edges.
   - Piles hidden behind a window (for example the dock pile under a maximized window) do not stop flakes: they keep falling down to the pile's base line.
   - Clear snow starts a directional wave front (random left or right) that turns pile columns into flying flakes. Falling flakes are swept by the same front, with a swirling motion on top of the wind. New snowfall resumes from the top once the screen is clear.

4. Settings Persistence:
   - `src/config.rs` stores the settings as simple `key=value` lines in `settings.txt` in the per-user config directory (`%APPDATA%\Snowfall`, `~/Library/Application Support/Snowfall` or `$XDG_CONFIG_HOME/snowfall`).
   - The file is loaded at startup, written about once per second after a change and once more on exit. Invalid values are ignored or clamped; Pause is intentionally not saved.

5. Tray & Settings UI:
   - Windows: a hidden message window owns the notification-area icon and a GDI settings window.
   - X11: a StatusNotifierItem over D-Bus (`ksni`); the menu is rebuilt from `AppState` each time it opens.
   - macOS: a status item (`tray-icon`) whose check marks are re-synced from `AppState` after every click.
   - Linux and macOS share the preset tables and the icon in `src/sys/tray_shared.rs`.

## CLI Debugging Flags

Use these flags during local development to inspect bounds, profiling metrics, and window manager behavior:

| Flag | Purpose |
|---|---|
| `--debug` | Visualizes the layout engine: draws detected window boundaries in red and computed snow landing edges in green. |
| `--perf` | Streams per-second frame render and commit times to stdout. |
| `--brushlog` | Outputs mouse-brush collision vectors and displacement stats to stdout. |
| `--dump` | After 5 seconds write a list of all top-level windows to `dump.txt` and exit, with the reason each one was accepted or rejected and the resulting landing segments. Contains window titles (where the platform exposes them), so do not share or commit this file. |