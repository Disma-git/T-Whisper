fn main() {
    // Bädda in app-ikonen i exe:n (syns i Utforskaren, Start-menyn osv.).
    if std::path::Path::new("assets/t-whisper.ico").exists() {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/t-whisper.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=kunde inte bädda in ikonen: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/t-whisper.ico");
}
