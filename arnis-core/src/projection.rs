//! Local-tangent-plane projection (Q037) at the configured stud scale (Q038).
//!
//! For tiles of ~200 m on a side the tangent-plane approximation has
//! sub-millimetre error relative to true geodesic distance, which is fine
//! for our use case. The projection is intentionally trivial so it can be
//! unit-tested without floating-point fuzz.

/// WGS84 equatorial radius in metres.
pub const WGS84_EQUATORIAL_RADIUS_M: f64 = 6_378_137.0;

/// Default stud scale from Q038: 2 studs per real-world metre.
pub const DEFAULT_STUDS_PER_METRE: f64 = 2.0;

/// A geographic origin (lat, lon) in degrees that local-tangent-plane
/// projection is rebased against. Per Q037 this is the tile's south-west
/// corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LtpOrigin {
    pub lat_deg: f64,
    pub lon_deg: f64,
}

impl LtpOrigin {
    pub const fn new(lat_deg: f64, lon_deg: f64) -> Self {
        Self { lat_deg, lon_deg }
    }

    /// Project a (lat, lon) point in degrees to local east/north metres
    /// using the small-angle tangent-plane approximation of Q037.
    pub fn project_metres(&self, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
        let dlat_rad = (lat_deg - self.lat_deg).to_radians();
        let dlon_rad = (lon_deg - self.lon_deg).to_radians();
        let cos_lat0 = self.lat_deg.to_radians().cos();
        let x_m = WGS84_EQUATORIAL_RADIUS_M * dlon_rad * cos_lat0;
        let y_m = WGS84_EQUATORIAL_RADIUS_M * dlat_rad;
        (x_m, y_m)
    }

    /// Convenience: project straight to studs at the given stud scale.
    pub fn project_studs(&self, lat_deg: f64, lon_deg: f64, studs_per_metre: f64) -> (f64, f64) {
        let (x_m, y_m) = self.project_metres(lat_deg, lon_deg);
        (x_m * studs_per_metre, y_m * studs_per_metre)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_projects_to_zero() {
        let o = LtpOrigin::new(59.91, 10.75); // Oslo
        let (x, y) = o.project_metres(59.91, 10.75);
        assert!(x.abs() < 1e-9, "x at origin must be 0, got {x}");
        assert!(y.abs() < 1e-9, "y at origin must be 0, got {y}");
    }

    #[test]
    fn one_degree_latitude_north_is_about_111km() {
        // 1° of latitude ≈ 111_320 m on the WGS84 sphere approximation used here.
        let o = LtpOrigin::new(0.0, 0.0);
        let (_x, y) = o.project_metres(1.0, 0.0);
        let expected = WGS84_EQUATORIAL_RADIUS_M * 1f64.to_radians();
        assert!((y - expected).abs() < 1.0, "y={y}, expected≈{expected}");
        assert!((y - 111_319.0).abs() < 5.0, "y must be ~111_320m, got {y}");
    }

    #[test]
    fn longitude_shrinks_with_cos_lat() {
        // 1° lon at the equator is ~111 km; at 60° lat it's ~55.5 km (cos 60° = 0.5).
        let eq = LtpOrigin::new(0.0, 0.0);
        let (x_eq, _) = eq.project_metres(0.0, 1.0);
        let polar = LtpOrigin::new(60.0, 0.0);
        let (x_60, _) = polar.project_metres(60.0, 1.0);
        // cos(60°) = 0.5 exactly.
        assert!(
            (x_60 / x_eq - 0.5).abs() < 1e-6,
            "ratio was {}",
            x_60 / x_eq
        );
    }

    #[test]
    fn stud_scale_doubles_metres() {
        let o = LtpOrigin::new(59.91, 10.75);
        let (xm, ym) = o.project_metres(59.911, 10.751);
        let (xs, ys) = o.project_studs(59.911, 10.751, DEFAULT_STUDS_PER_METRE);
        assert!((xs - xm * 2.0).abs() < 1e-9);
        assert!((ys - ym * 2.0).abs() < 1e-9);
    }
}
