fn main() {
    // Bädda in app-ikonen i exe:n (syns i Utforskaren, Start-menyn osv.).
    // Bara vid Windows-byggen - resurskompilatorn finns inte på andra plattformar.
    let target_windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    if target_windows && std::path::Path::new("assets/t-whisper.ico").exists() {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/t-whisper.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=kunde inte bädda in ikonen: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/t-whisper.ico");
}
