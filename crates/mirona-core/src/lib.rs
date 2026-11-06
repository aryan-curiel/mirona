//! Mirona domain model and pipeline events (NGR-132).
//!
//! Every type here is serialized with serde and exported to TypeScript with ts-rs:
//! `cargo test -p mirona-core` regenerates `app/src/bindings/`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type Id = String;

/// Avatar tint for a speaker; maps to the design tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Tint {
    Accent,
    Indigo,
    Amber,
}

/// A name the system thinks fits an unknown voice, with where it came from.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NameSuggestion {
    pub name: String,
    pub probability: f32,
    /// Short quote the guess comes from, e.g. "Inés, can you…".
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Speaker {
    pub id: Id,
    /// Display name, or "Speaker 3" while unknown.
    pub name: String,
    pub initial: String,
    pub tint: Tint,
    pub subtitle: String,
    pub voiceprint: Option<Voiceprint>,
    pub suggestion: Option<NameSuggestion>,
}

/// An enrolled voice. The embedding itself never leaves the device, so only metadata is here.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Voiceprint {
    pub meetings: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TagKind {
    Action,
    Decision,
    OpenQuestion,
    SmallTalk,
    Sensitive,
    Review,
}

/// A classification tag shown on a segment.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Tag {
    pub kind: TagKind,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum QuestionKind {
    YesNo,
    Choice,
    Score,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AnswerSource {
    Model,
    User,
}

/// One typed question answered by the classifier, e.g. "Is this an action item?" -> yes, 0.89.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Answer {
    pub id: Id,
    pub question: String,
    pub kind: QuestionKind,
    pub answer: bool,
    pub probability: f32,
    pub source: AnswerSource,
    /// The question's threshold when it was answered (NGR-158): a model "yes" with
    /// `probability` below it needs review instead of being acted on.
    #[serde(default)]
    pub threshold: f32,
}

impl Answer {
    /// A model "yes" below its threshold: shown as "Needs review", never acted on.
    pub fn needs_review(&self) -> bool {
        self.source == AnswerSource::Model && self.answer && self.probability < self.threshold
    }
}

