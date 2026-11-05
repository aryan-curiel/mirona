//! Tauri shell: typed commands over `mirona-store` and a fake capture pipeline that emits
//! `PipelineEvent`s on the "pipeline" channel while recording.

use mirona_core::{
    ActionItem, AgentCall, Correction, Locality, McpStatus, Meeting, MeetingSummary, PipelineEvent,
    Speaker, Stage, Tag,
};
use mirona_providers::check_privacy;
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
type Mdl<'a> = State<'a, Mutex<Models>>;

/// Where `config.toml` lives (the OS app-config dir).
struct ConfigPath(PathBuf);

fn save_config(models: &Models, path: &ConfigPath) -> Res<()> {
    models.config().save(&path.0)
}
type Res<T> = Result<T, String>;

/// Privacy mode, the provider picked per stage, and MCP status (moved out of the store).
/// ponytail: sample stages + fake MCP log for the Models screen; NGR-150 moves this to settings.
struct Models {
    stages: Vec<Stage>,
    mcp: McpStatus,
    privacy: bool,
}

impl Models {
    fn new() -> Self {
        let mut m = Self {
            stages: sample::stages(),
            mcp: sample::mcp(),
            privacy: false,
        };
        m.set_privacy(true);
        m
    }

    /// Apply a saved config. Every choice goes through the privacy gate, so a config that
    /// picks a cloud provider while privacy is on is rejected, not silently obeyed.
    fn apply_config(&mut self, cfg: &Config) -> Res<()> {
        self.set_privacy(cfg.privacy);
        for (stage, provider) in &cfg.providers {
            self.select_provider(stage, provider)
                .map_err(|e| format!("config: {stage} = {provider}: {e}"))?;
        }
        Ok(())
    }

    fn config(&self) -> Config {
        Config {
            privacy: self.privacy,
            providers: self
                .stages
                .iter()
                .map(|s| (s.id.clone(), s.selected.clone()))
                .collect(),
        }
    }

    /// Turning privacy on moves every stage back to a local provider.
    fn set_privacy(&mut self, on: bool) -> Vec<Stage> {
        self.privacy = on;
        if on {
            for st in &mut self.stages {
                let cloud = st
                    .options
                    .iter()
                    .any(|o| o.id == st.selected && o.locality == Locality::Cloud);
                if cloud {
                    st.selected = st
                        .options
                        .iter()
                        .find(|o| o.locality == Locality::Local)
                        .unwrap()
                        .id
                        .clone();
                }
            }
        }
        self.stages.clone()
    }

    fn select_provider(&mut self, stage_id: &str, option_id: &str) -> Res<Stage> {
        let privacy = self.privacy;
        let st = self
            .stages
            .iter_mut()
            .find(|s| s.id == stage_id)
            .ok_or("no stage")?;
        let opt = st
            .options
            .iter()
            .find(|o| o.id == option_id)
            .ok_or("no provider")?;
        check_privacy(opt, privacy).map_err(|e| e.to_string())?;
        st.selected = option_id.into();
        Ok(st.clone())
    }

    fn llm_name(&self) -> String {
        let st = self.stages.iter().find(|s| s.id == "llm").unwrap();
        st.options
            .iter()
            .find(|o| o.id == st.selected)
            .unwrap()
            .name
            .clone()
    }

    fn log_agent_call(&mut self, call: AgentCall) {
        self.mcp.log.insert(0, call);
        self.mcp.log.truncate(6);
    }
}

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
    db.lock().unwrap().action_items()
}

#[tauri::command]
fn toggle_action_item(db: Db, id: String) -> Res<ActionItem> {
    db.lock().unwrap().toggle_action_item(&id)
}

#[tauri::command]
fn list_people(db: Db) -> Vec<Speaker> {
    db.lock().unwrap().people().to_vec()
}

#[tauri::command]
fn enroll_speaker(db: Db, speaker_id: String, name: String) -> Res<Speaker> {
    db.lock().unwrap().enroll_speaker(&speaker_id, &name)
}

