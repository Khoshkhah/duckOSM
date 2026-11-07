//! Python module `duckosm._rust`: the Rust engine of duckOSM (the network step).
//!
//! ```python
//! from duckosm import _rust as dr
//! res = dr.build("monaco.osm.pbf", "out/", modes=["driving", "walking"])
//! res["modes"]["driving"]["edges"]      # 3092
//! dr.edge_id_hash(osm_id, source, target)
//! ```

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
    merge_segments = true, global_junctions = true, keep_largest_component = true,
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
    keep_largest_component: bool,
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
        keep_largest_component,
        cycling_dismount,
        merge_segments,
        global_junctions,
        connect_paths: connectivity_rescue,
        snap_m: connect_snap_m,
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

/// The database of `duckosm edit` in Python (src/duckosm/edit.py): the module has no DuckDB, so the server calls the methods of a Python
/// object (open, close, execute, texts, build).
struct PyDb(Py<PyAny>);

impl duckosm_edit::Db for PyDb {
    fn open(&mut self) -> Result<(), String> {
        Python::attach(|py| self.0.call_method0(py, "open").map(|_| ()).map_err(|e| e.to_string()))
    }
    fn close(&mut self) {
        let _ = Python::attach(|py| self.0.call_method0(py, "close"));
    }
    fn execute(&mut self, sql: &str) -> Result<(), String> {
        Python::attach(|py| self.0.call_method1(py, "execute", (sql,)).map(|_| ()).map_err(|e| e.to_string()))
    }
    fn texts(&mut self, sql: &str) -> Result<Vec<Option<String>>, String> {
        Python::attach(|py| self.0.call_method1(py, "texts", (sql,)).and_then(|r| r.bind(py).extract()).map_err(|e| e.to_string()))
    }
    fn build(&mut self, config: &str, dir: &std::path::Path, fixes: &std::path::Path, db: &std::path::Path) -> Result<String, String> {
        Python::attach(|py| {
            self.0.call_method1(py, "build", (config, dir.display().to_string(), fixes.display().to_string(), db.display().to_string()))
                .and_then(|r| r.bind(py).extract()).map_err(|e| e.to_string())
        })
    }
}

/// `duckosm edit` (rust/duckosm-edit, docs/design/edit_page.md): serve the page of the database `db` on 127.0.0.1:`port` in a thread of its
/// own, with the fixes file `fixes`; `backend` is the Python object of the database (duckosm/edit.py). Returns the address of the page at
/// once; the server stops with the Python process.
#[pyfunction]
fn edit(py: Python<'_>, db: PathBuf, fixes: PathBuf, port: u16, backend: Py<PyAny>) -> PyResult<String> {
    serve_page(py, db, fixes, port, backend, duckosm_edit::Mode::Edit)
}

/// `duckosm ask` (docs/design/ask_page.md): the same server, with roadstyle's data dock and Ask in place of the editor. `fixes` is the file whose
/// changes Ask can read (the table `extra.edits`).
#[pyfunction]
fn ask(py: Python<'_>, db: PathBuf, fixes: PathBuf, port: u16, backend: Py<PyAny>) -> PyResult<String> {
    serve_page(py, db, fixes, port, backend, duckosm_edit::Mode::Ask)
}

fn serve_page(py: Python<'_>, db: PathBuf, fixes: PathBuf, port: u16, backend: Py<PyAny>, mode: duckosm_edit::Mode) -> PyResult<String> {
    let listener = duckosm_edit::bind(port).map_err(PyValueError::new_err)?;
    let port = listener.local_addr().map_err(|e| PyRuntimeError::new_err(e.to_string()))?.port();
    let site = py.detach(|| duckosm_edit::Site::with_mode(Box::new(PyDb(backend)), &db, &fixes, port, mode)).map_err(PyRuntimeError::new_err)?;
    let url = site.url();
    std::thread::spawn(move || duckosm_edit::serve(std::sync::Arc::new(site), listener));
    Ok(url)
}

/// The stable edge id of duckOSM: `(hash(osm_id, source, target) >> 1)` with the DuckDB hash.
#[pyfunction]
fn edge_id_hash(osm_id: i64, source: i64, target: i64) -> i64 {
    duckosm_core::hash::edge_id(osm_id, source, target)
}

#[pymodule]
fn _rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(build, m)?)?;
    m.add_function(wrap_pyfunction!(build_from_config, m)?)?;
    m.add_function(wrap_pyfunction!(edge_id_hash, m)?)?;
    m.add_function(wrap_pyfunction!(edit, m)?)?;
    m.add_function(wrap_pyfunction!(ask, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
