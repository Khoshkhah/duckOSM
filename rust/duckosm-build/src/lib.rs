//! The whole build, shared by the CLI, the Python module and the browser (duckosm-wasm): read a PBF, build the
//! networks of the modes, write GeoParquet and/or a `.duckdb` file.

use duckosm_core::boundary::Boundary;
use duckosm_core::filter::Mode;
use duckosm_core::fixes::OsmFixes;
use duckosm_core::graph::Relation;
use duckosm_core::lanes::LaneEdges;
use duckosm_core::model::{Edge, Network, Way};
use duckosm_core::network::{build_network_files, Coords, EdgeFiles as NetworkEdges, NetworkFiles, Options};
use duckosm_core::osmjson::OsmJson;
use duckosm_core::raw::{Raw, RawPatch, Want};
use duckosm_core::{building_faces, cutfile, pbf, sidewalks};
use sha2::{Digest, Sha256};
use levels::LevelEdges;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub mod area;
pub mod config;
pub mod elevation;
pub mod h3;
pub mod levels;
pub mod union;

pub use area::{write_area_tables, write_lane_profile, write_levels, Area};

pub use duckosm_core::filter::Mode as NetworkMode;
pub use duckosm_core::network::Options as NetworkOptions;
/// The hook of the sub-steps of the network step (the memory probe).
pub use duckosm_core::util::STEP_HOOK;
pub use duckosm_core::tiles::Tile;
pub use levels::Method as LevelsMethod;

/// What to build and write.
#[derive(Clone, Debug)]
pub struct Params {
    pub modes: Vec<Mode>,
    /// Write `edges_<mode>.parquet`, `nodes_<mode>.parquet`, ... in the output folder.
    pub geoparquet: bool,
    /// Write `<name>.duckdb` in the output folder (needs the cargo feature `duckdb`).
    pub duckdb: bool,
    /// The name of the area: the file is `<out>/<name>.duckdb`.
    pub name: String,
    pub options: Options,
    /// Make `bus.routes` and `bus.route_edges` (duckOSM `options.bus_routes`, on by default; docs/bus.md). They need
    /// the driving mode and a boundary (the raw cut has the route relations).
    pub bus_routes: bool,
    /// The drawing order of the roads, `visualization.edge_levels` (docs/levels.md): the tags rule (the
    /// default), roadstyle's solver, or no table.
    pub levels: LevelsMethod,
    /// Run the sidewalk rule when the walking mode is built (duckOSM `infer_separate_sidewalks`).
    pub infer_sidewalks: bool,
    /// A duckOSM OSM fixes file (duckOSM `osm_overrides`).
    pub osm_fixes: Option<PathBuf>,
    /// A boundary (GeoJSON): the PBF is cut to it as osmium's `complete_ways` does. None: the PBF is the area.
    pub boundary: Option<PathBuf>,
    /// The build of one tile of a large build: the junctions file of the tile cut (`tilecut`), the junctions of the whole
    /// area at the nodes outside the tile. The tile does not have all ways of those nodes, so it can miss these junctions.
    pub outside_junctions: Option<PathBuf>,
    /// The build of one tile of a large build: only the part of the boundary in this tile (`Boundary::within_tile`).
    pub tile: Option<duckosm_core::tiles::Tile>,
    /// Also write duckOSM's `raw.*` of the area (`raw_nodes.parquet`, ...), the input of the base-map layers
    /// (docs/features.md). Needs a boundary.
    pub raw: bool,
    /// Also write the cut itself (the raw cut before the OSM fixes) as `cut.osm.pbf`, and with `cut_osm` as `cut.osm`
    /// (XML). Needs a boundary and a region file (`Input::Area`). docs/app.md.
    pub cut_pbf: bool,
    pub cut_osm: bool,
    /// The boundary of the area for `main.boundary` and `main.visualization_metadata`, when the PBF is
    /// already cut to it (a config with `--pbf-is-cut`). `boundary` is used when this is None.
    pub area_boundary: Option<PathBuf>,
    /// The build time in `main.visualization_metadata` (microseconds since 1970, UTC). None: now. A fixed
    /// value makes the output the same on each run.
    pub built_at: Option<i64>,
    /// The entry of the area's Geofabrik region in the page's time zone table (`www/timezones.json`, JSON text):
    /// the browser build has no world map of the zones. None: the map of `tzf-rs` (the cargo feature `tzf`).
    pub timezone_entry: Option<String>,
    /// A DEM (GeoTIFF, EPSG:4326): the elevation step of duckOSM (`duckosm elevation --dem`) runs on each mode
    /// and writes `elevation_nodes_<mode>.parquet`, `elevation_edges_<mode>.parquet` (with `z_vertices`, the
    /// height at each vertex) and `elevation_metadata_main.parquet`; the .duckdb gets duckOSM's columns.
    pub dem: Option<PathBuf>,
    /// The DEM as the bytes of a GeoTIFF and a file name, for a build with no file system (the browser). `dem` wins
    /// when both are set. The same step and the same files as `dem`; the metadata's `uri` is the file name.
    pub dem_bytes: Option<(String, Vec<u8>)>,
    /// The product (`DTM` or `DSM`), the vertical datum (for example `EGM2008`) and the license of the DEM, for
    /// `main.elevation_metadata`. The DEM is the area's reference ground (docs/concepts/ground-reference.md): the build
    /// stops when one of them is None, unless `dem_not_reference` is set. Then the unknown values are written as `unknown`.
    pub dem_product: Option<String>,
    pub dem_vertical_datum: Option<String>,
    pub dem_license: Option<String>,
    pub dem_not_reference: bool,
    /// Also make `main.building_faces` (`building_faces_main.parquet`, docs/osm-editing.md): the walls of the buildings of the raw cut, each with a
    /// type (front, back, side, party), from the street edges of the driving and walking modes. Needs a boundary and a region file, as `raw` does.
    pub building_faces: bool,
    /// duckOSM `options.h3_indexing` with `options.h3_resolution`: add `nodes.h3_cell` and `edges.from_cell`, `to_cell`
    /// and `lca_res` (h3.rs). None: no H3 columns.
    pub h3_resolution: Option<u8>,
    /// duckOSM `options.build_features`: make the base-map layers, `features.*`, and keep `raw.*` in the `.duckdb` file.
    /// duckOSM's SQL runs on DuckDB's spatial extension (docs/features.md). Needs a boundary and a region file, as `raw` does,
    /// and the `.duckdb` output.
    pub features: bool,
    /// duckOSM `options.sea` (with `features`): also make `features.ocean` from Overture Maps. It reads the network, and
    /// a failure is a warning, as in duckOSM.
    pub sea: bool,
    /// duckOSM `options.memory_limit` and `options.threads`: DuckDB's settings for the load of `raw.*` with `read_osm`
    /// (docs/design/streaming_load.md). None: DuckDB's defaults (80% of the RAM, all cores).
    pub memory_limit: Option<String>,
    pub threads: Option<usize>,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            modes: vec![Mode::Driving],
            geoparquet: true,
            duckdb: cfg!(feature = "duckdb"),
            name: "area".to_string(),
            options: Options::default(),
            bus_routes: true,
            levels: LevelsMethod::Tags,
            infer_sidewalks: true,
            osm_fixes: None,
            boundary: None,
            outside_junctions: None,
            tile: None,
            raw: false,
            cut_pbf: false,
            cut_osm: false,
            area_boundary: None,
            built_at: None,
            timezone_entry: None,
            dem: None,
            dem_bytes: None,
            dem_product: None,
            dem_vertical_datum: None,
            dem_license: None,
            dem_not_reference: false,
            building_faces: false,
            memory_limit: None,
            threads: None,
            h3_resolution: None,
            features: false,
            sea: false,
        }
    }
}

/// The numbers of one mode.
#[derive(Clone, Debug)]
pub struct ModeSummary {
    pub mode: Mode,
    pub edges: usize,
    pub nodes: usize,
    pub private_edges: usize,
    pub edge_id_map: usize,
    pub connectors: usize,
    /// The per-mode OSM fixes that matched a way of this mode.
    pub osm_fixes: usize,
    pub edge_graph: usize,
    pub turn_restrictions: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub modes: Vec<ModeSummary>,
    /// The files that were written: (kind, path).
    pub files: Vec<(String, PathBuf)>,
    /// The time of each step in seconds.
    pub steps: Vec<(String, f64)>,
    /// Roads that got `sidewalk=separate`, when the sidewalk rule ran.
    pub sidewalk_roads: Option<usize>,
    /// The number of bus routes, when they were made.
    pub bus_routes: Option<usize>,
    /// The rows of `raw.nodes`, `raw.ways` and `raw.relations`, when they were written.
    pub raw: Option<(usize, usize, usize)>,
    /// The polygons of `features.ocean`, when it was made (0: Overture was not reachable, or has no sea here).
    pub ocean: Option<usize>,
    /// The walls and the buildings of `main.building_faces`, when it was made.
    pub building_faces: Option<(usize, usize)>,
    /// A summary of the drawing levels, when they were made.
    pub levels: Option<String>,
    /// A summary of the lane profile, when the driving mode was built.
    pub lanes: Option<String>,
}

