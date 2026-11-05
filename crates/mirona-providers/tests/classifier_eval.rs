//! Runs `eval/chunks.jsonl` (50 labelled chunks, NGR-130) through the real classifiers and
//! checks they still clear the gate-0 bar from ADR 0001.
//!
//! ```sh
//! TYPESAFE_API_KEY=… cargo test -p mirona-providers --features jev,ollama \
//!   --test classifier_eval -- --ignored --nocapture
//! ```

use mirona_core::Segment;
use mirona_providers::Classifier;
use mirona_providers::jev::Jev;
use mirona_providers::ollama::Ollama;
use serde_json::Value;

/// Eval label key → `SEGMENT_QUESTIONS` id.
const LABELS: [(&str, &str); 5] = [
    ("action_item", "action"),
    ("decision", "decision"),
    ("open_question", "unanswered"),
    ("small_talk", "smalltalk"),
    ("sensitive", "sensitive"),
];

fn now<T>(fut: impl Future<Output = T>) -> T {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(fut).poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => panic!("classifier future was not ready"),
    }
}

struct Score {
    /// Correct answers per question, in `LABELS` order.
    correct: [usize; 5],
    sensitive_found: usize,
    sensitive_total: usize,
    chunks: usize,
}

impl Score {
    fn accuracy(&self, i: usize) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let a = self.correct[i] as f64 / self.chunks as f64;
        a
    }
    fn mean(&self) -> f64 {
        (0..5).map(|i| self.accuracy(i)).sum::<f64>() / 5.0
    }
    fn sensitive_recall(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let r = self.sensitive_found as f64 / self.sensitive_total as f64;
        r
    }
}

fn run(name: &str, c: &impl Classifier) -> Score {
    let chunks = include_str!("../../../eval/chunks.jsonl");
    let mut s = Score {
        correct: [0; 5],
        sensitive_found: 0,
        sensitive_total: 0,
        chunks: 0,
    };
    for line in chunks.lines().filter(|l| !l.trim().is_empty()) {
        let row: Value = serde_json::from_str(line).unwrap();
        let seg = Segment {
            id: row["id"].as_str().unwrap().into(),
            speaker_id: String::new(),
            start_ms: 0,
            text: row["text"].as_str().unwrap().into(),
            redacted: false,
            tags: vec![],
            answers: vec![],
            owners: vec![],
        };
        let answers = now(c.classify(&seg)).unwrap();
        for (i, (label, id)) in LABELS.iter().enumerate() {
            let truth = row["labels"][label] == "yes";
            let got = answers.iter().find(|a| a.id == *id).unwrap().answer;
            s.correct[i] += usize::from(got == truth);
            if *label == "sensitive" && truth {
                s.sensitive_total += 1;
                s.sensitive_found += usize::from(got);
            }
        }
        s.chunks += 1;
    }
    println!("{name}: {} chunks", s.chunks);
    for (i, (label, _)) in LABELS.iter().enumerate() {
        println!("  {label:<14} {:.2}", s.accuracy(i));
    }
    println!(
        "  mean           {:.2}   sensitive recall {:.2}",
        s.mean(),
        s.sensitive_recall()
    );
    s
}

#[test]
#[ignore = "needs TYPESAFE_API_KEY and network"]
fn jev_clears_the_gate_0_bar() {
    let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
    let s = run("jev", &Jev::new(key));
    assert_eq!(s.chunks, 50);
    assert!(s.mean() >= 0.90, "mean {:.2}", s.mean());
    assert!(
        s.sensitive_recall() >= 0.9,
        "sensitive recall {:.2}",
        s.sensitive_recall()
    );
}

#[test]
#[ignore = "needs a running Ollama with qwen3:4b"]
fn local_fallback_is_usable() {
    let s = run("ollama qwen3:4b", &Ollama::default());
    assert_eq!(s.chunks, 50);
    // ADR 0001: acceptable as the privacy-mode fallback, not as good as Jev.
    assert!(s.mean() >= 0.80, "mean {:.2}", s.mean());
}
