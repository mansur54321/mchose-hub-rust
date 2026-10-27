# Ace 75 8K

Ace75 support implements the device controls found in the official Hub's keyboard interface in native Rust/egui. It uses direct HID access; no JavaScript SDK, browser engine or cloud account runs inside the application. The existing mouse and MagDock views remain available through the device selector.

## Exact hardware

| Role | USB identity | Configuration collection |
| --- | --- | --- |
| Ace 75 8K | `3837:303c`, product `Ace 75` | Usage page `0001`, usage `0000` |
| Ace 75 Upgrader | `3837:303d` | Usage page `ff00`, usage `0001` |

The normal interface uses unnumbered 64-byte input/output payloads. Its ordinary keyboard, media and mouse input collections are not opened for polling. The 16K model uses a different PID (`301d`) and is not supported by this driver. The 8K model is wired: battery, wireless pairing, Bluetooth and 16/32 kHz controls are not exposed.

The keyboard layout has 81 ordinary keys, RT and wheel controls, a hidden RT double-click assignment, a 28-position strip, a logo and extra Space lighting positions. Those map into 128 protocol slots. LED-only and reserved slots must not be treated as physical magnetic keys.

## Native pages

| Page | Controls |
| --- | --- |
| Keys | Windows, Windows Fn, Mac and Mac Fn layers; single/multiple key selection; key, modifier/chord, media, mouse, macro and device actions. RT single/double-click assignments are global across all four layers. Unknown assignments remain representable as protocol bytes. |
| Triggers | Actuation, press/release Rapid Trigger, continuous RT, precision, top/bottom dead zones and switch selection. Calibration starts/stops/restarts explicitly; monitoring displays travel and per-key completion state. |
| Advanced | RS, SOCD, DKS, MT, TGL and LT. Paired actions require reciprocal key references. Advanced editing is available on the default Windows/Mac layers, as in Hub; Fn layers remain remappable on the Keys page. |
| Performance | Polling at 1/2/4/8 kHz, RTSmart, debounce optimization, bottom RT stability, adaptive travel, Mac mode, Win/Alt-Tab/Alt-F4 locks and lighting sleep. Stored scan-rate bits are preserved, rather than presented as an unsupported selector. |
| Lighting | Main effects, strip and logo settings; brightness 0–100%, speed, supported direction, mono/rainbow color and RGB calibration. Per-position color painting, native music lighting and a local static/animated theme editor. |
| Macros | Sixteen onboard slots, keyboard/modifier/mouse actions, delays and loop modes; local editing, import/export and recording from this application's focused window. Names/editor preferences are stored locally because they are not guaranteed to exist in the hardware bank. |
| Profiles | Enabled onboard profiles, order, active profile and names; local profile library, preview/edit/apply, Hub import/export and confirmed reset. Reads do not shrink the enabled onboard count to the website's three-profile presentation. |
| Device | Exact identity and firmware, refresh, official catalog check/download and explicitly confirmed firmware transfer. A connected bootloader can be selected for recovery transfer; physical recovery has not been tested. |

Trigger limits depend on switch travel and precision. The UI uses 0.01/0.005/0.001 mm precision modes; raw 9-bit values are storage units, not a promise that every switch supports 5.12 mm of travel. The latest switch option requires firmware 117. Macro delays start at 5 ms.

Firmware-specific lighting gates matter. Logo follow mode and its custom color range require 115. The production configuration sets the newer `GLWRemapkableLightV2` capability to **118**, even though a bundled SDK table says 117; it must stay disabled on firmware 117. The normal effect lists, per-position colors and native host playback are separate from that newer capability. No unsupported on-device GIF storage or live LED-read capability is claimed.

## Host lighting and files

Music captures the system playback endpoint through PipeWire on Linux or WASAPI on Windows. The microphone is not selected. The native FFT renderer maps spectrum bars to the captured keyboard geometry and can target the main keys, strip and logo separately. The renderer implements this feature natively; the original parent-page music visualization algorithm was not recovered in this bounded analysis.

