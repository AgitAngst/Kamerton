<p align="center">
  <img src="assets/icon/kamerton-128.png" width="96" alt="Kamerton">
</p>

<h1 align="center">Kamerton</h1>

<p align="center">
  <b>A fast tray volume mixer for Windows 10/11 — everything EarTrumpet does, and the things it doesn't.</b><br>
  One portable <code>.exe</code> · Rust · ~20–28 MB RAM · zero CPU at rest · English & Russian · light & dark
</p>

<p align="center">
  <a href="https://github.com/AgitAngst/Kamerton/releases/latest"><b>⬇ Download the latest release</b></a>
  · <a href="README.ru.md">По-русски</a>
</p>

---

## Why Kamerton instead of EarTrumpet

EarTrumpet is great at one thing: per-app volume in a tray flyout. Kamerton does that too, and then keeps going.

- 🎚️ **Control any device, not just the default one.** Every output device has its own slider in the flyout, and
  every device can have its own hotkeys — louder, quieter, mute — whether it is the default or not. Turn the
  headphones down while the speakers stay default; bind `Shift+VolumeUp` to your second device and use the media
  keys for both.
- 🧠 **It remembers.** Per-app volume and mute are restored when the app comes back; new apps start at a volume
  you choose; the device you moved an app to is remembered after the app closes (Windows forgets it).
- 🗂️ **Profiles.** Save default devices + every app's volume, mute and device as a profile; apply it from the tray
  menu, or automatically when a device is plugged in.
- 🛡️ **Rules.** A volume ceiling (global and per app) that nothing can push past, a quieter **night mode** by the
  clock, **auto-ducking** (when your call app speaks, everything else gets quieter, then comes back), and per-app
  rules ("when Discord starts — send it to the headset at 60 %").
- ⌨️ **Hotkeys for everything.** Global volume, next/previous output device, per device, per app. Taken
  combinations are flagged so you know which one another program holds.
- 💻 **Command line.** `kamerton --set spotify 30`, `--mute default toggle`, `--get input` — scriptable, with
  proper exit codes. Plus `--list` and a live `--watch` event stream.
- 📊 **Volume OSD** next to the taskbar when you scroll over the tray icon or use hotkeys — showing *which* device
  changed.
- ⚖️ **Balance** (L/R) per device in the full mixer window.
- 🎙️ **Microphones** and recording apps in the same flyout, one click away.
- 🪶 **Small and native.** A single Rust executable on Direct3D 11. No installer, no runtime, no Store. Drop an
  empty `settings.txt` next to the exe and it becomes fully portable.
- 🔄 **Self-updating** from GitHub Releases, verified by SHA-256; anything goes wrong — the old version stays.

### Side by side

| | EarTrumpet | **Kamerton** |
|---|:---:|:---:|
| Per-app volume in a tray flyout | ✅ | ✅ |
| Switch default device, move an app to another device | ✅ | ✅ (+ drag & drop) |
| Volume hotkeys for a **non-default** device | — | ✅ |
| Volume hotkeys per app | — | ✅ |
| Remembers app volume and app → device | — | ✅ |
| Profiles, auto-applied on device connect | — | ✅ |
| Volume ceiling, night mode, auto-ducking | — | ✅ |
| Command line control | — | ✅ |
| Own volume OSD | — | ✅ |
| Device balance (L/R) | — | ✅ |
| Portable single exe, no runtime | — | ✅ |

## Features

**Tray**
- The icon shows the default device volume (Kamerton's tuning-fork mark or a classic speaker, purple or azure).
- Click — flyout · middle click — mute · wheel — ±2 % · right click — devices, profiles, Windows sound panels.

**Flyout** (Windows 11 quick-settings look, docks to the taskbar on any side)
- All output devices with sliders, mute and live level meters; click a device to make it default.
- All apps across all devices, with a hint when an app plays on a non-default device.
- Drag an app onto another device to move it; right-click for device, mute, hotkeys, hide.
- Microphones and recording apps behind one button.

**Mixer window** — a column per device, with balance, sized to fit, remembers its place, optionally always on top.

**Settings** — theme, language, autostart, open animation, wheel-anywhere-while-open, logarithmic slider scale,
OSD, hidden apps and devices.

**Reliability** — rebuilds itself after the Windows audio service restarts or the session is unlocked, stays in
place after DPI changes, writes `crash.log` on failure.

## Install

1. Download `kamerton-X.Y.Z-windows-x64.zip` from [Releases](https://github.com/AgitAngst/Kamerton/releases/latest)
   (check it against `SHA256SUMS` if you like).
2. Unzip and run `kamerton.exe`. That's it — enable *Start with Windows* in Settings → System.

Settings live in `%APPDATA%\Kamerton\settings.txt`, or next to the exe for a portable copy.

## Command line

```text
kamerton --set <target> <volume>        # 45, 45%, +5, -5
kamerton --mute <target> [on|off|toggle]
kamerton --get <target>
kamerton --list | --watch [--peaks]
```

`<target>` is `default`, `input`, an app name (`spotify`) or a device name. Exit codes: `0` ok, `1` audio error,
`2` bad arguments, `3` target not found or ambiguous.

## Docs

Full documentation (in Russian): [how to use](docs/guide/usage.md) · [architecture](docs/architecture/overview.md)
· [development](docs/dev/development.md).

## Build from source

```bash
cargo build --release
```

Requires Rust (edition 2024) and Windows 10/11.

## License

MIT
