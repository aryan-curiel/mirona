//! Prints the demo meetings (`sample::seed`) as the app's commands return them, as one JSON
//! object. `app/scripts/shots/` feeds it to the UI to take the docs screenshots.
//! `cargo run -q -p mirona-store --example demo_json > app/scripts/shots/demo.json`; a folder
//! argument keeps the meeting files there (for the docs' file examples).

use serde_json::{Map, Value, json};

fn main() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let root = std::env::args()
        .nth(1)
        .map_or(dir.path().to_path_buf(), Into::into);
    let mut store = mirona_store::Store::open(&root)?;
    mirona_store::sample::seed(&mut store)?;
    let list = store.list_meetings();
    let (mut meetings, mut reviews) = (Map::new(), Map::new());
    for m in &list {
        meetings.insert(m.id.clone(), json!(store.meeting(&m.id)?));
        reviews.insert(m.id.clone(), json!(store.review(&m.id)?));
    }
    // Same shape as `list_people` in app/src-tauri/src/lib.rs.
    let people: Vec<Value> = store
        .people()
        .iter()
        .map(|p| {
            let seen: Vec<_> = list
                .iter()
                .filter(|m| {
                    meetings[&m.id]["speakers"]
                        .as_array()
                        .is_some_and(|s| s.iter().any(|s| s["id"] == p.id.as_str()))
                })
                .collect();
            let mut v = json!(p);
            v["meetings"] = json!(seen.len());
            v["lastSeen"] = json!(seen.first());
            v
        })
        .collect();
    let out = json!({
      "list": list,
      "meetings": meetings,
      "reviews": reviews,
      "actions": store.action_items(),
      "people": people,
      "knownNames": store.known_names(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
    );
    Ok(())
}
