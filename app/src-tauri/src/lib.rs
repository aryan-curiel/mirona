//! Tauri shell (NGR-141/142/143/144/150): typed commands over `mirona-engine` and the meeting
//! store at the configured folder, recording from the window, the tray and a global shortcut,
//! and settings that read and write `config.toml`.
//!
//! Call detection (NGR-170): a worker polls which call app uses the mic and, when one starts,
//! suggests recording on the "call" channel and in a system notification. It records on its
//! own only for tags the user opted in (`auto_record`, the tag's name in the calendar title).
//!
//! Long work (stopping + summarizing, import, regenerate) runs on its own thread and reports
//! on the "pipeline" event channel; "recording" carries the recorder state (`RecState`).

// Tauri hands commands their arguments by value.
#![allow(clippy::needless_pass_by_value)]

use mirona_capture::Permission;
use mirona_core::{
    ActionItem, Correction, Locality, Meeting, MeetingAnswers, MeetingSummary, PipelineEvent,
    ProviderOption, Review, Segment, Speaker, SpeakerEdit,
};
use mirona_engine::{Engine, Llm, Paths, Session};
use mirona_providers::config::{Config, default_floor, default_threshold};
use mirona_providers::{check_privacy, keychain};
use mirona_store::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

mod tray_icon;
mod update;

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
    /// Models are downloading (one run at a time), and the flag that stops it (NGR-174).
    downloading: std::sync::atomic::AtomicBool,
    cancel_download: std::sync::atomic::AtomicBool,
    /// Problems found at start (bad config, unreadable meetings, shortcut taken).
    startup_errors: Vec<String>,
    /// MCP over localhost HTTP, while `mcp_enabled` and `mcp_http` are on (NGR-168).
    mcp_http: Mutex<Option<mirona_mcp::HttpServer>>,
}

impl AppState {
    fn engine(&self) -> Arc<Engine> {
        self.engine.read().unwrap().clone()
    }

