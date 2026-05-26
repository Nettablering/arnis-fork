//! Compile-time-embedded JSON Schema for the Roblox manifest.

use jsonschema::JSONSchema;
use once_cell::sync::Lazy;

pub static SCHEMA_JSON: &str = include_str!("schema/manifest.v1.0.json");

pub static MANIFEST_SCHEMA: Lazy<JSONSchema> = Lazy::new(|| {
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_JSON).expect("manifest.v1.0.json must be valid JSON");
    JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .compile(&schema)
        .expect("manifest schema must compile")
});
