use anyhow::Result;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Transcriber {
    ctx: WhisperContext,
    language: String,
}

impl Transcriber {
    pub fn new(model_path: &str, language: &str) -> Result<Self> {
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())?;
        Ok(Self {
            ctx,
            language: language.to_string(),
        })
    }

    pub fn transcribe(&self, audio: &[f32], digits: bool) -> Result<String> {
        let mut state = self.ctx.create_state()?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&self.language));
        if digits {
            // Stilprompt: får modellen att föredra siffror framför talord.
            params.set_initial_prompt("Mötet den 3 juni: 25 deltagare, klockan 14.30, 5000 kronor.");
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
        let text = text.trim().to_string();
        if digits {
            Ok(crate::numbers::convert_sv(&text))
        } else {
            Ok(text)
        }
    }
}
