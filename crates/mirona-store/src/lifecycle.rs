//! Keeping and forgetting (NGR-165): turning encryption on and off, recordings in and out of
//! their sealed form, retention's building blocks, and deleting a meeting everywhere.

use crate::crypt::{self, Key};
use crate::{Result, Store, files, index};
use mirona_core::{AnswerSource, Id, MeetingPart, TagKind};
use std::path::{Path, PathBuf};

impl Store {
    pub fn encrypted(&self) -> bool {
        self.key.is_some()
    }

    /// Turn encryption on (`Some`) or off (`None`): every file that is kept sealed is rewritten,
    /// and the index moves into memory (or back to `index.sqlite`). Safe to rerun after an
    /// interruption: readers take both forms. Never call it while recording.
    pub fn set_encryption(&mut self, key: Option<Key>) -> Result<()> {
        let from = self.key.clone();
        let (from, to) = (from.as_ref(), key.as_ref());
        for e in &self.entries {
            crypt::reseal_lines(from, to, &e.dir.join(files::TRANSCRIPT))?;
            crypt::reseal_file(from, to, &e.dir.join(crate::search::EMBEDDINGS))?;
            if let Some(a) = &e.audio {
                crypt::reseal_file(from, to, &e.dir.join(a))?;
            }
        }
        crypt::reseal_file(from, to, &self.voices_path())?;
        crypt::reseal_lines(from, to, &self.labels_path())?;
        let file = self.root.join(".mirona").join("index.sqlite");
        self.db = if key.is_some() {
            std::fs::remove_file(&file).ok();
            index::open(Path::new(":memory:"))?
        } else {
            index::open(&file)?
        };
        self.key = key;
        self.reindex()?;
        self.touched();
        Ok(())
    }

    /// Seal a meeting's recording (after compression or import). No-op without a key.
    pub fn seal_audio(&self, id: &str) -> Result<()> {
        match (self.key.as_ref(), self.audio_path(id)?) {
            (Some(k), Some(p)) => crypt::reseal_file(None, Some(k), &p),
            _ => Ok(()),
        }
    }

    fn tmp_dir(&self) -> PathBuf {
        self.root.join(".mirona").join("tmp")
    }

    /// Clear decrypted copies left from playback or diarization.
    pub(crate) fn clear_tmp(&self) {
        std::fs::remove_dir_all(self.tmp_dir()).ok();
    }

    /// A path to the recording that audio code can read: the file itself, or a decrypted copy
    /// in `.mirona/tmp` when it is sealed. ponytail: a plaintext copy exists while it's used
    /// (until the next `open` or the meeting's next call); decode from memory to avoid it.
    pub fn plain_audio(&self, id: &str) -> Result<Option<PathBuf>> {
        let Some(path) = self.audio_path(id)? else {
            return Ok(None);
        };
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !crypt::is_sealed(&bytes) {
            return Ok(Some(path));
        }
        let plain = crypt::open(self.key.as_ref(), bytes)?;
        let dir = self.tmp_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("audio");
        let out = dir.join(format!("{id}.{ext}"));
        std::fs::write(&out, plain).map_err(|e| format!("{}: {e}", out.display()))?;
        Ok(Some(out))
    }

    /// Delete a meeting's recording and keep its text (retention).
    pub fn remove_audio(&mut self, id: &str) -> Result<bool> {
        let Some(path) = self.audio_path(id)? else {
            return Ok(false);
        };
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        std::fs::remove_file(self.tmp_dir().join(path.file_name().unwrap_or_default())).ok();
        let i = self.idx(id)?;
        self.entries[i].audio = None;
        self.save(i, false)?;
        Ok(true)
    }

    /// Remove generated parts of a meeting, keeping its transcript and the user's notes.
    pub fn clear_parts(&mut self, id: &str, parts: &[MeetingPart]) -> Result<()> {
        if parts.contains(&MeetingPart::Audio) {
            self.remove_audio(id)?;
        }
        let i = self.idx(id)?;
        let e = &mut self.entries[i];
        let mut transcript = false;
        for p in parts {
            match p {
                MeetingPart::Audio => {}
                MeetingPart::Summary => {
                    e.meeting.summary = None;
                    e.items.clear();
                }
                MeetingPart::Enhanced => {
                    e.meeting.notes.enhanced.clear();
                    e.meeting.notes.enhanced_by.clear();
                }
                MeetingPart::Classification => {
                    e.meeting.answers = None;
                    for s in &mut e.meeting.segments {
                        // Sensitive tags stay: dropping them would show hidden lines.
                        s.tags.retain(|t| t.kind == TagKind::Sensitive);
                        s.answers.retain(|a| a.source == AnswerSource::User);
                        s.owners.clear();
                        s.due = None;
                    }
                    transcript = true;
                }
            }
        }
        self.save(i, transcript)
    }

    /// Whole days since the meeting started.
    pub fn age_days(&self, id: &str) -> Result<i64> {
        let e = &self.entries[self.idx(id)?];
        let t = crate::timestamp(&e.created).ok_or("meeting has no start time")?;
        Ok(jiff::Timestamp::now().duration_since(t).as_secs() / 86_400)
    }

    pub fn has_audio(&self, id: &str) -> bool {
        self.audio_path(id)
            .is_ok_and(|a| a.is_some_and(|p| p.is_file()))
    }

    /// Delete every meeting with a tag (exact tag, any case). Returns the deleted ids.
    pub fn delete_tagged(&mut self, tag: &str) -> Result<Vec<Id>> {
        let ids: Vec<Id> = self
            .list_meetings()
            .into_iter()
            .filter(|m| crate::has_tag(&m.tags, tag))
            .map(|m| m.id)
            .collect();
        for id in &ids {
            self.delete_meeting(id)?;
        }
        Ok(ids)
    }

