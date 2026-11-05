//! Meeting store (NGR-140). The prototype keeps everything in memory, seeded with sample data.
//! ponytail: in-memory only; swap for Markdown files + `SQLite` index in NGR-140.

pub mod sample;

use mirona_core::{
    ActionItem, AgentCall, AnswerSource, Choice, Correction, EnhancedNote, Id, Locality, McpStatus,
    Meeting, MeetingSummary, Note, Segment, Speaker, Stage, Summary, Tag, TagKind,
};
use mirona_providers::check_privacy;
use mirona_providers::config::Config;

pub type Result<T> = std::result::Result<T, String>;

pub struct Store {
    pub meetings: Vec<Meeting>,
    pub people: Vec<Speaker>,
    pub action_items: Vec<ActionItem>,
    pub stages: Vec<Stage>,
    pub mcp: McpStatus,
    pub privacy: bool,
    pub corrections: Vec<Correction>,
}

impl Store {
    pub fn sample() -> Self {
        let mut s = Self {
            meetings: sample::meetings(),
            people: sample::people(),
            action_items: sample::action_items(),
            stages: sample::stages(),
            mcp: sample::mcp(),
            privacy: false,
            corrections: vec![],
        };
        s.set_privacy(true);
        s
    }

    /// Apply a saved config. Every choice goes through the privacy gate, so a config that
    /// picks a cloud provider while privacy is on is rejected, not silently obeyed.
    pub fn apply_config(&mut self, cfg: &Config) -> Result<()> {
        self.set_privacy(cfg.privacy);
        for (stage, provider) in &cfg.providers {
            self.select_provider(stage, provider)
                .map_err(|e| format!("config: {stage} = {provider}: {e}"))?;
        }
        Ok(())
    }

    pub fn config(&self) -> Config {
        Config {
            privacy: self.privacy,
            providers: self
                .stages
                .iter()
                .map(|s| (s.id.clone(), s.selected.clone()))
                .collect(),
        }
    }

    pub fn list_meetings(&self) -> Vec<MeetingSummary> {
        self.meetings
            .iter()
            .map(|m| MeetingSummary {
                id: m.id.clone(),
                title: m.title.clone(),
                client: m.client.clone(),
                when: m.when.clone(),
            })
            .collect()
    }

    fn meeting_mut(&mut self, id: &str) -> Result<&mut Meeting> {
        self.meetings
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or(format!("no meeting {id}"))
    }

    pub fn meeting(&self, id: &str) -> Result<Meeting> {
        self.meetings
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or(format!("no meeting {id}"))
    }

    /// Keyword search over titles, clients and transcript text.
    pub fn search(&self, query: &str) -> Vec<MeetingSummary> {
        let q = query.to_lowercase();
        let ids: Vec<&str> = self
            .meetings
            .iter()
            .filter(|m| {
                m.title.to_lowercase().contains(&q)
                    || m.client.to_lowercase().contains(&q)
                    || m.segments
                        .iter()
                        .any(|s| s.text.to_lowercase().contains(&q))
            })
            .map(|m| m.id.as_str())
            .collect();
        self.list_meetings()
            .into_iter()
            .filter(|m| ids.contains(&m.id.as_str()))
            .collect()
    }

    pub fn toggle_action_item(&mut self, id: &str) -> Result<ActionItem> {
        let item = self
            .action_items
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or("no action item")?;
        item.done = !item.done;
        Ok(item.clone())
    }

    /// Fix a model answer. The fix is kept as a labelled example and retags the segment.
    pub fn correct(&mut self, c: Correction) -> Result<Segment> {
        let meeting = self
            .meetings
            .iter_mut()
            .find(|m| m.segments.iter().any(|s| s.id == c.segment_id))
            .ok_or("no segment")?;
        let seg = meeting
            .segments
            .iter_mut()
            .find(|s| s.id == c.segment_id)
            .unwrap();
        let ans = seg
            .answers
            .iter_mut()
            .find(|a| a.id == c.answer_id)
            .ok_or("no answer")?;
        ans.answer = c.value;
        ans.probability = if c.value { 1.0 } else { 0.0 };
        ans.source = AnswerSource::User;
        let kind = match c.answer_id.as_str() {
            "action" => TagKind::Action,
            "decision" => TagKind::Decision,
            "unanswered" => TagKind::OpenQuestion,
            "smalltalk" => TagKind::SmallTalk,
            _ => TagKind::Sensitive,
        };
        seg.tags
            .retain(|t| t.kind != kind && t.kind != TagKind::Review);
        if c.value {
            let label = match kind {
                TagKind::Action => "Action · confirmed",
                TagKind::Decision => "Decision",
                TagKind::OpenQuestion => "Open question",
                TagKind::SmallTalk => "Small talk",
                _ => "Sensitive",
            };
            seg.tags.push(Tag {
                kind,
                label: label.into(),
            });
        }
        let seg = seg.clone();
        if c.answer_id == "action"
            && let Some(a) = self
                .action_items
                .iter_mut()
                .find(|a| a.at_ms == Some(seg.start_ms) && a.meeting_id == meeting.id)
        {
            a.needs_review = false;
        }
        self.corrections.push(c);
        Ok(seg)
    }

