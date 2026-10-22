# Windows build and native backends

The Windows application uses the same Rust/egui interface and codecs as Linux. It requires Windows 10 or newer, x64, and an OpenGL-capable graphics driver. It does not embed a browser.

## Run or install

Extract the complete Windows package and open `mchose-hub-rust.exe`. To install it for the current user with a Start menu shortcut, run PowerShell from the extracted directory:

```powershell
powershell -ExecutionPolicy Bypass -File .\install-windows.ps1 -Language en
```

Add `-DesktopShortcut` for a desktop shortcut. No administrator access is needed. The installer preserves an existing executable as `previous-<timestamp>.exe`. Application data lives separately in `%LOCALAPPDATA%\mchose-native`; installing a new executable does not replace profiles, macros or backups.

The portable executable can also run directly. `--lang ru` and `--lang en` select the language. For diagnostics, launch from PowerShell with `--list`, `--status`, `--snapshot` or `--help`. `--offline` starts the interface without opening HID devices.

## Build on Windows

Install stable Rust with the MSVC target and the Visual Studio C++ build tools, then:

```powershell
cargo test --locked --lib
cargo build --locked --release --bin mchose-hub-rust
```

`build.rs` embeds the official MCHOSE icon into the executable. The PNG version is embedded separately for the native window icon.

## Cross-build on Linux

Python 3.12 or later, Cargo and a working Linux C compiler are needed. The following installs the pinned official Rust compiler, matching host/Windows standard libraries and LLVM-MinGW **inside `.tools/`**. It does not configure rustup, replace the system compiler or edit shell startup files.

```sh
python3 scripts/setup-windows-cross.py
scripts/build-windows.sh
scripts/build-windows.sh --test # compile hardware-free Windows unit tests
```

Toolchain URLs and SHA-256 checksums are pinned in `scripts/windows-toolchains.json`. Downloads are verified before extraction. The result is `target/x86_64-pc-windows-gnullvm/release/mchose-hub-rust.exe`. The build sets `crt-static` so the LLVM unwinder is linked into the executable instead of requiring `libunwind.dll`. Windows system DLLs remain normal runtime dependencies. A matching official compiler is required: a distribution-patched Rust compiler can reject an official standard library with E0514 even when version and commit match.

## Platform details and validation limits

Windows HID uses `hidapi` 2.6.7's `windows-native` backend (`hid.dll`). MagDock's unnumbered output is padded to its descriptor length: report ID zero plus 64 payload bytes. Mouse output retains its numbered report ID and totals 64 bytes. The backend itself removes the leading zero from unnumbered input, so MagDock's decoder still receives `AA` first. Hardware-free tests cover descriptor padding and the backend's synchronous/asynchronous write completion conventions. These details were checked against the [pinned hidapi implementation](https://docs.rs/crate/hidapi/2.6.7/source/src/windows_native/mod.rs).

Music mode captures the default **playback** endpoint through [WASAPI loopback](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording), in shared mode, after the user applies Music. It does not open a microphone, write audio files or require PipeWire. COM and the audio client stay on one owned worker thread; stopping capture releases that thread and stream. Shared-mode conversion provides mono float32 at 16 kHz for the same FFT/LED mapping used on Linux. Silent/null WASAPI buffers and endpoint inactivity produce zero samples. Capture errors reach the interface. See the [wasapi AudioClient API](https://docs.rs/wasapi/0.26.0/wasapi/struct.AudioClient.html).

Keyboard and mouse macro recording uses the existing native winit window event hook on both platforms. The legacy X11 test recorder is compiled only for Linux tests.

A successful cross-build and Wine/offline checks establish executable structure and portable logic. They do not prove physical USB communication or live WASAPI loopback on a Windows machine; those require a Windows host with the mouse and dock attached.
