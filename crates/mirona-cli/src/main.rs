//! `mirona` CLI (NGR-149): the same engine and meeting folders as the app, for scripts,
//! terminals and testing the backend without the UI.
//!
//! Subcommands: `record` / `stop`, `list`, `show`, `search`, `actions`, `import`, `reindex`,
//! `retention`, `delete`, `mcp` (on stdio, or `--http` on localhost) and `eval` (the
//! classifier eval, see `eval.rs`). `--privacy` forces privacy mode for one run.
//!
//! Recording goes through [`mirona_engine::Engine::record`] and holds
//! [`mirona_engine::RecordingLock`], so the app and the CLI never record into the same folder
//! at once; `stop` drops a `recording.stop` file that the recording process watches for.
//! Readers open the store with [`mirona_store::Store::open_snapshot`] and never rewrite the
//! app's index.
//!
//! `MIRONA_APP_DIR` overrides the app folder (where `config.toml` lives).

mod eval;

use clap::{Parser, Subcommand};
use mirona_capture::Permission;
use mirona_core::{ActionItem, Meeting, PipelineEvent};
use mirona_engine::Engine;
use mirona_mcp::at;
use mirona_store::Store;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "mirona", version, about = "Local-first meeting memory")]
struct Cli {
    /// Privacy mode for this run, whatever config.toml says: only this machine is reachable
    #[arg(long, global = true)]
    privacy: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Record mic + system audio until Ctrl-C or `mirona stop`, then summarize
    Record,
    /// Stop the recording running in another terminal
    Stop,
    /// List meetings, newest first
    List,
    /// Summary, action items, notes and transcript of one meeting
    Show { id: String },
    /// Search transcripts, titles, tags and notes by keywords and meaning; filter by meeting
    /// tag, dates, speaker or line tag
    Search {
        query: Vec<String>,
        /// Only meetings with this tag, e.g. project:setr (part of it is enough)
        #[arg(long)]
        meeting_tag: Option<String>,
        /// First day, YYYY-MM-DD
        #[arg(long)]
        from: Option<String>,
        /// Last day, YYYY-MM-DD
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        speaker: Option<String>,
        /// action, decision, open-question, small-talk or sensitive
        #[arg(long)]
        tag: Option<String>,
        /// Show lines tagged sensitive too
        #[arg(long)]
        sensitive: bool,
    },
    /// Action items across meetings
    Actions {
        /// Only items from meetings with this tag (case-insensitive, part of it is enough)
        #[arg(long)]
        tag: Option<String>,
        /// Only items not done yet
        #[arg(long)]
        open: bool,
    },
    /// Transcribe and summarize an audio file (wav, mp3, m4a, flac)
    Import { audio: PathBuf },
    /// Rebuild the search index from the meeting files
    Reindex,
    /// Apply the retention rules now (old recordings, old meetings with a tag); also logged
    /// to `<meetings>/.mirona/retention.log`
    Retention,
    /// Delete a meeting everywhere (files, index, embeddings, voices, corrections), or every
    /// meeting with a tag
    Delete {
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        tag: Option<String>,
    },
    /// Compress recordings still kept as WAV (older meetings, crashed sessions) to Opus
    Compress,
    /// Run the MCP server on stdio (for Claude Code / Claude Desktop), or on localhost HTTP
    Mcp {
        /// Serve streamable HTTP on 127.0.0.1 instead of stdio. Clients need a token from
        /// Settings (`mcp_tokens` in config.toml)
        #[arg(long)]
        http: bool,
        /// HTTP port. Default: `mcp_port` from config.toml
        #[arg(long, requires = "http")]
        port: Option<u16>,
    },
    /// Show privacy mode, or turn it on/off (the same setting as the app's switch)
    Privacy {
        #[arg(value_parser = ["on", "off"])]
        state: Option<String>,
    },
    /// Score the classifiers on labelled examples: the public synthetic set plus your own
    /// corrections. Accuracy, precision/recall per question, latency
    Eval {
        /// Labelled JSONL (`{"id", "text", "labels": {...}}`) instead of the built-in public set
        #[arg(long)]
        set: Option<PathBuf>,
        /// Classifier to score: jev, local-llm or fake (offline). Repeatable. Default: the
        /// configured one plus the local baseline
        #[arg(long = "with")]
        with: Vec<String>,
        /// Leave out your own corrections (`<meetings>/.mirona/labels.jsonl`)
        #[arg(long)]
        no_labels: bool,
        /// Ask each example with the previous one's last two lines in front (context bleed)
        #[arg(long)]
        context: bool,
        /// Only the first N examples
        #[arg(long)]
        limit: Option<usize>,
    },
}

