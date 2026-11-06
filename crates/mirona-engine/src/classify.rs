//! Classification in the pipeline (NGR-155/156/157/158/162).
//!
//! - **Live, per segment:** a worker thread asks the segment questions (five yes/no, "is a due
//!   date said?", and "who owns it?" among the meeting's speakers) in one call per segment, with
//!   the previous lines as context. Capture never waits on it. Answers, tags and owners go to
//!   the store and out as `Classified`. When "is a due date said?" is yes, the phrase as said
//!   ("by Thursday") is kept on the segment ([`due_phrase`]).
//! - **After diarization:** owners chosen among speakers the meeting no longer has ("Them") are
//!   asked again among the real ones ([`refresh_owners`]). Action items take the owner and due
//!   date of the line they cite, unless the user set the owner (`Store::set_action_owner`).
//! - **After the call:** segments the worker missed (imports, failures) are classified, then the
//!   meeting questions (type → template, client, priority, tone, follow-up), the summary, the
//!   quality check ("does it cover every action item?" → one rewrite if not) and the follow-up
//!   draft.
//! - **Thresholds (NGR-158):** a model "yes" at or above its question's threshold becomes a tag;
//!   below it, a "Needs review" tag. Below-threshold meeting answers don't pick the template,
//!   file the client or write a draft.
//!
//! Jev is the default; privacy mode (or `[providers] classification = "local-llm"`, or no Jev
//! key) uses qwen3:4b through Ollama with the same questions.

use crate::{Labelled, Paths, Shared, block_on, stage, summarize_hinted};
use mirona_core::{
    ActionItem, Answer, AnswerSource, Choice, ChoiceAnswer, Locality, Meeting, MeetingAnswers,
    PipelineEvent, ProviderOption, Segment, Tag, TagKind,
};
use mirona_providers::config::Config;
use mirona_providers::jev::Jev;
use mirona_providers::ollama::Ollama;
use mirona_providers::questions::{
    Kind, NO_CLIENT, NO_OWNER, Question, Reply, coverage_question, meeting_questions, options,
    segment_questions, to_answers,
};
use mirona_providers::{
    Classifier, LlmProvider, Prompt, Provider, Result as PResult, check_privacy, keychain,
};
use std::collections::VecDeque;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

type Result<T> = std::result::Result<T, String>;

/// Previous lines sent along with a segment. Zero, measured: `mirona eval --context` (each
/// chunk asked with the previous one's last two lines, marked as context) took Jev from 0.94 to
/// 0.88 mean accuracy, action precision 0.95 → 0.62 and decision 0.73 → 0.57: the context's
/// own action or decision bleeds into the answer, which live would repeat a tag on the lines
/// after it. Thin ≤ 5 s segments are the price. ponytail: rerun on real meetings (NGR-163)
/// before raising it.
pub const CONTEXT_LINES: usize = 0;

/// Transcript characters asked about once per meeting (~12k tokens: fits Jev's 32k per
/// question and the 16k context we give Ollama). ponytail: a longer meeting is cut at the end;
/// ask over the summary or in parts if long meetings get the type wrong.
pub const MEETING_CHARS: usize = 48_000;

/// The configured classifier: Jev, or the local fallback.
pub enum AnyClassifier {
    Jev(Jev),
    Local(Ollama),
}

impl AnyClassifier {
    /// Jev unless privacy mode is on, the config says `local-llm`, or there is no Jev key in
    /// the keychain (account `jev`); `None` when the config says `off`. The second value says
    /// why it fell back, when it did against the config's wish.
    pub fn from_config(cfg: &Config) -> (Option<Self>, Option<String>) {
        let local = Some(Self::Local(Ollama::default()));
        match stage(cfg, "classification", "jev").as_str() {
            "off" => return (None, None),
            "local-llm" => return (local, None),
            _ => {}
        }
        let jev = ProviderOption {
            id: "jev".into(),
            name: "Jev".into(),
            locality: Locality::Cloud,
        };
        if check_privacy(&jev, cfg.privacy).is_err() {
            return (local, None);
        }
        match keychain::api_key("jev") {
            Ok(Some(key)) => (Some(Self::Jev(Jev::new(key))), None),
            _ => (
                local,
                Some("No Jev API key (Settings → API keys): tags come from the local model".into()),
            ),
        }
    }
}

impl Provider for AnyClassifier {
    fn id(&self) -> &str {
        match self {
            Self::Jev(c) => c.id(),
            Self::Local(c) => c.id(),
        }
    }
    fn locality(&self) -> Locality {
        match self {
            Self::Jev(c) => c.locality(),
            Self::Local(c) => c.locality(),
        }
    }
}

impl Classifier for AnyClassifier {
    async fn ask(&self, text: &str, questions: &[Question]) -> PResult<Vec<Reply>> {
        match self {
            Self::Jev(c) => c.ask(text, questions).await,
            Self::Local(c) => c.ask(text, questions).await,
        }
    }
}

fn is_local(c: &impl Provider) -> bool {
    c.locality() == Locality::Local
}

/// What a segment is classified with: earlier lines marked as context, then the segment.
pub fn segment_state(context: &[String], line: &str) -> String {
    if context.is_empty() {
        return line.into();
    }
    format!(
        "[Earlier lines, context only: judge only the transcript after them]\n{}\n[Transcript]\n{line}",
        context.join("\n")
    )
}

/// Tags from thresholded answers: a "yes" at/above threshold tags the segment; a "yes" below
/// it is "Needs review". An action names its owner when the owner choice clears its threshold.
pub fn tags(answers: &[Answer], owners: &[Choice], owner_threshold: f32) -> Vec<Tag> {
    answers
        .iter()
        .filter(|a| a.answer)
        .filter_map(|a| {
            let kind = TagKind::for_answer(&a.id)?;
            if a.needs_review() {
                return Some(mirona_store::review_tag(kind, a.probability));
            }
            let owner = owners
                .first()
                .filter(|o| o.probability >= owner_threshold && o.label != NO_OWNER);
            let label = match (kind, owner) {
                (TagKind::Action, Some(o)) => format!("Action · {}", o.label),
                _ => kind.label().into(),
            };
            Some(Tag { kind, label })
        })
        .collect()
}

