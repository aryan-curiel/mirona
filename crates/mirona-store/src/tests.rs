use super::*;
use mirona_core::{Tag, TagAnswer, TagKind};
use std::fs;
use tempfile::TempDir;

fn seeded() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(dir.path()).unwrap();
    sample::seed(&mut s).unwrap();
    (dir, s)
}

/// Everything a caller can read, as JSON (core types don't derive `PartialEq`).
fn snapshot(s: &Store) -> serde_json::Value {
    let meetings: Vec<Meeting> = s
        .list_meetings()
        .iter()
        .map(|m| s.meeting(&m.id).unwrap())
        .collect();
    serde_json::json!({
        "list": s.list_meetings(),
        "meetings": meetings,
        "items": s.action_items(),
        "people": s.people(),
        "corrections": s.corrections(),
    })
}

fn md(s: &Store, id: &str) -> PathBuf {
    s.meeting_dir(id).unwrap().join(files::MEETING)
}

pub(crate) fn live_seg(id: &str, who: &str, at_ms: u32, text: &str) -> Segment {
    let mut s = sample::seg(
        id,
        who,
        "00:00",
        text,
        [0.9, 0.1, 0.1, 0.1, 0.1],
        vec![],
        &[],
    );
    s.start_ms = at_ms;
    s
}

#[test]
fn corrections_enroll_and_search() {
    let (_d, mut s) = seeded();
    let seg = s
        .correct(Correction {
            segment_id: "c9".into(),
            answer_id: "action".into(),
            value: true,
        })
        .unwrap();
    assert!(seg.tags.iter().any(|t| t.kind == TagKind::Action));
    assert!(!seg.tags.iter().any(|t| t.kind == TagKind::Review));
    assert!(!s.action_items()[2].needs_review);
    assert_eq!(s.corrections().len(), 1);

    s.enroll_speaker("s3", "Inés").unwrap();
    assert_eq!(s.action_items()[0].owner, "Inés");
    assert_eq!(s.search("insurer").len(), 1);

    // All of it survived on disk.
    let s = Store::open(s.root()).unwrap();
    assert_eq!(s.action_items()[0].owner, "Inés");
    assert!(!s.action_items()[2].needs_review);
    assert_eq!(s.corrections().len(), 1);
    assert_eq!(
        s.people().iter().find(|p| p.id == "s3").unwrap().name,
        "Inés"
    );
    let c = s.meeting("claims-desk").unwrap();
    assert!(c.segments[4].tags[0].label.contains("Inés"));
}

#[test]
fn files_round_trip() {
    let (_d, mut s) = seeded();
    s.toggle_action_item("a4").unwrap();
    s.set_tags("claims-desk", &["project:Corvo".into(), "hiring".into()])
        .unwrap();
    s.set_notes(
        "corvo-review",
        vec!["line one".into(), String::new(), "- [ ] todo".into()],
    )
    .unwrap();
    let id = new_id();
    s.start_meeting(&id).unwrap();
    s.push_segment(&id, live_seg("x1", "me", 1200, "Hello \"there\"\nnew line"))
        .unwrap();
    s.set_audio(&id, "audio.wav").unwrap();
    s.set_title(&id, "Ünïcode: weekly / sync").unwrap();
    let before = snapshot(&s);
    let reopened = Store::open(s.root()).unwrap();
    assert!(
        reopened.load_errors().is_empty(),
        "{:?}",
        reopened.load_errors()
    );
    assert_eq!(snapshot(&reopened), before);
    assert_eq!(
        reopened.audio_path(&id).unwrap().unwrap(),
        reopened.meeting_dir(&id).unwrap().join("audio.wav")
    );
}

#[test]
fn files_open_cleanly_in_obsidian() {
    let (_d, s) = seeded();
    let text = fs::read_to_string(md(&s, "claims-desk")).unwrap();
    let rest = text.strip_prefix("---\n").unwrap();
    let (yaml, body) = rest.split_once("\n---\n").unwrap();
    let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).unwrap();
    assert_eq!(v["title"].as_str(), Some("Claims Desk weekly sync"));
    assert!(
        v["created"]
            .as_str()
            .unwrap()
            .parse::<jiff::Timestamp>()
            .is_ok()
    );
    for h in [
        "# Claims Desk weekly sync",
        "## Notes",
        "## Summary",
        "### Decisions",
        "## Action items",
    ] {
        assert!(body.lines().any(|l| l == h), "missing {h}");
    }
    assert!(body.contains("- [ ] Announce the insurer grouping"));
    let name = s.meeting_dir("claims-desk").unwrap();
    let name = name.file_name().unwrap().to_str().unwrap();
    assert!(name.ends_with("-claims-desk-weekly-sync"), "{name}");
    assert_eq!(name.len(), "2026-11-05-0930-claims-desk-weekly-sync".len());
}

