use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::f32::consts::TAU;
use std::time::Duration;

/// Spelar en kort ton asynkront (avfyra och glöm).
pub fn play(freq: f32, ms: u64, gain: f32) {
    std::thread::spawn(move || {
        if let Err(e) = play_blocking(freq, ms, gain) {
            eprintln!("kunde inte spela feedbackljud: {e}");
        }
    });
}

fn play_blocking(freq: f32, ms: u64, gain: f32) -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("ingen ljudutgång hittades"))?;
    let supported = device.default_output_config()?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Ok(()); // hoppa över feedbackljud på ovanliga format
    }
    let rate = supported.sample_rate().0 as f32;
    let channels = supported.channels() as usize;
    let cfg: cpal::StreamConfig = supported.into();

    let total = (rate * ms as f32 / 1000.0) as usize;
    let fade_in = ((rate * 0.005) as usize).max(1);
    let fade_out = ((rate * 0.02) as usize).max(1);
    let mut i = 0usize;

    let stream = device.build_output_stream(
        &cfg,
        move |data: &mut [f32], _| {
            for frame in data.chunks_mut(channels) {
                let v = if i < total {
                    let env_in = (i as f32 / fade_in as f32).min(1.0);
                    let env_out = ((total - i) as f32 / fade_out as f32).min(1.0);
                    (i as f32 / rate * freq * TAU).sin() * gain * env_in * env_out
                } else {
                    0.0
                };
                for s in frame.iter_mut() {
                    *s = v;
                }
                i += 1;
            }
        },
        |e| eprintln!("ljudfel: {e}"),
        None,
    )?;
    stream.play()?;
    std::thread::sleep(Duration::from_millis(ms + 60));
    Ok(())
}
