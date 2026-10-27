# Official firmware and protocol findings

The repository includes eight unmodified images from the official MCHOSE CDN. The seven mouse/receiver/MagDock images were downloaded on October 22, 2026; the Ace75 8K image was downloaded on October 27. Full source URLs, sizes and SHA-256 digests are in [the manifest](../firmware/manifest.json).

| Target | Catalog version | Images |
|---|---:|---|
| A7 V3 Ultra+ mouse | 30 | `A7V3Ultra_Mouse_Main_V0030_*.bin`, `A7V3Ultra_Mouse_Sub_V0030_*.bin` |
| 8K receiver | 6 | `Mchose_Dongle_Main_V0006_*.bin`, `Mchose_Dongle_Sub_V0006_*.bin` |
| 1K receiver | 11 | `MCHOSE_DONGLE_MAIN_V0011_*.bin`, `Mchose_Dongle_Sub_564C_V0011_*.bin` |
| MagDock | 0004 | `MagDock0004.bin` |
| Ace 75 8K keyboard | 117 | `Ace75_8K_V117_CS6698734A20260727_okby9heultrya4ba.bin` |

Main and Sub are the two controllers inside the selected target. They do not mean “mouse and receiver.” A correct pair and hardware role are required. The application checks the selected files against the official CDN before transfer; firmware images in this repository do not bypass that check.

Ace75 uses a single full image, not a Main/Sub pair. Its current bundled V117 file is 229800 bytes, SHA-256 `180c5a25afe55125dad36c607de93d0b5a2eb25b11db04b029ad1b0f52414e0e`. This is the 8K model, not Ace75 16K. Runtime checks fetch the official production catalog again and compare selected file contents against its current image; the saved manifest is provenance, not an authorization to flash arbitrary files offline.

## Ace75 transfer protocol

The keyboard SDK describes a normal `3837:303c` to Upgrader `3837:303d` transition, followed by erase `81`, 32-byte write chunks `80`, compare chunks `82`, end `83` and success/restart `84`. The full BIN is transferred; no guessed header is removed. The application requires a single matching keyboard/bootloader, validates the exact control collection, observes fresh USB enumeration and checks every write/compare acknowledgment. A missing final restart acknowledgment is tolerated only after all transfer verification acknowledgments and a normal-mode device returns with the expected firmware version.

Before a normal-mode update, the controller stops host activity and saves raw settings and macros. A bootloader-only recovery cannot read normal settings first. The update requires explicit confirmation and an official-image check; do not disconnect USB during transfer. Neither a physical Ace75 flash nor power-loss/boot recovery has been performed in development. The coordinator checks unique model/role selection; it does not establish USB-port continuity between the normal and bootloader identities.

## Findings used by the application

- Mouse command `0900` carries charging state and battery percentage separately. ARM Mouse V30 derives charging states 0/1/2 from two input bits, independently of the percentage. State 1 was confirmed during live charging. State 2 is consistent with a different externally powered phase, such as charge completion; the exact charging circuit was not obtained.
- The receiver generates `FF` when it does not receive a matching remote reply within approximately one second. This response has no ordinary XOR checksum. The decoder accepts only the captured timeout shape for known reads, treats it as a failed read and retains checksum checks for ordinary responses and writes.
- MagDock Music mode is identified by `sync=1`. Its state response can retain the previous ordinary effect in the mode byte. Comparing that byte directly with Music incorrectly rejects a successful write.
- In MagDock status `07`, payload byte 9 repeats the blue channel. The actual sleep fields are bytes 10 and 11. This differs from the web client's parser and was independently checked with read command `02` on MagDock 0003.
- MagDock command `29` accepts 36 bytes: twelve RGB triples. It supports the charging ring without modifying the firmware. A watchdog eventually restores the previous effect if frames stop; the application restores the saved settings explicitly instead of relying on it.
- Repeated Music selection can overwrite the firmware's remembered previous effect. The application maintains its own restoration state.

## Analysis and validation limits

The seven original mouse/receiver/MagDock images received REA/Binwalk inventories. Selected ARM mouse and receiver functions were inspected through REA/Hopper. The installed REA native provider did not support the RISC-V images, so those functions were examined separately with LLVM and the WCH compressed-instruction extension. Ace75 packet and transfer behavior was reconstructed from the distributed SDK and executed golden helpers; no complete Ace75 BIN decompilation is claimed.

The analysis covers official Mouse V30, receiver V6/V11 and MagDock 0004 images. Live captures used the connected devices, including an earlier Mouse V17 and MagDock 0003. A downloaded image is not a dump of the installed firmware. Full equivalence across versions, modified-image acceptance, recovery after a failed flash and the physical orientation of LED0 have not been established.

Research toolchains, local device backups, desktop captures and session logs are kept outside the published repository. The small captured protocol fixtures needed by the tests are included under `research/` and `tests/fixtures/`.
