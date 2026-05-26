//! arnis-core — engine-agnostic primitives for the Arnis world generator.
//!
//! Q463 created the placeholder; Q464 fills in the geometry primitives that
//! every emitter needs: a `TileCoord`/`IngestedTile` ingestion contract, a
//! local-tangent-plane projection (Q037), and the `Emitter` trait that
//! engine-specific writers (Roblox, Minecraft, Luanti) implement.
//!
//! Subsequent tickets will migrate the heavier modules out of
//! `arnis-cli/src/` (Overpass ingestion, elevation, land cover, …).

pub mod emitter;
pub mod projection;

/// Workspace reshape marker — lets downstream crates verify they are
/// linked against the Nettablering fork's split layout rather than the
/// upstream single-crate layout.
pub const NETTABLERING_FORK_LAYOUT_VERSION: u32 = 1;
