use anyhow::Result;
use std::path::Path;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

/// Transkriberingsmotor bakom ett gemensamt gränssnitt: whisper.cpp
/// (KB-Whisper, standard) eller Nemotron 3.5 ASR via ONNX Runtime.
/// Väljs med `engine` i konfigurationen.
pub enum Transcriber {
    Whisper(WhisperTranscriber),
    Nemotron(Box<NemotronTranscriber>),
}

impl Transcriber {
    pub fn new_whisper(model_path: &str, language: &str) -> Result<Self> {
        Ok(Self::Whisper(WhisperTranscriber::new(model_path, language)?))
    }

    pub fn new_nemotron(model_dir: &Path, language: &str) -> Result<Self> {
        Ok(Self::Nemotron(Box::new(NemotronTranscriber::new(
            model_dir, language,
        )?)))
    }

    /// Transkriberar ett helt yttrande (16 kHz mono f32) till text.
    /// Med `digits` skrivs talord som siffror via [`crate::numbers::convert_sv`].
    pub fn transcribe(&mut self, audio: &[f32], digits: bool) -> Result<String> {
        let text = match self {
            Self::Whisper(t) => t.transcribe(audio, digits)?,
            Self::Nemotron(t) => t.transcribe(audio)?,
        };
        if digits {
            Ok(crate::numbers::convert_sv(&text))
        } else {
            Ok(text)
        }
    }
}

pub struct WhisperTranscriber {
    state: WhisperState,
    language: String,
}

impl WhisperTranscriber {
    fn new(model_path: &str, language: &str) -> Result<Self> {
        let mut ctx_params = WhisperContextParameters::default();
        // Flash attention snabbar upp encode/decode rejält på moderna
        // NVIDIA-kort; ignoreras tyst där det saknar stöd.
        ctx_params.flash_attn(true);
        let ctx = WhisperContext::new_with_params(model_path, ctx_params)?;
        // Staten (kv-cache + compute-buffertar, ~330 MB) skapas en gång
        // och återanvänds mellan yttranden.
        let state = ctx.create_state()?;
        Ok(Self {
            state,
            language: language.to_string(),
        })
    }

    fn transcribe(&mut self, audio: &[f32], digits: bool) -> Result<String> {
        let state = &mut self.state;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&self.language));
        if digits {
            // Stilprompt: får modellen att föredra siffror framför talord.
            params
                .set_initial_prompt("Mötet den 3 juni: 25 deltagare, klockan 14.30, 5000 kronor.");
        }
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4)
            .min(8);
        params.set_n_threads(n_threads);

        state.full(params, audio)?;

        let mut text = String::new();
        let n = state.full_n_segments();
        for i in 0..n {
            let Some(segment) = state.get_segment(i) else {
                continue;
            };
            let seg = segment.to_str_lossy()?.into_owned();
            let t = seg.trim();
            // Hoppa över icke-tal-artefakter som "[MUSIK]", "(skratt)" och "<|nospeech|>".
            if (t.starts_with('[') && t.ends_with(']'))
                || (t.starts_with('(') && t.ends_with(')'))
                || t.starts_with("<|")
                || t.is_empty()
            {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t);
        }
        Ok(text.trim().to_string())
    }
}

pub struct NemotronTranscriber {
    model: parakeet_rs::Nemotron,
}

impl NemotronTranscriber {
    fn new(model_dir: &Path, language: &str) -> Result<Self> {
        let exec = execution_config();
        let mut model = parakeet_rs::Nemotron::from_pretrained(model_dir, Some(exec))?;
        // Konfigspråket är en whisper-kod ("sv"); Nemotrons promptordbok
        // tar både korta koder och lokaler ("sv" == "sv-SE"). Okänd kod
        // faller tillbaka på automatisk språkdetektering.
        if let Err(e) = model.set_target_lang(language) {
            crate::winutil::log(&format!(
                "nemotron: okänt språk '{language}' ({e}) — använder automatisk detektering"
            ));
        }
        Ok(Self { model })
    }

    fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        // Batch-läge: hela yttrandet matas chunkvis genom streaming-
        // encodern internt; tillståndet nollställs per yttrande.
        let text = self.model.transcribe_audio(audio)?;
        Ok(text.trim().to_string())
    }
}

/// ONNX Runtime-inställningar: GPU-provider efter byggflagga
/// (cuda/directml), annars CPU. Faller alltid tillbaka på CPU i drift.
fn execution_config() -> parakeet_rs::ExecutionConfig {
    let n_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let exec = parakeet_rs::ExecutionConfig::new().with_intra_threads(n_threads);
    #[cfg(feature = "cuda")]
    let exec = exec.with_execution_provider(parakeet_rs::ExecutionProvider::Cuda);
    #[cfg(all(feature = "directml", not(feature = "cuda")))]
    let exec = exec.with_execution_provider(parakeet_rs::ExecutionProvider::DirectML);
    exec
}
