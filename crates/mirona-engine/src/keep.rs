//! Encryption key and retention (NGR-165).
//!
//! The store key is 32 random bytes kept in the OS keychain (service "mirona", account
//! `store-key`); `config.toml` only says `encrypt = true`. Turning encryption off rewrites the
//! files in plain form and keeps the key, so backups made while it was on stay readable.
//! Retention runs on app start and then hourly (and on `mirona retention`); everything it
//! removes is appended to `<meetings>/.mirona/retention.log`.

use crate::{Engine, Result};
use mirona_providers::config::{Config, Retention, STORE_KEY};
use mirona_providers::keychain;
use mirona_store::Store;
use mirona_store::crypt::Key;
use std::io::Write;

/// The key to open the store with: `None` unless encryption is on.
pub fn store_key(cfg: &Config) -> Result<Option<Key>> {
    if !cfg.encrypt {
        return Ok(None);
    }
    let hex = keychain::api_key(STORE_KEY)
        .map_err(|e| e.to_string())?
        .ok_or("encryption is on but its key is missing from the keychain")?;
    Key::from_hex(&hex).map(Some)
}

fn key_or_new() -> Result<Key> {
    if let Some(hex) = keychain::api_key(STORE_KEY).map_err(|e| e.to_string())? {
        return Key::from_hex(&hex);
    }
    let k = Key::generate()?;
    keychain::set_api_key(STORE_KEY, &k.to_hex()).map_err(|e| e.to_string())?;
    // Read back: a key that didn't reach the keychain must never encrypt anything.
    match keychain::api_key(STORE_KEY).map_err(|e| e.to_string())? {
        Some(h) if h == k.to_hex() => Ok(k),
        _ => Err("couldn't save the encryption key in the keychain".into()),
    }
}

/// One thing retention will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Forget {
    /// The whole meeting (a tag rule).
    Meeting { id: String, why: String },
    /// Only the recording (`audio_days`).
    Audio { id: String, why: String },
}

/// What the rules say to remove now. A tag rule wins over the audio rule; with several
/// tags, the shortest rule applies.
pub fn plan(store: &Store, rules: &Retention) -> Vec<Forget> {
    let mut out = vec![];
    for m in store.list_meetings() {
        let Ok(age) = store.age_days(&m.id) else {
            continue;
        };
        let tag_rule = rules
            .tags
            .iter()
            .filter(|(t, _)| mirona_store::has_tag(&m.tags, t))
            .map(|(t, d)| (i64::from(*d), t))
            .min();
        if let Some((days, tag)) = tag_rule.filter(|(d, _)| age >= *d) {
            out.push(Forget::Meeting {
                id: m.id.clone(),
                why: format!("“{}” ({tag}), {age} days old; rule: {days} days", m.title),
            });
        } else if let Some(days) = rules.audio_days.map(i64::from).filter(|d| age >= *d)
            && store.has_audio(&m.id)
        {
            out.push(Forget::Audio {
                id: m.id.clone(),
                why: format!(
                    "recording of “{}”, {age} days old; rule: {days} days",
                    m.title
                ),
            });
        }
    }
    out
}

impl Engine {
    /// Encrypt the meetings folder (or decrypt it) and save the setting. Never while recording.
    pub fn set_encryption(&self, app_dir: &std::path::Path, on: bool) -> Result<()> {
        let mut cfg = self.config();
        let key = if on { Some(key_or_new()?) } else { None };
        self.store.lock().unwrap().set_encryption(key)?;
        cfg.encrypt = on;
        self.save_config(app_dir, cfg)
    }