/// `mirona mcp --http`: serves until killed.
fn mcp_http(app_dir: &Path, port: Option<u16>) -> Result {
    let cfg = mirona_mcp::load(app_dir)?.0;
    if !cfg.mcp_enabled {
        return Err(mirona_mcp::DISABLED.into());
    }
    let server = mirona_mcp::serve_http(app_dir.to_path_buf(), port.unwrap_or(cfg.mcp_port))?;
    eprintln!(
        "mirona: MCP on http://127.0.0.1:{}{}",
        server.port,
        mirona_mcp::HTTP_PATH
    );
    if cfg.mcp_tokens.is_empty() {
        eprintln!("mirona: no client tokens yet: add one in Settings › Agents (MCP)");
    }
    server.wait();
    Ok(())
}

fn main() {
    let app_dir = mirona_mcp::app_dir();
    let cli = Cli::parse();
    if cli.privacy {
        // SAFETY: set before any other thread exists.
        unsafe { std::env::set_var(mirona_providers::config::PRIVACY_ENV, "1") };
    }
    let res = match cli.cmd {
        Cmd::Record => record(&app_dir),
        Cmd::Stop => stop(&app_dir),
        Cmd::List => list(&app_dir),
        Cmd::Show { id } => show(&app_dir, &id),
        Cmd::Search {
            query,
            meeting_tag,
            from,
            to,
            speaker,
            tag,
            sensitive,
        } => search(
            &app_dir,
            &mirona_mcp::SearchArgs {
                query: query.join(" "),
                meeting_tag,
                from,
                to,
                speaker,
                tag,
                include_sensitive: Some(sensitive),
            },
        ),
        Cmd::Actions { tag, open } => actions(&app_dir, tag.as_deref(), open),
        Cmd::Import { audio } => import(&app_dir, &audio),
        Cmd::Reindex => reindex(&app_dir),
        Cmd::Retention => retention(&app_dir),
        Cmd::Delete { id, tag } => delete(&app_dir, id.as_deref(), tag.as_deref()),
        Cmd::Compress => compress(&app_dir),
        Cmd::Mcp { http: false, .. } => mirona_mcp::serve(),
        Cmd::Mcp { http: true, port } => mcp_http(&app_dir, port),
        Cmd::Privacy { state } => privacy(&app_dir, state.as_deref()),
        Cmd::Eval {
            set,
            with,
            no_labels,
            context,
            limit,
        } => run_eval(&app_dir, set.as_deref(), with, no_labels, context, limit),
    };
    if let Err(e) = res {
        eprintln!("mirona: {e}");
        std::process::exit(1);
    }
}

type Result<T = ()> = std::result::Result<T, String>;

/// A read-only view that never touches the app's index (decrypted with the keychain key when
/// encryption is on).
fn snapshot(app_dir: &Path) -> Result<Store> {
    let (cfg, paths) = mirona_mcp::load(app_dir)?;
    Store::open_snapshot_with(paths.meetings, mirona_engine::keep::store_key(&cfg)?)
}

fn list(app_dir: &Path) -> Result {
    for m in snapshot(app_dir)?.list_meetings() {
        println!(
            "{:<22} {:<12} {:<34} {}",
            m.id,
            m.when,
            m.title,
            m.tags.join(", ")
        );
    }
    Ok(())
}

fn show(app_dir: &Path, id: &str) -> Result {
    let store = snapshot(app_dir)?;
    let m = store.meeting(id)?;
    print_digest(&m, &store.action_items());
    println!("\n## Transcript");
    print!("{}", mirona_mcp::transcript(&m, None, None, false));
    Ok(())
}

fn print_digest(m: &Meeting, items: &[ActionItem]) {
    println!(
        "# {}\n{} · {} · {} min · {}",
        m.title,
        m.tags.join(", "),
        m.date_label,
        m.duration_min,
        m.id
    );
    if let Some(s) = &m.summary {
        println!("\n{}", s.overview);
        for (head, list) in [
            ("Decisions", &s.decisions),
            ("Open questions", &s.open_questions),
        ] {
            if !list.is_empty() {
                println!("\n## {head}");
                for d in list {
                    println!("- [{}] {}", at(d.at_ms), d.text);
                }
            }
        }
    }
    let items: Vec<_> = items.iter().filter(|a| a.meeting_id == m.id).collect();
    if !items.is_empty() {
        println!("\n## Action items");
        for a in items {
            print_item(a);
        }
    }
    if !m.notes.raw.is_empty() {
        println!("\n## Notes");
        for n in &m.notes.raw {
            println!("- {n}");
        }
    }
}

