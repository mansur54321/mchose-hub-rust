# Official firmware and protocol findings

The repository includes seven unmodified images downloaded from the official MCHOSE CDN on October 22, 2026. The downloads were repeated and matched the saved SHA-256 digests. Full source URLs and digests are in [the manifest](../firmware/manifest.json).

| Target | Catalog version | Images |
|---|---:|---|
| A7 V3 Ultra+ mouse | 30 | `A7V3Ultra_Mouse_Main_V0030_*.bin`, `A7V3Ultra_Mouse_Sub_V0030_*.bin` |
| 8K receiver | 6 | `Mchose_Dongle_Main_V0006_*.bin`, `Mchose_Dongle_Sub_V0006_*.bin` |
| 1K receiver | 11 | `MCHOSE_DONGLE_MAIN_V0011_*.bin`, `Mchose_Dongle_Sub_564C_V0011_*.bin` |
| MagDock | 0004 | `MagDock0004.bin` |

Main and Sub are the two controllers inside the selected target. They do not mean “mouse and receiver.” A correct pair and hardware role are required. The application checks the selected files against the official CDN before transfer; firmware images in this repository do not bypass that check.

## Findings used by the application

- Mouse command `0900` carries charging state and battery percentage separately. ARM Mouse V30 derives charging states 0/1/2 from two input bits, independently of the percentage. State 1 was confirmed during live charging. State 2 is consistent with a different externally powered phase, such as charge completion; the exact charging circuit was not obtained.
- The receiver generates `FF` when it does not receive a matching remote reply within approximately one second. This response has no ordinary XOR checksum. The decoder accepts only the captured timeout shape for known reads, treats it as a failed read and retains checksum checks for ordinary responses and writes.
- MagDock Music mode is identified by `sync=1`. Its state response can retain the previous ordinary effect in the mode byte. Comparing that byte directly with Music incorrectly rejects a successful write.
- In MagDock status `07`, payload byte 9 repeats the blue channel. The actual sleep fields are bytes 10 and 11. This differs from the web client's parser and was independently checked with read command `02` on MagDock 0003.
- MagDock command `29` accepts 36 bytes: twelve RGB triples. It supports the charging ring without modifying the firmware. A watchdog eventually restores the previous effect if frames stop; the application restores the saved settings explicitly instead of relying on it.
- Repeated Music selection can overwrite the firmware's remembered previous effect. The application maintains its own restoration state.

## Analysis and validation limits

All seven images received REA/Binwalk inventories. Selected ARM mouse and receiver functions were inspected through REA/Hopper. The installed REA native provider did not support the RISC-V images, so those functions were examined separately with LLVM and the WCH compressed-instruction extension.

The analysis covers official Mouse V30, receiver V6/V11 and MagDock 0004 images. Live captures used the connected devices, including an earlier Mouse V17 and MagDock 0003. A downloaded image is not a dump of the installed firmware. Full equivalence across versions, modified-image acceptance, recovery after a failed flash and the physical orientation of LED0 have not been established.

Research toolchains, local device backups, desktop captures and session logs are kept outside the published repository. The small captured protocol fixtures needed by the tests are included under `research/` and `tests/fixtures/`.