#[test]
fn transcript_is_append_only_while_recording() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(dir.path()).unwrap();
    s.start_meeting("live").unwrap();
    s.push_segment("live", live_seg("l1", "me", 0, "first"))
        .unwrap();
    let t = s.meeting_dir("live").unwrap().join(files::TRANSCRIPT);
    let after_one = fs::read_to_string(&t).unwrap();
    let md_one = fs::read_to_string(md(&s, "live")).unwrap();
    s.push_segment("live", live_seg("l2", "me", 900, "second"))
        .unwrap();
    let after_two = fs::read_to_string(&t).unwrap();
    assert!(after_two.starts_with(&after_one));
    assert_eq!(after_two.lines().count(), 2);
    assert_eq!(
        fs::read_to_string(md(&s, "live")).unwrap(),
        md_one,
        "same speaker: md untouched"
    );
    // A new speaker is the only reason to touch meeting.md.
    let sp = s
        .push_segment("live", live_seg("l3", "them", 1800, "third"))
        .unwrap();
    assert_eq!(sp.name, "Them");
    assert_eq!(s.meeting("live").unwrap().speakers.len(), 2);
    assert_eq!(
        s.edit_segment_text("live", "l2", "2nd").unwrap().text,
        "2nd"
    );
    let s = Store::open(dir.path()).unwrap();
    let texts: Vec<_> = s
        .meeting("live")
        .unwrap()
        .segments
        .into_iter()
        .map(|x| x.text)
        .collect();
    assert_eq!(texts, ["first", "2nd", "third"]);
}

#[test]
fn reindex_rebuilds_identical_results() {
    let (_d, mut s) = seeded();
    let queries = [
        "insurer",
        "ship monday",
        "onboarding",
        "lumenix",
        "insur",
        "nothing-here",
    ];
    let run = |s: &Store| -> Vec<_> {
        queries
            .iter()
            .map(|q| (serde_json::to_value(s.search(q)).unwrap(), s.search_hits(q)))
            .collect()
    };
    let before = run(&s);
    s.reindex().unwrap();
    assert_eq!(run(&s), before);
    let root = s.root().to_path_buf();
    drop(s);
    fs::remove_file(root.join(".mirona/index.sqlite")).unwrap();
    let s = Store::open(&root).unwrap();
    assert_eq!(run(&s), before);
    // A garbage index file is derived data: replaced, not fatal.
    drop(s);
    fs::write(root.join(".mirona/index.sqlite"), "not a database").unwrap();
    assert_eq!(run(&Store::open(&root).unwrap()), before);
}

#[test]
fn fts_search() {
    let (_d, mut s) = seeded();
    let ids = |r: Vec<MeetingSummary>| r.into_iter().map(|m| m.id).collect::<Vec<_>>();
    assert_eq!(ids(s.search("insurer")), ["claims-desk"]);
    assert_eq!(
        ids(s.search("INSUR")),
        ["claims-desk"],
        "case-insensitive prefix"
    );
    assert_eq!(ids(s.search("tagging estimate")), ["sprint-planning"]);
    assert_eq!(ids(s.search("Corvo")), ["corvo-review"], "client name");
    assert!(
        s.search("\"unbalanced OR (").is_empty(),
        "FTS syntax is neutralised"
    );
    assert!(
        s.search("Alder-Field-26").is_empty(),
        "redacted text is not indexed"
    );
    assert_eq!(s.search("").len(), 4);

    let hits = s.search_hits("overdue filter");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        (hits[0].meeting_id.as_str(), hits[0].segment_id.as_str()),
        ("claims-desk", "c6")
    );
    assert_eq!(hits[0].start_ms, sample::ms("04:45"));
    assert!(hits[0].snippet.contains("[overdue]"));

    // Live segments and edits are searchable straight away; diacritics fold.
    s.start_meeting("live").unwrap();
    s.push_segment("live", live_seg("l1", "me", 0, "Zebra crossing"))
        .unwrap();
    assert_eq!(ids(s.search("zebra")), ["live"]);
    s.edit_segment_text("live", "l1", "Café crossing").unwrap();
    assert!(s.search("zebra").is_empty());
    assert_eq!(ids(s.search("cafe")), ["live"]);
    s.set_notes("live", vec!["kangaroo".into()]).unwrap();
    assert_eq!(ids(s.search("kangaroo")), ["live"]);
}