    /// Drop a deleted meeting's corrections: they quote its transcript.
    pub(crate) fn forget_labels(&self, meeting_id: &str) -> Result<()> {
        let path = self.labels_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(());
        };
        let mut out = String::new();
        let mut dropped = false;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let plain = crypt::open_line(self.key.as_ref(), line)?;
            let row: serde_json::Value = serde_json::from_str(&plain).unwrap_or_default();
            if row["meeting_id"] == meeting_id {
                dropped = true;
            } else {
                out += line;
                out += "\n";
            }
        }
        if dropped {
            files::write_atomic(&path, &out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirona_core::Segment;

    fn line(id: &str, n: u32, text: &str) -> Segment {
        Segment {
            id: format!("{id}-{n}"),
            speaker_id: "them".into(),
            start_ms: n * 1000,
            end_ms: 0,
            text: text.into(),
            redacted: false,
            tags: vec![],
            answers: vec![],
            owners: vec![],
            due: None,
        }
    }

    fn grep(dir: &Path, needle: &str) -> Vec<PathBuf> {
        let mut hits = vec![];
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if std::fs::read(&p)
                    .unwrap()
                    .windows(needle.len())
                    .any(|w| w == needle.as_bytes())
                {
                    hits.push(p);
                }
            }
        }
        hits
    }

    #[test]
    fn encryption_leaves_no_transcript_text_on_disk_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut s = Store::open(root).unwrap();
        s.start_meeting("m-1").unwrap();
        s.push_segment("m-1", line("m-1", 0, "Zanzibar rollout slips a week."))
            .unwrap();
        std::fs::write(
            s.meeting_dir("m-1").unwrap().join("audio.caf"),
            b"Zanzibar-audio",
        )
        .unwrap();
        s.set_audio("m-1", "audio.caf").unwrap();
        s.set_embeddings("m-1", "toy", vec![("m-1-0".into(), vec![1.0, 2.0])])
            .unwrap();
        std::fs::write(
            s.labels_path(),
            "{\"meeting_id\":\"m-1\",\"text\":\"Zanzibar\"}\n",
        )
        .unwrap();

        let key = Key::generate().unwrap();
        s.set_encryption(Some(key.clone())).unwrap();
        // Live appends are sealed too.
        s.push_segment("m-1", line("m-1", 1, "Zanzibar budget is approved."))
            .unwrap();
        let leaks: Vec<PathBuf> = grep(root, "Zanzibar")
            .into_iter()
            .filter(|p| !p.ends_with("meeting.md"))
            .collect();
        assert_eq!(leaks, Vec::<PathBuf>::new(), "plaintext left on disk");
        assert!(!root.join(".mirona/index.sqlite").exists());
        assert_eq!(s.search_hits("Zanzibar").len(), 2, "search still works");

        // Reopening needs the key; without it the lines can't be read.
        let s2 = Store::open_with(root, Some(key.clone())).unwrap();
        assert_eq!(s2.meeting("m-1").unwrap().segments.len(), 2);
        let audio = s2.plain_audio("m-1").unwrap().unwrap();
        assert_eq!(std::fs::read(&audio).unwrap(), b"Zanzibar-audio");
        assert_eq!(
            s2.embedding_todo("m-1", "toy").unwrap().len(),
            1,
            "line 0 kept its vector"
        );
        assert!(s2.labels_text().unwrap().contains("Zanzibar"));
        let locked = Store::open_snapshot(root).unwrap();
        assert!(locked.meeting("m-1").unwrap().segments.is_empty());
        assert_ne!(locked.load_errors(), &[] as &[String]);

        // And off again: plain files, index back on disk.
        let mut s3 = Store::open_with(root, Some(key)).unwrap();
        s3.set_encryption(None).unwrap();
        assert!(root.join(".mirona/index.sqlite").is_file());
        let plain = Store::open(root).unwrap();
        assert_eq!(plain.meeting("m-1").unwrap().segments.len(), 2);
        assert_eq!(
            std::fs::read(plain.audio_path("m-1").unwrap().unwrap()).unwrap(),
            b"Zanzibar-audio"
        );
    }

    #[test]
    fn audio_goes_text_stays_and_tags_can_be_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        for (id, client) in [("m-1", "Northwind"), ("m-2", "Northwind"), ("m-3", "Helio")] {
            s.start_meeting(id).unwrap();
            s.push_segment(id, line(id, 0, "hello")).unwrap();
            let i = s.idx(id).unwrap();
            s.entries[i].meeting.tags = vec![format!("project:{client}")];
            std::fs::write(s.meeting_dir(id).unwrap().join("audio.caf"), b"a").unwrap();
            s.set_audio(id, "audio.caf").unwrap();
        }
        assert_eq!(s.age_days("m-1").unwrap(), 0);
        assert!(s.remove_audio("m-3").unwrap());
        assert!(!s.has_audio("m-3"));
        assert_eq!(s.meeting("m-3").unwrap().segments.len(), 1, "text stays");
        assert!(!s.remove_audio("m-3").unwrap(), "nothing left to remove");

        std::fs::write(
            s.labels_path(),
            "{\"meeting_id\":\"m-1\",\"text\":\"hello\"}\n{\"meeting_id\":\"m-3\",\"text\":\"x\"}\n",
        )
        .unwrap();
        let gone = s.delete_tagged("Project:northwind").unwrap();
        assert_eq!(gone.len(), 2);
        assert_eq!(s.list_meetings().len(), 1);
        let labels = std::fs::read_to_string(s.labels_path()).unwrap();
        assert!(
            !labels.contains("m-1") && labels.contains("m-3"),
            "{labels}"
        );
    }
}
