//! arnis-emitters — engine-specific writers built on top of `arnis-core`.
//!
//! Placeholder crate for the Nettablering workspace reshape (Q463). Future
//! tickets will migrate the existing Minecraft (`world_editor/*`) and
//! Luanti emitter code out of `arnis-cli/src/` and into this crate, behind
//! a stable `Emitter` trait. The `roblox` submodule below is the seed for
//! Q464 (Roblox/Rojo emitter).

/// Common contract every emitter will eventually implement.
///
/// The trait body is intentionally empty for now — the real surface is
/// designed in Q464 alongside the first non-Minecraft / non-Luanti
/// emitter. Defining the trait here means downstream callers can already
/// take `&dyn Emitter` arguments and the trait object will be filled in
/// without churn at the call sites.
pub trait Emitter {}

pub mod roblox;
