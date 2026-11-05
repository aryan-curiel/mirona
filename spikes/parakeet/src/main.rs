//! NGR-128 spike: Parakeet-TDT (sherpa-onnx) vs Whisper (whisper-rs) on 16 kHz mono clips.
//!
//! Usage: parakeet-spike <parakeet|whisper-base|whisper-small> <chunk_s> <clip.wav>...
//! Each clip needs a sibling <clip>.txt reference. Prints full-file RTF + WER, then
//! fixed-size chunk latency (a stand-in for VAD-cut segments) and chunked WER.

use std::time::Instant;

const SR: usize = 16_000;

enum Engine {
    Parakeet(sherpa_onnx::OfflineRecognizer),
    #[cfg(feature = "whisper")]
    Whisper(whisper_rs::WhisperContext),
}

impl Engine {
    fn load(name: &str, threads: i32) -> Engine {
        match name {
            "parakeet" => {
                let dir = "models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8";
                let mut c = sherpa_onnx::OfflineRecognizerConfig::default();
                let m = &mut c.model_config;
                m.transducer.encoder = Some(format!("{dir}/encoder.int8.onnx"));
                m.transducer.decoder = Some(format!("{dir}/decoder.int8.onnx"));
                m.transducer.joiner = Some(format!("{dir}/joiner.int8.onnx"));
                m.tokens = Some(format!("{dir}/tokens.txt"));
                m.model_type = Some("nemo_transducer".into());
                m.num_threads = threads;
                m.provider = Some(std::env::var("PROVIDER").unwrap_or("cpu".into()));
                Engine::Parakeet(sherpa_onnx::OfflineRecognizer::create(&c).expect("load parakeet"))
            }
            #[cfg(feature = "whisper")]
            w if w.starts_with("whisper-") => {
                let path = format!("models/ggml-{}.en.bin", &w[8..]);
                let mut p = whisper_rs::WhisperContextParameters::default();
                p.use_gpu = std::env::var("WHISPER_CPU").is_err(); // Metal unless WHISPER_CPU=1
                Engine::Whisper(whisper_rs::WhisperContext::new_with_params(&path, p).expect("load"))
            }
            _ => panic!("unknown engine {name} (whisper needs --features whisper)"),
        }
    }

