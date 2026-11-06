//! Tauri shell (NGR-141/142/143/144/150): typed commands over `mirona-engine` and the meeting
//! store at the configured folder, recording from the window, the tray and a global shortcut,
//! and settings that read and write `config.toml`.
//!
//! Long work (stopping + summarizing, import, regenerate) runs on its own thread and reports
//! on the "pipeline" event channel; "recording" carries the recorder state ([`RecState`]).

// Tauri hands commands their arguments by value.
#![allow(clippy::needless_pass_by_value)]

use mirona_capture::Permission;
use mirona_core::{
    ActionItem, Correction, Locality, Meeting, MeetingAnswers, MeetingSummary, PipelineEvent,
    ProviderOption, Review, Segment, Speaker,
};
use mirona_engine::{Engine, Llm, Paths, Session};
use mirona_providers::config::{Config, default_threshold};
use mirona_providers::questions::NO_CLIENT;
use mirona_providers::{check_privacy, keychain};
use mirona_store::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

type Res<T> = Result<T, String>;

struct AppState {
    app_dir: PathBuf,
    documents: PathBuf,
    /// Swapped when the meetings or models folder moves (never while recording).
    engine: RwLock<Arc<Engine>>,
    session: Mutex<Option<Session>>,
    /// Meetings whose recording stopped and that are being summarized.
    processing: Mutex<Vec<String>>,
    /// An audio file is being imported (one at a time).
    importing: std::sync::atomic::AtomicBool,
    /// Problems found at start (bad config, unreadable meetings, shortcut taken).
    startup_errors: Vec<String>,
}

impl AppState {
    fn engine(&self) -> Arc<Engine> {
        self.engine.read().unwrap().clone()
    }

    fn rec_state(&self) -> RecState {
        RecState {
            recording: self.session.lock().unwrap().as_ref().map(|s| s.id.clone()),
            processing: self.processing.lock().unwrap().clone(),
        }
    }
}

type St<'a> = State<'a, AppState>;

/// What the recorder is doing, sent on the "recording" channel whenever it changes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecState {
    recording: Option<String>,
    processing: Vec<String>,
}

// --- Meetings ---

/// Reload the store if the CLI or an import changed the meetings folder. Cheap when nothing did.
#[tauri::command]
fn refresh_store(st: St) -> Res<bool> {
    st.engine().store.lock().unwrap().refresh()
}

#[tauri::command]
fn list_meetings(st: St) -> Vec<MeetingSummary> {
    st.engine().store.lock().unwrap().list_meetings()
}

#[tauri::command]
fn get_meeting(st: St, id: String) -> Res<Meeting> {
    st.engine().store.lock().unwrap().meeting(&id)
}

#[tauri::command]
fn list_action_items(st: St) -> Vec<ActionItem> {
    st.engine().store.lock().unwrap().action_items()
}

#[tauri::command]
fn toggle_action_item(st: St, id: String) -> Res<ActionItem> {
    st.engine().store.lock().unwrap().toggle_action_item(&id)
}

/// A person for the People view (NGR-154): the speaker, how many meetings they're in and the
/// newest one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Person {
    #[serde(flatten)]
    speaker: Speaker,
    meetings: u32,
    last_seen: Option<MeetingSummary>,
}

#[tauri::command]
fn list_people(st: St) -> Res<Vec<Person>> {
    let engine = st.engine();
    let s = engine.store.lock().unwrap();
    let mut seen: Vec<(MeetingSummary, Vec<String>)> = vec![];
    for m in s.list_meetings() {
        let ids = s
            .meeting(&m.id)?
            .speakers
            .into_iter()
            .map(|p| p.id)
            .collect();
        seen.push((m, ids));
    }
    Ok(s.people()
        .iter()
        .map(|p| {
            let mut mine = seen.iter().filter(|(_, ids)| ids.contains(&p.id));
            let last_seen = mine.next().map(|(m, _)| m.clone());
            Person {
                speaker: p.clone(),
                meetings: u32::try_from(mine.count() + usize::from(last_seen.is_some()))
                    .unwrap_or(u32::MAX),
                last_seen,
            }
        })
        .collect())
}

#[tauri::command]
fn enroll_speaker(st: St, speaker_id: String, name: String) -> Res<Speaker> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .enroll_speaker(&speaker_id, &name)
}

/// Name a speaker in one meeting; fixes a wrong voice match (NGR-153/154).
#[tauri::command]
fn name_speaker(st: St, meeting_id: String, speaker_id: String, name: String) -> Res<Speaker> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .name_speaker(&meeting_id, &speaker_id, &name)
}

#[tauri::command]
fn merge_people(st: St, from: String, into: String) -> Res<Speaker> {
    st.engine().store.lock().unwrap().merge_people(&from, &into)
}

#[tauri::command]
fn delete_voiceprint(st: St, person_id: String) -> Res<()> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .delete_voiceprint(&person_id)
}

#[tauri::command]
fn correct_answer(st: St, correction: Correction) -> Res<usize> {
    let engine = st.engine();
    let mut s = engine.store.lock().unwrap();
    s.correct(correction)?;
    Ok(s.corrections().len())
}

#[tauri::command]
fn confirm_client(st: St, meeting_id: String, client: String) -> Res<Meeting> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .confirm_client(&meeting_id, &client)
}

