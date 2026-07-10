use chrono::Utc;

use crate::models::{ActionItem, Meeting, MeetingSummary};

/// Produces a summary and action items from a finished meeting. Swap this for a
/// cloud LLM or a local model without touching the recorder.
pub trait Summarizer: Send + Sync {
    fn summarize(&self, meeting: &Meeting) -> MeetingSummary;
    fn action_items(&self, meeting: &Meeting) -> Vec<ActionItem>;
}

/// A lightweight, dependency-free extractive summarizer. It selects salient
/// lines and detects decisions/commitments with simple heuristics — good enough
/// to make the pipeline useful offline, and a clean seam for a real model.
pub struct HeuristicSummarizer;

impl Summarizer for HeuristicSummarizer {
    fn summarize(&self, meeting: &Meeting) -> MeetingSummary {
        let sentences: Vec<&str> = meeting
            .transcript
            .iter()
            .map(|s| s.text.as_str())
            .collect();

        let tldr = if sentences.is_empty() {
            "No speech was captured for this session.".to_string()
        } else {
            let lead = sentences
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                "Discussion covering {}. {}",
                meeting.title.to_lowercase(),
                lead
            )
        };

        let key_points = sentences
            .iter()
            .take(4)
            .map(|s| s.trim_end_matches('.').to_string())
            .collect();

        let decisions = sentences
            .iter()
            .filter(|s| {
                let l = s.to_lowercase();
                ["decid", "agree", "commit", "let's", "we'll"]
                    .iter()
                    .any(|kw| l.contains(kw))
            })
            .take(3)
            .map(|s| s.trim_end_matches('.').to_string())
            .collect();

        MeetingSummary {
            tldr,
            key_points,
            decisions,
            generated_at: Utc::now(),
            model: "local · summarizer-v1".to_string(),
        }
    }

    fn action_items(&self, meeting: &Meeting) -> Vec<ActionItem> {
        meeting
            .transcript
            .iter()
            .filter(|s| {
                let l = s.text.to_lowercase();
                ["i'll", "we'll", "follow up", "need to", "will ", "action"]
                    .iter()
                    .any(|kw| l.contains(kw))
            })
            .take(5)
            .map(|s| ActionItem {
                id: format!("a_{}", uuid::Uuid::new_v4().simple()),
                text: s.text.clone(),
                done: false,
                assignee: Some(s.speaker.clone()),
                due_date: None,
            })
            .collect()
    }
}
