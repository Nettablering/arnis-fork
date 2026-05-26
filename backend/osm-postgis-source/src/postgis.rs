//! [`OsmSource`] backed by the Q085 PostGIS schema (`osm.planet_osm_*`).
//!
//! One query per bbox: a UNION across polygon/line/point with the
//! `ST_AsText` projection so we don't have to pull a binary WKB parser
//! into the crate. Polygons are projected via `ST_ExteriorRing` on the
//! first ring — the bake-pipeline currently only consumes the outer
//! ring (multipart polygons are tracked for a later ticket per
//! overpass_ingest.rs's comment on relations).

use std::collections::BTreeMap;

use sqlx::postgres::PgPool;
use sqlx::Row;

use crate::{Bbox, ElementKind, OsmElement, OsmSource, SourceError};

#[derive(Debug, Clone)]
pub struct PostGisSource {
    pool: PgPool,
}

impl PostGisSource {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl OsmSource for PostGisSource {
    async fn fetch_bbox(&self, bbox: Bbox) -> Result<Vec<OsmElement>, SourceError> {
        // The bbox arrives in WGS84 (SRID 4326); the `way` columns are
        // stored in the same SRID. ST_MakeEnvelope(minx, miny, maxx, maxy)
        // takes longitudes first.
        let sql = r#"
            SELECT osm_id, 'polygon'::text AS osm_type, tags::text AS tags,
                   ST_AsText(ST_ExteriorRing(ST_GeometryN((way), 1))) AS wkt
                FROM osm.planet_osm_polygon
                WHERE way && ST_MakeEnvelope($1, $2, $3, $4, 4326)
                  AND (tags ? 'building' OR tags ? 'natural' OR tags ? 'waterway')
            UNION ALL
            SELECT osm_id, 'line'::text, tags::text,
                   ST_AsText((way))
                FROM osm.planet_osm_line
                WHERE way && ST_MakeEnvelope($1, $2, $3, $4, 4326)
                  AND (tags ? 'highway' OR tags ? 'waterway')
            UNION ALL
            SELECT osm_id, 'point'::text, tags::text,
                   ST_AsText(way)
                FROM osm.planet_osm_point
                WHERE way && ST_MakeEnvelope($1, $2, $3, $4, 4326)
                  AND (tags ? 'amenity' OR tags ? 'shop' OR tags ? 'tourism')
            LIMIT 50000
        "#;

        let rows = sqlx::query(sql)
            .bind(bbox.west_lon)
            .bind(bbox.south_lat)
            .bind(bbox.east_lon)
            .bind(bbox.north_lat)
            .fetch_all(&self.pool)
            .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let osm_id: i64 = row.try_get("osm_id")?;
            let osm_type: String = row.try_get("osm_type")?;
            let tags_str: String = row.try_get("tags")?;
            let wkt: Option<String> = row.try_get("wkt").ok();
            let Some(wkt) = wkt else {
                continue;
            };

            let tags: BTreeMap<String, String> = match serde_json::from_str::<
                BTreeMap<String, serde_json::Value>,
            >(&tags_str)
            {
                Ok(m) => m
                    .into_iter()
                    .filter_map(|(k, v)| match v {
                        serde_json::Value::String(s) => Some((k, s)),
                        serde_json::Value::Number(n) => Some((k, n.to_string())),
                        serde_json::Value::Bool(b) => Some((k, b.to_string())),
                        _ => None,
                    })
                    .collect(),
                Err(_) => continue,
            };

            let geometry = parse_wkt_ring(&wkt);
            if geometry.is_empty() {
                continue;
            }

            // osm2pgsql stores closed-way polygons under positive osm_ids
            // and multipolygon relations under NEGATIVE ids — flip the
            // sign and re-key as a relation so the manifest matches what
            // Overpass would emit.
            let (kind, id_string) = match osm_type.as_str() {
                "polygon" => {
                    if osm_id < 0 {
                        (ElementKind::Relation, (-osm_id).to_string())
                    } else {
                        (ElementKind::Way, osm_id.to_string())
                    }
                }
                "line" => (ElementKind::Way, osm_id.to_string()),
                "point" => (ElementKind::Node, osm_id.to_string()),
                _ => continue,
            };

            out.push(OsmElement {
                osm_id: id_string,
                kind,
                tags,
                geometry,
            });
        }
        Ok(out)
    }

    fn name(&self) -> &'static str {
        "postgis"
    }
}

/// Parse a tiny subset of WKT we expect from PostGIS:
/// `POINT(lon lat)`, `LINESTRING(lon lat, lon lat, ...)`,
/// `LINEARRING(...)` (what `ST_ExteriorRing` emits inside a `POLYGON`).
fn parse_wkt_ring(wkt: &str) -> Vec<[f64; 2]> {
    let open = match wkt.find('(') {
        Some(i) => i + 1,
        None => return Vec::new(),
    };
    let close = match wkt.rfind(')') {
        Some(i) => i,
        None => return Vec::new(),
    };
    if close <= open {
        return Vec::new();
    }
    let body = &wkt[open..close];
    let mut out = Vec::new();
    for pair in body.split(',') {
        let mut nums = pair.split_whitespace();
        let lon = nums.next().and_then(|s| s.parse::<f64>().ok());
        let lat = nums.next().and_then(|s| s.parse::<f64>().ok());
        if let (Some(lon), Some(lat)) = (lon, lat) {
            out.push([lat, lon]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linestring_wkt() {
        let r = parse_wkt_ring("LINESTRING(6.150 62.472,6.151 62.473)");
        assert_eq!(r, vec![[62.472, 6.150], [62.473, 6.151]]);
    }

    #[test]
    fn parses_polygon_exterior_ring_wkt() {
        let r = parse_wkt_ring("LINEARRING(6.150 62.472,6.151 62.472,6.151 62.473,6.150 62.472)");
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn parses_point_wkt() {
        let r = parse_wkt_ring("POINT(6.150 62.472)");
        assert_eq!(r, vec![[62.472, 6.150]]);
    }

    #[test]
    fn empty_on_garbage() {
        assert!(parse_wkt_ring("GARBAGE").is_empty());
    }
}
