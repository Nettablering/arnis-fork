//! Compile-time-embedded JSON Schemas for the Roblox manifest.
//!
//! Q102 — manifest schema evolution. Every currently-supported schema
//! version is embedded here so the bake-server can validate against, and
//! project down to, any non-retired version. The policy lives in
//! `docs/manifest-schema-evolution.md`; the running version list lives
//! in `docs/manifest-versioning-changelog.md`.
//!
//! SemVer (`major.minor`):
//!  * minor bump  = additive (older clients tolerate unknown optional fields)
//!  * major bump  = breaking (clients must upgrade; server keeps both
//!                  versions side-by-side for the deprecation window)
//!  * patch bump  = docs/clarification only — no on-disk schema bump
//!
//! Adding a new minor version:
//!  1. drop `manifest.v{MAJOR}.{MINOR}.json` next to this file
//!  2. add it to [`SUPPORTED_VERSIONS`]
//!  3. add a projection rule in [`project_down`] for any fields newer
//!     than the requested target
//!  4. note the change in `docs/manifest-versioning-changelog.md`
//!
//! Retiring a version: move it to [`RETIRED_VERSIONS`]. The bake-server
//! returns HTTP 410 Gone for any retired version request.

use jsonschema::JSONSchema;
use once_cell::sync::Lazy;
use serde_json::Value;

/// Inline source of every embedded schema. Key = `"major.minor"` string.
pub static SUPPORTED_VERSIONS: &[(&str, &str)] = &[
    ("1.0", include_str!("schema/manifest.v1.0.json")),
    ("1.1", include_str!("schema/manifest.v1.1.json")),
];

/// Versions that were once shipped but are now hard-retired (HTTP 410
/// at the bake-server). Empty at launch — populated as the
/// deprecation/removal window rolls forward.
pub static RETIRED_VERSIONS: &[&str] = &[];

/// Latest supported `major.minor` — the default when no `schema_version`
/// is requested. Kept in lock-step with `manifest::MANIFEST_VERSION`.
pub const LATEST_VERSION: &str = "1.1";

/// Legacy single-schema constant kept for back-compat with callers that
/// were validating against v1.0 directly. New callers should look up
/// the schema for the version they actually care about via
/// [`schema_for`].
pub static SCHEMA_JSON: &str = include_str!("schema/manifest.v1.1.json");

/// Compiled latest schema (v1.1). Kept name-stable so existing emitter
/// validation code in `mod.rs` continues to work unchanged.
pub static MANIFEST_SCHEMA: Lazy<JSONSchema> = Lazy::new(|| compile_for(LATEST_VERSION));

/// Compile the embedded schema for `version`, or panic with a useful
/// message at startup if the version was retired or never shipped.
pub fn compile_for(version: &str) -> JSONSchema {
    let raw = schema_source_for(version)
        .unwrap_or_else(|| panic!("no embedded schema for manifest version {version}"));
    let value: Value =
        serde_json::from_str(raw).expect("embedded manifest schema must be valid JSON");
    JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .compile(&value)
        .expect("embedded manifest schema must compile")
}

/// Raw embedded JSON Schema text for `version`, or `None` if unknown.
pub fn schema_source_for(version: &str) -> Option<&'static str> {
    SUPPORTED_VERSIONS
        .iter()
        .find(|(v, _)| *v == version)
        .map(|(_, src)| *src)
}

/// Look up a compiled schema for `version`. Cached for repeat calls.
pub fn schema_for(version: &str) -> Option<&'static JSONSchema> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Lazy<Mutex<HashMap<String, &'static JSONSchema>>> =
        Lazy::new(|| Mutex::new(HashMap::new()));

    if !is_supported(version) {
        return None;
    }
    let mut g = CACHE.lock().expect("schema cache poisoned");
    if let Some(s) = g.get(version) {
        return Some(*s);
    }
    let boxed: &'static JSONSchema = Box::leak(Box::new(compile_for(version)));
    g.insert(version.to_string(), boxed);
    Some(boxed)
}

/// `true` if `version` is currently served (not retired, embedded).
pub fn is_supported(version: &str) -> bool {
    SUPPORTED_VERSIONS.iter().any(|(v, _)| *v == version)
}

/// `true` if `version` is past hard-removal — bake-server returns 410.
pub fn is_retired(version: &str) -> bool {
    RETIRED_VERSIONS.iter().any(|v| *v == version)
}

