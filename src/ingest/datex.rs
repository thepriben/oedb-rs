//! Bison Futé / DIR DATEX II ingestion.
//!
//! Same feed as dataroads-FR84 (`tipi.bison-fute.gouv.fr`, Evenementiel-DIR RRN),
//! filtered on the Vaucluse bounding box and mapped to the OEDB `traffic.*`
//! taxonomy.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use roxmltree::Node;

use crate::boundary;
use crate::model::{Event, EventKind};

pub const DEFAULT_FEED_URL: &str =
    "http://tipi.bison-fute.gouv.fr/bison-fute-ouvert/publicationsDIR/Evenementiel-DIR/grt/RRN/content.xml";

const DATEX_NS: &str = "http://datex2.eu/schema/2/2_0";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// DATEX II situationRecord xsi:type -> OEDB `what` taxonomy.
fn what_for_type(record_type: &str) -> &'static str {
    match record_type {
        "Accident" => "traffic.accident",
        "ConstructionWorks" | "MaintenanceWorks" | "RoadOrCarriagewayOrLaneManagement"
        | "SpeedManagement" | "ReroutingManagement" => "traffic.roadwork",
        "AbnormalTraffic" => "traffic.jam",
        "WeatherRelatedRoadConditions" => "weather.road",
        "InfrastructureDamageObstruction" | "EnvironmentalObstruction" | "VehicleObstruction"
        | "GeneralObstruction" | "AnimalPresenceObstruction" => "traffic.hazard",
        _ => "traffic.info",
    }
}

pub fn fetch(url: &str, user_agent: &str) -> Result<Vec<Event>> {
    let body = ureq::get(url)
        .set("User-Agent", user_agent)
        .set("Accept", "application/xml, text/xml, */*;q=0.1")
        .timeout(std::time::Duration::from_secs(90))
        .call()
        .with_context(|| format!("fetching DATEX II feed {url}"))?
        .into_string()
        .context("reading DATEX II response body")?;
    parse(&body)
}

/// Parse a DATEX II payload into OEDB events located inside Vaucluse.
pub fn parse(xml: &str) -> Result<Vec<Event>> {
    let doc = roxmltree::Document::parse(xml).context("parsing DATEX II XML")?;
    let now = Utc::now();
    let mut events = Vec::new();

    for record in doc
        .descendants()
        .filter(|node| node.has_tag_name((DATEX_NS, "situationRecord")))
    {
        if let Some(event) = record_to_event(record, now) {
            if event.validate().is_ok() {
                events.push(event);
            }
        }
    }

    Ok(events)
}

fn local_xsi_type(record: Node) -> String {
    let raw = record.attribute((XSI_NS, "type")).unwrap_or("");
    raw.rsplit(':').next().unwrap_or("").to_string()
}

fn text_of(node: Option<Node>) -> String {
    node.and_then(|n| n.text()).unwrap_or("").trim().to_string()
}

fn find_child<'a>(parent: Node<'a, 'a>, tag: &str) -> Option<Node<'a, 'a>> {
    parent
        .children()
        .find(|child| child.has_tag_name((DATEX_NS, tag)))
}

fn find_descendant<'a>(parent: Node<'a, 'a>, tag: &str) -> Option<Node<'a, 'a>> {
    parent
        .descendants()
        .find(|node| node.has_tag_name((DATEX_NS, tag)))
}

fn nested_text(parent: Node, path: &[&str]) -> String {
    let mut current = parent;
    for tag in path {
        match find_child(current, tag) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    text_of(Some(current))
}

/// First `<pointCoordinates>` as (lon, lat).
fn first_point(record: Node) -> Option<(f64, f64)> {
    let coords = find_descendant(record, "pointCoordinates")?;
    let lat: f64 = nested_text(coords, &["latitude"]).parse().ok()?;
    let lon: f64 = nested_text(coords, &["longitude"]).parse().ok()?;
    Some((lon, lat))
}

/// `<generalPublicComment>` values grouped by commentType.
fn comments_by_type(record: Node) -> BTreeMap<String, Vec<String>> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for comment in record
        .children()
        .filter(|node| node.has_tag_name((DATEX_NS, "generalPublicComment")))
    {
        let comment_type = {
            let value = nested_text(comment, &["commentType"]);
            if value.is_empty() { "other".to_string() } else { value }
        };
        let text = nested_text(comment, &["comment", "values", "value"]);
        if !text.is_empty() {
            grouped.entry(comment_type).or_default().push(text);
        }
    }
    grouped
}

/// Road (linkName) and town (townName) from TPEG point descriptors.
fn road_and_town(record: Node) -> (String, String) {
    let mut road = String::new();
    let mut town = String::new();
    for name in record
        .descendants()
        .filter(|node| node.has_tag_name((DATEX_NS, "name")))
    {
        let descriptor_type = nested_text(name, &["tpegOtherPointDescriptorType"]);
        let value = nested_text(name, &["descriptor", "values", "value"]);
        if value.is_empty() {
            continue;
        }
        if descriptor_type == "linkName" && road.is_empty() {
            road = value;
        } else if descriptor_type == "townName" && town.is_empty() {
            town = value;
        }
    }
    (road, town)
}

fn parse_datex_time(value: &str) -> Option<DateTime<chrono::FixedOffset>> {
    if value.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value).ok()
}

