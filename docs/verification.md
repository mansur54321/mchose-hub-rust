# Verification

Verification distinguishes protocol tests, Linux device checks and Windows portability. A successful build alone does not verify a physical USB device.

## Physical devices on Linux

The A7 V3 Ultra+ 8K receiver and MagDock were exercised on Linux through native HID access. The mouse reported firmware V17 and MagDock reported 0003 during these checks. Downloaded mouse V30 and dock 0004 images were not flashed.

- DPI and MagDock brightness writes were backed up, applied, read back and restored.
- Music mode was checked with system playback capture and actual RGB streaming. Stopping it restored the preceding dock effect.
- The charging indicator was checked while the user placed the mouse on MagDock and removed it. During 68.487 seconds of charging, 1,350 RGB frames were sent and battery state advanced from 98% to 99%. Removing the mouse produced charge status 0 and restored the original white Flow effect, including brightness, speed and direction.
- Charging status 2 is handled as external power; its physical interpretation is inferred from the firmware GPIO branches and was not independently tested with a fully charged mouse.

Device captures, original backups and desktop recordings remain local. Sanitized response fixtures used by automated tests are included in the repository.

## Automated coverage

The tests cover frame validation, sleeping-device responses, profile and macro serialization, local import rollback, checked dock writes, music mode normalization, charging colors and restoration after state transitions or errors. Tests use mock transports and do not flash firmware.

The local Linux `cargo test --locked --all-targets` run passed 70 library tests and one roundtrip example test. Localization checks cover placeholders, user names and paths, nested errors and UI catalog coverage. Windows report padding and write completion conventions are tested without hardware.

Native Windows CI also passed 70 library tests and one example test. In particular, saving an edited local profile preserves the original bytes in a backup and completes successfully on Windows. Nine isolated Linux installer tests cover migration from the earlier application name, repeated installs and preservation of unrelated files and user desktop-entry additions.

One legacy X11 test is ignored by default because it needs an isolated X server and input-focus setup. It is separate from the platform-independent macro recording tests.

The GitHub workflow builds and tests both platforms, runs CLI checks without attached devices, and packages the executable with the original firmware and notices. It does not run hardware-writing examples.

Native Linux interface checks cover all seven pages, dark/light themes, widths from 855 to 1440 pixels, RU/EN switching and persistence across restart. Windows executables were additionally checked under an isolated Wine prefix for hardware-free tests and offline dark/light rendering. Wine checks do not replace physical Windows device or audio testing.

## Windows limits

The Windows implementation uses hidapi's native Windows backend, the application's physical key events and WASAPI loopback for system audio. Output reports account for Windows HID padding and the backend's synchronous-write return value. The same protocol logic and settings validation are shared with Linux.

Physical A7/MagDock operation and real WASAPI playback capture on Windows have not been verified on a Windows machine. Linux hardware results do not establish those Windows runtime results. Windows-specific build details are in [windows.md](windows.md).

## Firmware limits

Every bundled BIN matches the source size and SHA-256 in the manifest. Transfer code requires explicit confirmation, validates role and official contents, and checks transport replies. No firmware was flashed during development of this release. Boot recovery, power-loss recovery and acceptance of modified firmware remain unverified.
