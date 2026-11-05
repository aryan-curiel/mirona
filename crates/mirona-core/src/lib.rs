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
    pub text: String,
    pub redacted: bool,
    pub tags: Vec<Tag>,
    pub answers: Vec<Answer>,
    /// "Who owns it?" distribution, only for action items.
    pub owners: Vec<Choice>,
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