impl TagKind {
    /// The tag a yes to segment question `answer_id` puts on a segment (none for "due").
    pub fn for_answer(answer_id: &str) -> Option<Self> {
        Some(match answer_id {
            "action" => Self::Action,
            "decision" => Self::Decision,
            "unanswered" => Self::OpenQuestion,
            "smalltalk" => Self::SmallTalk,
            "sensitive" => Self::Sensitive,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Action => "Action",
            Self::Decision => "Decision",
            Self::OpenQuestion => "Open question",
            Self::SmallTalk => "Small talk",
            Self::Sensitive => "Sensitive",
            Self::Review => "Needs review",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Choice {
    pub label: String,
    pub probability: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Segment {
    pub id: Id,
    pub speaker_id: Id,
    /// Milliseconds from the start of the call.
    pub start_ms: u32,
    /// Where the speech ends, from the call's start; 0 in transcripts from before it was kept
    /// (unknown: diarization estimates it).
    #[serde(default)]
    pub end_ms: u32,
    pub text: String,
    pub redacted: bool,
    pub tags: Vec<Tag>,
    pub answers: Vec<Answer>,
    /// "Who owns it?" distribution, only for action items.
    pub owners: Vec<Choice>,
    /// The due date as said ("by Thursday", "end of month") when "is a due date said?" is yes
    /// (NGR-156).
    #[serde(default)]
    pub due: Option<String>,
}

impl Segment {
    /// Kept from models and agents (NGR-166): redacted, tagged sensitive, or obviously
    /// sensitive by its words ([`sensitive_words`]) unless the user corrected it to "no".
    pub fn is_sensitive(&self) -> bool {
        if self.redacted || self.tags.iter().any(|t| t.kind == TagKind::Sensitive) {
            return true;
        }
        let user_no = self
            .answers
            .iter()
            .any(|a| a.id == "sensitive" && a.source == AnswerSource::User && !a.answer);
        !user_no && sensitive_words(&self.text)
    }
}

/// A safety net under the classifier (NGR-166/171): lines naming a password, a salary, an
/// account or card number, or health details are sensitive whatever the model says. The Gate
/// 3 audit caught the local model scoring "the salary would be 182,000" at 0.01.
/// ponytail: English keywords plus long digit runs; misses paraphrases, which the classifier
/// still catches. A user "not sensitive" correction removes the tag as usual.
pub fn sensitive_words(text: &str) -> bool {
    const WORDS: &[&str] = &[
        "password",
        "passcode",
        "passphrase",
        "pin code",
        "pin is",
        "pin number",
        "login is",
        "credentials",
        "api key",
        "secret key",
        "private key",
        "recovery code",
        "salary",
        "salaries",
        "paycheck",
        "compensation package",
        "bank account",
        "account number",
        "routing number",
        "sort code",
        "iban",
        "credit card",
        "card number",
        "cvv",
        "social security",
        "ssn",
        "passport number",
        "date of birth",
        "diagnosis",
        "diagnosed",
        "prescription",
        "medical condition",
    ];
    let t = text.to_lowercase();
    let word = |w: &str| {
        t.match_indices(w).any(|(i, _)| {
            let before = t[..i].chars().next_back();
            let after = t[i + w.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    };
    // Card and account numbers: 9+ digits, spaces and dashes allowed in between.
    let mut digits = 0;
    let mut long_number = false;
    for c in t.chars() {
        if c.is_ascii_digit() {
            digits += 1;
            long_number |= digits >= 9;
        } else if c != ' ' && c != '-' {
            digits = 0;
        }
    }
    long_number || WORDS.iter().any(|w| word(w))
}

/// A meeting-level choice question with a confidence threshold (type, client).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChoiceAnswer {
    pub value: String,
    pub probability: f32,
    pub threshold: f32,
    pub options: Vec<String>,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MeetingAnswers {
    pub meeting_type: ChoiceAnswer,
    pub client: ChoiceAnswer,
    /// 0..=5
    pub priority: f32,
    pub follow_up: bool,
    pub follow_up_probability: f32,
    pub follow_up_draft: String,
    /// Client tone, 0..=4: very unhappy, concerned, neutral, positive, delighted.
    #[serde(default)]
    pub tone: f32,
    /// Below it, `follow_up` needs review and no draft is written.
    #[serde(default)]
    pub follow_up_threshold: f32,
}

/// One answer below its threshold, collected per meeting (NGR-158): acted on only once the
/// user confirms it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Review {
    /// The segment it is about; `None` for meeting-level answers and action items.
    pub segment_id: Option<Id>,
    pub at_ms: Option<u32>,
    /// Answer id (`action`, `meeting_type`, `client`, `follow_up`) or `action_item`.
    pub question: String,
    /// What the model thinks: "yes", a choice, or the action item's text.
    pub value: String,
    pub probability: f32,
    pub threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnhancedNote {
    pub kind: TagKind,
    pub text: String,
    pub at_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Note {
    /// The user's notes, one Markdown line each, verbatim.
    pub raw: Vec<String>,
    /// Meeting time (ms) each `raw` line was first written, index for index; `None` for lines
    /// typed outside a recording or in another editor (NGR-144). Shorter than `raw` is fine.
    #[serde(default)]
    pub raw_at_ms: Vec<Option<u32>>,
    pub enhanced: Vec<EnhancedNote>,
    pub enhanced_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Decision {
    pub text: String,
    pub at_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Summary {
    pub template: String,
    pub overview: String,
    pub decisions: Vec<Decision>,
    pub open_questions: Vec<Decision>,
    pub written_by: String,
    pub quality: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ActionItem {
    pub id: Id,
    pub meeting_id: Id,
    pub meeting_title: String,
    pub at_ms: Option<u32>,
    pub text: String,
    pub owner: String,
    pub due: Option<String>,
    pub client: String,
    pub done: bool,
    pub needs_review: bool,
    /// The user set the owner: kept when the summary is rewritten or speakers change.
    #[serde(default)]
    pub owner_confirmed: bool,
}

/// Narrows a search (NGR-167). Every field is optional; text matches are case-insensitive and
/// a part of the name is enough.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct SearchFilters {
    pub client: Option<String>,
    /// First day, `YYYY-MM-DD` (inclusive).
    pub from: Option<String>,
    /// Last day, `YYYY-MM-DD` (inclusive).
    pub to: Option<String>,
    pub speaker: Option<String>,
    pub tag: Option<TagKind>,
}

/// One transcript moment a search found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Moment {
    pub meeting_id: Id,
    pub title: String,
    pub client: String,
    /// "Thu 5 Nov 2026 · 09:30".
    pub date: String,
    pub segment_id: Id,
    pub start_ms: u32,
    pub speaker: String,
    /// The line, with keyword hits in `[brackets]`.
    pub snippet: String,
    /// Tagged sensitive or redacted: show only on request.
    pub sensitive: bool,
    /// Matched by keywords, by meaning (embeddings), or both.
    pub by_keyword: bool,
    pub by_meaning: bool,
}

/// What a search returns: the best moments first, and the meetings they (or the title, client
/// or notes) come from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchResults {
    pub moments: Vec<Moment>,
    pub meetings: Vec<MeetingSummary>,
    /// Whether meaning (embeddings) took part; false when the embedder is off or unreachable.
    pub semantic: bool,
}

/// Row in the sidebar's recent list.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MeetingSummary {
    pub id: Id,
    pub title: String,
    pub client: String,
    pub when: String,
    /// RFC 3339 start time, for sorting and grouping by day.
    pub created: String,
    pub duration_min: u32,
    /// The classifier's meeting type ("" until classified).
    pub meeting_type: String,
    pub open_actions: u32,
    /// Answers below their threshold, waiting for the user (NGR-158).
    pub needs_review: u32,
    pub has_audio: bool,
    pub has_summary: bool,
    pub sample: bool,
}

/// What can be removed from a meeting while keeping its transcript and notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MeetingPart {
    /// The recording; the transcript stays.
    Audio,
    /// Summary and its action items.
    Summary,
    /// Enhanced notes; the user's own notes stay.
    Enhanced,
    /// Tags, typed answers, owners and meeting answers. Sensitive tags and the user's own
    /// corrections stay, so nothing hidden becomes visible.
    Classification,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Meeting {
    pub id: Id,
    pub title: String,
    pub client: String,
    pub when: String,
    pub date_label: String,
    pub duration_min: u32,
    pub language: String,
    pub sample: bool,
    pub speakers: Vec<Speaker>,
    pub segments: Vec<Segment>,
    pub answers: Option<MeetingAnswers>,
    pub notes: Note,
    pub summary: Option<Summary>,
}

/// A human fix to a model answer; saved as a labelled example for eval and fine-tuning.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Correction {
    pub segment_id: Id,
    pub answer_id: Id,
    pub value: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Locality {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderOption {
    pub id: Id,
    pub name: String,
    pub locality: Locality,
}

/// One pipeline stage and the provider picked for it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Stage {
    pub id: Id,
    pub name: String,
    pub subtitle: String,
    pub options: Vec<ProviderOption>,
    pub selected: Id,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentCall {
    pub client: String,
    pub ago: String,
    pub call: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct McpStatus {
    pub running: bool,
    pub transport: String,
    pub tools: Vec<String>,
    pub clients: Vec<String>,
    pub log: Vec<AgentCall>,
}

/// Events emitted by each pipeline stage, in order, while a meeting is recorded and processed.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "stage", rename_all = "camelCase")]
#[ts(export)]
#[allow(clippy::large_enum_variant)] // a few events per second; boxing buys nothing
pub enum PipelineEvent {
    /// Audio level ticks, 0..=1 per bar.
    #[serde(rename_all = "camelCase")]
    Captured {
        meeting_id: Id,
        elapsed_ms: u32,
        levels: Vec<f32>,
    },
    /// A new segment, already diarized and classified (tags arrive as segments do).
    #[serde(rename_all = "camelCase")]
    Transcribed {
        meeting_id: Id,
        segment: Segment,
        speaker: Speaker,
    },
    #[serde(rename_all = "camelCase")]
    Diarized { meeting_id: Id, speakers: u32 },
    #[serde(rename_all = "camelCase")]
    Classified {
        meeting_id: Id,
        segment_id: Id,
        tags: Vec<Tag>,
        #[serde(default)]
        answers: Vec<Answer>,
        #[serde(default)]
        owners: Vec<Choice>,
    },
    /// An audio file being transcribed into `meeting_id` (saved once transcription ends).
    /// `fraction` is 0..=1 of the file, `None` while the length is unknown.
    #[serde(rename_all = "camelCase")]
    Importing {
        meeting_id: Id,
        file: String,
        fraction: Option<f32>,
    },
    /// The summary as it streams in, a few times a second: the overview written so far and
    /// the length of the whole reply. `Summarized` follows with the real one.
    #[serde(rename_all = "camelCase")]
    Summarizing {
        meeting_id: Id,
        overview: String,
        words: u32,
    },
    #[serde(rename_all = "camelCase")]
    Summarized { meeting_id: Id },
    /// Something the user should see: a device change, or a stage that failed. The recording
    /// is never lost because a later stage failed.
    #[serde(rename_all = "camelCase")]
    Notice {
        meeting_id: Id,
        message: String,
        error: bool,
    },
}
