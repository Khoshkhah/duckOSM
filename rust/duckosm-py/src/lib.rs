//! Python module `duckosm._rust`: the Rust engine of duckOSM (the network step).
//!
//! ```python
//! from duckosm import _rust as dr
//! res = dr.build("monaco.osm.pbf", "out/", modes=["driving", "walking"])
//! res["modes"]["driving"]["edges"]      # 3092
//! dr.edge_id_hash(osm_id, source, target)
//! ```

#[cfg(feature = "duckdb")]
mod analysis;
#[cfg(feature = "duckdb")]
mod api;
#[cfg(feature = "duckdb")]
mod mobility;

use duckosm_build::{
    build as run_build, config, parse_levels, NetworkMode, NetworkOptions, Params, Summary,
};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::path::PathBuf;

/// Build the networks of `modes` from `pbf` and write them into the folder `out`.
///
/// The keyword names are the option names of duckOSM. `building_faces` also writes `building_faces_main.parquet` (the walls of the
/// buildings, with the wall rules of the fixes file); it needs `boundary`. Returns a dict with the numbers of each mode, the
/// files that were written and the time of each step.
#[pyfunction]
#[pyo3(signature = (
    pbf, out, *, modes = vec!["driving".to_string()], formats = None, name = "area".to_string(),
    merge_segments = true, global_junctions = true,
    cycling_dismount = true, infer_separate_sidewalks = true, connectivity_rescue = true,
    connect_snap_m = 10.0, process_speeds = true, calculate_costs = true, functional_types = true,
    osm_overrides = None, build_graph = true, extract_restrictions = true, boundary = None, ways_tables = true,
    bus_routes = true, levels = "tags".to_string(), building_faces = false
))]
#[allow(clippy::too_many_arguments)]
fn build<'py>(
    py: Python<'py>,
    pbf: PathBuf,
    out: PathBuf,
    modes: Vec<String>,
    formats: Option<Vec<String>>,
    name: String,
    merge_segments: bool,
    global_junctions: bool,
    cycling_dismount: bool,
    infer_separate_sidewalks: bool,
    connectivity_rescue: bool,
    connect_snap_m: f64,
    process_speeds: bool,
    calculate_costs: bool,
    functional_types: bool,
    osm_overrides: Option<PathBuf>,
    build_graph: bool,
    extract_restrictions: bool,
    boundary: Option<PathBuf>,
    ways_tables: bool,
    bus_routes: bool,
    levels: String,
    building_faces: bool,
) -> PyResult<Bound<'py, PyDict>> {
    let mut p = Params {
        name,
        ..Params::default()
    };
    p.modes = NetworkMode::parse_all(&modes).map_err(PyValueError::new_err)?;
    set_formats(&mut p, formats)?;
    p.infer_sidewalks = infer_separate_sidewalks;
    p.osm_fixes = osm_overrides;
    p.boundary = boundary;
    p.options = NetworkOptions {
        cycling_dismount,
        merge_segments,
        global_junctions,
        connect_paths: connectivity_rescue,
        snap_m: connect_snap_m,
        connect_near: None,
        process_speeds,
        // the costs need the speeds, as in the CLI
        calculate_costs: process_speeds && calculate_costs,
        functional_types,
        build_graph,
        extract_restrictions,
        ways_tables,
    };
    p.bus_routes = bus_routes;
    p.building_faces = building_faces;
    p.levels = parse_levels(&levels).map_err(PyValueError::new_err)?;
    // the build runs without the GIL, so other Python threads go on
    let sum = py
        .detach(|| run_build(&pbf, &out, &p))
        .map_err(PyRuntimeError::new_err)?;
    summary_dict(py, &sum)
}

fn summary_dict<'py>(py: Python<'py>, sum: &Summary) -> PyResult<Bound<'py, PyDict>> {
    let res = PyDict::new(py);
    let modes = PyDict::new(py);
    for m in &sum.modes {
        let d = PyDict::new(py);
        d.set_item("edges", m.edges)?;
        d.set_item("nodes", m.nodes)?;
        d.set_item("private_edges", m.private_edges)?;
        d.set_item("edge_id_map", m.edge_id_map)?;
        d.set_item("connectors", m.connectors)?;
        d.set_item("osm_fixes", m.osm_fixes)?;
        d.set_item("edge_graph", m.edge_graph)?;
        d.set_item("turn_restrictions", m.turn_restrictions)?;
        modes.set_item(m.mode.name(), d)?;
    }
    res.set_item("modes", modes)?;
    let files = PyDict::new(py);
    for (k, v) in &sum.files {
        files.set_item(k, v.to_string_lossy().to_string())?;
    }
    res.set_item("files", files)?;
    let steps = PyDict::new(py);
    for (k, v) in &sum.steps {
        steps.set_item(k, v)?;
    }
    res.set_item("seconds", steps)?;
    res.set_item("sidewalk_roads", sum.sidewalk_roads.unwrap_or(0))?;
    if let Some((walls, buildings)) = sum.building_faces {
        res.set_item("building_faces", (walls, buildings))?;
    }
    Ok(res)
}

