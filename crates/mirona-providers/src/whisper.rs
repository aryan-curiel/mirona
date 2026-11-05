//! Whisper (whisper-rs, Metal on macOS) `SttProvider`: the STT fallback (ADR 0001, NGR-139).
//! Use it for low-RAM machines (base.en), battery (the GPU does the work) and non-English
//! meetings (a multilingual ggml model).

use std::path::Path;
use std::sync::Mutex;

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::{Error, Locality, Provider, Result, SttProvider};

pub struct Whisper {
    // One decode at a time (single STT worker, ADR 0001); the state keeps the context alive.
    state: Mutex<WhisperState>,
    threads: i32,
    english_only: bool,
}

impl Whisper {
    /// Load a ggml model file such as `ggml-small.en.bin`.
    pub fn new(model: &Path, threads: i32) -> Result<Self> {
        if !model.is_file() {
            return Err(Error::Provider(format!(
                "whisper model file missing: {}",
                model.display()
            )));
        }
        whisper_rs::install_logging_hooks(); // whisper.cpp is chatty on stderr; silence it
        let err = |e: whisper_rs::WhisperError| Error::Provider(format!("whisper: {e}"));
        let ctx = WhisperContext::new_with_params(
            model,
            WhisperContextParameters::default(), // use_gpu: Metal when built with it
        )
        .map_err(err)?;
        let state = ctx.create_state().map_err(err)?;
        let name = model.file_name().unwrap_or_default().to_string_lossy();
        Ok(Self {
            state: Mutex::new(state),
            threads,
            english_only: name.contains(".en."),
        })
    }

    fn decode(&self, pcm: &[f32]) -> Result<String> {
        let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        p.set_n_threads(self.threads);
        p.set_language(Some(if self.english_only { "en" } else { "auto" }));
        p.set_no_context(true); // chunks are independent; context makes it repeat itself
        p.set_no_timestamps(true);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_special(false);
        let mut st = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        st.full(p, pcm)
            .map_err(|e| Error::Provider(format!("whisper: {e}")))?;
        let text: Vec<String> = (0..st.full_n_segments())
            .filter_map(|i| {
                st.get_segment(i)?
                    .to_str_lossy()
                    .ok()
                    .map(|s| s.trim().to_owned())
            })
            .collect();
        Ok(text.join(" ").trim().to_owned())
    }
}

impl Provider for Whisper {
    fn id(&self) -> &'static str {
        "whisper"
    }
    fn locality(&self) -> Locality {
        Locality::Local
    }
}

impl SttProvider for Whisper {
    /// `pcm` is one VAD segment of 16 kHz mono audio.
    fn transcribe(&self, pcm: &[f32]) -> impl Future<Output = Result<String>> + Send {
        // ponytail: blocking, like Parakeet; callers run it on the STT worker thread.
        std::future::ready(self.decode(pcm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_is_a_provider_error() {
        let Err(Error::Provider(msg)) = Whisper::new(Path::new("/nonexistent/ggml.bin"), 1) else {
            panic!("expected a provider error");
        };
        assert!(msg.contains("ggml.bin"), "{msg}");
    }

    /// Needs `MIRONA_WHISPER_MODEL=spikes/parakeet/models/ggml-base.en.bin`.
    #[test]
    #[ignore = "needs MIRONA_WHISPER_MODEL"]
    fn transcribes_speech() {
        let model = std::env::var("MIRONA_WHISPER_MODEL").expect("MIRONA_WHISPER_MODEL");
        let w = Whisper::new(Path::new(&model), 4).unwrap();
        let audio = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spikes/parakeet/audio");
        // First utterance only (3.5 s): Whisper's long-form decode is not what we ship.
        let pcm: Vec<f32> = hound::WavReader::open(format!("{audio}/librispeech-clean.wav"))
            .unwrap()
            .samples::<i16>()
            .take(56_000)
            .map(|s| f32::from(s.unwrap()) / 32768.0)
            .collect();
        let text = w.decode(&pcm).unwrap().to_lowercase();
        println!("whisper: {text}");
        assert!(text.contains("returned to its place"), "{text}");
    }
}
