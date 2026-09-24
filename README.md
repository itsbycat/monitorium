<p align="center">
  <img src="assets/icons/128x128.png" width="96" alt="Monitorium logo">
</p>

<h1 align="center">Monitorium</h1>

<p align="center">
  Change the brightness of all your monitors from the system tray.
</p>

---

Monitorium is a small tray app for Windows. Click the tray icon and a panel opens next to the taskbar with a brightness slider for every
connected monitor. It is written in Rust with a native UI ([egui](https://github.com/emilk/egui)),
so there's no browser engine inside and it stays light on memory.

## Features

- **One slider per monitor**, or link them all together and move them as one.
- **Works with almost any screen.** Monitorium picks the best way to change brightness for each
  monitor on its own (see [How brightness is changed](#how-brightness-is-changed)).
- **Scroll over the tray icon** to change brightness without opening anything.
- **Keyboard shortcuts** for brightness up, brightness down and turning displays off.
- **Turn off displays** with one click.
- **Starts with Windows** and waits quietly in the tray.
- **Light and dark theme**, following Windows by default.
- Per-monitor options: custom name, hide from the panel, minimum and maximum brightness, and
  forcing software dimming.

## How brightness is changed

Every monitor uses one of these methods. The panel shows which one under the monitor's name.

| Method | Used for | Notes |
| --- | --- | --- |
| **DDC/CI** | Most external monitors | Changes the monitor's real backlight, like the buttons on the monitor do. |
| **Built-in** | Laptop screens | The same control as the Windows brightness slider. |
| **SDR content brightness** | Monitors with HDR turned on | Most monitors lock their backlight in HDR mode, so Monitorium adjusts Windows' "SDR content brightness" instead. Can be turned off in Settings. |
| **Software (overlay)** | Monitors without DDC/CI | Puts a dark, click-through layer over the screen. Can dim almost to black. |
| **Software (gamma)** | Monitors without DDC/CI | Adjusts the screen's color curve instead of using a layer. Windows only allows about half brightness this way, and HDR screens use the overlay instead. |

You choose between overlay and gamma in **Settings → Brightness**. You can also force software
dimming for a single monitor in **Settings → Monitors**, for example if its DDC/CI misbehaves.

## Installing

Download `monitorium_<version>_x64-setup.exe` and run it. The installer doesn't need
administrator rights; Monitorium is installed for your user only.

After the first start, Monitorium runs when you sign in to Windows. You can turn that off in
Settings or from the tray menu.

## Using it

| Action | How |
| --- | --- |
| Open the brightness panel | Left-click the tray icon |
| Tray menu (refresh, turn off displays, settings, quit) | Right-click the tray icon |
| Change brightness quickly | Scroll the mouse wheel over the tray icon |
| Brightness up / down | `Ctrl + Alt + ↑` / `Ctrl + Alt + ↓` (changeable in Settings) |
| Close the panel | Click anywhere else, or press `Esc` |

> [!TIP]
> Windows sometimes hides new tray icons behind the **^** arrow. Drag the icon onto the taskbar
> to keep it visible.

### Command line

```text
monitorium --list          Show detected monitors and how their brightness is controlled
monitorium --set 70        Set every monitor to 70% and exit
monitorium --autostart     Start hidden in the tray (used when starting with Windows)
```

`--set` changes DDC/CI, built-in and SDR brightness. Software dimming only lasts while the tray
app is running, so use the tray app for monitors that need it.

## Troubleshooting

**The slider moves but the screen doesn't change.**
Check whether HDR is on (Windows Settings → System → Display). If it is, make sure
**Settings → Brightness → Use SDR content brightness on HDR displays** is enabled. Otherwise check
that DDC/CI is enabled in your monitor's own menu. Some monitors ship with it off.

**My monitor isn't listed, or only offers software dimming.**
Some docks, KVM switches, adapters and TVs don't pass DDC/CI through. Software dimming still
works on them. Click the refresh button in the panel after connecting a monitor.

**Where are my settings?**
`%APPDATA%\ByCat\Monitorium\config` (open it from Settings → About). The log file is in
`%LOCALAPPDATA%\ByCat\Monitorium\data\monitorium.log`.

## Building from source

You need [Rust](https://rustup.rs) (stable, 2024 edition) and the Visual Studio C++ build tools
(the MSVC toolchain).

```bash
cargo build --release
```

The app is `target/release/monitorium.exe`. It is built with a static C runtime (see
`.cargo/config.toml`), so it runs without installing the Visual C++ redistributable.

### Making the installer

The installer is built with [cargo-packager](https://github.com/crabnebula-dev/cargo-packager),
configured in `Cargo.toml` under `[package.metadata.packager]`.

```bash
cargo install cargo-packager --locked
cargo packager --release
```

The installer is written to `dist/`.

### Project layout

```text
src/                         The tray app: panel UI, tray icon and menu, shortcuts
crates/monitorium-core/      Settings, the monitor model and the background brightness worker
crates/monitorium-platform/  Windows code: DDC/CI, laptop screens, HDR, dimming, start with Windows
assets/icons/                App, tray and installer icons
```

Talking to monitors can be slow and some monitors stop responding for a while, so all monitor
communication happens on a background thread and the panel never freezes.

## Platform support

Windows 10 and 11 (x64 and ARM64). The code is structured for macOS as well, but the macOS side
isn't written yet.

## License

MIT
