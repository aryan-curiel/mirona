use super::*;
use mirona_core::Locality;
use mirona_providers::{Completion, Provider, Usage};
use serde_json::json;
use std::sync::Mutex;

/// Every provider here (fake and the blocking-HTTP real ones) is ready on the first poll.
fn block_on<T>(fut: impl Future<Output = T>) -> T {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(fut).poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => panic!("provider was not ready"),
    }
}

/// A fake LLM that replies from a script and records every prompt.
struct Scripted {
    replies: Mutex<Vec<String>>,
    prompts: Mutex<Vec<(String, String, bool)>>,
}

impl Scripted {
    fn new(replies: &[&str]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().rev().map(|r| (*r).to_string()).collect()),
            prompts: Mutex::new(vec![]),
        }
    }
    fn prompts(&self) -> Vec<(String, String, bool)> {
        self.prompts.lock().unwrap().clone()
    }
}

impl Provider for Scripted {
    fn id(&self) -> &'static str {
        "scripted"
    }
    fn locality(&self) -> Locality {
        Locality::Local
    }
}

impl LlmProvider for Scripted {
    fn complete(
        &self,
        prompt: &Prompt<'_>,
        on_text: &mut (dyn FnMut(&str) + Send),
    ) -> impl Future<Output = mirona_providers::Result<Completion>> + Send {
        self.prompts
            .lock()
            .unwrap()
            .push((prompt.system.into(), prompt.user.into(), prompt.json));
        let text = self.replies.lock().unwrap().pop().expect("script ran out");
        on_text(&text);
        std::future::ready(Ok(Completion {
            text,
            usage: Usage::default(),
        }))
    }
}

/// Built from JSON so fields added to core types with `#[serde(default)]` don't break it.
fn meeting(lines: &[(&str, &str, &str)], notes: &[&str]) -> Meeting {
    let segments: Vec<Value> = lines
        .iter()
        .enumerate()
        .map(|(i, (who, at, text))| {
            json!({ "id": format!("s{i}"), "speakerId": who, "startMs": parse_clock(at).unwrap(),
                    "text": text, "redacted": false, "tags": [], "answers": [], "owners": [] })
        })
        .collect();
    let speaker = |id: &str, name: &str| {
        json!({ "id": id, "name": name, "initial": &name[..1], "tint": "accent", "subtitle": "",
                "voiceprint": null, "suggestion": null })
    };
    serde_json::from_value(json!({
        "id": "m1", "title": "Claims Desk weekly sync", "client": "Alder Restoration",
        "when": "today", "dateLabel": "", "durationMin": 9, "language": "English", "sample": false,
        "speakers": [speaker("nadia", "Nadia"), speaker("aryan", "Aryan"), speaker("ines", "Inés")],
        "segments": segments, "answers": null,
        "notes": { "raw": notes, "enhanced": [], "enhancedBy": "" }, "summary": null,
    }))
    .unwrap()
}

fn claims() -> Meeting {
    meeting(
        &[
            (
                "nadia",
                "00:12",
                "Morning! Did anyone catch the match last night?",
            ),
            (
                "aryan",
                "01:05",
                "The collections workspace now groups claims by insurer, and the export works end to end.",
            ),
            (
                "ines",
                "02:40",
                "Great. Can we ship the insurer grouping to the field team on Monday?",
            ),
            (
                "nadia",
                "03:02",
                "Yes, let's ship it Monday. Inés, can you let the field team know?",
            ),
            (
                "ines",
                "03:20",
                "Sure, I'll send the announcement by Friday.",
            ),
            (
                "aryan",
                "04:45",
                "I'll add the overdue filter and send a preview build by Thursday.",
            ),
            ("nadia", "06:10", "The staging password is Alder-Field-26."),
            (
                "ines",
                "07:30",
                "What happens to claims that were already paid before the migration?",
            ),
            (
                "aryan",
                "08:05",
                "Let me think about whether we migrate those or archive them.",
            ),
        ],
        &[
            "insurer grouping → monday?",
            "overdue filter",
            "  ",
            "paid claims before migration??",
        ],
    )
}

