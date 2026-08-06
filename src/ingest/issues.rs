//! Write path: GitHub issues labelled `event` + `approved` become events.
//!
//! The Pages form pre-fills a GitHub issue form (`.github/ISSUE_TEMPLATE/event.yml`).
//! Once a maintainer adds the `approved` label, the build fetches the issue,
//! parses the structured body and persists it under `events/from-issues/` so the
//! event survives issue closure and API hiccups.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::ingest::curated::CuratedEvent;

#[derive(Debug, Deserialize)]
struct IssueLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Issue {
    number: u64,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    labels: Vec<IssueLabel>,
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

/// Fetch approved event issues from the repository (e.g. `thepriben/oedb-rs`).
pub fn fetch(repo: &str, token: Option<&str>, user_agent: &str) -> Result<Vec<CuratedEvent>> {
    let url = format!(
        "https://api.github.com/repos/{repo}/issues?labels=event,approved&state=all&per_page=100"
    );
    let mut request = ureq::get(&url)
        .set("User-Agent", user_agent)
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(30));
    if let Some(token) = token {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }

    let issues: Vec<Issue> = request
        .call()
        .with_context(|| format!("fetching issues from {repo}"))?
        .into_json()
        .context("decoding issues JSON")?;

    let mut events = Vec::new();
    for issue in issues {
        if issue.pull_request.is_some() {
            continue;
        }
        if !issue.labels.iter().any(|label| label.name == "approved") {
            continue;
        }
        let Some(body) = issue.body.as_deref() else { continue };
        match parse_issue_body(issue.number, body) {
            Some(event) => events.push(event),
            None => eprintln!("issue #{}: unparseable event body, skipped", issue.number),
        }
    }
    Ok(events)
}

/// Extract a Wikidata QID from a free-form value ("Q1962840", but also
/// "marché nocturne (Q1962840)" as produced by the autocomplete). Linking to
/// Wikidata is optional: `None` is perfectly fine.
fn extract_qid(value: &str) -> Option<String> {
    let start = value.find('Q')?;
    let digits: String = value[start + 1..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() || digits.starts_with('0') {
        return None;
    }
    Some(format!("Q{digits}"))
}

/// Parse the markdown rendered by the GitHub issue form: `### Heading` blocks
/// followed by the submitted value (`_No response_` when left empty).
pub fn parse_issue_body(number: u64, body: &str) -> Option<CuratedEvent> {
    let fields = split_form_fields(body);
    let get = |needle: &str| -> Option<String> {
        fields.iter().find_map(|(heading, value)| {
            let heading = heading.to_lowercase();
            // "Type" (scheduled/unscheduled) must not swallow "Type Wikidata".
            let matches = if needle == "type" {
                heading.contains("type") && !heading.contains("wikidata")
            } else {
                heading.contains(needle)
            };
            if matches && !value.is_empty() && value != "_No response_" {
                Some(value.clone())
            } else {
                None
            }
        })
    };

    let label = get("titre")?;
    let what = get("cat")?.split_whitespace().next()?.to_string();
    let kind = get("type")
        .map(|value| value.split_whitespace().next().unwrap_or("scheduled").to_string())
        .unwrap_or_else(|| "scheduled".to_string());
    let lat: f64 = get("latitude")?.replace(',', ".").parse().ok()?;
    let lon: f64 = get("longitude")?.replace(',', ".").parse().ok()?;

    let mut tags = BTreeMap::new();
    if let Some(description) = get("description") {
        tags.insert("description".to_string(), description);
    }
    tags.insert("issue".to_string(), format!("#{number}"));

    Some(CuratedEvent {
        id: format!("issue-{number}"),
        what,
        kind,
        label,
        start: get("début").or_else(|| get("debut")),
        stop: get("fin"),
        lon,
        lat,
        source: get("source"),
        created: None,
        wikidata: None,
        type_wikidata: get("type wikidata").as_deref().and_then(extract_qid),
        place_wikidata: get("lieu wikidata").as_deref().and_then(extract_qid),
        tags,
    })
}

fn split_form_fields(body: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_value = Vec::new();

    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            if let Some(previous) = current_heading.take() {
                fields.push((previous, current_value.join("\n").trim().to_string()));
            }
            current_heading = Some(heading.trim().to_string());
            current_value.clear();
        } else if current_heading.is_some() {
            current_value.push(line);
        }
    }
    if let Some(previous) = current_heading {
        fields.push((previous, current_value.join("\n").trim().to_string()));
    }
    fields
}