/// What in a meeting is below its threshold and not yet confirmed (NGR-158).
#[tauri::command]
fn review(st: St, meeting_id: String) -> Res<Vec<Review>> {
    st.engine().store.lock().unwrap().review(&meeting_id)
}

/// Confirm (`accept`) or reject one review item in one click (NGR-158). For the meeting type
/// and client, `review.value` is the option the user picked. A confirmed "needs a follow-up?"
/// gets its draft written in the background (a `Notice` says when).
#[tauri::command]
fn resolve_review(
    app: AppHandle,
    st: St,
    meeting_id: String,
    review: Review,
    accept: bool,
) -> Res<()> {
    let engine = st.engine();
    let draft = resolve(
        &mut engine.store.lock().unwrap(),
        &meeting_id,
        &review,
        accept,
    )?;
    if draft {
        let tx = events(&app);
        std::thread::spawn(move || {
            let res = Llm::from_config(&engine.config()).and_then(|llm| {
                mirona_engine::classify::follow_up_draft(&engine.store, &meeting_id, &llm)
            });
            let ev = match res {
                Ok(Some(_)) => notice(&meeting_id, "Follow-up draft written.".into(), false),
                Ok(None) => notice(&meeting_id, "No summary yet to draft from.".into(), false),
                Err(e) => notice(&meeting_id, format!("Follow-up draft failed: {e}"), true),
            };
            tx.send(ev).ok();
        });
    }
    Ok(())
}

/// Applies one review decision; true when a follow-up draft should be written now.
fn resolve(s: &mut Store, meeting_id: &str, r: &Review, accept: bool) -> Res<bool> {
    let fix = |s: &mut Store, segment_id: &str, answer_id: &str| {
        s.correct(Correction {
            segment_id: segment_id.into(),
            answer_id: answer_id.into(),
            value: accept,
        })
        .map(drop)
    };
    match (r.segment_id.as_deref(), r.question.as_str()) {
        (Some(seg), q) => fix(s, seg, q)?,
        (None, "action_item") => {
            let m = s.meeting(meeting_id)?;
            let seg = m.segments.iter().find(|x| {
                Some(x.start_ms) == r.at_ms && x.answers.iter().any(|a| a.id == "action")
            });
            if let Some(seg) = seg {
                fix(s, &seg.id, "action")?;
            } else {
                let item = s
                    .action_items()
                    .into_iter()
                    .find(|a| a.meeting_id == meeting_id && a.text == r.value && a.at_ms == r.at_ms)
                    .ok_or("That action item is gone.")?;
                s.resolve_action_item(&item.id, accept)?;
            }
        }
        (None, "client") => {
            let client = if accept && r.value != NO_CLIENT {
                r.value.as_str()
            } else {
                ""
            };
            s.confirm_client(meeting_id, client)?;
        }
        (None, q @ ("meeting_type" | "follow_up")) => {
            let mut a = s.meeting(meeting_id)?.answers.ok_or("no meeting answers")?;
            confirm_answer(&mut a, q, &r.value, accept);
            let draft = q == "follow_up" && accept && a.follow_up_draft.is_empty();
            let label = if q == "meeting_type" {
                a.meeting_type.value.clone()
            } else {
                (if a.follow_up { "yes" } else { "no" }).to_string()
            };
            s.set_answers(meeting_id, a)?;
            s.label_meeting(meeting_id, q, &label)?;
            return Ok(draft);
        }
        (None, q) => return Err(format!("nothing to review for {q}")),
    }
    Ok(false)
}

/// The user's answer to a meeting question: certain from now on.
fn confirm_answer(a: &mut MeetingAnswers, question: &str, value: &str, accept: bool) {
    let p = if accept { 1.0 } else { 0.0 };
    if question == "meeting_type" {
        let t = &mut a.meeting_type;
        t.value = if accept { value } else { "other" }.into();
        t.probability = 1.0;
        t.confirmed = true;
    } else {
        a.follow_up = accept;
        a.follow_up_probability = p;
    }
}

/// The user picks who owns an action (NGR-159).
#[tauri::command]
fn set_owner(st: St, meeting_id: String, segment_id: String, owner: String) -> Res<Segment> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .set_owner(&meeting_id, &segment_id, &owner)
}

/// Refuses a rerun on a meeting still recording or being processed.
fn idle(st: &AppState, meeting_id: &str) -> Res<()> {
    let s = st.rec_state();
    if s.recording.as_deref() == Some(meeting_id) || s.processing.iter().any(|p| p == meeting_id) {
        return Err("Wait until this meeting has finished processing.".into());
    }
    Ok(())
}

/// Separate the speakers of a finished meeting again (NGR-154). Returns at once; a `Diarized`
/// event and a `Notice` follow.
#[tauri::command]
fn rerun_diarization(app: AppHandle, st: St, meeting_id: String) -> Res<()> {
    idle(&st, &meeting_id)?;
    if mirona_engine::stage(&st.engine().config(), "diarization", "speakrs") == "off" {
        return Err("Speaker detection is off in Models and MCP.".into());
    }
    let (engine, tx) = (st.engine(), events(&app));
    std::thread::spawn(move || {
        let ev = match engine.diarize(&meeting_id, &tx) {
            Ok(0) => notice(&meeting_id, "No other voices found.".into(), false),
            Ok(n) => notice(
                &meeting_id,
                format!("Speakers separated: {n} voice(s)."),
                false,
            ),
            Err(e) => notice(&meeting_id, format!("Speakers not separated: {e}"), true),
        };
        tx.send(ev).ok();
    });
    Ok(())
}

