//! The M1 pipeline in one place: record (or import) → transcribe → store → summary and
//! enhanced notes → store. The app and the CLI both drive meetings through this crate.
//!
//! Speakers come from the audio source until diarization lands (M2): the mic is "me", the
//! system audio is "them", an imported file is "speaker".

mod llm;

pub use llm::Llm;

use mirona_capture::{Event, Recorder, RecorderConfig, Source, Stt, VadConfig};
use mirona_core::{PipelineEvent, Segment};
use mirona_providers::LlmProvider;
use mirona_providers::config::Config;
use mirona_store::Store;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

pub type Result<T> = std::result::Result<T, String>;
pub type Shared = Arc<Mutex<Store>>;

/// File name of the recording inside the meeting folder.
pub const AUDIO_FILE: &str = "audio.wav";

/// Folders the engine works in, resolved from `Config` plus the app's defaults.
#[derive(Debug, Clone)]
pub struct Paths {
    pub meetings: PathBuf,
    pub models: PathBuf,
    pub templates: PathBuf,
}

impl Paths {
    /// `app_dir` is the per-user app folder (e.g. `~/Library/Application Support/<id>`);
    /// `documents` is where meetings go unless the config says otherwise.
    pub fn resolve(cfg: &Config, app_dir: &Path, documents: &Path) -> Self {
        Self {
            meetings: cfg
                .meetings_dir
                .clone()
                .unwrap_or_else(|| documents.join("Mirona")),
            models: cfg
                .models_dir
                .clone()
                .unwrap_or_else(|| app_dir.join("models")),
            templates: app_dir.join("templates"),
        }
    }
}

/// The app's bundle identifier: also the name of its per-user folder.
pub const APP_ID: &str = "com.nagare.mirona";

/// The per-user app folder, the same one Tauri's `app_config_dir` uses, so the app, the CLI
/// and the MCP server share `config.toml`, models and templates. `MIRONA_APP_DIR` overrides it
/// (tests, a second profile).
pub fn app_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MIRONA_APP_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let base = if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home().join(".config"), PathBuf::from)
    };
    base.join(APP_ID)
}

/// Where meetings go by default: `~/Documents/Mirona`.
pub fn documents_dir() -> PathBuf {
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    PathBuf::from(home).join("Documents")
}

/// Model files that recording and import need but `models` lacks, for the configured STT
/// (`scripts/fetch-models.sh <models>` fetches them). Empty: ready to record.
pub fn missing_models(cfg: &Config, models: &Path) -> Vec<String> {
    use mirona_capture::{PARAKEET_DIR, VAD_MODEL, WHISPER_MODELS};
    let mut out = vec![];
    if !models.join(VAD_MODEL).is_file() {
        out.push(VAD_MODEL.to_string());
    }
    if stage(cfg, "stt", "parakeet") == "whisper" {
        if !WHISPER_MODELS.iter().any(|m| models.join(m).is_file()) {
            out.push(WHISPER_MODELS[0].to_string());
        }
    } else if !models.join(PARAKEET_DIR).join("tokens.txt").is_file() {
        out.push(format!("{PARAKEET_DIR}/"));
    }
    out
}

pub fn config_path(app_dir: &Path) -> PathBuf {
    app_dir.join("config.toml")
}

pub struct Engine {
    pub store: Shared,
    pub paths: Paths,
    cfg: Mutex<Config>,
    /// Loaded on first use and kept: loading Parakeet takes a few seconds.
    stt: Mutex<Option<(String, Arc<Stt>)>>,
}

impl Engine {
    pub fn new(store: Shared, cfg: Config, paths: Paths) -> Self {
        Self {
            store,
            paths,
            cfg: Mutex::new(cfg),
            stt: Mutex::new(None),
        }
    }

    /// Load `config.toml` from `app_dir` (defaults if missing) and open the meeting store.
    pub fn open(app_dir: &Path, documents: &Path) -> Result<Self> {
        let cfg = Config::load(&config_path(app_dir))?;
        let paths = Paths::resolve(&cfg, app_dir, documents);
        let store = Store::open(&paths.meetings)?;
        Ok(Self::new(Arc::new(Mutex::new(store)), cfg, paths))
    }

    /// Save the config next to the app's other files and use it from now on.
    pub fn save_config(&self, app_dir: &Path, cfg: Config) -> Result<()> {
        cfg.save(&config_path(app_dir))?;
        self.set_config(cfg);
        Ok(())
    }

    pub fn config(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }

    pub fn set_config(&self, cfg: Config) {
        *self.cfg.lock().unwrap() = cfg;
    }

    fn stt(&self) -> Result<Arc<Stt>> {
        let id = stage(&self.config(), "stt", "parakeet");
        let mut slot = self.stt.lock().unwrap();
        if let Some((loaded, stt)) = slot.as_ref()
            && *loaded == id
        {
            return Ok(stt.clone());
        }
        let stt = Arc::new(Stt::load(&id, &self.paths.models)?);
        *slot = Some((id, stt.clone()));
        Ok(stt)
    }

