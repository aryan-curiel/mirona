//! Runs the `mirona` binary against a temp app dir + sample store (`MIRONA_APP_DIR`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let meetings = tmp.path().join("meetings");
    let mut store = mirona_store::Store::open(&meetings).unwrap();
    mirona_store::sample::seed(&mut store).unwrap();
    config(tmp.path(), None);
    tmp
}

fn config(tmp: &Path, models: Option<PathBuf>) {
    let cfg = mirona_providers::config::Config {
        meetings_dir: Some(tmp.join("meetings")),
        models_dir: models,
        ..Default::default()
    };
    cfg.save(&tmp.join("app/config.toml")).unwrap();
}

fn mirona(tmp: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mirona"))
        .env("MIRONA_APP_DIR", tmp.join("app"))
        .args(args)
        .output()
        .unwrap()
}

/// stdout of a command that must succeed.
fn ok(tmp: &Path, args: &[&str]) -> String {
    let out = mirona(tmp, args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{args:?}: {stderr}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn list_show_search_actions_reindex() {
    let tmp = fixture();
    let t = tmp.path();

    let list = ok(t, &["list"]);
    assert_eq!(list.lines().count(), 4, "{list}");
    assert!(list.lines().next().unwrap().starts_with("claims-desk"));

    let show = ok(t, &["show", "corvo-review"]);
    assert!(show.starts_with("# Corvo Live product review"), "{show}");
    assert!(show.contains("- [ ] Review onboarding copy for the mobile app · Aryan · due Wed"));
    assert!(show.contains("[7:02] Aryan: I'll review the onboarding copy"));

    let found = ok(t, &["search", "onboarding", "copy"]);
    assert!(found.contains("(corvo-review)"), "{found}");
    assert!(found.contains("[6:45] [Onboarding] [copy]"), "{found}");
    assert_eq!(ok(t, &["search", "zebra"]), "");

    let actions = ok(t, &["actions", "--client", "CORVO", "--open"]);
    assert_eq!(actions.lines().count(), 1, "{actions}");

    // reindex rebuilds the app's index file from the meeting folders.
    let index = t.join("meetings/.mirona/index.sqlite");
    std::fs::remove_file(&index).unwrap();
    ok(t, &["list"]);
    assert!(!index.exists(), "reads must not write the app's index");
    assert!(ok(t, &["reindex"]).starts_with("Indexed 4 meetings"));
    assert!(index.is_file());

    let out = mirona(t, &["show", "nope"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no meeting nope"));
}

#[test]
fn stop_talks_to_the_recorder_through_files() {
    let tmp = fixture();
    let app = tmp.path().join("app");

    let out = mirona(tmp.path(), &["stop"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("not recording"));

    // A live recorder picks up `recording.stop`; play its part from a thread.
    std::fs::write(app.join("recording.lock"), "4242").unwrap();
    let stop = app.join("recording.stop");
    let recorder = std::thread::spawn(move || {
        while std::fs::remove_file(&stop).is_err() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
    assert!(ok(tmp.path(), &["stop"]).contains("Stopping the recording (pid 4242)"));
    recorder.join().unwrap();

    // Nobody answers: a stale lock from a crashed recorder gets cleared.
    assert!(ok(tmp.path(), &["stop"]).contains("cleared the stale lock"));
    assert!(!app.join("recording.lock").exists());
    assert!(!app.join("recording.stop").exists());
}

/// Needs the Parakeet + Silero models (`MIRONA_MODELS_DIR`, default the spike's) and Ollama
/// serving qwen3:4b for the summary.
#[test]
#[ignore = "needs models and Ollama"]
fn import_transcribes_and_summarizes() {
    let tmp = fixture();
    let models = std::env::var_os("MIRONA_MODELS_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/parakeet/models"),
        PathBuf::from,
    );
    config(tmp.path(), Some(models));
    let audio = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mirona-capture/tests/fixtures/librispeech-2utt.wav");
    let out = ok(tmp.path(), &["import", audio.to_str().unwrap()]);
    println!("{out}");
    assert!(out.starts_with("\n# librispeech-2utt"), "{out}");

    let list = ok(tmp.path(), &["list"]);
    assert_eq!(list.lines().count(), 5, "{list}");
    let id = list
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let show = ok(tmp.path(), &["show", id]);
    assert!(show.contains("[0:0"), "{show}");
}

/// CI-safe: the fake classifier on the built-in public set, plus the user's corrections.
#[test]
fn eval_runs_offline_on_the_public_set_and_own_labels() {
    let tmp = fixture();
    let t = tmp.path();
    let out = ok(t, &["eval", "--with", "fake"]);
    assert!(out.starts_with("# Classifier eval: 50 examples"), "{out}");
    assert!(out.contains("## fake (local)"), "{out}");
    assert!(out.contains("| action | 50 |"), "{out}");
    assert!(out.contains("| client | 50 |"), "{out}");
    assert!(out.contains("p50"), "{out}");

    let labels = t.join("meetings/.mirona/labels.jsonl");
    std::fs::write(
        &labels,
        r#"{"id":"m-1-0","text":"Me: I'll send it Friday","labels":{"action":"yes"}}"#,
    )
    .unwrap();
    let out = ok(t, &["eval", "--with", "fake", "--limit", "1000"]);
    assert!(out.starts_with("# Classifier eval: 51 examples"), "{out}");
    assert!(out.contains("| action | 51 |"), "{out}");
    let out = ok(t, &["eval", "--with", "fake", "--no-labels", "--context"]);
    assert!(out.starts_with("# Classifier eval: 50 examples"), "{out}");
    assert!(!mirona(t, &["eval", "--with", "nope"]).status.success());
}
