//! Static `/api` tree generation (OEDB-compatible, read-only).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::model::{Event, EventKind};

pub struct EmitSummary {
    pub total: usize,
    pub by_what: BTreeMap<String, usize>,
}

/// Deduplicate by id (later sources win), drop expired events, sort by id.
pub fn consolidate(events: Vec<Event>, now: DateTime<Utc>) -> Vec<Event> {
    let mut by_id: BTreeMap<String, Event> = BTreeMap::new();
    for event in events {
        if event.is_expired(now) {
            continue;
        }
        by_id.insert(event.id.clone(), event);
    }
    by_id.into_values().collect()
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let content = serde_json::to_string(value)?;
    fs::write(path, content + "\n").with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Write the static API tree under `out`:
/// - `api/event.json` (+ aliases `api/events.json`, `api/vaucluse.geojson`)
/// - `api/event/{id}.json`
/// - `api/stats.json`
pub fn emit(out: &Path, events: &[Event], now: DateTime<Utc>) -> Result<EmitSummary> {
    let api = out.join("api");
    let event_dir = api.join("event");
    if event_dir.exists() {
        fs::remove_dir_all(&event_dir).context("clearing api/event")?;
    }
    fs::create_dir_all(&event_dir).context("creating api/event")?;

    let features: Vec<serde_json::Value> = events.iter().map(Event::to_feature).collect();
    let collection = serde_json::json!({
        "type": "FeatureCollection",
        "features": features,
        "_cache": {
            "generated_at": now.to_rfc3339(),
            "source_name": "oedb-rs (instance OEDB statique FR-84)",
            "source_url": "https://github.com/thepriben/oedb-rs",
            "count": events.len(),
        },
    });

    write_json(&api.join("event.json"), &collection)?;
    write_json(&api.join("events.json"), &collection)?;
    write_json(&api.join("vaucluse.geojson"), &collection)?;

    for event in events {
        write_json(&event_dir.join(format!("{}.json", event.id)), &event.to_feature())?;
    }

    let mut by_what: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    for event in events {
        *by_what.entry(event.what.clone()).or_insert(0) += 1;
        let kind = match event.kind {
            EventKind::Scheduled => "scheduled",
            EventKind::Unscheduled => "unscheduled",
        };
        *by_type.entry(kind.to_string()).or_insert(0) += 1;
    }
    let stats = serde_json::json!({
        "generated_at": now.to_rfc3339(),
        "total": events.len(),
        "by_what": by_what,
        "by_type": by_type,
    });
    write_json(&api.join("stats.json"), &stats)?;

    // GitHub Pages: skip Jekyll processing.
    fs::write(out.join(".nojekyll"), "").context("writing .nojekyll")?;

    Ok(EmitSummary {
        total: events.len(),
        by_what,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EventKind;
    use chrono::TimeZone;
    use std::collections::BTreeMap as Map;

    fn event(id: &str, stop: Option<&str>) -> Event {
        Event {
            id: id.into(),
            what: "traffic.accident".into(),
            kind: EventKind::Unscheduled,
            label: "Test".into(),
            start: None,
            stop: stop.map(|s| DateTime::parse_from_rfc3339(s).unwrap()),
            lon: 4.81,
            lat: 44.14,
            source: "test".into(),
            createdate: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            lastupdate: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            tags: Map::new(),
        }
    }

    #[test]
    fn consolidates_dedupes_and_purges() {
        let now = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
        let events = vec![
            event("a", None),
            event("a", None),
            event("old", Some("2026-08-01T00:00:00+00:00")),
        ];
        let kept = consolidate(events, now);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "a");
    }

    #[test]
    fn emits_api_tree() {
        let tmp = std::env::temp_dir().join(format!("oedb-emit-test-{}", std::process::id()));
        let now = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
        let events = vec![event("a", None), event("b", None)];
        let summary = emit(&tmp, &events, now).unwrap();
        assert_eq!(summary.total, 2);

        let collection: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(tmp.join("api/event.json")).unwrap()).unwrap();
        assert_eq!(collection["features"].as_array().unwrap().len(), 2);
        assert!(tmp.join("api/event/a.json").exists());
        assert!(tmp.join("api/stats.json").exists());
        assert!(tmp.join(".nojekyll").exists());

        fs::remove_dir_all(&tmp).unwrap();
    }
}
