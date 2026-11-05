//! Sample data for the prototype. All people, meetings and numbers are made up.

use mirona_core::{
    ActionItem, AgentCall, Answer, AnswerSource, Choice, ChoiceAnswer, Decision, EnhancedNote,
    Locality, McpStatus, Meeting, MeetingAnswers, NameSuggestion, Note, ProviderOption,
    QuestionKind, Segment, Speaker, Stage, Summary, Tag, TagKind, Tint, Voiceprint,
};

pub const QUESTIONS: [(&str, &str); 5] = [
    ("action", "Is this an action item?"),
    ("decision", "Is this a decision?"),
    ("unanswered", "Is this a question nobody answered?"),
    ("smalltalk", "Is this small talk?"),
    ("sensitive", "Is this sensitive?"),
];
pub const THRESHOLD: f32 = 0.6;
pub const CLIENTS: [&str; 4] = [
    "Alder Restoration",
    "Juniper Apparel",
    "Corvo",
    "Lumenix",
];

pub fn ms(mmss: &str) -> u32 {
    let (m, s) = mmss.split_once(':').expect("mm:ss");
    (m.parse::<u32>().unwrap() * 60 + s.parse::<u32>().unwrap()) * 1000
}

fn tag(kind: TagKind, label: &str) -> Tag {
    Tag {
        kind,
        label: label.into(),
    }
}

fn answers(p: [f32; 5]) -> Vec<Answer> {
    QUESTIONS
        .iter()
        .zip(p)
        .map(|((id, q), p)| Answer {
            id: (*id).into(),
            question: (*q).into(),
            kind: QuestionKind::YesNo,
            answer: p >= THRESHOLD,
            probability: p,
            source: AnswerSource::Model,
        })
        .collect()
}

pub fn seg(
    id: &str,
    speaker: &str,
    at: &str,
    text: &str,
    p: [f32; 5],
    tags: Vec<Tag>,
    owners: &[(&str, f32)],
) -> Segment {
    Segment {
        id: id.into(),
        speaker_id: speaker.into(),
        start_ms: ms(at),
        text: text.into(),
        redacted: false,
        tags,
        answers: answers(p),
        owners: owners
            .iter()
            .map(|(l, p)| Choice {
                label: (*l).into(),
                probability: *p,
            })
            .collect(),
    }
}

fn speaker(id: &str, name: &str, tint: Tint, subtitle: &str, meetings: Option<u32>) -> Speaker {
    Speaker {
        id: id.into(),
        name: name.into(),
        initial: name
            .chars()
            .last()
            .filter(char::is_ascii_digit)
            .map_or_else(
                || name.chars().next().unwrap().to_string(),
                |d| d.to_string(),
            ),
        tint,
        subtitle: subtitle.into(),
        voiceprint: meetings.map(|meetings| Voiceprint { meetings }),
        suggestion: None,
    }
}

pub fn people() -> Vec<Speaker> {
    let mut s3 = speaker(
        "s3",
        "Speaker 3",
        Tint::Amber,
        "Unknown voice in Claims Desk weekly sync",
        None,
    );
    s3.suggestion = Some(NameSuggestion {
        name: "Inés".into(),
        probability: 0.84,
        evidence: "Inés, can you…".into(),
    });
    vec![
        speaker("aryan", "Aryan", Tint::Accent, "You", Some(12)),
        speaker("nadia", "Nadia", Tint::Indigo, "Alder Restoration", Some(4)),
        s3,
        speaker("greta", "Greta", Tint::Indigo, "Juniper Apparel", Some(3)),
        speaker("andres", "Andrés", Tint::Amber, "Corvo", Some(2)),
        speaker("keiko", "Keiko", Tint::Indigo, "Lumenix", Some(5)),
    ]
}

fn pick(ids: &[&str]) -> Vec<Speaker> {
    people()
        .into_iter()
        .filter(|p| ids.contains(&p.id.as_str()))
        .collect()
}

fn choice(value: &str, p: f32, threshold: f32, options: &[&str]) -> ChoiceAnswer {
    ChoiceAnswer {
        value: value.into(),
        probability: p,
        threshold,
        options: options.iter().map(|s| (*s).into()).collect(),
        confirmed: p >= threshold,
    }
}