#[test]
fn hand_edited_notes_are_read_back() {
    let (_d, mut s) = seeded();
    let path = md(&s, "sprint-planning");
    let text = fs::read_to_string(&path).unwrap();
    let (head, tail) = text.split_once("## Notes\n").unwrap();
    let generated = &tail[tail.find("<!-- mirona").unwrap()..];
    let edited = format!(
        "{head}## Notes\n\nEdited in Obsidian\n## My own heading\n- bullet\n\n{generated}\n## Summary\nhand-edited summary is ignored\n"
    );
    fs::write(&path, edited.replace('\n', "\r\n")).unwrap();
    s.reindex().unwrap();
    let m = s.meeting("sprint-planning").unwrap();
    assert_eq!(
        m.notes.raw,
        ["Edited in Obsidian", "## My own heading", "- bullet"]
    );
    assert!(m.summary.unwrap().overview.starts_with("Sprint goal set"));
    assert_eq!(s.search("obsidian").len(), 1);
}

#[test]
fn corrupt_meeting_is_skipped_not_fatal() {
    let (_d, s) = seeded();
    let root = s.root().to_path_buf();
    let bad = md(&s, "corvo-review");
    drop(s);
    fs::write(&bad, "---\nid: [unclosed\n---\n").unwrap();
    let t = root.join("2026-01-01-0000-x");
    fs::create_dir_all(&t).unwrap();
    fs::write(t.join(files::MEETING), "no frontmatter at all").unwrap();
    let claims = Store::open(&root)
        .unwrap()
        .meeting_dir("claims-desk")
        .unwrap();
    fs::write(claims.join(files::TRANSCRIPT), "{oops\n").unwrap(); // bad line, meeting still loads

    let s = Store::open(&root).unwrap();
    assert_eq!(
        s.list_meetings().len(),
        3,
        "corvo-review and the stray folder are skipped"
    );
    assert_eq!(s.load_errors().len(), 3, "{:?}", s.load_errors());
    assert!(s.meeting("claims-desk").unwrap().segments.is_empty());
}

#[test]
fn meetings_dir_is_configurable() {
    let d = TempDir::new().unwrap();
    let a = d.path().join("nested/not/yet/there");
    let mut s = Store::open(&a).unwrap();
    s.start_meeting("one").unwrap();
    assert!(s.meeting_dir("one").unwrap().starts_with(&a));
    assert!(a.join(".mirona/index.sqlite").is_file());
    let other = Store::open(d.path().join("elsewhere")).unwrap();
    assert!(other.list_meetings().is_empty());
}

#[test]
fn title_renames_the_folder() {
    let d = TempDir::new().unwrap();
    let mut s = Store::open(d.path()).unwrap();
    s.start_meeting("m").unwrap();
    s.push_segment("m", live_seg("l1", "me", 0, "hi")).unwrap();
    let old = s.meeting_dir("m").unwrap();
    assert!(old.to_string_lossy().ends_with("-new-meeting"));
    s.set_title("m", "Board demo prep").unwrap();
    let new = s.meeting_dir("m").unwrap();
    assert!(new.to_string_lossy().ends_with("-board-demo-prep"));
    assert!(!old.exists() && new.join(files::TRANSCRIPT).is_file());
    // A second meeting with the same minute and title gets a suffix.
    let created = Store::open(d.path()).unwrap().entries[0].created.clone();
    let mut m = s.meeting("m").unwrap();
    m.id = "m2".into();
    s.add_meeting(m, &created, vec![]).unwrap();
    assert!(
        s.meeting_dir("m2")
            .unwrap()
            .to_string_lossy()
            .ends_with("-board-demo-prep-2")
    );
}

