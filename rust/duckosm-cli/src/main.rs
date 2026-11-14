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
    /// The build of one tile of a large build: the junctions file of the tile cut (z_x_y.junctions), the junctions of the
    /// whole area at the nodes outside the tile.
    #[arg(long, value_name = "FILE", conflicts_with = "config")]
    outside_junctions: Option<PathBuf>,
    /// The build of one tile of a large build (duckosm-rust tiles): only the part of the boundary in this map tile.
    #[arg(long, value_name = "Z/X/Y", requires = "boundary")]
    tile: Option<String>,
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

/// duckosm-rust cut: step 0 alone. Cut a region file to a boundary and write the area as a `.osm.pbf` file
/// (osmium's smart cut). Only lists of ids stay in memory.
#[derive(Parser)]
#[command(name = "duckosm-rust cut", version)]
struct CutCli {
    /// The region file (.osm.pbf, sorted by type and id, as the files of Geofabrik).
    region: PathBuf,
    /// The boundary (GeoJSON).
    #[arg(long)]
    boundary: PathBuf,
    /// The area file to write (.osm.pbf).
    #[arg(short, long)]
    out: PathBuf,
}

fn cut(args: &[String]) -> Result<(), String> {
    let cli = CutCli::parse_from(args);
    let text = std::fs::read_to_string(&cli.boundary).map_err(|e| format!("{}: {e}", cli.boundary.display()))?;
    let t = std::time::Instant::now();
    let n = duckosm_build::cut_file(&cli.region, &text, &cli.out)?;
    eprintln!(
        "cut: {} nodes, {} ways, {} relations in {:.2} s",
        n.nodes,
        n.ways,
        n.relations,
        t.elapsed().as_secs_f64()
    );
    #[cfg(feature = "heap-probe")]
    heap::report("cut");
    Ok(())
}

/// duckosm-rust quack: the features table of QuackOSM (feature_id, tags, geometry) from an .osm.pbf file, with the rows of
/// QuackOSM 0.20.0 with its default options (docs/rust/quack.md). To make the table of an area, cut the region file first
/// (duckosm-rust cut).
#[derive(Parser)]
#[command(name = "duckosm-rust quack", version)]
struct QuackCli {
    /// The input .osm.pbf file.
    pbf: PathBuf,
    /// The output folder.
    #[arg(short, long, default_value = ".")]
    out: PathBuf,
    /// The name of the area: the files are <out>/<name>_quack.duckdb and <out>/<name>_quack.parquet.
    #[arg(long, default_value = "area")]
    name: String,
    /// Output formats, comma separated: geoparquet, duckdb (the table main.quackosm).
    #[arg(short, long, default_value = "duckdb,geoparquet")]
    format: String,
    /// The threads (default: all the cores), as QuackOSM's cpu_limit: the pool of the Rust threads, the blocks of the file
    /// that are decoded at the same time, and the threads of DuckDB.
    #[arg(long)]
    threads: Option<usize>,
    /// Do not sort the rows (QuackOSM sort_result=False). The default sorts them as QuackOSM does (Sort-Tile-Recursive
    /// on the centers of the geometries), so each row group holds a small area; the rows then wait in memory.
    #[arg(long)]
    no_sort: bool,
}

fn quack(args: &[String]) -> Result<(), String> {
    let cli = QuackCli::parse_from(args);
    let (mut duckdb, mut geoparquet) = (false, false);
    for f in cli.format.split(',') {
        match f.trim() {
            "duckdb" => duckdb = true,
            "geoparquet" => geoparquet = true,
            other => return Err(format!("unknown format {other}: use geoparquet, duckdb or both")),
        }
    }
    std::fs::create_dir_all(&cli.out).map_err(|e| format!("{}: {e}", cli.out.display()))?;
    let parquet = geoparquet.then(|| cli.out.join(format!("{}_quack.parquet", cli.name)));
    let db = duckdb.then(|| cli.out.join(format!("{}_quack.duckdb", cli.name)));
    let threads = cli.threads.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let n = duckosm_build::quack(&cli.pbf, parquet.as_deref(), db.as_deref(), threads, !cli.no_sort)?;
    eprintln!(
        "quack: {} nodes, {} ways, {} relations in {:.2} s",
        n.nodes,
        n.ways,
        n.relations,
        t.elapsed().as_secs_f64()
    );
    for p in db.iter().chain(parquet.iter()) {
        eprintln!("  {}", p.display());
    }
    Ok(())
}

