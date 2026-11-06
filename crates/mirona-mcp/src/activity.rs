//! Which agent called which tool (NGR-168): one JSON line per call in
//! `<app_dir>/mcp-activity.jsonl`. Never arguments or results: they can hold meeting content.
//! ponytail: stdio and HTTP servers append without a lock, so two calls trimming at the same
//! instant can lose a line; a lock file if the log ever matters for more than a glance.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Past this size (~9000 calls) the log is cut down to its newest `KEEP` lines (or half, if
/// lines are unusually long).
const MAX_BYTES: u64 = 1024 * 1024;
const KEEP: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    /// RFC 3339, UTC.
    pub time: String,
    /// "stdio", or the HTTP client's name from its token.
    pub client: String,
    pub tool: String,
    pub ok: bool,
}

pub fn path(app_dir: &Path) -> PathBuf {
    app_dir.join("mcp-activity.jsonl")
}

/// Appends one entry. Logging never fails a tool call: errors go to stderr.
pub fn record(app_dir: &Path, client: &str, tool: &str, ok: bool) {
    let time = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let line = serde_json::to_string(&Activity {
        time,
        client: client.into(),
        tool: tool.into(),
        ok,
    })
    .unwrap_or_default();
    if let Err(e) = append(&path(app_dir), &line) {
        eprintln!("mirona-mcp: activity log: {e}");
    }
}

fn append(p: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)?;
    writeln!(f, "{line}")?;
    if f.metadata()?.len() > MAX_BYTES {
        let all = std::fs::read_to_string(p)?;
        let lines: Vec<&str> = all.lines().collect();
        let keep = KEEP.min(lines.len() / 2);
        let tail = lines[lines.len() - keep..].join("\n");
        std::fs::write(p, tail + "\n")?;
    }
    Ok(())
}

/// The newest `limit` entries, newest first. A missing log is empty.
pub fn activity(app_dir: &Path, limit: usize) -> Vec<Activity> {
    let all = std::fs::read_to_string(path(app_dir)).unwrap_or_default();
    all.lines()
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), "stdio", "get_meeting", true);
        record(dir.path(), "Claude Desktop", "search_meetings", false);
        let a = activity(dir.path(), 10);
        assert_eq!(a.len(), 2);
        assert_eq!(
            (a[0].client.as_str(), a[0].tool.as_str(), a[0].ok),
            ("Claude Desktop", "search_meetings", false)
        );
        assert!(a[0].time.ends_with('Z'), "{}", a[0].time);

        let long = "x".repeat(2000);
        for _ in 0..600 {
            record(dir.path(), &long, "t", true);
        }
        let len = std::fs::metadata(path(dir.path())).unwrap().len();
        assert!(len <= MAX_BYTES, "{len}");
        assert!(activity(dir.path(), 1000).len() < 600);
        assert_eq!(activity(dir.path(), 1)[0].client, long);
    }
}
