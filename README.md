<p align="center">
  <img src="assets/icons/128x128.png" width="96" alt="Monitorium logo">
</p>

<h1 align="center">Monitorium</h1>

<p align="center">
  Change the brightness of all your monitors from the system tray or the menu bar.
</p>

---

Monitorium is a small app for Windows and macOS. Click its icon in the tray (Windows) or the menu
bar (macOS) and a panel opens with a brightness slider for every connected monitor. It is written
in Rust with a native UI ([egui](https://github.com/emilk/egui)), so there's no browser engine
inside and it stays light on memory.

## Features

- **One slider per monitor**, or link them all together and move them as one.
- **Works with almost any screen.** Monitorium picks the best way to change brightness for each
  monitor on its own (see [How brightness is changed](#how-brightness-is-changed)).
- **Scroll over the tray or menu bar icon** to change brightness without opening anything.
- **Keyboard shortcuts** for brightness up, brightness down and turning displays off.
- **Turn off displays** with one click.
- **Starts when you sign in** and waits quietly in the tray or menu bar.
- **Light and dark theme**, following the system by default.
- Per-monitor options: custom name, hide from the panel, minimum and maximum brightness, and
  forcing software dimming.

## How brightness is changed

Every monitor uses one of these methods. The panel shows which one under the monitor's name.

| Method | Used for | Notes |
| --- | --- | --- |
| **DDC/CI** | Most external monitors | Changes the monitor's real backlight, like the buttons on the monitor do. |
| **Built-in** (Windows) | Laptop screens | The same control as the Windows brightness slider. |
| **Native** (macOS) | MacBook screens and Apple displays | The same control as the brightness keys. |
| **SDR content brightness** (Windows) | Monitors with HDR turned on | Most monitors lock their backlight in HDR mode, so Monitorium adjusts Windows' "SDR content brightness" instead. Can be turned off in Settings. |
| **Software (overlay)** | Monitors without DDC/CI | Puts a dark, click-through layer over the screen. Can dim almost to black. |
| **Software (gamma)** | Monitors without DDC/CI | Adjusts the screen's color curve instead of using a layer. Windows only allows about half brightness this way, and HDR screens use the overlay instead. On macOS it dims as far as the overlay, but Night Shift and apps like f.lux can undo it for a moment. |

You choose between overlay and gamma in **Settings → Brightness**. You can also force software
dimming for a single monitor in **Settings → Monitors**, for example if its DDC/CI misbehaves.

## Installing

### Windows

Download `monitorium_<version>_x64-setup.exe` and run it. The installer doesn't need
administrator rights; Monitorium is installed for your user only.

After the first start, Monitorium runs when you sign in to Windows. You can turn that off in
Settings or from the tray menu.

### macOS

Download `Monitorium_<version>_aarch64.dmg`, open it and drag Monitorium into Applications. Open
it from there: Monitorium isn't notarized by Apple, so the first time macOS won't open it. Go to
**System Settings → Privacy & Security**, click **Open Anyway** next to the message about
Monitorium, and confirm.

Monitorium lives in the menu bar and has no Dock icon. After the first start it opens when you
log in (it shows up under **System Settings → General → Login Items**). You can turn that off in
Settings or from the menu bar icon's menu.

## Using it

| Action | Windows | macOS |
| --- | --- | --- |
| Open the brightness panel | Left-click the tray icon | Click the menu bar icon |
| Menu (refresh, turn off displays, settings, quit) | Right-click the tray icon | Right-click the menu bar icon |
| Change brightness quickly | Scroll the mouse wheel over the tray icon | Scroll over the menu bar icon |
| Brightness up / down | `Ctrl + Alt + ↑` / `Ctrl + Alt + ↓` | `⌃ ⌥ ⌘ ↑` / `⌃ ⌥ ⌘ ↓` |
| Close the panel | Click anywhere else, or press `Esc` | Click anywhere else, or press `Esc` |

The shortcuts can be changed in Settings.

> [!TIP]
> Windows sometimes hides new tray icons behind the **^** arrow. Drag the icon onto the taskbar
> to keep it visible. On a Mac with a full menu bar, macOS hides icons that don't fit, for
> example behind the camera notch. Open Monitorium again from Applications or Spotlight to show
> the panel anyway.

### Command line

```text
monitorium --list          Show detected monitors and how their brightness is controlled
monitorium --set 70        Set every monitor to 70% and exit
monitorium --autostart     Start hidden in the tray (used when starting at sign-in)
```

On macOS the program is inside the app:
`/Applications/Monitorium.app/Contents/MacOS/monitorium --list`.

`--set` changes DDC/CI, built-in, native and SDR brightness. Software dimming only lasts while
the tray app is running, so use the tray app for monitors that need it.

## Troubleshooting

**The slider moves but the screen doesn't change.**
On Windows, check whether HDR is on (Windows Settings → System → Display). If it is, make sure
**Settings → Brightness → Use SDR content brightness on HDR displays** is enabled. Otherwise check
that DDC/CI is enabled in your monitor's own menu. Some monitors ship with it off. macOS has no
SDR brightness setting, so if a monitor ignores DDC/CI while HDR is on, force software dimming for
it in **Settings → Monitors**.

**My monitor isn't listed, or only offers software dimming.**
Some docks, KVM switches, adapters and TVs don't pass DDC/CI through. Neither does the HDMI port
of some Macs, for example the M1 Mac mini; a USB-C or Thunderbolt connection usually works.
Software dimming still works on all of them. Click the refresh button in the panel after
connecting a monitor.

**Monitorium doesn't open at login on macOS.**
Check **System Settings → General → Login Items**. If Monitorium is switched off there, the
**Open at login** setting in Monitorium can't tell, so switch it back on in System Settings.

**Where are my settings?**
On Windows in `%APPDATA%\Monitorium\config` (open it from Settings → About). The log file is in
`%LOCALAPPDATA%\Monitorium\data\monitorium.log`. On macOS both are in
`~/Library/Application Support/Monitorium`.

## Building from source

You need [Rust](https://rustup.rs) (stable, 2024 edition), and on Windows the Visual Studio C++
build tools (the MSVC toolchain) or on macOS the Xcode command line tools
(`xcode-select --install`).

```bash
cargo build --release
```

On Windows the app is `target/release/monitorium.exe`. It is built with a static C runtime (see
`.cargo/config.toml`), so it runs without installing the Visual C++ redistributable. On macOS it
is `target/release/monitorium`.

### Making the installer

The installer is built with [cargo-packager](https://github.com/crabnebula-dev/cargo-packager),
configured in `Cargo.toml` under `[package.metadata.packager]`.

```bash
cargo install cargo-packager --locked
```

On Windows:

```bash
cargo packager --release
```

On macOS, which builds `Monitorium.app` and a disk image:

```bash
cargo packager --release --formats app,dmg
```

Everything is written to `dist/`. On macOS, laying out the disk image window uses Finder, which
macOS asks you to allow; set `CI=true` to skip that step. The app is signed ad hoc; to sign it
with your own certificate, change `signing-identity` under `[package.metadata.packager.macos]`.

### Project layout

```text
src/                         The tray app: panel UI, tray icon and menu, shortcuts
crates/monitorium-core/      Settings, the monitor model and the background brightness worker
crates/monitorium-platform/  Windows and macOS code: DDC/CI, built-in screens, HDR, dimming, start at sign-in
assets/icons/                App, tray and installer icons
```

Talking to monitors can be slow and some monitors stop responding for a while, so all monitor
communication happens on a background thread and the panel never freezes.

## Platform support

- Windows 10 and 11 (x64 and ARM64).
- macOS 11 or later on Macs with Apple silicon.

## License

MIT
