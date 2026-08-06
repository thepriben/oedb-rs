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

/// Radius (in degrees of latitude, ~25 m) of the circle used to spread events
/// that share the exact same point.
const SPREAD_RADIUS_DEG: f64 = 0.00025;

/// Events sharing the exact same coordinates (e.g. the four Jeudis d'Orange
/// evenings on the town square) are spread on a small deterministic circle so
/// each occurrence gets its own position. Ordering by id keeps positions
/// stable from build to build; map-side clustering regroups them at low zoom.
pub fn spread_shared_locations(events: &mut [Event]) {
    let mut groups: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (index, event) in events.iter().enumerate() {
        let key = (
            (event.lon * 1e5).round() as i64,
            (event.lat * 1e5).round() as i64,
        );
        groups.entry(key).or_default().push(index);
    }

    for indices in groups.values() {
        if indices.len() < 2 {
            continue;
        }
        let mut ordered = indices.clone();
        ordered.sort_by(|&a, &b| events[a].id.cmp(&events[b].id));
        let count = ordered.len() as f64;
        for (slot, &index) in ordered.iter().enumerate() {
            let angle =
                std::f64::consts::TAU * (slot as f64) / count - std::f64::consts::FRAC_PI_2;
            // Compensate longitude by cos(lat) so the circle stays round.
            let cos_lat = events[index].lat.to_radians().cos().max(0.1);
            events[index].lon += SPREAD_RADIUS_DEG * angle.cos() / cos_lat;
            events[index].lat += SPREAD_RADIUS_DEG * angle.sin();
        }
    }
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
            wikidata: None,
            type_wikidata: None,
            place_wikidata: None,
            tags: Map::new(),
        }
    }

    #[test]
    fn spreads_events_sharing_a_point() {
        let mut events = vec![event("j1", None), event("j2", None), event("j3", None), event("j4", None)];
        let lone = event("seul", None);
        events.push(Event { lon: 5.05, lat: 44.05, ..lone });

        spread_shared_locations(&mut events);

        // The lone event keeps its exact position.
        assert_eq!(events[4].lon, 5.05);
        assert_eq!(events[4].lat, 44.05);

        // The four co-located events all get distinct positions…
        let mut positions: Vec<(String, String)> = events[..4]
            .iter()
            .map(|e| (format!("{:.7}", e.lon), format!("{:.7}", e.lat)))
            .collect();
        positions.sort();
        positions.dedup();
        assert_eq!(positions.len(), 4);

        // …still within ~30 m of the original point.
        for e in &events[..4] {
            assert!((e.lon - 4.81).abs() < 0.0005, "{} lon {}", e.id, e.lon);
            assert!((e.lat - 44.14).abs() < 0.0005, "{} lat {}", e.id, e.lat);
        }

        // Deterministic: re-running from the same input gives the same result.
        let mut again = vec![event("j1", None), event("j2", None), event("j3", None), event("j4", None)];
        spread_shared_locations(&mut again);
        for (a, b) in events[..4].iter().zip(again.iter()) {
            assert_eq!(a.lon, b.lon);
            assert_eq!(a.lat, b.lat);
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