pub use mirona_core::sensitive_words;

/// The safety-net tag, added when the classifier didn't tag the line sensitive itself.
pub fn add_sensitive_net(text: &str, tags: &mut Vec<Tag>) {
    if sensitive_words(text) && !tags.iter().any(|t| t.kind == TagKind::Sensitive) {
        tags.push(Tag {
            kind: TagKind::Sensitive,
            label: "Sensitive · keyword".into(),
        });
    }
}

/// Ask the segment questions about `line` (with `context`), thresholded per `cfg`.
pub fn classify_segment(
    c: &impl Classifier,
    cfg: &Config,
    speakers: &[String],
    context: &[String],
    line: &str,
) -> PResult<(Vec<Answer>, Vec<Tag>, Vec<Choice>)> {
    let qs = segment_questions(speakers);
    let replies = block_on(c.ask(&segment_state(context, line), &qs))?;
    let (mut answers, mut owners) = to_answers(&qs, &replies);
    let local = is_local(c);
    for a in &mut answers {
        a.threshold = cfg.threshold(&a.id, local);
    }
    if !answers.iter().any(|a| a.id == "action" && a.answer) {
        owners.clear();
    }
    let mut tags = tags(&answers, &owners, cfg.threshold("owner", local));
    add_sensitive_net(line, &mut tags);
    Ok((answers, tags, owners))
}

fn speaker_name(m: &Meeting, s: &Segment) -> String {
    m.speakers
        .iter()
        .find(|p| p.id == s.speaker_id)
        .map_or_else(|| s.speaker_id.clone(), |p| p.name.clone())
}

/// Classify `segments` of meeting `id` in order, storing each result and sending `Classified`.
/// Returns how many failed (the first failure goes out as a notice).
fn run(
    store: &Shared,
    id: &str,
    c: &impl Classifier,
    cfg: &Config,
    events: &Sender<PipelineEvent>,
    segments: impl IntoIterator<Item = Segment>,
) -> usize {
    let mut context: VecDeque<String> = VecDeque::new();
    let mut failed = 0;
    for seg in segments {
        let Ok(m) = store.lock().unwrap().meeting(id) else {
            return failed;
        };
        let speakers: Vec<String> = m.speakers.iter().map(|p| p.name.clone()).collect();
        let line = format!("{}: {}", speaker_name(&m, &seg), seg.text);
        let ctx: Vec<String> = context.iter().cloned().collect();
        let res = classify_segment(c, cfg, &speakers, &ctx, &line).map_err(|e| e.to_string());
        let res = res.and_then(|(answers, tags, owners)| {
            let due_said = answers
                .iter()
                .any(|a| a.id == "due" && a.answer && !a.needs_review());
            let due = due_said.then(|| due_phrase(&seg.text)).flatten();
            store
                .lock()
                .unwrap()
                .set_classification(id, &seg.id, answers, tags, owners, due)
        });
        match res {
            Ok(s) => {
                // A sensitive line isn't sent again as context.
                if !s.is_sensitive() {
                    context.push_back(line);
                }
                events
                    .send(PipelineEvent::Classified {
                        meeting_id: id.into(),
                        segment_id: s.id,
                        tags: s.tags,
                        answers: s.answers,
                        owners: s.owners,
                    })
                    .ok();
            }
            Err(e) => {
                if failed == 0 {
                    events
                        .send(crate::notice(id, format!("Tagging failed: {e}"), true))
                        .ok();
                }
                failed += 1;
                context.push_back(line);
            }
        }
        while context.len() > CONTEXT_LINES {
            context.pop_front();
        }
    }
    failed
}

/// The due date as said in `text` ("by Thursday", "by the end of the month", "next Friday",
/// "tomorrow", "in two weeks", "March 3"), taken when "is a due date said?" is yes. A small
/// word pass, not a date parser: the phrase is kept as said, never turned into a date.
/// ponytail: English only and the first match wins; ask a choice question if it misses
/// phrasings in real meetings (NGR-156).
pub fn due_phrase(text: &str) -> Option<String> {
    const DAYS: [&str; 10] = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "tomorrow",
        "tonight",
        "today",
    ];
    const SHORT: [&str; 6] = ["eod", "eow", "eom", "cob", "noon", "midnight"];
    const PERIODS: [&str; 9] = [
        "day",
        "week",
        "month",
        "quarter",
        "year",
        "sprint",
        "morning",
        "afternoon",
        "evening",
    ];
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    const BEFORE: [&str; 7] = ["by", "before", "until", "till", "on", "due", "for"];
    let words: Vec<(usize, &str)> = text
        .split_whitespace()
        .map(|w| (w.as_ptr().addr() - text.as_ptr().addr(), w))
        .collect();
    let norm: Vec<String> = words
        .iter()
        .map(|(_, w)| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect();
    let at = |i: usize| norm.get(i).map_or("", String::as_str);
    let numberish = |w: &str| {
        w.trim_end_matches(|c: char| c.is_alphabetic())
            .parse::<u32>()
            .is_ok()
            || ["a", "one", "two", "three", "four", "few", "couple"].contains(&w)
    };
    let date = |w: &str| w.contains('/') && w.split('/').all(|p| p.parse::<u32>().is_ok());
    for i in 0..norm.len() {
        let w = at(i);
        // (first word, last word) of the phrase.
        let span = if DAYS.contains(&w) || SHORT.contains(&w) || date(w) {
            Some((i, i))
        } else if (w == "next" || w == "this")
            && (DAYS.contains(&at(i + 1)) || PERIODS.contains(&at(i + 1)))
        {
            Some((i, i + 1))
        } else if w == "end" && at(i + 1) == "of" {
            let k = if at(i + 2) == "the" { i + 3 } else { i + 2 };
            let k = if at(k) == "next" || at(k) == "this" {
                k + 1
            } else {
                k
            };
            (PERIODS.contains(&at(k)) || DAYS.contains(&at(k)) || MONTHS.contains(&at(k)))
                .then_some((i, k))
        } else if MONTHS.contains(&w) && (w != "may" || numberish(at(i + 1))) {
            Some((i, if numberish(at(i + 1)) { i + 1 } else { i }))
        } else if w == "in" && numberish(at(i + 1)) {
            let k = if at(i + 2) == "of" { i + 3 } else { i + 2 };
            ["days", "weeks", "months", "day", "week", "month", "hours"]
                .contains(&at(k))
                .then_some((i, k))
        } else {
            None
        };
        let Some((mut first, last)) = span else {
            continue;
        };
        // "on a" / "of" etc. aren't part of it; "by", "by the", "no later than" are.
        if first > 0 && at(first - 1) == "the" && first > 1 && BEFORE.contains(&at(first - 2)) {
            first -= 2;
        } else if first > 0 && BEFORE.contains(&at(first - 1)) {
            first -= 1;
        }
        let start = words[first].0;
        let (end_at, end_word) = words[last];
        let phrase =
            text[start..end_at + end_word.len()].trim_end_matches(|c: char| !c.is_alphanumeric());
        return Some(phrase.to_string());
    }
    None
}

