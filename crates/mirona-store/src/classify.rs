//! Classification results in the store (NGR-156/158/160): per-segment answers and tags, the
//! per-meeting review list, and corrections kept as labelled examples.
//!
//! `<root>/.mirona/labels.jsonl` gets one line per correction, local only:
//! `{"id", "meeting_id", "text", "labels": {"action": "yes"}, "model_probability", "at"}`.
//! `mirona eval` reads it next to the public `eval/chunks.jsonl`.

use crate::files::Entry;
use crate::{Result, Store};
use mirona_core::{ActionItem, Answer, Choice, Meeting, Review, Segment, Tag, TagKind};
use mirona_providers::questions::NO_OWNER;
use std::io::Write;
use std::path::PathBuf;

/// Who a segment's Action tag names: "Action · Inés" → Inés. The engine names an owner
/// there only when the owner choice clears its threshold (and not on a "Needs review" action).
fn tagged_owner(seg: &Segment) -> Option<&str> {
    seg.tags
        .iter()
        .filter(|t| t.kind == TagKind::Action)
        .find_map(|t| t.label.strip_prefix("Action · "))
        .filter(|o| *o != "confirmed")
}

/// Action items follow the classifier on the line they cite (NGR-156): the owner its Action
/// tag names, and the due date said there. Owners the user set stay.
pub(crate) fn fill_items(e: &mut Entry) {
    for a in &mut e.items {
        let Some(seg) = e
            .meeting
            .segments
            .iter()
            .find(|s| Some(s.start_ms) == a.at_ms)
        else {
            continue;
        };
        if let Some(owner) = tagged_owner(seg).filter(|_| !a.owner_confirmed) {
            a.owner = owner.into();
            a.needs_review = false;
        }
        if seg.due.is_some() {
            a.due.clone_from(&seg.due);
        }
    }
}

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
        due: Option<String>,
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
        seg.due = due;
        let seg = seg.clone();
        fill_items(&mut self.entries[i]);
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

    /// The user picks who owns an action (NGR-159): the owner choice, the action tag and the
    /// action items at that moment follow.
    pub fn set_owner(
        &mut self,
        meeting_id: &str,
        segment_id: &str,
        owner: &str,
    ) -> Result<Segment> {
        let owner = owner.trim();
        if owner.is_empty() {
            return Err("An owner needs a name.".into());
        }
        let i = self.idx(meeting_id)?;
        let e = &mut self.entries[i];
        let seg = e
            .meeting
            .segments
            .iter_mut()
            .find(|s| s.id == segment_id)
            .ok_or("no segment")?;
        seg.owners.retain(|o| o.label != owner);
        seg.owners.iter_mut().for_each(|o| o.probability = 0.0);
        seg.owners.insert(
            0,
            Choice {
                label: owner.into(),
                probability: 1.0,
            },
        );
        for t in seg.tags.iter_mut().filter(|t| t.kind == TagKind::Action) {
            t.label = format!("Action · {owner}");
        }
        let seg = seg.clone();
        for a in e.items.iter_mut().filter(|a| a.at_ms == Some(seg.start_ms)) {
            a.owner = owner.into();
            a.owner_confirmed = true;
        }
        self.save(i, true)?;
        self.label_owner(meeting_id, &seg, owner)?;
        Ok(seg)
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

    /// A meeting-level correction (meeting type, client, follow-up) as a labelled example: the
    /// meeting's transcript is the text, so `mirona eval` can re-ask the meeting questions.
    pub fn label_meeting(&self, meeting_id: &str, question: &str, value: &str) -> Result<()> {
        let m = &self.entries[self.idx(meeting_id)?].meeting;
        let mut text = String::new();
        for seg in m.segments.iter().filter(|s| !s.redacted) {
            let who = m
                .speakers
                .iter()
                .find(|p| p.id == seg.speaker_id)
                .map_or(seg.speaker_id.as_str(), |p| p.name.as_str());
            text.push_str(who);
            text.push_str(": ");
            text.push_str(&seg.text);
            text.push('\n');
            // ponytail: same cut as the meeting questions get (48k chars).
            if text.len() > 48_000 {
                break;
            }
        }
        self.write_label(&serde_json::json!({
            "id": format!("{meeting_id}-meeting"),
            "meeting_id": meeting_id,
            "text": text.trim_end(),
            "labels": { question: value },
            "at": jiff::Timestamp::now().to_string(),
        }))
    }

    /// The owner the user picked for an action, kept with the segment's other labels.
    fn label_owner(&self, meeting_id: &str, seg: &Segment, owner: &str) -> Result<()> {
        let m = &self.entries[self.idx(meeting_id)?].meeting;
        let who = m
            .speakers
            .iter()
            .find(|p| p.id == seg.speaker_id)
            .map_or(seg.speaker_id.as_str(), |p| p.name.as_str());
        self.write_label(&serde_json::json!({
            "id": seg.id,
            "meeting_id": meeting_id,
            "text": format!("{who}: {}", seg.text),
            "labels": { "owner": owner },
            "at": jiff::Timestamp::now().to_string(),
        }))
    }

    fn write_label(&self, row: &serde_json::Value) -> Result<()> {
        let path = self.labels_path();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let line = crate::crypt::seal_line(self.key.as_ref(), &row.to_string())?;
        writeln!(f, "{line}").map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The corrections file as plain JSONL, for `mirona eval`. Empty when there is none.
    pub fn labels_text(&self) -> Result<String> {
        let Ok(text) = std::fs::read_to_string(self.labels_path()) else {
            return Ok(String::new());
        };
        let mut out = String::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            out += &crate::crypt::open_line(self.key.as_ref(), line)?;
            out += "\n";
        }
        Ok(out)
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
        self.write_label(&row)
    }
}

/// An action item for a segment the user confirmed as an action and the summary missed.
pub(crate) fn item_from(m: &Meeting, seg: &Segment) -> ActionItem {
    let speaker = m.speakers.iter().find(|p| p.id == seg.speaker_id);
    let owner = seg
        .owners
        .first()
        .filter(|o| o.label != NO_OWNER)
        .map(|o| o.label.clone())
        .or_else(|| speaker.map(|p| p.name.clone()))
        .unwrap_or_default();
    ActionItem {
        id: format!("{}-ai-{}", m.id, seg.id),
        meeting_id: m.id.clone(),
        meeting_title: m.title.clone(),
        at_ms: Some(seg.start_ms),
        text: seg.text.clone(),
        owner,
        due: seg.due.clone(),
        client: m.client.clone(),
        done: false,
        needs_review: false,
        owner_confirmed: false,
    }
}
