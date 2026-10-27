# Third-party materials

MCHOSE Hub Rust is an independent compatibility application. It is not an official MCHOSE product and does not imply endorsement by MCHOSE.

The project's original code is licensed under [MIT](LICENSE). That license does not relicense the third-party materials listed below.

The MCHOSE name, logo, device illustrations and official firmware belong to their respective owners. Asset origins are recorded in [assets/provenance.json](assets/provenance.json); firmware download URLs, byte lengths and SHA-256 digests are recorded in [firmware/manifest.json](firmware/manifest.json). The firmware files are unmodified manufacturer images. They are not covered by the licenses of the Rust dependencies.

Ace75 layout geometry, default key tables and action/effect descriptions were derived from the publicly distributed MCHOSE Hub for device compatibility. Their provenance is recorded alongside the existing assets. Sanitized protocol responses and original-SDK golden vectors are identified in [tests/fixtures/ace75/README.md](tests/fixtures/ace75/README.md). The manufacturer's JavaScript SDK is not bundled or executed by the application. The Ace75 V117 BIN is an unmodified third-party firmware image; the project's MIT license does not relicense it.

The local `egui-winit` patch retains the upstream MIT and Apache-2.0 license texts in [vendor/egui-winit](vendor/egui-winit). Its scope is described in [MCHOSE-PATCH.md](vendor/egui-winit/MCHOSE-PATCH.md). Other Rust dependencies retain their respective licenses.

The Rust logo comes from the Rust project's [official artwork repository](https://github.com/rust-lang/rust-artwork/tree/main/logo) and is used under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Its shape is unchanged; PNG rendering and resizing are recorded in the asset provenance. Rust names and logos remain their respective owners' trademarks. This project does not imply endorsement by the Rust project or Rust Foundation.