fn client_call() -> Template {
    Template::default_for("client-call").unwrap()
}

const GOOD: &str = r#"Sure! Here is the summary:
```json
{"overview": "Insurer grouping ships Monday.",
 "decisions": [{"text": "Ship insurer grouping Monday", "segment": 3}],
 "action_items": [
   {"text": "Announce to the field team", "owner": "Inés", "due": "Friday", "segment": "4"},
   {"text": "Add the overdue filter", "owner": "Aryan", "due": null, "segment": "04:45"},
   {"text": "Decide on paid claims", "owner": "", "due": "none", "segment": 99}],
 "open_questions": [{"text": "Paid claims before the migration?", "segment": "[7 07:30]"}]}
```
Let me know if you need more."#;

#[test]
fn prompt_has_template_numbered_lines_and_hides_redacted_text() {
    let mut m = claims();
    m.segments[6].redacted = true;
    let llm = Scripted::new(&[GOOD]);
    block_on(summarize(&m, &client_call(), &llm)).unwrap();
    let (system, user, json) = &llm.prompts()[0];
    assert!(json);
    assert!(system.contains("\"action_items\"") && system.contains("# Client call"));
    assert!(user.starts_with("Meeting: Claims Desk weekly sync (client: Alder Restoration)"));
    assert!(
        user.contains("[3 03:02] Nadia: Yes, let's ship it Monday."),
        "{user}"
    );
    assert!(user.contains("[6 06:10] Nadia: [redacted]"));
    assert!(!user.contains("Alder-Field-26"));
}

#[test]
fn parses_a_chatty_fenced_reply_and_maps_citations() {
    let m = claims();
    let llm = Scripted::new(&[GOOD]);
    let (s, actions) = block_on(summarize(&m, &client_call(), &llm)).unwrap();
    assert_eq!(s.template, "Client call");
    assert_eq!(s.written_by, "scripted");
    assert_eq!(s.overview, "Insurer grouping ships Monday.");
    assert_eq!(s.decisions[0].at_ms, 182_000);
    assert_eq!(s.open_questions[0].at_ms, 450_000);
    assert_eq!(s.quality, "");

    assert_eq!(actions.len(), 3);
    let a = &actions[0];
    assert_eq!(
        (a.owner.as_str(), a.due.as_deref(), a.at_ms),
        ("Inés", Some("Friday"), Some(200_000))
    );
    assert!(!a.needs_review && !a.done);
    assert_eq!(
        (a.id.as_str(), a.meeting_id.as_str(), a.client.as_str()),
        ("m1-ai-1", "m1", "Alder Restoration")
    );
    // A clock time maps to the line spoken then; "null"/"none" dues are dropped.
    assert_eq!(
        (actions[1].at_ms, actions[1].due.as_deref()),
        (Some(285_000), None)
    );
    // Line 99 doesn't exist and there's no owner: kept, but flagged for review.
    assert_eq!((actions[2].at_ms, actions[2].due.as_deref()), (None, None));
    assert!(actions[2].needs_review);
}

#[test]
fn bad_citations_drop_decisions_and_questions() {
    let reply = r#"{"overview": "o",
        "decisions": [{"text": "real", "segment": 3}, {"text": "made up", "segment": 42},
                      {"text": "too late", "segment": "59:00"}, {"text": "nonsense", "segment": "soon"}],
        "action_items": [], "open_questions": ["no citation at all"]}"#;
    let (s, actions) = block_on(summarize(
        &claims(),
        &client_call(),
        &Scripted::new(&[reply]),
    ))
    .unwrap();
    assert_eq!(s.decisions.len(), 1);
    assert!(s.open_questions.is_empty() && actions.is_empty());
    assert_eq!(
        s.quality,
        "4 item(s) left out: no valid transcript reference"
    );
}

#[test]
fn garbage_is_a_clear_error() {
    for reply in [
        "I'm sorry, I can't help with that.",
        "{\"overview\": \"cut off",
        "{\"summary\": \"wrong shape\"}",
        "",
    ] {
        let e = block_on(summarize(
            &claims(),
            &client_call(),
            &Scripted::new(&[reply]),
        ))
        .unwrap_err();
        assert!(
            e.contains("JSON") || e.contains("overview"),
            "{reply:?} → {e}"
        );
    }
    let empty = meeting(&[], &[]);
    assert!(block_on(summarize(&empty, &client_call(), &Scripted::new(&[]))).is_err());
}