impl Summary {
    /// The result as lines of text: the CLI prints them and the page shows them.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(n) = self.sidewalk_roads {
            out.push(format!("sidewalk rule: {n} roads get sidewalk=separate"));
        }
        for m in &self.modes {
            out.push(format!(
                "{}: {} edges, {} nodes, {} private edges, {} connectors, {} edge_id_map rows, {} OSM fixes, {} edge_graph rows, {} turn restrictions",
                m.mode.name(),
                m.edges,
                m.nodes,
                m.private_edges,
                m.connectors,
                m.edge_id_map,
                m.osm_fixes,
                m.edge_graph,
                m.turn_restrictions
            ));
        }
        if let Some(n) = self.bus_routes {
            out.push(format!("bus: {n} routes"));
        }
        if let Some((n, w, r)) = self.raw {
            out.push(format!("raw: {n} nodes, {w} ways, {r} relations"));
        }
        if let Some(n) = self.ocean {
            out.push(format!("features.ocean: {n} polygons"));
        }
        if let Some((f, b)) = self.building_faces {
            out.push(format!("building faces: {f} walls of {b} buildings"));
        }
        out.extend(self.levels.iter().chain(&self.lanes).cloned());
        out
    }
}

/// Write the tables of one mode, each to a writer from `file`, which gets the file name (for example
/// `edges_driving.parquet`). The native build and the browser build (duckosm-wasm) both use it, so they
/// write the same tables: the edges, the private edges, the nodes, `edge_id_map` (with `merge_segments`),
/// and the tables of the edge graph and the turn restrictions that the options made.
pub fn write_tables<W: Write + Send>(
    net: &Network,
    mode: Mode,
    opts: &Options,
    all_ways: &[Way],
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<(), String> {
    use duckosm_parquet as pq;
    let n = mode.name();
    let oc = pq::EdgeCols::of(mode, opts);
    pq::write_edges_to(file(&format!("edges_{n}.parquet"))?, &net.edges, oc)?;
    pq::write_edges_to(file(&format!("private_edges_{n}.parquet"))?, &net.private_edges, oc)?;
    write_other_tables(net, mode, opts, Some(all_ways), file)
}

/// [`write_tables`] for a network with its edges in temporary files ([`build_network_files`]): the edge tables are
/// written from the files, one row group at a time.
pub fn write_tables_files<W: Write + Send>(
    net: &mut NetworkFiles,
    mode: Mode,
    opts: &Options,
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<(), String> {
    use duckosm_parquet as pq;
    let n = mode.name();
    let oc = pq::EdgeCols::of(mode, opts);
    let err = |e: std::io::Error| format!("temporary file of the network step: {e}");
    let rows = net.files.edges().map_err(err)?.map(|e| e.map_err(err));
    pq::write_edges_iter_to(file(&format!("edges_{n}.parquet"))?, rows, oc)?;
    let rows = net.files.private_edges().map_err(err)?.map(|e| e.map_err(err));
    pq::write_edges_iter_to(file(&format!("private_edges_{n}.parquet"))?, rows, oc)?;
    // the tables `ways` and `way_nodes` are written by the network step (`ways_out` of build_network_files)
    write_other_tables(&net.net, mode, opts, None, file)
}

/// The tables of a network but `edges` and `private_edges`.
fn write_other_tables<W: Write + Send>(
    net: &Network,
    mode: Mode,
    opts: &Options,
    all_ways: Option<&[Way]>,
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<(), String> {
    use duckosm_parquet as pq;
    let n = mode.name();
    let mut out = |table: &str| file(&format!("{table}_{n}.parquet"));
    pq::write_nodes_to(out("nodes")?, &net.nodes)?;
    // duckOSM: left over from its build and always empty (the virtual nodes are in `nodes`)
    pq::write_nodes_to(out("virtual_nodes")?, &[])?;
    if let Some(all_ways) = all_ways.filter(|_| opts.ways_tables) {
        pq::write_ways_to(out("ways")?, &net.ways, all_ways)?;
        pq::write_way_nodes_to(out("way_nodes")?, &net.ways)?;
    }
    if opts.merge_segments {
        pq::write_edge_id_map_to(out("edge_id_map")?, &net.edge_id_map)?;
    }
    let g = &net.graph;
    if let Some(rows) = &g.edge_graph {
        pq::write_edge_graph_to(out("edge_graph")?, rows)?;
    }
    if let Some(rows) = &g.turn_permission {
        pq::write_turn_permission_to(out("turn_permission")?, rows)?;
    }
    if let Some(rows) = &g.turn_restrictions {
        pq::write_turn_restrictions_to(out("turn_restrictions")?, rows)?;
    }
    if let Some(rows) = &g.turn_path_restrictions {
        pq::write_turn_path_restrictions_to(out("turn_path_restrictions")?, rows)?;
    }
    Ok(())
}

/// The street segments of an edge near a building (`near`; the others cannot be the street of a wall), to the faces' street file.
/// The roads that a building can face are the edges of a mode, but not the paths that run along or across a street (footways,
/// steps, cycleways), which would be the nearest "street" of a wall that faces the real one. An edge and its reverse edge give
/// the same segments: the faces keep the first copy of a segment.
fn street_segments(
    e: &duckosm_core::model::Edge,
    near: &building_faces::NearBuildings,
    out: &mut building_faces::StreetFile,
) -> Result<(), String> {
    const NOT_STREETS: [&str; 7] = ["footway", "path", "steps", "corridor", "bridleway", "cycleway", "track"];
    if NOT_STREETS.contains(&e.attrs.highway.as_str()) {
        return Ok(());
    }
    // one copy of the texts for each edge, not for each segment
    let name: Option<std::sync::Arc<str>> = e.attrs.name.as_deref().map(Into::into);
    let highway: std::sync::Arc<str> = e.attrs.highway.as_str().into();
    for w in e.geometry.windows(2).filter(|w| near.near(w[0], w[1])) {
        out.push(building_faces::Street {
            a: w[0],
            b: w[1],
            edge_id: e.edge_id,
            osm_id: e.osm_id,
            name: name.clone(),
            highway: highway.clone(),
        })?;
    }
    Ok(())
}

/// Write `bus.routes` and `bus.route_edges` (docs/bus.md) of the driving network `net`, from the route relations and
/// the ways of the raw cut: `routes_bus.parquet` and `route_edges_bus.parquet`. Returns the number of routes.
pub fn write_bus<W: Write + Send>(
    raw: &Raw,
    files: &mut NetworkEdges,
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<usize, String> {
    // only the edges of the member ways of the relations: the routes look up no other (bus::routes)
    let members: duckosm_core::util::FxHashSet<i64> = raw
        .relations
        .iter()
        .flat_map(|r| r.members.iter())
        .filter(|m| m.1 == duckosm_core::graph::MemberType::Way)
        .map(|m| m.0)
        .collect();
    let (mut edges, mut private_edges) = (Vec::new(), Vec::new());
    each_edge(files, false, |e| if members.contains(&e.osm_id) { edges.push(e) })?;
    each_edge(files, true, |e| if members.contains(&e.osm_id) { private_edges.push(e) })?;
    let (routes, edges) = duckosm_core::bus::routes(&raw.relations, &raw.ways, &edges, &private_edges);
    duckosm_parquet::write_routes_to(file("routes_bus.parquet")?, &routes)?;
    duckosm_parquet::write_route_edges_to(file("route_edges_bus.parquet")?, &edges)?;
    Ok(routes.len())
}

/// The edges (or the private edges) of a mode from its spill files, one at a time, in the order of `edge_id`. A read error
/// ends the edges and goes to `err`.
fn edges_of<'a>(
    files: &'a mut NetworkEdges,
    private: bool,
    err: &'a mut Option<String>,
) -> Result<Box<dyn Iterator<Item = Edge> + 'a>, String> {
    let msg = |e: std::io::Error| format!("temporary file of the network step: {e}");
    let rows: Box<dyn Iterator<Item = std::io::Result<Edge>> + 'a> = if private {
        Box::new(files.private_edges().map_err(msg)?)
    } else {
        Box::new(files.edges().map_err(msg)?)
    };
    Ok(Box::new(rows.map_while(move |r| r.map_err(|e| *err = Some(msg(e))).ok())))
}

/// Each edge (or each private edge) of a mode, from its spill files, in the order of `edge_id`.
fn each_edge(files: &mut NetworkEdges, private: bool, mut f: impl FnMut(Edge)) -> Result<(), String> {
    let err = |e: std::io::Error| format!("temporary file of the network step: {e}");
    if private {
        for e in files.private_edges().map_err(err)? {
            f(e.map_err(err)?);
        }
    } else {
        for e in files.edges().map_err(err)? {
            f(e.map_err(err)?);
        }
    }
    Ok(())
}

/// Write duckOSM's `raw.*` tables, each to a writer from `file`: `raw_nodes.parquet`, `raw_ways.parquet` and
/// `raw_relations.parquet`. `features.sql` loads them and makes the base-map layers.
pub fn write_raw<W: Write + Send>(
    raw: &Raw,
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<(), String> {
    use duckosm_parquet as pq;
    pq::write_raw_nodes_to(file("raw_nodes.parquet")?, &raw.nodes)?;
    pq::write_raw_ways_to(file("raw_ways.parquet")?, &raw.ways, &raw.nodes)?;
    pq::write_raw_relations_to(file("raw_relations.parquet")?, &raw.relations)
}

/// duckOSM's SQL of the base-map layers (`features.*`), from the `raw.*` files of [`write_raw`]: made by
/// tools/make_features_sql.py from duckOSM's `FeaturesBuilder`. It needs DuckDB's spatial extension.
pub const FEATURES_SQL: &str = include_str!("../sql/features.sql");

/// `layers.json`: the list of the edge and node files with their counts, as `duckosm export-gis`
/// writes it, so the roadstyle GeoLibre plugin opens the folder in one step.
fn write_layers_json(dir: &Path, sum: &Summary) -> Result<(), String> {
    let mut rows = Vec::new();
    for m in &sum.modes {
        let n = m.mode.name();
        for (kind, count) in [("edges", m.edges), ("nodes", m.nodes)] {
            rows.push(format!(
                "  {{\"file\": \"{kind}_{n}.parquet\", \"kind\": \"{kind}\", \"mode\": \"{n}\", \"count\": {count}}}"
            ));
        }
    }
    let json = format!("{{\"layers\": [\n{}\n]}}\n", rows.join(",\n"));
    std::fs::write(dir.join("layers.json"), json).map_err(|e| e.to_string())
}

impl Params {
    /// Set the output formats from their names: `duckdb`, `geoparquet`. Without a call, the formats are both, or
    /// geoparquet when the cargo feature `duckdb` is off.
    pub fn set_formats<S: AsRef<str>>(&mut self, formats: &[S]) -> Result<(), String> {
        self.geoparquet = false;
        self.duckdb = false;
        for f in formats {
            match f.as_ref().trim() {
                "duckdb" => self.duckdb = true,
                "geoparquet" => self.geoparquet = true,
                f => return Err(format!("unknown format '{f}' (use duckdb or geoparquet)")),
            }
        }
        Ok(())
    }
}

/// The method of the drawing levels from its name: tags, solve or none.
pub fn parse_levels(name: &str) -> Result<LevelsMethod, String> {
    LevelsMethod::parse(name).ok_or_else(|| format!("levels: '{name}' is not tags, solve or none"))
}

/// A new reader of the area file for each pass over it.
pub type Open<'a> = &'a dyn Fn() -> Result<Box<dyn Read + Send + 'a>, String>;

/// The input of [`run`]: a `.osm.pbf` file (its path, or its bytes in WebAssembly), OSM JSON (the output of the
/// Overpass API; with a boundary, only the ways with a node inside are kept), or an area file and its boundary (the
/// output of step 0, [`cut_file`]: the network has the ways with a node inside, as osmium's `complete_ways` cut;
/// `raw.*` is the whole file, as osmium's `smart` cut).
/// Another source of the network input (the ways, the coordinates and the turn restrictions) with the same values
/// as the reader of the file: the `raw.*` tables of a `.duckdb` file (docs/design/network_stream.md, part A1).
pub trait NetworkSource {
    fn ways(&self) -> Result<Vec<duckosm_core::model::Way>, String>;
    fn coords(&self, ways: &[duckosm_core::model::Way]) -> Result<Coords, String>;
    fn restrictions(&self) -> Result<Vec<duckosm_core::graph::Relation>, String>;
}

#[cfg(feature = "duckdb")]
impl NetworkSource for duckosm_duckdb::RawInput {
    fn ways(&self) -> Result<Vec<duckosm_core::model::Way>, String> {
        duckosm_duckdb::RawInput::ways(self)
    }
    fn coords(&self, ways: &[duckosm_core::model::Way]) -> Result<Coords, String> {
        duckosm_duckdb::RawInput::coords(self, ways)
    }
    fn restrictions(&self) -> Result<Vec<duckosm_core::graph::Relation>, String> {
        duckosm_duckdb::RawInput::restrictions(self)
    }
}

#[derive(Clone, Copy)]
pub enum Input<'a> {
    Path(&'a Path),
    /// The whole file, read from another source than the file (`raw.*` in DuckDB).
    Source(&'a dyn NetworkSource),
    Pbf(&'a [u8]),
    OsmJson(&'a [u8], Option<&'a Boundary>),
    Area(Open<'a>, &'a Boundary),
}

type Progress = Box<dyn Fn(&str) + Send>;
static PROGRESS: std::sync::Mutex<Option<Progress>> = std::sync::Mutex::new(None);

/// Call `f` with the name of each step that ends, in every build of this process (`duckosm studio` shows it). `None` takes the hook away.
/// ponytail: one hook for the process, so two builds at the same time share it; the page runs one build at a time.
pub fn set_progress(f: Option<Progress>) {
    *PROGRESS.lock().unwrap() = f;
}

/// The time and the memory of each step of [`run`]. The caller gives the clock: WebAssembly has no clock in std.
pub struct StepLog {
    clock: fn() -> f64,
    memory: fn() -> f64,
    last: f64,
    /// The name of each step, its milliseconds, and the memory in MiB after it (0 when not known).
    pub rows: Vec<(String, f64, f64)>,
}

impl StepLog {
    /// `clock` gives milliseconds, `memory` the memory in MiB.
    pub fn new(clock: fn() -> f64, memory: fn() -> f64) -> StepLog {
        StepLog {
            clock,
            memory,
            last: clock(),
            rows: Vec::new(),
        }
    }

    /// The system clock and no memory (not in WebAssembly).
    pub fn system() -> StepLog {
        StepLog::new(
            || {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
            },
            || 0.0,
        )
    }

    /// The end of a step: its time since the step before, and the memory now. The hook of [`set_progress`] gets the name.
    pub fn step(&mut self, name: &str) {
        let t = (self.clock)();
        self.rows
            .push((name.to_string(), t - self.last, (self.memory)()));
        self.last = t;
        if let Some(f) = PROGRESS.lock().unwrap().as_ref() {
            f(name);
        }
    }

    /// The steps as JSON (the names are plain text, with no quotes).
    pub fn json(&self) -> String {
        let rows: Vec<String> = self
            .rows
            .iter()
            .map(|(s, ms, mib)| format!(r#"{{"step":"{s}","ms":{ms:.1},"mib":{mib:.1}}}"#))
            .collect();
        format!("[{}]", rows.join(","))
    }
}

/// Read a PBF file in passes: the ways, the OSM fixes on them, the coordinates of their nodes, and the turn
/// restrictions when `relations` is true.
fn read_pbf(
    ways: impl FnOnce() -> Result<Vec<Way>, String>,
    coords: impl FnOnce(&[Way]) -> Result<Coords, String>,
    restrictions: impl FnOnce() -> Result<Vec<Relation>, String>,
    relations: bool,
    fixes: &OsmFixes,
    log: &mut StepLog,
) -> Result<(Vec<Way>, Coords, Vec<Relation>), String> {
    let mut ways = ways()?;
    log.step("read ways");
    // duckOSM: the fixes on the raw ways come before any step reads them
    fixes.apply_to_ways(&mut ways)?;
    let coords = coords(&ways)?;
    log.step("read nodes");
    let mut rels = Vec::new();
    if relations {
        rels = restrictions()?;
        log.step("read restrictions");
    }
    Ok((ways, coords, rels))
}

/// The ids that the OSM fixes touch: the OSM nodes of the new ways, the ways (removed, changed, edited, or whose tags a
/// new way takes), and the relations (edited).
fn fix_ids(fixes: &OsmFixes) -> [duckosm_core::util::FxHashSet<i64>; 3] {
    use duckosm_core::fixes::{Elem, NewNode};
    let mut ids: [duckosm_core::util::FxHashSet<i64>; 3] = Default::default();
    for n in &fixes.new_ways {
        ids[0].extend(n.nodes.iter().filter_map(|x| match x {
            NewNode::Osm(id) => Some(*id),
            NewNode::At(_) => None,
        }));
        ids[1].extend(n.tags_from);
    }
    ids[1].extend(fixes.ways.iter().map(|r| r.osm_id));
    for e in &fixes.tag_edits {
        let i = match e.elem {
            Elem::Node => 0,
            Elem::Way => 1,
            Elem::Relation => 2,
        };
        ids[i].insert(e.id);
    }
    ids
}

/// The part of the `raw.*` of an area file that the build needs in memory: the elements that the fixes touch (`ids`, with
/// all their tags), the route relations of the bus routes and their member ways (`bus`, all tags), and the buildings with
/// their nodes (`faces`, only the tags that the faces read: `building` and `addr:street` of a way, `entrance` of a node).
/// `way_nodes`: also the nodes of the kept ways (else only the nodes that the fixes touch).
fn read_raw_part(
    open: Open,
    ids: &[duckosm_core::util::FxHashSet<i64>; 3],
    bus: bool,
    faces: bool,
    way_nodes: bool,
) -> Result<Raw, String> {
    fn tag<'t>(t: &[(&'t str, &'t str)], k: &str) -> Option<&'t str> {
        t.iter().find(|x| x.0 == k).map(|x| x.1)
    }
    let building = |t: &[(&str, &str)]| tag(t, "building").is_some_and(|b| b != "no");
    let relation = |id: i64, t: &[(&str, &str)]| {
        ids[2].contains(&id)
            || (bus && tag(t, "route").is_some_and(|v| duckosm_core::bus::ROUTES.contains(&v)))
            || (faces && tag(t, "type") == Some("multipolygon") && building(t))
    };
    let way = |id: i64, t: &[(&str, &str)]| ids[1].contains(&id) || (faces && building(t));
    let node = |id: i64| ids[0].contains(&id);
    let tag = |kind: usize, id: i64, key: &str| {
        ids[kind].contains(&id) || if kind == 0 { key == "entrance" } else { key == "building" || key == "addr:street" }
    };
    duckosm_core::raw::read_part(open, &Want { relation: &relation, way: &way, node: &node, way_nodes, tag: &tag })
}

/// A hash of all tags of a way, in their order.
fn tags_hash(w: &Way) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = duckosm_core::util::FxHasher::default();
    for (k, v) in w.all_tags() {
        k.hash(&mut h);
        v.hash(&mut h);
    }
    h.finish()
}

/// The rings of the buildings of the faces from an area file, with the fixes, to a temporary file, and the entrance nodes. In
/// memory: the elements that the fixes touch, the building multipolygons with their member ways (a few), and the entrance nodes.
/// The building ways go from the file to a temporary file one at a time (the raw building ways were 0.3 GB on Kentucky); the
/// coordinates of their nodes come from a join with the nodes of the file as they come (`spill::join::Merge`).
fn faces_rings(open: Open, fixes: &OsmFixes) -> Result<building_faces::BuildingRings, String> {
    use duckosm_core::graph::MemberType;
    use duckosm_core::raw::{Patched, RawPatch, RawSink};
    use duckosm_core::spill::{self, join, Codec, Writer};
    const BUDGET: usize = 64 << 20; // each sort holds about this much
    let io = |e: std::io::Error| format!("a temporary file of the building faces: {e}");
    let store = spill::temp_store();
    let store = &*store;
    // the elements that the fixes touch, the building multipolygons and their member ways (all their tags), with the fixes
    let touched = fix_ids(fixes);
    let mut part = {
        fn tag<'t>(t: &[(&'t str, &'t str)], k: &str) -> Option<&'t str> {
            t.iter().find(|x| x.0 == k).map(|x| x.1)
        }
        let relation = |id: i64, t: &[(&str, &str)]| {
            touched[2].contains(&id)
                || (tag(t, "type") == Some("multipolygon") && tag(t, "building").is_some_and(|b| b != "no"))
        };
        let way = |id: i64, _: &[(&str, &str)]| touched[1].contains(&id);
        let node = |id: i64| touched[0].contains(&id);
        let tag = |kind: usize, id: i64, key: &str| touched[kind].contains(&id) || (kind == 0 && key == "entrance");
        let want = Want { relation: &relation, way: &way, node: &node, way_nodes: false, tag: &tag };
        duckosm_core::raw::read_part(open, &want)?
    };
    let read = RawPatch::ids(&part, &touched);
    part.apply_fixes(fixes);
    let patch = RawPatch { fixed: &part, read, net: Vec::new() };
    duckosm_core::util::step("building faces 0a: the fixes and the building multipolygons");
    // the building ways: their nodes to a temporary file (the order of the rings), and the nodes that the join must find, with
    // the number of the way. The member ways of the multipolygons (in memory) come after them.
    struct Ways<'p> {
        patch: &'p RawPatch<'p>,
        found: Vec<(i64, Vec<i64>, Option<String>)>,
    }
    impl Ways<'_> {
        fn add<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: Vec<i64>) {
            let tags: Vec<(&str, &str)> = tags.collect();
            if building_faces::is_building_way(tags.iter().copied(), &refs) {
                self.found.push((id, refs, building_faces::addr_of(tags.into_iter())));
            }
        }
    }
    impl RawSink for Ways<'_> {
        fn node<'a>(&mut self, _: i64, _: f64, _: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {}
        fn way<'a>(
            &mut self,
            id: i64,
            tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            refs: &mut dyn Iterator<Item = i64>,
        ) {
            match self.patch.way(id) {
                Patched::Removed => {}
                Patched::Fixed(w) => {
                    self.add(id, &mut w.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())), w.refs.clone())
                }
                Patched::Same => self.add(id, tags, refs.collect()),
            }
        }
        fn relation<'a>(
            &mut self,
            _: i64,
            _: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
        ) {
        }
    }
    struct Header;
    impl Codec<(i64, Vec<i64>, Option<String>)> for Header {
        fn put(&self, (id, refs, addr): &(i64, Vec<i64>, Option<String>), out: &mut Vec<u8>) {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&(refs.len() as u32).to_le_bytes());
            for r in refs {
                out.extend_from_slice(&r.to_le_bytes());
            }
            out.extend_from_slice(addr.as_deref().unwrap_or("\0").as_bytes());
        }
        fn get(&self, b: &mut &[u8]) -> (i64, Vec<i64>, Option<String>) {
            let i = |at: usize| i64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"));
            let n = u32::from_le_bytes(b[8..12].try_into().expect("4 bytes")) as usize;
            let refs = (0..n).map(|k| i(12 + 8 * k)).collect();
            let addr = std::str::from_utf8(&b[12 + 8 * n..]).expect("UTF-8 that was written");
            (i(0), refs, (addr != "\0").then(|| addr.to_string()))
        }
    }
    let (mut headers, mut requests) = (Writer::new(store).map_err(io)?, Writer::new(store).map_err(io)?);
    let mut number = 0u32;
    let mut put = |found: Vec<(i64, Vec<i64>, Option<String>)>| -> Result<(), String> {
        for h in found {
            for (k, &n) in h.1.iter().enumerate() {
                requests.push(&join::Request, &(n, number, k as u32)).map_err(io)?;
            }
            headers.push(&Header, &h).map_err(io)?;
            number += 1;
        }
        Ok(())
    };
    duckosm_core::cutstream::each_block(
        &mut open()?,
        |b, buf| {
            let mut s = Ways { patch: &patch, found: Vec::new() };
            duckosm_core::pbfraw::decode_block(b, [false, true, false], buf, &mut s)?;
            Ok(s.found)
        },
        |found, _| put(found).map(|_| true),
    )?;
    let mut s = Ways { patch: &patch, found: Vec::new() };
    for w in patch.new_ways() {
        s.add(w.id, &mut w.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())), w.refs.clone());
    }
    put(s.found)?;
    drop(put);
    let ways_end = number;
    // the member ways of the building multipolygons, after the fixes (as the part has them)
    let relations: Vec<&duckosm_core::graph::Relation> =
        part.relations.iter().filter(|r| building_faces::is_building_relation(r)).collect();
    let members: duckosm_core::util::FxHashSet<i64> =
        relations.iter().flat_map(|r| r.members.iter().filter(|m| m.1 == MemberType::Way).map(|m| m.0)).collect();
    let member_ways: Vec<&duckosm_core::raw::RawWay> = part.ways.iter().filter(|w| members.contains(&w.id)).collect();
    drop(members);
    for (m, w) in member_ways.iter().enumerate() {
        for (k, &n) in w.refs.iter().enumerate() {
            requests.push(&join::Request, &(n, ways_end + m as u32, k as u32)).map_err(io)?;
        }
    }
    let headers = headers.finish().map_err(io)?;
    duckosm_core::util::step("building faces 0b: the building ways");
    // the nodes of the file (but those that the fixes touch: the part has them, after the fixes): their coordinates to the join,
    // and the entrances
    struct Nodes(Vec<(i64, f64, f64, bool)>);
    impl RawSink for Nodes {
        fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
            self.0.push((id, lon, lat, building_faces::is_entrance(tags)));
        }
        fn way<'a>(&mut self, _: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = i64>) {}
        fn relation<'a>(
            &mut self,
            _: i64,
            _: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
        ) {
        }
    }
    let each_node = |f: &mut dyn FnMut(i64, f64, f64, bool) -> Result<(), String>| {
        duckosm_core::cutstream::each_block(
            &mut open()?,
            |b, buf| {
                let mut s = Nodes(Vec::new());
                duckosm_core::pbfraw::decode_block(b, [true, false, false], buf, &mut s)?;
                Ok(s.0)
            },
            |nodes, _| {
                for (id, lon, lat, entrance) in nodes {
                    if !touched[0].contains(&id) {
                        f(id, lon, lat, entrance)?;
                    }
                }
                Ok(true)
            },
        )
    };
    let mut entrances = duckosm_core::util::FxHashMap::default();
    for n in &part.nodes {
        if building_faces::is_entrance(n.tags.iter().map(|(k, v)| (k.as_str(), v.as_str()))) {
            entrances.insert(n.id, ());
        }
    }
    let extra: Vec<_> = part.nodes.iter().map(|n| (n.id, n.lon, n.lat)).collect();
    let requests = requests.finish().map_err(io)?;
    let mut merge = join::Merge::new(requests, extra, BUDGET, store).map_err(io)?;
    each_node(&mut |id, lon, lat, entrance| {
        if entrance {
            entrances.insert(id, ());
        }
        merge.node(id, lon, lat).map_err(io)
    })?;
    let points = match merge.finish(BUDGET, store).map_err(io)? {
        Some(points) => points,
        None => return Err("the building faces need an area file with its nodes in the order of their ids".into()),
    };
    duckosm_core::util::step("building faces 0c: the join with the nodes of the file");
    // the rings of the ways, in their order (a way with a node with no coordinates has no ring), then the coordinates of the
    // nodes of the member ways, and the rings of the multipolygons
    let mut rings = building_faces::RingWriter::new()?;
    let mut points = points.read(&join::Point).map_err(io)?.peekable();
    let mut member_pts: duckosm_core::util::FxHashMap<i64, duckosm_core::geom::Pt> = Default::default();
    let mut next = |number: u32| -> Result<Vec<duckosm_core::geom::Pt>, String> {
        let mut p = Vec::new();
        while let Some(x) = points.next_if(|x| x.as_ref().is_ok_and(|x| x.0 == number)) {
            let x = x.map_err(io)?;
            p.push((x.2, x.3));
        }
        if let Some(Err(_)) = points.peek() {
            return Err(io(points.next().expect("peeked").expect_err("an error")));
        }
        Ok(p)
    };
    for (i, h) in headers.read(&Header).map_err(io)?.enumerate() {
        let (id, refs, addr) = h.map_err(io)?;
        let p = next(i as u32)?;
        if p.len() == refs.len() {
            rings.way(id, refs, p, addr)?;
        }
    }
    for (m, w) in member_ways.iter().enumerate() {
        let p = next(ways_end + m as u32)?;
        if p.len() == w.refs.len() {
            member_pts.extend(w.refs.iter().copied().zip(p));
        }
    }
    let refs = |id: i64| member_ways.binary_search_by_key(&id, |w| w.id).ok().map(|i| member_ways[i].refs.clone());
    rings.relations(relations.into_iter(), refs, &member_pts)?;
    rings.finish(entrances)
}