#[test]
fn summary_and_demo_finish() {
    let d = TempDir::new().unwrap();
    let mut s = Store::open(d.path()).unwrap();
    s.start_meeting("m").unwrap();
    let mut seg = live_seg(
        "l1",
        "aryan",
        5000,
        "I'll prepare a short demo script by Tuesday.",
    );
    seg.tags.push(Tag {
        kind: TagKind::Action,
        label: "Action".into(),
    });
    s.push_segment("m", seg).unwrap();
    s.toggle_action_item("x").unwrap_err();
    let m = sample::demo_finish(&mut s, "m", 125_000, "Ollama (local)").unwrap();
    assert_eq!(
        (m.title.as_str(), m.tags.join(","), m.duration_min),
        (
            "Evaluation dashboard follow-up",
            "project:Lumenix".into(),
            2
        )
    );
    assert_eq!(
        s.action_items()[0].meeting_title,
        "Evaluation dashboard follow-up"
    );
    s.toggle_action_item("m-a0").unwrap();
    // Re-summarising keeps done state for items that keep their id.
    let items = s.action_items();
    let summary = s.meeting("m").unwrap().summary.unwrap();
    s.set_summary("m", summary, items).unwrap();
    assert!(Store::open(d.path()).unwrap().action_items()[0].done);
}

#[test]
fn note_times_round_trip_and_survive_outside_edits() {
    let (_d, mut s) = seeded();
    let lines = vec!["- first".into(), "**bold** line".into(), "- first".into()];
    s.set_notes_at("corvo-review", lines, vec![Some(1000), None, Some(3000)])
        .unwrap();
    let reopened = Store::open(s.root()).unwrap();
    let n = reopened.meeting("corvo-review").unwrap().notes;
    assert_eq!(n.raw, ["- first", "**bold** line", "- first"]);
    assert_eq!(n.raw_at_ms, [Some(1000), None, Some(3000)]);

    // Another editor rewrites one line and inserts one: only those lose their time.
    let path = md(&reopened, "corvo-review");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("**bold** line", "edited")
        .replacen("- first\n", "new top\n- first\n", 1);
    fs::write(&path, text).unwrap();
    let n = Store::open(s.root())
        .unwrap()
        .meeting("corvo-review")
        .unwrap()
        .notes;
    assert_eq!(n.raw, ["new top", "- first", "edited", "- first"]);
    assert_eq!(n.raw_at_ms, [None, Some(1000), None, Some(3000)]);
}

#[test]
fn delete_meeting_removes_folder_index_and_items() {
    let (_dir, mut s) = seeded();
    let id = s.list_meetings()[0].id.clone();
    let dir = s.meeting_dir(&id).unwrap();
    let word = s.meeting(&id).unwrap().segments[0]
        .text
        .split_whitespace()
        .find(|w| w.len() > 5)
        .unwrap()
        .to_string();
    s.delete_meeting(&id).unwrap();
    assert!(!dir.exists());
    assert!(s.meeting(&id).is_err());
    assert!(s.action_items().iter().all(|a| a.meeting_id != id));
    assert!(s.search_hits(&word).iter().all(|h| h.meeting_id != id));
    s.reindex().unwrap();
    assert!(s.meeting(&id).is_err(), "gone from disk too");
}

#[test]
fn clear_parts_keeps_transcript_notes_and_sensitive_tags() {
    use mirona_core::MeetingPart;
    let (_dir, mut s) = seeded();
    // A sample meeting with a summary, items and tags.
    let id = s
        .list_meetings()
        .into_iter()
        .find(|m| m.has_summary && m.open_actions > 0)
        .unwrap()
        .id;
    let before = s.meeting(&id).unwrap();
    let i = s.idx(&id).unwrap();
    s.entries[i].meeting.segments[0].tags = vec![
        Tag {
            kind: TagKind::Sensitive,
            label: "Sensitive".into(),
        },
        Tag {
            kind: TagKind::Action,
            label: "Action".into(),
        },
    ];
    s.clear_parts(&id, &[MeetingPart::Summary, MeetingPart::Classification])
        .unwrap();
    s.reindex().unwrap();
    let m = s.meeting(&id).unwrap();
    assert!(m.summary.is_none() && m.answers.is_none());
    assert!(s.action_items().iter().all(|a| a.meeting_id != id));
    assert_eq!(m.segments.len(), before.segments.len());
    assert_eq!(m.notes.raw, before.notes.raw);
    assert_eq!(m.segments[0].tags.len(), 1, "only the sensitive tag stays");
    assert_eq!(m.segments[0].tags[0].kind, TagKind::Sensitive);
    assert!(
        m.segments
            .iter()
            .skip(1)
            .all(|x| x.tags.iter().all(|t| t.kind == TagKind::Sensitive))
    );
    let row = s.list_meetings().into_iter().find(|r| r.id == id).unwrap();
    assert!(!row.has_summary && row.open_actions == 0 && row.meeting_type.is_empty());
}

