//! Meeting store (NGR-140): one folder per meeting, plus a `SQLite` index.
//!
//! ```text
//! <root>/<YYYY-MM-DD-HHMM>-<slug>/meeting.md        YAML frontmatter + Markdown (Obsidian-friendly)
//! <root>/<YYYY-MM-DD-HHMM>-<slug>/transcript.jsonl  one Segment per line, appended while recording
//! <root>/<YYYY-MM-DD-HHMM>-<slug>/audio.<ext>       written by capture; the store keeps its name
//! <root>/.mirona/index.sqlite                       derived; `reindex` rebuilds it from the files
//! <root>/<YYYY-MM-DD-HHMM>-<slug>/embeddings.json   one vector per line, for search (`search.rs`)
//! <root>/.mirona/voices.json                        known people + voice embeddings (`people.rs`)
//! ```
//!
//! The files are the source of truth and every write goes through to disk immediately.
//! ponytail: all meetings live in memory and the index is rebuilt on every `open`; fine for
//! hundreds of meetings, switch to lazy loading + mtime-based incremental reindex past that.

mod classify;
pub mod crypt;
mod files;
mod index;
mod lifecycle;
mod people;
pub mod sample;
mod search;

pub use classify::review_tag;
pub use index::Hit;
pub use people::{Cluster, Person, Voices, replace_name};
pub use search::{MIN_SIMILARITY, text_hash};

use files::Entry;
use mirona_core::{
    ActionItem, AnswerSource, Choice, Correction, EnhancedNote, Id, Meeting, MeetingAnswers,
    MeetingSummary, Note, Segment, Speaker, Summary, Tint,
};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, String>;

pub struct Store {
    root: PathBuf,
    db: Connection,
    entries: Vec<Entry>,
    people: Vec<Speaker>,
    errors: Vec<String>,
    /// Last `.mirona/generation` this store wrote or loaded. Every write bumps the file, so a
    /// store can tell when another process (CLI, an import) changed the folder: `refresh`.
    seen: std::cell::RefCell<String>,
    /// Encryption at rest (NGR-165): set, every sealed file is written sealed and the index
    /// stays in memory.
    key: Option<crypt::Key>,
}