/// Build from a duckOSM config file (YAML): the input, the output, the modes and the options come from it.
///
/// `pbf_is_cut`: the PBF is already cut to the boundary of the config (this program does not cut).
/// `strict`: a duckOSM step that this program does not have is an error. With `strict=False` the result
/// lists it under "skipped". Keys that have no effect here are under "ignored".
#[pyfunction]
#[pyo3(signature = (path, *, formats = None, out = None, pbf_is_cut = false, strict = true))]
fn build_from_config<'py>(
    py: Python<'py>,
    path: PathBuf,
    formats: Option<Vec<String>>,
    out: Option<PathBuf>,
    pbf_is_cut: bool,
    strict: bool,
) -> PyResult<Bound<'py, PyDict>> {
    let c = config::read(&path, pbf_is_cut, strict).map_err(PyValueError::new_err)?;
    let mut p = c.params.clone();
    set_formats(&mut p, formats)?;
    let out = out.unwrap_or(c.out.clone());
    let sum = py
        .detach(|| run_build(&c.pbf, &out, &p))
        .map_err(PyRuntimeError::new_err)?;
    let res = summary_dict(py, &sum)?;
    res.set_item("skipped", c.skipped.clone())?;
    res.set_item("ignored", c.ignored.clone())?;
    Ok(res)
}

fn set_formats(p: &mut Params, formats: Option<Vec<String>>) -> PyResult<()> {
    match formats {
        Some(f) => p.set_formats(&f).map_err(PyValueError::new_err),
        None => Ok(()),
    }
}

/// `duckosm edit` (rust/duckosm-edit, docs/design/edit_page.md): serve the page of the database `db` on 127.0.0.1:`port` in a thread of its
/// own, with the database of the program (`native::NativeDb`). The page reads the fixes file `load` and writes the fixes file `save` (None: not
/// chosen; the page asks for the save file at the first Save). Returns the address of the page at once; the server stops with the Python process.
#[pyfunction]
#[pyo3(signature = (db, load, save, port))]
fn edit(py: Python<'_>, db: PathBuf, load: Option<PathBuf>, save: Option<PathBuf>, port: u16) -> PyResult<String> {
    let listener = duckosm_edit::bind(port).map_err(PyValueError::new_err)?;
    let port = listener.local_addr().map_err(|e| PyRuntimeError::new_err(e.to_string()))?.port();
    let native = duckosm_edit::native::NativeDb::new(&db);
    let site = py.detach(|| duckosm_edit::Site::new(Box::new(native), &db, std::path::Path::new(""), port)).map_err(PyRuntimeError::new_err)?;
    site.set_files(load, save);
    let url = site.url();
    std::thread::spawn(move || duckosm_edit::serve(std::sync::Arc::new(site), listener));
    Ok(url)
}

/// `duckosm ask` (docs/design/ask_page.md): the same server, with roadstyle's data dock and Ask in place of the editor. `fixes` is the file whose
/// changes Ask can read (the table `extra.edits`).
#[pyfunction]
fn ask(py: Python<'_>, db: PathBuf, fixes: PathBuf, port: u16) -> PyResult<String> {
    let listener = duckosm_edit::bind(port).map_err(PyValueError::new_err)?;
    let port = listener.local_addr().map_err(|e| PyRuntimeError::new_err(e.to_string()))?.port();
    let native = duckosm_edit::native::NativeDb::new(&db);
    let site = py.detach(|| duckosm_edit::Site::with_mode(Box::new(native), &db, &fixes, port, duckosm_edit::Mode::Ask)).map_err(PyRuntimeError::new_err)?;
    let url = site.url();
    std::thread::spawn(move || duckosm_edit::serve(std::sync::Arc::new(site), listener));
    Ok(url)
}

/// The pixels of the DEM file `dem` over the box `bbox` (west, south, east, north) plus 3 pixels on each side, written to the Parquet file `out`: the table
/// `main.elevation_grid` (docs/reference/database.md). Gives the number of pixels. `duckosm studio` loads the file after its build.
#[pyfunction]
fn elevation_grid(py: Python<'_>, dem: PathBuf, out: PathBuf, bbox: [f64; 4]) -> PyResult<usize> {
    py.detach(|| duckosm_build::elevation::write_grid_file(&dem, &out, bbox, 3)).map_err(PyRuntimeError::new_err)
}

/// `duckosm studio` (rust/duckosm-edit/src/studio.rs): serve the build page on 127.0.0.1:`port` in a thread of its own, for the folder `out`, with
/// the host of the program (`native::NativeHost`: the build by tiles, the bundled DuckDB). `index`: the list of the Geofabrik extracts (URL or file;
/// None: Geofabrik's). Returns the address of the page.
#[pyfunction]
#[pyo3(signature = (out, port, index = None))]
fn studio(py: Python<'_>, out: PathBuf, port: u16, index: Option<String>) -> PyResult<String> {
    let listener = duckosm_edit::bind(port).map_err(PyValueError::new_err)?;
    let port = listener.local_addr().map_err(|e| PyRuntimeError::new_err(e.to_string()))?.port();
    let site = py.detach(|| duckosm_edit::studio::Studio::new(Box::new(duckosm_edit::native::NativeHost), &out, port, index)).map_err(PyRuntimeError::new_err)?;
    let url = site.url();
    std::thread::spawn(move || duckosm_edit::studio::serve(std::sync::Arc::new(site), listener));
    Ok(url)
}

