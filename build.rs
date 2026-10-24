// Значок exe и сведения о файле — ресурс Windows из `assets/icon/kamerton.ico`.
// Без компилятора ресурсов (rc.exe из Windows SDK) сборка не падает: exe выходит без значка.

use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let ico = dir.join("assets/icon/kamerton.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().expect("utf-8 path"));
    res.set("ProductName", "Kamerton");
    res.set("FileDescription", "Kamerton — tray volume mixer");
    if let Err(e) = res.compile() {
        println!("cargo:warning=exe icon skipped: {e}");
    }
}