Themes accept native JSON, Hub static/animated lighting JSON, GIF, PNG, JPEG and WebP. Image pixels are sampled at the real LED/key positions. GIF frames are composited by the Rust image decoder, and frame durations drive a looping host player. Overdue frames are skipped rather than accumulating a queue. The website's parent `TRANSFORM_GIF` resampling implementation was not observed, so imported GIFs are not claimed to match it pixel for pixel.

Themes use up to 512 frames, 128 RGB positions per frame, 5–60000 ms durations and a 32 MiB input-file limit. GIF decoding also has dimension and decoded-content limits. Save a theme with `.hub.json` to export Hub lighting JSON; other filenames save the native format. Hub animation import/export retains supported colors and timing, not arbitrary vendor envelope metadata.

Music and animation require the host application to remain running. Their packets target protocol profile zero, matching the SDK. Stopping playback restores the saved lighting of the active onboard profile and checks its settings. Music, animation and calibration are coordinated so they do not concurrently control the same device.

Keyboard profile JSON is separate from lighting JSON. Supported Hub keyboard fields are merged with the target profile, preserving unspecified device settings and the original envelope for export. Macro references in ordinary layers and advanced actions are resolved before writing: equivalent definitions are reused, otherwise a free slot is allocated. Existing nonempty macro slots are not overwritten to make room for an import.

## Linux access

Run the GUI as the regular desktop user. Check the current USB mode before diagnosing permissions; normal and Upgrader identities differ. If local `hidraw` access is missing, add these narrowly scoped rules to a dedicated file under `/etc/udev/rules.d/`, preserving and backing up any existing file first:

```udev
SUBSYSTEM=="hidraw", KERNEL=="hidraw*", ATTRS{idVendor}=="3837", ATTRS{idProduct}=="303c", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="hidraw", KERNEL=="hidraw*", ATTRS{idVendor}=="3837", ATTRS{idProduct}=="303d", MODE="0660", TAG+="uaccess"
```

Reload the rules and reconnect the keyboard. A working file open proves OS access; successful protocol reads are a separate check. The development machine already had working rules for both identities; no system permission changes were needed for this implementation.

```sh
mchose-hub-rust --device ace75 --status
mchose-hub-rust --device ace75 --snapshot
mchose-hub-rust --device ace75 --read-only
```

## Evidence and current verification

The official [keyboard Hub](https://www.mchose.com.cn/#/keyboard) resources were captured on October 27, 2026. Protocol and capability reconstruction used its [distributed SDK](https://www.mchose.com.cn/cizhou/CZ_SHARED_DATA/main.504a955bfd84363d1e76.js), SHA-256 `c16055520ed1c43483a5357f74e265e3a356b0f7d2a4298a3d90ad6cf0a127ff`, and the [production keyboard configuration](https://cdn.mchose.com.cn/configCenter/custom/keyboardConfig.json). Fresh remote configuration takes precedence over the bundled table.

Golden tests execute original SDK packet/codec helpers in an isolated Node VM with fake transport only. The product does not include that SDK. See [fixture provenance](../tests/fixtures/ace75/README.md) for covered commands and the distinction between executed helper code and real device traffic.

An actual Rust read on Linux succeeded against firmware 117: base selection, four layers, all 128 trigger/color positions, advanced records, custom data and the shared 8 KiB macro bank of the active profile. Info responses have 31 valid payload bytes despite a 56-byte request; the decoder respects that declared size. Device snapshots and macros remain private.

A controlled lighting write also succeeded: main brightness 20 → 21 → 20, with both readbacks verified and full raw snapshots of all four profiles/macros identical before and after restoration. A separate native music check captured nonzero PipeWire frames and streamed all three zones, then restored lighting with identical raw backups; visual output was not independently observed. These checks do not prove key/trigger/macro writes, calibration, animation playback, Windows USB operation or firmware flashing. The current results are recorded in [verification.md](verification.md). Firmware protocol reconstruction is based on the SDK; it is not evidence that the Ace75 BIN has been completely decompiled.
