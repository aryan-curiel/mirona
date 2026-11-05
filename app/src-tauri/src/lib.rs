//! Tauri shell: typed commands over `mirona-store` and a fake capture pipeline that emits
//! `PipelineEvent`s on the "pipeline" channel while recording.

use mirona_core::{
    ActionItem, AgentCall, Correction, McpStatus, Meeting, MeetingSummary, PipelineEvent, Speaker,
    Stage, Tag,
};
use mirona_providers::config::Config;
use mirona_store::{Store, sample};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct Recorder {
    on: Arc<AtomicBool>,
    meeting_id: Mutex<Option<String>>,
    started: Mutex<Option<Instant>>,
}

type Db<'a> = State<'a, Mutex<Store>>;

/// Where `config.toml` lives (the OS app-config dir).
struct ConfigPath(PathBuf);

fn save_config(store: &Store, path: &ConfigPath) -> Res<()> {
    store.config().save(&path.0)
}
type Res<T> = Result<T, String>;

#[tauri::command]
fn list_meetings(db: Db) -> Vec<MeetingSummary> {
    db.lock().unwrap().list_meetings()
}

#[tauri::command]
fn get_meeting(db: Db, id: String) -> Res<Meeting> {
    db.lock().unwrap().meeting(&id)
}

#[tauri::command]
fn list_action_items(db: Db) -> Vec<ActionItem> {
    db.lock().unwrap().action_items.clone()
}

#[tauri::command]
fn toggle_action_item(db: Db, id: String) -> Res<ActionItem> {
    db.lock().unwrap().toggle_action_item(&id)
}

#[tauri::command]
fn list_people(db: Db) -> Vec<Speaker> {
    db.lock().unwrap().people.clone()
}

#[tauri::command]
fn enroll_speaker(db: Db, speaker_id: String, name: String) -> Res<Speaker> {
    db.lock().unwrap().enroll_speaker(&speaker_id, &name)
}

#[tauri::command]
fn correct_answer(db: Db, correction: Correction) -> Res<usize> {
    let mut s = db.lock().unwrap();
    s.correct(correction)?;
    Ok(s.corrections.len())
}

#[tauri::command]
fn confirm_client(db: Db, meeting_id: String, client: String) -> Res<Meeting> {
    db.lock().unwrap().confirm_client(&meeting_id, &client)
}

#[tauri::command]
fn save_notes(db: Db, meeting_id: String, raw: Vec<String>) -> Res<()> {
    db.lock().unwrap().set_notes(&meeting_id, raw)
}

#[tauri::command]
fn list_stages(db: Db) -> Vec<Stage> {
    db.lock().unwrap().stages.clone()
}

#[tauri::command]
fn select_provider(
    db: Db,
    cfg: State<ConfigPath>,
    stage_id: String,
    option_id: String,
) -> Res<Stage> {
    let mut s = db.lock().unwrap();
    let stage = s.select_provider(&stage_id, &option_id)?;
    save_config(&s, &cfg)?;
    Ok(stage)
}

#[tauri::command]
fn get_privacy(db: Db) -> bool {
    db.lock().unwrap().privacy
}

#[tauri::command]
fn set_privacy(db: Db, cfg: State<ConfigPath>, on: bool) -> Res<Vec<Stage>> {
    let mut s = db.lock().unwrap();
    let stages = s.set_privacy(on);
    save_config(&s, &cfg)?;
    Ok(stages)
}

#[tauri::command]
fn get_mcp(db: Db) -> McpStatus {
    db.lock().unwrap().mcp.clone()
}

#[tauri::command]
fn set_mcp_running(db: Db, running: bool) -> McpStatus {
    let mut s = db.lock().unwrap();
    s.mcp.running = running;
    s.mcp.clone()
}

#[tauri::command]
fn start_recording(app: AppHandle, db: Db, rec: State<Recorder>) -> Res<Meeting> {
    if rec.on.swap(true, Ordering::SeqCst) {
        return Err("already recording".into());
    }
    let id = mirona_store::new_id();
    let meeting = db.lock().unwrap().start_meeting(&id);
    *rec.meeting_id.lock().unwrap() = Some(id.clone());
    *rec.started.lock().unwrap() = Some(Instant::now());
    let on = rec.on.clone();
    std::thread::spawn(move || capture_loop(&app, &on, &id));
    Ok(meeting)
}

