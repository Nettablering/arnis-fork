//! Roblox emitter — stub for Q464.
//!
//! When implemented, this module will emit a Rojo-compatible project
//! tree (`default.project.json` + a `src/` of `.rbxmx` / `.lua` files)
//! representing the same world that the Minecraft and Luanti emitters
//! currently produce. Today it only carries the type seam so the rest
//! of the workspace can already refer to it.

use crate::Emitter;

/// Placeholder Roblox emitter. Constructing one is allowed so the rest
/// of the workspace can wire the type through; calling any real method
/// will be added in Q464.
#[derive(Debug, Default, Clone, Copy)]
pub struct RobloxEmitter;

impl Emitter for RobloxEmitter {}
