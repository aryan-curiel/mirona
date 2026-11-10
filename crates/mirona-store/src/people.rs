//! People and voiceprints (NGR-153/154): a local-only registry of known people next to the
//! meetings, plus the people operations (name, merge, forget a voice).
//!
//! ```text
//! <root>/.mirona/voices.json   { "people": [{id, name}],
//!                                "clusters": [{meeting_id, speaker_id, speech_ms, embedding}] }
//! ```
//!
//! Embeddings never go into `meeting.md`. A diarized meeting leaves one cluster per voice it
//! heard (its averaged embedding); a person's voiceprint is the speech-weighted mean of the
//! clusters labelled with their id, computed when needed. So fixing a wrong match is just
//! relabelling a cluster, and the voiceprint follows. Segments point at their embedding through
//! `(meeting id, speaker id)`.

use crate::{Result, Store};
use mirona_core::{Id, NameSuggestion, Speaker, Voiceprint};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const VOICES: &str = "voices.json";
/// Source-labelled speakers shared by every undiarized meeting: naming one names it in one
/// meeting, never everywhere.
const GENERIC: [&str; 2] = ["them", "speaker"];

/// A known (named) person. Their id is the speaker id they carry in every meeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: Id,
    pub name: String,
}

/// One voice heard in one meeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cluster {
    pub meeting_id: Id,
    pub speaker_id: Id,
    pub speech_ms: u32,
    /// Averaged, L2-normalised speaker embedding (speakrs: 256 floats).
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Voices {
    #[serde(default)]
    pub people: Vec<Person>,
    #[serde(default)]
    pub clusters: Vec<Cluster>,
}

impl Voices {
    /// Speech-weighted mean of a person's clusters; `None` if they have none.
    pub fn voiceprint(&self, person: &str) -> Option<Vec<f32>> {
        let mut sum: Vec<f32> = vec![];
        for c in self.clusters.iter().filter(|c| c.speaker_id == person) {
            sum.resize(c.embedding.len().max(sum.len()), 0.0);
            #[allow(clippy::cast_precision_loss)]
            let w = c.speech_ms as f32 / 1000.0;
            sum.iter_mut()
                .zip(&c.embedding)
                .for_each(|(s, e)| *s += w * e);
        }
        let n = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
        (n > 0.0).then(|| sum.iter().map(|x| x / n).collect())
    }

    /// Meetings a person's voice was heard in.
    pub fn meetings(&self, person: &str) -> u32 {
        let mut ids: Vec<&str> = self
            .clusters
            .iter()
            .filter(|c| c.speaker_id == person)
            .map(|c| c.meeting_id.as_str())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        u32::try_from(ids.len()).unwrap_or(u32::MAX)
    }

    fn person(&self, id: &str) -> Option<&Person> {
        self.people.iter().find(|p| p.id == id)
    }

    fn by_name(&self, name: &str) -> Option<&Person> {
        self.people
            .iter()
            .find(|p| p.name.to_lowercase() == name.to_lowercase())
    }

    fn relabel(&mut self, meeting_id: Option<&str>, from: &str, to: &str) {
        for c in &mut self.clusters {
            if c.speaker_id == from && meeting_id.is_none_or(|m| c.meeting_id == m) {
                c.speaker_id = to.into();
            }
        }
    }
}

impl Store {
    pub(crate) fn voices_path(&self) -> PathBuf {
        self.root.join(".mirona").join(VOICES)
    }

