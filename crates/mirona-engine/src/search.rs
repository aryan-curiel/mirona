//! Hybrid search (NGR-167) as the app, the CLI and MCP all run it: the query is embedded with
//! the same local model as the meetings, then `Store::hybrid_search` fuses keywords and
//! meaning. Without the embedder (off in config, Ollama not running, model not pulled) it
//! falls back to keywords alone, and says so (`SearchResults::semantic`).
//!
//! `[providers] embeddings = "ollama"` (default) or `"off"`. Lines are embedded after each call
//! ([`embed_meeting`]) and older meetings on app start ([`backfill`]).

use crate::{Shared, stage};
use mirona_core::{SearchFilters, SearchResults};
use mirona_providers::config::Config;
use mirona_providers::ollama::OllamaEmbedder;
use mirona_store::Store;

/// nomic-embed-text was trained with these task prefixes.
const QUERY: &str = "search_query: ";
const DOCUMENT: &str = "search_document: ";
/// Lines per embedding call.
const BATCH: usize = 64;

/// Something that turns texts into vectors, named so stored vectors can be matched to it.
pub trait Embed {
    /// Stored next to the vectors; a different name means re-embedding.
    fn model(&self) -> String;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
}

impl Embed for OllamaEmbedder {
    fn model(&self) -> String {
        format!("ollama:{}", self.model)
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        self.embed_many(texts).map_err(|e| e.to_string())
    }
}

/// The configured embedder, or `None` when search by meaning is off.
pub fn embedder(cfg: &Config) -> Option<OllamaEmbedder> {
    (stage(cfg, "embeddings", "ollama") != "off").then(|| {
        let url = crate::ollama_url(cfg, "/api/embed");
        OllamaEmbedder::new(mirona_providers::ollama::EMBED_MODEL, url)
    })
}

/// Embed the lines of a meeting that have no current vector. Returns how many were embedded.
pub fn embed_meeting(store: &Shared, id: &str, emb: &impl Embed) -> Result<usize, String> {
    let model = emb.model();
    let todo = store.lock().unwrap().embedding_todo(id, &model)?;
    let mut done = vec![];
    for batch in todo.chunks(BATCH) {
        let texts: Vec<String> = batch
            .iter()
            .map(|(_, t)| format!("{DOCUMENT}{t}"))
            .collect();
        let vectors = emb.embed(&texts)?;
        done.extend(batch.iter().map(|(id, _)| id.clone()).zip(vectors));
    }
    let n = done.len();
    if n > 0 {
        store.lock().unwrap().set_embeddings(id, &model, done)?;
    }
    Ok(n)
}

/// Embed every meeting still missing vectors. Stops at the first failure (Ollama is down or
/// the model isn't pulled: every other meeting would fail the same way).
pub fn backfill(store: &Shared, emb: &impl Embed) -> Result<usize, String> {
    let ids: Vec<String> = store
        .lock()
        .unwrap()
        .list_meetings()
        .into_iter()
        .map(|m| m.id)
        .collect();
    let mut n = 0;
    for id in ids {
        n += embed_meeting(store, &id, emb)?;
    }
    Ok(n)
}

/// Search with meaning when the embedder answers, keywords otherwise.
pub fn search(
    store: &Store,
    cfg: &Config,
    query: &str,
    filters: &SearchFilters,
    limit: usize,
) -> SearchResults {
    let emb = embedder(cfg);
    search_with(store, emb.as_ref(), query, filters, limit)
}

pub fn search_with(
    store: &Store,
    emb: Option<&impl Embed>,
    query: &str,
    filters: &SearchFilters,
    limit: usize,
) -> SearchResults {
    let meaning = emb
        .filter(|_| !query.trim().is_empty())
        .and_then(|e| e.embed(&[format!("{QUERY}{}", query.trim())]).ok())
        .and_then(|mut v| v.pop());
    store.hybrid_search(query, meaning.as_deref(), filters, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirona_core::Segment;
    use std::sync::{Arc, Mutex};

    /// Meaning by topic word, so the test can tell meaning from keywords.
    struct Topics;
    impl Embed for Topics {
        fn model(&self) -> String {
            "topics".into()
        }
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            Ok(texts
                .iter()
                .map(|t| {
                    let t = t.to_lowercase();
                    vec![
                        f32::from(u8::from(t.contains("price") || t.contains("budget"))),
                        f32::from(u8::from(t.contains("hiring") || t.contains("recruit"))),
                    ]
                })
                .collect())
        }
    }

    struct Down;
    impl Embed for Down {
        fn model(&self) -> String {
            "down".into()
        }
        fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, String> {
            Err("connection refused".into())
        }
    }

    /// Real nomic-embed-text through Ollama on the demo meetings: a paraphrase with no shared
    /// word finds the line, and the sensitive one is never embedded.
    /// `ollama pull nomic-embed-text && cargo test -p mirona-engine real_embeddings -- --ignored`
    #[test]
    #[ignore = "needs Ollama with nomic-embed-text"]
    fn real_embeddings_find_paraphrases() {
        let shared: Shared = Arc::new(Mutex::new(Store::sample()));
        let emb = OllamaEmbedder::default();
        assert!(backfill(&shared, &emb).unwrap() > 10);
        let store = shared.lock().unwrap();
        let f = SearchFilters::default();
        for (q, expect) in [
            ("football game", "match last night"),
            ("how long will the work take", "five days"),
            (
                "old payments after moving systems",
                "already paid before the migration",
            ),
        ] {
            let r = search_with(&store, Some(&emb), q, &f, 5);
            assert!(r.semantic);
            let top: Vec<&str> = r.moments.iter().map(|m| m.snippet.as_str()).collect();
            assert!(top.iter().any(|s| s.contains(expect)), "{q}: {top:#?}");
        }
        let r = search_with(&store, Some(&emb), "staging credentials", &f, 10);
        assert!(
            r.moments.iter().all(|m| !m.snippet.contains("Alder")),
            "{r:?}"
        );
    }

    #[test]
    fn meetings_are_embedded_and_searched_by_meaning() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        store.start_meeting("m-1").unwrap();
        for (n, text) in ["We need to recruit two engineers.", "The budget is fixed."]
            .iter()
            .enumerate()
        {
            let n32 = u32::try_from(n).unwrap();
            store
                .push_segment(
                    "m-1",
                    Segment {
                        id: format!("m-1-{n}"),
                        speaker_id: "them".into(),
                        start_ms: n32 * 1000,
                        end_ms: 0,
                        text: (*text).into(),
                        redacted: false,
                        tags: vec![],
                        answers: vec![],
                        owners: vec![],
                        due: None,
                        by_hand: false,
                    },
                )
                .unwrap();
        }
        let shared: Shared = Arc::new(Mutex::new(store));
        assert_eq!(backfill(&shared, &Topics).unwrap(), 2);
        assert_eq!(embed_meeting(&shared, "m-1", &Topics).unwrap(), 0, "kept");
        assert!(backfill(&shared, &Down).is_err());

        let store = shared.lock().unwrap();
        let f = SearchFilters::default();
        let r = search_with(&store, Some(&Topics), "hiring plans", &f, 10);
        assert!(r.semantic);
        assert_eq!(r.moments.len(), 1);
        assert!(r.moments[0].snippet.contains("recruit"));
        // The embedder is down: keywords still work.
        let r = search_with(&store, Some(&Down), "budget", &f, 10);
        assert!(!r.semantic);
        assert_eq!(r.moments.len(), 1);
        let r = search_with(&store, None::<&Down>, "hiring plans", &f, 10);
        assert_eq!(r.moments.len(), 0);
    }
}