/// The whole build without a file system, for the native build and the browser build (duckosm-wasm): read the
/// input with the OSM fixes, the sidewalk rule, each mode (written with [`write_tables`]), the bus routes, `raw.*`,
/// the area tables (`boundary` is the boundary file, GeoJSON text), the drawing levels and the lane profile. Each
/// file goes to a writer from `file`, which gets the file name. `log` gets the end of each step.
pub fn run<W: Write + Send>(
    input: Input,
    fixes: &OsmFixes,
    boundary: Option<&str>,
    p: &Params,
    log: &mut StepLog,
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<Summary, String> {
    let opts = &p.options;
    let driving = p.modes.contains(&Mode::Driving);
    let need_relations = opts.extract_restrictions && driving;
    // the bus routes need the route relations and the member ways of the raw cut (docs/bus.md)
    let bus = p.bus_routes && driving && matches!(input, Input::Area(..)); // OSM JSON has no route relations
    let cut_file = p.cut_pbf || p.cut_osm;
    // the base-map layers run on raw.*, so they need it, but write it to a file only with p.raw (see `build`)
    let want_raw = p.raw || p.features;
    if (want_raw || p.building_faces) && !matches!(input, Input::Area(..) | Input::OsmJson(..)) {
        return Err(
            "raw.* (the base-map layers) and the building faces need a boundary and a region file, or OSM JSON".into(),
        );
    }
    if cut_file && !matches!(input, Input::Area(..)) {
        return Err("the cut file needs a boundary and a region file".into());
    }
    let mut raw = None;
    // with an area file: the ids of the raw part before the fixes (see RawPatch)
    let mut area_read = None;
    // with an area file and raw.*: the tags of each way as the file has them (a hash), to find the ways that the build changes
    let mut read_tags: Option<Vec<(i64, u64)>> = None;
    let (mut ways, mut coords, relations) = match input {
        Input::Path(path) => read_pbf(
            || pbf::read_ways(path),
            |w| pbf::read_coords(path, w),
            || pbf::read_restrictions(path),
            need_relations,
            fixes,
            log,
        )?,
        Input::Source(s) => read_pbf(
            || s.ways(),
            |w| s.coords(w),
            || s.restrictions(),
            need_relations,
            fixes,
            log,
        )?,
        Input::Pbf(bytes) => read_pbf(
            || pbf::read_ways_bytes(bytes),
            |w| pbf::read_coords_bytes(bytes, w),
            || pbf::read_restrictions_bytes(bytes),
            need_relations,
            fixes,
            log,
        )?,
        Input::Area(open, boundary) => {
            let mut ways = pbf::read_ways_from(open()?)?;
            log.step("read ways");
            let mut coords = pbf::read_coords_from(open()?, &ways)?;
            // osmium's complete_ways: the network has only the ways with a node inside. The area file has more: the
            // member ways of its multipolygons (osmium's smart cut)
            ways.retain(|w| {
                w.refs.iter().any(|n| coords.get(n).is_some_and(|p| boundary.contains(p.0, p.1)))
            });
            log.step("read nodes");
            // the cut as osmium gives it: before the fixes change the ways
            if p.cut_pbf {
                std::io::copy(&mut open()?, &mut file("cut.osm.pbf")?).map_err(|e| e.to_string())?;
                log.step("write cut.osm.pbf");
            }
            if p.cut_osm {
                // ponytail: all of the area in memory; the XML is only for small areas
                let all = |_: i64, _: &[(&str, &str)]| true;
                let want = Want { relation: &all, way: &all, node: &|_| true, way_nodes: false, tag: &|_, _, _| true };
                cutfile::write_xml(&duckosm_core::raw::read_part(open, &want)?, &mut file("cut.osm")?)?;
                log.step("write cut.osm");
            }
            if want_raw {
                let mut h: Vec<(i64, u64)> = ways.iter().map(|w| (w.id, tags_hash(w))).collect();
                h.sort_unstable();
                read_tags = Some(h);
            }
            fixes.apply_to_ways(&mut ways)?;
            // the OSM nodes of the new ways of the fixes that no read way has
            let missing: duckosm_core::util::FxHashSet<i64> = fix_ids(fixes)[0]
                .iter()
                .copied()
                .filter(|id| coords.get(id).is_none())
                .collect();
            if !missing.is_empty() {
                let more = pbf::read_node_coords_from(open()?, missing)?;
                coords.extend(more.iter().copied());
            }
            let relations = if need_relations {
                let r = pbf::read_restrictions_from(open()?)?;
                log.step("read restrictions");
                r
            } else {
                Vec::new()
            };
            if want_raw || bus {
                // the small part: the elements that the fixes touch and the route relations. The buildings are read in
                // the faces step (all of them held through the build: Kentucky 0.87 GB)
                let touched = fix_ids(fixes);
                let mut part = read_raw_part(open, &touched, bus, false, false)?;
                area_read = Some(RawPatch::ids(&part, &touched));
                part.apply_fixes(fixes);
                raw = Some(part);
                log.step("read the raw part");
            }
            (ways, coords, relations)
        }
        Input::OsmJson(bytes, boundary) => {
            let mut data = OsmJson::read(bytes, want_raw || p.building_faces)?;
            if let Some(b) = boundary {
                data.cut(b);
            }
            raw = data.raw.take().map(|mut r| {
                r.apply_fixes(fixes);
                r
            });
            let mut ways = std::mem::take(&mut data.ways);
            fixes.apply_to_ways(&mut ways)?;
            let coords = data.coords(&ways);
            log.step("read the file");
            (ways, coords, data.relations)
        }
    };
    let relations = if need_relations {
        relations
    } else {
        Vec::new()
    };
    fixes.add_new_nodes(&mut ways, &mut coords);
    let mut sum = Summary::default();
    if p.infer_sidewalks && p.modes.contains(&Mode::Walking) {
        sum.sidewalk_roads = Some(sidewalks::infer_separate_sidewalks(&mut ways, &coords));
        log.step("sidewalk rule");
    }
    if let Some(x) = p.dem_product.as_deref().filter(|x| !matches!(*x, "DTM" | "DSM")) {
        return Err(format!("the DEM product must be DTM (bare earth) or DSM (surface), not {x}"));
    }
    if (p.dem.is_some() || p.dem_bytes.is_some())
        && !p.dem_not_reference
        && (p.dem_product.is_none() || p.dem_vertical_datum.is_none() || p.dem_license.is_none())
    {
        return Err("the DEM is the area's reference ground: give its product, its vertical datum and its license \
                    (--dem-product, --dem-vertical-datum, --dem-license), or mark it as not the reference \
                    (--dem-not-reference)"
            .into());
    }
    let dem = match (&p.dem, &p.dem_bytes) {
        (Some(path), _) => Some(elevation::Dem::open(path)?),
        (None, Some((name, bytes))) => Some(elevation::Dem::from_reader(std::io::Cursor::new(bytes), name)?),
        (None, None) => None,
    };
    let (mut dem_nodes, mut dem_nodata) = (0i64, 0i64);
    let mut mode_nodes = Vec::new();
    // the edges of each mode stay in their spill files on disk; each step after the network reads them from there
    let mut mode_files: Vec<(Mode, NetworkEdges)> = Vec::new();
    // the global junctions depend only on the ways: one time for all modes and for main.global_junctions
    let junctions = if opts.global_junctions {
        let mut j = duckosm_core::network::global_junctions(&ways);
        if let Some(path) = &p.outside_junctions {
            j.extend(duckosm_core::tilecut::read_junctions(path)?);
        }
        j
    } else {
        if p.outside_junctions.is_some() {
            return Err("the outside junctions need the global junctions".into());
        }
        Default::default()
    };
    log.step("global junctions");
    // the coordinates go to a temporary file, sorted by id: the network reads them there (a join on disk), and the area tables
    // need only their box. The coordinates of all nodes of the ways stayed in memory to the area tables (Kentucky 198 MiB).
    let mut positions = {
        let store = duckosm_core::spill::temp_store();
        coords.positions_file(&*store).map_err(|e| format!("a temporary file of the coordinates: {e}"))?
    };
    let nodes_box = coords.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |[w, s, e, n], (_, p)| {
        [w.min(p.0), s.min(p.1), e.max(p.0), n.max(p.1)]
    });
    drop(coords);
    for mode in &p.modes {
        let n = mode.name();
        // the rows of edge_graph go to their file while the network step makes them (docs/design/network_stream.md, part C)
        let mut graph_file = if opts.build_graph {
            Some(duckosm_parquet::EdgeGraphWriter::new(file(&format!("edge_graph_{n}.parquet"))?)?)
        } else {
            None
        };
        let mut sink = |r| graph_file.as_mut().expect("a file when there is a graph").push(r);
        let mut ways_out = |mw: &[duckosm_core::filter::ModeWay]| -> Result<(), String> {
            duckosm_parquet::write_ways_to(file(&format!("ways_{n}.parquet"))?, mw, &ways)?;
            duckosm_parquet::write_way_nodes_to(file(&format!("way_nodes_{n}.parquet"))?, mw)
        };
        let mut net = build_network_files(
            &ways,
            &mut positions,
            *mode,
            opts,
            fixes,
            &relations,
            &junctions,
            if opts.build_graph { Some(&mut sink) } else { None },
            if opts.ways_tables { Some(&mut ways_out) } else { None },
        )?;
        if let Some(w) = graph_file {
            w.finish()?;
        }
        log.step(&format!("build {n}"));
        write_tables_files(&mut net, *mode, opts, &mut file)?;
        log.step(&format!("write geoparquet {n}"));
        if p.h3_resolution.is_some() || dem.is_some() {
            // ponytail: H3 and the heights take all edges of the mode in memory, for this part only
            let mut edges = Vec::new();
            each_edge(&mut net.files, false, |e| edges.push(e))?;
            if let Some(res) = p.h3_resolution {
                let (nodes, h3_edges) = h3::index(&net.net.nodes, &edges, res)?;
                duckosm_parquet::write_h3_nodes_to(file(&format!("h3_nodes_{n}.parquet"))?, &nodes)?;
                duckosm_parquet::write_h3_edges_to(file(&format!("h3_edges_{n}.parquet"))?, &h3_edges)?;
                log.step(&format!("h3 {n}"));
            }
            if let Some(d) = &dem {
                let (nz, ez, nodata) = elevation::sample_mode(d, &net.net.nodes, &edges, 0.0);
                dem_nodes += nz.len() as i64;
                dem_nodata += nodata as i64;
                duckosm_parquet::write_node_heights_to(
                    file(&format!("elevation_nodes_{n}.parquet"))?,
                    &nz.iter().map(|r| (r.node_id, r.ele)).collect::<Vec<_>>(),
                )?;
                duckosm_parquet::write_edge_heights_to(
                    file(&format!("elevation_edges_{n}.parquet"))?,
                    &ez.into_iter()
                        .map(|r| (r.edge_id, r.z_from, r.z_to, r.z_vertices))
                        .collect::<Vec<_>>(),
                )?;
                log.step(&format!("elevation {n}"));
            }
        }
        let (edges, private_edges) = net.files.counts();
        sum.modes.push(ModeSummary {
            mode: *mode,
            edges,
            nodes: net.net.nodes.len(),
            private_edges,
            edge_id_map: net.net.edge_id_map.len(),
            connectors: net.net.stats.connectors,
            osm_fixes: net.net.stats.osm_fixes,
            edge_graph: net.net.stats.edge_graph_rows,
            turn_restrictions: net.net.graph.turn_restrictions.as_ref().map_or(0, Vec::len),
        });
        // duckOSM: the arrows of the fixes go into the raw tags after this mode, so the next modes see them
        fixes.set_arrows(&mut ways, &net.net.ways, *mode);
        if let Some(r) = raw.as_ref().filter(|_| *mode == Mode::Driving && bus) {
            sum.bus_routes = Some(write_bus(r, &mut net.files, &mut file)?);
            log.step("bus routes");
        }
        // the rest of the network of this mode (its ways, its graph) goes here
        let NetworkFiles { net, files } = net;
        mode_nodes.push((*mode, net.nodes));
        mode_files.push((*mode, files));
    }
    // each step after the network reads its data when it runs, and each part goes after the last step that reads it: the area
    // tables first, so that the node positions, the junctions and the nodes go before the levels
    drop((relations, positions));
    // raw.* reads only the elements that the fixes touch (RawPatch): the route relations and their member ways go after the bus
    // routes (France: 0.8 GB)
    if let Some(read) = &area_read {
        if want_raw {
            if let Some(r) = raw.as_mut() {
                r.retain_touched(read);
            }
        } else {
            raw = None;
        }
    }
    let built_at = p.built_at.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_micros() as i64)
    });
    if let Some(d) = &dem {
        // the ground id: the SHA-256 of the bytes of the DEM file
        let mut hash = Sha256::new();
        let (name, uri) = match (&p.dem, &p.dem_bytes) {
            (Some(path), _) => {
                let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
                std::io::copy(&mut f, &mut hash).map_err(|e| format!("{}: {e}", path.display()))?;
                (
                    path.file_name().map_or(String::new(), |f| f.to_string_lossy().into_owned()),
                    std::fs::canonicalize(path).unwrap_or(path.clone()).to_string_lossy().into_owned(),
                )
            }
            (None, Some((n, bytes))) => {
                hash.update(bytes);
                (n.clone(), n.clone())
            }
            (None, None) => unreachable!("a DEM comes from dem or dem_bytes"),
        };
        let sampled = area::iso_time(p.built_at.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_micros() as i64)
        }));
        duckosm_parquet::write_elevation_metadata_to(
            file("elevation_metadata_main.parquet")?,
            [
                "ele",
                &name,
                "file",
                &uri,
                p.dem_product.as_deref().unwrap_or("unknown"),
                "EPSG:4326",
                p.dem_vertical_datum.as_deref().unwrap_or("unknown"),
                p.dem_license.as_deref().unwrap_or("unknown"),
                &sampled,
                concat!("duckosm-rust ", env!("CARGO_PKG_VERSION")),
                &format!("{:x}", hash.finalize()),
            ],
            d.resolution_m,
            0.0,
            dem_nodes,
            dem_nodata,
        )?;
        log.step("elevation metadata");
        // the ground between the roads: the pixels over the box of the nodes of every mode, 3 pixels more on each side
        let (mut w, mut s, mut e, mut n) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for nd in mode_nodes.iter().flat_map(|(_, ns)| ns) {
            (w, s, e, n) = (w.min(nd.lon), s.min(nd.lat), e.max(nd.lon), n.max(nd.lat));
        }
        if w <= e && s <= n {
            let cells = d.grid(w, s, e, n, 3);
            duckosm_parquet::write_elevation_grid_to(
                file("elevation_grid_main.parquet")?,
                &cells.iter().map(|c| (c.col, c.row, c.lon, c.lat, c.z)).collect::<Vec<_>>(),
            )?;
            log.step("elevation grid");
        }
    }
    let area = Area {
        boundary,
        ways: &ways,
        nodes_box,
        nodes: &mode_nodes,
        junctions: &junctions,
        opts,
        built_at,
        timezone_entry: p.timezone_entry.as_deref(),
    };
    write_area_tables(&area, &mut file)?;
    log.step("write the area tables");
    // the building faces and raw.* come last, on top of less: the raw part, the street segments and the ways
    drop((mode_nodes, junctions));
    {
        let mut lane_edges = LaneEdges::default();
        for (mode, files) in &mut mode_files {
            if matches!(*mode, Mode::Driving | Mode::Cycling) {
                // the private edges only for driving (its bus lanes); the files are read one after the other
                let mut private_edges = Vec::new();
                if *mode == Mode::Driving {
                    each_edge(files, true, |e| {
                        if e.access.as_deref() == Some("bus") {
                            private_edges.push(e)
                        }
                    })?;
                }
                let mut err = None;
                lane_edges.add_edges(*mode, edges_of(files, false, &mut err)?, private_edges.into_iter());
                err.map_or(Ok(()), Err)?;
            }
        }
        sum.lanes = write_lane_profile(&lane_edges, &ways, fixes, &mut file)?;
        if sum.lanes.is_some() {
            log.step("lane profile");
        }
    }
    // raw.* needs only the network's ways whose tags the build changed (the fixes, the sidewalk rule, the arrows of the fixes):
    // all ways stayed in memory through the levels and the faces (Kentucky 187 MiB)
    let ways: Vec<Way> = match read_tags.take() {
        Some(h) => ways
            .into_iter()
            .filter(|w| h.binary_search_by_key(&w.id, |x| x.0).map_or(true, |i| h[i].1 != tags_hash(w)))
            .collect(),
        None if want_raw => ways,
        None => {
            drop(ways); // a new binding alone keeps the old list to the end (France: 2.6 GB)
            Vec::new()
        }
    };
    if p.levels == LevelsMethod::Solve {
        // by tiles: the level edges, the roads and the rules of the whole area are never in memory at once (levels::tiled)
        sum.levels = levels::tiled::write_levels_tiled(&mut mode_files, built_at, &mut file)?;
        log.step("levels");
    } else if p.levels != LevelsMethod::None {
        let mut level_edges = LevelEdges::default();
        level_edges.reserve(mode_files.iter().map(|(_, f)| f.counts().0 + f.counts().1).sum());
        for (mode, files) in &mut mode_files {
            for private in [false, true] {
                let mut err = None;
                level_edges.add(*mode, private, edges_of(files, private, &mut err)?);
                err.map_or(Ok(()), Err)?;
            }
        }
        sum.levels = write_levels(&level_edges, p.levels, built_at, &mut file)?;
        if sum.levels.is_some() {
            log.step("levels");
        }
    }
    if p.building_faces {
        // the buildings with the fixes, read from the area file here (OSM JSON has its whole raw part in memory); the grid near
        // them; the street segments near them, from the edges of the spill files
        let mut rings = match input {
            Input::Area(open, _) => faces_rings(open, fixes)?,
            _ => {
                let r = raw.as_ref().ok_or("the building faces need a raw part")?;
                building_faces::building_rings(&r.ways, &r.nodes, &r.relations)?
            }
        };
        // the street segments near a building, from the edges of the spill files, to a temporary file
        let near = rings.near()?;
        let mut streets = building_faces::StreetFile::new()?;
        for (mode, files) in &mut mode_files {
            if matches!(*mode, Mode::Driving | Mode::Walking) {
                let mut err = None;
                each_edge(files, false, |e| {
                    if err.is_none() {
                        err = street_segments(&e, &near, &mut streets).err();
                    }
                })?;
                err.map_or(Ok(()), Err)?;
            }
        }
        drop(near);
        let streets = streets.finish()?;
        let mut w = duckosm_parquet::BuildingFacesWriter::new(file("building_faces_main.parquet")?)?;
        // the faces, and the buildings: a new building id after the one before
        let (mut faces, mut buildings, mut last) = (0, 0, None);
        building_faces::building_faces_of(rings, streets, &fixes.face_edits, |part| {
            faces += part.len();
            for f in &part {
                buildings += usize::from(last != Some(f.building_id));
                last = Some(f.building_id);
            }
            w.write(&part)
        })?;
        w.close()?;
        sum.building_faces = Some((faces, buildings));
        log.step("building faces");
    }
    drop(mode_files);
    if let Some(r) = raw.as_mut().filter(|_| want_raw) {
        // the tags that the steps set on the network's ways (the sidewalk rule, the arrows of the fixes)
        match (input, area_read) {
            (Input::Area(open, _), Some(read)) => {
                let mut net: Vec<&Way> = ways.iter().collect();
                net.sort_by_key(|w| w.id);
                let patch = RawPatch { fixed: r, read, net };
                let files = [file("raw_nodes.parquet")?, file("raw_ways.parquet")?, file("raw_relations.parquet")?];
                let n = duckosm_parquet::write_area_raw(open, &patch, files, 64 << 20)?;
                sum.raw = Some((n[0], n[1], n[2]));
            }
            _ => {
                r.sync_tags(&ways);
                write_raw(r, &mut file)?;
                sum.raw = Some((r.nodes.len(), r.ways.len(), r.relations.len()));
            }
        }
        log.step("write raw");
    }
    Ok(sum)
}

