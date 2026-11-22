# Snowfall ❄️

A lightweight **desktop overlay for Windows, Linux (X11) and macOS** that makes snow fall over your screen. Snowflakes land on the top edges of your open windows, on the taskbar and on the bottom of the screen, and build up into drifts. The overlay is transparent and click-through, so you can keep working as usual.

## Features

- Falling snow in several depth layers with a slowly changing wind.
- **Snow piles** on top of windows, on the taskbar ("Dock") and on the bottom edge of the screen ("Ground"). Piles are drawn *behind* windows that cover them.
- Piles follow their window when you drag it. Drag a window fast and the snow flies off it.
- Minimize or close a window and its pile crumbles and falls down.
- **Clear snow** blows everything away with a swirling wind gust: a wave front sweeps across the screen, lifting the piles and the falling flakes with it. The wind direction (left or right) is chosen randomly each time, and snowfall resumes from the top once the screen is clear.
- **Mouse interaction:** move the cursor over a pile to plow the snow into a mound (it spills off the window edges), or over falling flakes to push them around. On the taskbar pile the cursor gradually erases snow in a circular area.
- Tray icon (status bar item on macOS) with settings, global hotkey to quit, low CPU.

## Download

Get the latest version from the [Releases](https://github.com/Tomjerri1/Snowfall/releases/latest) page:

**Windows**

- **`snowfall-*-windows-x64-setup.exe`** (recommended): the installer. No administrator rights needed. It can add a desktop shortcut and start Snowfall with Windows.
- **`snowfall-*-windows-x64.zip`**: portable version. Unzip it and run `snowfall.exe`.

**Linux (x86-64, X11)**

- **`snowfall-*-linux-x64.tar.gz`**: extract it and run `./snowfall`. See [Linux notes](#linux-notes) for what the desktop needs.

**macOS (11 or newer, Apple Silicon and Intel)**

- **`snowfall-*-macos-universal.tar.gz`**: extract it and run `./snowfall`. See [macOS notes](#macos-notes) for the first launch.

> The files are not code-signed. Windows SmartScreen may show a warning: click **More info → Run anyway**. You can verify any download with the matching `.sha256` file from the release.

## Usage

| Action | Windows | Linux | macOS |
|---|---|---|---|
| Open settings | Left-click the tray icon | Click the tray icon (settings are in its menu) | Click the status bar icon (settings are in its menu) |
| Menu | Right-click the tray icon (Settings, Pause/Resume, Clear snow, Exit) | Tray menu | Status bar menu |
| Quit | `Ctrl+Alt+Q` or tray menu → Exit | `Ctrl+Alt+Q` or tray menu → Quit | `Ctrl+Option+Q` or menu → Quit |

## Settings

| Setting | What it does |
|---|---|
| Intensity | Amount of falling snow, shown in % |
| Opacity | Opacity of the falling flakes (10-100%). Piles are not affected |
| Strength | How strong the snowfall is (25-200%): scales the fall speed and the wind |
| Frame rate | 20 / 30 / 60 FPS (the overlay boosts to 60 FPS while a window is being moved) |
| Accumulate | Where snow collects: **Windows**, **Dock** (top of the taskbar), **Ground** (bottom of the screen) |
| Height limit | On: piles stop at a fixed height. Off: piles keep growing until they fill the screen |
| Mouse pushes snow | Cursor pushes falling flakes |
| Pause / Clear snow | Freeze the simulation / blow all snow away with a wind gust |

On Linux and macOS the settings window is replaced by the tray menu: Intensity, Opacity, Strength and Frame rate are submenus of presets, the other settings are check items. "Dock" is the top of the panel or Dock.

Settings are saved automatically and restored on the next start (Pause is not saved). The file is `settings.txt` in:

- Windows: `%APPDATA%\Snowfall\`
- macOS: `~/Library/Application Support/Snowfall/`
- Linux: `$XDG_CONFIG_HOME/snowfall/` (usually `~/.config/snowfall/`)

## Linux notes

- Works on **X11**, and on Wayland sessions through **XWayland**, where only X11 windows are visible to Snowfall: snow does not land on native Wayland windows. The Ground and Dock piles still work.
- Needs a **compositing window manager** (the overlay is transparent) that follows the EWMH standard (`_NET_CLIENT_LIST_STACKING`), which includes KDE, GNOME, Xfce, Cinnamon and most others.
- The tray icon uses the StatusNotifierItem protocol. KDE and most other desktops support it; **GNOME needs the AppIndicator extension**. Without it you can still quit with `Ctrl+Alt+Q`.
- Frames are sent to the X server through shared memory (MIT-SHM) when the server is local, and through plain `PutImage` otherwise.
- The update check runs `curl`, which must be installed.

## macOS notes

- The binary is not notarized, so Gatekeeper blocks it after a browser download. Remove the quarantine flag once, then run it:

      xattr -d com.apple.quarantine ./snowfall

  or allow it in **System Settings → Privacy & Security → Open Anyway**.
- No permissions are needed: Snowfall reads only window positions, never titles or screen contents.
- Only windows on the current Space are tracked. The overlay is drawn at one pixel per point, so snow looks slightly soft on Retina displays.
- The app has no Dock icon; it lives in the status bar.

## Known limitations

- The Linux and macOS versions are newer and less tested than the Windows one.
- The overlay is hidden while a full-screen application is in the foreground.
- One overlay covers the whole virtual screen; every monitor gets its own "Dock" and "Ground" piles.
- Windows on other virtual desktops or Spaces, and tool windows, are ignored.

## Privacy

Snowfall collects no data and makes no network connections on its own. It only reads window positions locally to find where snow can land (on Windows also window titles, to skip untitled helper windows). The one exception is **Check for updates**, in the settings window on Windows and in the tray menu on Linux and macOS: when you use it, Snowfall sends a single request to the GitHub API (api.github.com) to read the version number of the latest release. Nothing else is sent.

## License

[PolyForm Noncommercial License 1.0.0](LICENSE)