//! Landmark manifest loader.
//!
//! The bundled `top-1000-landmarks.toml` file ships with the crate via
//! `include_str!`. Each entry carries:
//!
//! ```toml
//! [[landmark]]
//! name = "Eiffel Tower"
//! lat = 48.8584
//! lon = 2.2945
//! seed_source = "curated"   # or "placeholder"
//! # optional explicit override (else computed from lat/lon at z=15):
//! # z = 15
//! # x = 16606
//! # y = 11277
//! ```
//!
//! `seed_source = "placeholder"` flags rows that exist only to fill the
//! ≤2000 budget per the Q083 grill; they are still enqueued (a real bake
//! at z=15 is cheap and warms the disk cache) but ops can grep them and
//! replace them with curated entries on the quarterly refresh.

use crate::DEFAULT_ZOOM;
use serde::Deserialize;
use std::f64::consts::PI;

/// Source-of-truth tag for a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SeedSource {
    /// Hand-curated by ops or sourced from a real list (Wikidata, UN, etc).
    Curated,
    /// Geometric fill — exists to round out the ≤2000 budget. Swap for
    /// a curated row on quarterly refresh.
    Placeholder,
}

impl SeedSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SeedSource::Curated => "curated",
            SeedSource::Placeholder => "placeholder",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Landmark {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub seed_source: SeedSource,
    #[serde(default)]
    pub z: Option<u32>,
    #[serde(default)]
    pub x: Option<u32>,
    #[serde(default)]
    pub y: Option<u32>,
}

impl Landmark {
    /// Resolve (z, x, y) — explicit fields win, otherwise compute from
    /// (lat, lon) at the crate default zoom.
    pub fn tile(&self) -> (u32, u32, u32) {
        let z = self.z.unwrap_or(DEFAULT_ZOOM);
        if let (Some(x), Some(y)) = (self.x, self.y) {
            return (z, x, y);
        }
        let (x, y) = lonlat_to_tile(self.lon, self.lat, z);
        (z, x, y)
    }

    pub fn tile_id(&self) -> String {
        let (z, x, y) = self.tile();
        format!("{z}/{x}/{y}")
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LandmarkManifest {
    #[serde(default, rename = "landmark")]
    pub landmarks: Vec<Landmark>,
}

impl LandmarkManifest {
    /// The canonical bundled list. Compiled into the crate so `wb-preheat`
    /// has no external file dependency.
    pub fn bundled() -> anyhow::Result<Self> {
        let raw = include_str!("top-1000-landmarks.toml");
        Self::from_str(raw)
    }

    pub fn from_str(raw: &str) -> anyhow::Result<Self> {
        let m: LandmarkManifest = toml::from_str(raw)?;
        Ok(m)
    }

    pub fn len(&self) -> usize {
        self.landmarks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.landmarks.is_empty()
    }

    /// Count of curated (non-placeholder) rows.
    pub fn curated_count(&self) -> usize {
        self.landmarks
            .iter()
            .filter(|l| l.seed_source == SeedSource::Curated)
            .count()
    }
}

/// Slippy-map projection at zoom `z`. Returns `(tile_x, tile_y)`.
///
/// This is the canonical OSM formula:
/// `x = floor((lon + 180) / 360 * 2^z)`
/// `y = floor((1 - asinh(tan(lat)) / π) / 2 * 2^z)`
pub fn lonlat_to_tile(lon: f64, lat: f64, z: u32) -> (u32, u32) {
    let n = 2f64.powi(z as i32);
    let lat_rad = lat.to_radians();
    let x = ((lon + 180.0) / 360.0 * n).floor() as i64;
    let y = ((1.0 - lat_rad.tan().asinh() / PI) / 2.0 * n).floor() as i64;
    let x = x.clamp(0, (n as i64) - 1) as u32;
    let y = y.clamp(0, (n as i64) - 1) as u32;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eiffel_tower_tile_is_in_paris_z15() {
        // The Q083 grill quotes (16606, 11277) using a slightly different
        // landmark lat/lon than the curated (48.8584, 2.2945) we use; the
        // canonical OSM projection on our coord lands at (16592, 11272).
        // Either is "Paris z=15" — what we assert here is that the
        // projection lands inside the Paris bbox at z=15 (not e.g. in
        // the Atlantic or in Beijing).
        let (x, y) = lonlat_to_tile(2.2945, 48.8584, 15);
        assert!((x as i64 - 16599).abs() < 20, "x={x} not near Paris");
        assert!((y as i64 - 11275).abs() < 20, "y={y} not near Paris");
    }

    #[test]
    fn projection_is_self_consistent_across_known_points() {
        // Cross-check the projection against a hand-computed reference
        // point (Greenwich Royal Observatory, ~lat 51.4769, lon 0).
        // At z=15, x should be exactly 2^15/2 = 16384 (lon == 0 lies on
        // the prime meridian / west edge of the right-hand half).
        let (x, _) = lonlat_to_tile(0.0, 51.4769, 15);
        assert_eq!(x, 16384, "lon=0 must project to x=2^14");
        // Sydney is in the southern hemisphere, eastern longitudes —
        // y must be > 2^14 (south of equator) and x > 2^14.
        let (xs, ys) = lonlat_to_tile(151.2153, -33.8568, 15);
        assert!(xs > 16384);
        assert!(ys > 16384);
    }

    #[test]
    fn bundled_manifest_loads_and_has_targets() {
        let m = LandmarkManifest::bundled().expect("manifest parses");
        assert!(
            m.len() >= 1000,
            "Q083 budget: at least 1000 entries, got {}",
            m.len()
        );
        assert!(
            m.curated_count() >= 200,
            "Q083 budget: at least 200 curated entries, got {}",
            m.curated_count()
        );
    }

    #[test]
    fn manifest_round_trip_inline() {
        let raw = r#"
[[landmark]]
name = "Eiffel Tower"
lat = 48.8584
lon = 2.2945
seed_source = "curated"

[[landmark]]
name = "Placeholder Fill"
lat = 0.0
lon = 0.0
seed_source = "placeholder"
"#;
        let m = LandmarkManifest::from_str(raw).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m.curated_count(), 1);
        let eiffel = &m.landmarks[0];
        // Canonical projection at z=15 for (48.8584, 2.2945).
        assert_eq!(eiffel.tile_id(), "15/16592/11272");
    }
}
