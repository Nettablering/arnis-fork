//! Building height (Q031) + road ribbon width (Q034) + water depth (Q035)
//! lookup tables. Engine-neutral metrics in metres; the caller multiplies
//! by stud scale.

/// Three-stage building height waterfall per Q031, expressed in metres.
/// Returns the chosen height **in metres** and the category string used.
pub fn building_height_m(
    explicit_height_m: Option<f32>,
    levels: Option<u16>,
    kind: Option<&str>,
    footprint_area_m2: f64,
) -> (f32, &'static str) {
    // Stage 1: explicit height tag.
    if let Some(h) = explicit_height_m {
        if h.is_finite() && h > 0.0 {
            return (h, "explicit");
        }
    }

    let kind_key = kind.map(|s| s.to_ascii_lowercase()).unwrap_or_default();

    // Stage 2: levels heuristic.
    if let Some(l) = levels {
        if l > 0 {
            let per_level = match kind_key.as_str() {
                "commercial" | "office" | "retail" | "supermarket" => 3.5,
                "industrial" | "warehouse" | "factory" => 4.0,
                _ => 3.0,
            };
            return ((l as f32) * per_level, "levels");
        }
    }

    // Stage 3: category default.
    let (default_m, category) = match kind_key.as_str() {
        "house" | "residential" | "detached" | "bungalow" | "cabin" => (6.0, "residential"),
        "apartments" if footprint_area_m2 > 800.0 => (18.0, "apartments_large"),
        "apartments" => (10.0, "apartments"),
        "commercial" | "office" | "retail" | "supermarket" => (12.0, "commercial"),
        "industrial" | "warehouse" | "factory" => (8.0, "industrial"),
        "public" | "civic" | "hospital" | "school" | "university" | "government" | "townhall" => {
            (15.0, "civic")
        }
        "church" | "cathedral" | "mosque" | "temple" | "synagogue" => (25.0, "religious"),
        "chapel" if footprint_area_m2 < 100.0 => (10.0, "religious_small"),
        "chapel" => (25.0, "religious"),
        "stadium" => (30.0, "stadium"),
        "train_station" | "transportation" => (12.0, "transport"),
        "barn" | "farm_auxiliary" | "greenhouse" | "stable" => (5.0, "agricultural"),
        "hut" | "shed" | "garage" => (3.0, "hut"),
        "skyscraper" => (100.0, "skyscraper"),
        "yes" | "" => (7.0, "generic"),
        _ => (7.0, "generic"),
    };
    (default_m, category)
}

/// Road ribbon width in metres per Q034. `lanes` overrides table width
/// when present (`max(table, lanes * 4 studs / 2 studs_per_m = lanes * 2m)`
/// — but we work in metres here and let the emitter convert later.
/// Returns (width_m, material).
pub fn road_width_m(highway: &str, lanes: Option<u8>) -> (f32, &'static str) {
    let key = highway.to_ascii_lowercase();
    let (base_m, material): (f32, &'static str) = match key.as_str() {
        "motorway" => (12.0, "asphalt_smooth"),
        "trunk" => (10.0, "asphalt"),
        "primary" => (8.0, "asphalt"),
        "secondary" => (6.0, "asphalt"),
        "tertiary" => (5.0, "asphalt"),
        "residential" | "unclassified" => (4.0, "asphalt"),
        "living_street" => (3.0, "paved"),
        "service" => (3.0, "concrete"),
        "pedestrian" => (3.0, "cobble"),
        "track" | "footway" | "bridleway" => (2.0, "dirt"),
        "cycleway" => (2.0, "asphalt_red"),
        "path" | "steps" => (1.5, "dirt"),
        _ => (4.0, "asphalt"),
    };

    let width_m = match lanes {
        Some(n) if n > 0 => base_m.max((n as f32) * 2.0),
        _ => base_m,
    };
    // Cap at 30 m (= 60 studs) per Q034.
    (width_m.min(30.0), material)
}

/// Q035 default depth in metres. Falls back to 3 m for unknown polygons.
pub fn water_depth_m(kind: &str) -> f32 {
    match kind.to_ascii_lowercase().as_str() {
        "river" | "riverbank" => 4.0,
        "canal" => 3.0,
        "stream" => 1.0,
        "ditch" | "drain" => 0.5,
        "reservoir" | "basin" | "dock" => 3.0,
        _ => 3.0,
    }
}

/// Shoelace area of a closed lat/lon ring projected to local metres via
/// the small-angle approximation at the ring's mean latitude.
/// Result is in **square metres** and unsigned.
pub fn polygon_area_m2(ring_latlon: &[[f64; 2]]) -> f64 {
    if ring_latlon.len() < 3 {
        return 0.0;
    }
    let mean_lat = ring_latlon.iter().map(|p| p[0]).sum::<f64>() / (ring_latlon.len() as f64);
    let r = 6_378_137.0_f64;
    let cos_lat = mean_lat.to_radians().cos();
    let to_xy = |p: [f64; 2]| -> (f64, f64) {
        let x = r * (p[1] - ring_latlon[0][1]).to_radians() * cos_lat;
        let y = r * (p[0] - ring_latlon[0][0]).to_radians();
        (x, y)
    };
    let mut sum = 0.0;
    for i in 0..ring_latlon.len() {
        let (x1, y1) = to_xy(ring_latlon[i]);
        let (x2, y2) = to_xy(ring_latlon[(i + 1) % ring_latlon.len()]);
        sum += x1 * y2 - x2 * y1;
    }
    (sum.abs()) * 0.5
}