/// After diarization (NGR-156): action items whose owners were chosen among speakers the
/// meeting no longer has ("Them", before it was split into people) are asked "who owns it?"
/// again among the real ones; their tags, and the action items citing them, follow. Sensitive
/// lines stay off cloud classifiers. Returns how many segments were asked.
pub fn refresh_owners(
    store: &Shared,
    cfg: &Config,
    id: &str,
    c: &impl Classifier,
    events: &Sender<PipelineEvent>,
) -> Result<usize> {
    let m = store.lock().unwrap().meeting(id)?;
    let names: Vec<String> = m.speakers.iter().map(|p| p.name.clone()).collect();
    let local = is_local(c);
    let qs: Vec<Question> = segment_questions(&names)
        .into_iter()
        .filter(|q| q.id == "owner")
        .collect();
    let stale = |s: &Segment| {
        s.answers.iter().any(|a| a.id == "action" && a.answer)
            && (s.owners.is_empty()
                || s.owners
                    .iter()
                    .any(|o| o.label != NO_OWNER && !names.contains(&o.label)))
            && !s.redacted
            && (local || !s.is_sensitive())
    };
    let mut asked = 0;
    for seg in m.segments.iter().filter(|s| stale(s)) {
        let line = format!("{}: {}", speaker_name(&m, seg), seg.text);
        let replies = block_on(c.ask(&line, &qs)).map_err(|e| e.to_string())?;
        let (_, owners) = to_answers(&qs, &replies);
        let tags = tags(&seg.answers, &owners, cfg.threshold("owner", local));
        let s = store.lock().unwrap().set_classification(
            id,
            &seg.id,
            seg.answers.clone(),
            tags,
            owners,
            seg.due.clone(),
        )?;
        asked += 1;
        events
            .send(PipelineEvent::Classified {
                meeting_id: id.into(),
                segment_id: s.id,
                tags: s.tags,
                answers: s.answers,
                owners: s.owners,
            })
            .ok();
    }
    Ok(asked)
}

/// The live worker: segments go in through the sender as they're transcribed; it ends once
/// every sender is dropped and the queue is drained.
pub fn live<C: Classifier + Send + Sync + 'static>(
    store: Shared,
    id: String,
    c: C,
    cfg: Config,
    events: Sender<PipelineEvent>,
) -> (Sender<Segment>, JoinHandle<()>) {
    let (tx, rx) = channel::<Segment>();
    let h = std::thread::spawn(move || {
        run(&store, &id, &c, &cfg, &events, rx);
    });
    (tx, h)
}

/// The meeting as `Name: text` lines for the meeting questions; sensitive lines stay off cloud
/// classifiers, redacted ones off all.
pub fn meeting_text(m: &Meeting, cloud: bool) -> String {
    let mut out = String::new();
    for s in &m.segments {
        let sensitive = s.is_sensitive();
        if s.redacted || (cloud && sensitive) {
            continue;
        }
        let line = format!("{}: {}\n", speaker_name(m, s), s.text);
        if out.len() + line.len() > MEETING_CHARS {
            break;
        }
        out.push_str(&line);
    }
    out
}

fn choice(q: Option<(&Question, &Reply)>, threshold: f32) -> ChoiceAnswer {
    let (value, probability) = q
        .and_then(|(_, r)| match r {
            Reply::Choice { value, .. } => Some((value.clone(), r.confidence())),
            _ => None,
        })
        .unwrap_or_default();
    ChoiceAnswer {
        value,
        probability,
        threshold,
        options: q.map(|(q, _)| options(q)).unwrap_or_default(),
        confirmed: false,
    }
}

