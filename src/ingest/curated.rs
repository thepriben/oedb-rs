//! Curated events from `events/curated/*.yaml` and `events/from-issues/*.yaml`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Event, EventKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratedEvent {
    pub id: String,
    pub what: String,
    #[serde(rename = "type", default = "default_kind")]
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    pub lon: f64,
    pub lat: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// Optional Wikidata QID of the event/series itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wikidata: Option<String>,
    /// Optional Wikidata QID of the event-type concept (e.g. Q1962840 night market).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_wikidata: Option<String>,
    /// Optional Wikidata QID of the place (e.g. Q187796 Orange).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_wikidata: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
}

fn default_kind() -> String {
    "scheduled".to_string()
}

impl CuratedEvent {
    pub fn into_event(self, now: DateTime<Utc>) -> Result<Event> {
        let kind = EventKind::parse(&self.kind)
            .with_context(|| format!("{}: invalid type `{}`", self.id, self.kind))?;
        let start = self
            .start
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .with_context(|| format!("{}: invalid start date", self.id))?;
        let stop = self
            .stop
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .with_context(|| format!("{}: invalid stop date", self.id))?;
        let created = self
            .created
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .with_context(|| format!("{}: invalid created date", self.id))?
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| start.map(|dt| dt.with_timezone(&Utc)))
            .unwrap_or(now);

        let event = Event {
            id: self.id,
            what: self.what,
            kind,
            label: self.label,
            start,
            stop,
            lon: self.lon,
            lat: self.lat,
            source: self.source.unwrap_or_else(|| "curation oedb-rs".to_string()),
            createdate: created,
            lastupdate: created,
            wikidata: self.wikidata,
            type_wikidata: self.type_wikidata,
            place_wikidata: self.place_wikidata,
            tags: self.tags,
        };
        event.validate().map_err(anyhow::Error::msg)?;
        Ok(event)
    }
}

/// Parse one YAML document containing a list of curated events.
pub fn parse_yaml(content: &str, now: DateTime<Utc>) -> Result<Vec<Event>> {
    let entries: Vec<CuratedEvent> =
        serde_yaml::from_str(content).context("parsing curated YAML")?;
    entries
        .into_iter()
        .map(|entry| entry.into_event(now))
        .collect()
}

/// Load every `*.yaml` / `*.yml` file of a directory. Missing directory is fine.
pub fn load_dir(dir: &Path, now: DateTime<Utc>) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    if !dir.is_dir() {
        return Ok(events);
    }

    let mut paths: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yaml") | Some("yml")
            )
        })
        .collect();
    paths.sort();

    for path in paths {
        let content =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let parsed =
            parse_yaml(&content, now).with_context(|| format!("in {}", path.display()))?;
        events.extend(parsed);
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
- id: jeudis-orange-2026-08-06
  what: culture.market.night
  type: scheduled
  label: "Jeudis d'Orange - marché nocturne"
  start: 2026-08-06T18:00:00+02:00
  stop: 2026-08-06T23:30:00+02:00
  lon: 4.8077
  lat: 44.1362
  source: https://www.ville-orange.fr/article2431.html
  tags:
    commune: Orange
"#;

    #[test]
    fn parses_curated_yaml() {
        let events = parse_yaml(YAML, Utc::now()).unwrap();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.what, "culture.market.night");
        assert_eq!(event.kind, EventKind::Scheduled);
        assert_eq!(event.tags.get("commune").unwrap(), "Orange");
        // createdate falls back to the start date.
        assert_eq!(event.createdate.to_rfc3339(), "2026-08-06T16:00:00+00:00");
    }

    #[test]
    fn rejects_invalid_entries() {
        let bad = YAML.replace("44.1362", "48.85");
        assert!(parse_yaml(&bad, Utc::now()).is_err());
    }
}
