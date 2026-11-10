//! `mirona eval` (NGR-160): labelled examples through the classifiers, scored.
//!
//! The set is the public synthetic `eval/chunks.jsonl` (built in; `--set` for another file)
//! plus the user's own corrections from `<meetings>/.mirona/labels.jsonl`, which never leave
//! the machine. Each example is one call carrying every question it has labels for. Reported
//! per question: accuracy, precision/recall for yes/no, and latency per call.
//! `--with fake` needs no network: CI runs that on the public set.

use mirona_core::Locality;
use mirona_engine::block_on;
use mirona_providers::questions::{
    Question, Reply, SEGMENT_QUESTIONS, client_question, meeting_type_question,
};
use mirona_providers::{Classifier, Provider};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;
use std::time::Instant;

pub const PUBLIC_SET: &str = include_str!("../../../eval/chunks.jsonl");

/// The fictional clients of the public set, described as the app's client list would.
pub const PUBLIC_CLIENTS: [(&str, &str); 5] = [
    (
        "northwind_bank",
        "Northwind Bank, contact: Karen, compliance",
    ),
    ("helio_health", "Helio Health clinics, contact: Dr. Osei"),
    ("bluepeak_retail", "Bluepeak Retail stores, contact: Ana"),
    (
        "orbital_freight",
        "Orbital Freight logistics, contact: Felix",
    ),
    (
        "lumen_studios",
        "Lumen Studios film production, contact: Mia",
    ),
];

/// Label keys of `eval/chunks.jsonl` → question ids (corrections already use the ids).
const ALIASES: [(&str, &str); 3] = [
    ("action_item", "action"),
    ("open_question", "unanswered"),
    ("small_talk", "smalltalk"),
];

#[derive(Debug, Clone)]
pub struct Example {
    pub id: String,
    pub text: String,
    /// Question id → expected answer ("yes"/"no", or the choice).
    pub labels: BTreeMap<String, String>,
}

/// JSONL rows `{"id", "text", "labels": {...}}`; rows sharing an id are merged (a later
/// correction of the same segment wins).
pub fn parse(jsonl: &str) -> Result<Vec<Example>, String> {
    let mut out: Vec<Example> = vec![];
    for (n, line) in jsonl
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let row: Value = serde_json::from_str(line).map_err(|e| format!("line {}: {e}", n + 1))?;
        let (Some(id), Some(text), Some(labels)) = (
            row["id"].as_str(),
            row["text"].as_str(),
            row["labels"].as_object(),
        ) else {
            return Err(format!("line {}: needs id, text and labels", n + 1));
        };
        let labels = labels.iter().filter_map(|(k, v)| {
            let k = ALIASES
                .iter()
                .find(|(a, _)| a == k)
                .map_or(k.as_str(), |(_, q)| q);
            Some((k.to_string(), v.as_str()?.to_string()))
        });
        match out.iter_mut().find(|e| e.id == id) {
            Some(e) => e.labels.extend(labels),
            None => out.push(Example {
                id: id.into(),
                text: text.into(),
                labels: labels.collect(),
            }),
        }
    }
    Ok(out)
}

/// The questions an example has labels for.
fn questions(e: &Example) -> Vec<Question> {
    let mut qs: Vec<Question> = SEGMENT_QUESTIONS
        .iter()
        .filter(|q| e.labels.contains_key(q.id))
        .map(mirona_providers::questions::YesNo::to_question)
        .collect();
    if e.labels.contains_key("meeting_type") {
        qs.push(meeting_type_question());
    }
    if e.labels.contains_key("follow_up")
        && let Some(q) =
            mirona_providers::questions::meeting_questions(&std::collections::BTreeMap::new())
                .into_iter()
                .find(|q| q.id == "follow_up")
    {
        qs.push(q);
    }
    if let Some(c) = e.labels.get("client") {
        // Your own clients (from corrections) join the public list as options.
        let extra = (!PUBLIC_CLIENTS.iter().any(|(n, _)| n == c) && c != "none")
            .then_some((c.as_str(), ""));
        qs.push(client_question(PUBLIC_CLIENTS.into_iter().chain(extra)));
    }
    qs
}

