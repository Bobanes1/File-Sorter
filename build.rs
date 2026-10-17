//! Embeds the app icon and version info into the Windows exe.
//!
//! Without an embedded icon, Windows shows a blank icon for a pinned taskbar
//! shortcut and in Explorer, even though the running window has one.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "File Sorter")
        .set("FileDescription", "File Sorter")
        .set("CompanyName", "File Sorter")
        .set("LegalCopyright", "GPL-3.0-or-later");
    if let Err(e) = res.compile() {
        // Release builds on Windows (CI) have the resource compiler; cross-checks from
        // Linux may not. The CI checks that the icon really ended up in the exe.
        println!("cargo:warning=could not embed the Windows icon: {e}");
    }
}
