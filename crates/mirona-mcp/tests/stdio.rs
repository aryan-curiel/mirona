//! Spawns the `mirona-mcp` binary over stdio against a temp app dir + sample store and speaks
//! raw JSON-RPC to it, the way Claude Code does.
#![allow(clippy::needless_pass_by_value)] // `json!` literals read better by value

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Client {
    fn spawn(app_dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mirona-mcp"))
            .env("MIRONA_APP_DIR", app_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next: 1,
        }
    }

    fn send(&mut self, msg: &Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let resp: Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("bad: {line}"));
        assert_eq!(resp["id"], id, "{resp}");
        resp
    }

    /// Calls a tool; returns its text and whether it was an error.
    fn call(&mut self, name: &str, args: Value) -> (String, bool) {
        let r = self.request("tools/call", json!({ "name": name, "arguments": args }));
        let res = &r["result"];
        let text = res["content"][0]["text"].as_str().unwrap().to_string();
        (text, res["isError"] == true)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// `<tmp>/app/config.toml` pointing at `<tmp>/meetings`, seeded with the sample meetings.
fn fixture(mcp_enabled: bool) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let meetings = tmp.path().join("meetings");
    let mut store = mirona_store::Store::open(&meetings).unwrap();
    mirona_store::sample::seed(&mut store).unwrap();
    let mut cfg = mirona_providers::config::Config {
        meetings_dir: Some(meetings),
        mcp_enabled,
        ..Default::default()
    };
    // Keywords only: results must not depend on a local Ollama.
    cfg.providers.insert("embeddings".into(), "off".into());
    cfg.save(&tmp.path().join("app/config.toml")).unwrap();
    tmp
}

#[test]
fn initialize_list_and_call_every_tool() {
    let tmp = fixture(true);
    let mut c = Client::spawn(&tmp.path().join("app"));

    let init = c.request(
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" } }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "mirona");
    c.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    let list = c.request("tools/list", json!({}));
    let mut names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "get_meeting",
            "get_transcript",
            "list_action_items",
            "search_meetings"
        ]
    );

    // search: transcript moments with timestamps.
    let (text, err) = c.call("search_meetings", json!({ "query": "onboarding" }));
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    let m = &v["moments"][0];
    assert_eq!(m["meeting_id"], "corvo-review");
    assert_eq!(m["at"], "7:02");
    assert!(m["snippet"].as_str().unwrap().contains("[onboarding]"));
    assert_eq!(v["meetings"][0]["meeting_id"], "corvo-review");

    // get_meeting: compact, no transcript.
    let (text, err) = c.call("get_meeting", json!({ "id": "corvo-review" }));
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["client"], "Corvo");
    assert!(v["overview"].as_str().unwrap().contains("Corvo Live"));
    assert_eq!(v["action_items"][0]["owner"], "Aryan");
    assert!(v.get("segments").is_none());

    // get_transcript: whole and a range.
    let (all, _) = c.call("get_transcript", json!({ "id": "corvo-review" }));
    assert_eq!(all.lines().count(), 4, "{all}");
    let (part, _) = c.call(
        "get_transcript",
        json!({ "id": "corvo-review", "from": "6:00", "to": "7:00" }),
    );
    assert_eq!(part.lines().count(), 2, "{part}");
    assert!(part.contains("[6:45]"));

    // list_action_items: client filter is case-insensitive.
    let count = |t: &str| serde_json::from_str::<Value>(t).unwrap()["count"].clone();
    let (text, _) = c.call("list_action_items", json!({ "client": "corvo" }));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["count"], 1);
    assert_eq!(v["items"][0]["meeting_id"], "corvo-review");

    // Fresh data: writes made by someone else (the app) show up on the next call.
    let mut store = mirona_store::Store::open(tmp.path().join("meetings")).unwrap();
    store.toggle_action_item("a5").unwrap();
    store.set_title("corvo-review", "Renamed review").unwrap();
    let args = json!({ "client": "corvo", "open_only": true });
    assert_eq!(count(&c.call("list_action_items", args).0), 0);
    let (text, _) = c.call("get_meeting", json!({ "id": "corvo-review" }));
    assert!(text.contains("Renamed review"), "{text}");

    // Errors come back as tool errors, not crashes.
    let (text, err) = c.call("get_meeting", json!({ "id": "nope" }));
    assert!(err && text.contains("no meeting"), "{text}");
    let (_, err) = c.call(
        "get_transcript",
        json!({ "id": "corvo-review", "from": "x" }),
    );
    assert!(err);

    // Every call is logged as coming from stdio.
    let log = mirona_mcp::activity(&tmp.path().join("app"), 100);
    assert_eq!(log.len(), 9);
    assert!(log.iter().all(|a| a.client == "stdio"));
    assert_eq!((log[0].tool.as_str(), log[0].ok), ("get_transcript", false));
}

#[test]
fn refuses_to_serve_when_disabled() {
    let tmp = fixture(false);
    let out = Command::new(env!("CARGO_BIN_EXE_mirona-mcp"))
        .env("MIRONA_APP_DIR", tmp.path().join("app"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("turned off"));
}
