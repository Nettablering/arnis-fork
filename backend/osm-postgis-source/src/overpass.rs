//! [`OsmSource`] backed by Overpass JSON.
//!
//! The constructor takes a path to a cached Overpass dump (the artifact
//! produced by `backend/scripts/fetch-overpass.sh`). This keeps the source
//! offline-testable; the live Overpass API is not on the bake hot path per
//! Q086 — only ad-hoc tooling and the integration cross-check sample it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Bbox, ElementKind, OsmElement, OsmSource, SourceError};

#[derive(Debug)]
pub struct OverpassSource {
    json_path: PathBuf,
}

impl OverpassSource {
    pub fn from_path(json_path: impl AsRef<Path>) -> Self {
        Self {
            json_path: json_path.as_ref().to_path_buf(),
        }
    }
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    elements: Vec<RawElement>,
}

#[derive(Deserialize)]
struct RawElement {
    #[serde(rename = "type")]
    el_type: String,
    #[serde(default)]
    id: u64,
    #[serde(default)]
    tags: BTreeMap<String, String>,
    #[serde(default)]
    geometry: Option<Vec<LatLon>>,
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lon: Option<f64>,
}

#[derive(Deserialize)]
struct LatLon {
    lat: f64,
    lon: f64,
}

#[async_trait::async_trait]
impl OsmSource for OverpassSource {
    async fn fetch_bbox(&self, bbox: Bbox) -> Result<Vec<OsmElement>, SourceError> {
        let raw = fs::read(&self.json_path)?;
        let resp: Response = serde_json::from_slice(&raw)?;

        let mut out = Vec::with_capacity(resp.elements.len());
        for el in resp.elements {
            let kind = match el.el_type.as_str() {
                "way" => ElementKind::Way,
                "node" => ElementKind::Node,
                "relation" => ElementKind::Relation,
                _ => continue,
            };
            let geometry: Vec<[f64; 2]> = match (&el.geometry, el.lat, el.lon) {
                (Some(g), _, _) => g.iter().map(|p| [p.lat, p.lon]).collect(),
                (None, Some(lat), Some(lon)) => vec![[lat, lon]],
                _ => continue,
            };
            // Drop anything fully outside the requested bbox — Overpass
            // sometimes returns geometry beyond the query bbox when ways
            // straddle it. We mirror PostgreSQL's `&&` overlap by checking
            // that at least one point falls inside.
            let inside = geometry.iter().any(|p| {
                p[0] >= bbox.south_lat
                    && p[0] <= bbox.north_lat
                    && p[1] >= bbox.west_lon
                    && p[1] <= bbox.east_lon
            });
            if !inside {
                continue;
            }
            out.push(OsmElement {
                osm_id: el.id.to_string(),
                kind,
                tags: el.tags,
                geometry,
            });
        }
        Ok(out)
    }

    fn name(&self) -> &'static str {
        "overpass"
    }
}
