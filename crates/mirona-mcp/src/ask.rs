//! `ask_meeting` (NGR-168): a question in plain words, routed by the classifier (one typed
//! choice question, the same Jev / local model as the tags) to the cheapest thing that answers
//! it. Lookups (where was X said, what was decided, who owes what, what was it about) come
//! straight from the store; only open questions reach the LLM, with the relevant transcript.
//! Without a classifier (off, or unreachable) a keyword rule routes instead.
//! Sensitive and redacted lines are never used, as everywhere in MCP.

use crate::{MAX_MOMENTS, at, sensitive};
use mirona_core::{Meeting, SearchFilters};
use mirona_engine::search::{Embed, search_with};
use mirona_providers::questions::{Kind, Question, Reply};
use mirona_providers::{Classifier, LlmProvider, Prompt};
use mirona_store::Store;
use serde_json::{Value, json};
use std::fmt::Write;

pub const ROUTES: [(&str, &str); 5] = [
    (
        "search",
        "Find where something was said, or who said it: a lookup of a moment in the calls.",
    ),
    ("decisions", "What was decided or agreed."),
    (
        "action_items",
        "Tasks and to-dos: who owes what, follow-ups, deadlines.",
    ),
    ("summary", "A recap of a meeting: what it was about."),
    (
        "open_question",
        "Anything that needs reasoning over what was said: why, how, opinions, comparisons, \
         advice.",
    ),
];

/// Transcript characters an open question may send to the LLM (~6k tokens).
const CONTEXT_CHARS: usize = 24_000;

pub fn route_question() -> Question {
    Question {
        id: "route".into(),
        question: "What kind of question is this?".into(),
        instructions: "A user asks this question about their meetings. What kind of question \
                       is it?"
            .into(),
        kind: Kind::Choice(
            ROUTES
                .iter()
                .map(|(k, d)| ((*k).into(), (*d).into()))
                .collect(),
        ),
    }
}

/// The fallback when no classifier answers.
pub fn route_by_keywords(q: &str) -> &'static str {
    let q = q.to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| q.contains(w));
    if has(&["decid", "agree", "settled", "decision"]) {
        "decisions"
    } else if has(&[
        "todo",
        "to-do",
        "task",
        "action item",
        "owe",
        "follow up",
        "follow-up",
        "deadline",
        "who will",
        "who's doing",
    ]) {
        "action_items"
    } else if has(&["summar", "recap", "overview", "what was", "about"]) {
        "summary"
    } else if has(&["where", "when did", "who said", "mention", "find"]) {
        "search"
    } else {
        "open_question"
    }
}

fn route(classifier: Option<&impl Classifier>, question: &str) -> (&'static str, String) {
    let asked = classifier.and_then(|c| {
        let replies = mirona_engine::block_on(c.ask(question, &[route_question()])).ok()?;
        match replies.into_iter().next()? {
            Reply::Choice { value, .. } => {
                let r = ROUTES.iter().find(|(k, _)| *k == value)?.0;
                Some((r, c.id().to_string()))
            }
            _ => None,
        }
    });
    asked.unwrap_or_else(|| (route_by_keywords(question), "keywords".into()))
}

