# Snowfall Developer Guide 🛠️

Technical architecture, build instructions, and debugging tools for developers and contributors.

## Build and Run

Prerequisites: Rust toolchain (stable).

Commands to build and run:

    git clone <your-repo-url>
    cd Snowfall
    cargo run --release

Note: Always benchmark and test in `--release` mode. Debug builds introduce severe frame timing latency due to CPU-bound rendering and physics steps.
Compiled binary output: `target\release\snowfall.exe`.

## Architecture & How It Works

1. Rendering & Compositing:
   - A single topmost, transparent window spanning the entire virtual desktop (`WS_EX_LAYERED | WS_EX_TRANSPARENT`).
   - Frame rendering is handled purely on the CPU via [`tiny-skia`](https://crates.io/crates/tiny-skia), painted directly into a DIB section, and presented via `UpdateLayeredWindow`.

2. Window Tracking & Hitboxes:
   - Application bounds are sampled using `EnumWindows`, `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`, and hooked dynamically via `SetWinEventHook`.
   - The visible top edge of each window (clipped against overlapping windows above it in the Z-order) is resolved into one or more 1D landing line segments.

3. Physics & Accumulation:
   - Settled flakes increment column height values in discrete segment arrays.
   - A local slope-relaxation pass simulates angle-of-repose physics, allowing drifts to slide naturally off edges.
   - Piles hidden behind a window (for example the dock pile under a maximized window) do not stop flakes: they keep falling down to the pile's base line.
   - Clear snow starts a directional wave front (random left or right) that turns pile columns into flying flakes. Falling flakes are swept by the same front, with a swirling motion on top of the wind. New snowfall resumes from the top once the screen is clear.

4. Settings Persistence:
   - `src/config.rs` stores the settings as simple `key=value` lines in `%APPDATA%\Snowfall\settings.txt`.
   - The file is loaded at startup, written about once per second after a change and once more on exit. Invalid values are ignored or clamped; Pause is intentionally not saved.

## CLI Debugging Flags

Use these flags during local development to inspect bounds, profiling metrics, and window manager behavior:

| Flag | Purpose |
|---|---|
| `--debug` | Visualizes the layout engine: draws detected window boundaries in red and computed snow landing edges in green. |
| `--perf` | Streams per-second frame render and commit times to stdout. |
| `--brushlog` | Outputs mouse-brush collision vectors and displacement stats to stdout. |
| `--dump` | After 5 seconds write a list of all top-level windows to `dump.txt` and exit. Contains window titles, so do not share or commit this file. |