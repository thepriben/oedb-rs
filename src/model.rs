//! OpenEventDatabase event model.
//!
//! Follows the schema of the OEDB backend (<https://github.com/openeventdatabase/backend>):
//! each event carries `what` (dotted taxonomy), `type` (scheduled/unscheduled),
//! `start`/`stop` (ISO 8601), `label`, `source`, `createdate`, `lastupdate`
//! and a Point geometry.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde::{Deserialize, Serialize};

/// Generous bounding box around Vaucluse (FR-84): lon_min, lat_min, lon_max, lat_max.
pub const VAUCLUSE_BBOX: (f64, f64, f64, f64) = (4.40, 43.55, 5.95, 44.55);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Scheduled,
    Unscheduled,
}

impl EventKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "scheduled" => Some(Self::Scheduled),
            "unscheduled" => Some(Self::Unscheduled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub id: String,
    pub what: String,
    pub kind: EventKind,
    pub label: String,
    pub start: Option<DateTime<FixedOffset>>,
    pub stop: Option<DateTime<FixedOffset>>,
    pub lon: f64,
    pub lat: f64,
    pub source: String,
    pub createdate: DateTime<Utc>,
    pub lastupdate: DateTime<Utc>,
    /// Wikidata QID of the event itself/series (e.g. a specific festival), when any.
    pub wikidata: Option<String>,
    /// Wikidata QID of the *kind* of event (e.g. Q1962840 « night market »).
    pub type_wikidata: Option<String>,
    /// Wikidata QID of the place (e.g. Q187796 « Orange »).
    pub place_wikidata: Option<String>,
    /// Free-form extra properties (road name, commune, description...).
    pub tags: BTreeMap<String, String>,
}

/// A Wikidata item id looks like `Q` followed by digits (no leading zero).
pub fn is_valid_qid(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('Q'))
        && matches!(chars.clone().next(), Some(c) if c.is_ascii_digit() && c != '0')
        && chars.all(|c| c.is_ascii_digit())
}

/// The OEDB taxonomy uses lowercase dotted paths (e.g. `traffic.accident`,
/// `culture.market.night`).
pub fn is_valid_what(what: &str) -> bool {
    !what.is_empty()
        && what.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        })
}