#[test]
fn think_blocks_are_skipped() {
    let reply = "<think>maybe {\"overview\": \"wrong\"}</think>{\"overview\": \"right\"}";
    let (s, _) = block_on(summarize(
        &claims(),
        &client_call(),
        &Scripted::new(&[reply]),
    ))
    .unwrap();
    assert_eq!(s.overview, "right");
}

#[test]
fn long_meetings_are_chunked_then_merged() {
    let filler = "word ".repeat(1500); // ~7.5k chars per line, so 9 lines need 3 chunks
    let lines: Vec<(&str, String, &str)> = (0..9)
        .map(|i| ("nadia", format!("{:02}:00", i * 5), filler.as_str()))
        .collect();
    let lines: Vec<(&str, &str, &str)> =
        lines.iter().map(|(a, b, c)| (*a, b.as_str(), *c)).collect();
    let m = meeting(&lines, &[]);
    let part = |n: u32| {
        format!(
            r#"{{"overview": "part {n}", "decisions": [{{"text": "d{n}", "segment": {}}}]}}"#,
            n * 3
        )
    };
    let parts = [part(0), part(1), part(2)];
    let llm = Scripted::new(&[&parts[0], &parts[1], &parts[2], "The whole meeting."]);
    let (s, _) = block_on(summarize(&m, &client_call(), &llm)).unwrap();
    let prompts = llm.prompts();
    assert_eq!(prompts.len(), 4);
    assert!(prompts[1].1.contains("part 2 of 3") && prompts[1].1.contains("[3 15:00]"));
    assert!(!prompts[3].2, "the merge asks for prose");
    assert_eq!(prompts[3].1, "part 0\n\npart 1\n\npart 2");
    assert_eq!(s.overview, "The whole meeting.");
    // Citations are global line numbers, whichever chunk they came from.
    let at: Vec<u32> = s.decisions.iter().map(|d| d.at_ms).collect();
    assert_eq!(at, [0, 900_000, 1_800_000]);
}

#[test]
fn enhance_sends_the_raw_notes_and_links_each_line() {
    let m = claims();
    let reply = r#"{"notes": [
        {"kind": "action", "text": "Aryan adds the overdue filter and sends a preview build by Thursday.", "segment": 5},
        {"kind": "decision", "text": "Insurer grouping → ships Monday.", "segment": 3},
        {"kind": "open_question", "text": "Paid claims before migration: migrate or archive?", "segment": "07:30"},
        {"kind": "note", "text": "Export works end to end.", "segment": 1},
        {"kind": "action", "text": "Invented", "segment": 77}]}"#;
    let llm = Scripted::new(&[reply]);
    let notes = block_on(enhance_notes(&m, &llm)).unwrap();
    let (system, user, json) = &llm.prompts()[0];
    assert!(*json && system.contains("Keep the person's own wording"));
    assert!(user.contains("My notes:\n- insurer grouping → monday?\n- overdue filter\n- paid claims before migration??\n\nTranscript:"), "{user}");
    let got: Vec<(TagKind, u32)> = notes.iter().map(|n| (n.kind, n.at_ms)).collect();
    assert_eq!(
        got,
        [
            (TagKind::Review, 65_000),
            (TagKind::Decision, 182_000),
            (TagKind::Action, 285_000),
            (TagKind::OpenQuestion, 450_000)
        ]
    );
    assert_eq!(m.notes.raw.len(), 4, "raw notes are never touched");
}