/// duckosm-rust tiles: build a large area by tiles. One read of the region file cuts it to the standard map tiles of the
/// boundary (one file for each tile, with the junctions of the nodes outside the tile); each tile builds alone, `--jobs`
/// tiles at a time, each in its own process; the union joins the tables of the tiles into the tables of the area.
/// Not yet in a build by tiles: the steps that need the whole area (the bus routes, the drawing levels, the filter of the
/// largest part), and the .duckdb output.
#[derive(Parser)]
#[command(name = "duckosm-rust tiles", version)]
struct TilesCli {
    /// The region file (.osm.pbf, sorted by type and id, as the files of Geofabrik).
    region: PathBuf,
    /// The boundary of the area (GeoJSON).
    #[arg(long)]
    boundary: PathBuf,
    /// The output folder: the GeoParquet files of the area.
    #[arg(short, long)]
    out: PathBuf,
    /// The zoom of the tiles (standard z/x/y map tiles; zoom 10 is about 27 km wide in France).
    #[arg(long, default_value_t = 10)]
    zoom: u8,
    /// The memory for the tile builds at the same time, for example 4G or 2500M. A tile starts only when the estimates of
    /// the running tiles and its own stay below it. More memory does not make the build faster (France: 2 GB and 8 GB took
    /// the same time, docs/design/build_by_tiles.md).
    #[arg(long, value_name = "SIZE", default_value = "2G")]
    memory: String,
    /// The most tiles that build at the same time (default: the cores divided by --tile-threads).
    #[arg(long)]
    jobs: Option<usize>,
    /// The threads of each tile build. France on 12 cores: 6 tiles of 2 threads was the fastest
    /// (docs/design/build_by_tiles.md).
    #[arg(long, default_value_t = 2)]
    tile_threads: usize,
    /// Keep the tile files and the tables of each tile (in <out>/tiles).
    #[arg(long)]
    keep_tiles: bool,
    /// Only the cut to the tiles (the command runs the cut as its own process with this option, so the memory of the cut
    /// goes back to the system before the tile builds).
    #[arg(long, hide = true)]
    only_cut: bool,
    /// The options of the build of each tile, after `--` (as for `duckosm-rust FILE`, for example `-- --modes driving --raw`).
    #[arg(last = true)]
    build: Vec<String>,
}