impl Event {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("empty id".into());
        }
        if !is_valid_what(&self.what) {
            return Err(format!("invalid taxonomy `what`: {}", self.what));
        }
        if self.label.trim().is_empty() {
            return Err(format!("{}: empty label", self.id));
        }
        let (lon_min, lat_min, lon_max, lat_max) = VAUCLUSE_BBOX;
        if self.lon < lon_min || self.lon > lon_max || self.lat < lat_min || self.lat > lat_max {
            return Err(format!(
                "{}: point ({}, {}) outside the FR-84 bounding box",
                self.id, self.lon, self.lat
            ));
        }
        if let (Some(start), Some(stop)) = (self.start, self.stop) {
            if stop < start {
                return Err(format!("{}: stop before start", self.id));
            }
        }
        for (field, value) in [
            ("wikidata", &self.wikidata),
            ("type_wikidata", &self.type_wikidata),
            ("place_wikidata", &self.place_wikidata),
        ] {
            if let Some(qid) = value {
                if !is_valid_qid(qid) {
                    return Err(format!("{}: invalid {field} QID `{qid}`", self.id));
                }
            }
        }
        Ok(())
    }

    /// An event is purged once its `stop` date is more than 24 h in the past.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        match self.stop {
            Some(stop) => stop.with_timezone(&Utc) + Duration::hours(24) < now,
            None => false,
        }
    }

    /// GeoJSON Feature carrying the OEDB properties.
    pub fn to_feature(&self) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        properties.insert("id".into(), self.id.clone().into());
        properties.insert("what".into(), self.what.clone().into());
        properties.insert(
            "type".into(),
            match self.kind {
                EventKind::Scheduled => "scheduled",
                EventKind::Unscheduled => "unscheduled",
            }
            .into(),
        );
        properties.insert("label".into(), self.label.clone().into());
        if let Some(start) = self.start {
            properties.insert("start".into(), start.to_rfc3339().into());
        }
        if let Some(stop) = self.stop {
            properties.insert("stop".into(), stop.to_rfc3339().into());
        }
        properties.insert("source".into(), self.source.clone().into());
        properties.insert("createdate".into(), self.createdate.to_rfc3339().into());
        properties.insert("lastupdate".into(), self.lastupdate.to_rfc3339().into());
        for (key, value) in [
            ("wikidata", &self.wikidata),
            ("type_wikidata", &self.type_wikidata),
            ("place_wikidata", &self.place_wikidata),
        ] {
            if let Some(qid) = value {
                properties.insert(key.into(), qid.clone().into());
            }
        }
        for (key, value) in &self.tags {
            properties
                .entry(key.clone())
                .or_insert_with(|| value.clone().into());
        }

        serde_json::json!({
            "type": "Feature",
            "id": self.id,
            "geometry": { "type": "Point", "coordinates": [self.lon, self.lat] },
            "properties": properties,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample() -> Event {
        Event {
            id: "test-1".into(),
            what: "traffic.accident".into(),
            kind: EventKind::Unscheduled,
            label: "Accident RD 950".into(),
            start: None,
            stop: None,
            lon: 4.81,
            lat: 44.14,
            source: "test".into(),
            createdate: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            lastupdate: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            wikidata: None,
            type_wikidata: None,
            place_wikidata: None,
            tags: BTreeMap::new(),
        }
    }

    #[test]
    fn accepts_valid_event() {
        assert!(sample().validate().is_ok());
    }

    #[test]
    fn validates_optional_wikidata_qids() {
        // Works entirely without Wikidata…
        assert!(sample().validate().is_ok());
        // …accepts well-formed QIDs…
        let mut event = sample();
        event.type_wikidata = Some("Q1962840".into());
        event.place_wikidata = Some("Q187796".into());
        assert!(event.validate().is_ok());
        let feature = event.to_feature();
        assert_eq!(feature["properties"]["type_wikidata"], "Q1962840");
        assert_eq!(feature["properties"]["place_wikidata"], "Q187796");
        // …and rejects malformed ones.
        event.wikidata = Some("42".into());
        assert!(event.validate().is_err());
        assert!(is_valid_qid("Q42"));
        assert!(!is_valid_qid("Q042"));
        assert!(!is_valid_qid("q42"));
        assert!(!is_valid_qid("Q"));
    }

    #[test]
    fn rejects_bad_taxonomy() {
        let mut event = sample();
        event.what = "Traffic.Accident".into();
        assert!(event.validate().is_err());
        assert!(is_valid_what("culture.market.night"));
        assert!(!is_valid_what("culture..night"));
        assert!(!is_valid_what(""));
    }

    #[test]
    fn rejects_point_outside_vaucluse() {
        let mut event = sample();
        event.lat = 48.85; // Paris
        assert!(event.validate().is_err());
    }

    #[test]
    fn expires_after_stop_plus_grace() {
        let mut event = sample();
        let now = Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap();
        assert!(!event.is_expired(now));
        event.stop = Some(
            DateTime::parse_from_rfc3339("2026-08-06T23:00:00+02:00").unwrap(),
        );
        assert!(event.is_expired(now));
        event.stop = Some(
            DateTime::parse_from_rfc3339("2026-08-10T09:00:00+02:00").unwrap(),
        );
        assert!(!event.is_expired(now));
    }

    #[test]
    fn feature_carries_oedb_properties() {
        let feature = sample().to_feature();
        assert_eq!(feature["properties"]["what"], "traffic.accident");
        assert_eq!(feature["properties"]["type"], "unscheduled");
        assert_eq!(feature["geometry"]["coordinates"][0], 4.81);
    }
}