/// Build from the PBF file `pbf_path` (cut to `p.boundary` when there is one) and write the files into `out`.
pub fn build(pbf_path: &Path, out: &Path, p: &Params) -> Result<Summary, String> {
    if p.duckdb && !cfg!(feature = "duckdb") {
        return Err("this build has no DuckDB output (cargo feature 'duckdb' is off)".into());
    }
    if !p.geoparquet && !p.duckdb {
        return Err("no output format: use geoparquet, duckdb or both".into());
    }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut log = StepLog::system();
    let fixes = match &p.osm_fixes {
        Some(f) => config::read_osm_fixes(f)?,
        None => OsmFixes::default(),
    };
    let read = |path: &PathBuf| {
        std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
    };
    let cut = match &p.boundary {
        Some(path) => {
            let b = Boundary::parse(&read(path)?)?;
            Some(match p.tile {
                Some(t) => b.within_tile(t),
                None => b,
            })
        }
        None => None,
    };
    if p.bus_routes && p.modes.contains(&Mode::Driving) && cut.is_none() {
        eprintln!("no bus tables: the build has no area outline (--boundary)");
    }
    let area_boundary = match p.area_boundary.as_ref().or(p.boundary.as_ref()) {
        Some(path) => Some(read(path)?),
        None => None,
    };
    // the GeoParquet files are the input of the .duckdb writer, so they always go to a folder: without the format
    // geoparquet, a new temporary folder for each call (two Python threads can build at the same time)
    let pq_dir = if p.geoparquet {
        out.to_path_buf()
    } else {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!("duckosm-{}-{n}-{t}", std::process::id()))
    };
    // a build of the whole file with the .duckdb output: raw.* first (read_osm), and the network reads its input from
    // raw.*, not from the file a second time (docs/design/network_stream.md, part A1)
    #[cfg(feature = "duckdb")]
    let raw_first = p.duckdb && cut.is_none();
    #[cfg(feature = "duckdb")]
    let (raw_rows, source) = if raw_first {
        std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
        let db = out.join(format!("{}.duckdb", p.name));
        let _ = std::fs::remove_file(&db);
        let n = duckosm_duckdb::write_raw(&db, pbf_path, p.memory_limit.as_deref(), p.threads)?;
        log.step("raw (read_osm)");
        (Some(n), Some(duckosm_duckdb::RawInput::open(&db)?))
    } else {
        (None, None)
    };
    std::fs::create_dir_all(&pq_dir).map_err(|e| e.to_string())?;
    // step 0: cut the region file to the area file. It is the output cut.osm.pbf when the user asks for it
    let area_file = match &cut {
        Some(_) if p.cut_pbf => Some(out.join("cut.osm.pbf")),
        Some(_) => Some(pq_dir.join("area.osm.pbf")),
        None => None,
    };
    let open = || -> Result<Box<dyn Read + Send>, String> {
        let f = area_file.as_deref().unwrap_or(pbf_path);
        let r = std::fs::File::open(f).map_err(|e| format!("{}: {e}", f.display()))?;
        Ok(Box::new(std::io::BufReader::with_capacity(1 << 20, r)))
    };
    let input = match &cut {
        Some(b) => Input::Area(&open, b),
        None => Input::Path(pbf_path),
    };
    #[cfg(feature = "duckdb")]
    let input = match &source {
        Some(s) => Input::Source(s),
        None => input,
    };
    // run() writes no cut.osm.pbf: step 0 has written it
    let p = &Params { cut_pbf: false, ..p.clone() };
    let res = (|| -> Result<Summary, String> {
        if let (Some(b), Some(f)) = (&cut, &area_file) {
            cut_to_file(pbf_path, b, f)?;
            log.step("cut to the boundary");
        }
        let mut files = Vec::new();
        let mut sum = run(
            input,
            &fixes,
            area_boundary.as_deref(),
            p,
            &mut log,
            |name| {
                // the cut file, and raw.* when the user asked for them (p.raw), go to the output folder. The raw files
                // of the base-map layers stay with the other files that the .duckdb writer reads.
                let path = if (name.starts_with("raw_") && p.raw) || name.starts_with("cut.") {
                    out
                } else {
                    &pq_dir
                }
                .join(name);
                files.push((name.trim_end_matches(".parquet").to_string(), path.clone()));
                std::fs::File::create(&path).map_err(|e| e.to_string())
            },
        )?;
        sum.files = files;
        if let Some(f) = area_file.as_ref().filter(|f| f.starts_with(out) && f.ends_with("cut.osm.pbf")) {
            sum.files.push(("cut.osm.pbf".to_string(), f.clone()));
        }
        if p.geoparquet {
            write_layers_json(&pq_dir, &sum)?;
            sum.files
                .push(("layers".to_string(), pq_dir.join("layers.json")));
        }
        #[cfg(feature = "duckdb")]
        if p.duckdb {
            let names: Vec<&str> = p.modes.iter().map(|m| m.name()).collect();
            let db = out.join(format!("{}.duckdb", p.name));
            if raw_first {
                // the file has raw.* already; the connection of the input closes first (one DuckDB per file in a process)
                if let Some(s) = &source {
                    s.close();
                }
                duckosm_duckdb::write_network_tables(&db, &pq_dir, &names)?;
                sum.raw = raw_rows;
            } else {
                duckosm_duckdb::write_database(&db, &pq_dir, &names)?;
            }
            log.step("write duckdb");
            if p.features {
                duckosm_duckdb::write_features(&db, &pq_dir, FEATURES_SQL)?;
                log.step("features");
                if p.sea {
                    sum.ocean = Some(duckosm_duckdb::write_ocean(&db)?);
                    log.step("ocean");
                }
                if !p.raw {
                    // the .duckdb file has raw.*: the files are only the input of the SQL
                    for f in ["raw_nodes", "raw_ways", "raw_relations"] {
                        let _ = std::fs::remove_file(pq_dir.join(format!("{f}.parquet")));
                    }
                    sum.files.retain(|(k, _)| !k.starts_with("raw_"));
                }
            }
            sum.files.push(("duckdb".to_string(), db));
        }
        Ok(sum)
    })();
    // the temporary folder and the area file go also after an error
    if let Some(f) = area_file.as_ref().filter(|f| f.ends_with("area.osm.pbf")) {
        let _ = std::fs::remove_file(f);
    }
    if !p.geoparquet {
        let _ = std::fs::remove_dir_all(&pq_dir);
    }
    let mut sum = res?;
    if !p.geoparquet {
        sum.files
            .retain(|(k, _)| k == "duckdb" || k.starts_with("raw_") || k.starts_with("cut."));
    }
    sum.steps = log
        .rows
        .iter()
        .map(|(s, ms, _)| (s.clone(), ms / 1000.0))
        .collect();
    Ok(sum)
}