/// Fake capture: audio levels every 120 ms, a new transcribed + classified segment every ~3.5 s.
fn capture_loop(app: &AppHandle, on: &AtomicBool, id: &str) {
    let started = Instant::now();
    let script = sample::live_script();
    let mut next = 0;
    let mut tick: u32 = 0;
    while on.load(Ordering::SeqCst) {
        #[allow(clippy::cast_possible_truncation)]
        let elapsed_ms = started.elapsed().as_millis() as u32;
        tick += 1;
        #[allow(clippy::cast_precision_loss)]
        let levels = (0..9)
            .map(|i| ((tick as f32 * 0.7 + i as f32 * 1.3).sin().abs() * 0.8 + 0.2).min(1.0))
            .collect();
        app.emit(
            "pipeline",
            PipelineEvent::Captured {
                meeting_id: id.into(),
                elapsed_ms,
                levels,
            },
        )
        .ok();

        if next < script.len() && elapsed_ms > 2500 + u32::try_from(next).unwrap() * 3500 {
            let (who, text, p, tag) = script[next];
            let mut seg = sample::seg(&format!("{id}-{next}"), who, "00:00", text, p, vec![], &[]);
            seg.start_ms = elapsed_ms;
            if p[0] >= sample::THRESHOLD {
                seg.owners = mirona_store::live_owner("Aryan");
            }
            let pushed = app
                .state::<Mutex<Store>>()
                .lock()
                .unwrap()
                .push_segment(id, seg.clone());
            if let Ok(speaker) = pushed {
                app.emit(
                    "pipeline",
                    PipelineEvent::Transcribed {
                        meeting_id: id.into(),
                        segment: seg.clone(),
                        speaker,
                    },
                )
                .ok();
                // Kev answers a beat after the text lands.
                if let Some((kind, label)) = tag {
                    std::thread::sleep(Duration::from_millis(700));
                    let tags = vec![Tag {
                        kind,
                        label: label.into(),
                    }];
                    let db = app.state::<Mutex<Store>>();
                    let mut s = db.lock().unwrap();
                    if let Some(m) = s.meetings.iter_mut().find(|m| m.id == id)
                        && let Some(sg) = m.segments.iter_mut().find(|x| x.id == seg.id)
                    {
                        sg.tags.clone_from(&tags);
                    }
                    app.emit(
                        "pipeline",
                        PipelineEvent::Classified {
                            meeting_id: id.into(),
                            segment_id: seg.id,
                            tags,
                        },
                    )
                    .ok();
                }
            }
            next += 1;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

#[tauri::command]
fn stop_recording(app: AppHandle, rec: State<Recorder>) -> Res<String> {
    rec.on.store(false, Ordering::SeqCst);
    let id = rec
        .meeting_id
        .lock()
        .unwrap()
        .take()
        .ok_or("not recording")?;
    #[allow(clippy::cast_possible_truncation)]
    let elapsed = rec
        .started
        .lock()
        .unwrap()
        .take()
        .map_or(0, |s| s.elapsed().as_millis() as u32);
    let mid = id.clone();
    std::thread::spawn(move || {
        app.emit(
            "pipeline",
            PipelineEvent::Diarized {
                meeting_id: mid.clone(),
                speakers: 2,
            },
        )
        .ok();
        std::thread::sleep(Duration::from_millis(1600)); // "processing"
        let db = app.state::<Mutex<Store>>();
        let mut s = db.lock().unwrap();
        if s.finish_meeting(&mid, elapsed).is_ok() {
            s.log_agent_call(AgentCall {
                client: "Claude Code".into(),
                ago: "just now".into(),
                call: "get_meeting(\"Evaluation dashboard follow-up\")".into(),
                result: "Summary · 1 decision · 1 action item".into(),
            });
            drop(s);
            app.emit("pipeline", PipelineEvent::Summarized { meeting_id: mid })
                .ok();
        }
    });
    Ok(id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("config.toml");
            let mut store = Store::sample();
            // A bad config must not stop the app; keep safe defaults and say why.
            if let Err(e) = Config::load(&path).and_then(|c| store.apply_config(&c)) {
                eprintln!("mirona: ignoring {}: {e}", path.display());
                store = Store::sample();
            }
            app.manage(Mutex::new(store));
            app.manage(ConfigPath(path));
            Ok(())
        })
        .manage(Recorder::default())
        .invoke_handler(tauri::generate_handler![
            list_meetings,
            get_meeting,
            list_action_items,
            toggle_action_item,
            list_people,
            enroll_speaker,
            correct_answer,
            confirm_client,
            save_notes,
            list_stages,
            select_provider,
            get_privacy,
            set_privacy,
            get_mcp,
            set_mcp_running,
            start_recording,
            stop_recording,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mirona");
}