/// Ask the classifier about every segment and the meeting again (NGR-156/157). Corrections
/// are kept. Returns at once; `Classified` events and a `Notice` follow.
#[tauri::command]
fn rerun_classification(app: AppHandle, st: St, meeting_id: String) -> Res<()> {
    idle(&st, &meeting_id)?;
    let engine = st.engine();
    let cfg = engine.config();
    let (c, why) = mirona_engine::classify::AnyClassifier::from_config(&cfg);
    let c = c.ok_or("Classification is off in Models and MCP.")?;
    let tx = events(&app);
    if let Some(why) = why {
        tx.send(notice(&meeting_id, why, false)).ok();
    }
    std::thread::spawn(move || {
        let res = mirona_engine::classify::reclassify(&engine.store, &cfg, &meeting_id, &c, &tx);
        let ev = match res {
            Ok(0) => notice(
                &meeting_id,
                "Tags and meeting answers updated.".into(),
                false,
            ),
            Ok(n) => notice(
                &meeting_id,
                format!("Updated; {n} segment(s) failed."),
                true,
            ),
            Err(e) => notice(&meeting_id, format!("Classification failed: {e}"), true),
        };
        tx.send(ev).ok();
    });
    Ok(())
}

#[tauri::command]
fn rename_meeting(st: St, meeting_id: String, title: String) -> Res<Meeting> {
    let title = title.trim();
    if title.is_empty() {
        return Err("A meeting needs a title.".into());
    }
    st.engine()
        .store
        .lock()
        .unwrap()
        .set_title(&meeting_id, title)
}

/// The user's notes, verbatim Markdown lines, and the meeting time each was written.
#[tauri::command]
fn save_notes(st: St, meeting_id: String, raw: Vec<String>, at_ms: Vec<Option<u32>>) -> Res<()> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .set_notes_at(&meeting_id, raw, at_ms)
}

/// Fix a word in the transcript (NGR-143).
#[tauri::command]
fn edit_segment(st: St, meeting_id: String, segment_id: String, text: String) -> Res<Segment> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .edit_segment_text(&meeting_id, &segment_id, text.trim())
}

/// The recording's path, for `convertFileSrc` (the meetings folder is in the asset scope).
#[tauri::command]
fn audio_path(st: St, meeting_id: String) -> Res<Option<PathBuf>> {
    let path = st.engine().store.lock().unwrap().audio_path(&meeting_id)?;
    Ok(path.filter(|p| p.is_file()))
}

/// Adds the four sample meetings, for a tour before the first real one.
#[tauri::command]
fn add_demo_meetings(st: St) -> Res<()> {
    mirona_store::sample::seed(&mut st.engine().store.lock().unwrap())
}

#[derive(Serialize)]
struct TemplateInfo {
    id: String,
    name: String,
}

#[tauri::command]
fn list_templates(st: St) -> Res<Vec<TemplateInfo>> {
    let list = mirona_summary::templates(&st.engine().paths.templates)?;
    Ok(list
        .into_iter()
        .map(|t| TemplateInfo {
            id: t.id,
            name: t.name,
        })
        .collect())
}

/// Rewrite a meeting's summary and enhanced notes with `template` and the configured LLM
/// (NGR-146/147). Returns at once; the result arrives as `Summarized` or a `Notice`.
#[tauri::command]
fn regenerate(app: AppHandle, st: St, meeting_id: String, template: String) {
    let engine = st.engine();
    let tx = events(&app);
    std::thread::spawn(move || {
        let ev = match engine.summarize(&meeting_id, &template, &tx) {
            Ok(()) => PipelineEvent::Summarized { meeting_id },
            Err(e) => notice(&meeting_id, format!("Summary failed: {e}"), true),
        };
        tx.send(ev).ok();
    });
}

/// Pick an audio file and transcribe it into a new meeting, in the background. Progress
/// arrives as `Importing` events; the final `Notice` says it's done. One import at a time.
#[tauri::command]
async fn import_audio(app: AppHandle) -> Res<Option<String>> {
    use std::sync::atomic::Ordering;
    let st = app.state::<AppState>();
    if st.importing.swap(true, Ordering::SeqCst) {
        return Err("Already importing a file. Wait for it to finish.".into());
    }
    let picked = app
        .dialog()
        .file()
        .add_filter("Audio", &["wav", "mp3", "m4a", "flac", "aac", "mp4"])
        .blocking_pick_file()
        .and_then(|f| f.into_path().ok());
    let Some(file) = picked else {
        st.importing.store(false, Ordering::SeqCst);
        return Ok(None);
    };
    let name = file
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let engine = st.engine();
    let tx = events(&app);
    std::thread::spawn(move || {
        let ev = match engine.import(&file, &tx) {
            Ok(id) => notice(&id, format!("Imported {}", file.display()), false),
            Err(e) => notice("", format!("Import failed: {e}"), true),
        };
        app.state::<AppState>()
            .importing
            .store(false, Ordering::SeqCst);
        tx.send(ev).ok();
    });
    Ok(Some(name))
}

// --- Recording (NGR-142) ---