fn tiles(args: &[String]) -> Result<(), String> {
    let cli = TilesCli::parse_from(args);
    let text = std::fs::read_to_string(&cli.boundary).map_err(|e| format!("{}: {e}", cli.boundary.display()))?;
    // the options of the tiles, as the build reads them (the union needs the modes and the options)
    // the global steps do not run in a tile: the bus routes, the drawing levels and the largest part need the whole area
    let global = ["--no-bus-routes", "--no-component-filter"];
    let mut opts: Vec<String> = cli.build.iter().filter(|a| !global.contains(&a.as_str())).cloned().collect();
    if let Some(i) = opts.iter().position(|a| a == "--levels") {
        opts.drain(i..(i + 2).min(opts.len()));
    }
    let mut all = vec!["duckosm-rust".to_string(), "tile.osm.pbf".to_string(), "--boundary".to_string()];
    all.push(cli.boundary.to_string_lossy().into_owned());
    all.extend(opts.iter().cloned());
    let (_, _, p) = params(&Cli::try_parse_from(&all).map_err(|e| e.to_string())?)?;
    let t0 = std::time::Instant::now();
    let dir = cli.out.join("tiles");
    if cli.only_cut {
        // the multipolygons of each tile are complete only when a part of the build reads them
        let multipolygons = p.raw || p.features || p.building_faces;
        duckosm_build::union::cut_tiles(&cli.region, &text, cli.zoom, multipolygons, &dir)?;
        return Ok(());
    }
    // the cut, as a process of this program: its memory goes back to the system when it ends
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let ok = std::process::Command::new(&exe)
        .arg("tiles")
        .arg(&cli.region)
        .arg("--boundary")
        .arg(&cli.boundary)
        .arg("-o")
        .arg(&cli.out)
        .args(["--zoom", &cli.zoom.to_string(), "--only-cut", "--"])
        .args(&cli.build)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return Err("the cut to the tiles failed".into());
    }
    // the tiles with a file
    let mut list: Vec<duckosm_build::Tile> = Vec::new();
    for e in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let name = e.map_err(|e| e.to_string())?.file_name().to_string_lossy().into_owned();
        if let Some(stem) = name.strip_suffix(".osm.pbf") {
            let n: Vec<u32> = stem.split('_').filter_map(|x| x.parse().ok()).collect();
            if let [z, x, y] = n[..] {
                list.push(duckosm_build::Tile { z: z as u8, x, y });
            }
        }
    }
    list.sort_by_key(|t| (std::cmp::Reverse(t.y), t.x));
    eprintln!("tiles: cut to {} tiles in {:.1} s", list.len(), t0.elapsed().as_secs_f64());

    // the tile builds, each in a process of this program (docs/design/build_by_tiles.md, "The scheduler")
    let t1 = std::time::Instant::now();
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let threads = cli.tile_threads.max(1);
    let jobs = cli.jobs.unwrap_or(cores / threads).max(1);
    let limit = parse_size(&cli.memory)?;
    let build = |t: &duckosm_build::Tile| -> Result<std::process::Child, String> {
        let name = format!("{}_{}_{}", t.z, t.x, t.y);
        let log = std::fs::File::create(dir.join(format!("{name}.log"))).map_err(|e| e.to_string())?;
        std::process::Command::new(&exe)
            .arg(dir.join(format!("{name}.osm.pbf")))
            .args(["--boundary".as_ref(), cli.boundary.as_os_str()])
            .args(["--tile", &t.name(), "--format", "geoparquet", "--levels", "none"])
            .args(global)
            .arg("--outside-junctions")
            .arg(dir.join(format!("{name}.junctions")))
            .arg("-o")
            .arg(dir.join(&name))
            .args(&opts)
            .env("RAYON_NUM_THREADS", threads.to_string())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| e.to_string())
    };
    let failed = std::sync::Mutex::new(Vec::new());
    let (peak, ratio) = schedule(&list, &dir, limit, jobs, &build, |t, ok| {
        let name = format!("{}_{}_{}", t.z, t.x, t.y);
        if !ok {
            failed.lock().unwrap().push(name);
        } else if !cli.keep_tiles {
            // the tile file and its junctions are read: they go at once (the disk keeps less at one time)
            let _ = std::fs::remove_file(dir.join(format!("{name}.osm.pbf")));
            let _ = std::fs::remove_file(dir.join(format!("{name}.junctions")));
        }
    })?;
    eprintln!(
        "tiles: memory {} MB, at most {jobs} tiles of {threads} threads; the largest tile process {} MB; memory of a tile: {:.1} times its file and more",
        limit >> 20,
        peak >> 20,
        ratio
    );
    let failed = failed.into_inner().unwrap();
    if !failed.is_empty() {
        return Err(format!("the build of {} tiles failed (see {}/<tile>.log): {}", failed.len(), dir.display(), failed.join(", ")));
    }
    eprintln!("tiles: {} tile builds in {:.1} s", list.len(), t1.elapsed().as_secs_f64());

    let t2 = std::time::Instant::now();
    let built: Vec<(duckosm_build::Tile, PathBuf)> = list.iter().map(|t| (*t, dir.join(format!("{}_{}_{}", t.z, t.x, t.y)))).collect();
    for (name, rows) in duckosm_build::union::union_all(&built, &text, &p, !cli.keep_tiles, &cli.out)? {
        if name.starts_with("edges_") || name.starts_with("nodes_") || name.starts_with("edge_graph_") {
            eprintln!("tiles: {name}: {rows} rows");
        }
    }
    eprintln!("tiles: union in {:.1} s; total {:.1} s", t2.elapsed().as_secs_f64(), t0.elapsed().as_secs_f64());
    if !cli.keep_tiles {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    Ok(())
}

/// A size: a number of bytes, or with K, M, G (1024 based), for example 4G.
fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim().to_ascii_uppercase();
    let (n, m) = match t.chars().last() {
        Some('K') => (&t[..t.len() - 1], 1u64 << 10),
        Some('M') => (&t[..t.len() - 1], 1 << 20),
        Some('G') => (&t[..t.len() - 1], 1 << 30),
        _ => (&t[..], 1),
    };
    n.trim().parse::<f64>().map(|v| (v * m as f64) as u64).map_err(|_| format!("{s}: not a size (for example 4G)"))
}