/// A question's content words: keyword search wants every word to match, and "where did we
/// talk about pricing" should look for "pricing".
pub fn keywords(q: &str) -> String {
    const STOP: &[&str] = &[
        "a",
        "about",
        "an",
        "and",
        "any",
        "are",
        "at",
        "by",
        "did",
        "do",
        "does",
        "find",
        "for",
        "from",
        "had",
        "has",
        "have",
        "how",
        "i",
        "in",
        "is",
        "it",
        "last",
        "me",
        "mention",
        "mentioned",
        "my",
        "of",
        "on",
        "or",
        "our",
        "said",
        "say",
        "search",
        "show",
        "talk",
        "talked",
        "tell",
        "that",
        "the",
        "they",
        "this",
        "to",
        "us",
        "was",
        "we",
        "were",
        "what",
        "when",
        "where",
        "which",
        "who",
        "why",
        "with",
        "you",
    ];
    q.split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|w| !w.is_empty() && !STOP.contains(&w.to_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Search with the whole question (meaning likes it), then with its keywords if that found
/// nothing.
fn search_question(
    store: &Store,
    emb: Option<&impl Embed>,
    question: &str,
    filters: &SearchFilters,
) -> mirona_core::SearchResults {
    let r = search_with(store, emb, question, filters, MAX_MOMENTS);
    if r.moments.is_empty() {
        search_with(store, emb, &keywords(question), filters, MAX_MOMENTS)
    } else {
        r
    }
}

/// The meetings a question is about: the one named, else those its search hits come from
/// (best first), else the newest.
fn meetings_for(
    store: &Store,
    emb: Option<&impl Embed>,
    question: &str,
    meeting_id: Option<&str>,
    n: usize,
) -> Result<Vec<Meeting>, String> {
    if let Some(id) = meeting_id {
        return Ok(vec![store.meeting(id)?]);
    }
    let r = search_question(store, emb, question, &SearchFilters::default());
    let mut ids: Vec<String> = vec![];
    for m in r.moments.iter().filter(|m| !m.sensitive) {
        if !ids.contains(&m.meeting_id) {
            ids.push(m.meeting_id.clone());
        }
    }
    if ids.is_empty() {
        ids = store.list_meetings().into_iter().map(|m| m.id).collect();
    }
    ids.into_iter()
        .take(n)
        .map(|id| store.meeting(&id))
        .collect()
}

fn head(m: &Meeting) -> Value {
    json!({ "meeting_id": m.id, "title": m.title, "tags": m.tags, "date": m.date_label })
}

// ponytail: a search about one meeting narrows to its first tag (the filter takes one tag);
// take all of them if agents ask across a meeting's projects.
/// Answer `question`. `llm` is only built (and only called) for open questions.
pub fn ask<L: LlmProvider + mirona_engine::Labelled>(
    store: &Store,
    classifier: Option<&impl Classifier>,
    emb: Option<&impl Embed>,
    llm: impl FnOnce() -> Result<L, String>,
    question: &str,
    meeting_id: Option<&str>,
) -> Result<Value, String> {
    let (route, routed_by) = route(classifier, question);
    let mut out = json!({ "route": route, "routed_by": routed_by });
    match route {
        "search" => {
            let mut filters = SearchFilters::default();
            if let Some(id) = meeting_id {
                filters.meeting_tag = store.meeting(id)?.tags.into_iter().next();
            }
            let r = search_question(store, emb, question, &filters);
            let moments: Vec<Value> = r
                .moments
                .iter()
                .filter(|m| !m.sensitive && meeting_id.is_none_or(|id| m.meeting_id == id))
                .take(10)
                .map(|m| {
                    json!({ "meeting_id": m.meeting_id, "title": m.title, "date": m.date,
                            "at": at(m.start_ms), "speaker": m.speaker, "snippet": m.snippet })
                })
                .collect();
            out["moments"] = moments.into();
        }
        "decisions" => {
            let ms = meetings_for(store, emb, question, meeting_id, 3)?;
            out["meetings"] = ms
                .iter()
                .map(|m| {
                    let mut v = head(m);
                    v["decisions"] = m
                        .summary
                        .iter()
                        .flat_map(|s| &s.decisions)
                        .map(|d| json!({ "at": at(d.at_ms), "text": d.text }))
                        .collect();
                    v
                })
                .collect();
        }
        "action_items" => {
            let ids: Vec<String> = meeting_id.map(|i| vec![i.to_string()]).unwrap_or_default();
            let items: Vec<Value> = store
                .action_items()
                .into_iter()
                .filter(|a| (ids.is_empty() && !a.done) || ids.contains(&a.meeting_id))
                .take(MAX_MOMENTS)
                .map(|a| {
                    let m = store.meeting(&a.meeting_id).ok();
                    json!({ "text": crate::item_text(m.as_ref(), &a), "owner": a.owner,
                            "due": a.due, "done": a.done, "meeting": a.meeting_title,
                            "meeting_id": a.meeting_id, "needs_review": a.needs_review })
                })
                .collect();
            out["action_items"] = items.into();
            out["note"] = "Items with needs_review are unconfirmed guesses.".into();
        }
        "summary" => {
            let m = meetings_for(store, emb, question, meeting_id, 1)?;
            let m = m.first().ok_or("no meetings yet")?;
            out["meeting"] = crate::meeting(store, &m.id)?;
        }
        _ => {
            let ms = meetings_for(store, emb, question, meeting_id, 3)?;
            let mut context = String::new();
            for m in &ms {
                let tags = m.tags.join(", ");
                writeln!(context, "## {} · {tags} · {}", m.title, m.date_label).ok();
                if let Some(s) = &m.summary {
                    writeln!(context, "Summary: {}", s.overview).ok();
                }
                for s in m.segments.iter().filter(|s| !s.redacted && !sensitive(s)) {
                    let who = m
                        .speakers
                        .iter()
                        .find(|p| p.id == s.speaker_id)
                        .map_or(s.speaker_id.as_str(), |p| p.name.as_str());
                    let line = format!("[{}] {who}: {}\n", at(s.start_ms), s.text);
                    if context.len() + line.len() > CONTEXT_CHARS {
                        break;
                    }
                    context += &line;
                }
            }
            let llm = llm()?;
            let prompt = Prompt {
                system: "You answer questions about the user's meetings using only the \
                         transcript excerpts given. Cite the meeting and [m:ss] time for every \
                         claim. If the excerpts don't say, answer that the meetings don't say. \
                         Be brief.",
                user: &format!("{context}\nQuestion: {question}"),
                json: false,
            };
            let done = mirona_engine::block_on(llm.complete(&prompt, &mut |_| {}))
                .map_err(|e| e.to_string())?;
            out["answer"] = done.text.trim().into();
            out["written_by"] = llm.label().into();
            out["meetings"] = ms.iter().map(head).collect();
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirona_providers::fake::Fake;
    use mirona_providers::ollama::OllamaEmbedder;

    const NO_EMB: Option<&OllamaEmbedder> = None;

    /// Writes back what it was shown, so tests can see what reached the model.
    struct Spy(std::sync::Mutex<String>);
    impl mirona_providers::Provider for &Spy {
        fn id(&self) -> &'static str {
            "spy"
        }
        fn locality(&self) -> mirona_core::Locality {
            mirona_core::Locality::Local
        }
    }
    impl mirona_engine::Labelled for &Spy {
        fn label(&self) -> String {
            "spy".into()
        }
    }
    impl LlmProvider for &Spy {
        fn complete(
            &self,
            p: &Prompt<'_>,
            _: &mut (dyn FnMut(&str) + Send),
        ) -> impl Future<Output = mirona_providers::Result<mirona_providers::Completion>> + Send
        {
            p.user.clone_into(&mut self.0.lock().unwrap());
            std::future::ready(Ok(mirona_providers::Completion {
                text: "From the calls: it depends.".into(),
                usage: mirona_providers::Usage::default(),
            }))
        }
    }

    /// Fails if called: lookups must not reach the LLM.
    fn no_llm() -> Result<&'static Spy, String> {
        Err("the LLM was called for a lookup".into())
    }

    #[test]
    fn lookups_skip_the_llm_and_open_questions_use_it() {
        let store = Store::sample();
        let c = Fake("fake");
        // The fake classifier picks the option named in the question.
        let v = ask(
            &store,
            Some(&c),
            NO_EMB,
            no_llm,
            "which decisions were made",
            None,
        )
        .unwrap();
        assert_eq!(v["route"], "decisions");
        assert_eq!(v["routed_by"], "fake");
        assert!(v["meetings"][0]["decisions"].is_array(), "{v}");
        let v = ask(
            &store,
            Some(&c),
            NO_EMB,
            no_llm,
            "list the action_items",
            None,
        )
        .unwrap();
        assert!(!v["action_items"].as_array().unwrap().is_empty(), "{v}");
        let v = ask(
            &store,
            Some(&c),
            NO_EMB,
            no_llm,
            "summary of the size guide call",
            None,
        )
        .unwrap();
        assert!(v["meeting"]["overview"].is_string(), "{v}");
        let v = ask(
            &store,
            Some(&c),
            NO_EMB,
            no_llm,
            "search the onboarding copy",
            None,
        )
        .unwrap();
        assert!(!v["moments"].as_array().unwrap().is_empty(), "{v}");
        assert_eq!(
            keywords("Where did we talk about the size guide?"),
            "size guide"
        );

        let spy = Spy(std::sync::Mutex::new(String::new()));
        let v = ask(
            &store,
            Some(&c),
            NO_EMB,
            || Ok(&spy),
            "open_question: why did the migration worry them?",
            None,
        )
        .unwrap();
        assert_eq!(v["route"], "open_question");
        assert_eq!(v["answer"], "From the calls: it depends.");
        assert_eq!(v["written_by"], "spy");
        assert!(spy.0.lock().unwrap().contains("migration"));

        // No classifier: keywords route.
        let v = ask(
            &store,
            None::<&Fake>,
            NO_EMB,
            no_llm,
            "What did we agree on pricing?",
            None,
        )
        .unwrap();
        assert_eq!(v["route"], "decisions");
        assert_eq!(v["routed_by"], "keywords");
    }

    #[test]
    fn open_questions_never_see_sensitive_lines() {
        let store = Store::sample();
        let spy = Spy(std::sync::Mutex::new(String::new()));
        let id = store
            .list_meetings()
            .into_iter()
            .find(|m| {
                store
                    .meeting(&m.id)
                    .is_ok_and(|m| m.segments.iter().any(sensitive))
            })
            .unwrap()
            .id;
        ask(
            &store,
            None::<&Fake>,
            NO_EMB,
            || Ok(&spy),
            "how does staging work",
            Some(&id),
        )
        .unwrap();
        let seen = spy.0.lock().unwrap().clone();
        assert!(
            seen.contains("Question:") && !seen.contains("Alder-Field"),
            "{seen}"
        );
    }
}