#[derive(Debug, Default, Clone)]
pub struct Score {
    pub n: usize,
    pub correct: usize,
    /// Yes/no only: true positives, predicted yes, actually yes.
    pub tp: usize,
    pub said_yes: usize,
    pub is_yes: usize,
    pub yes_no: bool,
}

impl Score {
    fn ratio(a: usize, b: usize) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        (b > 0).then(|| a as f64 / b as f64)
    }
    pub fn accuracy(&self) -> Option<f64> {
        Self::ratio(self.correct, self.n)
    }
    pub fn precision(&self) -> Option<f64> {
        Self::ratio(self.tp, self.said_yes)
    }
    pub fn recall(&self) -> Option<f64> {
        Self::ratio(self.tp, self.is_yes)
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub scores: BTreeMap<String, Score>,
    pub latencies_ms: Vec<f64>,
    pub failed: usize,
}

impl Report {
    pub fn mean_accuracy(&self) -> Option<f64> {
        let acc: Vec<f64> = self.scores.values().filter_map(Score::accuracy).collect();
        #[allow(clippy::cast_precision_loss)]
        (!acc.is_empty()).then(|| acc.iter().sum::<f64>() / acc.len() as f64)
    }

    /// p-th percentile (nearest rank) of the call latencies.
    pub fn latency(&self, p: f64) -> Option<f64> {
        let mut v = self.latencies_ms.clone();
        v.sort_by(f64::total_cmp);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let i = ((p / 100.0 * v.len() as f64).ceil() as usize).clamp(1, v.len().max(1)) - 1;
        v.get(i).copied()
    }

    fn add(&mut self, e: &Example, qs: &[Question], replies: &[Reply]) {
        for (q, r) in qs.iter().zip(replies) {
            let Some(truth) = e.labels.get(&q.id) else {
                continue;
            };
            let got = match r {
                Reply::YesNo(p) => if *p >= 0.5 { "yes" } else { "no" }.to_string(),
                Reply::Choice { value, .. } => value.clone(),
                Reply::Score { score, .. } => format!("{score:.0}"),
            };
            let s = self.scores.entry(q.id.clone()).or_default();
            s.n += 1;
            s.correct += usize::from(&got == truth);
            if matches!(r, Reply::YesNo(_)) {
                s.yes_no = true;
                s.said_yes += usize::from(got == "yes");
                s.is_yes += usize::from(truth == "yes");
                s.tp += usize::from(got == "yes" && truth == "yes");
            }
        }
    }

    pub fn markdown(&self, name: &str) -> String {
        let f = |v: Option<f64>| v.map_or_else(|| "–".into(), |v| format!("{v:.2}"));
        let ms = |v: Option<f64>| v.map_or_else(|| "–".into(), |v| format!("{v:.0}"));
        let mut out = format!(
            "## {name}\n\n| question | n | accuracy | precision | recall |\n|---|---|---|---|---|\n"
        );
        for (q, s) in &self.scores {
            let (p, r) = if s.yes_no {
                (f(s.precision()), f(s.recall()))
            } else {
                ("–".into(), "–".into())
            };
            let _ = writeln!(out, "| {q} | {} | {} | {p} | {r} |", s.n, f(s.accuracy()));
        }
        #[allow(clippy::cast_precision_loss)]
        let mean = (!self.latencies_ms.is_empty())
            .then(|| self.latencies_ms.iter().sum::<f64>() / self.latencies_ms.len() as f64);
        let _ = writeln!(
            out,
            "| **mean** | | **{}** | | |\n\nLatency per call (all questions): p50 {} ms, p95 {} ms, mean {} ms over {} calls; {} failed.",
            f(self.mean_accuracy()),
            ms(self.latency(50.0)),
            ms(self.latency(95.0)),
            ms(mean),
            self.latencies_ms.len(),
            self.failed,
        );
        out
    }
}

