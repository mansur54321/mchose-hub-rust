fn main() {
    println!("cargo:rerun-if-changed=assets/mchose-icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/mchose-icon.ico");
        resource.set("ProductName", "MCHOSE Hub Rust");
        resource.set(
            "FileDescription",
            "Native controller for MCHOSE A7 V3 Ultra+ and MagDock",
        );
        resource
            .compile()
            .expect("Failed to compile the Windows icon resource");
    }
}
