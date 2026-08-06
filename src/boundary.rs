//! Precise Vaucluse (FR-84) point-in-polygon test.
//!
//! The DATEX II feed is national: a loose bounding box wrongly captures events
//! from neighbouring departments (Gard across the Rhône, Bouches-du-Rhône to the
//! south). We therefore test each point against the actual department boundary
//! (simplified, including the Valréas enclave), embedded at compile time.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::model::VAUCLUSE_BBOX;

const BOUNDARY_GEOJSON: &str = include_str!("../data/vaucluse.geojson");

/// A ring is a closed list of [lon, lat] vertices.
type Ring = Vec<[f64; 2]>;
/// A polygon is an outer ring followed by optional holes.
type Polygon = Vec<Ring>;

#[derive(Deserialize)]
struct Feature {
    geometry: Geometry,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    kind: String,
    coordinates: serde_json::Value,
}

fn polygons() -> &'static Vec<Polygon> {
    static POLYGONS: OnceLock<Vec<Polygon>> = OnceLock::new();
    POLYGONS.get_or_init(|| {
        let feature: Feature =
            serde_json::from_str(BOUNDARY_GEOJSON).expect("valid embedded Vaucluse boundary");
        let raw = feature.geometry.coordinates;
        match feature.geometry.kind.as_str() {
            "MultiPolygon" => serde_json::from_value(raw).expect("MultiPolygon coordinates"),
            "Polygon" => {
                let poly: Polygon =
                    serde_json::from_value(raw).expect("Polygon coordinates");
                vec![poly]
            }
            other => panic!("unsupported boundary geometry: {other}"),
        }
    })
}

fn point_in_ring(lon: f64, lat: f64, ring: &Ring) -> bool {
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (ring[i][0], ring[i][1]);
        let (xj, yj) = (ring[j][0], ring[j][1]);
        if ((yi > lat) != (yj > lat)) && (lon < (xj - xi) * (lat - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// True when (lon, lat) lies inside the Vaucluse department boundary.
pub fn contains(lon: f64, lat: f64) -> bool {
    // Fast reject using the coarse envelope before the ray-casting test.
    let (lon_min, lat_min, lon_max, lat_max) = VAUCLUSE_BBOX;
    if lon < lon_min || lon > lon_max || lat < lat_min || lat > lat_max {
        return false;
    }
    for poly in polygons() {
        if let Some((outer, holes)) = poly.split_first() {
            if point_in_ring(lon, lat, outer)
                && !holes.iter().any(|hole| point_in_ring(lon, lat, hole))
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_neighbouring_departments() {
        // Events wrongly caught by the loose bbox (Gard / Bouches-du-Rhône).
        assert!(!contains(4.7095, 43.9506), "Saze (N100, Gard)");
        assert!(!contains(4.6553, 44.2517), "Pont-Saint-Esprit (N86, Gard)");
        assert!(!contains(4.7055, 44.0838), "Laudun-l'Ardoise (N580, Gard)");
        assert!(!contains(4.9893, 43.5924), "Miramas (N569, Bouches-du-Rhône)");
    }

    #[test]
    fn includes_vaucluse_towns() {
        assert!(contains(4.808, 44.138), "Orange");
        assert!(contains(4.805, 43.949), "Avignon");
        assert!(contains(5.048, 44.055), "Carpentras");
        assert!(contains(5.396, 43.876), "Apt");
        assert!(contains(4.9386, 44.3846), "Valréas (enclave des Papes)");
    }
}