#[test]
fn enhance_without_notes_and_on_garbage() {
    let mut m = claims();
    m.notes.raw.clear();
    let llm = Scripted::new(&[r#"{"notes": []}"#]);
    assert!(block_on(enhance_notes(&m, &llm)).unwrap().is_empty());
    assert!(llm.prompts()[0].1.contains("My notes:\n(none"));
    let e = block_on(enhance_notes(&m, &Scripted::new(&[r#"{"overview": "x"}"#]))).unwrap_err();
    assert!(e.contains("no notes list"), "{e}");
}

#[test]
fn citations_accept_ids_indexes_and_clock_times() {
    let m = claims();
    let c = |v: Value| cite(&v, &m);
    assert_eq!(c(json!(3)), Some(182_000));
    assert_eq!(c(json!("#3")), Some(182_000));
    assert_eq!(c(json!("s4")), Some(200_000));
    assert_eq!(c(json!([5, 6])), Some(285_000));
    assert_eq!(c(json!("03:10")), Some(182_000));
    assert_eq!(c(json!("0:03:10")), Some(182_000));
    assert_eq!(c(json!("00:00")), None, "before the first line");
    assert_eq!(c(json!("03:75")), None);
    assert_eq!(c(json!(-1)), None);
    assert_eq!(c(json!(9)), None);
    assert_eq!(clock(3_725_000), "1:02:05");
}

#[test]
fn defaults_are_written_once_and_user_edits_win() {
    let dir = std::env::temp_dir().join(format!("mirona-templates-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let all = templates(&dir).unwrap();
    let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["client-call", "one-on-one", "planning", "standup"]);
    let names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Client call", "1:1", "Planning", "Standup"]);
    for t in &all {
        for section in [
            "## Overview",
            "## Decisions",
            "## Action items",
            "## Open questions",
        ] {
            assert!(t.body.contains(section), "{} lacks {section}", t.id);
        }
    }

    std::fs::write(dir.join("standup.md"), "# Daily\nJust blockers.").unwrap();
    std::fs::write(dir.join("retro.md"), "# Retro\nWhat went well.").unwrap();
    let standup = template(&dir, "standup").unwrap();
    assert_eq!(
        (standup.name.as_str(), standup.body.as_str()),
        ("Daily", "# Daily\nJust blockers.")
    );
    assert_eq!(template(&dir, "retro").unwrap().name, "Retro");
    assert_eq!(templates(&dir).unwrap().len(), 5);
    assert!(template(&dir, "../etc/passwd").is_err());
    assert!(template(&dir, "missing").is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

/// `ollama serve` + `ollama pull qwen3:4b`, then
/// `cargo test -p mirona-summary -- --ignored --nocapture`
#[test]
#[ignore = "needs a running Ollama with qwen3:4b"]
fn live_summary_and_notes_with_qwen3() {
    let llm = mirona_providers::ollama::Ollama::default();
    let mut m = claims();
    m.segments[6].redacted = true; // the staging password, as the classifier marks it
    let (s, actions) = block_on(summarize(&m, &client_call(), &llm)).unwrap();
    println!("{s:#?}\n{actions:#?}");
    let starts: Vec<u32> = m.segments.iter().map(|s| s.start_ms).collect();
    assert_ne!(s.overview, "");
    assert!(!s.decisions.is_empty(), "the Monday ship decision");
    assert!(
        s.decisions
            .iter()
            .chain(&s.open_questions)
            .all(|d| starts.contains(&d.at_ms))
    );
    assert!(
        actions.len() >= 2,
        "Inés's announcement and Aryan's filter"
    );
    assert!(
        actions
            .iter()
            .filter_map(|a| a.at_ms)
            .all(|at| starts.contains(&at))
    );
    assert!(
        actions.iter().any(|a| a.owner.contains("Aryan")),
        "{actions:?}"
    );

    let notes = block_on(enhance_notes(&m, &llm)).unwrap();
    println!("{notes:#?}");
    assert!(notes.len() >= 3, "{notes:?}");
    assert!(notes.iter().all(|n| starts.contains(&n.at_ms)));
}

#[test]
fn enhance_tells_the_model_when_each_note_was_written() {
    let mut m = claims();
    // First line typed at 03:05 during the call, second one afterwards (no time).
    m.notes.raw_at_ms = vec![Some(185_000), None];
    let llm = Scripted::new(&[r#"{"notes": []}"#]);
    block_on(enhance_notes(&m, &llm)).unwrap();
    let (system, user, _) = &llm.prompts()[0];
    assert!(system.contains("(at m:ss)"), "{system}");
    assert!(
        user.contains("- (at 03:05) insurer grouping → monday?\n- overdue filter\n"),
        "{user}"
    );
}

/// Streams a fixed reply a few characters at a time and reports token use, like Ollama does.
struct Streaming(&'static str);

impl Provider for Streaming {
    fn id(&self) -> &'static str {
        "streaming"
    }
    fn locality(&self) -> Locality {
        Locality::Local
    }
}

impl LlmProvider for Streaming {
    fn complete(
        &self,
        _prompt: &Prompt<'_>,
        on_text: &mut (dyn FnMut(&str) + Send),
    ) -> impl Future<Output = mirona_providers::Result<Completion>> + Send {
        let chars: Vec<char> = self.0.chars().collect();
        for piece in chars.chunks(7) {
            on_text(&piece.iter().collect::<String>());
        }
        std::future::ready(Ok(Completion {
            text: self.0.into(),
            usage: Usage {
                input_tokens: 812,
                output_tokens: 240,
            },
        }))
    }
}

#[test]
fn streaming_hands_over_chunks_in_order_and_still_parses() {
    let mut pieces = vec![];
    let mut previews: Vec<String> = vec![];
    let mut so_far = String::new();
    let (s, actions) = block_on(summarize_streaming(
        &claims(),
        &client_call(),
        &Streaming(GOOD),
        &mut |t| {
            pieces.push(t.to_string());
            so_far.push_str(t);
            previews.push(partial_overview(&so_far));
        },
    ))
    .unwrap();
    assert!(pieces.len() > 10);
    assert_eq!(pieces.concat(), GOOD, "every chunk, in order");
    // The preview only ever grows, and ends as the real overview.
    assert!(previews.windows(2).all(|w| w[1].starts_with(&w[0])));
    assert_eq!(previews.last().unwrap(), "Insurer grouping ships Monday.");
    assert_eq!(s.overview, "Insurer grouping ships Monday.");
    assert_eq!(actions.len(), 3);
    assert_eq!(s.quality, "812 tokens in, 240 out");
}

#[test]
fn partial_overview_reads_an_unfinished_json_string() {
    assert_eq!(partial_overview(""), "");
    assert_eq!(partial_overview(r#"{"overv"#), "");
    assert_eq!(partial_overview(r#"{"overview": "#), "");
    assert_eq!(
        partial_overview(r#"{"overview": "Nadia said \"ship"#),
        "Nadia said \"ship"
    );
    assert_eq!(
        partial_overview(r#"{"overview" : "a\nb é", "decisions": []}"#),
        "a\nb é"
    );
    // The latest part wins when a long meeting is summarized in parts.
    assert_eq!(
        partial_overview(r#"{"overview": "one"} {"overview": "tw"#),
        "tw"
    );
}

fn tag(m: &mut Meeting, i: usize, kind: TagKind) {
    m.segments[i].tags.push(mirona_core::Tag {
        kind,
        label: kind.label().into(),
    });
    m.segments[i].answers =
        vec![serde_json::from_value(json!({
        "id": "action", "question": "", "kind": "yesNo", "answer": false, "probability": 0.1,
        "source": "model"
    }))
    .unwrap()];
}

#[test]
fn small_talk_is_dropped_and_sensitive_reaches_no_model() {
    let mut m = meeting(
        &[
            ("nadia", "00:01", "How was the weekend?"),
            ("aryan", "00:05", "The VPN password is Tulip-4471."),
            ("aryan", "00:09", "I'll send the SOW."),
        ],
        &[],
    );
    tag(&mut m, 0, TagKind::SmallTalk);
    tag(&mut m, 1, TagKind::Sensitive);
    tag(&mut m, 2, TagKind::Action);
    let lines = transcript_lines(&m);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("[1 ") && lines[0].contains("[redacted]"));
    assert!(!lines.join("\n").contains("Tulip"), "{lines:?}");

    // Line 2 is a classified action; line 1 isn't, so an item citing only it needs review.
    assert!(classified_action(&m, Some(9000)));
    m.segments[2].tags.clear();
    assert!(!classified_action(&m, Some(1000)));
}