impl Store {
    /// Opens (or creates) a store at `root` and indexes what is there. Meetings that fail to
    /// load are skipped and listed in [`Store::load_errors`].
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(root, None)
    }

    /// `open` for an encrypted store (NGR-165): the index lives in memory, and any
    /// `index.sqlite` left from before encryption is deleted.
    pub fn open_with(root: impl AsRef<Path>, key: Option<crypt::Key>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let meta = root.join(".mirona");
        std::fs::create_dir_all(&meta).map_err(|e| format!("{}: {e}", meta.display()))?;
        let file = meta.join("index.sqlite");
        let db = if key.is_some() {
            std::fs::remove_file(&file).ok();
            index::open(Path::new(":memory:"))?
        } else {
            index::open(&file)?
        };
        let s = Self::with_index(root, db, key)?;
        s.clear_tmp();
        Ok(s)
    }

    /// Like `open`, but indexes into a private in-memory database and leaves
    /// `.mirona/index.sqlite` alone: for readers (MCP server, CLI) running next to the app, so
    /// they never race its index. Writes still go to the meeting files.
    pub fn open_snapshot(root: impl AsRef<Path>) -> Result<Self> {
        Self::open_snapshot_with(root, None)
    }

    pub fn open_snapshot_with(root: impl AsRef<Path>, key: Option<crypt::Key>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        Self::with_index(root, index::open(Path::new(":memory:"))?, key)
    }

    fn with_index(root: PathBuf, db: Connection, key: Option<crypt::Key>) -> Result<Self> {
        let mut s = Self {
            root,
            db,
            entries: vec![],
            people: vec![],
            errors: vec![],
            seen: std::cell::RefCell::new(String::new()),
            key,
        };
        s.reindex()?;
        Ok(s)
    }

    /// A throwaway store in the temp dir, seeded with the sample meetings.
    /// ponytail: for the CLI/MCP/tests until NGR-148/149 point them at the real root; leaves a
    /// small dir in the OS temp dir per call.
    pub fn sample() -> Self {
        // The counter keeps parallel tests apart when the clock gives two of them one value.
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("mirona-sample-{}-{nanos}-{n}", std::process::id()));
        let mut s = Self::open(dir).expect("sample store");
        sample::seed(&mut s).expect("seed sample store");
        s
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn generation_path(&self) -> PathBuf {
        self.root.join(".mirona").join("generation")
    }

    fn read_generation(&self) -> String {
        std::fs::read_to_string(self.generation_path()).unwrap_or_default()
    }

    /// Mark the folder as changed by this store (called after every write).
    pub(crate) fn touched(&self) {
        let token = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        // Best effort: a failed bump only means another process notices a bit later.
        let _ = std::fs::write(self.generation_path(), &token);
        *self.seen.borrow_mut() = token;
    }

    /// Reload if another process wrote to the folder since this store last looked. Cheap when
    /// nothing changed (one small file read). Returns whether anything was reloaded.
    pub fn refresh(&mut self) -> Result<bool> {
        if self.read_generation() == *self.seen.borrow() {
            return Ok(false);
        }
        self.reindex()?;
        Ok(true)
    }

    /// Problems found while loading: unreadable meetings (skipped) and bad transcript lines.
    pub fn load_errors(&self) -> &[String] {
        &self.errors
    }

    /// Re-reads every meeting folder (picking up edits made outside the app) and rebuilds the
    /// index from scratch.
    pub fn reindex(&mut self) -> Result<()> {
        *self.seen.borrow_mut() = self.read_generation();
        let mut entries: Vec<Entry> = vec![];
        let mut errors = vec![];
        let dirs = std::fs::read_dir(&self.root).map_err(|e| e.to_string())?;
        let mut dirs: Vec<PathBuf> = dirs
            .filter_map(|d| d.ok().map(|d| d.path()))
            .filter(|p| p.join(files::MEETING).is_file())
            .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        dirs.sort();
        for dir in dirs {
            match files::read(&dir, self.key.as_ref()) {
                Ok((e, errs)) if entries.iter().any(|x| x.meeting.id == e.meeting.id) => {
                    errors.extend(errs);
                    errors.push(format!(
                        "{}: duplicate meeting id {}, skipped",
                        dir.display(),
                        e.meeting.id
                    ));
                }
                Ok((e, errs)) => {
                    errors.extend(errs);
                    entries.push(e);
                }
                Err(err) => errors.push(format!("{}: {err}", dir.join(files::MEETING).display())),
            }
        }
        entries.sort_by_key(|e| std::cmp::Reverse(timestamp(&e.created)));
        index::rebuild(&mut self.db, &entries)?;
        self.entries = entries;
        self.errors = errors;
        self.refresh_people();
        Ok(())
    }

    // --- Reads ---

    pub fn list_meetings(&self) -> Vec<MeetingSummary> {
        self.entries
            .iter()
            .map(|e| {
                let (when, _) = when_labels(&e.created);
                let m = &e.meeting;
                MeetingSummary {
                    id: m.id.clone(),
                    title: m.title.clone(),
                    tags: m.tags.clone(),
                    when,
                    created: e.created.clone(),
                    duration_min: m.duration_min,
                    meeting_type: m
                        .answers
                        .as_ref()
                        .map(|a| a.meeting_type.value.clone())
                        .unwrap_or_default(),
                    open_actions: count(e.items.iter().filter(|a| !a.done)),
                    needs_review: self.review(&m.id).map_or(0, |r| count(r.iter())),
                    has_audio: e.audio.is_some(),
                    has_summary: m.summary.is_some(),
                    sample: m.sample,
                }
            })
            .collect()
    }

    pub fn meeting(&self, id: &str) -> Result<Meeting> {
        Ok(labeled(&self.entries[self.idx(id)?]))
    }

    /// Every action item, newest meeting first.
    pub fn action_items(&self) -> Vec<ActionItem> {
        self.entries.iter().flat_map(|e| e.items.clone()).collect()
    }

    /// Every speaker seen in any meeting (newest copy wins).
    pub fn people(&self) -> &[Speaker] {
        &self.people
    }

    /// Human fixes to model answers, read back from the transcripts.
    pub fn corrections(&self) -> Vec<Correction> {
        let segs = self.entries.iter().flat_map(|e| &e.meeting.segments);
        segs.flat_map(|s| {
            s.answers
                .iter()
                .filter(|a| a.source == AnswerSource::User)
                .map(|a| Correction {
                    segment_id: s.id.clone(),
                    answer_id: a.id.clone(),
                    value: a.answer,
                })
        })
        .collect()
    }

    pub fn meeting_dir(&self, id: &str) -> Result<PathBuf> {
        Ok(self.entries[self.idx(id)?].dir.clone())
    }

    pub fn audio_path(&self, id: &str) -> Result<Option<PathBuf>> {
        let e = &self.entries[self.idx(id)?];
        Ok(e.audio.as_ref().map(|a| e.dir.join(a)))
    }

    /// Full-text search over titles, tags, notes and transcripts (FTS5, prefix match on
    /// every word). Meetings come back newest first; an empty query returns all of them.
    pub fn search(&self, query: &str) -> Vec<MeetingSummary> {
        if query.trim().is_empty() {
            return self.list_meetings();
        }
        let ids = index::meeting_ids(&self.db, query).unwrap_or_default();
        self.list_meetings()
            .into_iter()
            .filter(|m| ids.contains(&m.id))
            .collect()
    }

    /// The transcript moments matching `query`, best first.
    pub fn search_hits(&self, query: &str) -> Vec<Hit> {
        if query.trim().is_empty() {
            return vec![];
        }
        index::hits(&self.db, query).unwrap_or_default()
    }

    // --- Writes ---

    /// Adds a whole meeting (import, demo seed). `created` is RFC 3339.
    pub fn add_meeting(
        &mut self,
        meeting: Meeting,
        created: &str,
        items: Vec<ActionItem>,
    ) -> Result<Meeting> {
        if self.idx(&meeting.id).is_ok() {
            return Err(format!("meeting {} already exists", meeting.id));
        }
        timestamp(created).ok_or(format!("bad created time {created}"))?;
        let dir = self.free_dir(created, &meeting.title, None);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut e = Entry {
            meeting,
            created: created.into(),
            audio: None,
            items,
            dir,
            vectors: search::Vectors::default(),
        };
        files::normalize(&mut e);
        files::write_transcript(&e, self.key.as_ref())?;
        files::write_meeting(&e)?;
        index::reindex_meeting(&mut self.db, &e)?;
        self.touched();
        let t = timestamp(created);
        let at = self
            .entries
            .iter()
            .position(|x| timestamp(&x.created) < t)
            .unwrap_or(self.entries.len());
        self.entries.insert(at, e);
        self.refresh_people();
        Ok(labeled(&self.entries[at]))
    }

    /// Creates the folder for a meeting that is starting now.
    pub fn start_meeting(&mut self, id: &str) -> Result<Meeting> {
        let m = Meeting {
            id: id.into(),
            title: "New meeting".into(),
            tags: vec![],
            when: String::new(),
            date_label: String::new(),
            duration_min: 0,
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
        };
        let now = jiff::Zoned::now()
            .strftime("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        self.add_meeting(m, &now, vec![])
    }

    /// Appends one line to `transcript.jsonl`. `meeting.md` is only rewritten the first time a
    /// speaker shows up in this meeting. Unknown speaker ids become a new person.
    pub fn push_segment(&mut self, meeting_id: &str, seg: Segment) -> Result<Speaker> {
        let i = self.idx(meeting_id)?;
        let speaker = self
            .people
            .iter()
            .find(|p| p.id == seg.speaker_id)
            .cloned()
            .unwrap_or_else(|| new_speaker(&seg.speaker_id));
        files::append_segment(&self.entries[i].dir, &seg, self.key.as_ref())?;
        index::insert_segment(&self.db, meeting_id, &seg)?;
        self.touched();
        self.entries[i].meeting.segments.push(seg);
        if !self.entries[i]
            .meeting
            .speakers
            .iter()
            .any(|s| s.id == speaker.id)
        {
            self.entries[i].meeting.speakers.push(speaker.clone());
            self.refresh_people();
            self.save(i, false)?;
        }
        Ok(speaker)
    }

    /// Replaces a segment (e.g. when its classification lands). Rewrites the transcript.
    pub fn update_segment(&mut self, meeting_id: &str, seg: Segment) -> Result<()> {
        let i = self.idx(meeting_id)?;
        let s = self.entries[i]
            .meeting
            .segments
            .iter_mut()
            .find(|s| s.id == seg.id)
            .ok_or("no segment")?;
        *s = seg;
        self.save(i, true)
    }

    /// Join a finished meeting's lines split mid-sentence (see [`merge_lines`]). Returns how
    /// many lines were folded into the one before.
    pub fn merge_lines(&mut self, meeting_id: &str) -> Result<usize> {
        let i = self.idx(meeting_id)?;
        let n = merge_lines(&mut self.entries[i].meeting.segments);
        if n > 0 {
            classify::fill_items(&mut self.entries[i]);
            self.save(i, true)?;
        }
        Ok(n)
    }

    /// Fix a word in the transcript (NGR-143).
    pub fn edit_segment_text(
        &mut self,
        meeting_id: &str,
        segment_id: &str,
        text: &str,
    ) -> Result<Segment> {
        let i = self.idx(meeting_id)?;
        let s = self.entries[i]
            .meeting
            .segments
            .iter_mut()
            .find(|s| s.id == segment_id)
            .ok_or("no segment")?;
        s.text = text.into();
        let s = s.clone();
        self.save(i, true)?;
        Ok(s)
    }

    /// Delete a meeting everywhere: its folder (transcript, audio, notes, embeddings), the
    /// index, its anonymous voices and the corrections quoting it (NGR-165).
    pub fn delete_meeting(&mut self, id: &str) -> Result<()> {
        let i = self.idx(id)?;
        let e = self.entries.remove(i);
        std::fs::remove_dir_all(&e.dir).map_err(|err| format!("{}: {err}", e.dir.display()))?;
        index::remove_meeting(&mut self.db, id)?;
        self.touched();
        self.refresh_people();
        self.forget_meeting_voices(id)?;
        self.forget_labels(id)
    }

    /// Records the meeting length once recording stops. The summary comes from `set_summary`.
    pub fn finish_meeting(&mut self, meeting_id: &str, elapsed_ms: u32) -> Result<Meeting> {
        let i = self.idx(meeting_id)?;
        self.entries[i].meeting.duration_min = (elapsed_ms / 60_000).max(1);
        self.save(i, false)?;
        Ok(labeled(&self.entries[i]))
    }

    /// Renames the meeting and, best effort, its folder (kept if the rename fails, e.g. a file
    /// is open on Windows).
    pub fn set_title(&mut self, meeting_id: &str, title: &str) -> Result<Meeting> {
        let i = self.idx(meeting_id)?;
        self.entries[i].meeting.title = title.into();
        let e = &self.entries[i];
        let to = self.free_dir(&e.created, title, Some(&e.dir));
        if to != e.dir && std::fs::rename(&e.dir, &to).is_ok() {
            self.entries[i].dir = to;
        }
        self.save(i, false)?;
        Ok(labeled(&self.entries[i]))
    }

    pub fn set_notes(&mut self, meeting_id: &str, raw: Vec<String>) -> Result<()> {
        self.set_notes_at(meeting_id, raw, vec![])
    }

    /// Notes plus the meeting time each line was written (NGR-144), index for index.
    pub fn set_notes_at(
        &mut self,
        meeting_id: &str,
        raw: Vec<String>,
        mut at_ms: Vec<Option<u32>>,
    ) -> Result<()> {
        let i = self.idx(meeting_id)?;
        // Same shape as a reload gives back: empty when no line has a time, else one per line.
        at_ms.resize(raw.len(), None);
        if at_ms.iter().all(Option::is_none) {
            at_ms.clear();
        }
        let n = &mut self.entries[i].meeting.notes;
        n.raw = raw;
        n.raw_at_ms = at_ms;
        self.save(i, false)
    }

    /// Stores a summary and its action items. Items that keep their id keep their done state.
    pub fn set_summary(
        &mut self,
        meeting_id: &str,
        summary: Summary,
        mut items: Vec<ActionItem>,
    ) -> Result<()> {
        let i = self.idx(meeting_id)?;
        let e = &mut self.entries[i];
        for a in &mut items {
            if let Some(old) = e.items.iter().find(|o| o.id == a.id) {
                a.done = old.done;
            }
            // An owner the user set, for the task said on the same line.
            let set = e
                .items
                .iter()
                .find(|o| o.owner_confirmed && o.at_ms == a.at_ms);
            if let Some(old) = set.filter(|_| a.at_ms.is_some()) {
                a.owner.clone_from(&old.owner);
                a.owner_confirmed = true;
            }
        }
        e.meeting.summary = Some(summary);
        e.items = items;
        classify::fill_items(e);
        self.save(i, false)
    }

    pub fn set_enhanced_notes(
        &mut self,
        meeting_id: &str,
        notes: Vec<EnhancedNote>,
        by: &str,
    ) -> Result<()> {
        let i = self.idx(meeting_id)?;
        let n = &mut self.entries[i].meeting.notes;
        n.enhanced = notes;
        n.enhanced_by = by.into();
        self.save(i, false)
    }

    /// Meeting-level answers (type, tags, priority, follow-up). A tag the classifier says yes
    /// to at/above its threshold is added; tags are never removed here (NGR-158). Decisions
    /// the user already made stay.
    pub fn set_answers(&mut self, meeting_id: &str, mut answers: MeetingAnswers) -> Result<()> {
        let i = self.idx(meeting_id)?;
        let m = &mut self.entries[i].meeting;
        if let Some(old) = &m.answers {
            for t in old.tags.iter().filter(|t| t.confirmed) {
                answers.tags.retain(|n| !n.tag.eq_ignore_ascii_case(&t.tag));
                answers.tags.push(t.clone());
            }
        }
        for t in answers.tags.iter().filter(|t| !t.confirmed && t.assigned()) {
            if !has_tag(&m.tags, &t.tag) {
                m.tags.push(t.tag.clone());
            }
        }
        m.answers = Some(answers);
        self.save(i, false)
    }

    /// Records the recording's file name (inside `meeting_dir`).
    pub fn set_audio(&mut self, meeting_id: &str, file_name: &str) -> Result<()> {
        let i = self.idx(meeting_id)?;
        self.entries[i].audio = Some(file_name.into());
        self.save(i, false)
    }

    pub fn toggle_action_item(&mut self, id: &str) -> Result<ActionItem> {
        let (i, j) = self
            .entries
            .iter()
            .enumerate()
            .find_map(|(i, e)| e.items.iter().position(|a| a.id == id).map(|j| (i, j)))
            .ok_or("no action item")?;
        let a = &mut self.entries[i].items[j];
        a.done = !a.done;
        let a = a.clone();
        self.save(i, false)?;
        Ok(a)
    }

    /// Keep (confirm) or drop an action item waiting for review (NGR-158).
    pub fn resolve_action_item(&mut self, id: &str, keep: bool) -> Result<()> {
        let i = self
            .entries
            .iter()
            .position(|e| e.items.iter().any(|a| a.id == id))
            .ok_or("no action item")?;
        let items = &mut self.entries[i].items;
        if keep {
            items
                .iter_mut()
                .filter(|a| a.id == id)
                .for_each(|a| a.needs_review = false);
        } else {
            items.retain(|a| a.id != id);
        }
        self.save(i, false)
    }

    /// The user sets who owns an action item; later summaries and speaker changes keep it.
    pub fn set_action_owner(&mut self, id: &str, owner: &str) -> Result<ActionItem> {
        let (i, j) = self
            .entries
            .iter()
            .enumerate()
            .find_map(|(i, e)| e.items.iter().position(|a| a.id == id).map(|j| (i, j)))
            .ok_or("no action item")?;
        let a = &mut self.entries[i].items[j];
        a.owner = owner.trim().into();
        a.owner_confirmed = true;
        let a = a.clone();
        self.save(i, false)?;
        Ok(a)
    }

    /// Fix a model answer. The fix is kept as a labelled example (`labels_path`, NGR-160) and
    /// retags the segment.
    #[allow(clippy::needless_pass_by_value)] // takes the correction as the UI sends it
    pub fn correct(&mut self, c: Correction) -> Result<Segment> {
        let i = self
            .entries
            .iter()
            .position(|e| e.meeting.segments.iter().any(|s| s.id == c.segment_id))
            .ok_or("no segment")?;
        let e = &mut self.entries[i];
        let seg = e
            .meeting
            .segments
            .iter_mut()
            .find(|s| s.id == c.segment_id)
            .unwrap();
        let ans = seg
            .answers
            .iter_mut()
            .find(|a| a.id == c.answer_id)
            .ok_or("no answer")?;
        let model_probability = ans.probability;
        ans.answer = c.value;
        ans.probability = if c.value { 1.0 } else { 0.0 };
        ans.source = AnswerSource::User;
        Self::retag(seg, &c.answer_id, c.value);
        let seg = seg.clone();
        let meeting_id = e.meeting.id.clone();
        // The action items follow (NGR-159): a confirmed action is filed, a rejected one goes.
        if c.answer_id == "action" {
            let here = |a: &ActionItem| a.at_ms == Some(seg.start_ms);
            if !c.value {
                e.items.retain(|a| !here(a));
            } else if let Some(a) = e.items.iter_mut().find(|a| here(a)) {
                a.needs_review = false;
            } else {
                let item = classify::item_from(&e.meeting, &seg);
                e.items.push(item);
            }
        }
        self.save(i, true)?;
        self.append_label(&meeting_id, &seg, &c.answer_id, c.value, model_probability)?;
        Ok(seg)
    }

    /// The user set the meeting's tags: every tag answer counts as decided, so a rerun of the
    /// classifier never adds back a tag the user removed.
    pub fn set_tags(&mut self, meeting_id: &str, tags: &[String]) -> Result<Meeting> {
        let i = self.idx(meeting_id)?;
        let m = &mut self.entries[i].meeting;
        m.tags.clear();
        for t in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            if !has_tag(&m.tags, t) {
                m.tags.push(t.into());
            }
        }
        if let Some(a) = m.answers.as_mut() {
            for t in &mut a.tags {
                t.confirmed = true;
            }
        }
        files::normalize(&mut self.entries[i]);
        self.save(i, false)?;
        let value = self.entries[i].meeting.tags.join(", ");
        self.label_meeting(meeting_id, "tags", &value)?;
        Ok(labeled(&self.entries[i]))
    }

    /// Rename a tag in every meeting that has it (and in its answers). Returns how many changed.
    pub fn rename_tag(&mut self, old: &str, new: &str) -> Result<usize> {
        let mut n = 0;
        for i in 0..self.entries.len() {
            let m = &mut self.entries[i].meeting;
            if !has_tag(&m.tags, old) {
                continue;
            }
            m.tags.retain(|t| !t.eq_ignore_ascii_case(old.trim()));
            if !has_tag(&m.tags, new) {
                m.tags.push(new.trim().into());
            }
            for t in m.answers.iter_mut().flat_map(|a| &mut a.tags) {
                if t.tag.eq_ignore_ascii_case(old.trim()) {
                    t.tag = new.trim().into();
                }
            }
            files::normalize(&mut self.entries[i]);
            self.save(i, false)?;
            n += 1;
        }
        Ok(n)
    }

    /// Keep (`yes`) or drop one tag the classifier was unsure of, or a proposed new tag.
    pub fn answer_tag(&mut self, meeting_id: &str, tag: &str, yes: bool) -> Result<Meeting> {
        let i = self.idx(meeting_id)?;
        let m = &mut self.entries[i].meeting;
        if let Some(t) = m
            .answers
            .as_mut()
            .and_then(|a| a.tags.iter_mut().find(|t| t.tag.eq_ignore_ascii_case(tag)))
        {
            t.confirmed = true;
        }
        if yes && !has_tag(&m.tags, tag) {
            m.tags.push(tag.trim().into());
        } else if !yes {
            m.tags.retain(|t| !t.eq_ignore_ascii_case(tag.trim()));
        }
        files::normalize(&mut self.entries[i]);
        self.save(i, false)?;
        let value = self.entries[i].meeting.tags.join(", ");
        self.label_meeting(meeting_id, "tags", &value)?;
        Ok(labeled(&self.entries[i]))
    }

    // --- Internals ---

    /// People are whoever appears in a meeting, newest meeting first.
    /// Known people and their voiceprints live in `people.rs` (`.mirona/voices.json`).
    fn refresh_people(&mut self) {
        self.people.clear();
        for sp in self.entries.iter().flat_map(|e| &e.meeting.speakers) {
            if !self.people.iter().any(|p| p.id == sp.id) {
                self.people.push(sp.clone());
            }
        }
    }

    fn idx(&self, id: &str) -> Result<usize> {
        self.entries
            .iter()
            .position(|e| e.meeting.id == id)
            .ok_or(format!("no meeting {id}"))
    }

    /// Writes `meeting.md` (and the whole transcript if asked) and reindexes the meeting.
    fn save(&mut self, i: usize, transcript: bool) -> Result<()> {
        let e = &mut self.entries[i];
        files::normalize(e);
        if transcript {
            files::write_transcript(e, self.key.as_ref())?;
        }
        files::write_meeting(e)?;
        index::reindex_meeting(&mut self.db, e)?;
        self.touched();
        Ok(())
    }

    /// `<root>/<YYYY-MM-DD-HHMM>-<slug>`, suffixed `-2`, `-3`… if taken (`current` is free).
    fn free_dir(&self, created: &str, title: &str, current: Option<&Path>) -> PathBuf {
        let stamp = timestamp(created).map_or_else(
            || "undated".into(),
            |t| {
                t.to_zoned(jiff::tz::TimeZone::system())
                    .strftime("%Y-%m-%d-%H%M")
                    .to_string()
            },
        );
        let base = format!("{stamp}-{}", files::slug(title));
        (1..10_000)
            .map(|n| {
                let name = if n == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                };
                self.root.join(name)
            })
            .find(|p| Some(p.as_path()) == current || !p.exists())
            .expect("fewer than 10k meetings per minute and title")
    }
}

