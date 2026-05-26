//! arnis-emitters — engine-specific writers built on top of `arnis-core`.
//!
//! Q464 promotes this crate from placeholder to first real emitter: the
//! Roblox JSON-manifest writer. Future tickets will migrate the existing
//! Minecraft (`world_editor/*`) and Luanti emitter code out of
//! `arnis-cli/src/` and into this crate behind the same `Emitter` trait.

pub mod overpass_ingest;
pub mod roblox;

// Re-export so callers can write `arnis_emitters::Emitter` without
// reaching into the core crate. Source of truth stays in arnis-core.
pub use arnis_core::emitter::{Emitter, EmitterError, IngestedTile, TileCoord};
pub use roblox::RobloxEmitter;