#[tauri::command]
fn correct_answer(db: Db, correction: Correction) -> Res<usize> {
    let mut s = db.lock().unwrap();
    s.correct(correction)?;
    Ok(s.corrections().len())
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
fn list_stages(models: Mdl) -> Vec<Stage> {
    models.lock().unwrap().stages.clone()
}

#[tauri::command]
fn select_provider(
    models: Mdl,
    cfg: State<ConfigPath>,
    stage_id: String,
    option_id: String,
) -> Res<Stage> {
    let mut s = models.lock().unwrap();
    let stage = s.select_provider(&stage_id, &option_id)?;
    save_config(&s, &cfg)?;
    Ok(stage)
}

#[tauri::command]
fn get_privacy(models: Mdl) -> bool {
    models.lock().unwrap().privacy
}

#[tauri::command]
fn set_privacy(models: Mdl, cfg: State<ConfigPath>, on: bool) -> Res<Vec<Stage>> {
    let mut s = models.lock().unwrap();
    let stages = s.set_privacy(on);
    save_config(&s, &cfg)?;
    Ok(stages)
}

#[tauri::command]
fn get_mcp(models: Mdl) -> McpStatus {
    models.lock().unwrap().mcp.clone()
}

#[tauri::command]
fn set_mcp_running(models: Mdl, running: bool) -> McpStatus {
    let mut s = models.lock().unwrap();
    s.mcp.running = running;
    s.mcp.clone()
}

#[tauri::command]
fn start_recording(app: AppHandle, db: Db, rec: State<Recorder>) -> Res<Meeting> {
    if rec.on.swap(true, Ordering::SeqCst) {
        return Err("already recording".into());
    }
    let id = mirona_store::new_id();
    let meeting = match db.lock().unwrap().start_meeting(&id) {
        Ok(m) => m,
        Err(e) => {
            rec.on.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
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
                    let mut tagged = seg.clone();
                    tagged.tags.clone_from(&tags);
                    let db = app.state::<Mutex<Store>>();
                    if let Err(e) = db.lock().unwrap().update_segment(id, tagged) {
                        eprintln!("mirona: {e}");
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
        let models = app.state::<Mutex<Models>>();
        let llm = models.lock().unwrap().llm_name();
        let db = app.state::<Mutex<Store>>();
        let mut s = db.lock().unwrap();
        if sample::demo_finish(&mut s, &mid, elapsed, &llm).is_ok() {
            models.lock().unwrap().log_agent_call(AgentCall {
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
            let mut models = Models::new();
            // A bad config must not stop the app; keep safe defaults and say why.
            if let Err(e) = Config::load(&path).and_then(|c| models.apply_config(&c)) {
                eprintln!("mirona: ignoring {}: {e}", path.display());
                models = Models::new();
            }
            // ponytail: fixed root and demo seed until NGR-150 makes the folder a setting.
            let root = app.path().document_dir()?.join("Mirona");
            let mut store = Store::open(&root)?;
            for e in store.load_errors() {
                eprintln!("mirona: {e}");
            }
            if store.list_meetings().is_empty() {
                sample::seed(&mut store)?;
            }
            app.manage(Mutex::new(store));
            app.manage(Mutex::new(models));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_gate() {
        let mut s = Models::new();
        assert!(
            s.select_provider("llm", "anthropic").is_err(),
            "cloud blocked in privacy mode"
        );
        s.set_privacy(false);
        s.select_provider("llm", "anthropic").unwrap();
        s.set_privacy(true);
        assert_eq!(s.stages[3].selected, "ollama", "privacy on resets to local");
    }

    #[test]
    fn config_picks_providers_through_the_privacy_gate() {
        let mut s = Models::new();
        let sel = |s: &Models, id: &str| {
            s.stages
                .iter()
                .find(|x| x.id == id)
                .unwrap()
                .selected
                .clone()
        };
        assert_eq!(
            sel(&s, "classification"),
            "local-llm",
            "privacy on by default"
        );

        let mut cfg = Config::default();
        cfg.providers.insert("classification".into(), "jev".into());
        assert!(
            s.apply_config(&cfg).is_err(),
            "cloud refused while privacy is on"
        );

        cfg.privacy = false;
        s.apply_config(&cfg).unwrap();
        assert_eq!(sel(&s, "classification"), "jev");
        assert_eq!(s.config().providers["classification"], "jev");

        cfg.providers.insert("stt".into(), "nope".into());
        assert!(s.apply_config(&cfg).is_err(), "unknown provider");
    }
}