    /// Start a live recording of mic + system audio into a new meeting.
    pub fn record(&self, events: Sender<PipelineEvent>) -> Result<Session> {
        // One recorder per meetings folder, whether it's the app or `mirona record`.
        let lock = Arc::new(RecordingLock::take(&self.paths.meetings)?);
        let stt = self.stt()?;
        let id = mirona_store::new_id();
        let dir = {
            let mut s = self.store.lock().unwrap();
            s.start_meeting(&id)?;
            s.meeting_dir(&id)?
        };
        let (tx, rx) = channel();
        let recorder = Recorder::start(
            RecorderConfig {
                models_dir: self.paths.models.clone(),
                vad: VadConfig::default(),
                recording: dir.join(AUDIO_FILE),
                mic: true,
                system: true,
            },
            stt,
            tx,
        )
        .inspect_err(|_| {
            // Nothing was recorded: don't leave an empty meeting behind.
            let _ = self.store.lock().unwrap().delete_meeting(&id);
        })?;
        self.store.lock().unwrap().set_audio(&id, AUDIO_FILE)?;
        let started = Instant::now();
        let forward = {
            let (store, id, lock) = (self.store.clone(), id.clone(), lock.clone());
            std::thread::spawn(move || forward(&store, &id, started, &rx, &events, &lock))
        };
        Ok(Session {
            id,
            recorder,
            forward,
            started,
            lock,
        })
    }

    /// Transcribe an audio file into a new meeting, then summarize it.
    pub fn import(&self, path: &Path, events: &Sender<PipelineEvent>) -> Result<String> {
        let stt = self.stt()?;
        let segments = mirona_capture::transcribe_file(
            path,
            &self.paths.models,
            VadConfig::default(),
            stt.as_ref(),
        )?;
        let id = mirona_store::new_id();
        let end_ms = segments.last().map_or(0, |s| s.end_ms);
        {
            let mut s = self.store.lock().unwrap();
            s.start_meeting(&id)?;
            if let Some(stem) = path.file_stem().and_then(|n| n.to_str()) {
                s.set_title(&id, stem)?;
            }
            for (n, seg) in segments.into_iter().enumerate() {
                s.push_segment(&id, segment(&id, n, &seg))?;
            }
            // Keep a copy next to the transcript, like a live recording.
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("audio");
            let name = format!("audio.{ext}");
            std::fs::copy(path, s.meeting_dir(&id)?.join(&name)).map_err(|e| e.to_string())?;
            s.set_audio(&id, &name)?;
            s.finish_meeting(&id, end_ms)?;
        }
        self.post_process(&id, events);
        Ok(id)
    }

    /// Summary + enhanced notes for a finished meeting, with the configured LLM. Failures are
    /// reported as notices; the meeting itself is already saved.
    pub fn post_process(&self, id: &str, events: &Sender<PipelineEvent>) {
        let cfg = self.config();
        let notice = |message: String, error: bool| {
            events
                .send(PipelineEvent::Notice {
                    meeting_id: id.into(),
                    message,
                    error,
                })
                .ok();
        };
        match Llm::from_config(&cfg) {
            Ok(llm) => match summarize_with(&self.store, &self.paths, id, &cfg.template, &llm) {
                Ok(()) => {
                    events
                        .send(PipelineEvent::Summarized {
                            meeting_id: id.into(),
                        })
                        .ok();
                }
                Err(e) => notice(format!("Summary failed: {e}"), true),
            },
            Err(e) => notice(format!("No summary: {e}"), true),
        }
    }

    /// Write (or rewrite) the summary with another template or model (NGR-146).
    pub fn summarize(&self, id: &str, template: &str) -> Result<()> {
        let llm = Llm::from_config(&self.config())?;
        summarize_with(&self.store, &self.paths, id, template, &llm)
    }
}

/// Summary + enhanced notes with any LLM; saved straight into the meeting.
pub fn summarize_with(
    store: &Shared,
    paths: &Paths,
    id: &str,
    template: &str,
    llm: &(impl LlmProvider + Labelled),
) -> Result<()> {
    let meeting = store.lock().unwrap().meeting(id)?;
    let template = mirona_summary::template(&paths.templates, template)?;
    let (mut summary, items) = block_on(mirona_summary::summarize(&meeting, &template, llm))?;
    summary.written_by = llm.label();
    store.lock().unwrap().set_summary(id, summary, items)?;
    if !meeting.notes.raw.is_empty() {
        let notes = block_on(mirona_summary::enhance_notes(&meeting, llm))?;
        store
            .lock()
            .unwrap()
            .set_enhanced_notes(id, notes, &llm.label())?;
    }
    Ok(())
}

/// A human-readable "model (provider)" name for `Summary.written_by`.
pub trait Labelled {
    fn label(&self) -> String;
}