    pub fn confirm_client(&mut self, meeting_id: &str, client: &str) -> Result<Meeting> {
        let m = self.meeting_mut(meeting_id)?;
        if let Some(a) = m.answers.as_mut() {
            a.client.value = client.into();
            a.client.confirmed = true;
            a.client.probability = 1.0;
        }
        m.client = client.into();
        Ok(m.clone())
    }

    /// Name an unknown voice everywhere it appears.
    pub fn enroll_speaker(&mut self, speaker_id: &str, name: &str) -> Result<Speaker> {
        let rename = |s: &mut Speaker| {
            s.name = name.into();
            s.initial = name.chars().next().unwrap_or('?').to_string();
            s.subtitle = "Enrolled from Claims Desk weekly sync".into();
            s.suggestion = None;
            s.voiceprint = Some(mirona_core::Voiceprint { meetings: 1 });
        };
        let person = self
            .people
            .iter_mut()
            .find(|p| p.id == speaker_id)
            .ok_or("no speaker")?;
        let old = person.name.clone();
        rename(person);
        let person = person.clone();
        for m in &mut self.meetings {
            m.speakers
                .iter_mut()
                .filter(|s| s.id == speaker_id)
                .for_each(rename);
            for seg in &mut m.segments {
                for t in &mut seg.tags {
                    t.label = t.label.replace(&old, name);
                }
                for o in &mut seg.owners {
                    if o.label == old {
                        o.label = name.into();
                    }
                }
            }
        }
        for a in self.action_items.iter_mut().filter(|a| a.owner == old) {
            a.owner = name.into();
        }
        Ok(person)
    }

    /// Turning privacy on moves every stage back to a local provider.
    pub fn set_privacy(&mut self, on: bool) -> Vec<Stage> {
        self.privacy = on;
        if on {
            for st in &mut self.stages {
                let cloud = st
                    .options
                    .iter()
                    .any(|o| o.id == st.selected && o.locality == Locality::Cloud);
                if cloud {
                    st.selected = st
                        .options
                        .iter()
                        .find(|o| o.locality == Locality::Local)
                        .unwrap()
                        .id
                        .clone();
                }
            }
        }
        self.stages.clone()
    }

    pub fn select_provider(&mut self, stage_id: &str, option_id: &str) -> Result<Stage> {
        let privacy = self.privacy;
        let st = self
            .stages
            .iter_mut()
            .find(|s| s.id == stage_id)
            .ok_or("no stage")?;
        let opt = st
            .options
            .iter()
            .find(|o| o.id == option_id)
            .ok_or("no provider")?;
        check_privacy(opt, privacy).map_err(|e| e.to_string())?;
        st.selected = option_id.into();
        Ok(st.clone())
    }

    pub fn llm_name(&self) -> String {
        let st = self.stages.iter().find(|s| s.id == "llm").unwrap();
        st.options
            .iter()
            .find(|o| o.id == st.selected)
            .unwrap()
            .name
            .clone()
    }

    pub fn log_agent_call(&mut self, call: AgentCall) {
        self.mcp.log.insert(0, call);
        self.mcp.log.truncate(6);
    }

    // --- Live recording (fake capture pipeline) ---

    pub fn start_meeting(&mut self, id: &str) -> Meeting {
        let m = Meeting {
            id: id.into(),
            title: "New meeting".into(),
            client: "Unsorted".into(),
            when: "now".into(),
            date_label: "Thu 5 Nov 2026 · now".into(),
            duration_min: 0,
            language: "English".into(),
            sample: true,
            speakers: vec![],
            segments: vec![],
            answers: None,
            notes: Note {
                raw: vec![],
                enhanced: vec![],
                enhanced_by: String::new(),
            },
            summary: None,
        };
        self.meetings.insert(0, m.clone());
        m
    }

    pub fn push_segment(&mut self, meeting_id: &str, seg: Segment) -> Result<Speaker> {
        let speaker = self
            .people
            .iter()
            .find(|p| p.id == seg.speaker_id)
            .cloned()
            .ok_or("no speaker")?;
        let m = self.meeting_mut(meeting_id)?;
        if !m.speakers.iter().any(|s| s.id == speaker.id) {
            m.speakers.push(speaker.clone());
        }
        m.segments.push(seg);
        Ok(speaker)
    }

    pub fn set_notes(&mut self, meeting_id: &str, raw: Vec<String>) -> Result<()> {
        self.meeting_mut(meeting_id)?.notes.raw = raw;
        Ok(())
    }

