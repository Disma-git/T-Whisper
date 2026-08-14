// Tillfälligt röktest för Nemotron-motorn: läser en 16 kHz mono 16-bit
// PCM-WAV och transkriberar den med samma parakeet-rs-anrop som appen.
// Körs med: cargo run --release --features cuda --example nemotron_smoke -- fil.wav [språk]

fn main() -> anyhow::Result<()> {
    let wav_path = std::env::args().nth(1).expect("ange en wav-fil");
    let lang = std::env::args().nth(2).unwrap_or_else(|| "en".into());
    let wav = std::fs::read(&wav_path)?;
    let data_pos = wav
        .windows(4)
        .position(|w| w == b"data")
        .expect("ingen data-chunk i wav-filen")
        + 8;
    let samples: Vec<f32> = wav[data_pos..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect();
    println!(
        "ljud: {} samples ({:.1} s)",
        samples.len(),
        samples.len() as f32 / 16000.0
    );

    let dir = dirs::config_dir()
        .unwrap()
        .join("T-Whisper")
        .join("models")
        .join("nemotron-3.5-asr-streaming-0.6b");
    let n_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let exec = parakeet_rs::ExecutionConfig::new().with_intra_threads(n_threads);
    #[cfg(feature = "cuda")]
    let exec = exec.with_execution_provider(parakeet_rs::ExecutionProvider::Cuda);

    let t0 = std::time::Instant::now();
    let mut model = parakeet_rs::Nemotron::from_pretrained(&dir, Some(exec))?;
    println!("modelladdning: {:.1} s", t0.elapsed().as_secs_f32());
    if let Err(e) = model.set_target_lang(&lang) {
        println!("okänt språk '{lang}': {e} — automatisk detektering");
    }

    let t0 = std::time::Instant::now();
    let text = model.transcribe_audio(&samples)?;
    println!("inferens: {:.2} s", t0.elapsed().as_secs_f32());
    println!("TEXT: {text}");
    Ok(())
}
