//! Встраивание иконки и метаданных в exe (Windows).

fn main() {
    #[cfg(windows)]
    {
        let icon = std::path::Path::new("assets/icon.ico");
        if icon.exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon.to_string_lossy().as_ref());
            res.set("ProductName", "YanLan");
            res.set("FileDescription", "YanLan — лаунчер Yandere Simulator");
            res.set("LegalCopyright", "Неофициальный проект");
            let _ = res.compile();
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