#[tauri::command]
async fn start_recording(app: AppHandle) -> Res<Meeting> {
    tauri::async_runtime::spawn_blocking(move || start(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn stop_recording(app: AppHandle) -> Res<String> {
    stop(&app)
}

#[tauri::command]
fn recording_state(st: St) -> RecState {
    st.rec_state()
}

fn start(app: &AppHandle) -> Res<Meeting> {
    let st = app.state::<AppState>();
    let engine = st.engine();
    let missing = mirona_engine::missing_models(&engine.config(), &engine.paths.models);
    if !missing.is_empty() {
        return Err(format!(
            "Speech models are missing ({}). Run: {}",
            missing.join(", "),
            fetch_command(&engine)
        ));
    }
    let mut slot = st.session.lock().unwrap();
    if slot.is_some() {
        return Err("Already recording.".into());
    }
    let session = engine.record(events(app))?;
    let meeting = engine.store.lock().unwrap().meeting(&session.id)?;
    *slot = Some(session);
    drop(slot);
    rec_changed(app);
    Ok(meeting)
}

/// Stop at once; flushing, saving and summarizing happen on a worker thread.
fn stop(app: &AppHandle) -> Res<String> {
    let st = app.state::<AppState>();
    let session = st.session.lock().unwrap().take().ok_or("Not recording.")?;
    let id = session.id.clone();
    st.processing.lock().unwrap().push(id.clone());
    rec_changed(app);
    let (engine, tx, app) = (st.engine(), events(app), app.clone());
    let mid = id.clone();
    std::thread::spawn(move || {
        if let Err(e) = session.stop(&engine, &tx) {
            tx.send(notice(&mid, format!("Recording stopped badly: {e}"), true))
                .ok();
        }
        let st = app.state::<AppState>();
        st.processing.lock().unwrap().retain(|p| *p != mid);
        rec_changed(&app);
    });
    Ok(id)
}

/// Tray and shortcut entry point: never blocks the main thread, errors become notices.
fn toggle(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let recording = app.state::<AppState>().session.lock().unwrap().is_some();
        let res = if recording {
            stop(&app).map(drop)
        } else {
            start(&app).map(drop)
        };
        if let Err(e) = res {
            app.emit("pipeline", notice("", e, true)).ok();
        }
    });
}

struct TrayItem(MenuItem<tauri::Wry>);

fn rec_changed(app: &AppHandle) {
    let state = app.state::<AppState>().rec_state();
    if let Some(item) = app.try_state::<TrayItem>() {
        let text = if state.recording.is_some() {
            "Stop recording"
        } else {
            "Start recording"
        };
        item.0.set_text(text).ok();
    }
    app.emit("recording", state).ok();
}

/// A sender whose events go out on the "pipeline" channel, until every clone is dropped.
fn events(app: &AppHandle) -> Sender<PipelineEvent> {
    let (tx, rx) = channel::<PipelineEvent>();
    let app = app.clone();
    std::thread::spawn(move || {
        for e in rx {
            app.emit("pipeline", e).ok();
        }
    });
    tx
}

fn notice(meeting_id: &str, message: String, error: bool) -> PipelineEvent {
    PipelineEvent::Notice {
        meeting_id: meeting_id.into(),
        message,
        error,
    }
}

// --- Permissions and setup (NGR-142) ---

fn permission(p: Permission) -> &'static str {
    match p {
        Permission::Granted => "granted",
        Permission::Denied => "denied",
        Permission::NotDetermined => "notDetermined",
    }
}