#[test]
fn enrolling_names_the_real_meetings() {
    let (_dir, mut s) = seeded();
    let mut a = live_seg("a-0", "them", 0, "hello");
    let id_a = new_meeting("Design review", &mut s);
    a.id = format!("{id_a}-0");
    s.push_segment(&id_a, a).unwrap();
    let p = s.enroll_speaker("them", "Nadia").unwrap();
    assert_eq!(p.subtitle, "Named in Design review");
    // Named, but no voice heard yet (not diarized): no voiceprint.
    assert!(p.voiceprint.is_none());

    let id_b = new_meeting("Budget sync", &mut s);
    s.push_segment(&id_b, live_seg(&format!("{id_b}-0"), "them", 0, "hi"))
        .unwrap();
    let p = s.enroll_speaker("them", "Nadia").unwrap();
    assert!(
        p.subtitle.starts_with("Named in ") && p.subtitle.ends_with(" and 1 more"),
        "{}",
        p.subtitle
    );
}

fn new_meeting(title: &str, s: &mut Store) -> String {
    let id = format!("m-{title}");
    s.start_meeting(&id).unwrap();
    s.set_title(&id, title).unwrap();
    id
}

#[test]
fn classification_review_list_and_labels() {
    let (_d, mut s) = seeded();
    let m = s.meeting("claims-desk").unwrap();
    let seg = m.segments[0].clone();
    let mut answers = seg.answers.clone();
    answers[0].answer = true;
    answers[0].probability = 0.7;
    answers[0].threshold = 0.99;
    let tags = vec![review_tag(TagKind::Action, 0.7)];
    let back = s
        .set_classification("claims-desk", &seg.id, answers, tags, vec![], None)
        .unwrap();
    assert_eq!(back.text, seg.text, "text left alone");
    assert_eq!(back.tags[0].label, "Needs review · action? 0.70");
    let review = s.review("claims-desk").unwrap();
    assert!(
        review
            .iter()
            .any(|r| r.segment_id.as_deref() == Some(seg.id.as_str()) && r.question == "action"),
        "{review:?}"
    );

    // The fix clears the review, tags the segment and becomes a labelled example.
    let fixed = s
        .correct(Correction {
            segment_id: seg.id.clone(),
            answer_id: "action".into(),
            value: true,
        })
        .unwrap();
    assert!(fixed.tags.iter().any(|t| t.kind == TagKind::Action));
    assert!(!fixed.tags.iter().any(|t| t.kind == TagKind::Review));
    let labels = fs::read_to_string(s.labels_path()).unwrap();
    let row: serde_json::Value = serde_json::from_str(labels.lines().last().unwrap()).unwrap();
    assert_eq!(row["labels"]["action"], "yes");
    assert!(row["text"].as_str().unwrap().ends_with(&seg.text));
    assert!((row["model_probability"].as_f64().unwrap() - 0.7).abs() < 1e-6);

    // A later classification keeps the user's fix.
    let mut again = fixed.answers.clone();
    again[0].source = mirona_core::AnswerSource::Model;
    again[0].probability = 0.1;
    let kept = s
        .set_classification("claims-desk", &seg.id, again, fixed.tags, vec![], None)
        .unwrap();
    assert!((kept.answers[0].probability - 1.0).abs() < 1e-6);
}