fn timestamp(created: &str) -> Option<jiff::Timestamp> {
    created.parse().ok()
}

/// ("today" | "yesterday" | "Tue" | "last week" | "5 Nov", "Thu 5 Nov 2026 · 09:30").
/// Tags compare without case: `Project:Setr` is `project:setr`.
pub fn has_tag(tags: &[String], tag: &str) -> bool {
    tags.iter().any(|t| t.eq_ignore_ascii_case(tag.trim()))
}

fn count<T>(it: impl Iterator<Item = T>) -> u32 {
    u32::try_from(it.count()).unwrap_or(u32::MAX)
}

fn when_labels(created: &str) -> (String, String) {
    let Some(t) = timestamp(created) else {
        return (created.into(), created.into());
    };
    let tz = jiff::tz::TimeZone::system();
    let z = t.to_zoned(tz.clone());
    let today = jiff::Zoned::now().with_time_zone(tz).date();
    let days = z.date().until(today).map_or(0, |s| s.get_days());
    let when = match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=6 => z.strftime("%a").to_string(),
        7..=13 => "last week".into(),
        _ => z.strftime("%-d %b").to_string(),
    };
    (when, z.strftime("%a %-d %b %Y · %H:%M").to_string())
}

fn labeled(e: &Entry) -> Meeting {
    let mut m = e.meeting.clone();
    (m.when, m.date_label) = when_labels(&e.created);
    m
}

