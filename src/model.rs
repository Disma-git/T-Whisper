use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::PathBuf;

const MODEL_FILE: &str = "ggml-model-q5_0.bin";

/// Ser till att GGML-modellen finns lokalt; laddar annars ner den från Hugging Face.
pub fn ensure_model(name: &str) -> Result<PathBuf> {
    let dir = crate::config::config_dir()
        .join("models")
        .join(format!("kb-whisper-{name}"));
    let path = dir.join(MODEL_FILE);
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir)?;

    let url = format!("https://huggingface.co/KBLab/kb-whisper-{name}/resolve/main/{MODEL_FILE}");
    eprintln!("Laddar ner modell (första starten): {url}");

    let mut resp = reqwest::blocking::get(&url).context("nedladdningen misslyckades")?;
    if !resp.status().is_success() {
        bail!("HTTP {} vid hämtning av {url}", resp.status());
    }
    let total_mb = resp.content_length().unwrap_or(0) >> 20;

    let tmp = dir.join(format!("{MODEL_FILE}.part"));
    let mut out = std::fs::File::create(&tmp)?;
    let mut buf = [0u8; 1 << 16];
    let mut done: u64 = 0;
    let mut last_report: u64 = 0;
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if done - last_report > 50 << 20 {
            last_report = done;
            eprintln!("  {} / {} MB", done >> 20, total_mb);
        }
    }
    out.flush()?;
    drop(out);
    std::fs::rename(&tmp, &path)?;
    eprintln!("Modell sparad: {}", path.display());
    Ok(path)
}