/// Step 0 alone: cut the region file `region` to `boundary` (GeoJSON text) and write the area to `out` as a
/// `.osm.pbf` file. The cut keeps only lists of ids in memory ([`duckosm_core::cutstream`]).
pub fn cut_file(region: &Path, boundary: &str, out: &Path) -> Result<duckosm_core::cutstream::CutCounts, String> {
    cut_to_file(region, &Boundary::parse(boundary)?, out)
}

fn cut_to_file(region: &Path, b: &Boundary, out: &Path) -> Result<duckosm_core::cutstream::CutCounts, String> {
    let open = || {
        std::fs::File::open(region)
            .map(|f| std::io::BufReader::with_capacity(1 << 20, f))
            .map_err(|e| format!("{}: {e}", region.display()))
    };
    let f = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let (w, n) = duckosm_core::cutstream::cut_to_pbf(open, b, std::io::BufWriter::with_capacity(1 << 20, f))?;
    w.into_inner().map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(n)
}

#[cfg(test)]
mod area_raw_tests {
    use super::*;
    use std::io::Cursor;

    /// `raw.*` from the area file (step 0's file, the raw part, the patch while the file is read) has the rows of the
    /// cut in memory (`read_area_raw`, `apply_fixes`, `sync_tags`, `write_raw`): Monaco, with the fixes of both fixture
    /// files (removed ways, `keep_nodes`, new ways with `tags_from`, tag edits) and the tags of the sidewalk rule.
    #[test]
    fn raw_of_the_area_file_equals_the_cut_in_memory() {
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
        let region = std::fs::read(data.join("monaco_sample.osm.pbf")).unwrap();
        let b = Boundary::parse(&std::fs::read_to_string(data.join("monaco.geojson")).unwrap()).unwrap();
        let dir = std::env::temp_dir().join(format!("duckosm-area-raw-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (k, fixes) in ["osm_fixes.yaml", "page_fixes/osm_fixes.yaml"].iter().enumerate() {
            let fixes = config::read_osm_fixes(&data.join(fixes)).unwrap();
            // the cut in memory, and the network's ways with the fixes and the sidewalk rule
            let open = || pbf::BlobReader::new_seekable(Cursor::new(&region[..])).map_err(|e| e.to_string());
            let (mut ways, coords, _, mut want) = pbf::read_area_raw(open, &b).unwrap();
            fixes.apply_to_ways(&mut ways).unwrap();
            assert!(sidewalks::infer_separate_sidewalks(&mut ways, &coords) > 0);
            want.apply_fixes(&fixes);
            want.sync_tags(&ways);
            let file = |n: &str| std::fs::File::create(dir.join(format!("want_{k}_{n}"))).map_err(|e| e.to_string());
            write_raw(&want, file).unwrap();
            // the area file
            let (area, _) = duckosm_core::cutstream::cut_to_pbf(|| Ok(Cursor::new(&region[..])), &b, Vec::new()).unwrap();
            let open = || -> Result<Box<dyn Read + Send>, String> { Ok(Box::new(Cursor::new(area.clone()))) };
            let touched = fix_ids(&fixes);
            let mut part = read_raw_part(&open, &touched, true, true, true).unwrap();
            let read = RawPatch::ids(&part, &touched);
            part.apply_fixes(&fixes);
            let mut net: Vec<&Way> = ways.iter().collect();
            net.sort_by_key(|w| w.id);
            let files = ["raw_nodes", "raw_ways", "raw_relations"]
                .map(|n| std::fs::File::create(dir.join(format!("got_{k}_{n}.parquet"))).unwrap());
            let n = duckosm_parquet::write_area_raw(&open, &RawPatch { fixed: &part, read, net }, files, 1 << 20).unwrap();
            assert_eq!(n, [want.nodes.len(), want.ways.len(), want.relations.len()]);
            let con = duckdb::Connection::open_in_memory().unwrap();
            con.execute_batch("SET threads = 1").unwrap();
            for (t, cols) in [
                ("raw_nodes", "osm_id, lat, lon, tags::VARCHAR"),
                ("raw_ways", "osm_id, tags::VARCHAR, refs::VARCHAR, hex(line)"),
                ("raw_relations", "osm_id, tags::VARCHAR, refs::VARCHAR, ref_roles::VARCHAR, ref_types::VARCHAR"),
            ] {
                let rows = |who: &str| -> (i64, String) {
                    let f = dir.join(format!("{who}_{k}_{t}.parquet")).display().to_string();
                    con.query_row(
                        &format!("SELECT count(*), md5(string_agg(concat_ws('|', {cols}), '\n')) FROM read_parquet('{f}')"),
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap()
                };
                assert_eq!(rows("got"), rows("want"), "{t}, fixes file {k}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