    #[cfg_attr(not(feature = "whisper"), allow(unused_variables))]
    fn transcribe(&self, pcm: &[f32], threads: i32) -> String {
        match self {
            Engine::Parakeet(r) => {
                let s = r.create_stream();
                s.accept_waveform(SR as i32, pcm);
                r.decode(&s);
                s.get_result().map(|r| r.text).unwrap_or_default()
            }
            #[cfg(feature = "whisper")]
            Engine::Whisper(ctx) => {
                use whisper_rs::{FullParams, SamplingStrategy};
                let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                p.set_n_threads(threads);
                p.set_language(Some("en"));
                p.set_no_timestamps(true);
                p.set_print_progress(false);
                p.set_print_realtime(false);
                p.set_print_special(false);
                let mut st = ctx.create_state().expect("state");
                st.full(p, pcm).expect("whisper full");
                (0..st.full_n_segments())
                    .filter_map(|i| st.get_segment(i)?.to_str_lossy().ok().map(|s| s.into_owned()))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        }
    }
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .replace(['-', '—'], " ")
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '\'')
        .collect::<String>()
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// Word error rate: word-level Levenshtein distance / reference length.
fn wer(reference: &str, hyp: &str) -> f64 {
    let (r, h) = (words(reference), words(hyp));
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(rw != hw)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len().max(1) as f64
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// Silero VAD segments, force-cut at `max_s` like a live pipeline would.
fn vad_segments(pcm: &[f32], max_s: f32) -> Vec<Vec<f32>> {
    let mut c = sherpa_onnx::VadModelConfig::default();
    c.silero_vad.model = Some("models/silero_vad.onnx".into());
    c.silero_vad.threshold = 0.5;
    c.silero_vad.min_silence_duration = 0.25;
    c.silero_vad.min_speech_duration = 0.25;
    c.silero_vad.window_size = 512;
    c.silero_vad.max_speech_duration = max_s;
    c.sample_rate = SR as i32;
    c.num_threads = 1;
    let vad = sherpa_onnx::VoiceActivityDetector::create(&c, 60.0).expect("load vad");
    let mut out = Vec::new();
    let mut drain = |vad: &sherpa_onnx::VoiceActivityDetector| {
        while let Some(seg) = vad.front() {
            out.push(seg.samples().to_vec());
            vad.pop();
        }
    };
    for w in pcm.chunks(512) {
        vad.accept_waveform(w);
        drain(&vad);
    }
    vad.flush();
    drain(&vad);
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(args.len() >= 4, "usage: {} <engine> <chunk_s> <clip.wav>...", args[0]);
    let chunk_s: f32 = args[2].parse().unwrap();
    let threads: i32 = std::env::var("THREADS").map_or(4, |t| t.parse().unwrap());
    let skip_full = std::env::var("NOFULL").is_ok(); // chunk-only run, for live peak memory

    let t = Instant::now();
    let engine = Engine::load(&args[1], threads);
    println!("engine={} threads={threads} load={:.2}s", args[1], t.elapsed().as_secs_f64());
    engine.transcribe(&vec![0.0; SR], threads); // warm-up, excluded from timings

    // per mode (fixed, vad): chunk latencies (s), chunk audio (s), processing time (s)
    let mut lat = [Vec::new(), Vec::new()];
    let mut seg_s = [0.0, 0.0];
    let mut proc_s = [0.0, 0.0];
    let mut audio_s = 0.0;
    for wav in &args[3..] {
        let reference = std::fs::read_to_string(wav.replace(".wav", ".txt")).expect("reference");
        let mut r = hound::WavReader::open(wav).expect("wav");
        assert_eq!((r.spec().sample_rate, r.spec().channels), (16_000, 1), "need 16k mono");
        let pcm: Vec<f32> = r.samples::<i16>().map(|s| f32::from(s.unwrap()) / 32768.0).collect();
        let dur = pcm.len() as f64 / SR as f64;
        audio_s += dur;
        print!("{wav}: {dur:.1}s");

        if !skip_full {
            let t = Instant::now();
            let full = engine.transcribe(&pcm, threads);
            let full_s = t.elapsed().as_secs_f64();
            print!(" | full {full_s:.2}s RTF {:.3} WER {:.1}%", full_s / dur, 100.0 * wer(&reference, &full));
            if std::env::var("SHOW").is_ok() {
                print!("\n  HYP: {full}\n ");
            }
        }

        let fixed: Vec<Vec<f32>> = pcm.chunks((chunk_s * SR as f32) as usize).map(<[f32]>::to_vec).collect();
        let t = Instant::now();
        let vad = vad_segments(&pcm, chunk_s);
        let vad_ms = t.elapsed().as_secs_f64() * 1e3;
        for (m, (name, segs)) in [("fixed", fixed), ("vad", vad)].into_iter().enumerate() {
            let mut text = Vec::new();
            for seg in &segs {
                let t = Instant::now();
                text.push(engine.transcribe(seg, threads));
                let s = t.elapsed().as_secs_f64();
                if s > 1.0 {
                    print!(" [slow {name} chunk {:.1}s audio -> {s:.2}s]", seg.len() as f64 / SR as f64);
                }
                lat[m].push(s);
                proc_s[m] += s;
                seg_s[m] += seg.len() as f64 / SR as f64;
            }
            print!(" | {name} n={} WER {:.1}%", segs.len(), 100.0 * wer(&reference, &text.join(" ")));
        }
        println!(" | vad itself {vad_ms:.0}ms");
    }
    for (m, name) in ["fixed", "vad"].into_iter().enumerate() {
        let l = &mut lat[m];
        l.sort_by(f64::total_cmp);
        println!(
            "{name} chunks<={chunk_s}s: n={} mean len {:.2}s | latency p50 {:.0}ms p95 {:.0}ms max {:.0}ms | RTF {:.3}",
            l.len(),
            seg_s[m] / l.len() as f64,
            1e3 * pct(l, 0.5),
            1e3 * pct(l, 0.95),
            1e3 * l[l.len() - 1],
            proc_s[m] / audio_s,
        );
    }
}

#[test]
fn wer_check() {
    assert_eq!(wer("the cat sat", "the cat sat"), 0.0);
    assert!((wer("the cat sat down", "the bat sat") - 0.5).abs() < 1e-9); // 1 sub + 1 del
    assert_eq!(wer("Mm-hmm, it's OK.", "mm hmm it's ok"), 0.0);
}
