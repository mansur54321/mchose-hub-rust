# Verification

Verification distinguishes protocol tests, Linux device checks and Windows portability. A successful build alone does not verify a physical USB device.

## Physical devices on Linux

The A7 V3 Ultra+ 8K receiver and MagDock were exercised on Linux through native HID access. The mouse reported firmware V17 and MagDock reported 0003 during these checks. Downloaded mouse V30 and dock 0004 images were not flashed.

- DPI and MagDock brightness writes were backed up, applied, read back and restored.
- Music mode was checked with system playback capture and actual RGB streaming. Stopping it restored the preceding dock effect.
- The charging indicator was checked while the user placed the mouse on MagDock and removed it. During 68.487 seconds of charging, 1,350 RGB frames were sent and battery state advanced from 98% to 99%. Removing the mouse produced charge status 0 and restored the original white Flow effect, including brightness, speed and direction.
- Charging status 2 is handled as external power; its physical interpretation is inferred from the firmware GPIO branches and was not independently tested with a fully charged mouse.

Device captures, original backups and desktop recordings remain local. Sanitized response fixtures used by automated tests are included in the repository.

## Ace75 implementation checks

On October 27, 2026 the attached Ace 75 8K was identified as normal `3837:303c`, control usage `0001:0000`, with 64-byte unnumbered input/output payloads. Existing normal-user Linux access worked. Firmware was reported as `117`; the 16K model and ordinary keyboard input collections were excluded.

The actual Rust `--device ace75 --snapshot` path read the active profile completely: four layers of 128 assignments, 128 trigger records, 128 RGB positions, the advanced banks, the 1024-byte custom area, profile names and the shared 8192-byte macro bank. Info and Base reads were also checked independently against captured response framing. These were read operations, not a firmware dump or a settings-write test.

A two-second native music check captured PipeWire system playback and generated 30 frames with nonzero colors. Frames were streamed to the main, strip and logo zones; stopping capture restored the saved lighting and verified its settings. Full raw profiles/macros matched before and after this check. LED output was not independently observed, and the test does not establish visual equivalence with the website.

A later controlled write changed main lighting brightness from 20 to 21 and restored 20. Both writes were read back and verified. Full raw backups of all four profiles and the macro bank were compared before and after the roundtrip and matched exactly. This establishes the tested lighting write/readback/restoration path while preserving the other captured settings. Personal backups and the live roundtrip log remain local.

ACE75 golden fixtures execute the original distributed SDK's packet and codec helpers against a fake transport, covering checksums, chunk boundaries/overlap, performance and lighting fields, triggers, music zone packets and firmware packet construction. Additional mock tests cover advanced references, macro metadata preflight, profile import dependencies, host frame timing, lighting file validation and bootloader role selection. The SDK itself is not part of the native product.

The host music renderer and GIF-to-layout sampling are native implementations. Their output is not claimed to be identical to the website's parent bridge. Key remapping, trigger/advanced/macro changes, calibration, Windows USB operation, Windows system-audio streaming, firmware transfer and boot recovery require separate physical evidence; a lighting roundtrip or mock test does not establish them. Final build totals belong in the release record below.

## Automated coverage

The tests cover frame validation, sleeping-device responses, profile and macro serialization, local import rollback, checked dock writes, music mode normalization, charging colors and restoration after state transitions or errors. Tests use mock transports and do not flash firmware.

For the original v0.1.0 mouse/dock release, the local Linux `cargo test --locked --all-targets` run passed 70 library tests and one roundtrip example test. Localization checks covered placeholders, user names and paths, nested errors and UI catalog coverage. Windows report padding and write completion conventions were tested without hardware.

Native Windows CI for v0.1.0 also passed 70 library tests and one example test. In particular, saving an edited local profile preserved the original bytes in a backup and completed successfully on Windows. Nine isolated Linux installer tests covered migration from the earlier application name, repeated installs and preservation of unrelated files and user desktop-entry additions.

One legacy X11 test is ignored by default because it needs an isolated X server and input-focus setup. It is separate from the platform-independent macro recording tests.