/// Wait for a child process: whether it ended well, and its peak memory in bytes (0 when it is not known).
fn wait_peak(mut child: std::process::Child) -> (bool, u64) {
    #[cfg(unix)]
    {
        let mut status = 0;
        // SAFETY: rusage is plain data; wait4 waits for this child only (it is not waited for anywhere else)
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::wait4(child.id() as libc::pid_t, &mut status, 0, &mut usage) };
        if r > 0 {
            let ok = libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0;
            // ru_maxrss: bytes on macOS, kilobytes on Linux
            let unit = if cfg!(target_os = "macos") { 1 } else { 1024 };
            return (ok, usage.ru_maxrss as u64 * unit);
        }
    }
    (child.wait().is_ok_and(|s| s.success()), 0)
}

/// Run the builds of `tiles` (`start` starts one) at most `jobs` at a time, largest tile file first, while the estimates of
/// the running tiles stay below `limit` bytes; a tile that is larger than the limit runs alone. The estimate of a tile is
/// a line of its file size, base + factor × size, plus a margin: the line fits the peaks of the finished tiles (least
/// squares), and the margin is the largest peak above the line. Before 8 tiles have ended: 100 MB + 12 × size (France,
/// driving: 38 MB + 10.5 × size, 70 MB margin; Kentucky with raw.*: 73 MB + 22.7 × size). `done` gets each tile when it
/// ends. Returns the largest peak and the factor of the line.
fn schedule(
    tiles: &[duckosm_build::Tile],
    dir: &std::path::Path,
    limit: u64,
    jobs: usize,
    start: &(dyn Fn(&duckosm_build::Tile) -> Result<std::process::Child, String> + Sync),
    mut done: impl FnMut(&duckosm_build::Tile, bool),
) -> Result<(u64, f64), String> {
    let size = |t: &duckosm_build::Tile| {
        std::fs::metadata(dir.join(format!("{}_{}_{}.osm.pbf", t.z, t.x, t.y))).map_or(0, |m| m.len())
    };
    let mut all: Vec<(duckosm_build::Tile, u64)> = tiles.iter().map(|t| (*t, size(t))).collect();
    all.sort_by_key(|q| std::cmp::Reverse(q.1)); // the largest first
    // the line (base, factor) and the margin, in bytes; the finished tiles (size, peak)
    let (mut line, mut peak) = ((100.0 * 1048576.0, 12.0, 0.0), 0u64);
    let mut seen: Vec<(f64, f64)> = Vec::new();
    let estimate = |s: u64, l: (f64, f64, f64)| (l.0 + l.1 * s as f64 + l.2).max(0.0) as u64;
    let (tx, rx) = std::sync::mpsc::channel::<(usize, bool, u64)>();
    let mut running: Vec<(usize, u64)> = Vec::new(); // (place in `all`, estimate)
    let mut next: Vec<usize> = (0..all.len()).collect();
    let mut log = String::new();
    std::thread::scope(|sc| -> Result<(), String> {
        loop {
            // start the largest tiles that fit
            while running.len() < jobs {
                let used: u64 = running.iter().map(|r| r.1).sum();
                let fit = next.iter().position(|&k| running.is_empty() || used + estimate(all[k].1, line) <= limit);
                let Some(i) = fit else { break };
                let k = next.remove(i);
                let child = start(&all[k].0)?;
                running.push((k, estimate(all[k].1, line)));
                let tx = tx.clone();
                sc.spawn(move || {
                    let (ok, p) = wait_peak(child);
                    let _ = tx.send((k, ok, p));
                });
            }
            if running.is_empty() {
                break;
            }
            let (k, ok, p) = rx.recv().map_err(|e| e.to_string())?;
            running.retain(|r| r.0 != k);
            peak = peak.max(p);
            if p > 0 {
                seen.push((all[k].1 as f64, p as f64));
                if let Some(l) = fit_line(&seen) {
                    line = l;
                }
            }
            log += &format!("{} {} {} {}\n", all[k].0.name(), all[k].1, p, ok);
            done(&all[k].0, ok);
        }
        Ok(())
    })?;
    // each tile: its name, file size, peak memory (bytes) and whether it ended well; next to the folder of the tiles, which goes
    let _ = std::fs::write(dir.with_file_name("tiles.log"), log);
    Ok((peak, line.1))
}

