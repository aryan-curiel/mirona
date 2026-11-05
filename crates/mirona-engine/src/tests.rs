use super::*;
use mirona_core::Locality;
use mirona_providers::{Completion, Prompt, Provider, Usage};
use std::sync::mpsc::channel;

/// Answers the summary and notes prompts with fixed JSON, like a well-behaved model.
struct Scripted;

impl Provider for Scripted {
    fn id(&self) -> &'static str {
        "scripted"
    }
    fn locality(&self) -> Locality {
        Locality::Local
    }
}

impl Labelled for Scripted {
    fn label(&self) -> String {
        "scripted-1 (Test)".into()
    }
}

impl LlmProvider for Scripted {
    fn complete(
        &self,
        prompt: &Prompt<'_>,
        on_text: &mut (dyn FnMut(&str) + Send),
    ) -> impl Future<Output = mirona_providers::Result<Completion>> + Send {
        let text = if prompt.system.contains("\"notes\"") {
            r#"{"notes": [{"kind": "action", "text": "Keiko sends the SOW Thursday", "segment": 0}]}"#
        } else {
            r#"{"overview": "Agreed the SOW and fixed price.",
                "decisions": [{"text": "Go with fixed price", "segment": 1}],
                "action_items": [{"text": "Send the revised SOW", "owner": "Keiko", "due": "Thursday", "segment": 0}],
                "open_questions": []}"#
        };
        on_text(text);
        std::future::ready(Ok(Completion {
            text: text.into(),
            usage: Usage::default(),
        }))
    }
}

/// Pending until a value arrives on the channel.
struct Wait(std::sync::mpsc::Receiver<u8>);

impl Future for Wait {
    type Output = u8;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<u8> {
        if let Ok(v) = self.0.try_recv() {
            std::task::Poll::Ready(v)
        } else {
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    }
}

fn engine(dir: &Path) -> Engine {
    let store = Store::open(dir.join("meetings")).unwrap();
    let paths = Paths {
        meetings: dir.join("meetings"),
        models: dir.join("models"),
        templates: dir.join("templates"),
    };
    Engine::new(Arc::new(Mutex::new(store)), Config::default(), paths)
}

/// A finished two-line meeting with rough notes, as a recording would leave it.
fn recorded(e: &Engine) -> String {
    let mut s = e.store.lock().unwrap();
    let id = mirona_store::new_id();
    s.start_meeting(&id).unwrap();
    for (n, (source, text)) in [
        (Source::Mic, "I'll send the revised SOW by Thursday."),
        (
            Source::System,
            "Great, then we go with the fixed price option.",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let seg = mirona_capture::Segment {
            source,
            start_ms: u32::try_from(n).unwrap() * 4000,
            end_ms: u32::try_from(n).unwrap() * 4000 + 3000,
            text: text.into(),
        };
        s.push_segment(&id, segment(&id, n, &seg)).unwrap();
    }
    s.set_notes(&id, vec!["SOW thursday".into()]).unwrap();
    s.finish_meeting(&id, 8000).unwrap();
    id
}

#[test]
fn speakers_come_from_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let e = engine(dir.path());
    let id = recorded(&e);
    let m = e.store.lock().unwrap().meeting(&id).unwrap();
    let ids: Vec<_> = m.segments.iter().map(|s| s.speaker_id.as_str()).collect();
    assert_eq!(ids, ["me", "them"]);
    let names: Vec<_> = m.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Me", "Them"]);
}

#[test]
fn summary_and_enhanced_notes_are_saved_into_the_meeting() {
    let dir = tempfile::tempdir().unwrap();
    let e = engine(dir.path());
    let id = recorded(&e);
    summarize_with(&e.store, &e.paths, &id, "client-call", &Scripted).unwrap();

    // Survives a reopen: it's in the files, not just in memory.
    let store = Store::open(dir.path().join("meetings")).unwrap();
    let m = store.meeting(&id).unwrap();
    let summary = m.summary.unwrap();
    assert_eq!(summary.overview, "Agreed the SOW and fixed price.");
    assert_eq!(summary.written_by, "scripted-1 (Test)");
    assert_eq!(summary.decisions[0].text, "Go with fixed price");
    assert_eq!(m.notes.enhanced.len(), 1);
    assert_eq!(
        m.notes.raw,
        ["SOW thursday"],
        "raw notes are never overwritten"
    );
    let item = &store.action_items()[0];
    assert_eq!(
        (item.owner.as_str(), item.due.as_deref()),
        ("Keiko", Some("Thursday"))
    );

    let md = std::fs::read_to_string(store.meeting_dir(&id).unwrap().join("meeting.md")).unwrap();
    assert!(md.contains("Agreed the SOW and fixed price."), "{md}");
}

#[test]
fn a_failed_summary_keeps_the_meeting_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let e = engine(dir.path());
    let id = recorded(&e);
    let mut cfg = e.config();
    cfg.providers.insert("llm".into(), "nope".into());
    e.set_config(cfg);

