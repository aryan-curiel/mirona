//! `mirona-mcp`: MCP server over stdio (NGR-148).
//! ponytail: hand-rolled JSON-RPC over stdin lines; move to the official `rmcp` SDK in NGR-148.

use mirona_store::Store;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn tools() -> Value {
    let tool = |name: &str, desc: &str, arg: Option<&str>| {
        let props = arg.map_or(json!({}), |a| json!({ a: { "type": "string" } }));
        json!({ "name": name, "description": desc, "inputSchema": { "type": "object", "properties": props } })
    };
    json!([
        tool(
            "search_meetings",
            "Keyword search over meetings",
            Some("query")
        ),
        tool(
            "get_meeting",
            "Meeting summary, decisions and open questions",
            Some("id")
        ),
        tool(
            "get_transcript",
            "Full transcript of one meeting",
            Some("id")
        ),
        tool(
            "list_action_items",
            "Action items across meetings",
            Some("client")
        ),
    ])
}

fn call(store: &Store, name: &str, args: &Value) -> Result<Value, String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default();
    Ok(match name {
        "search_meetings" => json!(store.search(s("query"))),
        "get_meeting" => {
            let m = store.meeting(s("id"))?;
            json!({ "title": m.title, "client": m.client, "date": m.date_label, "summary": m.summary })
        }
        "get_transcript" => json!(store.meeting(s("id"))?.segments),
        "list_action_items" => json!(
            store
                .action_items()
                .into_iter()
                .filter(|a| s("client").is_empty() || a.client == s("client"))
                .collect::<Vec<_>>()
        ),
        _ => return Err(format!("unknown tool {name}")),
    })
}

fn handle(store: &Store, req: &Value) -> Option<Value> {
    let id = req.get("id")?.clone();
    let result = match req["method"].as_str()? {
        "initialize" => json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "mirona", "version": env!("CARGO_PKG_VERSION") }
        }),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => match call(
            store,
            req["params"]["name"].as_str().unwrap_or(""),
            &req["params"]["arguments"],
        ) {
            Ok(v) => json!({ "content": [{ "type": "text", "text": v.to_string() }] }),
            Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
        },
        _ => {
            return Some(
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "method not found" } }),
            );
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn main() {
    let store = Store::sample();
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(resp) = handle(&store, &req) {
            writeln!(out, "{resp}").ok();
            out.flush().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_calls_tools() {
        let store = Store::sample();
        let r = handle(
            &store,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .unwrap();
        assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 4);
        let r = handle(&store, &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_action_items","arguments":{"client":"Corvo"}}})).unwrap();
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("onboarding")
        );
    }
}