    fn rec_state(&self) -> RecState {
        let session = self.session.lock().unwrap();
        RecState {
            recording: session.as_ref().map(|s| s.id.clone()),
            mic_muted: session.as_ref().is_some_and(Session::mic_muted),
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
    /// The mic records silence (only while recording).
    mic_muted: bool,
    processing: Vec<String>,
}

// --- Meetings ---

/// Reload the store if the CLI or an import changed the meetings folder. Cheap when nothing did.
#[tauri::command]
fn refresh_store(st: St) -> Res<bool> {
    let engine = st.engine();
    let changed = engine.store.lock().unwrap().refresh()?;
    if changed {
        embed_later(engine);
    }
    Ok(changed)
}

/// Embed the lines of meetings that have no vectors yet (meetings from the CLI, demo meetings,
/// ones recorded while Ollama was off), off the UI thread and one run at a time (NGR-167).
fn embed_later(engine: Arc<Engine>) {
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        if let Some(emb) = mirona_engine::search::embedder(&engine.config())
            && let Err(e) = mirona_engine::search::backfill(&engine.store, &emb)
        {
            eprintln!("mirona: search by meaning unavailable: {e}");
        }
        RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

#[tauri::command]
fn list_meetings(st: St) -> Vec<MeetingSummary> {
    st.engine().store.lock().unwrap().list_meetings()
}

/// Keywords and meaning across every meeting (NGR-167), the same search MCP runs. Async so
/// embedding the query never blocks the window.
#[tauri::command]
async fn search(
    st: St<'_>,
    query: String,
    filters: mirona_core::SearchFilters,
) -> Res<mirona_core::SearchResults> {
    let engine = st.engine();
    let emb = mirona_engine::search::embedder(&engine.config());
    let store = engine.store.lock().unwrap();
    Ok(mirona_engine::search::search_with(
        &store,
        emb.as_ref(),
        &query,
        &filters,
        50,
    ))
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

/// A rough picture of a voiceprint for the People page: `VOICE_BINS` values in 0..=1. Lossy on
/// purpose (the window only draws it); the embedding stays in the store and MCP never sees this.
#[tauri::command]
fn voiceprint_shape(st: St, person_id: String) -> Option<Vec<f32>> {
    let engine = st.engine();
    let v = engine
        .store
        .lock()
        .unwrap()
        .voices()
        .voiceprint(&person_id)?;
    Some(voice_shape(&v, VOICE_BINS))
}

const VOICE_BINS: usize = 48;

/// Mean |x| per slice of `v`, scaled so the quietest slice is 0 and the loudest 1.
#[allow(clippy::cast_precision_loss)] // slices of a few dozen values
fn voice_shape(v: &[f32], bins: usize) -> Vec<f32> {
    if v.is_empty() || bins == 0 {
        return vec![];
    }
    // Exactly `bins` slices, split proportionally (a slice repeats when `v` is shorter).
    let raw: Vec<f32> = (0..bins)
        .map(|i| {
            let start = (i * v.len() / bins).min(v.len() - 1);
            let c = &v[start..((i + 1) * v.len() / bins).max(start + 1)];
            c.iter().map(|x| x.abs()).sum::<f32>() / c.len() as f32
        })
        .collect();
    let (lo, hi) = raw
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    let span = (hi - lo).max(f32::EPSILON);
    raw.iter().map(|x| (x - lo) / span).collect()
}

#[tauri::command]
fn enroll_speaker(st: St, speaker_id: String, name: String) -> Res<Speaker> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .enroll_speaker(&speaker_id, &name)
}

/// Commit a meeting's pending speaker changes together: voices named, lines moved to another
/// person; fixes wrong voice matches (NGR-153/154/201).
#[tauri::command]
fn apply_speaker_edits(st: St, meeting_id: String, edits: Vec<SpeakerEdit>) -> Res<()> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .apply_speaker_edits(&meeting_id, &edits)
}

/// Voices in a meeting nobody named yet (NGR-201).
#[tauri::command]
fn unnamed_speakers(st: St, meeting_id: String) -> Res<Vec<String>> {
    st.engine()
        .store
        .lock()
        .unwrap()
        .unnamed_speakers(&meeting_id)
}

/// Everyone who has a name, for the person pickers (NGR-201).
#[tauri::command]
fn known_names(st: St) -> Vec<String> {
    st.engine().store.lock().unwrap().known_names()
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
fn correct_answer(app: AppHandle, st: St, correction: Correction) -> Res<usize> {
    let engine = st.engine();
    let n = {
        let mut s = engine.store.lock().unwrap();
        s.correct(correction.clone())?;
        s.corrections().len()
    };
    if correction.answer_id == "action" && correction.value {
        word_confirmed_action(&app, engine, &correction.segment_id);
    }
    Ok(n)
}

/// A line the user confirmed as an action is filed with its raw text; the LLM rewords it as a
/// short task in the background (a `Notice` makes the app reload it).
fn word_confirmed_action(app: &AppHandle, engine: Arc<Engine>, segment_id: &str) {
    let suffix = format!("-ai-{segment_id}");
    let item = engine
        .store
        .lock()
        .unwrap()
        .action_items()
        .into_iter()
        .find(|a| a.id.ends_with(&suffix));
    let Some(item) = item else { return };
    let tx = events(app);
    std::thread::spawn(move || {
        let res = Llm::from_config(&engine.config())
            .and_then(|llm| mirona_engine::classify::word_action(&engine.store, &item.id, &llm));
        let ev = match res {
            Ok(true) => notice(
                &item.meeting_id,
                "Action item worded as a task.".into(),
                false,
            ),
            Ok(false) => return,
            Err(e) => notice(
                &item.meeting_id,
                format!("Action item kept as said: {e}"),
                true,
            ),
        };
        tx.send(ev).ok();
    });
}

/// The user set a meeting's tags; ones not in the list yet join it.
#[tauri::command]
fn set_meeting_tags(app: AppHandle, st: St, meeting_id: String, tags: Vec<String>) -> Res<Meeting> {
    let engine = st.engine();
    let tags = tags
        .iter()
        .map(|t| mirona_engine::clean_tag(t))
        .collect::<Res<Vec<_>>>()?;
    for t in &tags {
        engine.add_tag(&st.app_dir, t)?;
    }
    let m = engine.store.lock().unwrap().set_tags(&meeting_id, &tags)?;
    changed(&app, Ok(m))
}

/// Rename a tag in the list, its rules and every meeting. Returns how many meetings changed.
#[tauri::command]
fn rename_tag(app: AppHandle, st: St, old: String, new: String) -> Res<usize> {
    changed(&app, st.engine().rename_tag(&st.app_dir, &old, &new))
}

/// What in a meeting is below its threshold and not yet confirmed (NGR-158).
#[tauri::command]
fn review(st: St, meeting_id: String) -> Res<Vec<Review>> {
    st.engine().store.lock().unwrap().review(&meeting_id)
}

/// Confirm (`accept`) or reject one review item in one click (NGR-158). For the meeting type,
/// `review.value` is the option the user picked. An accepted new tag joins the tag list. A
/// confirmed "needs a follow-up?" gets its draft written in the background (a `Notice` says
/// when).
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
    if accept && review.question == "new_tag" {
        changed(&app, engine.add_tag(&st.app_dir, &review.value))?;
    }
    if accept && matches!(review.question.as_str(), "action" | "action_item") {
        let seg = review.segment_id.clone().or_else(|| {
            let m = engine.store.lock().unwrap().meeting(&meeting_id).ok()?;
            m.segments
                .into_iter()
                .find(|x| Some(x.start_ms) == review.at_ms)
                .map(|x| x.id)
        });
        if let Some(seg) = seg {
            word_confirmed_action(&app, engine.clone(), &seg);
        }
    }
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
        (None, "tag" | "new_tag") => {
            s.answer_tag(meeting_id, &r.value, accept)?;
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
        return Err("Speaker detection is off in Settings › AI models.".into());
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
    let c = c.ok_or("Classification is off in Settings › AI models.")?;
    let tx = events(&app);
    if let Some(why) = why {
        tx.send(notice(&meeting_id, why, false)).ok();
    }
    std::thread::spawn(move || {
        let res = mirona_engine::classify::reclassify(
            &engine.store,
            &engine.paths.templates,
            &cfg,
            &meeting_id,
            &c,
            &tx,
        );
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

/// The recording's path, for `convertFileSrc` (the meetings folder is in the asset scope). An
/// encrypted recording is decrypted to `.mirona/tmp` first (cleared on the next start).
#[tauri::command]
fn audio_path(app: AppHandle, st: St, meeting_id: String) -> Res<Option<PathBuf>> {
    let path = st.engine().store.lock().unwrap().plain_audio(&meeting_id)?;
    let path = path.filter(|p| p.is_file());
    // The decrypted copy sits under `.mirona/`, which the folder-wide scope doesn't match
    // (dot folders need a literal match): allow that one file.
    if let Some(p) = &path {
        app.asset_protocol_scope()
            .allow_file(p)
            .map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// Delete a meeting everywhere: folder, index, embeddings, voices and corrections (NGR-165).
#[tauri::command]
fn delete_meeting(st: St, meeting_id: String) -> Res<()> {
    if st.rec_state().recording.as_deref() == Some(meeting_id.as_str()) {
        return Err("Stop the recording first.".into());
    }
    st.engine()
        .store
        .lock()
        .unwrap()
        .delete_meeting(&meeting_id)
}

/// Remove generated parts of a meeting (recording, summary, enhanced notes, tags); the
/// transcript and the user's notes stay.
#[tauri::command]
fn clear_meeting_parts(
    st: St,
    meeting_id: String,
    parts: Vec<mirona_core::MeetingPart>,
) -> Res<()> {
    if st.rec_state().recording.as_deref() == Some(meeting_id.as_str()) {
        return Err("Stop the recording first.".into());
    }
    st.engine()
        .store
        .lock()
        .unwrap()
        .clear_parts(&meeting_id, &parts)
}

/// Delete every meeting with a tag. Returns how many went.
#[tauri::command]
fn delete_tagged(app: AppHandle, st: St, tag: String) -> Res<usize> {
    if st.session.lock().unwrap().is_some() {
        return Err("Stop the recording first.".into());
    }
    changed(&app, st.engine().delete_tagged(&tag))
}

/// Apply the retention rules now; what was removed, one line each.
#[tauri::command]
fn run_retention(app: AppHandle, st: St) -> Res<Vec<String>> {
    changed(&app, st.engine().retention())
}

/// Disk use in bytes, one `(label, bytes)` per bucket; empty buckets are left out.
#[tauri::command]
async fn storage_usage(st: St<'_>) -> Res<Vec<(String, u64)>> {
    let paths = st.engine().paths.clone();
    let (mut audio, mut text, mut index) = (0, 0, 0);
    walk(&paths.meetings, &mut |p, n| {
        if p.components().any(|c| c.as_os_str() == ".mirona") {
            index += n;
        } else if p
            .file_name()
            .is_some_and(|f| f.to_string_lossy().starts_with("audio"))
        {
            audio += n;
        } else {
            text += n;
        }
    });
    let mut models = 0;
    walk(&paths.models, &mut |_, n| {
        models += n;
    });
    let mut ollama = 0;
    if let Some(home) = std::env::var_os("HOME") {
        walk(&PathBuf::from(home).join(".ollama/models"), &mut |_, n| {
            ollama += n;
        });
    }
    let rows = [
        ("Meeting audio", audio),
        ("Transcripts, notes and summaries", text),
        ("Search index and logs", index),
        ("Speech and speaker models", models),
        ("Ollama models (shared with other apps)", ollama),
    ];
    Ok(rows
        .into_iter()
        .filter(|r| r.1 > 0)
        .map(|(l, n)| (l.to_string(), n))
        .collect())
}

/// Every file under `dir` (symlinks not followed); unreadable entries are skipped.
fn walk(dir: &Path, f: &mut impl FnMut(&Path, u64)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            walk(&e.path(), f);
        } else {
            f(&e.path(), meta.len());
        }
    }
}

/// Encrypt (or decrypt) every meeting at rest. Rewrites files: never while recording.
#[tauri::command]
async fn set_encryption(app: AppHandle, st: St<'_>, on: bool) -> Res<Settings> {
    if st.session.lock().unwrap().is_some() {
        return Err("Stop the recording before changing encryption.".into());
    }
    let engine = st.engine();
    engine.set_encryption(&st.app_dir, on)?;
    changed(&app, Ok(Settings::of(&engine.config(), &engine.paths)))
}

/// Adds the four sample meetings, for a tour before the first real one.
#[tauri::command]
fn add_demo_meetings(st: St) -> Res<()> {
    let engine = st.engine();
    mirona_store::sample::seed(&mut engine.store.lock().unwrap())?;
    embed_later(engine);
    Ok(())
}

/// Shows the summary templates folder in Finder (defaults written first, so it's never empty).
#[tauri::command]
fn open_templates(app: AppHandle, st: St) -> Res<()> {
    let dir = st.engine().paths.templates.clone();
    mirona_summary::templates(&dir)?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
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

/// Mute or unmute the mic in the current recording.
#[tauri::command]
fn set_mic_muted(app: AppHandle, muted: bool) -> Res<()> {
    mute(&app, Some(muted))
}

/// `None` toggles (tray).
fn mute(app: &AppHandle, muted: Option<bool>) -> Res<()> {
    {
        let st = app.state::<AppState>();
        let slot = st.session.lock().unwrap();
        let s = slot.as_ref().ok_or("Not recording.")?;
        s.set_mic_muted(muted.unwrap_or(!s.mic_muted()));
    }
    rec_changed(app);
    Ok(())
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
            "Speech models are missing ({}). Download them in Settings › AI models.",
            missing.join(", ")
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

/// Start recording and name the meeting after the calendar event; tag it with the tag the
/// title names (NGR-170).
fn record_call(app: &AppHandle, title: Option<&str>, tag: Option<&str>) -> Res<Meeting> {
    let mut meeting = start(app)?;
    *PENDING_CALL.lock().unwrap() = None;
    let engine = app.state::<AppState>().engine();
    let mut s = engine.store.lock().unwrap();
    if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
        meeting = s.set_title(&meeting.id, t)?;
    }
    if let Some(t) = tag {
        meeting = s.set_tags(&meeting.id, &[t.to_string()])?;
    }
    Ok(meeting)
}

#[tauri::command]
async fn start_call_recording(
    app: AppHandle,
    title: Option<String>,
    tag: Option<String>,
) -> Res<Meeting> {
    tauri::async_runtime::spawn_blocking(move || {
        record_call(&app, title.as_deref(), tag.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A call app started using the mic; sent on the "call" channel.
#[derive(Debug, Clone, Serialize)]
struct CallSuggestion {
    app: String,
    /// The calendar event happening now, when calendar titles are on.
    title: Option<String>,
    /// The tag whose name is in that title.
    tag: Option<String>,
}

/// The suggestion still on offer: the window asks for it when it loads, so a call that started
/// before the window was listening (or before Mirona opened) is never missed. Cleared when the
/// call ends, when it is recorded, or when the user dismisses it.
static PENDING_CALL: Mutex<Option<CallSuggestion>> = Mutex::new(None);

#[tauri::command]
fn pending_call() -> Option<CallSuggestion> {
    PENDING_CALL.lock().unwrap().clone()
}

#[tauri::command]
fn dismiss_call() {
    *PENDING_CALL.lock().unwrap() = None;
}

/// Every 3 s: when a call app newly opens the mic and nothing records, suggest recording, or
/// record right away if the calendar names a tag the user allowed to auto-record.
fn watch_calls(app: AppHandle) {
    std::thread::spawn(move || {
        let mut seen: Vec<String> = vec![];
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let cfg = app.state::<AppState>().engine().config();
            if !cfg.detect_calls {
                seen.clear();
                continue;
            }
            let calls = mirona_capture::calls::calls_using_mic();
            let new = calls.iter().find(|c| !seen.contains(&c.name)).cloned();
            // The suggested app let go of the mic: the call is over, the banner goes.
            {
                let mut pending = PENDING_CALL.lock().unwrap();
                if pending
                    .as_ref()
                    .is_some_and(|p| !calls.iter().any(|c| c.name == p.app))
                {
                    *pending = None;
                    app.emit("call", None::<CallSuggestion>).ok();
                }
            }
            seen = calls.into_iter().map(|c| c.name).collect();
            let Some(call) = new else { continue };
            if app.state::<AppState>().session.lock().unwrap().is_some() {
                continue;
            }
            let title = if cfg.calendar_titles {
                mirona_capture::calls::current_event_title()
            } else {
                None
            };
            let tag = title
                .as_deref()
                .and_then(|t| mirona_capture::calls::tag_in_title(t, cfg.tags.keys()));
            let notify = |title: &str, body: &str| {
                if let Err(e) = app.notification().builder().title(title).body(body).show() {
                    eprintln!("mirona: notification: {e}");
                }
            };
            if let Some(c) = tag.as_ref().filter(|c| cfg.auto_record.contains(c)) {
                match record_call(&app, title.as_deref(), Some(c)) {
                    Ok(m) => notify("Recording", &format!("{} ({}): {}", m.title, c, call.name)),
                    Err(e) => {
                        app.emit("pipeline", notice("", e, true)).ok();
                    }
                }
                continue;
            }
            notify(
                "Record this call?",
                &format!(
                    "{} is using the microphone. Open Mirona to record.",
                    call.name
                ),
            );
            let suggestion = CallSuggestion {
                app: call.name,
                title,
                tag,
            };
            *PENDING_CALL.lock().unwrap() = Some(suggestion.clone());
            app.emit("call", Some(suggestion)).ok();
        }
    });
}

/// Tray and shortcut entry point: never blocks the main thread, errors become notices.
fn toggle(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let recording = app.state::<AppState>().session.lock().unwrap().is_some();
        let res = if recording {
            stop(&app).map(drop)
        } else {
            start(&app).map(|m| {
                // No window to ask in first: say it as the recording starts (NGR-169).
                if app.state::<AppState>().engine().config().consent_reminder {
                    let msg = "Recording. Remember to ask everyone for consent (message in \
                               Settings › General › Consent reminder).";
                    app.emit("pipeline", notice(&m.id, msg.into(), false)).ok();
                }
            })
        };
        if let Err(e) = res {
            app.emit("pipeline", notice("", e, true)).ok();
        }
    });
}

fn rec_changed(app: &AppHandle) {
    let state = app.state::<AppState>().rec_state();
    if state.recording.is_some() {
        animate_tray(app);
    }
    app.emit("recording", state).ok();
}

const TRAY_PNG: &[u8] = include_bytes!("../icons/tray.png");
static TRAY_ANIMATING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// While recording, the menu bar icon's bars move (~10 fps) next to a red dot, or a muted
/// mic while the mic is muted; the plain icon comes back when recording stops. macOS only.
fn animate_tray(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    if !cfg!(target_os = "macos") || TRAY_ANIMATING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let mut dark = false;
        for n in 0u32.. {
            let Some(tray) = app.tray_by_id("mirona") else {
                break;
            };
            let state = app.state::<AppState>().rec_state();
            if state.recording.is_none() {
                TRAY_ANIMATING.store(false, Ordering::SeqCst);
                if let Ok(icon) = tauri::image::Image::from_bytes(TRAY_PNG) {
                    tray.set_icon(Some(icon)).ok();
                }
                tray.set_icon_as_template(true).ok();
                return;
            }
            if n == 0 {
                tray.set_icon_as_template(false).ok();
            }
            if n % 10 == 0 {
                // The bars aren't a template image, so follow light/dark by hand.
                dark = app
                    .get_webview_window("main")
                    .and_then(|w| w.theme().ok())
                    .is_some_and(|t| t == tauri::Theme::Dark);
            }
            let badge = if state.mic_muted {
                tray_icon::Badge::Muted
            } else {
                tray_icon::Badge::Dot
            };
            let rgba = tray_icon::frame(start.elapsed().as_secs_f32(), badge, dark);
            let size = tray_icon::SIZE;
            tray.set_icon(Some(tauri::image::Image::new_owned(rgba, size, size)))
                .ok();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        TRAY_ANIMATING.store(false, Ordering::SeqCst);
    });
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

/// Shows the OS prompt (blocks until answered, so off the main thread).
/// `kind`: mic | system | calendar.
#[tauri::command]
async fn request_permission(kind: String) -> Res<bool> {
    tauri::async_runtime::spawn_blocking(move || match kind.as_str() {
        "mic" => Ok(mirona_capture::request_mic_permission()),
        "system" => Ok(mirona_capture::request_system_audio_permission()),
        "calendar" => Ok(mirona_capture::calls::request_calendar_permission()),
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
    /// Bytes the downloader would fetch for them.
    download_size: u64,
    /// Providers with an API key in the keychain.
    keys: Vec<String>,
    config_path: PathBuf,
    /// How MCP clients start the server: this app binary, run with `mcp`.
    cli: String,
    /// The record shortcut, as in config.toml.
    shortcut: String,
    mute_shortcut: String,
    /// This build includes Whisper (the `whisper` feature).
    whisper: bool,
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
        download_size: mirona_engine::models::needed(&engine.config(), &engine.paths.models)
            .iter()
            .map(|f| f.size)
            .sum(),
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
        mute_shortcut: engine.config().mute_shortcut,
        whisper: mirona_capture::WHISPER_BUILT,
        errors,
    }
}

/// Download progress in bytes, on the "models" channel; `null` when a run ends.
#[derive(Clone, Serialize)]
struct ModelProgress {
    done: u64,
    total: u64,
}

/// Downloads the models the configured providers need (NGR-174), progress on "models".
/// Resolves when done; a cancelled or failed run keeps its partial files and resumes next time.
#[tauri::command]
async fn download_models(app: AppHandle) -> Res<()> {
    use std::sync::atomic::Ordering;
    let st = app.state::<AppState>();
    if st.downloading.swap(true, Ordering::SeqCst) {
        return Err("Already downloading.".into());
    }
    st.cancel_download.store(false, Ordering::SeqCst);
    let engine = st.engine();
    let worker = app.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        let st = worker.state::<AppState>();
        let mut sent = std::time::Instant::now();
        mirona_engine::models::download(
            &engine.config(),
            &engine.paths.models,
            &st.cancel_download,
            |done, total| {
                // A few updates a second is enough for a progress bar.
                if done == 0 || done == total || sent.elapsed().as_millis() >= 200 {
                    sent = std::time::Instant::now();
                    worker
                        .emit("models", Some(ModelProgress { done, total }))
                        .ok();
                }
            },
        )
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    st.downloading.store(false, Ordering::SeqCst);
    app.emit("models", None::<ModelProgress>).ok();
    changed(&app, res)
}

#[tauri::command]
fn cancel_download(st: St) {
    st.cancel_download
        .store(true, std::sync::atomic::Ordering::SeqCst);
}

/// The command MCP clients run: this app's own binary with `mcp` (see `main.rs`), so it works
/// without installing the CLI.
fn cli_path() -> String {
    std::env::current_exe().map_or_else(|_| "mirona".into(), |p| p.display().to_string())
}

/// Empty `key` removes it. Keys never touch `config.toml`.
#[tauri::command]
fn set_api_key(app: AppHandle, provider: String, key: String) -> Res<()> {
    if !KEY_PROVIDERS.contains(&provider.as_str()) {
        return Err(format!("no API key for {provider}"));
    }
    let key = key.trim();
    let res = if key.is_empty() {
        keychain::delete_api_key(&provider)
    } else {
        keychain::set_api_key(&provider, key)
    };
    changed(&app, res.map_err(|e| e.to_string()))
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<PathBuf> {
    app.dialog().file().blocking_pick_folder()?.into_path().ok()
}

// --- Settings (NGR-150) ---

/// Everything the Settings window edits. Mirrors `config.toml`; folders are shown
/// resolved (the default when the config leaves them out).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // independent on/off settings
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
    /// MCP over HTTP on 127.0.0.1:`mcp_port`, and whether agents may write (NGR-168).
    #[serde(default)]
    mcp_http: bool,
    #[serde(default)]
    mcp_port: u16,
    #[serde(default)]
    mcp_write: bool,
    #[serde(default)]
    mcp_sensitive: bool,
    /// Names of the HTTP clients holding a token; read-only (tokens never leave the backend
    /// after they're created).
    #[serde(default, skip_deserializing)]
    mcp_clients: Vec<String>,
    /// Global start/stop shortcut, e.g. "CmdOrCtrl+Alt+Shift+R".
    shortcut: String,
    /// Global mute/unmute-my-mic shortcut.
    mute_shortcut: String,
    meetings_dir: PathBuf,
    models_dir: PathBuf,
    /// jev | local-llm | off
    #[serde(default)]
    classification: String,
    /// The local classifier's Ollama model and chat URL; empty: `qwen3:4b`, the summary's server.
    #[serde(default)]
    classifier_model: String,
    #[serde(default)]
    classifier_base_url: String,
    /// speakrs (`CoreML` when it can) | speakrs-cpu | off
    #[serde(default)]
    diarization: String,
    /// Question id → the user's threshold; questions left out use `threshold_defaults`.
    #[serde(default)]
    thresholds: BTreeMap<String, f32>,
    /// The defaults for the classifier in use (local or Jev); read-only.
    #[serde(default, skip_deserializing)]
    threshold_defaults: BTreeMap<String, f32>,
    /// Question id → the user's floor (below it: rejected without asking); see `floor_defaults`.
    #[serde(default)]
    floors: BTreeMap<String, f32>,
    #[serde(default, skip_deserializing)]
    floor_defaults: BTreeMap<String, f32>,
    /// Tags (`group:name` or `name`) → "" (no descriptions since NGR-202); one yes/no each per
    /// meeting.
    #[serde(default)]
    tags: BTreeMap<String, String>,
    /// Suggest recording when a call app opens the mic (NGR-170).
    #[serde(default)]
    detect_calls: bool,
    /// Name meetings after the calendar event happening now.
    #[serde(default)]
    calendar_titles: bool,
    /// Tags whose calls start recording without a click (needs calendar titles).
    #[serde(default)]
    auto_record: Vec<String>,
    /// Consent reminder before recording (NGR-169).
    #[serde(default)]
    consent_reminder: bool,
    #[serde(default)]
    consent_message: String,
    /// Tags whose people already agreed to be recorded.
    #[serde(default)]
    consent_given: Vec<String>,
    /// Encryption at rest; read-only here: `set_encryption` turns it on or off (NGR-165).
    #[serde(default, skip_deserializing)]
    encrypt: bool,
    /// Delete recordings this many days old (text stays). `None`: keep them.
    #[serde(default)]
    retention_audio_days: Option<u32>,
    /// Tag → delete its meetings this many days old.
    #[serde(default)]
    retention_tags: BTreeMap<String, u32>,
    /// Check GitHub for a new version on start and daily.
    #[serde(default)]
    check_updates: bool,
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
    "tag",
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
            mcp_http: cfg.mcp_http,
            mcp_port: cfg.mcp_port,
            mcp_write: cfg.mcp_write,
            mcp_sensitive: cfg.mcp_sensitive,
            mcp_clients: cfg.mcp_tokens.keys().cloned().collect(),
            shortcut: cfg.shortcut.clone(),
            mute_shortcut: cfg.mute_shortcut.clone(),
            meetings_dir: paths.meetings.clone(),
            models_dir: paths.models.clone(),
            classification: mirona_engine::stage(cfg, "classification", "jev"),
            classifier_model: cfg.classifier.model.clone(),
            classifier_base_url: cfg.classifier.base_url.clone(),
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
            floors: cfg.floors.clone(),
            floor_defaults: QUESTIONS
                .iter()
                .map(|q| ((*q).to_string(), default_floor(q)))
                .collect(),
            tags: cfg.tags.clone(),
            detect_calls: cfg.detect_calls,
            calendar_titles: cfg.calendar_titles,
            auto_record: cfg.auto_record.clone(),
            consent_reminder: cfg.consent_reminder,
            consent_message: cfg.consent_message.clone(),
            consent_given: cfg.consent_given.clone(),
            encrypt: cfg.encrypt,
            retention_audio_days: cfg.retention.audio_days,
            retention_tags: cfg.retention.tags.clone(),
            check_updates: cfg.check_updates,
        }
    }

    /// The classifier's Ollama URL; in privacy mode only this Mac or the local network.
    fn classifier_url(&self) -> Res<&str> {
        let url = self.classifier_base_url.trim();
        if self.privacy && !url.is_empty() && !mirona_providers::net::is_local(url) {
            return Err(
                "In privacy mode the classification server must be on this Mac or your \
                        local network."
                    .into(),
            );
        }
        Ok(url)
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
                let local = mirona_providers::net::is_local(url);
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
        if parse_shortcut(&self.shortcut)?.id() == parse_shortcut(&self.mute_shortcut)?.id() {
            return Err("The record and mute shortcuts must differ.".into());
        }
        if self.mcp_port < 1024 {
            return Err("Pick an MCP port from 1024 to 65535.".into());
        }
        if !["jev", "local-llm", "off"].contains(&self.classification.as_str()) {
            return Err(format!("unknown classifier {}", self.classification));
        }
        let classifier_url = self.classifier_url()?;
        if !["speakrs", "speakrs-cpu", "off"].contains(&self.diarization.as_str()) {
            return Err(format!("unknown speaker detection {}", self.diarization));
        }
        for (q, t) in self.thresholds.iter().chain(&self.floors) {
            if !QUESTIONS.contains(&q.as_str()) || !(0.0..=1.0).contains(t) {
                return Err(format!("a threshold for {q} must be between 0 and 1"));
            }
        }
        let mut tags = BTreeMap::new();
        for tag in self.tags.keys() {
            tags.insert(mirona_engine::clean_tag(tag)?, String::new());
        }
        let dir = |p: &Path, default: &Path| (p != default).then(|| p.to_path_buf());
        cfg.privacy = self.privacy;
        cfg.providers.insert("stt".into(), self.stt.clone());
        cfg.providers.insert("llm".into(), self.llm.clone());
        cfg.llm.model = self.llm_model.trim().into();
        cfg.llm.base_url = base.into();
        cfg.classifier.model = self.classifier_model.trim().into();
        cfg.classifier.base_url = classifier_url.into();
        cfg.template = self.template.trim().into();
        cfg.mcp_enabled = self.mcp_enabled;
        cfg.mcp_http = self.mcp_http;
        cfg.mcp_port = self.mcp_port;
        cfg.mcp_write = self.mcp_write;
        cfg.mcp_sensitive = self.mcp_sensitive;
        cfg.shortcut = self.shortcut.trim().into();
        cfg.mute_shortcut = self.mute_shortcut.trim().into();
        cfg.meetings_dir = dir(&self.meetings_dir, &defaults.meetings);
        cfg.models_dir = dir(&self.models_dir, &defaults.models);
        cfg.providers
            .insert("classification".into(), self.classification.clone());
        cfg.providers
            .insert("diarization".into(), self.diarization.clone());
        cfg.thresholds.clone_from(&self.thresholds);
        cfg.floors.clone_from(&self.floors);
        // A removed tag stops auto-recording with it.
        cfg.auto_record = self
            .auto_record
            .iter()
            .map(|c| c.trim().to_string())
            .filter(|c| tags.contains_key(c))
            .collect();
        cfg.tags = tags;
        cfg.detect_calls = self.detect_calls;
        cfg.calendar_titles = self.calendar_titles;
        cfg.check_updates = self.check_updates;
        if self.retention_audio_days == Some(0) || self.retention_tags.values().any(|d| *d == 0) {
            return Err("Retention needs at least 1 day.".into());
        }
        self.apply_consent_and_retention(&mut cfg);
        Ok(cfg)
    }

    /// The consent reminder (NGR-169) and retention (NGR-165) part of [`Settings::apply`].
    fn apply_consent_and_retention(&self, cfg: &mut Config) {
        cfg.consent_reminder = self.consent_reminder;
        cfg.consent_message = match self.consent_message.trim() {
            "" => mirona_providers::config::DEFAULT_CONSENT_MESSAGE.into(),
            m => m.into(),
        };
        cfg.consent_given = self
            .consent_given
            .iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();
        cfg.retention.audio_days = self.retention_audio_days;
        cfg.retention.tags = self
            .retention_tags
            .iter()
            .map(|(c, d)| (c.trim().to_string(), *d))
            .filter(|(c, _)| !c.is_empty())
            .collect();
    }
}

#[tauri::command]
fn get_settings(st: St) -> Settings {
    let engine = st.engine();
    Settings::of(&engine.config(), &engine.paths)
}

#[tauri::command]
fn save_settings(app: AppHandle, st: St, settings: Settings) -> Res<Settings> {
    changed(&app, save_settings_now(&app, &st, settings))
}

fn save_settings_now(app: &AppHandle, st: &AppState, settings: Settings) -> Res<Settings> {
    let engine = st.engine();
    let defaults = Paths::resolve(&Config::default(), &st.app_dir, &st.documents);
    let cfg = settings.apply(engine.config(), &defaults)?;
    let paths = Paths::resolve(&cfg, &st.app_dir, &st.documents);
    let old = engine.config();
    swap_shortcut(app, &old.shortcut, &cfg.shortcut, false)?;
    swap_shortcut(app, &old.mute_shortcut, &cfg.mute_shortcut, true)?;
    sync_mcp_http(st, &cfg)?;
    if paths.meetings == engine.paths.meetings && paths.models == engine.paths.models {
        engine.save_config(&st.app_dir, cfg.clone())?;
    } else {
        if st.session.lock().unwrap().is_some() {
            return Err("Stop the recording before moving folders.".into());
        }
        // Open the new folder first, so a bad pick never reaches config.toml.
        let store = Store::open_with(&paths.meetings, mirona_engine::keep::store_key(&cfg)?)?;
        cfg.save(&mirona_engine::config_path(&st.app_dir))?;
        allow_audio(app, &paths.meetings);
        let next = Engine::new(Arc::new(Mutex::new(store)), cfg.clone(), paths.clone());
        *st.engine.write().unwrap() = Arc::new(next);
    }
    Ok(Settings::of(&cfg, &paths))
}

/// Start, stop or move the MCP HTTP server to match `cfg`. If the new port can't be bound,
/// the old server keeps running and the error says why.
fn sync_mcp_http(st: &AppState, cfg: &Config) -> Res<()> {
    let want = (cfg.mcp_enabled && cfg.mcp_http).then_some(cfg.mcp_port);
    let mut running = st.mcp_http.lock().unwrap();
    let old = running.as_ref().map(|s| s.port);
    if old == want {
        return Ok(());
    }
    *running = None; // stops it, freeing the port
    let Some(port) = want else { return Ok(()) };
    match mirona_mcp::serve_http(st.app_dir.clone(), port) {
        Ok(s) => {
            *running = Some(s);
            Ok(())
        }
        Err(e) => {
            *running = old.and_then(|p| mirona_mcp::serve_http(st.app_dir.clone(), p).ok());
            Err(format!(
                "MCP over HTTP: {e}. Another app may use that port."
            ))
        }
    }
}

/// Newest MCP tool calls first: who called what, ok or not (NGR-168).
#[tauri::command]
fn mcp_activity(st: St, limit: usize) -> Vec<mirona_mcp::Activity> {
    mirona_mcp::activity(&st.app_dir, limit)
}

/// A new HTTP client token for `name`, returned once (the UI shows it in a config snippet).
#[tauri::command]
fn add_mcp_client(app: AppHandle, st: St, name: String) -> Res<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the client a name, e.g. Claude Desktop.".into());
    }
    let engine = st.engine();
    let mut cfg = engine.config();
    if cfg.mcp_tokens.contains_key(name) {
        return Err(format!("{name} already has a token. Revoke it first."));
    }
    let token = mirona_mcp::new_token()?;
    cfg.mcp_tokens.insert(name.into(), token.clone());
    engine.save_config(&st.app_dir, cfg)?;
    changed(&app, Ok(token))
}

#[tauri::command]
fn revoke_mcp_client(app: AppHandle, st: St, name: String) -> Res<()> {
    let engine = st.engine();
    let mut cfg = engine.config();
    cfg.mcp_tokens.remove(&name);
    changed(&app, engine.save_config(&st.app_dir, cfg))
}

/// Tells every window that settings (or the meetings they delete) changed, so the main
/// window and the settings window never drift apart. Passes `res` through.
fn changed<T>(app: &AppHandle, res: Res<T>) -> Res<T> {
    if res.is_ok() {
        app.emit("settings", ()).ok();
    }
    res
}

const SETTINGS_TABS: [&str; 7] = [
    "general", "privacy", "models", "tags", "agents", "advanced", "about",
];

/// Settings live in their own window. Opening it again focuses it on `tab`.
/// Async: on Windows a window built from a sync command deadlocks.
#[tauri::command]
async fn open_settings(app: AppHandle, tab: Option<String>) -> Res<()> {
    show_settings(&app, tab.as_deref())
}

/// Written to the app dir once the first onboarding closes; until then the main window stays hidden.
const ONBOARDED: &str = "onboarded";

/// Settings › About and "Finish setup": the onboarding on top of the window that asked.
/// Async: on Windows a window built from a sync command deadlocks.
#[tauri::command]
async fn show_onboarding(
    app: AppHandle,
    window: tauri::WebviewWindow,
    step: Option<u8>,
) -> Res<()> {
    open_onboarding(&app, step.unwrap_or(0), Some(&window))
}

fn open_onboarding(app: &AppHandle, step: u8, over: Option<&tauri::WebviewWindow>) -> Res<()> {
    const W: f64 = 980.0;
    const H: f64 = 660.0;
    if let Some(w) = app.get_webview_window("onboarding") {
        app.emit_to("onboarding", "onboarding-step", step).ok();
        w.show().ok();
        return w.set_focus().map_err(|e| e.to_string());
    }
    let url = tauri::WebviewUrl::App("index.html".into());
    let mut b = tauri::WebviewWindowBuilder::new(app, "onboarding", url)
        .title("Welcome to Mirona")
        .theme(Some(tauri::Theme::Light))
        .inner_size(W, H)
        .resizable(false)
        .maximizable(false)
        .initialization_script(format!("window.miOnboardingStep = {step};"));
    // Over the window that asked (it may be on another display); centred on screen otherwise.
    let at = over.and_then(|p| {
        let scale = p.scale_factor().ok()?;
        let pos = p.outer_position().ok()?.to_logical::<f64>(scale);
        let size = p.outer_size().ok()?.to_logical::<f64>(scale);
        Some((
            pos.x + (size.width - W) / 2.0,
            pos.y + (size.height - H) / 2.0,
        ))
    });
    b = match at {
        Some((x, y)) => b.position(x, y),
        None => b.center(),
    };
    if let Some(parent) = over {
        b = b.parent(parent).map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    let b = b
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(18.0, 22.0));
    b.build().map(|_| ()).map_err(|e| e.to_string())
}

/// The onboarding closed (finished, skipped or the close button): the first time, remember
/// it and show the main window.
fn onboarding_closed(app: &AppHandle) {
    let marker = app.state::<AppState>().app_dir.join(ONBOARDED);
    if marker.exists() {
        return;
    }
    if let Err(e) = std::fs::write(&marker, "") {
        eprintln!("mirona: onboarding: {e}");
    }
    if let Some(w) = app.get_webview_window("main") {
        w.show().ok();
        w.set_focus().ok();
    }
}

fn show_settings(app: &AppHandle, tab: Option<&str>) -> Res<()> {
    let tab = tab
        .filter(|t| SETTINGS_TABS.contains(t))
        .unwrap_or("general");
    if let Some(w) = app.get_webview_window("settings") {
        app.emit_to("settings", "settings-tab", tab).ok();
        w.unminimize().ok();
        w.show().ok();
        return w.set_focus().map_err(|e| e.to_string());
    }
    let url = tauri::WebviewUrl::App("index.html".into());
    let b = tauri::WebviewWindowBuilder::new(app, "settings", url)
        .title("Settings")
        .theme(Some(tauri::Theme::Light))
        .inner_size(1040.0, 760.0)
        .min_inner_size(860.0, 560.0)
        .initialization_script(format!("window.miSettingsTab = \"{tab}\";"));
    #[cfg(target_os = "macos")]
    let b = b
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(18.0, 22.0))
        .transparent(true)
        .effects(
            tauri::window::EffectsBuilder::new()
                .effect(tauri::window::Effect::Sidebar)
                .state(tauri::window::EffectState::FollowsWindowActiveState)
                .build(),
        );
    b.build().map(|_| ()).map_err(|e| e.to_string())
}

/// Move the record (or `mute`) shortcut from `old` to `new` right away. If `new` can't be
/// registered (another app owns it), `old` stays and the error says so.
fn swap_shortcut(app: &AppHandle, old: &str, new: &str, mute: bool) -> Res<()> {
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
            "Couldn't use {new} as the {} shortcut: another app or the system may already \
             use it ({e}). Still using {old}.",
            if mute { "mute" } else { "record" }
        ));
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

fn tray(app: &tauri::App) -> tauri::Result<()> {
    let icon = if cfg!(target_os = "macos") {
        tauri::image::Image::from_bytes(TRAY_PNG)?
    } else {
        tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?
    };
    TrayIconBuilder::with_id("mirona")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Mirona")
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = ev
                && let Err(e) = toggle_popover(tray.app_handle(), rect)
            {
                eprintln!("mirona: menu bar popover: {e}");
            }
        })
        .build(app)?;
    Ok(())
}

const POPOVER_W: f64 = 304.0;
const POPOVER_H: f64 = 264.0;
/// When the popover last hid itself on losing focus, so the icon click that caused it
/// doesn't open it again straight away.
static POPOVER_HIDDEN: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// The menu bar popover (window `tray`, `TrayWindow` in `views/Tray.tsx`), under the icon
/// (above it when the icon sits low, as on the Windows taskbar). Built on first click.
fn toggle_popover(app: &AppHandle, rect: tauri::Rect) -> Res<()> {
    let w = match app.get_webview_window("tray") {
        Some(w) => w,
        None => build_popover(app)?,
    };
    if w.is_visible().unwrap_or(false) {
        return w.hide().map_err(|e| e.to_string());
    }
    let just_hid = POPOVER_HIDDEN
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(300));
    if just_hid {
        return Ok(());
    }
    let monitor = clicked_monitor(app, rect)
        .or_else(|| w.primary_monitor().ok().flatten())
        .ok_or("no display")?;
    let scale = monitor.scale_factor();
    let (pos, size) = (
        rect.position.to_logical::<f64>(scale),
        rect.size.to_logical::<f64>(scale),
    );
    let area = monitor.work_area();
    let (left, top) = (
        f64::from(area.position.x) / scale,
        f64::from(area.position.y) / scale,
    );
    let (right, bottom) = (
        left + f64::from(area.size.width) / scale,
        top + f64::from(area.size.height) / scale,
    );
    let x = (pos.x + size.width / 2.0 - POPOVER_W / 2.0).clamp(left + 8.0, right - POPOVER_W - 8.0);
    let y = if pos.y + size.height + POPOVER_H < bottom {
        pos.y + size.height + 6.0
    } else {
        pos.y - POPOVER_H - 6.0
    };
    w.set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())
}

