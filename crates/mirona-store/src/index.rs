//! `<root>/.mirona/index.sqlite` (NGR-140): a derived index, rebuilt from the files at will.
//! Meetings, speakers and action items as plain tables; titles/notes and transcript text in FTS5.

use crate::Result;
use crate::files::Entry;
use mirona_core::{Id, Segment};
use rusqlite::{Connection, params};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

const SCHEMA: &str = "
DROP TABLE IF EXISTS meetings;
DROP TABLE IF EXISTS speakers;
DROP TABLE IF EXISTS action_items;
DROP TABLE IF EXISTS meetings_fts;
DROP TABLE IF EXISTS segments;
CREATE TABLE meetings(id TEXT PRIMARY KEY, dir TEXT NOT NULL, title TEXT NOT NULL,
  client TEXT NOT NULL, created TEXT NOT NULL, duration_min INTEGER NOT NULL);
CREATE TABLE speakers(meeting_id TEXT NOT NULL, speaker_id TEXT NOT NULL, name TEXT NOT NULL);
CREATE TABLE action_items(id TEXT NOT NULL, meeting_id TEXT NOT NULL, text TEXT NOT NULL,
  owner TEXT NOT NULL, due TEXT, at_ms INTEGER, done INTEGER NOT NULL);
CREATE VIRTUAL TABLE meetings_fts USING fts5(meeting_id UNINDEXED, title, client, notes,
  tokenize = 'unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE segments USING fts5(meeting_id UNINDEXED, segment_id UNINDEXED,
  start_ms UNINDEXED, text, tokenize = 'unicode61 remove_diacritics 2');
";

/// A transcript moment that matched a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub meeting_id: Id,
    pub segment_id: Id,
    pub start_ms: u32,
    /// The matching text with the hit in `[brackets]`.
    pub snippet: String,
}

#[allow(clippy::needless_pass_by_value)] // used as `.map_err(e)`
fn e(err: rusqlite::Error) -> String {
    format!("index: {err}")
}

/// Opens the index; an unreadable file is derived data, so it is deleted and recreated.
pub fn open(path: &Path) -> Result<Connection> {
    let try_open = || -> rusqlite::Result<Connection> {
        let c = Connection::open(path)?;
        c.query_row("PRAGMA schema_version", [], |_| Ok(()))?;
        Ok(c)
    };
    try_open().or_else(|_| {
        std::fs::remove_file(path).ok();
        try_open().map_err(e)
    })
}

pub fn rebuild(db: &mut Connection, entries: &[Entry]) -> Result<()> {
    let tx = db.transaction().map_err(e)?;
    tx.execute_batch(SCHEMA).map_err(e)?;
    for en in entries {
        insert(&tx, en)?;
    }
    tx.commit().map_err(e)
}

/// Replace everything indexed for one meeting.
pub fn reindex_meeting(db: &mut Connection, en: &Entry) -> Result<()> {
    let tx = db.transaction().map_err(e)?;
    delete(&tx, &en.meeting.id)?;
    insert(&tx, en)?;
    tx.commit().map_err(e)
}

/// Drop everything indexed for one meeting.
pub fn remove_meeting(db: &mut Connection, id: &str) -> Result<()> {
    let tx = db.transaction().map_err(e)?;
    delete(&tx, id)?;
    tx.commit().map_err(e)
}

fn delete(tx: &Connection, id: &str) -> Result<()> {
    for t in [
        "meetings",
        "speakers",
        "action_items",
        "meetings_fts",
        "segments",
    ] {
        let col = if t == "meetings" { "id" } else { "meeting_id" };
        tx.execute(&format!("DELETE FROM {t} WHERE {col} = ?1"), [id])
            .map_err(e)?;
    }
    Ok(())
}

fn insert(db: &Connection, en: &Entry) -> Result<()> {
    let m = &en.meeting;
    db.execute(
        "INSERT INTO meetings VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            m.id,
            en.dir.to_string_lossy(),
            m.title,
            m.client,
            en.created,
            m.duration_min
        ],
    )
    .map_err(e)?;
    db.execute(
        "INSERT INTO meetings_fts VALUES (?1, ?2, ?3, ?4)",
        params![m.id, m.title, m.client, m.notes.raw.join("\n")],
    )
    .map_err(e)?;
    for s in &m.speakers {
        db.execute(
            "INSERT INTO speakers VALUES (?1, ?2, ?3)",
            params![m.id, s.id, s.name],
        )
        .map_err(e)?;
    }
    for a in &en.items {
        db.execute(
            "INSERT INTO action_items VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![a.id, m.id, a.text, a.owner, a.due, a.at_ms, a.done],
        )
        .map_err(e)?;
    }
    for s in &m.segments {
        insert_segment(db, &m.id, s)?;
    }
    Ok(())
}

/// Redacted text is never indexed, so search can't surface a password.
pub fn insert_segment(db: &Connection, meeting_id: &str, s: &Segment) -> Result<()> {
    if s.redacted {
        return Ok(());
    }
    db.execute(
        "INSERT INTO segments VALUES (?1, ?2, ?3, ?4)",
        params![meeting_id, s.id, s.start_ms, s.text],
    )
    .map(|_| ())
    .map_err(e)
}

/// User text → FTS5 query: every word must match, as a prefix, with FTS syntax neutralised.
fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn meeting_ids(db: &Connection, q: &str) -> Result<HashSet<Id>> {
    let mut st = db
        .prepare(
            "SELECT meeting_id FROM meetings_fts WHERE meetings_fts MATCH ?1
             UNION SELECT meeting_id FROM segments WHERE segments MATCH ?1",
        )
        .map_err(e)?;
    st.query_map([fts_query(q)], |r| r.get(0))
        .map_err(e)?
        .collect::<rusqlite::Result<_>>()
        .map_err(e)
}

pub fn hits(db: &Connection, q: &str) -> Result<Vec<Hit>> {
    let mut st = db
        .prepare(
            "SELECT meeting_id, segment_id, start_ms, snippet(segments, 3, '[', ']', '…', 12)
             FROM segments WHERE segments MATCH ?1 ORDER BY rank",
        )
        .map_err(e)?;
    st.query_map([fts_query(q)], |r| {
        Ok(Hit {
            meeting_id: r.get(0)?,
            segment_id: r.get(1)?,
            start_ms: r.get(2)?,
            snippet: r.get(3)?,
        })
    })
    .map_err(e)?
    .collect::<rusqlite::Result<_>>()
    .map_err(e)
}
