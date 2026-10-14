//! Windows only: embed the app icon and version info into `matcraft.exe`. Elsewhere a no-op.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/matcraft.ico");
    println!("cargo:rerun-if-env-changed=MATCRAFT_REQUIRE_WINRES");
    println!("cargo:rerun-if-env-changed=MATCRAFT_BUILD_SHA");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    if std::path::Path::new("../../assets/app-icon/matcraft.ico").exists() {
        res.set_icon("../../assets/app-icon/matcraft.ico");
    }
    res.set("ProductName", "MatCraft")
        .set("FileDescription", "MatCraft numerical computing environment")
        .set("CompanyName", "Learning Machines LLC")
        .set("LegalCopyright", "Copyright (c) 2026 ArtCraft Team and the MatCraft contributors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "matcraft.exe")
        .set("InternalName", "matcraft");
    if let Err(e) = res.compile() {
        if std::env::var_os("MATCRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=matcraft.exe built without icon/version resources: {e}");
    }
}