#[test]
fn tags_below_threshold_wait_for_review_and_user_choices_stick() {
    let (_d, mut s) = seeded();
    let tags = |s: &Store| s.meeting("claims-desk").unwrap().tags;
    let reviews = |s: &Store| -> Vec<(String, String)> {
        s.review("claims-desk")
            .unwrap()
            .into_iter()
            .filter(|r| r.question.ends_with("tag"))
            .map(|r| (r.question, r.value))
            .collect()
    };
    // The demo: billing at 0.66 is below 0.75, so it is suggested, not added.
    assert_eq!(tags(&s), ["project:Alder Restoration"]);
    assert_eq!(reviews(&s), [("tag".into(), "topic:billing".into())]);

    // A new run: billing clears its threshold, hiring is a "no", a new tag is proposed.
    let mut a = s.meeting("claims-desk").unwrap().answers.unwrap();
    let answer = |tag: &str, p: f32, proposed: bool| TagAnswer {
        tag: tag.into(),
        probability: p,
        threshold: 0.6,
        confirmed: false,
        proposed,
        about: String::new(),
    };
    a.tags = vec![
        answer("topic:billing", 0.9, false),
        answer("hiring", 0.2, false),
        answer("project:Insurers", 0.0, true),
    ];
    s.set_answers("claims-desk", a.clone()).unwrap();
    assert_eq!(tags(&s), ["project:Alder Restoration", "topic:billing"]);
    assert_eq!(reviews(&s), [("new_tag".into(), "project:Insurers".into())]);
    assert_eq!(
        s.action_items()
            .iter()
            .find(|i| i.meeting_id == "claims-desk")
            .unwrap()
            .tags
            .len(),
        2,
        "action items carry the meeting's tags"
    );

    // Accept the proposal; remove billing by hand. A rerun adds neither back nor re-asks.
    s.answer_tag("claims-desk", "project:Insurers", true)
        .unwrap();
    s.set_tags(
        "claims-desk",
        &[
            "project:Alder Restoration".into(),
            "project:Insurers".into(),
        ],
    )
    .unwrap();
    s.set_answers("claims-desk", a).unwrap();
    assert_eq!(tags(&s), ["project:Alder Restoration", "project:Insurers"]);
    assert_eq!(reviews(&s), []);
}

#[test]
fn meetings_from_before_tags_read_their_client_as_a_project_tag() {
    let (_d, s) = seeded();
    let path = s.meeting_dir("claims-desk").unwrap().join("meeting.md");
    let text = std::fs::read_to_string(&path).unwrap();
    let old = text.replacen(
        "tags:\n- project:Alder Restoration\n",
        "client: Bingham\n",
        1,
    );
    assert_ne!(old, text, "{text}");
    std::fs::write(&path, old).unwrap();
    let s = Store::open(s.root()).unwrap();
    assert!(s.load_errors().is_empty(), "{:?}", s.load_errors());
    assert_eq!(s.meeting("claims-desk").unwrap().tags, ["project:Bingham"]);
}

#[test]
fn corrections_file_drop_and_reassign_action_items() {
    let (_d, mut s) = seeded();
    let items = |s: &Store| -> Vec<ActionItem> {
        s.action_items()
            .into_iter()
            .filter(|a| a.meeting_id == "claims-desk")
            .collect()
    };
    let fix = |s: &mut Store, seg: &str, value: bool| {
        s.correct(Correction {
            segment_id: seg.into(),
            answer_id: "action".into(),
            value,
        })
        .unwrap()
    };
    // Not an action after all: its item goes.
    fix(&mut s, "c6", false);
    assert!(
        !items(&s)
            .iter()
            .any(|a| a.at_ms == Some(sample::ms("04:45")))
    );
    // An action the summary missed: filed, owned by the speaker.
    fix(&mut s, "c3", true);
    let added = items(&s)
        .into_iter()
        .find(|a| a.at_ms == Some(sample::ms("02:40")))
        .unwrap();
    assert_eq!(added.owner, "Speaker 3");
    assert!(!added.needs_review);

    // Owner choice: tag, owners and items follow.
    let seg = s.set_owner("claims-desk", "c5", "Nadia").unwrap();
    assert_eq!(seg.owners[0].label, "Nadia");
    assert!(seg.tags.iter().any(|t| t.label == "Action · Nadia"));
    assert_eq!(items(&s)[0].owner, "Nadia");

    // Review list items resolve in one call.
    s.resolve_action_item("a3", true).unwrap();
    assert!(!items(&s).iter().any(|a| a.needs_review));
    s.resolve_action_item("a3", false).unwrap();
    assert!(!items(&s).iter().any(|a| a.id == "a3"));

    let s = Store::open(s.root()).unwrap();
    assert_eq!(items(&s)[0].owner, "Nadia");
    assert!(
        items(&s)
            .iter()
            .any(|a| a.at_ms == Some(sample::ms("02:40")))
    );
}

