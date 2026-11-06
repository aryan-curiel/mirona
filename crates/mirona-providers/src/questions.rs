//! Typed questions (yes/no, choice, score) asked about one piece of text in one call (NGR-155).
//!
//! Jev and the local LLM fallback get exactly the same wording. The segment and meeting-type /
//! client wording is the one measured in the NGR-130 spike (`docs/spikes/ngr-130-jev.md`), so
//! change it only with a rerun of `mirona eval`.

use mirona_core::{Answer, AnswerSource, Choice, QuestionKind};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    YesNo {
        yes: String,
        no: String,
    },
    /// Option → what it means. The option is what comes back.
    Choice(Vec<(String, String)>),
    /// Levels from low to high (2 to 10); the answer is a level number.
    Score(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub id: String,
    /// Short question shown in the inspector.
    pub question: String,
    /// What the model is asked to judge.
    pub instructions: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// P(yes).
    YesNo(f32),
    /// The chosen option and every option's probability, in question order.
    Choice {
        value: String,
        probabilities: Vec<(String, f32)>,
    },
    /// Expected level, `0..=levels-1`, and each level's probability.
    Score { score: f32, probabilities: Vec<f32> },
}

impl Reply {
    /// How sure the model is of what it answered: P(yes) or P(no) for yes/no, the chosen
    /// option's probability for a choice, the likeliest level's for a score.
    pub fn confidence(&self) -> f32 {
        match self {
            Self::YesNo(p) => p.max(1.0 - p),
            Self::Choice {
                value,
                probabilities,
            } => probabilities
                .iter()
                .find(|(o, _)| o == value)
                .map_or(0.0, |(_, p)| *p),
            Self::Score { probabilities, .. } => probabilities.iter().copied().fold(0.0, f32::max),
        }
    }

    pub fn p_yes(&self) -> Option<f32> {
        match self {
            Self::YesNo(p) => Some(*p),
            _ => None,
        }
    }
}

/// The yes/no questions asked about every segment, as measured in NGR-130.
pub struct YesNo {
    pub id: &'static str,
    pub question: &'static str,
    pub instructions: &'static str,
    pub yes: &'static str,
    pub no: &'static str,
}