/// Run every example through `c`. With `context`, each example is asked with the previous
/// example's last two lines in front (unrelated text: measures how much context bleeds).
pub fn run(c: &impl Classifier, examples: &[Example], context: bool) -> Report {
    let mut r = Report::default();
    let mut prev: Vec<String> = vec![];
    for e in examples {
        let qs = questions(e);
        let text = if context {
            mirona_engine::classify::segment_state(&prev, &e.text)
        } else {
            e.text.clone()
        };
        let t = Instant::now();
        match block_on(c.ask(&text, &qs)) {
            Ok(replies) => {
                r.latencies_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                r.add(e, &qs, &replies);
            }
            Err(err) => {
                eprintln!("{}: {err}", e.id);
                r.failed += 1;
            }
        }
        let lines: Vec<&str> = e.text.lines().collect();
        prev = lines[lines.len().saturating_sub(2)..]
            .iter()
            .map(|l| (*l).to_string())
            .collect();
    }
    r
}

/// The examples: `set` (or the built-in public set), plus the user's corrections.
/// `labels` is the corrections file's text (decrypted), with its path for messages.
pub fn load(set: Option<&Path>, labels: Option<(&Path, &str)>) -> Result<Vec<Example>, String> {
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let mut out = match set {
        Some(p) => parse(&read(p)?)?,
        None => parse(PUBLIC_SET)?,
    };
    if let Some((p, text)) = labels.filter(|(_, t)| !t.trim().is_empty()) {
        let own = parse(text).map_err(|e| format!("{}: {e}", p.display()))?;
        eprintln!(
            "+ {} of your own labelled examples ({})",
            own.len(),
            p.display()
        );
        out.extend(own);
    }
    Ok(out)
}

pub fn name(c: &impl Provider) -> String {
    let where_ = match c.locality() {
        Locality::Local => "local",
        Locality::Cloud => "cloud",
    };
    format!("{} ({where_})", c.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirona_providers::fake::Fake;

    #[test]
    fn the_public_set_parses_with_question_ids() {
        let set = parse(PUBLIC_SET).unwrap();
        assert_eq!(set.len(), 50);
        assert!(set[0].labels.contains_key("action"));
        assert!(set[0].labels.contains_key("client"));
        assert_eq!(questions(&set[0]).len(), 7);
    }

    #[test]
    fn corrections_merge_by_id_and_ask_only_what_is_labelled() {
        let rows = r#"{"id":"m-1","text":"Me: I'll send it","labels":{"action":"yes"}}
{"id":"m-1","text":"Me: I'll send it","labels":{"sensitive":"no"}}"#;
        let set = parse(rows).unwrap();
        assert_eq!(set.len(), 1);
        let ids: Vec<String> = questions(&set[0]).into_iter().map(|q| q.id).collect();
        assert_eq!(ids, ["action", "sensitive"]);
    }

    #[test]
    fn scores_precision_recall_and_latency() {
        let set = parse(PUBLIC_SET).unwrap();
        let r = run(&Fake("fake"), &set, false);
        assert_eq!(r.failed, 0);
        assert_eq!(r.latencies_ms.len(), 50);
        let action = &r.scores["action"];
        assert_eq!(
            (action.n, action.is_yes),
            (50, 21),
            "21 labelled action items"
        );
        assert!(action.precision().is_some() && action.recall().is_some());
        assert!(
            r.scores["client"].precision().is_none(),
            "no P/R for choices"
        );
        let md = r.markdown("fake");
        assert!(md.contains("| action | 50 |"), "{md}");
        assert!(md.contains("p50"), "{md}");
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let r = Report {
            latencies_ms: (1..=100).map(f64::from).collect(),
            ..Report::default()
        };
        assert_eq!(r.latency(50.0), Some(50.0));
        assert_eq!(r.latency(95.0), Some(95.0));
    }
}
