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
    ActionItem, Correction, Locality, Meeting, MeetingSummary, PipelineEvent, ProviderOption,
    Segment, Speaker,
};
use mirona_engine::{Engine, Paths, Session};
use mirona_providers::config::Config;
use mirona_providers::{check_privacy, keychain};
use mirona_store::Store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

type Res<T> = Result<T, String>;

/// Start/stop recording from anywhere. Cmd+Shift+R is a browser's hard reload, so one more
/// modifier keeps it out of other apps' way. ponytail: fixed; make it a setting when asked.
const SHORTCUT: &str = "CmdOrCtrl+Alt+Shift+R";

struct AppState {
    app_dir: PathBuf,
    documents: PathBuf,
    /// Swapped when the meetings or models folder moves (never while recording).
    engine: RwLock<Arc<Engine>>,
    session: Mutex<Option<Session>>,
    /// Meetings whose recording stopped and that are being summarized.
    processing: Mutex<Vec<String>>,
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

#[tauri::command]
fn list_people(st: St) -> Vec<Speaker> {
    st.engine().store.lock().unwrap().people().to_vec()
}

#[tauri::command]
fn enroll_speaker(st: St, speaker_id: String, name: String) -> Res<Speaker> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .enroll_speaker(&speaker_id, &name)
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
        let ev = match engine.summarize(&meeting_id, &template) {
            Ok(()) => PipelineEvent::Summarized { meeting_id },
            Err(e) => notice(&meeting_id, format!("Summary failed: {e}"), true),
        };
        tx.send(ev).ok();
    });
}

/// Pick an audio file and transcribe it into a new meeting, in the background.
#[tauri::command]
async fn import_audio(app: AppHandle) -> Option<String> {
    let file = app
        .dialog()
        .file()
        .add_filter("Audio", &["wav", "mp3", "m4a", "flac", "aac", "mp4"])
        .blocking_pick_file()?
        .into_path()
        .ok()?;
    let name = file.file_name()?.to_string_lossy().to_string();
    let engine = app.state::<AppState>().engine();
    let tx = events(&app);
    std::thread::spawn(move || {
        let ev = match engine.import(&file, &tx) {
            Ok(id) => notice(&id, format!("Imported {}", file.display()), false),
            Err(e) => notice("", format!("Import failed: {e}"), true),
        };
        tx.send(ev).ok();
    });
    Some(name)
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
    /// How MCP clients start the server: the `mirona` CLI.
    cli: String,
    shortcut: &'static str,
    errors: Vec<String>,
}

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
            ["openai", "anthropic"]
                .into_iter()
                .filter(|p| matches!(keychain::api_key(p), Ok(Some(_))))
                .map(String::from)
                .collect()
        } else {
            vec![]
        },
        config_path: mirona_engine::config_path(&st.app_dir),
        cli: cli_path(),
        shortcut: SHORTCUT,
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

/// The `mirona` CLI next to the app binary when bundled, else whatever is on PATH.
fn cli_path() -> String {
    let exe = if cfg!(windows) {
        "mirona.exe"
    } else {
        "mirona"
    };
    std::env::current_exe()
        .ok()
        .and_then(|p| Some(p.parent()?.join(exe)))
        .filter(|p| p.is_file())
        .map_or_else(|| "mirona".into(), |p| p.display().to_string())
}

/// Empty `key` removes it. Keys never touch `config.toml`.
#[tauri::command]
fn set_api_key(provider: String, key: String) -> Res<()> {
    if !["openai", "anthropic"].contains(&provider.as_str()) {
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
    meetings_dir: PathBuf,
    models_dir: PathBuf,
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
            meetings_dir: paths.meetings.clone(),
            models_dir: paths.models.clone(),
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
        let dir = |p: &Path, default: &Path| (p != default).then(|| p.to_path_buf());
        cfg.privacy = self.privacy;
        cfg.providers.insert("stt".into(), self.stt.clone());
        cfg.providers.insert("llm".into(), self.llm.clone());
        cfg.llm.model = self.llm_model.trim().into();
        cfg.llm.base_url = base.into();
        cfg.template = self.template.trim().into();
        cfg.mcp_enabled = self.mcp_enabled;
        cfg.meetings_dir = dir(&self.meetings_dir, &defaults.meetings);
        cfg.models_dir = dir(&self.models_dir, &defaults.models);
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

fn tray(app: &tauri::App) -> tauri::Result<()> {
    let toggle_item = MenuItem::with_id(app, "toggle", "Start recording", true, Some(SHORTCUT))?;
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
            if let Err(e) = app.global_shortcut().register(SHORTCUT) {
                errors.push(format!("Shortcut {SHORTCUT} is taken: {e}"));
            }
            if let Err(e) = tray(app) {
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
                startup_errors: errors,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_meetings,
            get_meeting,
            list_action_items,
            toggle_action_item,
            list_people,
            enroll_speaker,
            correct_answer,
            confirm_client,
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
        .run(tauri::generate_context!())
        .expect("error while running Mirona");
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