impl YesNo {
    pub fn to_question(&self) -> Question {
        Question {
            id: self.id.into(),
            question: self.question.into(),
            instructions: self.instructions.into(),
            kind: Kind::YesNo {
                yes: self.yes.into(),
                no: self.no.into(),
            },
        }
    }
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

/// Asked with the segment questions; not part of the NGR-130 eval (no labels yet).
pub const DUE: YesNo = YesNo {
    id: "due",
    question: "Is a due date said?",
    instructions: "A deadline or due date is given for a task in this transcript.",
    yes: "A date, day, time or deadline is said for when a task must be done ('by Thursday', 'end of day').",
    no: "No deadline is said.",
};

/// The "owner" option for a task nobody in the meeting took.
pub const NO_OWNER: &str = "nobody here";

/// The five segment questions, plus "is a due date said?" and, when the meeting has speakers,
/// "who owns it?" as a choice among them.
pub fn segment_questions(speakers: &[String]) -> Vec<Question> {
    let mut qs: Vec<Question> = SEGMENT_QUESTIONS.iter().map(YesNo::to_question).collect();
    qs.push(DUE.to_question());
    if !speakers.is_empty() {
        let mut options: Vec<(String, String)> =
            speakers.iter().map(|s| (s.clone(), s.clone())).collect();
        options.push((
            NO_OWNER.into(),
            "Nobody in this transcript, or no task at all.".into(),
        ));
        qs.push(Question {
            id: "owner".into(),
            question: "Who owns it?".into(),
            instructions: "Who takes on, or is given, the work task in this transcript?".into(),
            kind: Kind::Choice(options),
        });
    }
    qs
}

/// The "who is this voice?" option for none of the known people.
pub const NO_NAME: &str = "none of them";

/// Who an unknown voice is, from how the others address them (NGR-153): a choice among known
/// people plus [`NO_NAME`], asked about the lines around `speaker`'s turns.
pub fn speaker_name_question(speaker: &str, names: &[String]) -> Question {
    let mut options: Vec<(String, String)> = names.iter().map(|n| (n.clone(), n.clone())).collect();
    options.push((
        NO_NAME.into(),
        format!("Nobody names {speaker}, or they are called something else."),
    ));
    Question {
        id: "speaker_name".into(),
        question: "Who is this unknown speaker?".into(),
        instructions: format!(
            "Who is {speaker}, based on how the others address or name them (\"Thanks, Ana\", \"Ana, can you…\", \"over to you, Ana\")? Pick a name only if the transcript shows it is {speaker}'s."
        ),
        kind: Kind::Choice(options),
    }
}

pub const MEETING_TYPES: [(&str, &str); 5] = [
    (
        "standup",
        "Daily standup: short status updates and blockers.",
    ),
    ("client_call", "A call with an external client or customer."),
    (
        "one_on_one",
        "A private 1:1 between a manager and a report or two colleagues.",
    ),
    ("planning", "Sprint, roadmap, or project planning."),
    (
        "other",
        "Anything else: retro, all-hands, interview, incident review, vendor demo, training, social.",
    ),
];

/// The client option for "none of them".
pub const NO_CLIENT: &str = "none";

pub fn meeting_type_question() -> Question {
    Question {
        id: "meeting_type".into(),
        question: "What kind of meeting is this?".into(),
        instructions: "What kind of meeting is this transcript from?".into(),
        kind: Kind::Choice(
            MEETING_TYPES
                .iter()
                .map(|(o, d)| ((*o).into(), (*d).into()))
                .collect(),
        ),
    }
}

/// Client name → a short description (contact, what they do), as kept in Settings.
pub fn client_question<'a>(clients: impl IntoIterator<Item = (&'a str, &'a str)>) -> Question {
    let mut options: Vec<(String, String)> = clients
        .into_iter()
        .map(|(name, about)| {
            let d = if about.is_empty() {
                name.to_string()
            } else {
                format!("{name} ({about}).")
            };
            (name.to_string(), d)
        })
        .collect();
    options.push((
        NO_CLIENT.into(),
        "Internal discussion not about any of these clients.".into(),
    ));
    Question {
        id: "client".into(),
        question: "Which client is it for?".into(),
        instructions: "Which client is this transcript about?".into(),
        kind: Kind::Choice(options),
    }
}

/// Asked once over the whole meeting after the call (NGR-157). Without clients, no client
/// question.
pub fn meeting_questions<'a>(
    clients: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<Question> {
    let levels = |l: &[&str]| l.iter().map(|s| (*s).to_string()).collect();
    let client = client_question(clients);
    let mut qs = vec![meeting_type_question()];
    if matches!(&client.kind, Kind::Choice(o) if o.len() > 1) {
        qs.push(client);
    }
    qs.extend([
        Question {
            id: "priority".into(),
            question: "How urgent is the follow-up work?".into(),
            instructions: "How urgent is the work that comes out of this meeting?".into(),
            kind: Kind::Score(levels(&[
                "Nothing to do",
                "Low: whenever there is time",
                "Normal",
                "High: this week",
                "Very high: in the next day or two",
                "Critical: drop everything",
            ])),
        },
        Question {
            id: "tone".into(),
            question: "How does the client sound?".into(),
            instructions: "How does the client or the other side sound about the work in this meeting?".into(),
            kind: Kind::Score(levels(&[
                "Very unhappy or angry",
                "Concerned or frustrated",
                "Neutral",
                "Positive",
                "Delighted",
            ])),
        },
        Question {
            id: "follow_up".into(),
            question: "Needs a follow-up email?".into(),
            instructions: "After this meeting, should someone send the other participants a follow-up email recapping what was agreed?".into(),
            kind: Kind::YesNo {
                yes: "Things were agreed, assigned or left open with people who will expect a written recap (clients, partners, cross-team).".into(),
                no: "A routine internal check-in or social chat where a recap email would be noise.".into(),
            },
        },
    ]);
    qs
}