/// Shows the OS prompt (blocks until answered, so off the main thread). `kind`: mic | system.
#[tauri::command]
async fn request_permission(kind: String) -> Res<bool> {
    tauri::async_runtime::spawn_blocking(move || match kind.as_str() {
        "mic" => Ok(mirona_capture::request_mic_permission()),
        "system" => Ok(mirona_capture::request_system_audio_permission()),
        other => Err(format!("unknown permission {other}")),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    mic: &'static str,
    system_audio: &'static str,
    /// Model files the configured STT needs and the models folder lacks.
    missing_models: Vec<String>,
    fetch_command: String,
    /// Providers with an API key in the keychain.
    keys: Vec<String>,
    config_path: PathBuf,
    /// How MCP clients start the server: this app binary, run with `mcp`.
    cli: String,
    /// The record shortcut, as in config.toml.
    shortcut: String,
    errors: Vec<String>,
}

/// Providers whose API key the user brings, kept in the OS keychain (account = provider id).
/// Jev classifies segments (M2); `OpenAI` and Anthropic write summaries.
const KEY_PROVIDERS: [&str; 3] = ["openai", "anthropic", "jev"];

#[tauri::command]
fn status(st: St, keys: bool) -> Status {
    let engine = st.engine();
    let mut errors = st.startup_errors.clone();
    errors.extend(engine.store.lock().unwrap().load_errors().iter().cloned());
    Status {
        mic: permission(mirona_capture::mic_permission()),
        system_audio: permission(mirona_capture::system_audio_permission()),
        missing_models: mirona_engine::missing_models(&engine.config(), &engine.paths.models),
        fetch_command: fetch_command(&engine),
        // Only when asked: reading the keychain can prompt on unsigned dev builds.
        keys: if keys {
            KEY_PROVIDERS
                .into_iter()
                .filter(|p| matches!(keychain::api_key(p), Ok(Some(_))))
                .map(String::from)
                .collect()
        } else {
            vec![]
        },
        config_path: mirona_engine::config_path(&st.app_dir),
        cli: cli_path(),
        shortcut: engine.config().shortcut,
        errors,
    }
}

fn fetch_command(engine: &Engine) -> String {
    let whisper = mirona_engine::stage(&engine.config(), "stt", "parakeet") == "whisper";
    format!(
        "scripts/fetch-models.sh \"{}\"{}",
        engine.paths.models.display(),
        if whisper { " --whisper" } else { "" }
    )
}

/// The command MCP clients run: this app's own binary with `mcp` (see `main.rs`), so it works
/// without installing the CLI.
fn cli_path() -> String {
    std::env::current_exe().map_or_else(|_| "mirona".into(), |p| p.display().to_string())
}

/// Empty `key` removes it. Keys never touch `config.toml`.
#[tauri::command]
fn set_api_key(provider: String, key: String) -> Res<()> {
    if !KEY_PROVIDERS.contains(&provider.as_str()) {
        return Err(format!("no API key for {provider}"));
    }
    let key = key.trim();
    let res = if key.is_empty() {
        keychain::delete_api_key(&provider)
    } else {
        keychain::set_api_key(&provider, key)
    };
    res.map_err(|e| e.to_string())
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<PathBuf> {
    app.dialog().file().blocking_pick_folder()?.into_path().ok()
}

// --- Settings (NGR-150) ---

/// Everything the Models and MCP screen edits. Mirrors `config.toml`; folders are shown
/// resolved (the default when the config leaves them out).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    privacy: bool,
    /// parakeet | whisper
    stt: String,
    /// ollama | openai | anthropic
    llm: String,
    llm_model: String,
    llm_base_url: String,
    template: String,
    mcp_enabled: bool,
    /// Global start/stop shortcut, e.g. "CmdOrCtrl+Alt+Shift+R".
    shortcut: String,
    meetings_dir: PathBuf,
    models_dir: PathBuf,
    /// jev | local-llm | off
    #[serde(default)]
    classification: String,
    /// speakrs (`CoreML` when it can) | speakrs-cpu | off
    #[serde(default)]
    diarization: String,
    /// Question id → the user's threshold; questions left out use `threshold_defaults`.
    #[serde(default)]
    thresholds: BTreeMap<String, f32>,
    /// The defaults for the classifier in use (local or Jev); read-only.
    #[serde(default, skip_deserializing)]
    threshold_defaults: BTreeMap<String, f32>,
    /// Client name → short description, for the client question (NGR-157).
    #[serde(default)]
    clients: BTreeMap<String, String>,
}

/// Questions with a threshold the user can set (NGR-158).
const QUESTIONS: [&str; 11] = [
    "action",
    "decision",
    "unanswered",
    "smalltalk",
    "sensitive",
    "due",
    "owner",
    "meeting_type",
    "client",
    "follow_up",
    "coverage",
];

/// A shortcut the user can set: a valid accelerator with Cmd, Ctrl or Alt (Shift alone would
/// eat capital letters while typing).
fn parse_shortcut(s: &str) -> Res<Shortcut> {
    let s = s.trim();
    let sc: Shortcut = s
        .parse()
        .map_err(|e| format!("“{s}” is not a shortcut Mirona understands ({e})."))?;
    if !sc
        .mods
        .intersects(Modifiers::SUPER | Modifiers::CONTROL | Modifiers::ALT)
    {
        return Err(format!(
            "“{s}” needs Cmd, Ctrl or Alt/Option, so it doesn't fire while you type."
        ));
    }
    Ok(sc)
}

impl Settings {
    fn of(cfg: &Config, paths: &Paths) -> Self {
        Self {
            privacy: cfg.privacy,
            stt: mirona_engine::stage(cfg, "stt", "parakeet"),
            llm: mirona_engine::stage(cfg, "llm", "ollama"),
            llm_model: cfg.llm.model.clone(),
            llm_base_url: cfg.llm.base_url.clone(),
            template: cfg.template.clone(),
            mcp_enabled: cfg.mcp_enabled,
            shortcut: cfg.shortcut.clone(),
            meetings_dir: paths.meetings.clone(),
            models_dir: paths.models.clone(),
            classification: mirona_engine::stage(cfg, "classification", "jev"),
            diarization: mirona_engine::stage(cfg, "diarization", "speakrs"),
            thresholds: cfg.thresholds.clone(),
            threshold_defaults: {
                let local = cfg.privacy
                    || mirona_engine::stage(cfg, "classification", "jev") == "local-llm";
                QUESTIONS
                    .iter()
                    .map(|q| ((*q).to_string(), default_threshold(q, local)))
                    .collect()
            },
            clients: cfg.clients.clone(),
        }
    }

    /// `cfg` with these settings applied. The LLM goes through the privacy gate; folders equal
    /// to the defaults stay out of the file. Other config keys are kept.
    fn apply(&self, mut cfg: Config, defaults: &Paths) -> Res<Config> {
        if !["parakeet", "whisper"].contains(&self.stt.as_str()) {
            return Err(format!("unknown speech-to-text provider {}", self.stt));
        }
        let base = self.llm_base_url.trim();
        let (name, locality) = match self.llm.as_str() {
            "ollama" => ("Ollama", Locality::Local),
            "anthropic" => ("Anthropic", Locality::Cloud),
            "openai" => {
                let url = if base.is_empty() {
                    mirona_providers::openai::OPENAI_URL
                } else {
                    base
                };
                let local = mirona_providers::openai::is_loopback(url);
                let loc = if local {
                    Locality::Local
                } else {
                    Locality::Cloud
                };
                ("OpenAI-compatible", loc)
            }
            other => return Err(format!("unknown LLM provider {other}")),
        };
        let option = ProviderOption {
            id: self.llm.clone(),
            name: name.into(),
            locality,
        };
        check_privacy(&option, self.privacy).map_err(|e| e.to_string())?;
        if self.template.trim().is_empty() {
            return Err("pick a summary template".into());
        }
        parse_shortcut(&self.shortcut)?;
        if !["jev", "local-llm", "off"].contains(&self.classification.as_str()) {
            return Err(format!("unknown classifier {}", self.classification));
        }
        if !["speakrs", "speakrs-cpu", "off"].contains(&self.diarization.as_str()) {
            return Err(format!("unknown speaker detection {}", self.diarization));
        }
        for (q, t) in &self.thresholds {
            if !QUESTIONS.contains(&q.as_str()) || !(0.0..=1.0).contains(t) {
                return Err(format!("a threshold for {q} must be between 0 and 1"));
            }
        }
        let mut clients = BTreeMap::new();
        for (name, about) in &self.clients {
            let name = name.trim();
            if name.is_empty() || name == NO_CLIENT {
                return Err("Every client needs a name (and not “none”).".into());
            }
            clients.insert(name.to_string(), about.trim().to_string());
        }
        let dir = |p: &Path, default: &Path| (p != default).then(|| p.to_path_buf());
        cfg.privacy = self.privacy;
        cfg.providers.insert("stt".into(), self.stt.clone());
        cfg.providers.insert("llm".into(), self.llm.clone());
        cfg.llm.model = self.llm_model.trim().into();
        cfg.llm.base_url = base.into();
        cfg.template = self.template.trim().into();
        cfg.mcp_enabled = self.mcp_enabled;
        cfg.shortcut = self.shortcut.trim().into();
        cfg.meetings_dir = dir(&self.meetings_dir, &defaults.meetings);
        cfg.models_dir = dir(&self.models_dir, &defaults.models);
        cfg.providers
            .insert("classification".into(), self.classification.clone());
        cfg.providers
            .insert("diarization".into(), self.diarization.clone());
        cfg.thresholds.clone_from(&self.thresholds);
        cfg.clients = clients;
        Ok(cfg)
    }
}

#[tauri::command]
fn get_settings(st: St) -> Settings {
    let engine = st.engine();
    Settings::of(&engine.config(), &engine.paths)
}

#[tauri::command]
fn save_settings(app: AppHandle, st: St, settings: Settings) -> Res<Settings> {
    let engine = st.engine();
    let defaults = Paths::resolve(&Config::default(), &st.app_dir, &st.documents);
    let cfg = settings.apply(engine.config(), &defaults)?;
    let paths = Paths::resolve(&cfg, &st.app_dir, &st.documents);
    swap_shortcut(&app, &engine.config().shortcut, &cfg.shortcut)?;
    if paths.meetings == engine.paths.meetings && paths.models == engine.paths.models {
        engine.save_config(&st.app_dir, cfg.clone())?;
    } else {
        if st.session.lock().unwrap().is_some() {
            return Err("Stop the recording before moving folders.".into());
        }
        // Open the new folder first, so a bad pick never reaches config.toml.
        let store = Store::open(&paths.meetings)?;
        cfg.save(&mirona_engine::config_path(&st.app_dir))?;
        allow_audio(&app, &paths.meetings);
        let next = Engine::new(Arc::new(Mutex::new(store)), cfg.clone(), paths.clone());
        *st.engine.write().unwrap() = Arc::new(next);
    }
    Ok(Settings::of(&cfg, &paths))
}

/// Move the record shortcut from `old` to `new` right away. If `new` can't be registered
/// (another app owns it), `old` stays and the error says so.
fn swap_shortcut(app: &AppHandle, old: &str, new: &str) -> Res<()> {
    let next = parse_shortcut(new)?;
    let prev = parse_shortcut(old).ok();
    if prev.is_some_and(|p| p.id() == next.id()) {
        return Ok(());
    }
    let gs = app.global_shortcut();
    if let Some(p) = prev {
        gs.unregister(p).ok();
    }
    if let Err(e) = gs.register(next) {
        if let Some(p) = prev {
            gs.register(p).ok();
        }
        return Err(format!(
            "Couldn't use {new} as the record shortcut: another app or the system may already \
             use it ({e}). Still using {old}."
        ));
    }
    if let Some(item) = app.try_state::<TrayItem>() {
        item.0.set_accelerator(Some(new)).ok();
    }
    Ok(())
}

fn allow_audio(app: &AppHandle, meetings: &Path) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(meetings, true) {
        eprintln!("mirona: audio playback: {e}");
    }
}

/// The engine at the configured folders. A broken `config.toml` is reported and the app runs
/// on defaults (the file is left alone until the user saves settings).
fn open_engine(app_dir: &Path, documents: &Path) -> Res<(Engine, Vec<String>)> {
    match Engine::open(app_dir, documents) {
        Ok(e) => Ok((e, vec![])),
        Err(err) => {
            let cfg = Config::default();
            let paths = Paths::resolve(&cfg, app_dir, documents);
            let store = Store::open(&paths.meetings)?;
            let engine = Engine::new(Arc::new(Mutex::new(store)), cfg, paths);
            Ok((engine, vec![format!("Using default settings: {err}")]))
        }
    }
}

fn tray(app: &tauri::App, shortcut: &str) -> tauri::Result<()> {
    // An accelerator the menu can't show must not cost the tray icon.
    let accel = parse_shortcut(shortcut).is_ok().then_some(shortcut);
    let toggle_item = MenuItem::with_id(app, "toggle", "Start recording", true, accel)?;
    let show = MenuItem::with_id(app, "show", "Open Mirona", true, None::<&str>)?;
    let quit = PredefinedMenuItem::quit(app, Some("Quit Mirona"))?;
    let menu = Menu::with_items(app, &[&toggle_item, &show, &quit])?;
    let icon = if cfg!(target_os = "macos") {
        tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?
    } else {
        tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?
    };
    TrayIconBuilder::with_id("mirona")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Mirona")
        .menu(&menu)
        .on_menu_event(|app, ev| match ev.id.as_ref() {
            "toggle" => toggle(app),
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    w.show().ok();
                    w.set_focus().ok();
                }
            }
            _ => {}
        })
        .build(app)?;
    app.manage(TrayItem(toggle_item));
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::too_many_lines)] // mostly the list of commands
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, ev| {
                    if ev.state == ShortcutState::Pressed {
                        toggle(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let app_dir = mirona_engine::app_dir();
            let documents = mirona_engine::documents_dir();
            let (engine, mut errors) = open_engine(&app_dir, &documents)?;
            allow_audio(app.handle(), &engine.paths.meetings);
            let shortcut = engine.config().shortcut;
            let registered = parse_shortcut(&shortcut)
                .and_then(|sc| app.global_shortcut().register(sc).map_err(|e| e.to_string()));
            if let Err(e) = registered {
                errors.push(format!(
                    "Record shortcut {shortcut} is unavailable ({e}). Pick another in Models and MCP."
                ));
            }
            if let Err(e) = tray(app, &shortcut) {
                errors.push(format!("No menu bar icon: {e}"));
            }
            for e in &errors {
                eprintln!("mirona: {e}");
            }
            app.manage(AppState {
                app_dir,
                documents,
                engine: RwLock::new(Arc::new(engine)),
                session: Mutex::new(None),
                processing: Mutex::new(vec![]),
                importing: false.into(),
                startup_errors: errors,
            });
            // Recordings a crash left as WAV get compressed in the background.
            let engine = app.state::<AppState>().engine();
            std::thread::spawn(move || {
                for e in engine.compress_recordings() {
                    eprintln!("mirona: compressing a recording: {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_meetings,
            refresh_store,
            get_meeting,
            list_action_items,
            toggle_action_item,
            list_people,
            enroll_speaker,
            name_speaker,
            merge_people,
            delete_voiceprint,
            correct_answer,
            confirm_client,
            review,
            resolve_review,
            set_owner,
            rerun_diarization,
            rerun_classification,
            rename_meeting,
            save_notes,
            edit_segment,
            audio_path,
            add_demo_meetings,
            list_templates,
            regenerate,
            import_audio,
            start_recording,
            stop_recording,
            recording_state,
            request_permission,
            status,
            set_api_key,
            pick_folder,
            get_settings,
            save_settings,
        ])
        // Closing the window keeps Mirona in the menu bar, so recording can still be started
        // from the tray or the shortcut. Quit from the tray menu.
        .on_window_event(|w, e| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                w.hide().ok();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while starting Mirona")
        .run(|app, e| {
            // Clicking the Dock icon brings the hidden window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = e
                && let Some(w) = app.get_webview_window("main")
            {
                w.show().ok();
                w.set_focus().ok();
            }
            let _ = (app, e);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Paths {
        Paths::resolve(
            &Config::default(),
            Path::new("/app"),
            Path::new("/home/me/Documents"),
        )
    }

    #[test]
    fn settings_round_trip_and_keep_default_folders_out_of_the_file() {
        let d = defaults();
        let s = Settings::of(&Config::default(), &d);
        assert_eq!(s.meetings_dir, PathBuf::from("/home/me/Documents/Mirona"));
        let cfg = s.apply(Config::default(), &d).unwrap();
        assert_eq!(cfg.meetings_dir, None, "default folder is not written");
        assert_eq!(Settings::of(&cfg, &d), s);

        let mut moved = s.clone();
        moved.meetings_dir = "/elsewhere".into();
        moved.stt = "whisper".into();
        moved.mcp_enabled = false;
        let cfg = moved.apply(cfg, &d).unwrap();
        assert_eq!(cfg.meetings_dir, Some("/elsewhere".into()));
        assert_eq!(cfg.providers["stt"], "whisper");
        assert!(!cfg.mcp_enabled);
        let paths = Paths::resolve(&cfg, Path::new("/app"), Path::new("/home/me/Documents"));
        assert_eq!(Settings::of(&cfg, &paths), moved);
    }

    #[test]
    fn classifier_diarization_thresholds_and_clients_are_settings() {
        let d = defaults();
        let mut s = Settings::of(&Config::default(), &d);
        assert_eq!(
            (s.classification.as_str(), s.diarization.as_str()),
            ("jev", "speakrs")
        );
        // Privacy mode: the local fallback's stricter defaults.
        assert!((s.threshold_defaults["action"] - 0.99).abs() < 1e-6);
        s.classification = "local-llm".into();
        s.diarization = "speakrs-cpu".into();
        s.thresholds.insert("action".into(), 0.8);
        s.clients
            .insert(" Northwind Bank ".into(), " contact: Karen ".into());
        let cfg = s.apply(Config::default(), &d).unwrap();
        assert_eq!(cfg.providers["classification"], "local-llm");
        assert_eq!(cfg.providers["diarization"], "speakrs-cpu");
        assert!((cfg.threshold("action", true) - 0.8).abs() < 1e-6);
        assert_eq!(cfg.clients["Northwind Bank"], "contact: Karen");
        let back = Settings::of(&cfg, &d);
        assert_eq!(back.thresholds, cfg.thresholds);

        for bad in [
            Settings {
                classification: "kev".into(),
                ..back.clone()
            },
            Settings {
                diarization: "pyannote".into(),
                ..back.clone()
            },
            Settings {
                thresholds: [("action".into(), 1.5)].into(),
                ..back.clone()
            },
            Settings {
                thresholds: [("nope".into(), 0.5)].into(),
                ..back.clone()
            },
            Settings {
                clients: [(" ".into(), String::new())].into(),
                ..back.clone()
            },
        ] {
            assert!(bad.apply(cfg.clone(), &d).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn review_items_resolve_in_one_click() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        mirona_store::sample::seed(&mut s).unwrap();
        let id = "claims-desk";
        let r = |question: &str, value: &str| Review {
            segment_id: None,
            at_ms: None,
            question: question.into(),
            value: value.into(),
            probability: 0.5,
            threshold: 0.6,
        };
        assert!(!resolve(&mut s, id, &r("meeting_type", "standup"), true).unwrap());
        let a = s.meeting(id).unwrap().answers.unwrap();
        assert_eq!(a.meeting_type.value, "standup");
        assert!(a.meeting_type.confirmed);

        assert!(!resolve(&mut s, id, &r("client", "none"), true).unwrap());
        assert_eq!(s.meeting(id).unwrap().client, "");
        resolve(&mut s, id, &r("client", "Corvo"), true).unwrap();
        assert_eq!(s.meeting(id).unwrap().client, "Corvo");

        // Follow-up: a confirmed yes asks for a draft only when there is none.
        assert!(
            !resolve(&mut s, id, &r("follow_up", "yes"), true).unwrap(),
            "sample has one"
        );
        resolve(&mut s, id, &r("follow_up", "yes"), false).unwrap();
        assert!(!s.meeting(id).unwrap().answers.unwrap().follow_up);

        // An action item waiting for review, confirmed through its segment.
        let item = s.review(id).unwrap();
        let item = item.iter().find(|x| x.question == "action_item").unwrap();
        resolve(&mut s, id, item, true).unwrap();
        assert!(!s.action_items().iter().any(|a| a.needs_review));
        assert!(resolve(&mut s, id, &r("tone", ""), true).is_err());
    }

    #[test]
    fn shortcuts_are_validated() {
        assert!(parse_shortcut(mirona_providers::config::DEFAULT_SHORTCUT).is_ok());
        assert!(parse_shortcut(" Ctrl+Shift+F9 ").is_ok());
        assert!(
            parse_shortcut("Alt+KeyM").is_ok(),
            "KeyboardEvent.code names work"
        );
        for bad in [
            "",
            "R",
            "Shift+R",
            "CmdOrCtrl+",
            "Ctrl+Nope",
            "Ctrl+R+Shift",
        ] {
            assert!(parse_shortcut(bad).is_err(), "{bad:?}");
        }
        let d = defaults();
        let mut s = Settings::of(&Config::default(), &d);
        assert_eq!(s.shortcut, "CmdOrCtrl+Alt+Shift+R");
        s.shortcut = "Shift+R".into();
        assert!(s.apply(Config::default(), &d).is_err());
        s.shortcut = " CmdOrCtrl+Shift+F9 ".into();
        let cfg = s.apply(Config::default(), &d).unwrap();
        assert_eq!(cfg.shortcut, "CmdOrCtrl+Shift+F9");
    }

    #[test]
    fn cloud_llms_go_through_the_privacy_gate() {
        let d = defaults();
        let mut s = Settings::of(&Config::default(), &d);
        assert!(s.privacy, "privacy on by default");
        s.llm = "anthropic".into();
        assert!(s.apply(Config::default(), &d).is_err());
        s.llm = "openai".into();
        assert!(s.apply(Config::default(), &d).is_err(), "api.openai.com");
        s.llm_base_url = "http://localhost:1234/v1".into();
        assert!(s.apply(Config::default(), &d).is_ok(), "LM Studio is local");
        s.privacy = false;
        s.llm = "anthropic".into();
        let cfg = s.apply(Config::default(), &d).unwrap();
        assert_eq!(cfg.providers["llm"], "anthropic");
        s.llm = "gpt-5".into();
        assert!(s.apply(Config::default(), &d).is_err(), "unknown provider");
    }
}