/// Ask the meeting questions over the whole transcript and store the answers. A client the
/// user already confirmed is kept.
pub fn classify_meeting(
    store: &Shared,
    id: &str,
    c: &impl Classifier,
    cfg: &Config,
) -> Result<MeetingAnswers> {
    let m = store.lock().unwrap().meeting(id)?;
    let text = meeting_text(&m, !is_local(c));
    if text.trim().is_empty() {
        return Err("nothing to classify".into());
    }
    let qs = meeting_questions(cfg.clients.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let replies = block_on(c.ask(&text, &qs)).map_err(|e| e.to_string())?;
    let local = is_local(c);
    let get = |id: &str| qs.iter().zip(&replies).find(|(q, _)| q.id == id);
    let level = |id: &str| match get(id) {
        Some((_, Reply::Score { score, .. })) => *score,
        _ => 0.0,
    };
    let p_follow = get("follow_up").and_then(|(_, r)| r.p_yes()).unwrap_or(0.0);
    let mut answers = MeetingAnswers {
        meeting_type: choice(get("meeting_type"), cfg.threshold("meeting_type", local)),
        client: choice(get("client"), cfg.threshold("client", local)),
        priority: level("priority"),
        tone: level("tone"),
        follow_up: p_follow >= 0.5,
        follow_up_probability: p_follow,
        follow_up_threshold: cfg.threshold("follow_up", local),
        follow_up_draft: String::new(),
    };
    if answers.client.value == NO_CLIENT {
        answers.client.value.clear();
    }
    if let Some(old) = m.answers.filter(|a| a.client.confirmed) {
        answers.client = old.client;
    }
    store.lock().unwrap().set_answers(id, answers.clone())?;
    Ok(answers)
}

/// The summary template for a meeting type at/above its threshold; `default` otherwise.
pub fn template_for(a: &MeetingAnswers, default: &str) -> String {
    let t = &a.meeting_type;
    if !t.confirmed && t.probability < t.threshold {
        return default.into();
    }
    match t.value.as_str() {
        "standup" => "standup",
        "client_call" => "client-call",
        "one_on_one" => "one-on-one",
        "planning" => "planning",
        _ => default,
    }
    .into()
}

/// The summary quality check (NGR-157): how well the summary's action items cover the ones
/// tagged in the transcript, 0..=1, plus the tagged lines. `None` when nothing was tagged.
pub fn coverage(
    store: &Shared,
    id: &str,
    c: &impl Classifier,
) -> Result<Option<(f32, Vec<String>)>> {
    let (m, items) = {
        let s = store.lock().unwrap();
        let items: Vec<ActionItem> = s
            .action_items()
            .into_iter()
            .filter(|a| a.meeting_id == id)
            .collect();
        (s.meeting(id)?, items)
    };
    let cloud = !is_local(c);
    let said: Vec<String> = m
        .segments
        .iter()
        .filter(|s| s.tags.iter().any(|t| t.kind == TagKind::Action))
        .filter(|s| !(cloud && s.is_sensitive()))
        .map(|s| format!("- {}: {}", speaker_name(&m, s), s.text))
        .collect();
    if said.is_empty() {
        return Ok(None);
    }
    let summary: Vec<String> = items
        .iter()
        .map(|a| format!("- {} ({})", a.text, a.owner))
        .collect();
    let summary = if summary.is_empty() {
        "(none)".to_string()
    } else {
        summary.join("\n")
    };
    let state = format!(
        "Action items said in the meeting:\n{}\n\nAction items in the summary:\n{summary}",
        said.join("\n")
    );
    let q = coverage_question();
    let levels = match &q.kind {
        Kind::Score(l) => l.len(),
        _ => unreachable!("coverage is a score"),
    };
    let replies = block_on(c.ask(&state, std::slice::from_ref(&q))).map_err(|e| e.to_string())?;
    let Some(Reply::Score { score, .. }) = replies.first() else {
        return Err("coverage: no score".into());
    };
    #[allow(clippy::cast_precision_loss)]
    let top = (levels - 1) as f32;
    Ok(Some(((score / top).clamp(0.0, 1.0), said)))
}

const FOLLOW_UP_RULES: &str = "You write a short follow-up email after a meeting, from the \
facts below only. Plain text: a one-line greeting, one sentence of thanks, then short sections \
for decisions, action items (who, what, by when) and open questions; leave out empty sections. \
Use only the names listed as confirmed; write \"someone\" for any other owner. Never invent \
facts, dates or names. End with \"Best,\" and no name. Write in the language of the facts.";

/// Draft the follow-up email (NGR-162) when "needs a follow-up?" is yes at/above threshold:
/// decisions, action items and open questions, only above-threshold ones, with confirmed names.
pub fn follow_up_draft(store: &Shared, id: &str, llm: &impl LlmProvider) -> Result<Option<String>> {
    let (m, items) = {
        let s = store.lock().unwrap();
        let items: Vec<ActionItem> = s
            .action_items()
            .into_iter()
            .filter(|a| a.meeting_id == id && !a.needs_review)
            .collect();
        (s.meeting(id)?, items)
    };
    let Some(mut answers) = m.answers.clone() else {
        return Ok(None);
    };
    if !answers.follow_up || answers.follow_up_probability < answers.follow_up_threshold {
        return Ok(None);
    }
    let Some(summary) = &m.summary else {
        return Ok(None);
    };
    // Names a person confirmed: enrolled voices. "Me" is the user.
    let confirmed: Vec<&str> = m
        .speakers
        .iter()
        .filter(|p| p.voiceprint.is_some())
        .map(|p| p.name.as_str())
        .collect();
    // A decision or question cited on a line still waiting for review stays out.
    let unsure = |at_ms: u32, kind: TagKind| {
        let prefix = format!("Needs review · {}?", kind.label().to_lowercase());
        m.segments.iter().any(|s| {
            s.start_ms == at_ms
                && s.tags
                    .iter()
                    .any(|t| t.kind == TagKind::Review && t.label.starts_with(&prefix))
        })
    };
    let list = |v: &[mirona_core::Decision], kind| -> Vec<String> {
        v.iter()
            .filter(|d| !unsure(d.at_ms, kind))
            .map(|d| format!("- {}", d.text))
            .collect()
    };
    // An item said on a sensitive line never goes into an email (NGR-166).
    let secret = |at: Option<u32>| {
        m.segments
            .iter()
            .any(|s| Some(s.start_ms) == at && s.is_sensitive())
    };
    let actions: Vec<String> = items
        .iter()
        .filter(|a| !secret(a.at_ms))
        .map(|a| {
            let owner = if confirmed.contains(&a.owner.as_str()) {
                a.owner.as_str()
            } else if a.owner.eq_ignore_ascii_case("me") {
                "me"
            } else {
                "someone"
            };
            let due = a
                .due
                .as_deref()
                .map(|d| format!(" (due {d})"))
                .unwrap_or_default();
            format!("- {owner}: {}{due}", a.text)
        })
        .collect();
    let user = format!(
        "Meeting: {}\nConfirmed names: {}\n\nDecisions:\n{}\n\nAction items:\n{}\n\nOpen questions:\n{}",
        m.title,
        if confirmed.is_empty() {
            "(none)".into()
        } else {
            confirmed.join(", ")
        },
        list(&summary.decisions, TagKind::Decision).join("\n"),
        actions.join("\n"),
        list(&summary.open_questions, TagKind::OpenQuestion).join("\n"),
    );
    let prompt = Prompt {
        system: FOLLOW_UP_RULES,
        user: &user,
        json: false,
    };
    let draft = block_on(llm.complete(&prompt, &mut |_| {}))
        .map_err(|e| e.to_string())?
        .text;
    let draft = draft
        .split_once("</think>")
        .map_or(draft.as_str(), |(_, a)| a)
        .trim()
        .to_string();
    answers.follow_up_draft.clone_from(&draft);
    store.lock().unwrap().set_answers(id, answers)?;
    Ok(Some(draft))
}

/// Ask everything again for a finished meeting ("Re-run classification"): every segment the
/// user hasn't corrected, then the meeting questions. Returns how many segments failed.
/// The summary is left alone (rewrite it with the template the new type picks).
pub fn reclassify(
    store: &Shared,
    cfg: &Config,
    id: &str,
    c: &impl Classifier,
    events: &Sender<PipelineEvent>,
) -> Result<usize> {
    let segs: Vec<Segment> = store.lock().unwrap().meeting(id)?.segments;
    let user = |s: &Segment| s.answers.iter().any(|a| a.source == AnswerSource::User);
    let failed = run(
        store,
        id,
        c,
        cfg,
        events,
        segs.into_iter().filter(|s| !user(s)),
    );
    classify_meeting(store, id, c, cfg)?;
    Ok(failed)
}

/// Everything after the call, after diarization: tag the segments still untagged, answer the
/// meeting questions, summarize with the template they pick, check the summary covers every
/// action item (rewrite once if not), and draft the follow-up email. Failures are notices.
/// Without a classifier (`classification = "off"`) it's the plain M1 summary.
pub fn after_call(
    store: &Shared,
    paths: &Paths,
    cfg: &Config,
    id: &str,
    c: Option<&impl Classifier>,
    llm: std::result::Result<&(impl LlmProvider + Labelled), String>,
    events: &Sender<PipelineEvent>,
) {
    let notice = |message: String, error: bool| {
        events.send(crate::notice(id, message, error)).ok();
    };
    // A broken LLM is said first; the tags and meeting answers still get done.
    let llm = llm.map_err(|e| notice(format!("No summary: {e}"), true));
    let template = match c {
        Some(c) => classify_all(store, cfg, id, c, events),
        None => cfg.template.clone(),
    };
    let Ok(llm) = llm else { return };
    let summarize = |hint: &str| {
        summarize_hinted(
            store,
            paths,
            id,
            &template,
            hint,
            llm,
            &mut crate::preview(id, events),
        )
    };
    if let Err(e) = summarize("") {
        return notice(format!("Summary failed: {e}"), true);
    }
    if let Some(c) = c {
        check_summary(store, cfg, id, c, &summarize, &notice);
    }
    if let Err(e) = follow_up_draft(store, id, llm) {
        notice(format!("Follow-up draft failed: {e}"), true);
    }
    events
        .send(PipelineEvent::Summarized {
            meeting_id: id.into(),
        })
        .ok();
}

/// Tag the segments still untagged and answer the meeting questions; returns the template.
fn classify_all(
    store: &Shared,
    cfg: &Config,
    id: &str,
    c: &impl Classifier,
    events: &Sender<PipelineEvent>,
) -> String {
    let pending: Vec<Segment> = store.lock().unwrap().meeting(id).map_or(vec![], |m| {
        m.segments
            .into_iter()
            .filter(|s| s.answers.is_empty())
            .collect()
    });
    run(store, id, c, cfg, events, pending);
    match classify_meeting(store, id, c, cfg) {
        Ok(a) => template_for(&a, &cfg.template),
        Err(e) => {
            events
                .send(crate::notice(
                    id,
                    format!("Meeting questions failed: {e}"),
                    true,
                ))
                .ok();
            cfg.template.clone()
        }
    }
}

/// The quality check: rewrite the summary once if it misses tagged action items.
fn check_summary(
    store: &Shared,
    cfg: &Config,
    id: &str,
    c: &impl Classifier,
    summarize: &dyn Fn(&str) -> Result<()>,
    notice: &dyn Fn(String, bool),
) {
    match coverage(store, id, c) {
        Ok(Some((cov, said))) if cov < cfg.threshold("coverage", is_local(c)) => {
            notice(
                format!("The summary missed action items ({cov:.2}); rewriting it once"),
                false,
            );
            let hint = format!(
                "\n\nThe action items list must cover every one of these lines:\n{}",
                said.join("\n")
            );
            match summarize(&hint).and_then(|()| coverage(store, id, c)) {
                Ok(Some((after, _))) => note_coverage(store, id, after, c),
                Ok(None) => {}
                Err(e) => notice(format!("Summary rewrite failed: {e}"), true),
            }
        }
        Ok(Some((cov, _))) => note_coverage(store, id, cov, c),
        Ok(None) => {}
        Err(e) => notice(format!("Summary check failed: {e}"), true),
    }
}

/// Adds "covers action items 0.92 (jev)" to `Summary.quality`.
fn note_coverage(store: &Shared, id: &str, cov: f32, c: &impl Provider) {
    let mut s = store.lock().unwrap();
    let Ok(Some(mut summary)) = s.meeting(id).map(|m| m.summary) else {
        return;
    };
    let items: Vec<ActionItem> = s
        .action_items()
        .into_iter()
        .filter(|a| a.meeting_id == id)
        .collect();
    let note = format!("covers action items {cov:.2} ({})", c.id());
    summary.quality = if summary.quality.is_empty() {
        note
    } else {
        format!("{} · {note}", summary.quality)
    };
    s.set_summary(id, summary, items).ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirona_store::Store;
    use std::sync::{Arc, Mutex};

    /// Answers like a model would, from the text; logs every state it was asked about.
    #[derive(Clone, Default)]
    struct Scripted {
        states: Arc<Mutex<Vec<String>>>,
    }

    impl Provider for Scripted {
        fn id(&self) -> &'static str {
            "scripted"
        }
        fn locality(&self) -> Locality {
            Locality::Cloud
        }
    }

    impl Classifier for Scripted {
        fn ask(
            &self,
            text: &str,
            questions: &[Question],
        ) -> impl Future<Output = PResult<Vec<Reply>>> + Send {
            self.states.lock().unwrap().push(text.into());
            let last = text.lines().last().unwrap_or_default();
            let replies = questions
                .iter()
                .map(|q| match q.id.as_str() {
                    "action" => Reply::YesNo(if last.contains("I'll") { 0.9 } else { 0.05 }),
                    // Below decision's 0.95: needs review.
                    "decision" => Reply::YesNo(if last.contains("go with") { 0.7 } else { 0.05 }),
                    "due" => {
                        let said = last.contains("Thursday") || last.contains("month");
                        Reply::YesNo(if said { 0.8 } else { 0.1 })
                    }
                    "follow_up" => Reply::YesNo(0.9),
                    "owner" => {
                        let who = last.split(':').next().unwrap_or_default().to_string();
                        let probabilities = options(q)
                            .into_iter()
                            .map(|o| {
                                let p = if o == who { 0.9 } else { 0.05 };
                                (o, p)
                            })
                            .collect();
                        Reply::Choice {
                            value: who,
                            probabilities,
                        }
                    }
                    "meeting_type" => Reply::Choice {
                        value: "client_call".into(),
                        probabilities: vec![("client_call".into(), 0.9)],
                    },
                    // Below the 0.6 threshold: not filed.
                    "client" => Reply::Choice {
                        value: "Northwind Bank".into(),
                        probabilities: vec![("Northwind Bank".into(), 0.4)],
                    },
                    "priority" | "tone" => Reply::Score {
                        score: 3.0,
                        probabilities: vec![0.0, 0.0, 0.0, 1.0],
                    },
                    // Covered only once the summary names the SOW.
                    "coverage" => {
                        let ok = text.split("in the summary").nth(1).unwrap().contains("SOW");
                        Reply::Score {
                            score: if ok { 2.0 } else { 0.0 },
                            probabilities: vec![],
                        }
                    }
                    _ => Reply::YesNo(0.05),
                })
                .collect();
            std::future::ready(Ok(replies))
        }
    }

    /// Summaries miss the action item unless told about it; drafts a fixed email.
    struct Llm;

    impl Provider for Llm {
        fn id(&self) -> &'static str {
            "llm"
        }
        fn locality(&self) -> Locality {
            Locality::Local
        }
    }

    impl Labelled for Llm {
        fn label(&self) -> String {
            "llm (Test)".into()
        }
    }

    impl LlmProvider for Llm {
        fn complete(
            &self,
            prompt: &Prompt<'_>,
            on_text: &mut (dyn FnMut(&str) + Send),
        ) -> impl Future<Output = PResult<mirona_providers::Completion>> + Send {
            let text = if prompt.system.contains("follow-up email") {
                assert!(
                    prompt.user.contains("- me: Send the revised SOW"),
                    "{}",
                    prompt.user
                );
                "Hi all,\n\nThanks for today.\n\nBest,".to_string()
            } else if prompt.system.contains("must cover every one") {
                r#"{"overview": "SOW and price.", "decisions": [],
                    "action_items": [{"text": "Send the revised SOW", "owner": "Me", "segment": 0}],
                    "open_questions": []}"#
                    .to_string()
            } else {
                r#"{"overview": "Price.", "decisions": [], "action_items": [], "open_questions": []}"#
                    .to_string()
            };
            on_text(&text);
            std::future::ready(Ok(mirona_providers::Completion {
                text,
                usage: mirona_providers::Usage::default(),
            }))
        }
    }

    fn meeting(dir: &std::path::Path) -> (Shared, Vec<Segment>) {
        let mut s = Store::open(dir.join("meetings")).unwrap();
        s.start_meeting("m1").unwrap();
        let segs: Vec<Segment> = [
            ("me", "I'll send the revised SOW by Thursday."),
            ("them", "Great, then we go with the fixed price."),
        ]
        .iter()
        .enumerate()
        .map(|(n, (who, text))| Segment {
            id: format!("m1-{n}"),
            speaker_id: (*who).into(),
            start_ms: u32::try_from(n).unwrap() * 4000,
            end_ms: u32::try_from(n).unwrap() * 4000 + 3000,
            text: (*text).into(),
            redacted: false,
            tags: vec![],
            answers: vec![],
            owners: vec![],
            due: None,
        })
        .collect();
        for seg in &segs {
            s.push_segment("m1", seg.clone()).unwrap();
        }
        (Arc::new(Mutex::new(s)), segs)
    }

    fn cfg() -> Config {
        let mut cfg = Config::default();
        cfg.clients
            .insert("Northwind Bank".into(), "contact: Karen".into());
        cfg
    }

    #[test]
    fn obvious_secrets_are_sensitive_whatever_the_model_says() {
        for yes in [
            "The salary would be 182,000 a year.",
            "the staging password is Tulip4471",
            "My PIN is 4471.",
            "card 4111 1111 1111 1111",
            "IBAN DE89 3704 0044 0532 0130 00",
            "She was diagnosed last spring.",
        ] {
            assert!(sensitive_words(yes), "{yes}");
        }
        for no in [
            "We agreed to ship on Monday.",
            "Pinned the issue to the board.",
            "It costs 4,500 a month.",
            "Call me at 3 pm on the 14th.",
            "Our compensation for travel is fine.",
        ] {
            assert!(!sensitive_words(no), "{no}");
        }
        let mut tags = vec![];
        add_sensitive_net("the password is hunter2", &mut tags);
        add_sensitive_net("the password is hunter2", &mut tags);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].kind, TagKind::Sensitive);
    }

    #[test]
    fn live_segments_are_tagged_with_context_and_thresholds() {
        let dir = tempfile::tempdir().unwrap();
        let (store, segs) = meeting(dir.path());
        let c = Scripted::default();
        let (events, rx) = channel();
        let (tx, worker) = live(store.clone(), "m1".into(), c.clone(), cfg(), events);
        for s in segs {
            tx.send(s).unwrap();
        }
        drop(tx);
        worker.join().unwrap();

        let tagged: Vec<(String, Vec<Tag>)> = rx
            .try_iter()
            .filter_map(|e| match e {
                PipelineEvent::Classified {
                    segment_id, tags, ..
                } => Some((segment_id, tags)),
                _ => None,
            })
            .collect();
        assert_eq!(tagged.len(), 2);
        assert_eq!(tagged[0].1[0].label, "Action · Me");
        assert_eq!(tagged[1].1[0].kind, TagKind::Review, "{tagged:?}");
        assert_eq!(tagged[1].1[0].label, "Needs review · decision? 0.70");

        // Each segment is asked about alone, with its speaker's name (CONTEXT_LINES = 0).
        let states = c.states.lock().unwrap().clone();
        assert_eq!(states[1], "Them: Great, then we go with the fixed price.");
        assert!(segment_state(&["A: x".into()], "B: y").ends_with("[Transcript]\nB: y"));

        let s = store.lock().unwrap();
        let m = s.meeting("m1").unwrap();
        let due = m.segments[0]
            .answers
            .iter()
            .find(|a| a.id == "due")
            .unwrap();
        assert!(due.answer);
        assert_eq!(m.segments[0].owners[0].label, "Me");
        assert_eq!(m.segments[1].owners.len(), 0, "owners only for actions");
        let review = s.review("m1").unwrap();
        assert_eq!(review.len(), 1);
        assert_eq!(review[0].question, "decision");
        assert!((review[0].threshold - 0.95).abs() < 1e-6);
    }

    #[test]
    fn after_the_call_answers_pick_the_template_check_the_summary_and_draft() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = meeting(dir.path());
        let paths = Paths {
            meetings: dir.path().join("meetings"),
            models: dir.path().join("models"),
            templates: dir.path().join("templates"),
        };
        let c = Scripted::default();
        let (events, rx) = channel();
        after_call(&store, &paths, &cfg(), "m1", Some(&c), Ok(&Llm), &events);
        let events: Vec<PipelineEvent> = rx.try_iter().collect();
        let errors: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, PipelineEvent::Notice { error: true, .. }))
            .collect();
        assert_eq!(errors.len(), 0, "{errors:?}");
        assert!(matches!(
            events.last(),
            Some(PipelineEvent::Summarized { .. })
        ));

        let s = store.lock().unwrap();
        let m = s.meeting("m1").unwrap();
        // Segments the live worker never saw were tagged here.
        assert!(m.segments.iter().all(|s| !s.answers.is_empty()));
        let a = m.answers.clone().unwrap();
        assert_eq!(a.meeting_type.value, "client_call");
        // Client below threshold: suggested, not filed.
        assert_eq!(a.client.value, "Northwind Bank");
        assert_eq!(m.client, "Unsorted");
        assert!(
            s.review("m1")
                .unwrap()
                .iter()
                .any(|r| r.question == "client")
        );
        assert!((a.priority - 3.0).abs() < 1e-6 && (a.tone - 3.0).abs() < 1e-6);
        // Meeting type picked the template; the first summary missed the action item, so it
        // was rewritten once and now covers it.
        let summary = m.summary.unwrap();
        assert_eq!(summary.template, "Client call");
        assert!(
            summary.quality.contains("covers action items 1.00"),
            "{}",
            summary.quality
        );
        let items = s.action_items();
        assert_eq!(items[0].text, "Send the revised SOW");
        assert!(!items[0].needs_review, "line 0 is a tagged action");
        // Follow-up said yes above threshold: drafted from the confirmed items.
        assert_eq!(a.follow_up_draft, "Hi all,\n\nThanks for today.\n\nBest,");
    }

    #[test]
    fn reclassify_asks_again_but_keeps_corrections() {
        let dir = tempfile::tempdir().unwrap();
        let (store, segs) = meeting(dir.path());
        let c = Scripted::default();
        let (events, _rx) = channel();
        assert_eq!(reclassify(&store, &cfg(), "m1", &c, &events).unwrap(), 0);
        let fix = mirona_core::Correction {
            segment_id: segs[0].id.clone(),
            answer_id: "action".into(),
            value: false,
        };
        store.lock().unwrap().correct(fix).unwrap();
        c.states.lock().unwrap().clear();
        reclassify(&store, &cfg(), "m1", &c, &events).unwrap();
        let asked = c.states.lock().unwrap().clone();
        assert!(
            !asked.iter().any(|s| s.contains("SOW") && !s.contains('\n')),
            "{asked:?}"
        );
        let m = store.lock().unwrap().meeting("m1").unwrap();
        assert!(!m.segments[0].tags.iter().any(|t| t.kind == TagKind::Action));
        assert!(m.answers.is_some());
    }

    #[test]
    fn below_threshold_meeting_type_keeps_the_default_template() {
        let mut a = MeetingAnswers {
            meeting_type: ChoiceAnswer {
                value: "standup".into(),
                probability: 0.5,
                threshold: 0.6,
                options: vec![],
                confirmed: false,
            },
            client: choice(None, 0.6),
            priority: 0.0,
            tone: 0.0,
            follow_up: false,
            follow_up_probability: 0.0,
            follow_up_threshold: 0.6,
            follow_up_draft: String::new(),
        };
        assert_eq!(template_for(&a, "client-call"), "client-call");
        a.meeting_type.confirmed = true;
        assert_eq!(template_for(&a, "client-call"), "standup");
    }

    #[test]
    fn privacy_mode_uses_the_local_classifier() {
        let (c, why) = AnyClassifier::from_config(&Config::default());
        assert!(matches!(c, Some(AnyClassifier::Local(_))) && why.is_none());
        let mut cfg = Config {
            privacy: false,
            ..Config::default()
        };
        cfg.providers
            .insert("classification".into(), "local-llm".into());
        let c = AnyClassifier::from_config(&cfg).0.unwrap();
        assert_eq!(c.locality(), Locality::Local);
        cfg.providers.insert("classification".into(), "off".into());
        assert!(AnyClassifier::from_config(&cfg).0.is_none());
    }

    #[test]
    fn due_phrases_are_kept_as_said() {
        let cases = [
            (
                "I'll send the revised SOW by Thursday.",
                Some("by Thursday"),
            ),
            (
                "Can you get it done by the end of the month?",
                Some("by the end of the month"),
            ),
            ("Draft it by end of week, please", Some("by end of week")),
            ("Let's ship it next Friday!", Some("next Friday")),
            ("I'll have numbers in two weeks.", Some("in two weeks")),
            ("Due March 3, no later.", Some("Due March 3")),
            ("Send it before 3/14.", Some("before 3/14")),
            ("I'll do it tomorrow", Some("tomorrow")),
            ("Ping me EOD.", Some("EOD")),
            ("We may want to revisit pricing.", None),
            ("Thanks for the update.", None),
        ];
        for (text, want) in cases {
            assert_eq!(due_phrase(text).as_deref(), want, "{text}");
        }
    }

    fn speaker(id: &str, name: &str) -> mirona_core::Speaker {
        mirona_core::Speaker {
            id: id.into(),
            name: name.into(),
            initial: name[..1].into(),
            tint: mirona_core::Tint::Indigo,
            subtitle: String::new(),
            voiceprint: None,
            suggestion: None,
        }
    }

    fn item(n: u32, at_ms: u32, owner: &str) -> ActionItem {
        ActionItem {
            id: format!("m1-ai-{n}"),
            meeting_id: "m1".into(),
            meeting_title: String::new(),
            at_ms: Some(at_ms),
            text: "Draft the contract".into(),
            owner: owner.into(),
            due: None,
            client: String::new(),
            done: false,
            needs_review: false,
            owner_confirmed: false,
        }
    }

    #[test]
    fn after_diarization_owners_are_asked_again_and_items_follow() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut segs) = meeting(dir.path());
        let mut late = segs[1].clone();
        late.id = "m1-2".into();
        late.start_ms = 8_000;
        late.text = "I'll draft the contract by the end of the month.".into();
        store
            .lock()
            .unwrap()
            .push_segment("m1", late.clone())
            .unwrap();
        segs.push(late);
        let c = Scripted::default();
        let (events, rx) = channel();
        assert_eq!(run(&store, "m1", &c, &cfg(), &events, segs), 0);
        let summary = |s: &Shared, owner: &str| {
            let sum = mirona_core::Summary {
                template: "general".into(),
                overview: String::new(),
                decisions: vec![],
                open_questions: vec![],
                written_by: String::new(),
                quality: String::new(),
            };
            let items = vec![item(1, 0, "me"), item(2, 8_000, owner)];
            s.lock().unwrap().set_summary("m1", sum, items).unwrap();
        };
        summary(&store, "");
        {
            let s = store.lock().unwrap();
            let m = s.meeting("m1").unwrap();
            // Live, "them" owns it, and the due date is the phrase as said.
            assert_eq!(m.segments[2].owners[0].label, "Them");
            assert_eq!(
                m.segments[2].due.as_deref(),
                Some("by the end of the month")
            );
            assert_eq!(m.segments[0].due.as_deref(), Some("by Thursday"));
            assert_eq!(m.segments[1].due, None, "no due date said");
            let items = s.action_items();
            assert_eq!(items[0].due.as_deref(), Some("by Thursday"));
            assert_eq!(
                (items[1].owner.as_str(), items[1].due.as_deref()),
                ("Them", Some("by the end of the month"))
            );
        }
        // Nothing changed speakers yet: nothing to ask.
        assert_eq!(
            refresh_owners(&store, &cfg(), "m1", &c, &events).unwrap(),
            0
        );

        // Diarization splits "them" into Speaker 1.
        store
            .lock()
            .unwrap()
            .set_diarization(
                "m1",
                &["me".into(), "m1-s1".into(), "m1-s1".into()],
                vec![speaker("m1-s1", "Speaker 1")],
                vec![],
            )
            .unwrap();
        // Both actions were owned among Me/Them: both are asked among Me/Speaker 1.
        rx.try_iter().count();
        assert_eq!(
            refresh_owners(&store, &cfg(), "m1", &c, &events).unwrap(),
            2
        );
        assert_eq!(rx.try_iter().count(), 2);
        let s = store.lock().unwrap();
        let m = s.meeting("m1").unwrap();
        assert_eq!(m.segments[2].owners[0].label, "Speaker 1");
        assert_eq!(m.segments[2].tags[0].label, "Action · Speaker 1");
        assert_eq!(s.action_items()[1].owner, "Speaker 1");
        drop(s);

        // An owner the user set stays through a rewritten summary.
        store
            .lock()
            .unwrap()
            .set_action_owner("m1-ai-2", "Nadia")
            .unwrap();
        summary(&store, "Them");
        let items = store.lock().unwrap().action_items();
        assert_eq!(
            (items[1].owner.as_str(), items[1].owner_confirmed),
            ("Nadia", true)
        );
        assert_eq!(items[0].owner, "Me", "the model's owner, from the tag");
    }
}