/// The summary quality check: does the summary cover every action item said? (NGR-157)
pub fn coverage_question() -> Question {
    Question {
        id: "coverage".into(),
        question: "Does the summary cover every action item?".into(),
        instructions: "Compare the action items said in the meeting with the action items in the summary. Does the summary cover every one of them?".into(),
        kind: Kind::Score(vec![
            "It misses most of them".into(),
            "It misses some of them".into(),
            "It covers every one of them".into(),
        ]),
    }
}

/// Yes/no replies as `Answer`s (in question order), plus the owner distribution when there is
/// an "owner" choice. Probability is P(yes), like the inspector shows it.
pub fn to_answers(questions: &[Question], replies: &[Reply]) -> (Vec<Answer>, Vec<Choice>) {
    let mut answers = vec![];
    let mut owners = vec![];
    for (q, r) in questions.iter().zip(replies) {
        match r {
            Reply::YesNo(p) => answers.push(Answer {
                id: q.id.clone(),
                question: q.question.clone(),
                kind: QuestionKind::YesNo,
                answer: *p >= 0.5,
                probability: *p,
                source: AnswerSource::Model,
                threshold: 0.0,
            }),
            Reply::Choice { probabilities, .. } if q.id == "owner" => {
                owners = probabilities
                    .iter()
                    .map(|(label, p)| Choice {
                        label: label.clone(),
                        probability: *p,
                    })
                    .collect();
                owners.sort_by(|a, b| b.probability.total_cmp(&a.probability));
            }
            _ => {}
        }
    }
    (answers, owners)
}

/// Options of a choice question, levels of a score: what a reply may name.
pub fn options(q: &Question) -> Vec<String> {
    match &q.kind {
        Kind::YesNo { .. } => vec!["yes".into(), "no".into()],
        Kind::Choice(o) => o.iter().map(|(k, _)| k.clone()).collect(),
        Kind::Score(l) => (0..l.len()).map(|i| i.to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_questions_add_due_and_owner() {
        let qs = segment_questions(&["Me".into(), "Them".into()]);
        let ids: Vec<_> = qs.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "action",
                "decision",
                "unanswered",
                "smalltalk",
                "sensitive",
                "due",
                "owner"
            ]
        );
        assert_eq!(options(&qs[6]), ["Me", "Them", NO_OWNER]);
        assert_eq!(segment_questions(&[]).len(), 6);
    }

    #[test]
    fn meeting_questions_skip_the_client_without_clients() {
        assert!(!meeting_questions([]).iter().any(|q| q.id == "client"));
        let qs = meeting_questions([("Northwind Bank", "contact: Karen")]);
        let client = qs.iter().find(|q| q.id == "client").unwrap();
        assert_eq!(options(client), ["Northwind Bank", NO_CLIENT]);
        assert_eq!(options(&qs[2]).len(), 6, "priority has six levels");
    }

    #[test]
    fn replies_become_answers_and_owners() {
        let qs = segment_questions(&["Me".into(), "Them".into()]);
        let mut replies: Vec<Reply> = (0..6).map(|_| Reply::YesNo(0.2)).collect();
        replies[0] = Reply::YesNo(0.9);
        replies.push(Reply::Choice {
            value: "Them".into(),
            probabilities: vec![
                ("Me".into(), 0.1),
                ("Them".into(), 0.8),
                (NO_OWNER.into(), 0.1),
            ],
        });
        assert!((replies[6].confidence() - 0.8).abs() < 1e-6);
        let (answers, owners) = to_answers(&qs, &replies);
        assert_eq!(answers.len(), 6);
        assert!(answers[0].answer && !answers[1].answer);
        assert_eq!(owners[0].label, "Them");
    }
}