/// The display whose menu bar (or taskbar) holds the clicked icon. On macOS each display's
/// "physical" numbers are its own points times its own scale, so the click rect can't be
/// looked up across a Retina laptop and a 1x monitor (it lands on the wrong one); the cursor,
/// in the primary display's scale, converts back to points that every display can be matched in.
#[cfg(target_os = "macos")]
fn clicked_monitor(app: &AppHandle, _rect: tauri::Rect) -> Option<tauri::Monitor> {
    let primary = app.primary_monitor().ok().flatten()?;
    let c = app
        .cursor_position()
        .ok()?
        .to_logical::<f64>(primary.scale_factor());
    app.available_monitors().ok()?.into_iter().find(|m| {
        let s = m.scale_factor();
        let (p, z) = (
            m.position().to_logical::<f64>(s),
            m.size().to_logical::<f64>(s),
        );
        (p.x..p.x + z.width).contains(&c.x) && (p.y..p.y + z.height).contains(&c.y)
    })
}

#[cfg(not(target_os = "macos"))]
fn clicked_monitor(app: &AppHandle, rect: tauri::Rect) -> Option<tauri::Monitor> {
    let at = rect.position.to_physical::<f64>(1.0);
    app.monitor_from_point(at.x, at.y).ok().flatten()
}