fn meeting_answers(
    kind: (&str, f32),
    client: (&str, f32),
    priority: f32,
    follow_up: (bool, f32),
    draft: &str,
) -> MeetingAnswers {
    MeetingAnswers {
        meeting_type: choice(
            kind.0,
            kind.1,
            THRESHOLD,
            &["Client call", "Internal", "Planning", "Review"],
        ),
        client: choice(client.0, client.1, 0.75, &CLIENTS),
        priority,
        follow_up: follow_up.0,
        follow_up_probability: follow_up.1,
        follow_up_draft: draft.into(),
    }
}

fn note(kind: TagKind, text: &str, at: &str) -> EnhancedNote {
    EnhancedNote {
        kind,
        text: text.into(),
        at_ms: ms(at),
    }
}

fn dec(text: &str, at: &str) -> Decision {
    Decision {
        text: text.into(),
        at_ms: ms(at),
    }
}

const NO: [f32; 5] = [0.05, 0.04, 0.02, 0.03, 0.01];

#[allow(clippy::too_many_lines)] // sample data
pub fn meetings() -> Vec<Meeting> {
    use TagKind::{Action, Decision as Dec, OpenQuestion, Review, Sensitive, SmallTalk};
    let mut sensitive = seg(
        "c7",
        "nadia",
        "06:10",
        "The staging login is claims-admin and the password is Alder-Field-26.",
        [0.03, 0.02, 0.01, 0.02, 0.97],
        vec![tag(Sensitive, "Sensitive · redacted")],
        &[],
    );
    sensitive.redacted = true;

    let claims = Meeting {
        id: "claims-desk".into(),
        title: "Claims Desk weekly sync".into(),
        client: "Alder Restoration".into(),
        when: "today".into(),
        date_label: "Thu 5 Nov 2026 · 09:30".into(),
        duration_min: 8,
        language: "English".into(),
        sample: true,
        speakers: pick(&["aryan", "nadia", "s3"]),
        segments: vec![
            seg("c1", "nadia", "00:12", "Morning! Before we start, did anyone catch the match last night?", [0.02, 0.01, 0.05, 0.94, 0.01], vec![tag(SmallTalk, "Small talk")], &[]),
            seg("c2", "aryan", "01:05", "Quick update: the collections workspace now groups claims by insurer, and the export works end to end.", [0.10, 0.15, 0.02, 0.03, 0.01], vec![], &[]),
            seg("c3", "s3", "02:40", "Great. Can we ship the insurer grouping to the field team on Monday?", [0.20, 0.30, 0.35, 0.02, 0.01], vec![], &[]),
            seg("c4", "nadia", "03:02", "Yes, let's ship it Monday. Inés, can you let the field team know?", [0.45, 0.91, 0.02, 0.01, 0.01], vec![tag(Dec, "Decision")], &[]),
            seg("c5", "s3", "03:20", "Sure, I'll send the announcement by Friday.", [0.89, 0.12, 0.02, 0.04, 0.01], vec![tag(Action, "Action · Speaker 3 · Fri")], &[("Speaker 3", 0.86), ("Nadia", 0.09), ("Aryan", 0.05)]),
            seg("c6", "aryan", "04:45", "I'll add the overdue filter and send a preview build by Thursday.", [0.93, 0.08, 0.01, 0.02, 0.01], vec![tag(Action, "Action · Aryan · Thu")], &[("Aryan", 0.94), ("Nadia", 0.04), ("Speaker 3", 0.02)]),
            sensitive,
            seg("c8", "s3", "07:30", "What happens to claims that were already paid before the migration?", [0.10, 0.04, 0.81, 0.01, 0.01], vec![tag(OpenQuestion, "Open question")], &[]),
            seg("c9", "aryan", "08:05", "Let me think about whether we migrate those or archive them.", [0.48, 0.10, 0.20, 0.02, 0.01], vec![tag(Review, "Needs review · action? 0.48")], &[("Aryan", 0.71), ("Nadia", 0.18), ("Speaker 3", 0.11)]),
        ],
        answers: Some(meeting_answers(("Client call", 0.93), ("Alder Restoration", 0.71), 3.6, (true, 0.86),
            "Hi Nadia,\n\nThanks for today. Confirming: the insurer grouping ships to the field team on Monday, Speaker 3 announces it by Friday, and I'll send a preview build with the overdue filter by Thursday.\n\nStill open: what we do with claims paid before the migration.\n\nAryan")),
        notes: Note {
            raw: vec!["insurer grouping → monday?".into(), "overdue filter".into(), "paid claims before migration??".into()],
            enhanced: vec![
                note(Dec, "Insurer grouping ships to the field team on Monday.", "03:02"),
                note(Action, "Speaker 3 announces it to the field team by Friday.", "03:20"),
                note(Action, "Aryan adds the overdue filter and sends a preview build by Thursday.", "04:45"),
                note(OpenQuestion, "Claims paid before the migration: migrate or archive? Aryan to decide.", "07:30"),
            ],
            enhanced_by: "Ollama (local)".into(),
        },
        summary: Some(Summary {
            template: "Client call".into(),
            overview: "Weekly check-in on the Claims Desk collections workspace. Insurer grouping is ready and ships to the field team on Monday. One migration question is still open.".into(),
            decisions: vec![dec("Ship insurer grouping to the field team on Monday", "03:02")],
            open_questions: vec![dec("What happens to claims already paid before the migration?", "07:30")],
            written_by: "Ollama (local)".into(),
            quality: "Quality check: the summary covers 2 of 2 confirmed action items (score 1.0)".into(),
        }),
    };

    let sprint = Meeting {
        id: "sprint-planning".into(),
        title: "Sprint planning".into(),
        client: "Juniper Apparel".into(),
        when: "Tue".into(),
        date_label: "Tue 3 Nov 2026 · 14:00".into(),
        duration_min: 24,
        language: "English".into(),
        sample: true,
        speakers: pick(&["aryan", "greta"]),
        segments: vec![
            seg("s1", "greta", "00:40", "Let's lock the sprint goal first: product tagging for the spring catalogue.", [0.20, 0.78, 0.03, 0.02, 0.01], vec![tag(Dec, "Decision")], &[]),
            seg("s2", "aryan", "03:15", "Tagging is about five days if we reuse the attribute model.", NO, vec![], &[]),
            seg("s3", "greta", "05:02", "Can you share the tagging estimate with the team by Monday?", [0.71, 0.10, 0.05, 0.01, 0.01], vec![], &[]),
            seg("s4", "aryan", "05:10", "Yes, I'll share it Monday morning.", [0.88, 0.05, 0.01, 0.02, 0.01], vec![tag(Action, "Action · Aryan · Mon")], &[("Aryan", 0.91), ("Greta", 0.09)]),
            seg("s5", "greta", "11:30", "Do we still need the size guide in this sprint?", [0.12, 0.05, 0.74, 0.01, 0.01], vec![tag(OpenQuestion, "Open question")], &[]),
        ],
        answers: Some(meeting_answers(("Planning", 0.88), ("Juniper Apparel", 0.95), 2.8, (false, 0.22), "")),
        notes: Note { raw: vec!["tagging estimate".into(), "size guide?".into()], enhanced: vec![
            note(Dec, "Sprint goal: product tagging for the spring catalogue.", "00:40"),
            note(Action, "Aryan shares the tagging estimate by Monday.", "05:10"),
            note(OpenQuestion, "Is the size guide still in this sprint?", "11:30"),
        ], enhanced_by: "Ollama (local)".into() },
        summary: Some(Summary {
            template: "Planning".into(),
            overview: "Sprint goal set to product tagging for the spring catalogue. Estimate follows on Monday.".into(),
            decisions: vec![dec("Sprint goal: product tagging for the spring catalogue", "00:40")],
            open_questions: vec![dec("Do we still need the size guide in this sprint?", "11:30")],
            written_by: "Ollama (local)".into(),
            quality: "Quality check: the summary covers 1 of 1 confirmed action items (score 1.0)".into(),
        }),
    };

    let corvo = Meeting {
        id: "corvo-review".into(),
        title: "Corvo Live product review".into(),
        client: "Corvo".into(),
        when: "Mon".into(),
        date_label: "Mon 2 Nov 2026 · 16:30".into(),
        duration_min: 31,
        language: "English".into(),
        sample: true,
        speakers: pick(&["aryan", "andres"]),
        segments: vec![
            seg("t1", "andres", "00:20", "The live scoring view feels much faster since last week.", NO, vec![], &[]),
            seg("t2", "andres", "06:45", "Onboarding copy in the mobile app still reads like a draft.", [0.35, 0.05, 0.05, 0.02, 0.01], vec![], &[]),
            seg("t3", "aryan", "07:02", "I'll review the onboarding copy and send suggestions Wednesday.", [0.90, 0.06, 0.01, 0.02, 0.01], vec![tag(Action, "Action · Aryan · Wed")], &[("Aryan", 0.93), ("Andrés", 0.07)]),
        ],
        answers: Some(meeting_answers(("Review", 0.81), ("Corvo", 0.97), 2.1, (true, 0.64), "Hi Andrés,\n\nThanks for the review. I'll send onboarding copy suggestions on Wednesday.\n\nAryan")),
        notes: Note { raw: vec!["onboarding copy".into()], enhanced: vec![note(Action, "Aryan reviews the mobile onboarding copy by Wednesday.", "07:02")], enhanced_by: "Ollama (local)".into() },
        summary: Some(Summary {
            template: "Review".into(),
            overview: "Product review of Corvo Live. Scoring view is faster; mobile onboarding copy needs a pass.".into(),
            decisions: vec![],
            open_questions: vec![],
            written_by: "Ollama (local)".into(),
            quality: "Quality check: the summary covers 1 of 1 confirmed action items (score 1.0)".into(),
        }),
    };

    let roadmap = Meeting {
        id: "roadmap-checkin".into(),
        title: "Roadmap check-in".into(),
        client: "Lumenix".into(),
        when: "last week".into(),
        date_label: "Wed 28 Oct 2026 · 11:00".into(),
        duration_min: 19,
        language: "English".into(),
        sample: true,
        speakers: pick(&["aryan", "keiko"]),
        segments: vec![
            seg(
                "r1",
                "keiko",
                "00:30",
                "We agreed the agent console moves to Q1.",
                [0.10, 0.84, 0.02, 0.01, 0.01],
                vec![tag(Dec, "Decision")],
                &[],
            ),
            seg(
                "r2",
                "aryan",
                "02:10",
                "That gives us room for the evaluation dashboard first.",
                NO,
                vec![],
                &[],
            ),
        ],
        answers: Some(meeting_answers(
            ("Planning", 0.77),
            ("Lumenix", 0.92),
            3.0,
            (false, 0.31),
            "",
        )),
        notes: Note {
            raw: vec![],
            enhanced: vec![note(Dec, "Agent console moves to Q1.", "00:30")],
            enhanced_by: "Ollama (local)".into(),
        },
        summary: Some(Summary {
            template: "Planning".into(),
            overview:
                "Roadmap check-in. Agent console moves to Q1; evaluation dashboard goes first."
                    .into(),
            decisions: vec![dec("Agent console moves to Q1", "00:30")],
            open_questions: vec![],
            written_by: "Ollama (local)".into(),
            quality: "Quality check: no action items to cover".into(),
        }),
    };

    vec![claims, sprint, corvo, roadmap]
}

