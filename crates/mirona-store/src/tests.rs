use super::*;
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

fn live_seg(id: &str, who: &str, at_ms: u32, text: &str) -> Segment {
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
    s.confirm_client("claims-desk", "Corvo").unwrap();
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
        (m.title.as_str(), m.client.as_str(), m.duration_min),
        ("Evaluation dashboard follow-up", "Lumenix", 2)
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