fn print_item(a: &ActionItem) {
    let done = if a.done { "x" } else { " " };
    let due = a
        .due
        .as_deref()
        .map(|d| format!(" · due {d}"))
        .unwrap_or_default();
    println!("- [{done}] {} · {}{due}", a.text, a.owner);
}

fn search(app_dir: &Path, args: &mirona_mcp::SearchArgs) -> Result {
    let store = snapshot(app_dir)?;
    let (cfg, _) = mirona_mcp::load(app_dir)?;
    let emb = mirona_engine::search::embedder(&cfg);
    let v = mirona_mcp::search(&store, emb.as_ref(), args)?;
    let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    for m in v["moments"].as_array().into_iter().flatten() {
        println!(
            "{} · {} ({}) [{}] {}: {}",
            s(&m["title"]),
            s(&m["date"]),
            s(&m["meeting_id"]),
            s(&m["at"]),
            s(&m["speaker"]),
            s(&m["snippet"])
        );
    }
    if v["moments"].as_array().is_some_and(Vec::is_empty) {
        for m in v["meetings"].as_array().into_iter().flatten() {
            println!(
                "{} · {} · {} ({})",
                s(&m["title"]),
                m["tags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(s)
                    .collect::<Vec<_>>()
                    .join(", "),
                s(&m["date"]),
                s(&m["meeting_id"])
            );
        }
    }
    if v["search_by_meaning"] == false && !args.query.trim().is_empty() {
        eprintln!(
            "(keywords only: run Ollama with `ollama pull nomic-embed-text` to search by meaning)"
        );
    }
    Ok(())
}

fn actions(app_dir: &Path, tag: Option<&str>, open: bool) -> Result {
    let tag = tag.unwrap_or_default().to_lowercase();
    for a in snapshot(app_dir)?.action_items() {
        let tagged = tag.is_empty() || a.tags.iter().any(|t| t.to_lowercase().contains(&tag));
        if tagged && !(open && a.done) {
            print_item(&a);
        }
    }
    Ok(())
}

fn run_eval(
    app_dir: &Path,
    set: Option<&Path>,
    mut with: Vec<String>,
    no_labels: bool,
    context: bool,
    limit: Option<usize>,
) -> Result {
    use mirona_engine::classify::AnyClassifier;
    use mirona_providers::{fake::Fake, jev::Jev, ollama::Ollama};
    let (cfg, _) = mirona_mcp::load(app_dir)?;
    let labels = if no_labels {
        None
    } else {
        let s = snapshot(app_dir)?;
        Some((s.labels_path(), s.labels_text()?))
    };
    let labels = labels.as_ref().map(|(p, t)| (p.as_path(), t.as_str()));
    let mut examples = eval::load(set, labels)?;
    examples.truncate(limit.unwrap_or(usize::MAX));
    if with.is_empty() {
        // The configured classifier (without reading the keychain yet) and the local baseline.
        let configured = mirona_engine::stage(&cfg, "classification", "jev");
        if configured == "jev" && !cfg.privacy {
            with.push("jev".into());
        }
        with.push("local-llm".into());
    }
    println!(
        "# Classifier eval: {} examples
",
        examples.len()
    );
    for id in &with {
        let report = match id.as_str() {
            "jev" if cfg.privacy => {
                return Err(
                    "privacy mode is on, so Jev can't be reached: `mirona privacy off`".into(),
                );
            }
            "jev" => {
                // The keychain key (Settings → API keys), or TYPESAFE_API_KEY for dev and CI.
                let key = mirona_providers::keychain::api_key("jev")
                    .ok()
                    .flatten()
                    .or_else(|| std::env::var("TYPESAFE_API_KEY").ok())
                    .ok_or("no Jev key: add it in Settings → API keys or set TYPESAFE_API_KEY")?;
                let c = AnyClassifier::Jev(Jev::new(key));
                (
                    eval::name(&c) + " " + mirona_providers::jev::MODEL,
                    eval::run(&c, &examples, context),
                )
            }
            "local-llm" => {
                let c = Ollama::default();
                (
                    eval::name(&c) + " " + mirona_providers::ollama::MODEL,
                    eval::run(&c, &examples, context),
                )
            }
            "fake" => {
                let c = Fake("fake");
                (eval::name(&c), eval::run(&c, &examples, context))
            }
            other => {
                return Err(format!(
                    "unknown classifier {other}: use jev, local-llm or fake"
                ));
            }
        };
        println!("{}", report.1.markdown(&report.0));
    }
    Ok(())
}

fn privacy(app_dir: &Path, state: Option<&str>) -> Result {
    use mirona_providers::config::Config;
    let path = mirona_engine::config_path(app_dir);
    if let Some(state) = state {
        // Read the file itself: `--privacy` must not be written back as the saved setting.
        let mut cfg = Config::load(&path)?;
        if std::env::var_os(mirona_providers::config::PRIVACY_ENV).is_some() {
            return Err("`mirona privacy on|off` saves the setting; drop --privacy".into());
        }
        cfg.privacy = state == "on";
        cfg.save(&path)?;
    }
    let on = Config::load(&path)?.privacy;
    println!(
        "Privacy mode is {}",
        if on {
            "on: only this machine is reachable (Ollama, local models)"
        } else {
            "off: cloud providers can be used"
        }
    );
    Ok(())
}

fn retention(app_dir: &Path) -> Result {
    let engine = Engine::open(app_dir, &mirona_engine::documents_dir())?;
    let done = engine.retention()?;
    if done.is_empty() {
        println!("Nothing to remove.");
    }
    for line in done {
        println!("{line}");
    }
    Ok(())
}

fn delete(app_dir: &Path, id: Option<&str>, tag: Option<&str>) -> Result {
    let engine = Engine::open(app_dir, &mirona_engine::documents_dir())?;
    match (id, tag) {
        (Some(id), _) => {
            engine.store.lock().unwrap().delete_meeting(id)?;
            println!("Deleted {id}.");
        }
        (None, Some(t)) => println!("Deleted {} meetings tagged {t}.", engine.delete_tagged(t)?),
        (None, None) => return Err("say which: `mirona delete <id>` or `--tag <tag>`".into()),
    }
    Ok(())
}

fn reindex(app_dir: &Path) -> Result {
    let (cfg, paths) = mirona_mcp::load(app_dir)?;
    let store = Store::open_with(paths.meetings, mirona_engine::keep::store_key(&cfg)?)?;
    for e in store.load_errors() {
        eprintln!("skipped: {e}");
    }
    let n = store.list_meetings().len();
    println!("Indexed {n} meetings in {}", store.root().display());
    Ok(())
}

fn compress(app_dir: &Path) -> Result {
    let engine = Engine::open(app_dir, &mirona_engine::documents_dir())?;
    let errors = engine.compress_recordings();
    for e in &errors {
        eprintln!("kept as WAV: {e}");
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("{} recordings not compressed", errors.len()))
    }
}

