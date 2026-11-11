use clap::Parser;
use duckosm_build::{build, config, parse_levels, NetworkMode, NetworkOptions, Params};
use std::path::PathBuf;

/// duckosm-rust: build the road networks of duckOSM (driving, walking, cycling) from an .osm.pbf file.
#[derive(Parser)]
#[command(name = "duckosm-rust", version, after_help = "To edit the OSM data of a database on a page: duckosm-rust edit <AREA>.duckdb (duckosm-rust edit --help). To make the data of an area on a page: duckosm-rust studio (duckosm-rust studio --help)")]
struct Cli {
    /// The input .osm.pbf file (already cut to the area). Not with --config.
    #[arg(required_unless_present = "config", conflicts_with = "config")]
    pbf: Option<PathBuf>,
    /// A duckOSM config file (YAML). It gives the input, the output, the modes and the options.
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// With --config: the PBF is already cut to the boundary of the config (this program does not cut).
    #[arg(long, requires = "config")]
    pbf_is_cut: bool,
    /// With --config: go on when the config asks for a duckOSM step that this program does not have.
    #[arg(long, requires = "config")]
    allow_skipped: bool,
    /// The output folder (default: the current folder, or output_path of the config).
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// The name of the area: the .duckdb file is <out>/<name>.duckdb.
    #[arg(long, default_value = "area", conflicts_with = "config")]
    name: String,
    /// Output formats, comma separated: duckdb, geoparquet. Default: both, or geoparquet when this
    /// program is built without the cargo feature "duckdb".
    #[arg(short, long)]
    format: Option<String>,
    /// Modes, comma separated: driving, walking, cycling.
    #[arg(short, long, default_value = "driving", conflicts_with = "config")]
    modes: String,
    /// Cycling without the footways that are walked (dismount).
    #[arg(long, conflicts_with = "config")]
    no_cycling_dismount: bool,
    /// Cut each mode at its own junctions (duckOSM option global_junctions: false).
    #[arg(long, conflicts_with = "config")]
    no_global_junctions: bool,
    /// Do not infer sidewalk=separate on roads that have a sidewalk line on both sides (duckOSM option infer_separate_sidewalks).
    #[arg(long, conflicts_with = "config")]
    no_sidewalk_inference: bool,
    /// Do not connect the dangling path ends to the network (duckOSM option connectivity_rescue: false).
    #[arg(long, conflicts_with = "config")]
    no_connect_paths: bool,
    /// The distance in metres within which a dangling path end is connected.
    #[arg(long, default_value_t = 10.0, conflicts_with = "config")]
    snap_m: f64,
    /// Do not compute maxspeed_kmh (the raw maxspeed text stays; duckOSM option process_speeds: false).
    #[arg(long, conflicts_with = "config")]
    no_speeds: bool,
    /// Do not compute cost_s, the travel time (duckOSM option calculate_costs: false).
    #[arg(long, conflicts_with = "config")]
    no_costs: bool,
    /// Do not add walk_type and cycle_type (duckOSM option functional_types: false).
    #[arg(long, conflicts_with = "config")]
    no_functional_types: bool,
    /// Do not join the chains of edges that are one road (duckOSM option merge_segments).
    #[arg(long, conflicts_with = "config")]
    no_merge_segments: bool,
    /// Do not keep only the largest connected part of the graph.
    #[arg(long, conflicts_with = "config")]
    no_component_filter: bool,
    /// Do not build edge_graph and turn_permission (duckOSM option build_graph: false).
    #[arg(long, conflicts_with = "config")]
    no_graph: bool,
    /// Do not build turn_restrictions and turn_path_restrictions (duckOSM option extract_restrictions: false).
    #[arg(long, conflicts_with = "config")]
    no_restrictions: bool,
    /// Do not write ways and way_nodes (duckOSM writes them always; off saves memory in a large area).
    #[arg(long)]
    no_ways_tables: bool,
    /// Do not make the bus routes, bus.routes and bus.route_edges (duckOSM option bus_routes: false). They need
    /// --boundary and the driving mode.
    #[arg(long, conflicts_with = "config")]
    no_bus_routes: bool,
    /// A duckOSM OSM fixes file (YAML). With --config, it replaces the config's osm_overrides.
    #[arg(long, value_name = "FILE")]
    fixes: Option<PathBuf>,
    /// A boundary (GeoJSON, as `duckosm boundary` writes it): the PBF is cut to it (as osmium complete_ways).
    #[arg(long, value_name = "FILE", conflicts_with = "config")]
    boundary: Option<PathBuf>,
    /// With --boundary: also write duckOSM's raw.* of the area (raw_nodes.parquet, raw_ways.parquet,
    /// raw_relations.parquet, as osmium's smart cut), the input of the base-map layers (docs/features.md).
    #[arg(long, requires = "boundary")]
    raw: bool,
    /// With --boundary and the .duckdb output: make the base-map layers, features.* (water, land, buildings, ...), and keep raw.* in the
    /// file, as duckOSM does (docs/reference/features.md). duckOSM's SQL runs on DuckDB's spatial extension (the first run downloads it).
    #[arg(long, requires = "boundary")]
    features: bool,
    /// With --features: also make features.ocean from Overture Maps (reads the network; a failure is a warning).
    #[arg(long, requires = "features")]
    sea: bool,
    /// Add duckOSM's H3 columns (nodes.h3_cell, edges.from_cell, to_cell, lca_res) at this resolution (0 to 15; duckOSM's default is 8).
    #[arg(long, value_name = "RES", conflicts_with = "config")]
    h3_resolution: Option<u8>,
    /// With --boundary: also write the cut area as cut.osm.pbf (the nodes, ways and relations of osmium's smart cut,
    /// before the OSM fixes), to read in other tools or to use as the cut file of the browser page.
    #[arg(long, requires = "boundary")]
    cut_pbf: bool,
    /// With --boundary: also write the cut area as cut.osm (XML).
    #[arg(long, requires = "boundary")]
    cut_osm: bool,
    /// With --boundary: also write main.building_faces (building_faces_main.parquet): the walls of the buildings, each with a type
    /// (front, back, side, party), from the streets of the driving and walking modes (docs/osm-editing.md). Not in duckOSM.
    #[arg(long, requires = "boundary")]
    building_faces: bool,
    /// The drawing order of the roads (visualization.edge_levels, docs/levels.md): tags (a rule on layer,
    /// bridge and tunnel), solve (roadstyle's optimization) or none. Default: tags, or the config's
    /// options.levels (on: solve).
    #[arg(long, value_name = "METHOD")]
    levels: Option<String>,
    /// A DEM (GeoTIFF in EPSG:4326, 32-bit floats): sample it at every node and at every vertex of every edge
    /// (duckOSM `duckosm elevation --dem`). Writes nodes.ele, edges.z_from, z_to and z_vertices.
    #[arg(long, value_name = "FILE")]
    dem: Option<PathBuf>,
    /// The product of the --dem file: DTM (bare earth) or DSM (surface), for main.elevation_metadata. Necessary when the
    /// DEM is the area's reference ground.
    #[arg(long, value_name = "DTM|DSM", requires = "dem")]
    dem_product: Option<String>,
    /// The vertical datum of the --dem file (for example EGM2008), for main.elevation_metadata. Necessary when the
    /// DEM is the area's reference ground (docs/concepts/ground-reference.md).
    #[arg(long, value_name = "DATUM", requires = "dem")]
    dem_vertical_datum: Option<String>,
    /// The license of the --dem file, for main.elevation_metadata. Necessary when the DEM is the area's reference ground.
    #[arg(long, value_name = "TEXT", requires = "dem")]
    dem_license: Option<String>,
    /// The --dem file is not the area's reference ground (a test or a second surface): its product, its vertical datum
    /// and its license can stay unknown.
    #[arg(long, requires = "dem")]
    dem_not_reference: bool,
    /// Print the time of each step.
    #[arg(long)]
    timing: bool,
}

