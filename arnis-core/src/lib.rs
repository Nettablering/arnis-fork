//! arnis-core — engine-agnostic primitives for the Arnis world generator.
//!
//! This crate is intentionally a placeholder at the start of the
//! Nettablering workspace reshape (Q463). Subsequent tickets will migrate
//! the following modules out of `arnis-cli/src/` and into here:
//!
//! - `coordinate_system` (geographic <-> cartesian projection)
//! - `osm_parser` + `overture` (Overpass + Overture Maps ingestion)
//! - `elevation`, `elevation_data` (SRTM / TIFF DEM handling)
//! - `land_cover`, `land_cover_bridge_repair`, `land_cover_osm_water_override`
//! - `floodfill`, `floodfill_cache`, `bresenham`, `clipping` (geometry kernels)
//! - `colors`, `block_definitions` (engine-neutral material taxonomy)
//!
//! Each migration is gated on `cargo build --workspace --release
//! --no-default-features` staying green and on upstream-merge friction
//! remaining manageable.

/// Workspace reshape marker — lets downstream crates verify they are
/// linked against the Nettablering fork's split layout rather than the
/// upstream single-crate layout.
pub const NETTABLERING_FORK_LAYOUT_VERSION: u32 = 1;