fn build_popover(app: &AppHandle) -> Res<tauri::WebviewWindow> {
    let url = tauri::WebviewUrl::App("index.html".into());
    let b = tauri::WebviewWindowBuilder::new(app, "tray", url)
        .title("Mirona")
        .theme(Some(tauri::Theme::Light))
        .inner_size(POPOVER_W, POPOVER_H)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false);
    #[cfg(target_os = "macos")]
    let b = b.effects(
        tauri::window::EffectsBuilder::new()
            .effect(tauri::window::Effect::Popover)
            .state(tauri::window::EffectState::Active)
            .radius(12.0)
            .build(),
    );
    b.build().map_err(|e| e.to_string())
}

/// The popover's buttons. Each closes it except mute, which flips in place.
/// Async: on Windows a window built from a sync command deadlocks (settings).
#[tauri::command]
async fn tray_action(app: AppHandle, action: String) -> Res<()> {
    if action != "mute"
        && let Some(w) = app.get_webview_window("tray")
    {
        w.hide().ok();
    }
    match action.as_str() {
        "toggle" => toggle(&app),
        "mute" => mute(&app, None)?,
        "show" => {
            if let Some(w) = app.get_webview_window("main") {
                w.show().ok();
                w.set_focus().ok();
            }
        }
        "settings" => show_settings(&app, None)?,
        "quit" => app.exit(0),
        "hide" => {} // Escape: hidden above
        _ => return Err(format!("Unknown tray action {action}.")),
    }
    Ok(())
}

