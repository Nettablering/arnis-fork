//! `bake-postgis-tile` — Q086 CLI mirror of `arnis-emitters --example bake-tile`.
//!
//! Difference: instead of consuming a cached Overpass JSON, this reads the
//! same bbox out of the worldbuilders PostGIS DB and runs the identical
//! classifier + Roblox emitter. Output is byte-equivalent to the Overpass
//! variant when the same OSM snapshot is loaded (see integration test
//! `tests/equivalence.rs`).
//!
//! Build:  cargo build -p osm-postgis-source --bin bake-postgis-tile --features cli --release
//! Usage:
//!   bake-postgis-tile --source <overpass|postgis> \
//!       --center-lat <lat> --center-lon <lon> --z <z> --output <manifest.json>
//!       [--input <overpass.json>] [--database-url postgres://…]

use std::{fs, path::PathBuf};

use arnis_core::emitter::Emitter;
use arnis_emitters::roblox::RobloxEmitter;
use clap::{Parser, ValueEnum};
use osm_postgis_source::{
    classify, slippy_tile_bbox, slippy_tile_for, OsmSource, OverpassSource, PostGisSource,
};

#[derive(Parser, Debug)]
#[command(name = "bake-postgis-tile", about = "Q086 — bake-tile via OsmSource trait")]
struct Args {
    #[arg(long, value_enum, default_value_t = SourceKind::Postgis)]
    source: SourceKind,

    /// For --source=overpass: path to Overpass JSON dump.
    #[arg(long)]
    input: Option<PathBuf>,

    /// For --source=postgis: full DATABASE_URL.
    /// Defaults to $DATABASE_URL.
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,

    #[arg(long)]
    center_lat: f64,
    #[arg(long)]
    center_lon: f64,
    #[arg(long, default_value_t = 16)]
    z: u8,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value = "NO_rural_subarctic")]
    region_key: String,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum SourceKind {
    Overpass,
    Postgis,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let coord = slippy_tile_for(args.center_lat, args.center_lon, args.z);
    let bbox = slippy_tile_bbox(coord).into();

    let elements = match args.source {
        SourceKind::Overpass => {
            let path = args
                .input
                .ok_or_else(|| anyhow::anyhow!("--input required for --source=overpass"))?;
            OverpassSource::from_path(path).fetch_bbox(bbox).await?
        }
        SourceKind::Postgis => {
            let url = args
                .database_url
                .ok_or_else(|| anyhow::anyhow!("DATABASE_URL not set"))?;
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(4)
                .connect(&url)
                .await?;
            PostGisSource::new(pool).fetch_bbox(bbox).await?
        }
    };

    let mut tile = classify(elements, coord);
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
        "[bake-postgis-tile] source={:?} z={} x={} y={} buildings={} roads={} water={} bytes={}",
        args.source,
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
