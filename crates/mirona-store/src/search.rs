//! Hybrid search (NGR-167): keywords (`SQLite` FTS5) plus meaning (one embedding per transcript
//! line), merged with reciprocal rank fusion and narrowed by meeting tag, dates, speaker and tag.
//!
//! Embeddings live next to the transcript in `<meeting>/embeddings.json`, so deleting a
//! meeting deletes them and moving the folder keeps them. Each vector carries a hash of the
//! line it was made from: an edited line is simply re-embedded. Sensitive and redacted lines
//! are never embedded. The store doesn't embed anything itself: the engine asks for what's
//! missing ([`Store::embedding_todo`]) and hands vectors back ([`Store::set_embeddings`]).
//! ponytail: brute-force cosine over every vector in memory; fine for ~100k lines, use
//! sqlite-vec or an ANN index past that.

use crate::files::Entry;
use crate::{Result, Store, index, when_labels};
use mirona_core::{Id, MeetingSummary, Moment, SearchFilters, SearchResults, Segment};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const EMBEDDINGS: &str = "embeddings.json";

/// Reciprocal rank fusion constant (the usual 60).
const RRF_K: f32 = 60.0;
/// Most nearest neighbours a query takes from the embeddings.
const MEANING_TOP: usize = 50;
/// Below this cosine a line isn't "about" the query, and lines more than 0.1 below the
/// best match are noise. ponytail: measured on the demo meetings with nomic-embed-text and its
/// search prefixes (related 0.54–0.65, unrelated 0.35–0.52); make both per model if another
/// one is used.
pub const MIN_SIMILARITY: f32 = 0.5;
const WINDOW: f32 = 0.1;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Vectors {
    pub model: String,
    pub segments: BTreeMap<Id, Vector>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vector {
    /// [`text_hash`] of the line the vector was made from.
    pub hash: String,
    pub v: Vec<f32>,
}

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
pub fn text_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn embeddable(s: &Segment) -> bool {
    !s.redacted && !is_sensitive(s) && !s.text.trim().is_empty()
}

pub(crate) fn is_sensitive(s: &Segment) -> bool {
    s.is_sensitive()
}

fn normalized(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / n).collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn has(hay: &str, needle: Option<&String>) -> bool {
    needle.is_none_or(|n| hay.to_lowercase().contains(&n.trim().to_lowercase()))
}

fn speaker_name<'a>(e: &'a Entry, s: &'a Segment) -> &'a str {
    e.meeting
        .speakers
        .iter()
        .find(|p| p.id == s.speaker_id)
        .map_or(s.speaker_id.as_str(), |p| p.name.as_str())
}

/// Meeting tag and date range: the filters that apply to a whole meeting.
fn meeting_ok(e: &Entry, f: &SearchFilters) -> bool {
    let day = crate::timestamp(&e.created).map(|t| {
        t.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d")
            .to_string()
    });
    let day = day.unwrap_or_default();
    f.meeting_tag.as_ref().is_none_or(|_| {
        e.meeting
            .tags
            .iter()
            .any(|t| has(t, f.meeting_tag.as_ref()))
    }) && f.from.as_ref().is_none_or(|from| day >= *from)
        && f.to.as_ref().is_none_or(|to| day <= *to)
}

fn segment_ok(e: &Entry, s: &Segment, f: &SearchFilters) -> bool {
    has(speaker_name(e, s), f.speaker.as_ref())
        && f.tag.is_none_or(|k| s.tags.iter().any(|t| t.kind == k))
}

/// A line found by keywords, meaning or both, with its fused score.
#[derive(Default)]
struct Candidate {
    score: f32,
    keyword: bool,
    meaning: bool,
    /// The keyword snippet, with hits in brackets.
    snippet: Option<String>,
}

#[allow(clippy::cast_precision_loss)]
fn rrf(rank: usize) -> f32 {
    1.0 / (RRF_K + rank as f32 + 1.0)
}

impl Store {
    /// Keyword hits and nearest lines by meaning, fused by rank: (meeting, segment) → found.
    fn candidates(&self, query: &str, meaning: Option<&[f32]>) -> HashMap<(Id, Id), Candidate> {
        let mut found: HashMap<(Id, Id), Candidate> = HashMap::new();
        if !query.trim().is_empty() {
            let hits = index::hits(&self.db, query).unwrap_or_default();
            for (rank, h) in hits.into_iter().enumerate() {
                let f = found.entry((h.meeting_id, h.segment_id)).or_default();
                f.score += rrf(rank);
                f.keyword = true;
                f.snippet = Some(h.snippet);
            }
        }
        let Some(q) = meaning else {
            return found;
        };
        let q = normalized(q);
        let mut near: Vec<(f32, &Entry, &Id)> = vec![];
        for e in &self.entries {
            for s in e.meeting.segments.iter().filter(|s| embeddable(s)) {
                let Some(v) = e.vectors.segments.get(&s.id) else {
                    continue;
                };
                if v.v.len() != q.len() || v.hash != text_hash(&s.text) {
                    continue;
                }
                let sim = dot(&q, &normalized(&v.v));
                if sim >= MIN_SIMILARITY {
                    near.push((sim, e, &s.id));
                }
            }
        }
        near.sort_by(|a, b| b.0.total_cmp(&a.0));
        let best = near.first().map_or(0.0, |n| n.0);
        near.retain(|n| n.0 >= best - WINDOW);
        for (rank, (_, e, sid)) in near.into_iter().take(MEANING_TOP).enumerate() {
            let f = found
                .entry((e.meeting.id.clone(), sid.clone()))
                .or_default();
            f.score += rrf(rank);
            f.meaning = true;
        }
        found
    }

    /// Lines of a meeting that have no up-to-date vector from `model`: `(segment id, text)`.
    pub fn embedding_todo(&self, meeting_id: &str, model: &str) -> Result<Vec<(Id, String)>> {
        let e = &self.entries[self.idx(meeting_id)?];
        let same_model = e.vectors.model == model;
        Ok(e.meeting
            .segments
            .iter()
            .filter(|s| embeddable(s))
            .filter(|s| {
                !same_model
                    || e.vectors
                        .segments
                        .get(&s.id)
                        .is_none_or(|v| v.hash != text_hash(&s.text))
            })
            .map(|s| (s.id.clone(), s.text.clone()))
            .collect())
    }

    /// Store vectors for a meeting's lines. Vectors from another model are dropped, as are
    /// vectors of lines that are gone or became sensitive.
    pub fn set_embeddings(
        &mut self,
        meeting_id: &str,
        model: &str,
        vectors: Vec<(Id, Vec<f32>)>,
    ) -> Result<()> {
        let i = self.idx(meeting_id)?;
        let e = &mut self.entries[i];
        if e.vectors.model != model {
            e.vectors = Vectors {
                model: model.into(),
                segments: BTreeMap::new(),
            };
        }
        for (id, v) in vectors {
            if let Some(s) = e.meeting.segments.iter().find(|s| s.id == id) {
                let hash = text_hash(&s.text);
                e.vectors.segments.insert(id, Vector { hash, v });
            }
        }
        let keep: HashMap<&str, &Segment> = e
            .meeting
            .segments
            .iter()
            .filter(|s| embeddable(s))
            .map(|s| (s.id.as_str(), s))
            .collect();
        e.vectors
            .segments
            .retain(|id, _| keep.contains_key(id.as_str()));
        crate::files::write_vectors(e, self.key.as_ref())?;
        self.touched();
        Ok(())
    }

    /// Keywords and meaning together. `meaning` is the query's embedding (from the same model
    /// as the stored vectors), or `None` for keywords only. An empty query lists the meetings
    /// that pass the filters, with no moments.
    pub fn hybrid_search(
        &self,
        query: &str,
        meaning: Option<&[f32]>,
        filters: &SearchFilters,
        limit: usize,
    ) -> SearchResults {
        let by_id: HashMap<&str, &Entry> = self
            .entries
            .iter()
            .map(|e| (e.meeting.id.as_str(), e))
            .collect();
        let semantic = meaning.is_some() && !query.trim().is_empty();
        let found = self.candidates(query, meaning.filter(|_| semantic));
        let mut moments: Vec<(f32, Moment)> = found
            .into_iter()
            .filter_map(|((mid, sid), c)| {
                let Candidate {
                    score,
                    keyword: kw,
                    meaning: mean,
                    snippet,
                } = c;
                let e = by_id.get(mid.as_str())?;
                let s = e.meeting.segments.iter().find(|s| s.id == sid)?;
                if !meeting_ok(e, filters) || !segment_ok(e, s, filters) {
                    return None;
                }
                let snippet = snippet.unwrap_or_else(|| {
                    let t: String = s.text.chars().take(200).collect();
                    if t.len() < s.text.len() {
                        format!("{t}…")
                    } else {
                        t
                    }
                });
                Some((
                    score,
                    Moment {
                        meeting_id: mid,
                        title: e.meeting.title.clone(),
                        tags: e.meeting.tags.clone(),
                        date: when_labels(&e.created).1,
                        segment_id: sid,
                        start_ms: s.start_ms,
                        speaker: speaker_name(e, s).into(),
                        snippet,
                        sensitive: is_sensitive(s),
                        by_keyword: kw,
                        by_meaning: mean,
                    },
                ))
            })
            .collect();
        // Best first; ties (same rank in both lists) go to the newer line.
        moments.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| a.1.meeting_id.cmp(&b.1.meeting_id))
                .then_with(|| a.1.start_ms.cmp(&b.1.start_ms))
        });
        moments.truncate(limit);
        let moments: Vec<Moment> = moments.into_iter().map(|(_, m)| m).collect();

        let keyword_meetings: Vec<Id> = if query.trim().is_empty() {
            self.entries.iter().map(|e| e.meeting.id.clone()).collect()
        } else {
            self.search(query).into_iter().map(|m| m.id).collect()
        };
        let segment_filters = filters.speaker.is_some() || filters.tag.is_some();
        let meetings: Vec<MeetingSummary> = self
            .list_meetings()
            .into_iter()
            .filter(|m| {
                let e = by_id[m.id.as_str()];
                let hit = moments.iter().any(|x| x.meeting_id == m.id);
                let listed = keyword_meetings.contains(&m.id)
                    && (!segment_filters
                        || e.meeting.segments.iter().any(|s| segment_ok(e, s, filters)));
                meeting_ok(e, filters) && (hit || listed)
            })
            .collect();
        SearchResults {
            moments,
            meetings,
            semantic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirona_core::{Tag, TagKind};

    fn seg(n: u32, who: &str, text: &str) -> Segment {
        Segment {
            id: format!("s{n}"),
            speaker_id: who.into(),
            start_ms: n * 1000,
            end_ms: n * 1000 + 900,
            text: text.into(),
            redacted: false,
            tags: vec![],
            answers: vec![],
            owners: vec![],
            due: None,
            by_hand: false,
        }
    }

    /// A toy embedding: one dimension per topic word, so meaning is easy to reason about.
    fn embed(text: &str) -> Vec<f32> {
        let t = text.to_lowercase();
        ["price", "contract", "holiday"]
            .iter()
            .map(|w| {
                let syn: &[&str] = match *w {
                    "price" => &["price", "cost", "budget", "fee"],
                    "contract" => &["contract", "sow", "agreement"],
                    _ => &["holiday", "vacation", "weekend"],
                };
                if syn.iter().any(|s| t.contains(s)) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect()
    }

    #[test]
    fn keywords_and_meaning_merge_and_filters_narrow() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        for (id, client, lines) in [
            (
                "m-a",
                "Northwind",
                vec![
                    seg(0, "me", "The budget is tight this quarter."),
                    seg(1, "them", "Let's sign the agreement next week."),
                    seg(2, "them", "My salary is private."),
                ],
            ),
            (
                "m-b",
                "Helio",
                vec![
                    seg(0, "me", "How was your vacation?"),
                    seg(1, "them", "We need to cut the cost of the pilot."),
                ],
            ),
        ] {
            store.start_meeting(id).unwrap();
            for mut s in lines {
                s.id = format!("{id}-{}", s.id);
                if s.text.contains("salary") {
                    s.tags.push(Tag {
                        kind: TagKind::Sensitive,
                        label: "Sensitive".into(),
                    });
                }
                store.push_segment(id, s).unwrap();
            }
            let i = store.idx(id).unwrap();
            store.entries[i].meeting.tags = vec![format!("project:{client}")];
        }
        for id in ["m-a", "m-b"] {
            let todo = store.embedding_todo(id, "toy").unwrap();
            assert!(todo.iter().all(|(_, t)| !t.contains("salary")), "sensitive");
            let v = todo.iter().map(|(s, t)| (s.clone(), embed(t))).collect();
            store.set_embeddings(id, "toy", v).unwrap();
            assert_eq!(store.embedding_todo(id, "toy").unwrap(), vec![]);
        }

        // "price" is said nowhere: only meaning finds the budget and the cost lines.
        let q = embed("price");
        let r = store.hybrid_search("price", Some(&q), &SearchFilters::default(), 10);
        assert!(r.semantic);
        let texts: Vec<&str> = r.moments.iter().map(|m| m.snippet.as_str()).collect();
        assert_eq!(r.moments.len(), 2, "{texts:?}");
        assert!(r.moments.iter().all(|m| m.by_meaning && !m.by_keyword));
        assert_eq!(r.meetings.len(), 2);

        // Keywords alone, then both: a line found both ways comes first.
        let r = store.hybrid_search("cost", None, &SearchFilters::default(), 10);
        assert_eq!(r.moments.len(), 1);
        assert!(r.moments[0].snippet.contains("[cost]"), "{:?}", r.moments);
        let r = store.hybrid_search("cost", Some(&q), &SearchFilters::default(), 10);
        assert!(r.moments[0].by_keyword && r.moments[0].by_meaning);
        assert_eq!(r.moments[0].start_ms, 1000);
        assert_eq!(r.moments[0].tags, ["project:Helio"]);

        // Filters: meeting tag, speaker, tag, dates.
        let only = |f: SearchFilters| store.hybrid_search("price", Some(&q), &f, 10).moments;
        let northwind = only(SearchFilters {
            meeting_tag: Some("north".into()),
            ..SearchFilters::default()
        });
        assert_eq!(northwind.len(), 1);
        assert_eq!(northwind[0].meeting_id, "m-a");
        let them = only(SearchFilters {
            speaker: Some("THEM".into()),
            ..SearchFilters::default()
        });
        assert_eq!(them.len(), 1);
        let decisions = only(SearchFilters {
            tag: Some(TagKind::Decision),
            ..SearchFilters::default()
        });
        assert_eq!(decisions, vec![]);
        let long_ago = only(SearchFilters {
            to: Some("2000-01-01".into()),
            ..SearchFilters::default()
        });
        assert_eq!(long_ago, vec![]);

        // A sensitive line is found by keyword (the app hides it) but flagged.
        let r = store.hybrid_search("salary", None, &SearchFilters::default(), 10);
        assert!(r.moments[0].sensitive);

        // Vectors survive a reopen; an edited line needs a new one.
        let mut store = Store::open(dir.path()).unwrap();
        assert_eq!(store.embedding_todo("m-b", "toy").unwrap(), vec![]);
        store
            .edit_segment_text("m-b", "m-b-s1", "The fee is too high.")
            .unwrap();
        assert_eq!(store.embedding_todo("m-b", "toy").unwrap().len(), 1);
        assert_eq!(store.embedding_todo("m-b", "other").unwrap().len(), 2);
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(text_hash(""), "cbf29ce484222325");
        assert_ne!(text_hash("a"), text_hash("b"));
    }
}
