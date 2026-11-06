//! MCP over streamable HTTP (NGR-168): token auth, the activity log and the write tools,
//! against a temp app dir + sample store, speaking raw JSON-RPC over HTTP like a client does.
#![allow(clippy::needless_pass_by_value)] // `json!` literals read better by value

use mirona_providers::config::Config;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Client {
    url: String,
    agent: ureq::Agent,
    token: String,
    session: Option<String>,
    next: u64,
}

impl Client {
    /// POSTs one JSON-RPC message: (status, body as text).
    fn post(&mut self, msg: &Value) -> (u16, String) {
        let mut req = self
            .agent
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", &format!("Bearer {}", self.token));
        if let Some(s) = &self.session {
            req = req.header("Mcp-Session-Id", s);
        }
        let mut resp = req.send(msg.to_string()).unwrap();
        if let Some(s) = resp.headers().get("mcp-session-id") {
            self.session = Some(s.to_str().unwrap().into());
        }
        let status = resp.status().as_u16();
        (status, resp.body_mut().read_to_string().unwrap())
    }

    /// A request's JSON-RPC response (the body is SSE or plain JSON).
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let (status, body) = self.post(&msg);
        assert_eq!(status, 200, "{body}");
        body.lines()
            .map(|l| l.strip_prefix("data:").unwrap_or(l).trim())
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["id"] == id)
            .unwrap_or_else(|| panic!("no response in {body}"))
    }

    fn tools(&mut self) -> Vec<String> {
        let list = self.request("tools/list", json!({}));
        let mut names: Vec<String> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        names.sort_unstable();
        names
    }

    fn call(&mut self, name: &str, args: Value) -> (String, bool) {
        let r = self.request("tools/call", json!({ "name": name, "arguments": args }));
        let res = &r["result"];
        if res.is_null() {
            return (r["error"]["message"].to_string(), true);
        }
        let text = res["content"][0]["text"].as_str().unwrap().to_string();
        (text, res["isError"] == true)
    }
}

/// `<tmp>/app/config.toml` pointing at `<tmp>/meetings` (sample meetings), one client token.
fn fixture() -> (tempfile::TempDir, PathBuf, Config) {
    let tmp = tempfile::tempdir().unwrap();
    let meetings = tmp.path().join("meetings");
    let mut store = mirona_store::Store::open(&meetings).unwrap();
    mirona_store::sample::seed(&mut store).unwrap();
    let mut cfg = Config {
        meetings_dir: Some(meetings),
        mcp_enabled: true,
        mcp_http: true,
        ..Default::default()
    };
    cfg.mcp_tokens.insert("Test Agent".into(), "s3cret".into());
    let app = tmp.path().join("app");
    cfg.save(&mirona_engine::config_path(&app)).unwrap();
    (tmp, app, cfg)
}

fn client(port: u16, token: &str) -> Client {
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    Client {
        url: format!("http://127.0.0.1:{port}{}", mirona_mcp::HTTP_PATH),
        agent,
        token: token.into(),
        session: None,
        next: 1,
    }
}

fn init(c: &mut Client) {
    let init = c.request(
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" } }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "mirona");
    let (status, _) = c.post(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    assert!(status < 300, "{status}");
}

fn save(app: &Path, cfg: &Config) {
    cfg.save(&mirona_engine::config_path(app)).unwrap();
}

#[test]
fn tokens_activity_and_write_tools() {
    let (tmp, app, mut cfg) = fixture();
    let server = mirona_mcp::serve_http(app.clone(), 0).unwrap();

    // No token, a wrong one: 401 before MCP sees the request.
    let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
    for token in ["", "wrong"] {
        let (status, body) = client(server.port, token).post(&ping);
        assert_eq!(status, 401, "{body}");
    }

    let mut c = client(server.port, "s3cret");
    init(&mut c);
    let read = [
        "get_meeting",
        "get_transcript",
        "list_action_items",
        "search_meetings",
    ];
    assert_eq!(c.tools(), read);
    let (text, err) = c.call("get_meeting", json!({ "id": "corvo-review" }));
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert!(
        v["speakers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == "andres"),
        "{v}"
    );

    // Write tools are off: not listed, and calling one is refused.
    let (_, err) = c.call("set_action_item_done", json!({ "id": "a5", "done": true }));
    assert!(err);

    // The activity log names the client from its token; newest first, nothing else.
    let log = mirona_mcp::activity(&app, 10);
    assert_eq!(
        (log[0].client.as_str(), log[0].tool.as_str(), log[0].ok),
        ("Test Agent", "set_action_item_done", false)
    );
    assert_eq!((log[1].tool.as_str(), log[1].ok), ("get_meeting", true));
    assert!(
        !std::fs::read_to_string(mirona_mcp::activity::path(&app))
            .unwrap()
            .contains("corvo-review")
    );

    // Write tools on (config is reread per request): listed, and they change the files.
    cfg.mcp_write = true;
    save(&app, &cfg);
    let mut all = c.tools();
    all.retain(|t| !read.contains(&t.as_str()));
    assert_eq!(all, ["rename_speaker", "set_action_item_done"]);
    for _ in 0..2 {
        let (text, err) = c.call("set_action_item_done", json!({ "id": "a5", "done": true }));
        assert!(!err && text.contains("\"done\":true"), "{text}");
    }
    let (text, err) = c.call(
        "rename_speaker",
        json!({ "meeting_id": "corvo-review", "speaker_id": "andres", "name": "Andrés Vidal" }),
    );
    assert!(!err, "{text}");
    let store = mirona_store::Store::open(tmp.path().join("meetings")).unwrap();
    assert!(store.action_items().iter().any(|a| a.id == "a5" && a.done));
    let m = store.meeting("corvo-review").unwrap();
    assert!(
        m.speakers.iter().any(|s| s.name == "Andrés Vidal"),
        "{:?}",
        m.speakers
    );

    // A revoked token stops working at once.
    cfg.mcp_tokens.clear();
    save(&app, &cfg);
    let (status, _) = c.post(&ping);
    assert_eq!(status, 401);
}