    /// The registry; empty when there is none yet (or it can't be read).
    pub fn voices(&self) -> Voices {
        crate::crypt::read_string(self.key.as_ref(), &self.voices_path())
            .ok()
            .flatten()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save_voices(&self, v: &Voices) -> Result<()> {
        let path = self.voices_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let json = serde_json::to_string(v).map_err(|e| e.to_string())?;
        crate::files::write_sealed(&path, self.key.as_ref(), json.as_bytes())?;
        self.touched();
        Ok(())
    }

    /// Known people with a voiceprint, for `speakrs::match_voiceprint`.
    pub fn voiceprints(&self) -> Vec<(Id, Vec<f32>)> {
        let v = self.voices();
        v.people
            .iter()
            .filter_map(|p| Some((p.id.clone(), v.voiceprint(&p.id)?)))
            .collect()
    }

    /// Diarization result for a meeting: one speaker id per segment (in transcript order), the
    /// meeting's speakers, and the voices heard (replacing any earlier ones for this meeting).
    pub fn set_diarization(
        &mut self,
        meeting_id: &str,
        speaker_ids: &[Id],
        speakers: Vec<Speaker>,
        clusters: Vec<Cluster>,
    ) -> Result<()> {
        let i = self.idx(meeting_id)?;
        let m = &mut self.entries[i].meeting;
        if speaker_ids.len() != m.segments.len() {
            return Err(format!(
                "{} speaker ids for {} segments",
                speaker_ids.len(),
                m.segments.len()
            ));
        }
        for (seg, id) in m.segments.iter_mut().zip(speaker_ids) {
            seg.speaker_id.clone_from(id);
        }
        let used = |id: &str| speaker_ids.iter().any(|s| s == id);
        let mut list: Vec<Speaker> = m
            .speakers
            .drain(..)
            .filter(|s| used(&s.id) && !speakers.iter().any(|n| n.id == s.id))
            .collect();
        list.extend(speakers.into_iter().filter(|s| used(&s.id)));
        m.speakers = list;
        let mut v = self.voices();
        v.clusters.retain(|c| c.meeting_id != meeting_id);
        v.clusters.extend(clusters);
        self.save_voices(&v)?;
        self.save(i, true)?;
        let ids = speaker_ids.to_vec();
        self.sync_voiceprints(&v, &ids)
    }

    /// Name a speaker in one meeting.
    /// - A name a known person already has: this voice is them (merged in this meeting).
    /// - A known person given another name: a wrong match. This meeting's voice moves to a new
    ///   (or the named) person and the old person's voiceprint drops it.
    /// - Anyone else: they become a known person under that name, everywhere they appear.
    pub fn name_speaker(
        &mut self,
        meeting_id: &str,
        speaker_id: &str,
        name: &str,
    ) -> Result<Speaker> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A person needs a name.".into());
        }
        let i = self.idx(meeting_id)?;
        if !self.entries[i]
            .meeting
            .speakers
            .iter()
            .any(|s| s.id == speaker_id)
        {
            return Err(format!("no speaker {speaker_id} in this meeting"));
        }
        let mut v = self.voices();
        let target = v.by_name(name).cloned();
        match (target, v.person(speaker_id).cloned()) {
            (Some(t), _) if t.id == speaker_id => self.person_speaker(&t.id),
            (Some(t), _) => {
                v.relabel(Some(meeting_id), speaker_id, &t.id);
                self.save_voices(&v)?;
                let to = self.person_speaker(&t.id)?;
                self.relabel(i, speaker_id, &to)?;
                self.sync_voiceprints(&v, &[speaker_id.into(), t.id.clone()])?;
                self.person_speaker(&t.id)
            }
            // A known person under another name, or the generic source voice "them": a new
            // person, in this meeting only.
            (None, known) if known.is_some() || GENERIC.contains(&speaker_id) => {
                let id = new_person(&mut v, &self.people, meeting_id, name);
                v.relabel(Some(meeting_id), speaker_id, &id);
                self.save_voices(&v)?;
                self.relabel(i, speaker_id, &named(&id, name))?;
                self.sync_voiceprints(&v, &[speaker_id.into(), id.clone()])?;
                self.person_speaker(&id)
            }
            (None, _) => self.enroll_speaker(speaker_id, name),
        }
    }

    /// Name a voice everywhere it appears (and rename a known person). It joins the registry,
    /// so later meetings recognize it. Segments, the summary and action-item owners follow.
    pub fn enroll_speaker(&mut self, speaker_id: &str, name: &str) -> Result<Speaker> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A person needs a name.".into());
        }
        let old = self
            .people
            .iter()
            .find(|p| p.id == speaker_id)
            .ok_or("no speaker")?
            .name
            .clone();
        let mut v = self.voices();
        match v.people.iter_mut().find(|p| p.id == speaker_id) {
            Some(p) => p.name = name.into(),
            None => v.people.push(Person {
                id: speaker_id.into(),
                name: name.into(),
            }),
        }
        self.save_voices(&v)?;
        let seen: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.meeting.speakers.iter().any(|s| s.id == speaker_id))
            .map(|e| e.meeting.title.clone())
            .collect();
        let subtitle = match seen.as_slice() {
            [] => String::new(),
            [only] => format!("Named in {only}"),
            [.., last] => format!("Named in {last} and {} more", seen.len() - 1),
        };
        for i in 0..self.entries.len() {
            let e = &self.entries[i];
            if !e.meeting.speakers.iter().any(|s| s.id == speaker_id)
                && !e.items.iter().any(|a| a.owner == old)
            {
                continue;
            }
            let mut to = e
                .meeting
                .speakers
                .iter()
                .find(|s| s.id == speaker_id)
                .cloned()
                .unwrap_or_else(|| named(speaker_id, name));
            rename(&mut to, name);
            to.subtitle.clone_from(&subtitle);
            self.relabel(i, speaker_id, &to)?;
        }
        self.sync_voiceprints(&v, &[speaker_id.into()])?;
        self.person_speaker(speaker_id)
    }

    /// Merge two people (or voices): `from` becomes `into` in every meeting, and its voice
    /// joins `into`'s voiceprint.
    pub fn merge_people(&mut self, from: &str, into: &str) -> Result<Speaker> {
        if from == into {
            return self.person_speaker(into);
        }
        let to = self.person_speaker(into)?;
        self.person_speaker(from)?;
        let mut v = self.voices();
        let was_known = v.person(from).is_some();
        v.people.retain(|p| p.id != from);
        if was_known && v.person(into).is_none() {
            v.people.push(Person {
                id: into.into(),
                name: to.name.clone(),
            });
        }
        v.relabel(None, from, into);
        self.save_voices(&v)?;
        for i in 0..self.entries.len() {
            if self.entries[i]
                .meeting
                .speakers
                .iter()
                .any(|s| s.id == from)
            {
                let mine = self.entries[i]
                    .meeting
                    .speakers
                    .iter()
                    .find(|s| s.id == into)
                    .cloned()
                    .unwrap_or_else(|| to.clone());
                self.relabel(i, from, &mine)?;
            }
        }
        self.sync_voiceprints(&v, &[from.into(), into.into()])?;
        self.person_speaker(into)
    }

    /// Replace the name suggested for a speaker of one meeting (`None` clears it).
    pub fn set_suggestion(
        &mut self,
        meeting_id: &str,
        speaker_id: &str,
        suggestion: Option<NameSuggestion>,
    ) -> Result<()> {
        let i = self.idx(meeting_id)?;
        let sp = self.entries[i]
            .meeting
            .speakers
            .iter_mut()
            .find(|s| s.id == speaker_id)
            .ok_or("no speaker")?;
        sp.suggestion = suggestion;
        self.save(i, false)?;
        self.refresh_people();
        Ok(())
    }

    /// Move one line to another person (NGR-201): a known name (any case) is that person, any
    /// other name becomes a new person with no voiceprint. The line is marked `by_hand`, so
    /// speaker detection leaves it alone; a voice left with no lines leaves the meeting.
    // ponytail: a moved line never updates voiceprints (one line is too short, a misclick
    // shouldn't train a voice); NGR-201 if users ask for it.
    pub fn set_line_speaker(
        &mut self,
        meeting_id: &str,
        segment_id: &str,
        name: &str,
    ) -> Result<Speaker> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A person needs a name.".into());
        }
        let i = self.idx(meeting_id)?;
        let from = self.entries[i]
            .meeting
            .segments
            .iter()
            .find(|g| g.id == segment_id)
            .ok_or_else(|| format!("no line {segment_id} in this meeting"))?
            .speaker_id
            .clone();
        if from == "me" {
            return Err("Lines from your mic can't be moved.".into());
        }
        let mut v = self.voices();
        let to = if let Some(p) = v.by_name(name) {
            p.id.clone()
        } else {
            let id = new_person(&mut v, &self.people, meeting_id, name);
            self.save_voices(&v)?;
            id
        };
        if to == "me" {
            return Err("Lines can't be moved to you.".into());
        }
        let sp = self
            .person_speaker(&to)
            .unwrap_or_else(|_| named(&to, name));
        let m = &mut self.entries[i].meeting;
        for g in m.segments.iter_mut().filter(|g| g.id == segment_id) {
            g.speaker_id.clone_from(&to);
            g.by_hand = true;
        }
        if !m.speakers.iter().any(|s| s.id == to) {
            m.speakers.push(sp.clone());
        }
        if !m.segments.iter().any(|g| g.speaker_id == from) {
            m.speakers.retain(|s| s.id != from);
        }
        self.save(i, true)?;
        self.refresh_people();
        Ok(sp)
    }

    /// Voices in a meeting nobody named yet (not in the registry), "me" aside.
    pub fn unnamed_speakers(&self, meeting_id: &str) -> Result<Vec<Id>> {
        let v = self.voices();
        Ok(self.entries[self.idx(meeting_id)?]
            .meeting
            .speakers
            .iter()
            .filter(|s| s.id != "me" && v.person(&s.id).is_none())
            .map(|s| s.id.clone())
            .collect())
    }

    /// Forget a person's voice: their voiceprint and every embedding behind it are deleted.
    /// Meetings keep the name; new meetings won't recognize the voice any more.
    pub fn delete_voiceprint(&mut self, person_id: &str) -> Result<()> {
        let mut v = self.voices();
        v.people.retain(|p| p.id != person_id);
        v.clusters.retain(|c| c.speaker_id != person_id);
        self.save_voices(&v)?;
        self.sync_voiceprints(&v, &[person_id.into()])
    }

    /// Drop the anonymous voices of a deleted meeting. Voices of known people stay: they are
    /// those people's voiceprints (`delete_voiceprint` removes them).
    pub(crate) fn forget_meeting_voices(&self, meeting_id: &str) -> Result<()> {
        let mut v = self.voices();
        let before = v.clusters.len();
        let known: Vec<Id> = v.people.iter().map(|p| p.id.clone()).collect();
        v.clusters
            .retain(|c| c.meeting_id != meeting_id || known.contains(&c.speaker_id));
        if v.clusters.len() == before {
            return Ok(());
        }
        self.save_voices(&v)
    }

    /// The newest copy of a person as a speaker.
    fn person_speaker(&self, id: &str) -> Result<Speaker> {
        self.people
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| format!("no speaker {id}"))
    }

    /// In meeting `i`, speaker `from` becomes `to`: segments, the speaker list, tag labels,
    /// owners, action items and the summary text.
    fn relabel(&mut self, i: usize, from: &str, to: &Speaker) -> Result<()> {
        let e = &mut self.entries[i];
        let m = &mut e.meeting;
        let old = m
            .speakers
            .iter()
            .find(|s| s.id == from)
            .map(|s| s.name.clone());
        for seg in m.segments.iter_mut().filter(|s| s.speaker_id == from) {
            seg.speaker_id.clone_from(&to.id);
        }
        let at = m
            .speakers
            .iter()
            .position(|s| s.id == from || s.id == to.id);
        m.speakers.retain(|s| s.id != from && s.id != to.id);
        m.speakers
            .insert(at.unwrap_or(m.speakers.len()), to.clone());
        if let Some(old) = old.filter(|o| *o != to.name) {
            let new = to.name.as_str();
            for seg in &mut m.segments {
                for t in &mut seg.tags {
                    t.label = replace_name(&t.label, &old, new);
                }
                for o in seg.owners.iter_mut().filter(|o| o.label == old) {
                    o.label = new.into();
                }
            }
            if let Some(s) = &mut m.summary {
                s.overview = replace_name(&s.overview, &old, new);
                for d in s.decisions.iter_mut().chain(&mut s.open_questions) {
                    d.text = replace_name(&d.text, &old, new);
                }
            }
            for a in e.items.iter_mut().filter(|a| a.owner == old) {
                a.owner = new.into();
            }
        }
        self.save(i, true)?;
        self.refresh_people();
        Ok(())
    }

    /// Bring `Speaker.voiceprint` (meeting count) of speakers `ids` in line with the registry,
    /// in every meeting.
    fn sync_voiceprints(&mut self, v: &Voices, ids: &[Id]) -> Result<()> {
        for i in 0..self.entries.len() {
            let mut changed = false;
            let speakers = self.entries[i].meeting.speakers.iter_mut();
            for s in speakers.filter(|s| ids.contains(&s.id)) {
                let vp = v.person(&s.id).and_then(|p| {
                    let meetings = v.meetings(&p.id);
                    (meetings > 0).then_some(Voiceprint { meetings })
                });
                if vp.as_ref().map(|x| x.meetings) != s.voiceprint.as_ref().map(|x| x.meetings) {
                    s.voiceprint = vp;
                    changed = true;
                }
                if v.person(&s.id).is_some() && s.suggestion.is_some() {
                    s.suggestion = None;
                    changed = true;
                }
            }
            if changed {
                self.save(i, false)?;
            }
        }
        self.refresh_people();
        Ok(())
    }
}