fn new_speaker(id: &str) -> Speaker {
    let mut c = id.chars();
    let name: String = c
        .next()
        .map_or_else(|| "Unknown".into(), |f| f.to_uppercase().chain(c).collect());
    Speaker {
        id: id.into(),
        initial: name.chars().next().unwrap_or('?').to_string(),
        name,
        tint: Tint::Indigo,
        subtitle: String::new(),
        voiceprint: None,
        suggestion: None,
    }
}

/// The longest pause a sentence is joined across, and the longest line joining may make.
const JOIN_GAP_MS: u32 = 1_200;
const JOIN_MAX_MS: u32 = 30_000;

/// Speech is cut at 0.25 s pauses for the recognizer (ADR 0001), so one sentence can land on
/// several lines. A line joins the one before it when it's the same speaker, the pause is
/// short and the line before doesn't end a sentence. Redacted lines, lines the user corrected
/// and lines without an end time are left alone. Joined lines lose their answers so the
/// after-call classification asks again; their tags stay until then. Returns how many lines
/// were folded into the one before.
// ponytail: punctuation + pause heuristic; Parakeet sometimes ends a mid-sentence chunk with
// a period, which keeps that split. Lean on the pause alone if real meetings show it a lot.
pub fn merge_lines(segs: &mut Vec<Segment>) -> usize {
    let before = segs.len();
    let mut out: Vec<Segment> = Vec::with_capacity(before);
    for s in segs.drain(..) {
        match out.last_mut() {
            Some(prev) if joins(prev, &s) => {
                prev.text = format!("{} {}", prev.text.trim_end(), s.text.trim_start());
                prev.end_ms = s.end_ms;
                for t in s.tags {
                    if !prev
                        .tags
                        .iter()
                        .any(|p| p.kind == t.kind && p.label == t.label)
                    {
                        prev.tags.push(t);
                    }
                }
                prev.answers.clear();
                if prev.owners.is_empty() {
                    prev.owners = s.owners;
                }
                prev.due = prev.due.take().or(s.due);
            }
            _ => out.push(s),
        }
    }
    *segs = out;
    before - segs.len()
}

fn joins(prev: &Segment, next: &Segment) -> bool {
    let fixed = |s: &Segment| {
        s.redacted || s.end_ms == 0 || s.answers.iter().any(|a| a.source == AnswerSource::User)
    };
    prev.speaker_id == next.speaker_id
        && !fixed(prev)
        && !fixed(next)
        && next.start_ms.saturating_sub(prev.end_ms) <= JOIN_GAP_MS
        && next.end_ms.saturating_sub(prev.start_ms) <= JOIN_MAX_MS
        && !prev.text.trim_end().ends_with(['.', '?', '!'])
}

/// Owner distribution for a live segment: the speaker owns their own "I'll…".
pub fn live_owner(speaker: &str) -> Vec<Choice> {
    vec![
        Choice {
            label: speaker.into(),
            probability: 0.9,
        },
        Choice {
            label: "Others".into(),
            probability: 0.1,
        },
    ]
}

pub fn new_id() -> Id {
    format!(
        "m-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    )
}

#[cfg(test)]
mod tests;