/// The line base + factor × size through the points (size, peak) by least squares, and the largest peak above it; none
/// before 8 points, or when all sizes are equal.
fn fit_line(pts: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
    let n = pts.len() as f64;
    if pts.len() < 8 {
        return None;
    }
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (sxx, sxy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0 * p.0, a.1 + p.0 * p.1));
    let d = n * sxx - sx * sx;
    if d <= 0.0 {
        return None;
    }
    let factor = ((n * sxy - sx * sy) / d).max(0.0);
    let base = (sy - factor * sx) / n;
    let margin = pts.iter().map(|p| p.1 - (base + factor * p.0)).fold(0.0, f64::max);
    Some((base, factor, margin))
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

/// The live and the peak Rust heap after each step of a build (cargo feature `heap-probe`, for measurements).
#[cfg(feature = "heap-probe")]
mod heap {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    struct Counting;
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let p = unsafe { System.alloc(l) };
            if !p.is_null() {
                PEAK.fetch_max(LIVE.fetch_add(l.size(), Relaxed) + l.size(), Relaxed);
            }
            p
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) };
            LIVE.fetch_sub(l.size(), Relaxed);
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            let q = unsafe { System.realloc(p, l, new) };
            if !q.is_null() {
                if new >= l.size() {
                    PEAK.fetch_max(LIVE.fetch_add(new - l.size(), Relaxed) + new - l.size(), Relaxed);
                } else {
                    LIVE.fetch_sub(l.size() - new, Relaxed);
                }
            }
            q
        }
    }

    #[global_allocator]
    static A: Counting = Counting;

    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static LAST_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Print the live heap after the step `name`, its peak during the step and the time of the step, then start a new
    /// peak. A network sub-step reports at its start: its line has the peak and the time of the part before it.
    pub fn report(name: &str) {
        let mib = |b: usize| b as f64 / 1048576.0;
        let live = LIVE.load(Relaxed);
        let now = START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_nanos() as u64;
        let secs = (now - LAST_NS.swap(now, Relaxed)) as f64 / 1e9;
        eprintln!(
            "  heap {name:<30} live {:>8.1} MiB   peak {:>8.1} MiB   {secs:>6.3} s",
            mib(live),
            mib(PEAK.load(Relaxed))
        );
        PEAK.store(live, Relaxed);
    }
}

fn main() {
    #[cfg(feature = "heap-probe")]
    heap::report("start");
    #[cfg(feature = "heap-probe")]
    duckosm_build::set_progress(Some(Box::new(heap::report)));
    #[cfg(feature = "heap-probe")]
    let _ = duckosm_build::STEP_HOOK.set(heap::report);
    let args: Vec<String> = std::env::args().collect();
    // `duckosm-rust edit ...`: its own options (the build's first argument is a PBF file)
    let r = match args.get(1).map(String::as_str) {
        Some("edit") => edit(&args[1..]),
        Some("studio") => studio(&args[1..]),
        Some("cut") => cut(&args[1..]),
        Some("tiles") => tiles(&args[1..]),
        Some("quack") => quack(&args[1..]),
        _ => run(),
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let (pbf, out, p) = params(&cli)?;
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

/// The input file, the output folder and the parameters of a build from the command line.
fn params(cli: &Cli) -> Result<(PathBuf, PathBuf, Params), String> {
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
        p.outside_junctions = cli.outside_junctions.clone();
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
    if let Some(t) = &cli.tile {
        let n: Vec<u32> = t.split('/').map(|x| x.parse().map_err(|_| format!("--tile {t}: not z/x/y"))).collect::<Result<_, _>>()?;
        let [z, x, y] = n[..] else {
            return Err(format!("--tile {t}: not z/x/y"));
        };
        p.tile = Some(duckosm_build::Tile { z: z as u8, x, y });
    }
    Ok((pbf, out, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_of_the_tiles_and_the_sizes() {
        // points on 40 + 10 x, one of them 7 above the line
        let mut pts: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, 40.0 + 10.0 * i as f64)).collect();
        pts[3].1 += 7.0;
        let (base, factor, margin) = fit_line(&pts).unwrap();
        assert!((factor - 10.0).abs() < 0.5 && (base - 40.0).abs() < 2.0 && margin >= 6.0, "{base} {factor} {margin}");
        assert_eq!(fit_line(&pts[..7]), None);
        assert_eq!(parse_size("4G").unwrap(), 4 << 30);
        assert_eq!(parse_size("2500m").unwrap(), 2500 << 20);
        assert!(parse_size("lots").is_err());
    }
}
