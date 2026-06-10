use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Uppdaterar den delade toppnivån (f32 lagrad som bitar i AtomicU32) om
/// den nya toppen är högre. Nollställs av avläsaren via swap.
fn bump_peak(level: &AtomicU32, peak: f32) {
    let cur = f32::from_bits(level.load(Ordering::Relaxed));
    if peak > cur {
        level.store(peak.to_bits(), Ordering::Relaxed);
    }
}

/// Mikrofoninspelare. Strömmen är alltid igång men samplar buffras bara
/// när `active` är satt — det ger noll fördröjning vid push-to-talk-start.
pub struct Recorder {
    _stream: cpal::Stream,
    buf: Arc<Mutex<Vec<f32>>>,
    active: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
}

impl Recorder {
    /// `level` matas kontinuerligt med mikrofonens toppnivå (även när
    /// inspelning inte pågår) så att UI:t kan visa en nivåmätare.
    pub fn new(level: Arc<AtomicU32>) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow!("ingen mikrofon hittades"))?;
        eprintln!(
            "Mikrofon: {}",
            device.name().unwrap_or_else(|_| "okänd".into())
        );
        let supported = device.default_input_config()?;
        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels();
        let sample_format = supported.sample_format();
        let stream_cfg: cpal::StreamConfig = supported.into();

        let buf: Arc<Mutex<Vec<f32>>> = Arc::default();
        let active = Arc::new(AtomicBool::new(false));
        let b = buf.clone();
        let a = active.clone();
        let lvl = level.clone();
        let err_fn = |e| eprintln!("ljudfel: {e}");

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_cfg,
                move |data: &[f32], _| {
                    let peak = data.iter().fold(0f32, |m, s| m.max(s.abs()));
                    bump_peak(&lvl, peak);
                    if a.load(Ordering::Relaxed) {
                        b.lock().unwrap().extend_from_slice(data);
                    }
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_cfg,
                move |data: &[i16], _| {
                    let peak = data
                        .iter()
                        .fold(0f32, |m, s| m.max((*s as f32 / 32768.0).abs()));
                    bump_peak(&lvl, peak);
                    if a.load(Ordering::Relaxed) {
                        let mut b = b.lock().unwrap();
                        b.extend(data.iter().map(|s| *s as f32 / 32768.0));
                    }
                },
                err_fn,
                None,
            )?,
            f => return Err(anyhow!("ljudformatet stöds inte: {f:?}")),
        };
        stream.play()?;

        Ok(Self {
            _stream: stream,
            buf,
            active,
            sample_rate,
            channels,
        })
    }

    pub fn start(&self) {
        self.buf.lock().unwrap().clear();
        self.active.store(true, Ordering::Relaxed);
    }

    /// Stoppar buffringen och returnerar ljudet som 16 kHz mono f32 (whispers indataformat).
    pub fn stop(&self) -> Vec<f32> {
        self.active.store(false, Ordering::Relaxed);
        let raw = std::mem::take(&mut *self.buf.lock().unwrap());
        to_whisper_input(&raw, self.channels, self.sample_rate)
    }
}

fn to_whisper_input(raw: &[f32], channels: u16, rate: u32) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    let mono: Vec<f32> = raw
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect();
    if rate == 16_000 {
        return mono;
    }
    // Linjär interpolation räcker gott för tal.
    let ratio = rate as f64 / 16_000.0;
    let out_len = (mono.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let i0 = pos as usize;
            let frac = (pos - i0 as f64) as f32;
            let a = mono[i0];
            let b = *mono.get(i0 + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}
