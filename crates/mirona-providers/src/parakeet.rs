//! Parakeet-TDT 0.6B v2 (int8) through sherpa-onnx: the default STT (ADR 0001, NGR-139).

use std::path::Path;

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

use crate::{Error, Locality, Provider, Result, SttProvider};

pub struct Parakeet {
    // sherpa-onnx declares the recognizer Send + Sync, so no Mutex is needed.
    recognizer: OfflineRecognizer,
}

impl Parakeet {
    /// Load `encoder/decoder/joiner.int8.onnx` + `tokens.txt` from `model_dir`.
    pub fn new(model_dir: &Path, threads: i32) -> Result<Self> {
        let file = |name: &str| {
            let p = model_dir.join(name);
            if p.is_file() {
                Ok(Some(p.to_string_lossy().into_owned()))
            } else {
                Err(Error::Provider(format!(
                    "parakeet model file missing: {}",
                    p.display()
                )))
            }
        };
        let mut c = OfflineRecognizerConfig::default();
        let m = &mut c.model_config;
        m.transducer.encoder = file("encoder.int8.onnx")?;
        m.transducer.decoder = file("decoder.int8.onnx")?;
        m.transducer.joiner = file("joiner.int8.onnx")?;
        m.tokens = file("tokens.txt")?;
        m.model_type = Some("nemo_transducer".into());
        m.num_threads = threads;
        m.provider = Some("cpu".into()); // prebuilt sherpa-onnx has no CoreML (ADR 0001)
        let recognizer = OfflineRecognizer::create(&c)
            .ok_or_else(|| Error::Provider("parakeet failed to load".into()))?;
        Ok(Self { recognizer })
    }

    fn decode(&self, pcm: &[f32]) -> String {
        let s = self.recognizer.create_stream();
        s.accept_waveform(16_000, pcm);
        self.recognizer.decode(&s);
        s.get_result()
            .map(|r| r.text.trim().to_owned())
            .unwrap_or_default()
    }
}

impl Provider for Parakeet {
    fn id(&self) -> &'static str {
        "parakeet"
    }
    fn locality(&self) -> Locality {
        Locality::Local
    }
}

impl SttProvider for Parakeet {
    /// `pcm` is one VAD segment (≤ 5 s, NGR-138) of 16 kHz mono audio.
    fn transcribe(&self, pcm: &[f32]) -> impl Future<Output = Result<String>> + Send {
        // ponytail: decodes right here, blocking, and hands back a ready future; callers must
        // call it off the async executor (single STT worker, ADR 0001). spawn_blocking in NGR-139.
        std::future::ready(Ok(self.decode(pcm)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_is_a_provider_error() {
        let Err(Error::Provider(msg)) = Parakeet::new(Path::new("/nonexistent"), 1) else {
            panic!("expected a provider error");
        };
        assert!(msg.contains("encoder.int8.onnx"), "{msg}");
    }
}

/// Needs the model: `MIRONA_PARAKEET_DIR=spikes/parakeet/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8`.
#[cfg(test)]
mod model_tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.to_lowercase()
            .replace('-', " ")
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '\'')
            .collect::<String>()
            .split_whitespace()
            .map(String::from)
            .collect()
    }

    /// Word-level Levenshtein distance / reference length.
    #[allow(clippy::cast_precision_loss)]
    fn wer(reference: &str, hyp: &str) -> f64 {
        let (r, h) = (words(reference), words(hyp));
        let mut prev: Vec<usize> = (0..=h.len()).collect();
        for (i, rw) in r.iter().enumerate() {
            let mut cur = vec![i + 1; h.len() + 1];
            for (j, hw) in h.iter().enumerate() {
                cur[j + 1] = (prev[j] + usize::from(rw != hw))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1);
            }
            prev = cur;
        }
        prev[h.len()] as f64 / r.len().max(1) as f64
    }

    #[test]
    fn wer_helper() {
        assert!(wer("the cat sat", "The cat sat.") < f64::EPSILON);
        assert!((wer("the cat sat down", "the bat sat") - 0.5).abs() < 1e-9);
    }

    #[test]
    #[ignore = "needs MIRONA_PARAKEET_DIR"]
    fn transcribes_librispeech_clean() {
        let dir = std::env::var("MIRONA_PARAKEET_DIR").expect("MIRONA_PARAKEET_DIR");
        let p = Parakeet::new(Path::new(&dir), 4).unwrap();
        let audio = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spikes/parakeet/audio");
        let pcm: Vec<f32> = hound::WavReader::open(format!("{audio}/librispeech-clean.wav"))
            .unwrap()
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32768.0)
            .collect();
        let reference = std::fs::read_to_string(format!("{audio}/librispeech-clean.txt")).unwrap();
        // The whole 2 min clip as one segment; live audio goes through VAD first (NGR-138).
        let wer = wer(&reference, &now(p.transcribe(&pcm)).unwrap());
        println!("parakeet librispeech-clean WER {:.1}%", 100.0 * wer);
        assert!(wer < 0.05, "WER {wer}");
    }

    /// The future has no await points, so one poll finishes it.
    fn now<T>(fut: impl Future<Output = T>) -> T {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match std::pin::pin!(fut).poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("not ready"),
        }
    }
}
