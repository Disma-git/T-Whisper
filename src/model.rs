use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MODEL_FILE: &str = "ggml-model-q5_0.bin";
const VAD_MODEL_FILE: &str = "ggml-silero-v5.1.2.bin";

/// ONNX-artefakterna som Nemotron-motorn behöver (parakeet-rs läser dem
/// från en katalog). Färdigexporterade av parakeet-rs-projektet.
const NEMOTRON_DIR: &str = "nemotron-3.5-asr-streaming-0.6b";
const NEMOTRON_BASE_URL: &str =
    "https://huggingface.co/altunenes/parakeet-rs/resolve/main/nemotron-3.5-asr-streaming-0.6b-onnx";
const NEMOTRON_FILES: [&str; 4] = [
    "encoder.onnx",
    "encoder.onnx.data",
    "decoder_joint.onnx",
    "tokenizer.model",
];

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
    download_file(&url, &path)?;
    Ok(path)
}

/// Ser till att Nemotron-modellens ONNX-filer finns lokalt; laddar annars
/// ner dem från Hugging Face. Returnerar katalogen med filerna.
pub fn ensure_nemotron_model() -> Result<PathBuf> {
    let dir = crate::config::config_dir().join("models").join(NEMOTRON_DIR);
    std::fs::create_dir_all(&dir)?;
    for file in NEMOTRON_FILES {
        let path = dir.join(file);
        if path.exists() {
            continue;
        }
        let url = format!("{NEMOTRON_BASE_URL}/{file}");
        download_file(&url, &path)?;
    }
    Ok(dir)
}

/// Laddar ner en fil till `path` via en .part-fil (avbruten nedladdning
/// lämnar aldrig en halv fil på målplatsen). Loggar förlopp var 50:e MB.
fn download_file(url: &str, path: &Path) -> Result<()> {
    eprintln!("Laddar ner modellfil (första starten): {url}");

    let mut resp = reqwest::blocking::get(url).context("nedladdningen misslyckades")?;
    if !resp.status().is_success() {
        bail!("HTTP {} vid hämtning av {url}", resp.status());
    }
    let total_mb = resp.content_length().unwrap_or(0) >> 20;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("ogiltigt filnamn")?;
    let tmp = path.with_file_name(format!("{file_name}.part"));
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
    std::fs::rename(&tmp, path)?;
    eprintln!("Modellfil sparad: {}", path.display());
    Ok(())
}

/// Ser till att Silero-VAD-modellen (<1 MB) finns lokalt; laddar annars
/// ner den från Hugging Face. Används av kontinuerligt läge.
pub fn ensure_vad_model() -> Result<PathBuf> {
    let dir = crate::config::config_dir().join("models");
    let path = dir.join(VAD_MODEL_FILE);
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir)?;

    let url = format!("https://huggingface.co/ggml-org/whisper-vad/resolve/main/{VAD_MODEL_FILE}");
    crate::winutil::log(&format!("Laddar ner VAD-modell: {url}"));

    let resp = reqwest::blocking::get(&url).context("nedladdningen misslyckades")?;
    if !resp.status().is_success() {
        bail!("HTTP {} vid hämtning av {url}", resp.status());
    }
    let bytes = resp.bytes().context("nedladdningen avbröts")?;

    let tmp = dir.join(format!("{VAD_MODEL_FILE}.part"));
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    crate::winutil::log(&format!("VAD-modell sparad: {}", path.display()));
    Ok(path)
}