#[test]
fn transcripts_from_before_end_times_and_due_dates_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(files::TRANSCRIPT);
    let old = r#"{"id":"a-0","speakerId":"me","startMs":1000,"text":"Hi.","redacted":false,"tags":[],"answers":[],"owners":[]}"#;
    std::fs::write(&path, format!("{old}\n")).unwrap();
    let (segs, errors) = files::read_transcript(&path, None);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!((segs[0].end_ms, segs[0].due.clone()), (0, None));
}

#[test]
fn refresh_picks_up_writes_from_another_process() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = Store::open(dir.path()).unwrap();
    assert!(!app.refresh().unwrap(), "nothing changed yet");

    // Another process (the CLI, an import) adds a meeting.
    let mut cli = Store::open(dir.path()).unwrap();
    let id = new_meeting("Imported", &mut cli);
    assert!(app.meeting(&id).is_err());
    assert!(app.refresh().unwrap());
    assert_eq!(app.meeting(&id).unwrap().title, "Imported");
    assert!(!app.refresh().unwrap(), "already up to date");

    // The app's own writes don't trigger a reload.
    app.set_title(&id, "Renamed").unwrap();
    assert!(!app.refresh().unwrap());
    assert!(cli.refresh().unwrap(), "but the other store sees them");
    assert_eq!(cli.meeting(&id).unwrap().title, "Renamed");
}

#[test]
fn meeting_level_and_owner_corrections_become_labels() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(dir.path()).unwrap();
    let id = new_meeting("Pilot review", &mut s);
    let mut seg = live_seg(
        &format!("{id}-0"),
        "them",
        0,
        "I'll send the proposal by Thursday.",
    );
    seg.speaker_id = "them".into();
    s.push_segment(&id, seg).unwrap();
    s.set_tags(&id, &["project:Northwind Bank".into()]).unwrap();
    s.label_meeting(&id, "meeting_type", "client_call").unwrap();
    s.set_owner(&id, &format!("{id}-0"), "Nadia").unwrap();

    let rows: Vec<serde_json::Value> = std::fs::read_to_string(s.labels_path())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let label = |k: &str| {
        rows.iter()
            .find_map(|r| r["labels"][k].as_str().map(String::from))
    };
    assert_eq!(label("tags").as_deref(), Some("project:Northwind Bank"));
    assert_eq!(label("meeting_type").as_deref(), Some("client_call"));
    assert_eq!(label("owner").as_deref(), Some("Nadia"));
    let meeting_row = rows
        .iter()
        .find(|r| r["labels"]["tags"].is_string())
        .unwrap();
    assert_eq!(meeting_row["id"], format!("{id}-meeting"));
    assert!(
        meeting_row["text"]
            .as_str()
            .unwrap()
            .contains("send the proposal")
    );
}

#[test]
fn split_sentences_are_joined() {
    let line = |who: &str, at: u32, end: u32, text: &str| {
        let mut s = live_seg(&format!("m-{at}"), who, at, text);
        s.end_ms = end;
        s
    };
    let mut segs = vec![
        line("me", 0, 2_000, "So what I think we should"),
        line("me", 2_500, 4_000, "do is ship on Friday."),
        line("me", 4_300, 6_000, "Then we"),
        line("them", 6_200, 7_000, "Sure"),
        line("them", 9_000, 10_000, "and also"),
        line("them", 10_100, 41_000, "a very long monologue"),
    ];
    assert_eq!(merge_lines(&mut segs), 1);
    let texts: Vec<&str> = segs.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "So what I think we should do is ship on Friday.", // same speaker, short pause
            "Then we",                                         // ended a sentence before it
            "Sure",                                            // another speaker
            "and also",                                        // 2 s pause
            "a very long monologue",                           // would pass 30 s
        ]
    );
    assert_eq!((segs[0].id.as_str(), segs[0].end_ms), ("m-0", 4_000));
    assert!(segs[0].answers.is_empty());
}