    let (tx, rx) = channel();
    e.post_process(&id, &tx);
    match rx.try_recv().unwrap() {
        PipelineEvent::Notice { error, message, .. } => {
            assert!(error && message.contains("nope"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    let m = e.store.lock().unwrap().meeting(&id).unwrap();
    assert_eq!(m.segments.len(), 2);
    assert!(m.summary.is_none());
}

#[test]
fn privacy_mode_refuses_a_cloud_llm() {
    let mut cfg = Config::default();
    cfg.providers.insert("llm".into(), "openai".into());
    cfg.llm.base_url = "https://api.openai.com/v1".into();
    let err = Llm::from_config(&cfg).err().unwrap();
    assert!(err.contains("privacy"), "{err}");

    // A local OpenAI-compatible server (LM Studio) is fine.
    cfg.llm.base_url = "http://localhost:1234/v1".into();
    assert!(Llm::from_config(&cfg).is_ok());
}

#[test]
fn paths_default_to_documents_and_app_dir() {
    let p = Paths::resolve(&Config::default(), Path::new("/app"), Path::new("/docs"));
    assert_eq!(p.meetings, Path::new("/docs/Mirona"));
    assert_eq!(p.models, Path::new("/app/models"));
    let cfg = Config {
        meetings_dir: Some("/elsewhere".into()),
        ..Config::default()
    };
    assert_eq!(
        Paths::resolve(&cfg, Path::new("/app"), Path::new("/docs")).meetings,
        Path::new("/elsewhere")
    );
}

#[test]
fn block_on_waits_for_a_woken_future() {
    let (tx, rx) = channel::<u8>();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        tx.send(7).unwrap();
    });
    assert_eq!(block_on(Wait(rx)), 7);
}

/// Real models + Ollama: import a `LibriSpeech` clip, get a transcript and a summary.
/// `cargo test -p mirona-engine -- --ignored` (models from `MIRONA_MODELS_DIR` or the spike).
#[test]
#[ignore = "needs speech models and a running Ollama with qwen3:4b"]
fn import_transcribes_and_summarizes_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.paths.models = std::env::var_os("MIRONA_MODELS_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/parakeet/models"),
        PathBuf::from,
    );
    let clip = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mirona-capture/tests/fixtures/librispeech-2utt.wav");
    let (tx, rx) = channel();
    let id = e.import(&clip, &tx).unwrap();
    let events: Vec<_> = rx.try_iter().collect();

    let s = e.store.lock().unwrap();
    let m = s.meeting(&id).unwrap();
    assert_eq!(m.title, "librispeech-2utt");
    assert!(!m.segments.is_empty());
    let text = m
        .segments
        .iter()
        .map(|s| s.text.to_lowercase())
        .collect::<String>();
    assert!(text.contains("concord"), "{text}");
    assert!(s.meeting_dir(&id).unwrap().join("audio.wav").exists());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, PipelineEvent::Summarized { .. })),
        "{events:?}"
    );
    let summary = m.summary.as_ref().unwrap();
    assert!(summary.written_by.contains("qwen3:4b"), "{summary:?}");
}

#[test]
fn open_reads_config_and_creates_the_meetings_dir() {
    let dir = tempfile::tempdir().unwrap();
    let (app, docs) = (dir.path().join("app"), dir.path().join("docs"));
    let e = Engine::open(&app, &docs).unwrap();
    assert!(docs.join("Mirona").is_dir());
    let mut cfg = e.config();
    cfg.meetings_dir = Some(dir.path().join("elsewhere"));
    e.save_config(&app, cfg).unwrap();
    let e = Engine::open(&app, &docs).unwrap();
    assert_eq!(e.paths.meetings, dir.path().join("elsewhere"));
    assert!(e.paths.meetings.is_dir());
}