fn import(app_dir: &Path, audio: &Path) -> Result {
    if !audio.is_file() {
        return Err(format!("{}: no such file", audio.display()));
    }
    let engine = Engine::open(app_dir, &mirona_engine::documents_dir())?;
    eprintln!("Transcribing {}…", audio.display());
    let (tx, printer) = printer();
    let id = engine.import(audio, &tx);
    drop(tx);
    printer.join().ok();
    print_result(&engine, &id?)
}

fn print_result(engine: &Engine, id: &str) -> Result {
    let store = engine.store.lock().unwrap();
    println!();
    print_digest(&store.meeting(id)?, &store.action_items());
    println!("\nSaved in {}", store.meeting_dir(id)?.display());
    Ok(())
}

/// Prints pipeline events as they arrive, until every sender is gone.
fn printer() -> (Sender<PipelineEvent>, std::thread::JoinHandle<()>) {
    let (tx, rx) = channel();
    let h = std::thread::spawn(move || {
        for ev in rx {
            match ev {
                PipelineEvent::Transcribed {
                    segment, speaker, ..
                } => println!(
                    "[{}] {}: {}",
                    at(segment.start_ms),
                    speaker.name,
                    segment.text
                ),
                PipelineEvent::Notice { message, error, .. } => {
                    eprintln!("{}{message}", if error { "error: " } else { "" });
                }
                PipelineEvent::Summarized { .. } => eprintln!("Summary written."),
                PipelineEvent::Importing {
                    fraction: Some(f), ..
                } => eprint!("\r{:.0}% transcribed", f * 100.0),
                _ => {}
            }
        }
    });
    (tx, h)
}

