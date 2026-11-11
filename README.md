# Snowfall ❄️

A lightweight **Windows desktop overlay** that makes snow fall over your screen. Snowflakes land on the top edges of your open windows, on the taskbar and on the bottom of the screen, and build up into drifts. The overlay is transparent and click-through, so you can keep working as usual.

## Features

- Falling snow in several depth layers with a slowly changing wind.
- **Snow piles** on top of windows, on the taskbar ("Dock") and on the bottom edge of the screen ("Ground"). Piles are drawn *behind* windows that cover them.
- Piles follow their window when you drag it. Drag a window fast and the snow flies off it.
- Minimize or close a window and its pile crumbles and falls down.
- **Mouse interaction:** move the cursor over a pile to plow the snow into a mound (it spills off the window edges), or over falling flakes to push them around. On the taskbar pile the snow simply disappears under the cursor.
- Tray icon with a small settings window, global hotkey to quit, low CPU.

## Usage

| Action | How |
|---|---|
| Open settings | Left-click the tray icon |
| Menu | Right-click the tray icon (Settings, Pause/Resume, Clear snow, Exit) |
| Quit | `Ctrl+Alt+Q` or tray menu → Exit |

## Settings

| Setting | What it does |
|---|---|
| Intensity | Number of falling flakes (50-3000) |
| Opacity | Opacity of the falling flakes (10-100%). Piles are not affected |
| Frame rate | 20 / 30 / 60 FPS (the overlay boosts to 60 FPS while a window is being moved) |
| Accumulate | Where snow collects: **Windows**, **Dock** (top of the taskbar), **Ground** (bottom of the screen) |
| Height limit | On: piles stop at a fixed height. Off: piles keep growing until they fill the screen |
| Mouse pushes snow | Cursor pushes falling flakes |
| Pause / Clear snow | Freeze the simulation / remove all snow |

*Note: Settings are not saved between runs yet.*

## Known limitations

- Windows only.
- The overlay is hidden while a full-screen application is in the foreground.
- One overlay covers the whole virtual screen; the "Dock" snow follows the primary taskbar.
- Cloaked windows (other virtual desktops) and tool windows are ignored.

## Privacy

Snowfall makes no network connections and collects no data. It only reads window positions and titles locally to find where snow can land.

## License

[PolyForm Noncommercial License 1.0.0](LICENSE)