pub fn action_items() -> Vec<ActionItem> {
    let item = |id: &str,
                m: &str,
                title: &str,
                at: Option<&str>,
                text: &str,
                owner: &str,
                due: Option<&str>,
                client: &str,
                review: bool| ActionItem {
        id: id.into(),
        meeting_id: m.into(),
        meeting_title: title.into(),
        at_ms: at.map(ms),
        text: text.into(),
        owner: owner.into(),
        due: due.map(Into::into),
        client: client.into(),
        done: false,
        needs_review: review,
    };
    vec![
        item(
            "a1",
            "claims-desk",
            "Claims Desk weekly sync",
            Some("03:20"),
            "Announce the insurer grouping to the field team",
            "Speaker 3",
            Some("Fri"),
            "Alder Restoration",
            false,
        ),
        item(
            "a2",
            "claims-desk",
            "Claims Desk weekly sync",
            Some("04:45"),
            "Add the overdue filter and send a preview build",
            "Aryan",
            Some("Thu"),
            "Alder Restoration",
            false,
        ),
        item(
            "a3",
            "claims-desk",
            "Claims Desk weekly sync",
            Some("08:05"),
            "Decide whether to migrate or archive claims paid before the migration",
            "Aryan",
            None,
            "Alder Restoration",
            true,
        ),
        item(
            "a4",
            "sprint-planning",
            "Sprint planning",
            Some("05:10"),
            "Share the tagging estimate",
            "Aryan",
            Some("Mon"),
            "Juniper Apparel",
            false,
        ),
        item(
            "a5",
            "corvo-review",
            "Corvo Live product review",
            Some("07:02"),
            "Review onboarding copy for the mobile app",
            "Aryan",
            Some("Wed"),
            "Corvo",
            false,
        ),
    ]
}