The GitHub workflow builds and tests both platforms, runs CLI checks without attached devices, and packages the executable with the original firmware and notices. It does not run hardware-writing examples.

The v0.1.0 native Linux interface checks covered its seven mouse/dock pages, dark/light themes, widths from 855 to 1440 pixels, RU/EN switching and persistence across restart. Windows executables were additionally checked under an isolated Wine prefix for hardware-free tests and offline dark/light rendering. These earlier checks do not establish coverage of the new keyboard pages. Wine checks do not replace physical Windows device or audio testing.

## v0.2.0 release checks

The final local Linux all-targets run passed 157 library tests and one example test; one isolated X11 recorder test remains ignored by default. All nine installer tests passed. Keyboard backend and UI localization coverage checks found no missing Russian-to-English entries and preserved formatting placeholders.

The Ace75 native GUI was captured across eight pages, RU/EN, dark/light themes and 855×700/1440×940 sizes (64 combinations). Eight representative captures were repeated after layout fixes. Window-only input checks exercised multiple key selection, trigger drafts/discard, macro recording, focus-loss release, and local macro save/update with backups. The A7 view was also checked at 855×700 in offline light mode. These checks ran on an isolated X server and did not remap or calibrate the physical keyboard.

Linux and Windows release builds succeeded locally. An intermediate Windows test executable also passed 148 hardware-free library tests under Wine. The native CI runs and packaged artifacts are linked from the release; Wine is supplementary portability evidence.

## Windows limits

The Windows implementation uses hidapi's native Windows backend, the application's physical key events and WASAPI loopback for system audio. Output reports account for Windows HID padding and the backend's synchronous-write return value. The same protocol logic and settings validation are shared with Linux.

The user subsequently reported that v0.1.0 works on Windows. No per-feature Windows hardware test record was supplied, so this does not establish every A7/MagDock, WASAPI or firmware path. Ace75 physical Windows operation has not been established. Windows-specific build details are in [windows.md](windows.md).

## Firmware limits

Every bundled BIN matches the source size and SHA-256 in the manifest. Transfer code requires explicit confirmation, validates role and official contents, and checks transport replies. No firmware was flashed during development of this release. Boot recovery, power-loss recovery and acceptance of modified firmware remain unverified.

## v0.2.1 interface refresh

The local all-targets run passed 161 library tests and one example test, with the isolated legacy X11 test ignored. New regressions cover one-time WASD selection without configuration edits, separate frame/onboard color selections, Tab/Space activation of the native switch, and palette wrapping inside the actual scrolling inspector. The backend HID implementation is unchanged.

An initial 64-case render matrix covered all eight pages, both languages/themes and 855×700/1440×940 windows. Follow-up representative captures and actual window input checked the corrected side panes, A and Ctrl+A+D selection, RS/LT help, three trigger tabs, strip/logo selection, wrapped palette entries and the fourth DKS phase dropdown. Final wide/narrow trigger captures verify the original switch illustration and the compact layout with the editable slider in the first viewport. All these sessions used `--read-only`; no device settings were written.

The final screenshots in [dark](images/ace75-dark.png) and [light](images/ace75-light.png) are captures of the native application. Private inspection logs and intermediate screenshots remain excluded from the repository.

Design delivery gate, reviewed against `DESIGN.md`:

| Gate | Status | Evidence |
| --- | --- | --- |
| Hard gate | PASS | Native controls remain bound to existing actions; RU/EN, empty/read-only states and both window sizes were rendered and exercised. |
| Purpose | PASS | Official keyboard geometry, original switch/brand assets, source theme colors and grouped settings; each adaptation and its purpose is recorded in `DESIGN.md`. |
| Liveliness | PASS | ENERGY 1 / RHYTHM 2 / MOTION 1; the physical keyboard and selected settings are the focal point, with blue used for actual selection. |
| Craftsmanship | PASS | Pointer/keyboard checks and regression tests pass; text contrast is 15.00:1/15.91:1 on neutral caps, control borders 3.49:1/3.79:1, muted text 5.10:1/6.49:1, selected white text 4.77:1. |