fn sanitize_id(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn record_to_event(record: Node, now: DateTime<Utc>) -> Option<Event> {
    let (lon, lat) = first_point(record)?;
    // The national feed also carries events from neighbouring departments; keep
    // only those genuinely inside the Vaucluse boundary (not just its bbox).
    if !boundary::contains(lon, lat) {
        return None;
    }

    let record_type = local_xsi_type(record);
    let what = what_for_type(&record_type);
    let comments = comments_by_type(record);
    let (road, town) = road_and_town(record);

    let raw_id = record.attribute("id").unwrap_or("");
    if raw_id.is_empty() {
        return None;
    }

    let description = comments
        .get("description")
        .or_else(|| comments.get("locationDescriptor"))
        .map(|values| values.join(" "))
        .unwrap_or_default();

    let mut label = description.clone();
    if label.is_empty() {
        let mut parts = vec![record_type.clone()];
        if !road.is_empty() {
            parts.push(road.clone());
        }
        if !town.is_empty() {
            parts.push(format!("({town})"));
        }
        label = parts.join(" ");
    }

    let start = parse_datex_time(&nested_text(
        record,
        &["validity", "validityTimeSpecification", "overallStartTime"],
    ));
    let stop = parse_datex_time(&nested_text(
        record,
        &["validity", "validityTimeSpecification", "overallEndTime"],
    ));
    let created = parse_datex_time(&nested_text(record, &["situationRecordCreationTime"]))
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(now);
    let updated = parse_datex_time(&nested_text(record, &["situationRecordVersionTime"]))
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(created);

    let mut tags = BTreeMap::new();
    tags.insert("datex_type".into(), record_type);
    if !road.is_empty() {
        tags.insert("road".into(), road);
    }
    if !town.is_empty() {
        tags.insert("commune".into(), town);
    }
    if !description.is_empty() {
        tags.insert("description".into(), description);
    }
    if let Some(version) = record.attribute("version") {
        tags.insert("datex_version".into(), version.to_string());
    }

    Some(Event {
        id: format!("bison-{}", sanitize_id(raw_id)),
        what: what.to_string(),
        kind: if what == "traffic.roadwork" {
            EventKind::Scheduled
        } else {
            EventKind::Unscheduled
        },
        label,
        start,
        stop,
        lon,
        lat,
        source: "Bison Futé / DIR (DATEX II)".into(),
        createdate: created,
        lastupdate: updated,
        tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<d2LogicalModel xmlns="http://datex2.eu/schema/2/2_0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <payloadPublication xsi:type="SituationPublication">
    <situation id="SIT1">
      <situationRecord xsi:type="ns2:Accident" id="REC/001-A" version="3">
        <situationRecordCreationTime>2026-08-06T08:00:00+02:00</situationRecordCreationTime>
        <situationRecordVersionTime>2026-08-06T09:30:00+02:00</situationRecordVersionTime>
        <validity>
          <validityTimeSpecification>
            <overallStartTime>2026-08-06T08:00:00+02:00</overallStartTime>
            <overallEndTime>2026-08-06T12:00:00+02:00</overallEndTime>
          </validityTimeSpecification>
        </validity>
        <generalPublicComment>
          <comment><values><value>Accident sur la RD 950</value></values></comment>
          <commentType>description</commentType>
        </generalPublicComment>
        <groupOfLocations>
          <locationForDisplay>
            <pointCoordinates>
              <latitude>44.14</latitude>
              <longitude>4.81</longitude>
            </pointCoordinates>
          </locationForDisplay>
          <name>
            <descriptor><values><value>RD 950</value></values></descriptor>
            <tpegOtherPointDescriptorType>linkName</tpegOtherPointDescriptorType>
          </name>
          <name>
            <descriptor><values><value>Orange</value></values></descriptor>
            <tpegOtherPointDescriptorType>townName</tpegOtherPointDescriptorType>
          </name>
        </groupOfLocations>
      </situationRecord>
      <situationRecord xsi:type="ns2:MaintenanceWorks" id="REC/002-B" version="1">
        <groupOfLocations>
          <locationForDisplay>
            <pointCoordinates>
              <latitude>48.85</latitude>
              <longitude>2.35</longitude>
            </pointCoordinates>
          </locationForDisplay>
        </groupOfLocations>
      </situationRecord>
    </situation>
  </payloadPublication>
</d2LogicalModel>"#;

    #[test]
    fn parses_and_filters_vaucluse() {
        let events = parse(FIXTURE).unwrap();
        // The Paris record is filtered out by the bounding box.
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.id, "bison-rec-001-a");
        assert_eq!(event.what, "traffic.accident");
        assert_eq!(event.kind, EventKind::Unscheduled);
        assert_eq!(event.label, "Accident sur la RD 950");
        assert_eq!(event.tags.get("road").unwrap(), "RD 950");
        assert_eq!(event.tags.get("commune").unwrap(), "Orange");
        assert!(event.start.is_some());
        assert!(event.stop.is_some());
    }

    #[test]
    fn maps_types_to_taxonomy() {
        assert_eq!(what_for_type("Accident"), "traffic.accident");
        assert_eq!(what_for_type("MaintenanceWorks"), "traffic.roadwork");
        assert_eq!(what_for_type("AbnormalTraffic"), "traffic.jam");
        assert_eq!(what_for_type("VehicleObstruction"), "traffic.hazard");
        assert_eq!(what_for_type("WeatherRelatedRoadConditions"), "weather.road");
        assert_eq!(what_for_type("PublicEvent"), "traffic.info");
    }
}