/// A recording in progress.
pub struct Session {
    pub id: String,
    recorder: Recorder,
    forward: JoinHandle<()>,
    started: Instant,
    lock: Arc<RecordingLock>,
}

impl Session {
    pub fn elapsed_ms(&self) -> u32 {
        u32::try_from(self.started.elapsed().as_millis()).unwrap_or(u32::MAX)
    }

    /// Stop capturing, flush the last speech, save the meeting, then summarize it.
    pub fn stop(self, engine: &Engine, events: &Sender<PipelineEvent>) -> Result<String> {
        let elapsed = self.elapsed_ms();
        let stopped = self.recorder.stop();
        // The recorder dropped its sender: the forwarder drains the rest and ends.
        self.forward.join().ok();
        // Capture is over: another recording may start while this one is summarized.
        drop(self.lock);
        engine
            .store
            .lock()
            .unwrap()
            .finish_meeting(&self.id, elapsed)?;
        stopped?;
        engine.post_process(&self.id, events);
        Ok(self.id)
    }
}

/// How often a live recording refreshes its lock, and when an unrefreshed lock counts as left
/// over from a crash.
const LOCK_BEAT: std::time::Duration = std::time::Duration::from_secs(3);
const LOCK_STALE: std::time::Duration = std::time::Duration::from_secs(15);

/// `<meetings>/.mirona/recording.lock`: one live recording per meetings folder across the app
/// and the CLI. Kept fresh while recording; removed when capture stops (or on drop).
pub struct RecordingLock(PathBuf);

impl RecordingLock {
    pub fn path(meetings: &Path) -> PathBuf {
        meetings.join(".mirona").join("recording.lock")
    }

    pub fn take(meetings: &Path) -> Result<Self> {
        let path = Self::path(meetings);
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < LOCK_STALE);
        if fresh {
            let who = std::fs::read_to_string(&path).unwrap_or_default();
            return Err(format!(
                "Already recording into this meetings folder ({}). Stop that recording first.",
                who.trim()
            ));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, format!("pid {}", std::process::id()))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self(path))
    }

    fn beat(&self) {
        if let Ok(f) = std::fs::File::options().write(true).open(&self.0) {
            f.set_modified(std::time::SystemTime::now()).ok();
        }
    }
}

impl Drop for RecordingLock {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

/// Capture events → store + pipeline events, in arrival order.
fn forward(
    store: &Shared,
    id: &str,
    started: Instant,
    rx: &std::sync::mpsc::Receiver<Event>,
    events: &Sender<PipelineEvent>,
    lock: &RecordingLock,
) {
    let mut n = 0;
    let mut beat = Instant::now();
    for ev in rx {
        if beat.elapsed() >= LOCK_BEAT {
            lock.beat();
            beat = Instant::now();
        }
        let out = match ev {
            Event::Levels { mic, system } => PipelineEvent::Captured {
                meeting_id: id.into(),
                elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
                levels: vec![mic, system],
            },
            Event::Segment(seg) => {
                let seg = segment(id, n, &seg);
                n += 1;
                match store.lock().unwrap().push_segment(id, seg.clone()) {
                    Ok(speaker) => PipelineEvent::Transcribed {
                        meeting_id: id.into(),
                        segment: seg,
                        speaker,
                    },
                    Err(e) => notice(id, format!("Could not save a segment: {e}"), true),
                }
            }
            Event::Device { source, name } => {
                notice(id, format!("{} → {name}", source_label(source)), false)
            }
            Event::Error { message, .. } => notice(id, message, true),
        };
        events.send(out).ok();
    }
}

fn notice(id: &str, message: String, error: bool) -> PipelineEvent {
    PipelineEvent::Notice {
        meeting_id: id.into(),
        message,
        error,
    }
}

fn source_label(s: Source) -> &'static str {
    match s {
        Source::Mic => "Microphone",
        Source::System => "System audio",
        Source::File => "File",
    }
}

/// Until diarization (M2) the source says who spoke.
pub fn speaker_id(s: Source) -> &'static str {
    match s {
        Source::Mic => "me",
        Source::System => "them",
        Source::File => "speaker",
    }
}

fn segment(meeting_id: &str, n: usize, s: &mirona_capture::Segment) -> Segment {
    Segment {
        id: format!("{meeting_id}-{n}"),
        speaker_id: speaker_id(s.source).into(),
        start_ms: s.start_ms,
        text: s.text.clone(),
        redacted: false,
        tags: vec![],
        answers: vec![],
        owners: vec![],
    }
}

/// The provider id a stage is set to in config, or its default.
pub fn stage(cfg: &Config, stage: &str, default: &str) -> String {
    cfg.providers
        .get(stage)
        .cloned()
        .unwrap_or_else(|| default.into())
}

/// Run a future to completion on this thread. Providers block inside their futures, so this
/// is all the executor the engine needs.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        if let std::task::Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}

#[cfg(test)]
mod tests;
