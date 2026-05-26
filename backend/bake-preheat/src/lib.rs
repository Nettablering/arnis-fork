//! Worldbuilders bake-preheat (Q083).
//!
//! Cron-driven preheat of the top-N hot tiles. Reads two sources, dedupes,
//! and enqueues `Priority::HotPreheat` jobs to `wb:bake.requests.hot` via
//! the [`bake_queue::Producer`]:
//!
//!  1. **Pinned set** — `wb:cache.hot.tiles` Redis sorted-set (Q082).
//!     Members are tile-ids in `"{z}/{x}/{y}"` form. Read with
//!     `ZRANGEBYSCORE -inf +inf`.
//!  2. **Landmarks manifest** — A static `landmarks.toml` bundled inside
//!     this crate, listing the curated top-1000 global hot tiles
//!     (capitals + iconic landmarks) by `(name, lat, lon)`. Tile
//!     coordinates are computed at load time using the slippy-map
//!     projection at z=15.
//!
//! The preheat module is the *only* writer to `wb:bake.requests.hot`;
//! the cache module (Q082) deliberately never enqueues bakes itself.
//! That boundary is documented in `CONTEXT.md` (Q082+Q083 contract).
//!
//! See `docs/grill/q083-hot-tile-preheating.md`.

pub mod landmarks;
pub mod preheat;
pub mod pinned;

pub use landmarks::{Landmark, LandmarkManifest, SeedSource};
pub use preheat::{PreheatOutcome, PreheatRunner, Tier};
pub use pinned::{PinnedReader, RedisPinnedReader};

/// Default style version used by preheated bakes. The cron picks the
/// current production style version; for the lib default we mirror the
/// bake-server's compile-time constant (Q082 / Q474 keep these in sync
/// at deploy time).
pub const DEFAULT_STYLE_VERSION: u32 = 7;

/// Default OSM snapshot tag used when none is configured. The cron
/// resolves this from `/srv/worldbuilders/state/osm-snapshot` at run
/// time; the lib default is only used in tests and dry-runs.
pub const DEFAULT_OSM_SNAPSHOT: &str = "preheat-default";

/// Default z-level of HOT-tier tiles per Q083 (z=15 only for HOT).
pub const DEFAULT_ZOOM: u32 = 15;

/// Default rate-limit (enqueues per second). The CLI flag overrides.
pub const DEFAULT_RATE_LIMIT: u32 = 50;
