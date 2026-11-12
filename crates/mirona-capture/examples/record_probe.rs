//! Real-hardware check: run the actual `Recorder` (mic + system audio → VAD → Parakeet) for 14 s
//! and log every event. `record_probe <models-dir> <out-dir>`. On macOS, wrap it in an app bundle
//! with `NSMicrophoneUsageDescription` + `NSAudioCaptureUsageDescription` so it gets its own
//! permissions, then play speech (e.g. `afplay tests/fixtures/librispeech-2utt.wav`) while it runs.

use std::io::Write;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (models, out_dir) = (
        std::path::PathBuf::from(&args[1]),
        std::path::PathBuf::from(&args[2]),
    );
    let mut log = std::fs::File::create(out_dir.join("events.txt")).unwrap();
    writeln!(
        log,
        "mic granted: {}, system audio granted: {}",
        mirona_capture::request_mic_permission(),
        mirona_capture::request_system_audio_permission()
    )
    .ok();
    let stt = Arc::new(mirona_capture::Stt::load("parakeet", &models).unwrap());
    let (tx, rx) = channel();
    let rec = mirona_capture::Recorder::start(
        mirona_capture::RecorderConfig {
            models_dir: models,
            vad: mirona_capture::VadConfig::default(),
            recording: out_dir.join("audio.wav"),
            system_recording: Some(out_dir.join("system.wav")),
            mic: true,
            system: true,
        },
        stt,
        tx,
    );
    let rec = match rec {
        Ok(r) => r,
        Err(e) => {
            writeln!(log, "start failed: {e}").ok();
            return;
        }
    };
    let t = Instant::now();
    let mut peak = (0.0f32, 0.0f32);
    while t.elapsed() < Duration::from_secs(14) {
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
            match ev {
                mirona_capture::Event::Levels { mic, system } => {
                    peak = (peak.0.max(mic), peak.1.max(system));
                }
                other => writeln!(log, "{:>5.1}s {other:?}", t.elapsed().as_secs_f32()).unwrap(),
            }
        }
    }
    rec.stop().unwrap();
    for ev in rx.try_iter() {
        if !matches!(ev, mirona_capture::Event::Levels { .. }) {
            writeln!(log, " stop {ev:?}").unwrap();
        }
    }
    writeln!(log, "peak levels mic={:.4} system={:.4}", peak.0, peak.1).unwrap();
}
