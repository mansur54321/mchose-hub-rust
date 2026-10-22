![MCHOSE Hub Rust — MCHOSE and Rust](docs/images/banner.png)

# MCHOSE Hub Rust

[Русский](README.ru.md)

An **open-source native alternative to MCHOSE Hub**, built in Rust for **MCHOSE A7 V3 Ultra+ and MagDock** on Windows and Linux. Built with egui/eframe and direct HID access. No browser engine, Electron, WebView or cloud account.

Developed through **reverse engineering** of the publicly distributed MCHOSE Hub web application, its HID protocol and official firmware. This is an independent project, not an official MCHOSE product.

[Download Windows and Linux packages](https://github.com/mansur54321/mchose-hub-rust/releases/latest).

## Screenshots

The native application on Linux:

| Dark theme | Light theme / MagDock |
| --- | --- |
| ![MCHOSE Hub Rust button settings in dark mode](docs/images/interface-dark.png) | ![MCHOSE Hub Rust MagDock settings in light mode](docs/images/interface-light.png) |

## Features

- Battery and charging state, connection type, active profile and DPI, firmware versions.
- Button assignments, keyboard shortcuts, multimedia controls and onboard macros.
- Up to six DPI stages, independent X/Y values and supported sensor/performance settings.
- Three onboard profiles, local profile and macro libraries, M HUB JSON import/export.
- MagDock effects, brightness, speed, color and direction.
- Music lighting from the system playback output; the microphone is not selected.
- Charging ring: half yellow at 50%, full green at 100%, with smooth color and partial-segment transitions. The previous effect resumes when external charging power is removed.
- Russian/English interface and light/dark themes.
- Official firmware download and verified transfer with explicit confirmation.

Device writes are read back and checked. Backups are saved before settings are replaced. Reset and firmware-transfer actions require confirmation in the application. Macro recording observes input delivered to the application's own window and stops on focus loss.

## Run

On Windows, launch `mchose-hub-rust.exe`. On Linux:

```sh
./mchose-hub-rust
```

The application must remain running for music lighting and the charging ring. The charging signal comes from the mouse, so the ring also works when the mouse charges through a cable while MagDock is connected.

```sh
mchose-hub-rust --lang en
mchose-hub-rust --lang ru
mchose-hub-rust --read-only
mchose-hub-rust --offline
mchose-hub-rust --status
mchose-hub-rust --snapshot
mchose-hub-rust --list
```

`--read-only` disables device writes. `--offline` opens the local editors without HID access. `--status` and `--snapshot` print JSON; the latter also reads the device configuration. `--help` lists the remaining options.

Supported USB identity: VID `3837`, mouse PIDs `4033`/`1014`/`1018` with the exact product name `MCHOSE A7 V3 Ultra+`, and MagDock PID `1012`. Other devices sharing a PID are deliberately excluded. Bluetooth does not expose the configuration interface used by this utility.

## Build

Install stable Rust and build from the repository root:

```sh
cargo build --release --locked
cargo test --all-targets --locked
```

Linux additionally needs the development libraries for `libudev`, Wayland/XKB and OpenGL. On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libudev-dev libxkbcommon-dev libwayland-dev libgl-dev
```

Linux music lighting uses `pw-dump` and `pw-cat` from PipeWire. Windows music lighting uses WASAPI loopback. Device access on Linux requires an appropriate `hidraw` permission rule; the GUI should run as the regular user.

Install a built Linux application in the current user's menu:

```sh
python3 scripts/install-local.py
```

Build output is `target/release/mchose-hub-rust` on Linux and `target/release/mchose-hub-rust.exe` on Windows. The local `egui-winit` patch preserves physical key events for the macro recorder; see [its scope and licenses](vendor/egui-winit/MCHOSE-PATCH.md).

## Data and profiles

Local data is stored in the application's platform data directory. It contains `backups/`, `presets/`, `macros/` and saved UI preferences. Linux uses `${XDG_DATA_HOME:-$HOME/.local/share}/mchose-native`; Windows uses the local application-data directory.

The existing `mchose-native` data directory and Linux command alias are retained for compatibility with earlier builds.

Profiles and macros can be imported from M HUB JSON. If a profile references a macro only by name, import that macro into the local library first. Missing definitions stop an import before any device write. The application does not overwrite existing export files.

## Bundled firmware

The [firmware directory](firmware) contains seven unmodified official images for the mouse, both receiver variants and MagDock. [The manifest](firmware/manifest.json) records their source URLs, sizes and SHA-256 digests.

Main/Sub are the two controllers **inside one selected device**, not a mouse/receiver pair. Firmware transfer checks the exact device role and file contents against the official CDN, so an internet connection is required for that check. MagDock versions below 0003 require the manufacturer's separate transition utility.

See [firmware findings and limitations](docs/firmware.md) and [validation](docs/verification.md). A successful build or transfer receipt does not prove an update succeeded on physical hardware.

## Project scope

This is an independent implementation based on the publicly distributed MCHOSE Hub and captured device responses. It supports local device configuration; cloud accounts, shared online profiles, theme stores and other mouse models are outside its scope.

The MCHOSE logo, illustrations and firmware remain third-party materials. Origins and dependency notices are recorded in [NOTICES.md](NOTICES.md). Local desktop captures, device backups, browser profiles and research toolchains are excluded from the repository.

The project's original code is available under the [MIT License](LICENSE). Third-party artwork, firmware and vendored dependencies retain their own terms.
