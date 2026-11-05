//! The typed yes/no questions asked about every transcript segment.
//!
//! Jev and the local LLM fallback get exactly the same wording. It's the wording measured in
//! the NGR-130 spike (`docs/spikes/ngr-130-jev.md`), so change it only with a rerun of the eval.

pub struct YesNo {
    pub id: &'static str,
    /// Short question shown in the inspector.
    pub question: &'static str,
    /// What the model is asked to judge.
    pub instructions: &'static str,
    pub yes: &'static str,
    pub no: &'static str,
}

pub const SEGMENT_QUESTIONS: [YesNo; 5] = [
    YesNo {
        id: "action",
        question: "Is this an action item?",
        instructions: "Someone in this transcript commits to, or is assigned, a concrete work task to do after the meeting.",
        yes: "A person takes on or is given a specific work task (explicitly or implicitly, e.g. 'I can ping them', 'Sure' to a request).",
        no: "Only status updates, vague intentions with no owner ('we should talk about that'), or social plans.",
    },
    YesNo {
        id: "decision",
        question: "Is this a decision?",
        instructions: "The participants settle a choice in this transcript: something is approved, chosen, rejected, or announced as decided.",
        yes: "A proposal is accepted or rejected, or an option is chosen or announced as final.",
        no: "Options are only discussed, deferred, or left open.",
    },
    YesNo {
        id: "unanswered",
        question: "Is this a question nobody answered?",
        instructions: "A question raised in this transcript is left unanswered or unresolved by the end of it.",
        yes: "Someone asks a question and nobody answers it, or the answer is deferred ('I don't know', 'let me check').",
        no: "No question is asked, or every question asked is answered within the transcript.",
    },
    YesNo {
        id: "smalltalk",
        question: "Is this small talk?",
        instructions: "This transcript is mainly social small talk rather than work discussion.",
        yes: "Most of the exchange is personal chit-chat, greetings, or social plans.",
        no: "Most of the exchange is about work.",
    },
    YesNo {
        id: "sensitive",
        question: "Is this sensitive?",
        instructions: "This transcript contains sensitive information that should not leave the device.",
        yes: "Passwords or credentials, salaries or compensation budgets, health or medical details, HR or performance matters about a named person, or information explicitly marked confidential.",
        no: "Ordinary work content with none of the above.",
    },
];

/// A model answer: `p_yes` is the probability of "yes"; the answer is yes at ≥ 0.5.
#[cfg(any(feature = "jev", feature = "ollama"))]
pub(crate) fn answer(q: &YesNo, p_yes: f32) -> mirona_core::Answer {
    mirona_core::Answer {
        id: q.id.into(),
        question: q.question.into(),
        kind: mirona_core::QuestionKind::YesNo,
        answer: p_yes >= 0.5,
        probability: p_yes,
        source: mirona_core::AnswerSource::Model,
    }
}
