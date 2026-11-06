//! App updates: Tauri's updater, pointed at the GitHub releases (`net::UPDATE_URL`).
//!
//! Nothing is checked unless the user turns on `check_updates` (then on start and daily) or
//! clicks "Check now" in Settings › About. Every request asks `net::allow_update` first, so
//! the only host reached is github.com. A found update is announced on the "update" channel
//! and in a notification; installing it needs a click, verifies the signature against the
//! public key in `tauri.conf.json`, replaces the app and restarts it.

use super::{AppState, Res};
use mirona_providers::net;
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State, Url};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    notes: String,
}

/// The update found by the last check, waiting for "Install and restart".
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

fn info(u: &Update) -> UpdateInfo {
    UpdateInfo {
        version: u.version.clone(),
        notes: u.body.clone().unwrap_or_default(),
    }
}

/// Asks GitHub for a newer version, remembers it and tells every window.
async fn check(app: &AppHandle) -> Res<Option<UpdateInfo>> {
    net::allow_update(net::UPDATE_URL).map_err(|e| e.to_string())?;
    let url = Url::parse(net::UPDATE_URL).map_err(|e| e.to_string())?;
    let update = app
        .updater_builder()
        .endpoints(vec![url])
        .and_then(tauri_plugin_updater::UpdaterBuilder::build)
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Couldn't check for updates: {e}"))?;
    if let Some(u) = &update {
        net::allow_update(u.download_url.as_str()).map_err(|e| e.to_string())?;
    }
    let found = update.as_ref().map(info);
    *app.state::<Pending>().0.lock().unwrap() = update;
    app.emit("update", found.clone()).ok();
    Ok(found)
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Res<Option<UpdateInfo>> {
    check(&app).await
}

#[tauri::command]
pub fn pending_update(pending: State<Pending>) -> Option<UpdateInfo> {
    pending.0.lock().unwrap().as_ref().map(info)
}

/// Downloads, verifies and installs the pending update, then restarts Mirona.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Res<()> {
    {
        let st = app.state::<AppState>();
        if st.session.lock().unwrap().is_some() || !st.processing.lock().unwrap().is_empty() {
            return Err("Update after the recording is stopped and summarized.".into());
        }
    }
    let pending = app.state::<Pending>();
    let update = pending.0.lock().unwrap().take();
    let Some(update) = update else {
        return Err("No update to install. Check again in Settings › About.".into());
    };
    net::allow_update(update.download_url.as_str()).map_err(|e| e.to_string())?;
    if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
        *pending.0.lock().unwrap() = Some(update);
        return Err(format!("Couldn't install the update: {e}"));
    }
    app.restart()
}

/// While `check_updates` is on: check on start, then daily; a new version also gets a
/// notification (once), since Mirona often runs with its window hidden.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<Instant> = None;
        let mut told = String::new();
        loop {
            let on = app.state::<AppState>().engine().config().check_updates;
            if on && last.is_none_or(|t| t.elapsed() >= Duration::from_hours(24)) {
                last = Some(Instant::now());
                match tauri::async_runtime::block_on(check(&app)) {
                    Ok(Some(u)) if u.version != told => {
                        let body = format!("Mirona {} is ready to install.", u.version);
                        app.notification()
                            .builder()
                            .title("Update available")
                            .body(body)
                            .show()
                            .ok();
                        told = u.version;
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("mirona: {e}"),
                }
            }
            // Hourly, so turning the setting on is picked up without a restart.
            std::thread::sleep(Duration::from_hours(1));
        }
    });
}