/// Project a manifest JSON value emitted at `LATEST_VERSION` down to
/// `target_version`. Strips fields that only exist in versions newer
/// than `target_version`. The output is mutated in place.
///
/// This is the canonical place to encode "what got added in each
/// minor". When you add a new optional field in v1.N, add a branch
/// here that removes it when `target_version < 1.N`.
pub fn project_down(manifest: &mut Value, target_version: &str) -> Result<(), ProjectError> {
    if !is_supported(target_version) {
        return Err(ProjectError::Unsupported(target_version.to_string()));
    }
    // Always rewrite the header so a client that asked for v1.0 sees
    // `manifest_version: "1.0"` and not "1.1".
    if let Some(obj) = manifest.as_object_mut() {
        obj.insert(
            "manifest_version".to_string(),
            Value::String(target_version.to_string()),
        );
    }

    if version_lt(target_version, "1.1") {
        strip_v1_1_additions(manifest);
    }
    Ok(())
}

fn strip_v1_1_additions(manifest: &mut Value) {
    // Q210: enrichment lives under each landmark.
    if let Some(landmarks) = manifest
        .get_mut("landmarks")
        .and_then(|v| v.as_array_mut())
    {
        for l in landmarks.iter_mut() {
            if let Some(obj) = l.as_object_mut() {
                obj.remove("enrichment");
            }
        }
    }
    // Q211: rarity_score + rarity_tier on buildings.
    if let Some(buildings) = manifest
        .get_mut("buildings")
        .and_then(|v| v.as_array_mut())
    {
        for b in buildings.iter_mut() {
            if let Some(obj) = b.as_object_mut() {
                obj.remove("rarity_score");
                obj.remove("rarity_tier");
            }
        }
    }
}

/// Compare two `major.minor` strings. `true` if `a < b`. Falls back
/// to lexicographic when either side fails to parse — robust enough
/// for the "1.x" universe we ship; a real semver lib lands when we
/// cross to 2.x.
pub fn version_lt(a: &str, b: &str) -> bool {
    let pa = parse_mm(a);
    let pb = parse_mm(b);
    match (pa, pb) {
        (Some((am, an)), Some((bm, bn))) => (am, an) < (bm, bn),
        _ => a < b,
    }
}

fn parse_mm(v: &str) -> Option<(u32, u32)> {
    let (maj, min) = v.split_once('.')?;
    Some((maj.parse().ok()?, min.parse().ok()?))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProjectError {
    #[error("unsupported manifest schema version: {0}")]
    Unsupported(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn latest_is_in_supported() {
        assert!(is_supported(LATEST_VERSION));
        assert!(!is_retired(LATEST_VERSION));
    }

    #[test]
    fn schema_for_unknown_returns_none() {
        assert!(schema_for("9.9").is_none());
    }

    #[test]
    fn schema_for_known_versions_compiles() {
        for (v, _) in SUPPORTED_VERSIONS {
            assert!(schema_for(v).is_some(), "compile failed for {v}");
        }
    }

    #[test]
    fn version_lt_basic() {
        assert!(version_lt("1.0", "1.1"));
        assert!(!version_lt("1.1", "1.0"));
        assert!(!version_lt("1.1", "1.1"));
        assert!(version_lt("1.9", "2.0"));
    }

    #[test]
    fn projection_strips_enrichment_and_rarity_for_v1_0() {
        let mut m = json!({
            "manifest_version": "1.1",
            "landmarks": [
                {"osm_id":"n/1","position_studs":[0.0,0.0],"kind":"x","label":"L",
                 "enrichment":{"wikidata_qid":"Q42"}}
            ],
            "buildings": [
                {"osm_id":"w/1","footprint_studs":[[0.0,0.0],[1.0,0.0],[1.0,1.0]],
                 "height_studs":3.0,"wall_colour_hex":"#aabbcc","roof_colour_hex":"#112233",
                 "category":"residential","claimable":true,
                 "rarity_score":0.42,"rarity_tier":"Rare"}
            ]
        });
        project_down(&mut m, "1.0").unwrap();
        assert_eq!(m["manifest_version"], "1.0");
        assert!(m["landmarks"][0].get("enrichment").is_none());
        assert!(m["buildings"][0].get("rarity_score").is_none());
        assert!(m["buildings"][0].get("rarity_tier").is_none());
    }

    #[test]
    fn projection_to_latest_is_identity_for_payload_fields() {
        let mut m = json!({
            "manifest_version": "1.1",
            "buildings": [
                {"osm_id":"w/1","footprint_studs":[[0.0,0.0],[1.0,0.0],[1.0,1.0]],
                 "height_studs":3.0,"wall_colour_hex":"#aabbcc","roof_colour_hex":"#112233",
                 "category":"residential","claimable":true,
                 "rarity_score":0.42,"rarity_tier":"Rare"}
            ],
            "landmarks": []
        });
        project_down(&mut m, "1.1").unwrap();
        assert_eq!(m["buildings"][0]["rarity_tier"], "Rare");
    }

    #[test]
    fn projection_to_unsupported_errors() {
        let mut m = json!({"manifest_version":"1.1"});
        assert_eq!(
            project_down(&mut m, "9.9"),
            Err(ProjectError::Unsupported("9.9".into()))
        );
    }
}
