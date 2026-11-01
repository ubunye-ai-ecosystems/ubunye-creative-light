//! Windows only: embed the app icon and version info (VERSIONINFO) into `lightcraft.exe`, so
//! Explorer, the taskbar, the Start menu and Alt-Tab show the app icon.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `LIGHTCRAFT_REQUIRE_WINRES=1` turns it
//! into an error (for release builds).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/lightcraft.ico");
    println!("cargo:rerun-if-env-changed=LIGHTCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/lightcraft.ico")
        .set("CompanyName", "Ubunye AI Ecosystems")
        .set("ProductName", "Ubunye Light")
        .set("FileDescription", "Ubunye Light photo library and raw developer")
        .set("LegalCopyright", "Copyright (c) the LightCraft authors and Ubunye AI Ecosystems. MIT OR Apache-2.0.")
        .set("OriginalFilename", "lightcraft.exe")
        .set("InternalName", "lightcraft");
    if let Err(e) = res.compile() {
        if std::env::var_os("LIGHTCRAFT_REQUIRE_WINRES").is_some() {
            panic!("embedding Windows resources failed: {e}");
        }
        println!("cargo:warning=lightcraft.exe built without icon/version resources: {e}");
    }
}
