use crate::args::Args;
use crate::coordinate_system::cartesian::XZBBox;
use crate::coordinate_system::geographic::LLBBox;
use crate::element_processing::*;
use crate::floodfill_cache::{CoordinateBitmap, FloodFillCache};
use crate::ground::Ground;
use crate::ground_generation;
use crate::map_renderer;
use crate::osm_parser::{ProcessedElement, ProcessedMemberRole};
use crate::progress::{emit_gui_progress_update, emit_map_preview_ready, emit_show_in_folder};
#[cfg(feature = "gui")]
use crate::telemetry::{send_log, LogLevel};
use crate::tile;
use crate::world_editor::{WorldEditor, WorldFormat};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

/// Generation options that can be passed separately from CLI Args
#[derive(Clone)]
pub struct GenerationOptions {
    pub path: PathBuf,
    pub format: WorldFormat,
    pub level_name: Option<String>,
    pub spawn_point: Option<(i32, i32)>,
}

/// Process a single element by dispatching to the appropriate element processor.
///
/// Extracted from the main loop to enable reuse in both the sequential
/// and parallel tile-based processing paths.
#[allow(clippy::too_many_arguments)]
fn process_element(
    editor: &mut WorldEditor<'_>,
    element: &ProcessedElement,
    args: &Args,
    highway_connectivity: &highways::HighwayConnectivityMap,
    flood_fill_cache: &FloodFillCache,
    building_footprints: &CoordinateBitmap,
    building_passages: &CoordinateBitmap,
    road_mask: &CoordinateBitmap,
    xzbbox: &XZBBox,
    suppressed_building_outlines: &HashSet<u64>,
    subway_points: &mut Vec<(i32, i32)>,
) {
    match element {
        ProcessedElement::Way(way) => {
            if way.tags.contains_key("building") || way.tags.contains_key("building:part") {
                // Skip building outlines that are suppressed by building relations with parts.
                // The individual building:part ways will render instead.
                if !suppressed_building_outlines.contains(&way.id) {
                    buildings::generate_buildings(
                        editor,
                        way,
                        args,
                        None,
                        None,
                        flood_fill_cache,
                        building_passages,
                    );
                }
            } else if way.tags.contains_key("highway") {
                highways::generate_highways(
                    editor,
                    element,
                    args,
                    highway_connectivity,
                    flood_fill_cache,
                    road_mask,
                );
            } else if way.tags.contains_key("landuse") {
                landuse::generate_landuse(editor, way, args, flood_fill_cache, building_footprints);
            } else if way.tags.contains_key("natural") {
                natural::generate_natural(
                    editor,
                    element,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if way.tags.contains_key("amenity") {
                amenities::generate_amenities(editor, element, args, flood_fill_cache, road_mask);
            } else if way.tags.contains_key("leisure") {
                leisure::generate_leisure(editor, way, args, flood_fill_cache, building_footprints);
            } else if way.tags.contains_key("barrier") {
                barriers::generate_barriers(editor, element);
            } else if let Some(val) = way.tags.get("waterway") {
                if val == "dock" {
                    // docks count as water areas
                    water_areas::generate_water_area_from_way(editor, way, xzbbox);
                } else {
                    waterways::generate_waterways(editor, way);
                }
            } else if way.tags.contains_key("bridge") {
                //bridges::generate_bridges(editor, way, ground_level); // TODO FIX
            } else if way.tags.contains_key("railway") {
                railways::generate_railways(editor, way, subway_points);
            } else if way.tags.contains_key("roller_coaster") {
                railways::generate_roller_coaster(editor, way);
            } else if way.tags.contains_key("aeroway") || way.tags.contains_key("area:aeroway") {
                highways::generate_aeroway(editor, way, args);
            } else if way.tags.get("service") == Some(&"siding".to_string()) {
                highways::generate_siding(editor, way);
            } else if way.tags.get("tomb") == Some(&"pyramid".to_string()) {
                historic::generate_pyramid(editor, way, args, flood_fill_cache);
            } else if way.tags.contains_key("man_made") {
                man_made::generate_man_made(editor, element, args);
            } else if way.tags.contains_key("power") {
                power::generate_power(editor, element);
            } else if way.tags.contains_key("place") {
                landuse::generate_place(editor, way, args, flood_fill_cache);
            }
        }
        ProcessedElement::Node(node) => {
            if node.tags.contains_key("door") || node.tags.contains_key("entrance") {
                doors::generate_doors(editor, node);
            } else if node.tags.contains_key("natural")
                && node.tags.get("natural") == Some(&"tree".to_string())
            {
                natural::generate_natural(
                    editor,
                    element,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if node.tags.contains_key("amenity") {
                amenities::generate_amenities(editor, element, args, flood_fill_cache, road_mask);
            } else if node.tags.contains_key("barrier") {
                barriers::generate_barrier_nodes(editor, node);
            } else if node.tags.contains_key("highway") {
                highways::generate_highways(
                    editor,
                    element,
                    args,
                    highway_connectivity,
                    flood_fill_cache,
                    road_mask,
                );
            } else if node.tags.contains_key("tourism") {
                tourisms::generate_tourisms(editor, node);
            } else if node.tags.contains_key("man_made") {
                man_made::generate_man_made_nodes(editor, node);
            } else if node.tags.contains_key("power") {
                power::generate_power_nodes(editor, node);
            } else if node.tags.contains_key("historic") {
                historic::generate_historic(editor, node);
            } else if node.tags.contains_key("emergency") {
                emergency::generate_emergency(editor, node);
            } else if node.tags.contains_key("advertising") {
                advertising::generate_advertising(editor, node);
            }
        }
        ProcessedElement::Relation(rel) => {
            let is_building_relation = rel.tags.contains_key("building")
                || rel.tags.contains_key("building:part")
                || rel.tags.get("type").map(|t| t.as_str()) == Some("building");
            if is_building_relation {
                buildings::generate_building_from_relation(
                    editor,
                    rel,
                    args,
                    flood_fill_cache,
                    xzbbox,
                    building_passages,
                );
            } else if rel.tags.contains_key("water")
                || rel
                    .tags
                    .get("natural")
                    .map(|val| val == "water" || val == "bay")
                    .unwrap_or(false)
            {
                water_areas::generate_water_areas_from_relation(editor, rel, xzbbox);
            } else if rel.tags.contains_key("natural") {
                natural::generate_natural_from_relation(
                    editor,
                    rel,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if rel.tags.contains_key("landuse") {
                landuse::generate_landuse_from_relation(
                    editor,
                    rel,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if rel.tags.get("leisure") == Some(&"park".to_string()) {
                leisure::generate_leisure_from_relation(
                    editor,
                    rel,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if rel.tags.contains_key("man_made") {
                man_made::generate_man_made(editor, element, args);
            }
        }
    }
}

/// Generate world with explicit format options (used by GUI for Bedrock support)
pub fn generate_world_with_options(
    elements: Vec<ProcessedElement>,
    xzbbox: XZBBox,
    llbbox: LLBBox,
    ground: Ground,
    args: &Args,
    options: GenerationOptions,
) -> Result<PathBuf, String> {
    let output_path = options.path.clone();
    let world_format = options.format;
    let generation_start = args.benchmark.then(std::time::Instant::now);

    // Create editor with appropriate format
    let mut editor: WorldEditor = WorldEditor::new_with_format_and_name(
        options.path,
        &xzbbox,
        llbbox,
        options.format,
        options.level_name.clone(),
        options.spawn_point,
        args.disable_height_limit,
    );
    editor.set_projection_info(&args.projection.to_string(), args.scale);
    let ground = Arc::new(ground);

    println!("{} Processing data...", "[4/7]".bold());

    // Build highway connectivity map once before processing
    let highway_connectivity = highways::build_highway_connectivity_map(&elements);

    // Collect subway centerline points for post-ground-fill air carving (phase 2).
    let mut subway_points: Vec<(i32, i32)> = Vec::new();

    // Set ground reference in the editor to enable elevation-aware block placement
    editor.set_ground(Arc::clone(&ground));

    println!("{} Processing terrain...", "[5/7]".bold());
    emit_gui_progress_update(25.0, "Processing terrain...");

    // Pre-compute all flood fills in parallel for better CPU utilization
    let mut flood_fill_cache = FloodFillCache::precompute(&elements, args.timeout.as_ref());

    // Collect building footprints to prevent trees from spawning inside buildings
    // Uses a memory-efficient bitmap (~1 bit per coordinate) instead of a HashSet (~24 bytes per coordinate)
    let building_footprints = flood_fill_cache.collect_building_footprints(&elements, &xzbbox);

    // Collect coordinates covered by tunnel=building_passage highways so that
    // building generation can cut ground-level openings through walls and floors.
    let building_passages =
        highways::collect_building_passage_coords(&elements, &xzbbox, args.scale);

    // Pre-build a bitmap of every (x, z) block coordinate covered by a rendered
    // road or path surface. Uses the same Bresenham + block_range geometry as
    // generate_highways_internal, so the bitmap is a 1:1 match of what gets placed.
    // Amenity processors use this for O(1) nearest-road-block lookups.
    let road_mask = highways::collect_road_surface_coords(&elements, &xzbbox, args.scale);

    // Pre-scan: detect building relation outlines that should be suppressed.
    // Only applies to type=building relations (NOT type=multipolygon).
    // When a type=building relation has "part" members, the outline way should not
    // render as a standalone building, the individual parts render instead.
    let suppressed_building_outlines: HashSet<u64> = {
        let mut outlines = HashSet::new();
        for element in &elements {
            if let ProcessedElement::Relation(rel) = element {
                let is_building_type = rel.tags.get("type").map(|t| t.as_str()) == Some("building");
                if is_building_type {
                    let has_parts = rel
                        .members
                        .iter()
                        .any(|m| m.role == ProcessedMemberRole::Part);
                    if has_parts {
                        for member in &rel.members {
                            if member.role == ProcessedMemberRole::Outer {
                                outlines.insert(member.way.id);
                            }
                        }
                    }
                }
            }
        }
        outlines
    };

    // Decide between sequential and parallel processing based on world size.
    // Tile subdivision is aligned to 512-block Minecraft region boundaries.
    let tiles = tile::create_tiles(&xzbbox, tile::DEFAULT_TILE_SIZE);

    println!(
        "  xzbbox: ({},{}) to ({},{}), size: {}x{}",
        xzbbox.min_x(),
        xzbbox.min_z(),
        xzbbox.max_x(),
        xzbbox.max_z(),
        xzbbox.max_x() - xzbbox.min_x(),
        xzbbox.max_z() - xzbbox.min_z(),
    );
    println!("  Tiles: {}, Elements: {}", tiles.len(), elements.len());

    if tiles.len() >= 3 {
        // Large area: process tiles in parallel using rayon.
        // Each tile gets its own WorldEditor with an expanded bounding box (64-block
        // halo) so that elements whose centroid falls inside the tile can render blocks
        // that extend slightly beyond the strict tile boundary (e.g., wide buildings).
        // After each batch finishes, their WorldToModify results are merged back into the
        // main editor using authoritative bounds (strict tile area overwrites; halo
        // writes only if the target position is still AIR).
        //
        // Tiles are processed in batches (one tile per rayon thread) to cap peak memory.
        // Without batching, all tile WorldToModify structs would be in memory at once,
        // which can exceed RAM for large areas and cause disk thrashing.
        let tile_batch_size = rayon::current_num_threads().max(1);
        println!(
            "  Parallel processing: {} tiles (batch size: {})",
            tiles.len(),
            tile_batch_size
        );

        let tile_assignments = tile::assign_elements_to_tiles(&elements, &tiles);

        let phase_start = std::time::Instant::now();
        // LPT scheduling: sort tiles by element-count descending so dense urban
        // tiles run first. Without this, a straggler dense tile arriving in the
        // last batch (with otherwise-empty siblings) blocks the whole pipeline;
        // running it earlier lets the rest fill in around it.
        let mut indexed_tiles: Vec<(usize, &tile::TileBounds)> = tiles.iter().enumerate().collect();
        indexed_tiles.sort_by(|a, b| {
            tile_assignments[b.0]
                .len()
                .cmp(&tile_assignments[a.0].len())
        });

        for batch in indexed_tiles.chunks(tile_batch_size) {
            // Phase 1: process this batch of tiles in parallel
            let batch_results: Vec<_> = batch
                .par_iter()
                .map(|&(tile_idx, tile_bounds)| {
                    let tile_xzbbox = XZBBox::rect_from_min_max(
                        tile_bounds.min_x - tile::TILE_EDITOR_HALO,
                        tile_bounds.min_z - tile::TILE_EDITOR_HALO,
                        tile_bounds.max_x + tile::TILE_EDITOR_HALO,
                        tile_bounds.max_z + tile::TILE_EDITOR_HALO,
                    )
                    .expect("Failed to create tile XZBBox");

                    let mut tile_editor = WorldEditor::new(PathBuf::new(), &tile_xzbbox, llbbox);
                    tile_editor.set_ground(Arc::clone(&ground));
                    tile_editor.set_ground_origin(xzbbox.min_x(), xzbbox.min_z());

                    let mut tile_subway_points: Vec<(i32, i32)> = Vec::new();

                    for &elem_idx in &tile_assignments[tile_idx] {
                        let element = &elements[elem_idx];
                        process_element(
                            &mut tile_editor,
                            element,
                            args,
                            &highway_connectivity,
                            &flood_fill_cache,
                            &building_footprints,
                            &building_passages,
                            &road_mask,
                            &tile_xzbbox,
                            &suppressed_building_outlines,
                            &mut tile_subway_points,
                        );
                    }

                    (tile_idx, tile_editor.into_world(), tile_subway_points)
                })
                .collect();

            // Phase 2: merge this batch's results into the main editor (sequential).
            // batch_results is dropped after this loop, freeing memory before next batch.
            for (tile_idx, tile_world, tile_subway_pts) in batch_results {
                editor.merge_world(
                    tile_world,
                    tiles[tile_idx].min_x,
                    tiles[tile_idx].min_z,
                    tiles[tile_idx].max_x - 1,
                    tiles[tile_idx].max_z - 1,
                );
                subway_points.extend(tile_subway_pts);
            }
        }

        println!(
            "  Element processing completed in {:.1}s",
            phase_start.elapsed().as_secs_f64()
        );

        emit_gui_progress_update(70.0, "");
    } else {
        // Small area: sequential processing along the original code path.
        let elements_count: usize = elements.len();
        let process_pb: ProgressBar = ProgressBar::new(elements_count as u64);
        process_pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:45.white/black}] {pos}/{len} elements ({eta}) {msg}")
            .unwrap()
            .progress_chars("█▓░"));

        let progress_increment_prcs: f64 = 45.0 / elements_count as f64;
        let mut current_progress_prcs: f64 = 25.0;
        let mut last_emitted_progress: f64 = current_progress_prcs;
        let desired_updates: u64 = 500;
        let pb_batch_size: u64 = (elements_count as u64 / desired_updates).max(1);
        let mut element_counter: u64 = 0;

        for element in elements.into_iter() {
            element_counter += 1;
            if element_counter.is_multiple_of(pb_batch_size) {
                process_pb.inc(pb_batch_size);
            }
            current_progress_prcs += progress_increment_prcs;
            if (current_progress_prcs - last_emitted_progress).abs() > 0.25 {
                emit_gui_progress_update(current_progress_prcs, "");
                last_emitted_progress = current_progress_prcs;
            }

            if args.debug {
                process_pb.set_message(format!(
                    "(Element ID: {} / Type: {})",
                    element.id(),
                    element.kind()
                ));
            } else {
                // Clear on every non-debug iteration so any transient warning
                // message set by downstream element processing (missing nodes,
                // etc.) doesn't stick for the rest of the run.
                process_pb.set_message("");
            }

            process_element(
                &mut editor,
                &element,
                args,
                &highway_connectivity,
                &flood_fill_cache,
                &building_footprints,
                &building_passages,
                &road_mask,
                &xzbbox,
                &suppressed_building_outlines,
                &mut subway_points,
            );

            // Release flood fill cache entries for memory optimization.
            // (Skipped in the parallel path where the cache is shared immutably.)
            match &element {
                ProcessedElement::Way(way) => {
                    flood_fill_cache.remove_way(way.id);
                }
                ProcessedElement::Relation(rel) => {
                    let way_ids: Vec<u64> = rel.members.iter().map(|m| m.way.id).collect();
                    flood_fill_cache.remove_relation_ways(&way_ids);
                }
                _ => {}
            }
            // Element is dropped here, freeing its memory immediately.
        }

        process_pb.inc(element_counter % pb_batch_size);
        process_pb.finish();
    }

    // Drop remaining caches
    drop(highway_connectivity);
    drop(flood_fill_cache);
    drop(road_mask);

    // Generate ground layer (surface blocks, vegetation, shorelines, underground fill)
    let ground_gen_start = std::time::Instant::now();
    ground_generation::generate_ground_layer(
        &mut editor,
        ground.as_ref(),
        args,
        &xzbbox,
        &building_footprints,
    )?;
    println!(
        "  Ground generation completed in {:.1}s",
        ground_gen_start.elapsed().as_secs_f64()
    );

    // Carve subway tunnel interiors now that underground is filled with stone.
    // This must happen after ground generation so AIR blocks are not overwritten.
    if !subway_points.is_empty() {
        railways::carve_subway_interior(&mut editor, &subway_points);
    }

    // Save world
    let save_start = std::time::Instant::now();
    let (total_chunks, total_sections) = editor.world_stats();
    println!(
        "  Regions to save: {}, chunks: {}, sections: {}",
        editor.region_count(),
        total_chunks,
        total_sections,
    );
    if let Err(e) = editor.save() {
        return Err(e.to_string());
    }
    println!(
        "  Save completed in {:.1}s",
        save_start.elapsed().as_secs_f64()
    );

    if let Some(start) = generation_start {
        let gen_ms = start.elapsed().as_millis();
        eprintln!("[BENCHMARK] generation_time_ms={gen_ms}");
    }

    emit_gui_progress_update(99.0, "Finalizing world...");

    // Update player spawn Y coordinate based on terrain height after generation
    #[cfg(feature = "gui")]
    if world_format == WorldFormat::JavaAnvil {
        use crate::gui::update_player_spawn_y_after_generation;
        // Reconstruct bbox string to match the format that GUI originally provided.
        // This ensures LLBBox::from_str() can parse it correctly.
        let bbox_string = format!(
            "{},{},{},{}",
            args.bbox.min().lat(),
            args.bbox.min().lng(),
            args.bbox.max().lat(),
            args.bbox.max().lng()
        );

        // Always update spawn Y since we now always set a spawn point (user-selected or default)
        if let Some(ref world_path) = args.path {
            if let Err(e) = update_player_spawn_y_after_generation(
                world_path,
                bbox_string,
                args.scale,
                ground.as_ref(),
            ) {
                let warning_msg = format!("Failed to update spawn point Y coordinate: {}", e);
                eprintln!("Warning: {}", warning_msg);
                #[cfg(feature = "gui")]
                send_log(LogLevel::Warning, &warning_msg);
            }
        }
    }

    // For Bedrock format, emit event to open the mcworld file
    if world_format == WorldFormat::BedrockMcWorld {
        if let Some(path_str) = output_path.to_str() {
            emit_show_in_folder(path_str);
        }
    }

    // For Java worlds saved to the Desktop (GUI falls back there when .minecraft/saves
    // is missing), open the folder in the file explorer so the user can find the world.
    if world_format == WorldFormat::JavaAnvil {
        if let Some(desktop) = dirs::desktop_dir() {
            if output_path.starts_with(&desktop) {
                if let Some(path_str) = output_path.to_str() {
                    emit_show_in_folder(path_str);
                }
            }
        }
    }

    Ok(output_path)
}

/// Information needed to generate a map preview after world generation is complete
#[derive(Clone)]
pub struct MapPreviewInfo {
    pub world_path: PathBuf,
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    pub world_area: i64,
}

impl MapPreviewInfo {
    /// Create MapPreviewInfo from world bounds
    pub fn new(world_path: PathBuf, xzbbox: &XZBBox) -> Self {
        let world_width = (xzbbox.max_x() - xzbbox.min_x()) as i64;
        let world_height = (xzbbox.max_z() - xzbbox.min_z()) as i64;
        Self {
            world_path,
            min_x: xzbbox.min_x(),
            max_x: xzbbox.max_x(),
            min_z: xzbbox.min_z(),
            max_z: xzbbox.max_z(),
            world_area: world_width * world_height,
        }
    }
}

/// Maximum area for which map preview generation is allowed (to avoid memory issues)
pub const MAX_MAP_PREVIEW_AREA: i64 = 6400 * 6900;

/// Start map preview generation in a background thread.
/// This should be called AFTER the world generation is complete, the session lock is released,
/// and the GUI has been notified of 100% completion.
///
/// For Java worlds only, and only if the world area is within limits.
pub fn start_map_preview_generation(info: MapPreviewInfo) {
    if info.world_area > MAX_MAP_PREVIEW_AREA {
        return;
    }

    std::thread::spawn(move || {
        // Use catch_unwind to prevent any panic from affecting the application
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            map_renderer::render_world_map(
                &info.world_path,
                info.min_x,
                info.max_x,
                info.min_z,
                info.max_z,
            )
        }));

        match result {
            Ok(Ok(_path)) => {
                // Notify the GUI that the map preview is ready
                emit_map_preview_ready();
            }
            Ok(Err(e)) => {
                eprintln!("Warning: Failed to generate map preview: {}", e);
            }
            Err(_) => {
                eprintln!("Warning: Map preview generation panicked unexpectedly");
            }
        }
    });
}