/// duckosm-rust edit: a page on 127.0.0.1 that edits the OSM data of a database (docs/guides/edit-osm-data.md, "Edit on your computer").
/// Each change goes into the fixes file at once. Build again builds the database again with the file.
#[derive(Parser)]
#[command(name = "duckosm-rust edit", version)]
struct EditCli {
    /// The database (<area>.duckdb) that duckosm build or duckosm-rust wrote.
    db: PathBuf,
    /// The fixes file (default: osm_fixes.yaml beside the database). It need not exist: the first change makes it.
    #[arg(long)]
    fixes: Option<PathBuf>,
    /// The port of the page on 127.0.0.1.
    #[arg(long, default_value_t = 8790)]
    port: u16,
}

/// duckosm-rust studio: a page on 127.0.0.1 that makes the data of an area, step by step (docs/guides/studio.md): the area, the OSM data, the settings,
/// the ground (DEM), the build and the result. The build is the native engine. Each area is made in `<OUT>/<name>/`.
#[derive(Parser)]
#[command(name = "duckosm-rust studio", version)]
struct StudioCli {
    /// The folder of the areas (default: the current folder).
    out: Option<PathBuf>,
    /// The port of the page on 127.0.0.1.
    #[arg(long, default_value_t = 8792)]
    port: u16,
    /// The list of the Geofabrik extracts (a URL or a file). Default: https://download.geofabrik.de/index-v1.json.
    #[arg(long, value_name = "URL_OR_FILE")]
    geofabrik_index: Option<String>,
    /// Do not open the page in the browser.
    #[arg(long)]
    no_open: bool,
}