/// Persist issue events as YAML so they remain after the issue is closed.
pub fn persist(dir: &Path, events: &[CuratedEvent]) -> Result<bool> {
    if events.is_empty() {
        return Ok(false);
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    let mut changed = false;
    for event in events {
        let path = dir.join(format!("{}.yaml", event.id));
        let content = serde_yaml::to_string(&vec![event.clone()])
            .with_context(|| format!("serializing {}", event.id))?;
        let existing = fs::read_to_string(&path).ok();
        if existing.as_deref() != Some(content.as_str()) {
            fs::write(&path, &content).with_context(|| format!("writing {}", path.display()))?;
            changed = true;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    const BODY: &str = "### Titre de l'événement\n\nBrocante nocturne du centre-ville\n\n### Catégorie (what)\n\nculture.market.night\n\n### Type\n\nscheduled — programmé\n\n### Type Wikidata (QID, optionnel)\n\nmarché nocturne (Q1962840)\n\n### Lieu Wikidata (QID, optionnel)\n\nQ187796\n\n### Début (ISO 8601)\n\n2026-08-13T18:00:00+02:00\n\n### Fin (ISO 8601)\n\n2026-08-13T23:00:00+02:00\n\n### Latitude\n\n44.1362\n\n### Longitude\n\n4.8077\n\n### Source (URL)\n\nhttps://www.ville-orange.fr/article2431.html\n\n### Description\n\n_No response_";

    #[test]
    fn parses_issue_form_body() {
        let event = parse_issue_body(42, BODY).unwrap();
        assert_eq!(event.id, "issue-42");
        assert_eq!(event.what, "culture.market.night");
        assert_eq!(event.kind, "scheduled");
        assert_eq!(event.lat, 44.1362);
        assert_eq!(event.lon, 4.8077);
        assert_eq!(event.start.as_deref(), Some("2026-08-13T18:00:00+02:00"));
        assert_eq!(event.type_wikidata.as_deref(), Some("Q1962840"));
        assert_eq!(event.place_wikidata.as_deref(), Some("Q187796"));
        assert!(event.tags.get("description").is_none());

        // Round-trips through the shared curated pipeline.
        let converted = event.into_event(Utc::now()).unwrap();
        assert_eq!(converted.label, "Brocante nocturne du centre-ville");
        assert_eq!(converted.type_wikidata.as_deref(), Some("Q1962840"));
    }

    #[test]
    fn works_without_wikidata_fields() {
        let body = "### Titre de l'événement\n\nVide-grenier\n\n### Catégorie (what)\n\nculture.festival\n\n### Latitude\n\n44.05\n\n### Longitude\n\n5.05";
        let event = parse_issue_body(7, body).unwrap();
        assert!(event.type_wikidata.is_none());
        assert!(event.place_wikidata.is_none());
        assert!(event.into_event(Utc::now()).is_ok());
    }

    #[test]
    fn extracts_qids_tolerantly() {
        assert_eq!(extract_qid("Q42").as_deref(), Some("Q42"));
        assert_eq!(extract_qid("Orange (Q187796)").as_deref(), Some("Q187796"));
        assert_eq!(extract_qid("rien"), None);
        assert_eq!(extract_qid("Q0ينة"), None);
    }

    #[test]
    fn rejects_incomplete_body() {
        assert!(parse_issue_body(1, "### Titre de l'événement\n\nX").is_none());
    }
}