fn opt(id: &str, name: &str, locality: Locality) -> ProviderOption {
    ProviderOption {
        id: id.into(),
        name: name.into(),
        locality,
    }
}

pub fn stages() -> Vec<Stage> {
    use Locality::{Cloud, Local};
    let stage = |id: &str, name: &str, sub: &str, options: Vec<ProviderOption>| Stage {
        id: id.into(),
        name: name.into(),
        subtitle: sub.into(),
        selected: options[0].id.clone(),
        options,
    };
    vec![
        stage(
            "stt",
            "Speech-to-text",
            "Live transcript",
            vec![
                opt("parakeet", "Parakeet-TDT-0.6B", Local),
                opt("whisper", "Whisper (whisper-rs)", Local),
                opt("cloud-stt", "Cloud STT API", Cloud),
            ],
        ),
        stage(
            "diarization",
            "Speaker detection",
            "Who said what",
            vec![
                opt("speakrs", "speakrs", Local),
                opt("pyannote", "pyannote-rs", Local),
            ],
        ),
        stage(
            "classification",
            "Classification",
            "Typed questions: choice, yes/no, score",
            vec![
                opt("jev", "Jev (hosted)", Cloud),
                opt("local-llm", "Local LLM (JSON schema)", Local),
            ],
        ),
        stage(
            "llm",
            "Summaries and notes",
            "The only stage that writes prose",
            vec![
                opt("ollama", "Ollama (local)", Local),
                opt("anthropic", "Anthropic Claude", Cloud),
                opt("openai", "OpenAI-compatible", Cloud),
            ],
        ),
        stage(
            "embeddings",
            "Search embeddings",
            "Hybrid search for agents",
            vec![
                opt("local-embed", "Local embeddings", Local),
                opt("cloud-embed", "Embeddings API", Cloud),
            ],
        ),
    ]
}