#[cfg(feature = "duckdb")]
fn studio(args: &[String]) -> Result<(), String> {
    let cli = StudioCli::parse_from(args);
    let out = cli.out.unwrap_or_else(|| PathBuf::from("."));
    let listener = duckosm_edit::bind(cli.port)?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let site = duckosm_edit::studio::Studio::new(Box::new(duckosm_edit::native::NativeHost), &out, port, cli.geofabrik_index)?;
    eprintln!("duckosm-rust studio: {}  (the areas are in {}; Ctrl+C stops)", site.url(), site.out().display());
    if !cli.no_open {
        // the browser of the desktop; on a computer with no desktop (an ssh session) nothing opens, and the address above is the way
        let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
        let _ = std::process::Command::new(opener).arg(site.url()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    }
    duckosm_edit::studio::serve(std::sync::Arc::new(site), listener);
    Ok(())
}

#[cfg(not(feature = "duckdb"))]
fn studio(_: &[String]) -> Result<(), String> {
    Err("studio needs DuckDB: this program is built without the cargo feature \"duckdb\"".into())
}

#[cfg(feature = "duckdb")]
fn edit(args: &[String]) -> Result<(), String> {
    let cli = EditCli::parse_from(args);
    let db = std::fs::canonicalize(&cli.db).map_err(|e| format!("{}: {e}", cli.db.display()))?;
    let fixes = match cli.fixes {
        Some(f) => std::path::absolute(&f).map_err(|e| format!("{}: {e}", f.display()))?,
        None => db.with_file_name("osm_fixes.yaml"),
    };
    let listener = duckosm_edit::bind(cli.port)?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let site = duckosm_edit::Site::new(Box::new(duckosm_edit::native::NativeDb::new(&db)), &db, &fixes, port)?;
    eprintln!("duckosm-rust edit: {}  (the fixes file: {}; Ctrl+C stops)", site.url(), fixes.display());
    duckosm_edit::serve(std::sync::Arc::new(site), listener);
    Ok(())
}

#[cfg(not(feature = "duckdb"))]
fn edit(_: &[String]) -> Result<(), String> {
    Err("edit needs DuckDB: this program is built without the cargo feature \"duckdb\"".into())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `duckosm-rust edit ...`: its own options (the build's first argument is a PBF file)
    let r = match args.get(1).map(String::as_str) {
        Some("edit") => edit(&args[1..]),
        Some("studio") => studio(&args[1..]),
        _ => run(),
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let (pbf, out, mut p) = if let Some(cfg) = &cli.config {
        let c = config::read(cfg, cli.pbf_is_cut, !cli.allow_skipped)?;
        for s in &c.skipped {
            eprintln!("skipped: {s}");
        }
        for s in &c.ignored {
            eprintln!("ignored: {s}");
        }
        (c.pbf, cli.out.clone().unwrap_or(c.out), c.params)
    } else {
        let mut p = Params {
            name: cli.name.clone(),
            ..Params::default()
        };
        p.modes = NetworkMode::parse_all(&cli.modes.split(',').collect::<Vec<_>>())?;
        p.infer_sidewalks = !cli.no_sidewalk_inference;
        p.bus_routes = !cli.no_bus_routes;
        p.options = NetworkOptions {
            keep_largest_component: !cli.no_component_filter,
            cycling_dismount: !cli.no_cycling_dismount,
            merge_segments: !cli.no_merge_segments,
            global_junctions: !cli.no_global_junctions,
            connect_paths: !cli.no_connect_paths,
            snap_m: cli.snap_m,
            process_speeds: !cli.no_speeds,
            calculate_costs: !cli.no_speeds && !cli.no_costs,
            functional_types: !cli.no_functional_types,
            build_graph: !cli.no_graph,
            extract_restrictions: !cli.no_restrictions,
            ways_tables: !cli.no_ways_tables,
        };
        (
            cli.pbf.clone().unwrap(),
            cli.out.clone().unwrap_or_else(|| PathBuf::from(".")),
            p,
        )
    };
    if let Some(m) = &cli.levels {
        p.levels = parse_levels(m)?;
    }
    // --dem wins over the config's elevation section, with its own datum and license
    if cli.dem.is_some() {
        p.dem = cli.dem.clone();
        p.dem_product = cli.dem_product.clone();
        p.dem_vertical_datum = cli.dem_vertical_datum.clone();
        p.dem_license = cli.dem_license.clone();
        p.dem_not_reference = cli.dem_not_reference;
    }
    if let Some(f) = &cli.fixes {
        p.osm_fixes = Some(f.clone());
    }
    if cli.boundary.is_some() {
        p.boundary = cli.boundary.clone();
        p.raw = cli.raw;
        p.cut_pbf = cli.cut_pbf;
        p.cut_osm = cli.cut_osm;
        p.building_faces = cli.building_faces;
        p.features = cli.features;
        p.sea = cli.sea;
    }
    if cli.config.is_none() {
        p.h3_resolution = cli.h3_resolution;
    }
    if let Some(f) = &cli.format {
        p.set_formats(&f.split(',').collect::<Vec<_>>())?;
    }
    let sum = build(&pbf, &out, &p)?;
    // the build's record in the database, for duckosm-rust edit (Build again): the config file and the folder of its relative paths
    #[cfg(feature = "duckdb")]
    if let (Some(cfg), true) = (&cli.config, p.duckdb) {
        let text = std::fs::read_to_string(cfg).map_err(|e| format!("{}: {e}", cfg.display()))?;
        let dir = std::env::current_dir().map_err(|e| e.to_string())?;
        duckosm_edit::native::record(&out.join(format!("{}.duckdb", p.name)), &text, &dir)?;
    }
    for l in sum.lines() {
        eprintln!("{l}");
    }
    if cli.timing {
        for (what, secs) in &sum.steps {
            eprintln!("  {what}: {secs:.3} s");
        }
        eprintln!(
            "  total: {:.3} s",
            sum.steps.iter().map(|s| s.1).sum::<f64>()
        );
    }
    Ok(())
}
