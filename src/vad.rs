use anyhow::{anyhow, Result};
use whisper_rs::{WhisperVadContext, WhisperVadContextParams};

/// Röstaktivitetsdetektor (Silero via whisper.cpp). Körs på CPU med en
/// tråd — modellen är liten (<1 MB) och en chunk tar långt under 1 ms.
pub struct Vad {
    ctx: WhisperVadContext,
    threshold: f32,
}

impl Vad {
    pub fn new(model_path: &str, threshold: f32) -> Result<Self> {
        let mut params = WhisperVadContextParams::new();
        params.set_n_threads(1);
        params.set_use_gpu(false);
        let ctx = WhisperVadContext::new(model_path, params)
            .map_err(|e| anyhow!("kunde inte ladda VAD-modellen: {e}"))?;
        Ok(Self {
            ctx,
            threshold: threshold.clamp(0.0, 1.0),
        })
    }

    /// True om chunken (16 kHz mono) innehåller tal någonstans.
    pub fn is_speech(&mut self, chunk: &[f32]) -> bool {
        match self.ctx.detect_speech(chunk) {
            Ok(()) => self.ctx.probabilities().iter().any(|&p| p > self.threshold),
            Err(e) => {
                crate::winutil::log(&format!("VAD-fel: {e}"));
                false
            }
        }
    }
}

/// Vad tillståndsmaskinen signalerar efter varje chunk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateEvent {
    /// Inget att göra — fortsätt lyssna/buffra.
    None,
    /// Tal har just börjat.
    SpeechStarted,
    /// Tal följt av tillräcklig tystnad — transkribera bufferten.
    UtteranceEnded,
    /// Maxlängden nådd mitt i tal — transkribera och fortsätt buffra.
    ForcedCut,
}

/// Tillståndsmaskin för kontinuerligt läge: matas med ett tal/tystnad-
/// beslut per chunk och avgör när ett yttrande börjar och slutar.
/// Helt fri från ljudberoenden så att den kan enhetstestas.
pub struct SpeechGate {
    speaking: bool,
    silence_ms: u32,
    speech_ms: u32,
    min_silence_ms: u32,
    max_speech_ms: u32,
}

impl SpeechGate {
    pub fn new(min_silence_ms: u32, max_speech_ms: u32) -> Self {
        Self {
            speaking: false,
            silence_ms: 0,
            speech_ms: 0,
            min_silence_ms,
            max_speech_ms,
        }
    }

    pub fn speaking(&self) -> bool {
        self.speaking
    }

    pub fn reset(&mut self) {
        self.speaking = false;
        self.silence_ms = 0;
        self.speech_ms = 0;
    }

    pub fn push(&mut self, speech: bool, chunk_ms: u32) -> GateEvent {
        if !self.speaking {
            if speech {
                self.speaking = true;
                self.speech_ms = chunk_ms;
                self.silence_ms = 0;
                GateEvent::SpeechStarted
            } else {
                GateEvent::None
            }
        } else {
            self.speech_ms += chunk_ms;
            if speech {
                self.silence_ms = 0;
            } else {
                self.silence_ms += chunk_ms;
            }
            if self.silence_ms >= self.min_silence_ms {
                self.reset();
                GateEvent::UtteranceEnded
            } else if self.speech_ms >= self.max_speech_ms {
                self.speech_ms = 0;
                GateEvent::ForcedCut
            } else {
                GateEvent::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tal_foljt_av_tystnad_klipper() {
        let mut g = SpeechGate::new(800, 30_000);
        assert_eq!(g.push(false, 150), GateEvent::None);
        assert_eq!(g.push(true, 150), GateEvent::SpeechStarted);
        assert_eq!(g.push(true, 150), GateEvent::None);
        // 800 ms tystnad i 150 ms-steg: 6:e chunken passerar tröskeln.
        for _ in 0..5 {
            assert_eq!(g.push(false, 150), GateEvent::None);
        }
        assert_eq!(g.push(false, 150), GateEvent::UtteranceEnded);
        assert!(!g.speaking());
    }

    #[test]
    fn kort_paus_avslutar_inte() {
        let mut g = SpeechGate::new(800, 30_000);
        g.push(true, 150);
        // 600 ms paus — under tröskeln — och sedan tal igen.
        for _ in 0..4 {
            assert_eq!(g.push(false, 150), GateEvent::None);
        }
        assert_eq!(g.push(true, 150), GateEvent::None);
        assert!(g.speaking());
    }

    #[test]
    fn maxlangd_tvangsklipper_och_fortsatter() {
        let mut g = SpeechGate::new(800, 1_000);
        g.push(true, 300);
        g.push(true, 300);
        g.push(true, 300);
        assert_eq!(g.push(true, 300), GateEvent::ForcedCut);
        assert!(g.speaking());
        // Tystnad efteråt avslutar som vanligt.
        g.push(false, 500);
        assert_eq!(g.push(false, 500), GateEvent::UtteranceEnded);
    }

    #[test]
    fn tystnad_utan_tal_gor_inget() {
        let mut g = SpeechGate::new(800, 30_000);
        for _ in 0..100 {
            assert_eq!(g.push(false, 150), GateEvent::None);
        }
        assert!(!g.speaking());
    }
}