fn rename(s: &mut Speaker, name: &str) {
    s.name = name.into();
    s.initial = name.chars().next().unwrap_or('?').to_uppercase().collect();
    s.suggestion = None;
}

/// Register a new person (no voiceprint yet) under a free `{meeting_id}-p{n}` id.
fn new_person(v: &mut Voices, people: &[Speaker], meeting_id: &str, name: &str) -> Id {
    let id = (1..=v.people.len() + people.len() + 1)
        .map(|n| format!("{meeting_id}-p{n}"))
        .find(|id| v.person(id).is_none() && !people.iter().any(|p| p.id == *id))
        .expect("a free id");
    v.people.push(Person {
        id: id.clone(),
        name: name.into(),
    });
    id
}

fn named(id: &str, name: &str) -> Speaker {
    let mut s = crate::new_speaker(id);
    rename(&mut s, name);
    s
}

/// Replace whole-word `old` with `new` ("Speaker 1" doesn't touch "Speaker 12").
pub fn replace_name(text: &str, old: &str, new: &str) -> String {
    if old.is_empty() {
        return text.into();
    }
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let (mut out, mut last) = (String::with_capacity(text.len()), 0);
    for (at, _) in text.match_indices(old) {
        let end = at + old.len();
        if !word(text[..at].chars().next_back()) && !word(text[end..].chars().next()) {
            out.push_str(&text[last..at]);
            out.push_str(new);
            last = end;
        }
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::live_seg;
    use mirona_core::{ActionItem, Decision, Summary};

    fn cluster(meeting_id: &str, speaker_id: &str, embedding: Vec<f32>) -> Cluster {
        Cluster {
            meeting_id: meeting_id.into(),
            speaker_id: speaker_id.into(),
            speech_ms: 60_000,
            embedding,
        }
    }

    /// A finished meeting whose two "them" segments were diarized into `voices` (id, embedding).
    fn diarized(s: &mut Store, id: &str, voices: &[(&str, Vec<f32>)]) {
        s.start_meeting(id).unwrap();
        s.push_segment(id, live_seg(&format!("{id}-0"), "me", 0, "Hi all."))
            .unwrap();
        for (n, (who, _)) in voices.iter().enumerate() {
            let at = 1000 * (u32::try_from(n).unwrap() + 1);
            let text = format!("{who} talking.");
            s.push_segment(id, live_seg(&format!("{id}-{}", n + 1), "them", at, &text))
                .unwrap();
        }
        let ids: Vec<Id> = std::iter::once("me".to_string())
            .chain(voices.iter().map(|(w, _)| (*w).to_string()))
            .collect();
        let known = s.voices();
        let speakers = voices
            .iter()
            .enumerate()
            .map(|(n, (w, _))| match known.person(w) {
                Some(p) => named(w, &p.name),
                None => named(w, &format!("Speaker {}", n + 1)),
            })
            .collect();
        let clusters = voices
            .iter()
            .map(|(w, e)| cluster(id, w, e.clone()))
            .collect();
        s.set_diarization(id, &ids, speakers, clusters).unwrap();
    }

    fn names(s: &Store, id: &str) -> Vec<String> {
        let m = s.meeting(id).unwrap();
        m.segments
            .iter()
            .map(|g| {
                let sp = m.speakers.iter().find(|p| p.id == g.speaker_id).unwrap();
                sp.name.clone()
            })
            .collect()
    }

    #[test]
    fn registry_round_trip_and_voiceprints() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        assert_eq!(s.voices(), Voices::default());
        diarized(
            &mut s,
            "m-a",
            &[("m-a-s1", vec![1.0, 0.0]), ("m-a-s2", vec![0.0, 1.0])],
        );
        assert_eq!(names(&s, "m-a"), ["Me", "Speaker 1", "Speaker 2"]);
        // Nobody is known yet: nothing to match against.
        assert_eq!(s.voiceprints(), []);

        let ana = s.name_speaker("m-a", "m-a-s1", "Ana").unwrap();
        assert_eq!((ana.id.as_str(), ana.name.as_str()), ("m-a-s1", "Ana"));
        assert_eq!(ana.voiceprint.unwrap().meetings, 1);
        assert_eq!(s.voiceprints(), [("m-a-s1".to_string(), vec![1.0, 0.0])]);

        // On disk, outside meeting.md, and read back by a fresh store.
        let path = dir.path().join(".mirona/voices.json");
        assert!(path.is_file());
        let md = std::fs::read_to_string(s.meeting_dir("m-a").unwrap().join("meeting.md")).unwrap();
        assert!(!md.contains("embedding"));
        let s2 = Store::open(dir.path()).unwrap();
        assert_eq!(s2.voices(), s.voices());
        assert_eq!(names(&s2, "m-a"), ["Me", "Ana", "Speaker 2"]);
    }

    #[test]
    fn a_known_voice_is_named_and_a_wrong_match_is_fixed() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        diarized(&mut s, "m-a", &[("m-a-s1", vec![1.0, 0.0])]);
        s.name_speaker("m-a", "m-a-s1", "Ana").unwrap();
        // Meeting b: the engine matched a voice to Ana (id m-a-s1)…
        diarized(
            &mut s,
            "m-b",
            &[("m-a-s1", vec![0.6, 0.8]), ("m-b-s2", vec![0.0, 1.0])],
        );
        assert_eq!(names(&s, "m-b"), ["Me", "Ana", "Speaker 2"]);
        assert_eq!(
            s.people()
                .iter()
                .find(|p| p.id == "m-a-s1")
                .unwrap()
                .voiceprint
                .as_ref()
                .unwrap()
                .meetings,
            2
        );
        let vp = s.voiceprints()[0].1.clone();
        assert!(vp[1] > 0.3, "b's voice pulled the voiceprint: {vp:?}");

        // …but it was Bo. Fixing it moves b's voice off Ana's voiceprint.
        let bo = s.name_speaker("m-b", "m-a-s1", "Bo").unwrap();
        assert_eq!(bo.name, "Bo");
        assert_ne!(bo.id, "m-a-s1");
        assert_eq!(names(&s, "m-a"), ["Me", "Ana"]);
        assert_eq!(names(&s, "m-b"), ["Me", "Bo", "Speaker 2"]);
        let prints = s.voiceprints();
        assert_eq!(prints.len(), 2);
        assert_eq!(prints[0], ("m-a-s1".to_string(), vec![1.0, 0.0]));
        assert_eq!(prints[1].0, bo.id);
        assert_eq!(
            s.people()
                .iter()
                .find(|p| p.id == "m-a-s1")
                .unwrap()
                .voiceprint
                .as_ref()
                .unwrap()
                .meetings,
            1
        );

        // Naming another voice with a known name merges it into that person, in this meeting.
        let ana = s.name_speaker("m-b", "m-b-s2", "ana").unwrap();
        assert_eq!(ana.id, "m-a-s1");
        assert_eq!(names(&s, "m-b"), ["Me", "Bo", "Ana"]);
        assert_eq!(s.voices().meetings("m-a-s1"), 2);
    }

    #[test]
    fn merge_and_delete_voiceprint() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        diarized(&mut s, "m-a", &[("m-a-s1", vec![1.0, 0.0])]);
        diarized(&mut s, "m-b", &[("m-b-s1", vec![0.8, 0.6])]);
        s.name_speaker("m-a", "m-a-s1", "Ana").unwrap();
        let ana = s.merge_people("m-b-s1", "m-a-s1").unwrap();
        assert_eq!(ana.voiceprint.unwrap().meetings, 2);
        assert_eq!(names(&s, "m-b"), ["Me", "Ana"]);
        assert!(s.people().iter().all(|p| p.id != "m-b-s1"));
        assert_eq!(s.voices().clusters.len(), 2);

        s.delete_voiceprint("m-a-s1").unwrap();
        let v = s.voices();
        assert!(v.people.is_empty() && v.clusters.is_empty(), "{v:?}");
        assert_eq!(s.voiceprints(), []);
        // The meetings keep the name; only the voice is forgotten.
        assert_eq!(names(&s, "m-b"), ["Me", "Ana"]);
        let p = s
            .people()
            .iter()
            .find(|p| p.id == "m-a-s1")
            .unwrap()
            .clone();
        assert!(p.voiceprint.is_none());
    }

    #[test]
    fn renaming_updates_segments_summary_and_owners() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        diarized(
            &mut s,
            "m-a",
            &[("m-a-s1", vec![1.0, 0.0]), ("m-a-s12", vec![0.0, 1.0])],
        );
        let item = |owner: &str| ActionItem {
            id: format!("a-{owner}"),
            meeting_id: "m-a".into(),
            meeting_title: String::new(),
            at_ms: Some(1000),
            text: "Send the SOW".into(),
            owner: owner.into(),
            due: None,
            tags: vec![],
            done: false,
            needs_review: false,
            owner_confirmed: false,
        };
        let summary = Summary {
            template: "general".into(),
            title: String::new(),
            overview: "Speaker 1 agreed; Speaker 12 objected.".into(),
            decisions: vec![Decision {
                text: "Speaker 1 owns pricing".into(),
                at_ms: 1000,
            }],
            open_questions: vec![],
            written_by: "test".into(),
            quality: String::new(),
        };
        s.set_summary("m-a", summary, vec![item("Speaker 1"), item("Speaker 2")])
            .unwrap();
        // "Speaker 12" in the transcript is the second voice, named "Speaker 2".
        s.name_speaker("m-a", "m-a-s1", "Inés").unwrap();
        let s2 = Store::open(dir.path()).unwrap();
        let m = s2.meeting("m-a").unwrap();
        let sum = m.summary.unwrap();
        assert_eq!(sum.overview, "Inés agreed; Speaker 12 objected.");
        assert_eq!(sum.decisions[0].text, "Inés owns pricing");
        let owners: Vec<String> = s2.action_items().into_iter().map(|a| a.owner).collect();
        assert_eq!(owners, ["Inés", "Speaker 2"]);
        assert_eq!(names(&s2, "m-a"), ["Me", "Inés", "Speaker 2"]);
        // A global rename of a known person reaches every meeting too.
        s.enroll_speaker("m-a-s1", "Ines").unwrap();
        assert_eq!(names(&s, "m-a")[1], "Ines");
        assert_eq!(s.voices().people[0].name, "Ines");
    }

    #[test]
    fn deleting_a_meeting_keeps_known_voices_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        diarized(
            &mut s,
            "m-a",
            &[("m-a-s1", vec![1.0, 0.0]), ("m-a-s2", vec![0.0, 1.0])],
        );
        s.name_speaker("m-a", "m-a-s1", "Ana").unwrap();
        s.delete_meeting("m-a").unwrap();
        let v = s.voices();
        assert_eq!(v.clusters.len(), 1);
        assert_eq!(v.clusters[0].speaker_id, "m-a-s1");
    }

    #[test]
    fn a_line_moves_to_a_known_person_and_the_empty_voice_drops() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        diarized(
            &mut s,
            "m-a",
            &[("m-a-s1", vec![1.0, 0.0]), ("m-a-s2", vec![0.0, 1.0])],
        );
        s.name_speaker("m-a", "m-a-s1", "Ana").unwrap();
        let sp = s.set_line_speaker("m-a", "m-a-2", "ana").unwrap();
        assert_eq!(sp.id, "m-a-s1");
        assert_eq!(names(&s, "m-a"), ["Me", "Ana", "Ana"]);
        let m = s.meeting("m-a").unwrap();
        assert!(m.segments[2].by_hand);
        assert!(!m.segments[1].by_hand);
        assert!(!m.speakers.iter().any(|p| p.id == "m-a-s2"));
        assert_eq!(s.voices().clusters.len(), 2, "voiceprints untouched");
    }

    #[test]
    fn a_line_moves_to_a_new_person_who_joins_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        diarized(&mut s, "m-a", &[("m-a-s1", vec![1.0, 0.0])]);
        let bo = s.set_line_speaker("m-a", "m-a-1", "Bo").unwrap();
        assert!(bo.id.starts_with("m-a-p"), "{}", bo.id);
        assert_eq!(bo.name, "Bo");
        assert_eq!(names(&s, "m-a"), ["Me", "Bo"]);
        assert!(s.voices().people.iter().any(|p| p.name == "Bo"));
        assert_eq!(s.unnamed_speakers("m-a").unwrap(), Vec::<Id>::new());
        // Naming a voice "Bo" later is the same person, not a second Bo.
        diarized(&mut s, "m-b", &[("m-b-s1", vec![0.0, 1.0])]);
        let again = s.name_speaker("m-b", "m-b-s1", "Bo").unwrap();
        assert_eq!(again.id, bo.id);
    }

    #[test]
    fn mic_lines_and_moves_to_me_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        diarized(&mut s, "m-a", &[("m-a-s1", vec![1.0, 0.0])]);
        assert!(s.set_line_speaker("m-a", "m-a-0", "Ana").is_err());
        assert!(s.set_line_speaker("m-a", "m-a-1", "  ").is_err());
        assert!(s.set_line_speaker("m-a", "nope", "Ana").is_err());
        assert_eq!(names(&s, "m-a"), ["Me", "Speaker 1"]);
    }

    #[test]
    fn unnamed_speakers_lists_unknown_voices_but_not_me() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        diarized(
            &mut s,
            "m-a",
            &[("m-a-s1", vec![1.0, 0.0]), ("m-a-s2", vec![0.0, 1.0])],
        );
        s.name_speaker("m-a", "m-a-s1", "Ana").unwrap();
        assert_eq!(s.unnamed_speakers("m-a").unwrap(), ["m-a-s2"]);
        // Not diarized: the other side is still "them".
        s.start_meeting("m-b").unwrap();
        s.push_segment("m-b", live_seg("m-b-0", "me", 0, "Hi."))
            .unwrap();
        s.push_segment("m-b", live_seg("m-b-1", "them", 900, "Hello."))
            .unwrap();
        assert_eq!(s.unnamed_speakers("m-b").unwrap(), ["them"]);
    }

    #[test]
    fn moving_a_line_works_on_an_encrypted_store() {
        let dir = tempfile::tempdir().unwrap();
        let key = crate::crypt::Key::generate().unwrap();
        let mut s = Store::open_with(dir.path(), Some(key.clone())).unwrap();
        diarized(&mut s, "m-a", &[("m-a-s1", vec![1.0, 0.0])]);
        s.set_line_speaker("m-a", "m-a-1", "Bo").unwrap();
        let s2 = Store::open_with(dir.path(), Some(key)).unwrap();
        let m = s2.meeting("m-a").unwrap();
        let line = m.segments.iter().find(|g| g.id == "m-a-1").unwrap();
        assert!(line.by_hand);
        assert_eq!(names(&s2, "m-a"), ["Me", "Bo"]);
    }

    #[test]
    fn whole_word_replace() {
        assert_eq!(
            replace_name("Speaker 1 and Speaker 12.", "Speaker 1", "Ana"),
            "Ana and Speaker 12."
        );
        assert_eq!(replace_name("Bo, Bob", "Bo", "Ana"), "Ana, Bob");
        assert_eq!(replace_name("x", "", "y"), "x");
    }
}