    /// Apply the retention rules now. Returns (and logs) one line per removal.
    pub fn retention(&self) -> Result<Vec<String>> {
        let rules = self.config().retention;
        let mut store = self.store.lock().unwrap();
        let mut done = vec![];
        for f in plan(&store, &rules) {
            match f {
                Forget::Meeting { id, why } => {
                    store.delete_meeting(&id)?;
                    done.push(format!("deleted meeting {why}"));
                }
                Forget::Audio { id, why } => {
                    store.remove_audio(&id)?;
                    done.push(format!("deleted {why}"));
                }
            }
        }
        if !done.is_empty() {
            let path = store.root().join(".mirona").join("retention.log");
            let now = jiff::Timestamp::now();
            let mut log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            for line in &done {
                writeln!(log, "{now} {line}").map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
        Ok(done)
    }

    /// Delete every meeting with a tag, logged like retention.
    pub fn delete_tagged(&self, tag: &str) -> Result<usize> {
        let mut store = self.store.lock().unwrap();
        let ids = store.delete_tagged(tag)?;
        let path = store.root().join(".mirona").join("retention.log");
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let now = jiff::Timestamp::now();
            writeln!(
                log,
                "{now} deleted {} meetings tagged {tag} (by hand)",
                ids.len()
            )
            .ok();
        }
        Ok(ids.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirona_core::{Meeting, Note};
    use std::sync::{Arc, Mutex};

    fn meeting(id: &str, tags: &[&str]) -> Meeting {
        Meeting {
            id: id.into(),
            title: format!("Call {id}"),
            tags: tags.iter().map(|t| (*t).into()).collect(),
            when: String::new(),
            date_label: String::new(),
            duration_min: 30,
            language: "English".into(),
            sample: false,
            speakers: vec![],
            segments: vec![],
            answers: None,
            notes: Note {
                raw: vec![],
                raw_at_ms: vec![],
                enhanced: vec![],
                enhanced_by: String::new(),
            },
            summary: None,
        }
    }

    #[test]
    fn retention_drops_old_audio_and_old_tagged_meetings_and_logs_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let day = |n: i64| {
            let t = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(24 * n);
            t.to_zoned(jiff::tz::TimeZone::UTC)
                .strftime("%Y-%m-%dT%H:%M:%S%:z")
                .to_string()
        };
        let bank: &[&str] = &["topic:billing", "project:Northwind Bank"];
        for (id, tags, age) in [
            ("m-new", &["project:Helio"][..], 1),
            ("m-old", &["project:Helio"], 40),
            ("m-bank", bank, 100),
            ("m-bank-new", bank, 5),
        ] {
            store
                .add_meeting(meeting(id, tags), &day(age), vec![])
                .unwrap();
            std::fs::write(store.meeting_dir(id).unwrap().join("audio.caf"), b"a").unwrap();
            store.set_audio(id, "audio.caf").unwrap();
        }
        let rules = Retention {
            audio_days: Some(30),
            tags: [
                ("project:northwind bank".to_string(), 90),
                ("topic:billing".to_string(), 365),
            ]
            .into(),
            ..Retention::default()
        };
        let p = plan(&store, &rules);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(
            p.iter()
                .any(|f| matches!(f, Forget::Audio { id, .. } if id == "m-old"))
        );
        assert!(
            p.iter()
                .any(|f| matches!(f, Forget::Meeting { id, .. } if id == "m-bank"))
        );

        let cfg = Config {
            retention: rules,
            ..Config::default()
        };
        let paths = crate::Paths::resolve(&cfg, dir.path(), dir.path());
        let e = Engine::new(Arc::new(Mutex::new(store)), cfg, paths);
        let done = e.retention().unwrap();
        assert_eq!(done.len(), 2, "{done:?}");
        let s = e.store.lock().unwrap();
        let ids: Vec<String> = s.list_meetings().into_iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(!ids.contains(&"m-bank".to_string()));
        assert!(!s.has_audio("m-old") && s.has_audio("m-new") && s.has_audio("m-bank-new"));
        let log = std::fs::read_to_string(dir.path().join(".mirona/retention.log")).unwrap();
        assert_eq!(log.lines().count(), 2, "{log}");
        assert!(
            log.contains("project:northwind bank") && log.contains("recording of"),
            "{log}"
        );
        drop(s);
        assert_eq!(
            e.retention().unwrap(),
            Vec::<String>::new(),
            "nothing left to do"
        );
    }
}