/// macOS: the standard app menu plus "Settings…" (⌘,) under Mirona, where Mac apps keep it.
#[cfg(target_os = "macos")]
fn app_menu(app: &tauri::App) -> tauri::Result<()> {
    let menu = Menu::default(app.handle())?;
    if let Some(tauri::menu::MenuItemKind::Submenu(mirona)) = menu.items()?.first() {
        let settings =
            MenuItem::with_id(app, "app-settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
        // After "About" and its separator.
        mirona.insert_items(&[&settings, &PredefinedMenuItem::separator(app)?], 2)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, ev| {
        if ev.id.as_ref() == "app-settings"
            && let Err(e) = show_settings(app, None)
        {
            eprintln!("mirona: settings window: {e}");
        }
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::too_many_lines)] // mostly the list of commands
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state != ShortcutState::Pressed {
                        return;
                    }
                    let mute_sc = app.state::<AppState>().engine().config().mute_shortcut;
                    if parse_shortcut(&mute_sc).is_ok_and(|m| m.id() == sc.id()) {
                        mute(app, None).ok(); // not recording: nothing to mute
                    } else {
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
            let cfg = engine.config();
            for (what, sc) in [("Record", &cfg.shortcut), ("Mute", &cfg.mute_shortcut)] {
                let registered = parse_shortcut(sc)
                    .and_then(|s| app.global_shortcut().register(s).map_err(|e| e.to_string()));
                if let Err(e) = registered {
                    errors.push(format!(
                        "{what} shortcut {sc} is unavailable ({e}). Pick another in Settings."
                    ));
                }
            }
            if let Err(e) = tray(app) {
                errors.push(format!("No menu bar icon: {e}"));
            }
            #[cfg(target_os = "macos")]
            if let Err(e) = app_menu(app) {
                eprintln!("mirona: app menu: {e}");
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
                downloading: false.into(),
                cancel_download: false.into(),
                startup_errors: errors,
                mcp_http: Mutex::new(None),
            });
            app.manage(update::Pending::default());
            watch_calls(app.handle().clone());
            update::watch(app.handle().clone());
            let st = app.state::<AppState>();
            // First run: the onboarding comes before the main window (hidden in tauri.conf.json).
            if st.app_dir.join(ONBOARDED).exists() {
                if let Some(w) = app.get_webview_window("main") {
                    w.show().ok();
                }
            } else if let Err(e) = open_onboarding(app.handle(), 0, None) {
                eprintln!("mirona: onboarding: {e}");
                onboarding_closed(app.handle());
            }
            if let Err(e) = sync_mcp_http(&st, &st.engine().config()) {
                eprintln!("mirona: {e}");
            }
            // Recordings a crash left as WAV get compressed in the background.
            let engine = app.state::<AppState>().engine();
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                for e in engine.compress_recordings() {
                    eprintln!("mirona: compressing a recording: {e}");
                }
                // Retention now and then hourly (NGR-165).
                // The engine is looked up each time: moving folders swaps it.
                std::thread::spawn(move || {
                    loop {
                        match handle.state::<AppState>().engine().retention() {
                            Ok(done) => done
                                .iter()
                                .for_each(|l| eprintln!("mirona: retention: {l}")),
                            Err(e) => eprintln!("mirona: retention: {e}"),
                        }
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                    }
                });
                // Meetings recorded before search by meaning (or while Ollama was off).
                embed_later(engine);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_meetings,
            search,
            refresh_store,
            get_meeting,
            list_action_items,
            toggle_action_item,
            list_people,
            enroll_speaker,
            apply_speaker_edits,
            unnamed_speakers,
            known_names,
            merge_people,
            delete_voiceprint,
            correct_answer,
            set_meeting_tags,
            rename_tag,
            review,
            resolve_review,
            set_owner,
            rerun_diarization,
            rerun_classification,
            rename_meeting,
            save_notes,
            edit_segment,
            audio_path,
            delete_meeting,
            clear_meeting_parts,
            delete_tagged,
            run_retention,
            storage_usage,
            set_encryption,
            pending_call,
            dismiss_call,
            add_demo_meetings,
            list_templates,
            open_templates,
            regenerate,
            import_audio,
            download_models,
            cancel_download,
            start_recording,
            start_call_recording,
            stop_recording,
            set_mic_muted,
            recording_state,
            request_permission,
            status,
            set_api_key,
            pick_folder,
            get_settings,
            save_settings,
            mcp_activity,
            add_mcp_client,
            revoke_mcp_client,
            open_settings,
            show_onboarding,
            tray_action,
            voiceprint_shape,
            update::check_update,
            update::pending_update,
            update::install_update,
            update::open_release_notes,
        ])
        // Closing the main window keeps Mirona in the menu bar, so recording can still be
        // started from the tray or the shortcut. Quit from the menu bar popover. Settings just close.
        .on_window_event(|w, e| match e {
            tauri::WindowEvent::CloseRequested { api, .. } if w.label() == "main" => {
                w.hide().ok();
                api.prevent_close();
            }
            // The popover closes like a menu: on any click elsewhere.
            tauri::WindowEvent::Focused(false) if w.label() == "tray" => {
                w.hide().ok();
                *POPOVER_HIDDEN.lock().unwrap() = Some(std::time::Instant::now());
            }
            tauri::WindowEvent::Destroyed if w.label() == "onboarding" => {
                onboarding_closed(w.app_handle());
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while starting Mirona")
        .run(|app, e| {
            // Clicking the Dock icon brings the hidden window back (the onboarding while it's open).
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = e
                && let Some(w) = app
                    .get_webview_window("onboarding")
                    .or_else(|| app.get_webview_window("main"))
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

    #[test]
    fn voice_shape_is_scaled_and_tells_voices_apart() {
        let a: Vec<f32> = (0u16..256).map(|i| (f32::from(i) * 0.37).sin()).collect();
        let b: Vec<f32> = (0u16..256)
            .map(|i| (f32::from(i) * 0.11).cos() * 0.5)
            .collect();
        let (sa, sb) = (voice_shape(&a, VOICE_BINS), voice_shape(&b, VOICE_BINS));
        assert_eq!(sa.len(), VOICE_BINS);
        assert!(sa.iter().all(|x| (0.0..=1.0).contains(x)));
        assert!(sa.iter().copied().fold(1.0, f32::min) < 1e-6);
        assert!(sa.iter().copied().fold(0.0, f32::max) > 1.0 - 1e-6);
        assert_ne!(sa, sb);
        assert_eq!(voice_shape(&[], VOICE_BINS), Vec::<f32>::new());
        assert_eq!(voice_shape(&[0.5; 10], 4).len(), 4);
        assert_eq!(voice_shape(&[0.1, 0.9], VOICE_BINS).len(), VOICE_BINS);
    }

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
        moved.mcp_http = true;
        moved.mcp_port = 9000;
        moved.mcp_write = true;
        let cfg = moved.apply(cfg, &d).unwrap();
        assert!(cfg.mcp_http && cfg.mcp_write);
        assert_eq!(cfg.mcp_port, 9000);
        assert_eq!(cfg.meetings_dir, Some("/elsewhere".into()));
        assert_eq!(cfg.providers["stt"], "whisper");
        assert!(!cfg.mcp_enabled);
        let paths = Paths::resolve(&cfg, Path::new("/app"), Path::new("/home/me/Documents"));
        assert_eq!(Settings::of(&cfg, &paths), moved);

        // Tokens aren't settings: saving keeps them, and only client names come back.
        let mut with_token = cfg.clone();
        with_token.mcp_tokens.insert("Cursor".into(), "t0k".into());
        let back = moved.apply(with_token, &d).unwrap();
        assert_eq!(back.mcp_tokens["Cursor"], "t0k");
        assert_eq!(Settings::of(&back, &paths).mcp_clients, ["Cursor"]);
        moved.mcp_port = 80;
        assert!(moved.apply(cfg, &d).is_err(), "privileged port");
    }

    #[test]
    fn classifier_diarization_thresholds_and_tags_are_settings() {
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
        s.tags
            .insert(" project : Northwind Bank ".into(), String::new());
        assert!(s.detect_calls && !s.calendar_titles);
        s.calendar_titles = true;
        s.auto_record = vec!["project:Northwind Bank".into(), "Gone Client".into()];
        let cfg = s.apply(Config::default(), &d).unwrap();
        assert!(cfg.calendar_titles);
        assert_eq!(
            cfg.auto_record,
            ["project:Northwind Bank"],
            "unknown tags are dropped"
        );
        assert_eq!(cfg.providers["classification"], "local-llm");
        assert_eq!(cfg.providers["diarization"], "speakrs-cpu");
        assert!((cfg.threshold("action", true) - 0.8).abs() < 1e-6);
        assert_eq!(cfg.tags["project:Northwind Bank"], "");
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
                tags: [(" ".into(), String::new())].into(),
                ..back.clone()
            },
            Settings {
                tags: [("a:b:c".into(), String::new())].into(),
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
            rejected: false,
        };
        assert!(!resolve(&mut s, id, &r("meeting_type", "standup"), true).unwrap());
        let a = s.meeting(id).unwrap().answers.unwrap();
        assert_eq!(a.meeting_type.value, "standup");
        assert!(a.meeting_type.confirmed);

        // The demo's billing "maybe": dropped, then kept.
        assert!(!resolve(&mut s, id, &r("tag", "topic:billing"), false).unwrap());
        assert_eq!(s.meeting(id).unwrap().tags, ["project:Alder Restoration"]);
        resolve(&mut s, id, &r("tag", "topic:billing"), true).unwrap();
        assert_eq!(
            s.meeting(id).unwrap().tags,
            ["project:Alder Restoration", "topic:billing"]
        );

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
        s.mute_shortcut = "CmdOrCtrl+Shift+F9".into();
        assert!(s.apply(Config::default(), &d).is_err(), "same as record");
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

    #[test]
    fn classifier_model_and_server_are_saved_and_gated() {
        let d = defaults();
        let mut s = Settings::of(&Config::default(), &d);
        s.classification = "local-llm".into();
        s.classifier_model = " qwen3:14b ".into();
        s.classifier_base_url = "http://192.168.1.20:11434/api/chat".into();
        let cfg = s.apply(Config::default(), &d).unwrap();
        assert_eq!(cfg.classifier.model, "qwen3:14b");
        assert_eq!(
            cfg.classifier.base_url,
            "http://192.168.1.20:11434/api/chat"
        );
        assert_eq!(Settings::of(&cfg, &d).classifier_model, "qwen3:14b");
        s.classifier_base_url = "https://ollama.example.com/api/chat".into();
        assert!(
            s.apply(Config::default(), &d).is_err(),
            "public server in privacy mode"
        );
        s.privacy = false;
        assert!(s.apply(Config::default(), &d).is_ok());
    }
}
