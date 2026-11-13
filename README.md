# Snowfall ❄️

A lightweight **Windows desktop overlay** that makes snow fall over your screen. Snowflakes land on the top edges of your open windows, on the taskbar and on the bottom of the screen, and build up into drifts. The overlay is transparent and click-through, so you can keep working as usual.

## Features

- Falling snow in several depth layers with a slowly changing wind.
- **Snow piles** on top of windows, on the taskbar ("Dock") and on the bottom edge of the screen ("Ground"). Piles are drawn *behind* windows that cover them.
- Piles follow their window when you drag it. Drag a window fast and the snow flies off it.
- Minimize or close a window and its pile crumbles and falls down.
- **Clear snow** blows everything away with a swirling wind gust: a wave front sweeps across the screen, lifting the piles and the falling flakes with it. The wind direction (left or right) is chosen randomly each time, and snowfall resumes from the top once the screen is clear.
- **Mouse interaction:** move the cursor over a pile to plow the snow into a mound (it spills off the window edges), or over falling flakes to push them around. On the taskbar pile the cursor gradually erases snow in a circular area.
- Tray icon with a small settings window, global hotkey to quit, low CPU.

## Download

Get the latest version from the [Releases](https://github.com/Tomjerri1/Snowfall/releases/latest) page:

- **`snowfall-*-setup.exe`** (recommended): the installer. No administrator rights needed. It can add a desktop shortcut and start Snowfall with Windows.
- **`snowfall-*-windows-x64.zip`**: portable version. Unzip it and run `snowfall.exe`.

> Windows SmartScreen may show a warning because the files are not code-signed. Click **More info → Run anyway**. You can verify a download with the matching `.sha256` file from the release.

## Usage

| Action | How |
|---|---|
| Open settings | Left-click the tray icon |
| Menu | Right-click the tray icon (Settings, Pause/Resume, Clear snow, Exit) |
| Quit | `Ctrl+Alt+Q` or tray menu → Exit |

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

Settings are saved automatically to `%APPDATA%\Snowfall\settings.txt` and restored on the next start (Pause is not saved).

## Known limitations

- Windows only.
- The overlay is hidden while a full-screen application is in the foreground.
- One overlay covers the whole virtual screen; the "Dock" snow follows the primary taskbar.
- Cloaked windows (other virtual desktops) and tool windows are ignored.

## Privacy

Snowfall makes no network connections and collects no data. It only reads window positions and titles locally to find where snow can land.

## License

[PolyForm Noncommercial License 1.0.0](LICENSE)