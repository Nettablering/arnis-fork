//! `bake-tile` — Q465 vertical slice CLI.
//!
//! Reads an Overpass JSON file (produced by `backend/scripts/fetch-overpass.sh`),
//! converts it to an [`arnis_core::emitter::IngestedTile`], runs the Roblox
//! emitter, validates the manifest against the embedded JSON Schema, and
//! writes `manifest.json` to disk.
//!
//! Build:
//!   cargo build -p arnis-emitters --example bake-tile --features cli --release
//!
//! Usage:
//!   bake-tile --input <overpass.json> --center-lat <lat> --center-lon <lon>
//!             --z <z> --output <manifest.json>
//!
//! `--z` defaults to 16 (Q465 first-tile zoom). The output filename pattern
//! `<z>-<x>-<y>.json` (used by the bake-server cache layout) is the caller's
//! responsibility — see `backend/scripts/bake-tile.sh`.

use std::{fs, path::PathBuf};

use arnis_core::emitter::Emitter;
use arnis_emitters::overpass_ingest::{ingest_overpass, slippy_tile_for};
use arnis_emitters::roblox::RobloxEmitter;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "bake-tile",
    about = "Bake one Roblox tile manifest from a cached Overpass JSON"
)]
struct Args {
    /// Overpass JSON file produced by `fetch-overpass.sh`.
    #[arg(long)]
    input: PathBuf,

    /// Tile centre latitude (used to compute slippy x/y).
    #[arg(long)]
    center_lat: f64,

    /// Tile centre longitude.
    #[arg(long)]
    center_lon: f64,

    /// Zoom level. Q465 default is 16 (~600 m tiles at this latitude).
    #[arg(long, default_value_t = 16)]
    z: u8,

    /// Output manifest path. Parent directory is created.
    #[arg(long)]
    output: PathBuf,

    /// Region key passed through to the emitter (palette selection).
    #[arg(long, default_value = "NO_rural_subarctic")]
    region_key: String,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let raw = fs::read(&args.input)?;
    let coord = slippy_tile_for(args.center_lat, args.center_lon, args.z);
    let mut tile = ingest_overpass(&raw, coord)?;
    tile.region_key = Some(args.region_key.clone());

    let emitter = RobloxEmitter::default();
    let manifest = emitter.build_manifest(&tile);
    emitter
        .validate(&manifest)
        .map_err(|e| anyhow::anyhow!("schema validation failed: {e}"))?;

    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&args.output, &json)?;

    eprintln!(
        "[bake-tile] z={} x={} y={} buildings={} roads={} water={} bytes={}",
        coord.z,
        coord.x,
        coord.y,
        manifest.buildings.len(),
        manifest.roads.len(),
        manifest.water.len(),
        json.len(),
    );

    Ok(())
}
