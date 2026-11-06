//! Classification results in the store (NGR-156/158/160): per-segment answers and tags, the
//! per-meeting review list, and corrections kept as labelled examples.
//!
//! `<root>/.mirona/labels.jsonl` gets one line per correction, local only:
//! `{"id", "meeting_id", "text", "labels": {"action": "yes"}, "model_probability", "at"}`.
//! `mirona eval` reads it next to the public `eval/chunks.jsonl`.

use crate::{Result, Store};
use mirona_core::{Answer, Choice, Review, Segment, Tag, TagKind};
use std::io::Write;
use std::path::PathBuf;

/// The tag for a model "yes" below its threshold: "Needs review · action? 0.48".
pub fn review_tag(kind: TagKind, p_yes: f32) -> Tag {
    Tag {
        kind: TagKind::Review,
        label: format!("{} {p_yes:.2}", review_prefix(kind)),
    }
}

fn review_prefix(kind: TagKind) -> String {
    format!("Needs review · {}?", kind.label().to_lowercase())
}

impl Store {
    /// Where corrections are kept as labelled examples.
    pub fn labels_path(&self) -> PathBuf {
        self.root.join(".mirona").join("labels.jsonl")
    }

    /// Stores a segment's model answers, tags and owners, leaving its text and speaker alone
    /// (the user may have edited them meanwhile). Answers the user corrected are kept.
    pub fn set_classification(
        &mut self,
        meeting_id: &str,
        segment_id: &str,
        answers: Vec<Answer>,
        tags: Vec<Tag>,
        owners: Vec<Choice>,
    ) -> Result<Segment> {
        let i = self.idx(meeting_id)?;
        let seg = self.entries[i]
            .meeting
            .segments
            .iter_mut()
            .find(|s| s.id == segment_id)
            .ok_or("no segment")?;
        let fixed: Vec<Answer> = seg
            .answers
            .iter()
            .filter(|a| a.source == mirona_core::AnswerSource::User)
            .cloned()
            .collect();
        seg.answers = answers
            .into_iter()
            .map(|a| fixed.iter().find(|f| f.id == a.id).cloned().unwrap_or(a))
            .collect();
        seg.tags = tags;
        seg.owners = owners;
        let seg = seg.clone();
        // ponytail: rewrites the whole transcript per segment (O(n²) over a meeting); fine for
        // hours of speech, append-only patches if it ever shows up in a profile.
        self.save(i, true)?;
        Ok(seg)
    }

    /// Everything in a meeting that is below its threshold and not yet confirmed (NGR-158):
    /// segment answers, meeting type, client, follow-up, and action items needing review.
    pub fn review(&self, meeting_id: &str) -> Result<Vec<Review>> {
        let e = &self.entries[self.idx(meeting_id)?];
        let m = &e.meeting;
        let mut out: Vec<Review> = m
            .segments
            .iter()
            .flat_map(|s| {
                s.answers
                    .iter()
                    .filter(|a| a.needs_review())
                    .map(|a| Review {
                        segment_id: Some(s.id.clone()),
                        at_ms: Some(s.start_ms),
                        question: a.id.clone(),
                        value: "yes".into(),
                        probability: a.probability,
                        threshold: a.threshold,
                    })
            })
            .collect();
        if let Some(a) = &m.answers {
            for (q, c) in [("meeting_type", &a.meeting_type), ("client", &a.client)] {
                if !c.confirmed && c.probability < c.threshold && !c.value.is_empty() {
                    out.push(Review {
                        segment_id: None,
                        at_ms: None,
                        question: q.into(),
                        value: c.value.clone(),
                        probability: c.probability,
                        threshold: c.threshold,
                    });
                }
            }
            if a.follow_up_probability >= 0.5 && a.follow_up_probability < a.follow_up_threshold {
                out.push(Review {
                    segment_id: None,
                    at_ms: None,
                    question: "follow_up".into(),
                    value: "yes".into(),
                    probability: a.follow_up_probability,
                    threshold: a.follow_up_threshold,
                });
            }
        }
        out.extend(e.items.iter().filter(|a| a.needs_review).map(|a| Review {
            segment_id: None,
            at_ms: a.at_ms,
            question: "action_item".into(),
            value: a.text.clone(),
            probability: 0.0,
            threshold: 0.0,
        }));
        Ok(out)
    }

    /// Retag a corrected segment: its answer's tag follows the fix, and its review tag goes.
    pub(crate) fn retag(seg: &mut Segment, answer_id: &str, value: bool) {
        let Some(kind) = TagKind::for_answer(answer_id) else {
            return;
        };
        let prefix = review_prefix(kind);
        let still_unsure = seg.answers.iter().any(Answer::needs_review);
        seg.tags.retain(|t| {
            t.kind != kind
                && !(t.kind == TagKind::Review && (!still_unsure || t.label.starts_with(&prefix)))
        });
        if value {
            seg.tags.push(Tag {
                kind,
                label: if kind == TagKind::Action {
                    "Action · confirmed".into()
                } else {
                    kind.label().into()
                },
            });
        }
    }

    /// One labelled example per correction, appended to [`Store::labels_path`].
    pub(crate) fn append_label(
        &self,
        meeting_id: &str,
        seg: &Segment,
        answer_id: &str,
        value: bool,
        model_probability: f32,
    ) -> Result<()> {
        let m = &self.entries[self.idx(meeting_id)?].meeting;
        let who = m
            .speakers
            .iter()
            .find(|p| p.id == seg.speaker_id)
            .map_or(seg.speaker_id.as_str(), |p| p.name.as_str());
        let row = serde_json::json!({
            "id": seg.id,
            "meeting_id": meeting_id,
            "text": format!("{who}: {}", seg.text),
            "labels": { answer_id: if value { "yes" } else { "no" } },
            "model_probability": model_probability,
            "at": jiff::Timestamp::now().to_string(),
        });
        let path = self.labels_path();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        writeln!(f, "{row}").map_err(|e| format!("{}: {e}", path.display()))
    }
}
