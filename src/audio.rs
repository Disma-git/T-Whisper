use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Maximal inspelningslängd som ringbufferten rymmer.
const MAX_RECORD_SECS: usize = 90;

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
/// Ljudcallbacken är lås- och allokeringsfri (SPSC-ringbuffert).
pub struct Recorder {
    _stream: cpal::Stream,
    consumer: rtrb::Consumer<f32>,
    active: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
}

/// Föredrar 16 kHz (whispers indataformat, ingen resampling) med så få
/// kanaler som möjligt; faller tillbaka till enhetens standardformat.
fn pick_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig> {
    let target = cpal::SampleRate(16_000);
    if let Ok(ranges) = device.supported_input_configs() {
        let mut best: Option<cpal::SupportedStreamConfig> = None;
        for range in ranges {
            let ok_format = matches!(
                range.sample_format(),
                cpal::SampleFormat::F32 | cpal::SampleFormat::I16
            );
            if ok_format && range.min_sample_rate() <= target && target <= range.max_sample_rate() {
                let candidate = range.with_sample_rate(target);
                let better = match &best {
                    None => true,
                    Some(b) => candidate.channels() < b.channels(),
                };
                if better {
                    best = Some(candidate);
                }
            }
        }
        if let Some(cfg) = best {
            return Ok(cfg);
        }
    }
    Ok(device.default_input_config()?)
}

/// Namnen på alla tillgängliga inspelningsenheter (för menyvalet).
pub fn input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

impl Recorder {
    /// `level` matas kontinuerligt med mikrofonens toppnivå (även när
    /// inspelning inte pågår) så att UI:t kan visa en nivåmätare.
    /// `device_name` väljer en specifik mikrofon; None (eller okänt namn,
    /// t.ex. ett urkopplat headset) ger Windows standardenhet.
    pub fn new(level: Arc<AtomicU32>, device_name: Option<&str>) -> Result<Self> {
        let host = cpal::default_host();
        let device = device_name
            .and_then(|wanted| {
                let found = host
                    .input_devices()
                    .ok()?
                    .find(|d| d.name().map(|n| n == wanted).unwrap_or(false));
                if found.is_none() {
                    crate::winutil::log(&format!(
                        "mikrofonen \"{wanted}\" hittades inte — använder standardenheten"
                    ));
                }
                found
            })
            .or_else(|| host.default_input_device())
            .ok_or_else(|| anyhow!("ingen mikrofon hittades"))?;
        let supported = pick_config(&device)?;
        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels();
        let sample_format = supported.sample_format();
        let stream_cfg: cpal::StreamConfig = supported.into();
        crate::winutil::log(&format!(
            "Mikrofon: {} ({} Hz, {} kanal(er), {sample_format:?})",
            device.name().unwrap_or_else(|_| "okänd".into()),
            sample_rate,
            channels
        ));

        let capacity = MAX_RECORD_SECS * sample_rate as usize * channels as usize;
        let (mut producer, consumer) = rtrb::RingBuffer::new(capacity);

        let active = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let a = active.clone();
        let lvl = level;
        let f = failed.clone();
        let err_fn = move |e| {
            crate::winutil::log(&format!("ljudfel på mikrofonströmmen: {e}"));
            f.store(true, Ordering::Relaxed);
        };

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_cfg,
                move |data: &[f32], _| {
                    let peak = data.iter().fold(0f32, |m, s| m.max(s.abs()));
                    bump_peak(&lvl, peak);
                    if a.load(Ordering::Relaxed) {
                        for &s in data {
                            // Full buffert => äldsta ljudet vinner; tappet loggas inte
                            // härifrån (callbacken får inte blockera eller allokera).
                            if producer.push(s).is_err() {
                                break;
                            }
                        }
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
                        for &s in data {
                            if producer.push(s as f32 / 32768.0).is_err() {
                                break;
                            }
                        }
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
            consumer,
            active,
            failed,
            sample_rate,
            channels,
        })
    }

    /// True om ljudströmmen har rapporterat fel (t.ex. enheten försvann).
    /// Inspelaren bör då byggas om med `Recorder::new`.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    pub fn start(&mut self) {
        // Töm rester från ev. tidigare inspelning.
        while self.consumer.pop().is_ok() {}
        self.active.store(true, Ordering::Relaxed);
    }

    /// Stoppar buffringen och returnerar ljudet som 16 kHz mono f32
    /// (whispers indataformat).
    pub fn stop(&mut self) -> Vec<f32> {
        self.active.store(false, Ordering::Relaxed);
        self.drain()
    }

    /// Tömmer det som buffrats hittills (16 kHz mono f32) utan att stoppa
    /// buffringen — används av kontinuerligt läge som pollar chunkvis.
    pub fn drain(&mut self) -> Vec<f32> {
        let mut raw = Vec::with_capacity(self.consumer.slots());
        while let Ok(s) = self.consumer.pop() {
            raw.push(s);
        }
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