/// The stable edge id of duckOSM: `(hash(osm_id, source, target) >> 1)` with the DuckDB hash.
#[pyfunction]
fn edge_id_hash(osm_id: i64, source: i64, target: i64) -> i64 {
    duckosm_core::hash::edge_id(osm_id, source, target)
}

/// The previous edge id (v1), with `is_reverse`: `(hash(osm_id, source, target, is_reverse) >> 1)` with the DuckDB hash.
#[pyfunction]
fn edge_id_hash_v1(osm_id: i64, source: i64, target: i64, is_reverse: bool) -> i64 {
    duckosm_core::hash::edge_id_v1(osm_id, source, target, is_reverse)
}

/// An `.osm.pbf` file at `path` from Python lists (for tests and small examples): nodes `(id, lon, lat, tags)`, ways `(id, refs, tags)`,
/// relations `(id, members, tags)` with members `(type, ref, role)`, type `"node"`, `"way"` or `"relation"`. Each list is sorted by id here.
#[pyfunction]
#[allow(clippy::type_complexity)]
fn write_pbf(
    path: PathBuf,
    nodes: Vec<(i64, f64, f64, Vec<(String, String)>)>,
    ways: Vec<(i64, Vec<i64>, Vec<(String, String)>)>,
    relations: Vec<(i64, Vec<(String, i64, String)>, Vec<(String, String)>)>,
) -> PyResult<()> {
    use duckosm_core::graph::{MemberType, Relation};
    use duckosm_core::raw::{Raw, RawNode, RawWay};
    let mut raw = Raw {
        nodes: nodes.into_iter().map(|(id, lon, lat, tags)| RawNode { id, lon, lat, tags }).collect(),
        ways: ways.into_iter().map(|(id, refs, tags)| RawWay { id, tags, refs }).collect(),
        relations: Vec::new(),
        timestamp: None,
    };
    for (id, members, tags) in relations {
        let members = members
            .into_iter()
            .map(|(t, r, role)| match t.as_str() {
                "node" => Ok((r, MemberType::Node, role)),
                "way" => Ok((r, MemberType::Way, role)),
                "relation" => Ok((r, MemberType::Relation, role)),
                _ => Err(PyValueError::new_err(format!("member type {t:?}: node, way or relation"))),
            })
            .collect::<PyResult<Vec<_>>>()?;
        raw.relations.push(Relation { id, tags, members });
    }
    raw.nodes.sort_by_key(|n| n.id);
    raw.ways.sort_by_key(|w| w.id);
    raw.relations.sort_by_key(|r| r.id);
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path).map_err(|e| PyRuntimeError::new_err(format!("{}: {e}", path.display())))?);
    duckosm_core::cutfile::write_pbf(&raw, None, &mut f).map_err(PyRuntimeError::new_err)?;
    std::io::Write::flush(&mut f).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

/// A `[west, south, east, north]` box as GeoJSON Polygon text (`duckosm.area.bbox_geojson`); a wrong box: ValueError.
#[pyfunction]
fn bbox_geojson(bbox: Vec<f64>) -> PyResult<String> {
    duckosm_build::place::bbox_geojson(&bbox).map(|v| v.to_string()).map_err(PyValueError::new_err)
}

/// The outline of an H3 cell as GeoJSON Polygon text (`duckosm.area.h3_geojson`); not a cell: ValueError.
#[pyfunction]
fn h3_geojson(cell: String) -> PyResult<String> {
    duckosm_build::place::h3_geojson(&cell).map(|v| v.to_string()).map_err(PyValueError::new_err)
}

#[pymodule]
fn _rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(build, m)?)?;
    m.add_function(wrap_pyfunction!(build_from_config, m)?)?;
    m.add_function(wrap_pyfunction!(edge_id_hash, m)?)?;
    m.add_function(wrap_pyfunction!(edge_id_hash_v1, m)?)?;
    m.add_function(wrap_pyfunction!(write_pbf, m)?)?;
    m.add_function(wrap_pyfunction!(bbox_geojson, m)?)?;
    m.add_function(wrap_pyfunction!(h3_geojson, m)?)?;
    m.add_function(wrap_pyfunction!(edit, m)?)?;
    m.add_function(wrap_pyfunction!(ask, m)?)?;
    m.add_function(wrap_pyfunction!(studio, m)?)?;
    m.add_function(wrap_pyfunction!(elevation_grid, m)?)?;
    #[cfg(feature = "duckdb")]
    api::add(m)?;
    #[cfg(feature = "duckdb")]
    analysis::add(m)?;
    #[cfg(feature = "duckdb")]
    mobility::add(m)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