    /// Stop-and-process: name the meeting, answer meeting-level questions, write summary and notes.
    pub fn finish_meeting(&mut self, meeting_id: &str, elapsed_ms: u32) -> Result<Meeting> {
        let llm = self.llm_name();
        let m = self.meeting_mut(meeting_id)?;
        let id = m.id.clone();
        m.title = "Evaluation dashboard follow-up".into();
        m.client = "Lumenix".into();
        m.when = "just now".into();
        m.duration_min = (elapsed_ms / 60_000).max(1);
        let tagged = |k: TagKind| {
            m.segments
                .iter()
                .filter(move |s| s.tags.iter().any(|t| t.kind == k))
        };
        let enhanced: Vec<EnhancedNote> = m
            .segments
            .iter()
            .filter_map(|s| {
                s.tags.first().map(|t| EnhancedNote {
                    kind: t.kind,
                    text: s.text.clone(),
                    at_ms: s.start_ms,
                })
            })
            .collect();
        let decisions = tagged(TagKind::Decision)
            .map(|s| mirona_core::Decision {
                text: s.text.clone(),
                at_ms: s.start_ms,
            })
            .collect();
        let open = tagged(TagKind::OpenQuestion)
            .map(|s| mirona_core::Decision {
                text: s.text.clone(),
                at_ms: s.start_ms,
            })
            .collect();
        let actions: Vec<Segment> = tagged(TagKind::Action).cloned().collect();
        m.notes.enhanced = enhanced;
        m.notes.enhanced_by.clone_from(&llm);
        m.summary = Some(Summary {
            template: "Client call".into(),
            overview: "Follow-up on the Lumenix evaluation dashboard. The board demo is set; one ownership question is open.".into(),
            decisions,
            open_questions: open,
            written_by: llm,
            quality: format!("Quality check: the summary covers {0} of {0} confirmed action items (score 1.0)", actions.len()),
        });
        let mut ans = sample::meetings()[0].answers.clone().unwrap();
        ans.client = sample::meetings()[3].answers.clone().unwrap().client;
        ans.priority = 3.2;
        ans.follow_up_draft = "Hi Keiko,\n\nThanks for the call. The evaluation dashboard demo is set for the board on the 20th, and I'll send a short demo script by Tuesday.\n\nStill open: who owns the data export for legal.\n\nAryan".into();
        m.answers = Some(ans);
        let title = m.title.clone();
        for (i, s) in actions.iter().enumerate() {
            self.action_items.insert(
                0,
                ActionItem {
                    id: format!("{id}-a{i}"),
                    meeting_id: id.clone(),
                    meeting_title: title.clone(),
                    at_ms: Some(s.start_ms),
                    text: "Prepare a short demo script".into(),
                    owner: "Aryan".into(),
                    due: Some("Tue".into()),
                    client: "Lumenix".into(),
                    done: false,
                    needs_review: false,
                },
            );
        }
        let m = self.meeting(&id)?;
        Ok(m)
    }
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
mod tests {
    use super::*;

    #[test]
    fn privacy_and_corrections() {
        let mut s = Store::sample();
        assert!(
            s.select_provider("llm", "anthropic").is_err(),
            "cloud blocked in privacy mode"
        );
        s.set_privacy(false);
        s.select_provider("llm", "anthropic").unwrap();
        s.set_privacy(true);
        assert_eq!(s.stages[3].selected, "ollama", "privacy on resets to local");

        let seg = s
            .correct(Correction {
                segment_id: "c9".into(),
                answer_id: "action".into(),
                value: true,
            })
            .unwrap();
        assert!(seg.tags.iter().any(|t| t.kind == TagKind::Action));
        assert!(!seg.tags.iter().any(|t| t.kind == TagKind::Review));
        assert!(!s.action_items[2].needs_review);

        s.enroll_speaker("s3", "Inés").unwrap();
        assert_eq!(s.action_items[0].owner, "Inés");
        assert_eq!(s.search("insurer").len(), 1);
    }

    #[test]
    fn config_picks_providers_through_the_privacy_gate() {
        let mut s = Store::sample();
        let sel = |s: &Store, id: &str| {
            s.stages
                .iter()
                .find(|x| x.id == id)
                .unwrap()
                .selected
                .clone()
        };
        assert_eq!(
            sel(&s, "classification"),
            "local-llm",
            "privacy on by default"
        );

        let mut cfg = Config::default();
        cfg.providers.insert("classification".into(), "jev".into());
        assert!(
            s.apply_config(&cfg).is_err(),
            "cloud refused while privacy is on"
        );

        cfg.privacy = false;
        s.apply_config(&cfg).unwrap();
        assert_eq!(sel(&s, "classification"), "jev");
        assert_eq!(s.config().providers["classification"], "jev");

        cfg.providers.insert("stt".into(), "nope".into());
        assert!(s.apply_config(&cfg).is_err(), "unknown provider");
    }
}