// --- Recording, and stopping it from another terminal ---
//
// `record` writes `recording.lock` (its pid) in the app dir; `mirona stop` creates
// `recording.stop`, which the recorder polls for and deletes. Plain files rather than signals,
// so it works the same on Windows.

fn lock_file(app_dir: &Path) -> PathBuf {
    app_dir.join("recording.lock")
}

fn stop_file(app_dir: &Path) -> PathBuf {
    app_dir.join("recording.stop")
}

/// Removes the lock when the recording ends, however it ends.
struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

/// Fails fast when macOS has denied the terminal mic or system-audio access; asks if never
/// asked (the prompt names the terminal app).
fn check_permissions() -> Result {
    use mirona_capture as c;
    ensure(
        "Microphone",
        "Microphone",
        c::mic_permission(),
        c::request_mic_permission,
    )?;
    ensure(
        "System audio",
        "Screen & System Audio Recording",
        c::system_audio_permission(),
        c::request_system_audio_permission,
    )
}

fn ensure(what: &str, pane: &str, status: Permission, request: fn() -> bool) -> Result {
    let ok = match status {
        Permission::Granted => true,
        Permission::Denied => false,
        Permission::NotDetermined => {
            // The prompt may never show (no GUI session, an IDE's terminal): don't hang.
            eprintln!(
                "Asking for {} access: answer the macOS prompt…",
                what.to_lowercase()
            );
            let (tx, rx) = channel();
            std::thread::spawn(move || tx.send(request()).ok());
            rx.recv_timeout(Duration::from_secs(30)).unwrap_or(false)
        }
    };
    if ok {
        return Ok(());
    }
    Err(format!(
        "{what} access is not granted to this terminal. Allow your terminal app in System \
         Settings › Privacy & Security › {pane}, restart the terminal, and try again."
    ))
}

fn record(app_dir: &Path) -> Result {
    let lock = lock_file(app_dir);
    if let Ok(pid) = std::fs::read_to_string(&lock) {
        return Err(format!(
            "already recording (pid {}). Run `mirona stop` (it also clears a stale lock).",
            pid.trim()
        ));
    }
    check_permissions()?;
    let engine = Engine::open(app_dir, &mirona_engine::documents_dir())?;
    std::fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
    std::fs::write(&lock, std::process::id().to_string()).map_err(|e| e.to_string())?;
    let _lock = Lock(lock);
    std::fs::remove_file(stop_file(app_dir)).ok();

    let (tx, printer) = printer();
    let session = engine.record(tx.clone())?;
    eprintln!(
        "Recording {}. Press Ctrl-C or run `mirona stop` to finish.",
        session.id
    );
    let (ctl_tx, ctl_rx) = channel();
    ctrlc::set_handler(move || {
        ctl_tx.send(()).ok();
    })
    .map_err(|e| e.to_string())?;
    wait_for_stop(&ctl_rx, &stop_file(app_dir));

    eprintln!("Stopping… writing the summary.");
    let id = session.stop(&engine, &tx);
    drop(tx);
    printer.join().ok();
    print_result(&engine, &id?)
}

fn wait_for_stop(ctrl_c: &Receiver<()>, stop: &Path) {
    loop {
        match ctrl_c.recv_timeout(Duration::from_millis(250)) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                if std::fs::remove_file(stop).is_ok() {
                    return;
                }
            }
        }
    }
}

fn stop(app_dir: &Path) -> Result {
    let lock = lock_file(app_dir);
    let Ok(pid) = std::fs::read_to_string(&lock) else {
        return Err("not recording".into());
    };
    let pid = pid.trim();
    let stop = stop_file(app_dir);
    std::fs::write(&stop, "").map_err(|e| e.to_string())?;
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(250));
        if !stop.exists() {
            println!("Stopping the recording (pid {pid}); its terminal shows the summary.");
            return Ok(());
        }
    }
    // Nobody picked it up: the recorder died without cleaning up.
    std::fs::remove_file(&stop).ok();
    std::fs::remove_file(&lock).ok();
    println!("No recording answered (pid {pid} is gone); cleared the stale lock.");
    Ok(())
}