pub fn mcp() -> McpStatus {
    let call = |client: &str, ago: &str, call: &str, result: &str| AgentCall {
        client: client.into(),
        ago: ago.into(),
        call: call.into(),
        result: result.into(),
    };
    McpStatus {
        running: true,
        transport: "stdio · http://127.0.0.1:[PORT]/mcp".into(),
        tools: [
            "search_meetings",
            "get_meeting",
            "get_transcript",
            "list_action_items",
            "ask_meeting",
        ]
        .map(Into::into)
        .to_vec(),
        clients: vec!["Claude Code".into(), "Claude Desktop".into()],
        log: vec![
            call(
                "Claude Code",
                "2 min ago",
                "search_meetings(\"insurer grouping\")",
                "1 meeting · Claims Desk weekly sync",
            ),
            call(
                "Claude Desktop",
                "5 min ago",
                "ask_meeting(\"what did we decide for Monday?\")",
                "Routed by Jev to: decisions lookup · no LLM call needed",
            ),
            call(
                "Claude Code",
                "9 min ago",
                "list_action_items(client=\"Alder Restoration\")",
                "3 items · 1 needs review",
            ),
        ],
    }
}

/// (speaker, text, probabilities, tag).
pub type ScriptLine = (
    &'static str,
    &'static str,
    [f32; 5],
    Option<(TagKind, &'static str)>,
);

/// Lines the fake recorder "hears", in order: (speaker, text, action probability, tag).
pub fn live_script() -> Vec<ScriptLine> {
    vec![
        (
            "keiko",
            "Thanks for jumping on. Quick follow-up on the evaluation dashboard.",
            NO,
            None,
        ),
        (
            "aryan",
            "Sure. The first version reads straight from the corrections dataset.",
            NO,
            None,
        ),
        (
            "keiko",
            "Then let's demo it to the board on the twentieth.",
            [0.30, 0.82, 0.03, 0.01, 0.01],
            Some((TagKind::Decision, "Decision")),
        ),
        (
            "aryan",
            "I'll prepare a short demo script by Tuesday.",
            [0.91, 0.07, 0.01, 0.02, 0.01],
            Some((TagKind::Action, "Action · Aryan · Tue")),
        ),
        (
            "keiko",
            "Who owns the data export for legal?",
            [0.15, 0.04, 0.77, 0.01, 0.01],
            Some((TagKind::OpenQuestion, "Open question")),
        ),
    ]
}
