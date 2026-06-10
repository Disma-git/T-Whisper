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

    pub fn transcribe(&self, audio: &[f32]) -> Result<String> {
        let mut state = self.ctx.create_state()?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&self.language));
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
        let n = state.full_n_segments()?;
        for i in 0..n {
            let seg = state.full_get_segment_text(i)?;
            let t = seg.trim();
            // Hoppa över icke-tal-artefakter som "[MUSIK]" och "(skratt)".
            if (t.starts_with('[') && t.ends_with(']'))
                || (t.starts_with('(') && t.ends_with(')'))
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
