//! The program `duckosm` and the Rust parts of duckOSM's Python API (docs/design/all_in_rust.md, step 6). Each function
//! returns its result as JSON text; `duckosm/_api.py` makes the Python values. Python releases the GIL while Rust works.

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use serde_json::{json, Value as J};
use std::path::PathBuf;

#[allow(dead_code, clippy::all)]
mod program {
    include!("../../duckosm-cli/src/main.rs");
}

fn err(e: String) -> PyErr {
    PyRuntimeError::new_err(e)
}

fn text(v: &J) -> String {
    v.to_string()
}

/// Run the program `duckosm` with `args` (the first is the program name). Returns the exit code.
#[pyfunction]
fn cli(py: Python<'_>, args: Vec<String>) -> i32 {
    py.detach(|| program::main_with(args))
}

#[pyfunction]
#[pyo3(signature = (db, out_dir, mode, net_name, connections, config, netconvert))]
fn to_sumo(py: Python<'_>, db: PathBuf, out_dir: PathBuf, mode: String, net_name: String, connections: bool, config: Option<PathBuf>, netconvert: bool) -> PyResult<String> {
    let s = py
        .detach(|| duckosm_export::sumo::to_sumo(&db, &out_dir, &mode, &net_name, connections, config.as_deref(), netconvert))
        .map_err(err)?;
    let mut v = json!({"nod": s.nod, "edg": s.edg, "n_nodes": s.n_nodes, "n_edges": s.n_edges, "n_connections": s.n_connections});
    if let Some(c) = s.con {
        v["con"] = json!(c);
    }
    let cfg = out_dir.join(format!("{net_name}.netccfg"));
    if cfg.exists() {
        v["netccfg"] = json!(cfg);
    }
    if let Some(n) = s.net {
        v["net"] = json!(n);
    }
    Ok(text(&v))
}

#[pyfunction]
fn to_gis(py: Python<'_>, db: PathBuf, out: PathBuf, modes: Vec<String>, fmt: String, boundary: bool, name: Option<String>, srs: String) -> PyResult<String> {
    let s = py.detach(|| duckosm_export::gis::to_gis(&db, &out, &modes, &fmt, boundary, name.as_deref(), &srs)).map_err(err)?;
    let layers: serde_json::Map<String, J> = s.layers.into_iter().map(|(k, n)| (k, json!(n))).collect();
    let mut v = json!({"fmt": s.fmt, "layers": layers, "files": s.files});
    if fmt == "gpkg" {
        v["path"] = json!(s.path);
    } else {
        v["dir"] = json!(s.path.unwrap_or(out));
    }
    Ok(text(&v))
}

#[pyfunction]
fn to_matsim(py: Python<'_>, db: PathBuf, out: PathBuf, mode: String, crs: Option<String>, gzip: bool) -> PyResult<String> {
    let s = py.detach(|| duckosm_export::matsim::to_matsim(&db, &out, &mode, crs.as_deref(), gzip)).map_err(err)?;
    Ok(text(&json!({"nodes": s.nodes, "links": s.links, "crs": s.crs})))
}

#[pyfunction]
fn to_matsim_lanes(py: Python<'_>, gmns_db: PathBuf, out_dir: PathBuf, mode: String, signals: bool, cycle_s: i64) -> PyResult<String> {
    let s = py.detach(|| duckosm_export::matsim_lanes::to_matsim_lanes(&gmns_db, &out_dir, &mode, signals, cycle_s)).map_err(err)?;
    let mut v = json!({"lanes_links": s.lanes_links});
    if let Some(n) = s.signal_systems {
        v["signal_systems"] = json!(n);
    }
    Ok(text(&v))
}

#[pyfunction]
fn to_opendrive(py: Python<'_>, db: PathBuf, out: PathBuf, mode: String, crs: Option<String>, junctions: bool) -> PyResult<String> {
    let s = py.detach(|| duckosm_export::opendrive::to_opendrive(&db, &out, &mode, crs.as_deref(), junctions)).map_err(err)?;
    let mut v = json!({"roads": s.roads});
    if junctions {
        v["connecting_roads"] = json!(s.connecting_roads);
        v["junctions"] = json!(s.junctions);
    }
    v["crs"] = json!(s.crs);
    Ok(text(&v))
}

#[pyfunction]
fn to_railml(py: Python<'_>, db: PathBuf, out: PathBuf) -> PyResult<String> {
    let s = py.detach(|| duckosm_export::railml::to_railml(&db, &out)).map_err(err)?;
    Ok(text(&json!({"tracks": s.tracks, "switches": s.switches, "signals": s.signals, "ocps": s.ocps})))
}

#[pyfunction]
fn to_lanelet2(py: Python<'_>, gmns_db: PathBuf, out: PathBuf, mode: String, snap_m: f64) -> PyResult<String> {
    let s = py.detach(|| duckosm_export::lanelet2::to_lanelet2(&gmns_db, &out, &mode, snap_m)).map_err(err)?;
    Ok(text(&json!({"lanelets": s.lanelets, "ways": s.ways, "nodes": s.nodes})))
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn to_elevation(
    py: Python<'_>,
    db: PathBuf,
    dem: Option<String>,
    source: String,
    modes: Vec<String>,
    nodata_fill: f64,
    suffix: String,
    product: Option<String>,
    vertical_datum: Option<String>,
    license: Option<String>,
    reference: bool,
) -> PyResult<String> {
    let o = duckosm_tools::elevation::Options { dem, source, modes, nodata_fill, suffix, product, vertical_datum, license, reference };
    let s = py.detach(|| duckosm_tools::elevation::to_elevation(&db, &o)).map_err(err)?;
    let modes: serde_json::Map<String, J> = s.modes.into_iter().map(|(m, n, nd)| (m, json!({"nodes": n, "nodata": nd}))).collect();
    Ok(text(&json!({"source": s.source, "modes": modes, "column": s.column, "n_nodes": s.n_nodes, "n_nodata": s.n_nodata})))
}

impl Router {
    /// The edge ids not in the graph, as an error.
    fn check(&self, ids: &[i64]) -> PyResult<()> {
        let missing: Vec<String> = ids.iter().filter(|e| !self.graph.contains(**e)).map(i64::to_string).collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(err(format!("edge id(s) not in the {} routing graph: [{}]", self.mode, missing.join(", "))))
    }
}

/// The routing graph of one mode, made once (duckOSM's `Router`).
#[pyclass(unsendable)]
struct Router {
    con: duckdb::Connection,
    graph: duckosm_route::graph::Graph,
    mode: String,
}

#[pymethods]
impl Router {
    #[new]
    fn new(py: Python<'_>, db: PathBuf, mode: String, weight: String) -> PyResult<Self> {
        let (con, graph) = py
            .detach(|| {
                let con = duckosm_route::route::open(&db)?;
                let g = duckosm_route::route::routing_graph(&con, &mode, &weight, &[])?;
                Ok::<_, String>((con, g))
            })
            .map_err(err)?;
        Ok(Router { con, graph, mode })
    }

    /// The route from `from_edge` to `to_edge` as JSON (`null`: not connected).
    fn route(&self, from_edge: i64, to_edge: i64) -> PyResult<String> {
        let r = duckosm_route::route::route(&self.con, &self.graph, from_edge, to_edge, &self.mode).map_err(err)?;
        Ok(text(&r.unwrap_or(J::Null)))
    }

    /// The path (edge ids) of each pair `(from_edge, to_edge)`, in parallel: the path of `route` (None: not connected).
    fn paths(&self, py: Python<'_>, pairs: Vec<(i64, i64)>) -> PyResult<Vec<Option<Vec<i64>>>> {
        use rayon::prelude::*;
        self.check(&pairs.iter().flat_map(|&(a, b)| [a, b]).collect::<Vec<_>>())?;
        let g = &self.graph;
        py.detach(|| pairs.par_iter().map(|&(a, b)| g.bidirectional_dijkstra(a, b).map(|r| r.map(|(_, p)| p))).collect::<Result<Vec<_>, _>>())
            .map_err(err)
    }

    /// Yen's `k` shortest loopless paths (edge ids) from `from_edge` to `to_edge`, the shortest first.
    fn k_shortest_paths(&self, py: Python<'_>, from_edge: i64, to_edge: i64, k: usize) -> PyResult<Vec<Vec<i64>>> {
        self.check(&[from_edge, to_edge])?;
        let g = &self.graph;
        let r = py.detach(|| g.k_shortest_paths(from_edge, to_edge, k)).map_err(err)?;
        Ok(r.into_iter().map(|(_, p)| p).collect())
    }

    /// The graph cost from each origin edge to each target edge (NaN: not reached, more than `limit`, or an edge that is not in
    /// the graph): one search for each origin, in parallel. A row for each origin.
    fn costs(&self, py: Python<'_>, origins: Vec<i64>, targets: Vec<i64>, limit: f64) -> PyResult<Vec<Vec<f64>>> {
        use rayon::prelude::*;
        let g = &self.graph;
        let at: Vec<Option<usize>> = targets.iter().map(|&t| g.index_of(t)).collect();
        Ok(py.detach(|| {
            origins
                .par_iter()
                .map(|&o| match g.index_of(o) {
                    Some(i) => {
                        let d = g.costs_from(i, limit);
                        at.iter().map(|t| t.map_or(f64::NAN, |j| d[j])).collect()
                    }
                    None => vec![f64::NAN; at.len()],
                })
                .collect()
        }))
    }

    /// The edges reached from each origin edge within `limit`, with their graph cost: `(edge ids, costs)` for each origin (empty:
    /// an edge that is not in the graph).
    fn reach(&self, py: Python<'_>, origins: Vec<i64>, limit: f64) -> PyResult<Vec<(Vec<i64>, Vec<f64>)>> {
        use rayon::prelude::*;
        let g = &self.graph;
        Ok(py.detach(|| {
            origins
                .par_iter()
                .map(|&o| {
                    let Some(i) = g.index_of(o) else { return (Vec::new(), Vec::new()) };
                    let d = g.costs_from(i, limit);
                    let hit: Vec<usize> = (0..d.len()).filter(|&i| !d[i].is_nan()).collect();
                    (hit.iter().map(|&i| g.ids[i]).collect(), hit.iter().map(|&i| d[i]).collect())
                })
                .collect()
        }))
    }
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn route_multimodal(
    py: Python<'_>,
    db: PathBuf,
    src: i64,
    dst: i64,
    schema: String,
    start_mode: String,
    end_mode: String,
    enforce_sequence: bool,
    modes: Option<Vec<String>>,
) -> PyResult<String> {
    py.detach(|| {
        let con = duckosm_route::route::open(&db)?;
        let net = duckosm_route::points::network(&con, &schema, modes.as_deref())?;
        duckosm_route::points::route_multimodal(&con, &net, Some(src), Some(dst), &schema, &start_mode, &end_mode, enforce_sequence, None)
    })
    .map(|v| text(&v))
    .map_err(err)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn route_points(py: Python<'_>, db: PathBuf, a: (f64, f64), b: (f64, f64), mode: String, weight: String, radius_m: f64, access_kmh: f64) -> PyResult<String> {
    py.detach(|| {
        let con = duckosm_route::route::open(&db)?;
        duckosm_route::points::route_points(&con, a, b, &mode, &weight, radius_m, access_kmh)
    })
    .map(|v| text(&v))
    .map_err(err)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn route_multimodal_points(
    py: Python<'_>,
    db: PathBuf,
    a: (f64, f64),
    b: (f64, f64),
    radius_m: f64,
    access_kmh: f64,
    schema: String,
    modes: Option<Vec<String>>,
    enforce_sequence: bool,
) -> PyResult<String> {
    py.detach(|| {
        let con = duckosm_route::route::open(&db)?;
        let net = duckosm_route::points::network(&con, &schema, modes.as_deref())?;
        duckosm_route::points::route_multimodal_points(&con, &net, a, b, radius_m, access_kmh, &schema, modes.as_deref(), enforce_sequence)
    })
    .map(|v| text(&v))
    .map_err(err)
}

/// `route` is a route of `route` or `route_multimodal`, as JSON.
#[pyfunction]
fn directions(py: Python<'_>, db: PathBuf, route: String, mode: String) -> PyResult<String> {
    let r: J = serde_json::from_str(&route).map_err(|e| err(e.to_string()))?;
    py.detach(|| {
        let con = duckosm_route::route::open(&db)?;
        duckosm_route::directions::directions(&con, &r, &mode)
    })
    .map(|v| text(&v))
    .map_err(err)
}

#[pyfunction]
fn build_lane_graph(py: Python<'_>, gmns_db: PathBuf, mode: String) -> PyResult<(i64, i64, i64)> {
    py.detach(|| duckosm_route::lanes::build_lane_graph(&gmns_db, &mode)).map_err(err)
}

#[pyfunction]
fn route_lanes(py: Python<'_>, gmns_db: PathBuf, from_lane: String, to_lane: String, mode: String) -> PyResult<String> {
    py.detach(|| duckosm_route::lanes::route_lanes(&gmns_db, &from_lane, &to_lane, &mode)).map(|v| text(&v)).map_err(err)
}

/// The rows of the junction graph and the turn graph of `mode` (duckosm_route::graphs), for `duckosm.to_rustworkx`: the junctions
/// `(node_id, x, y)`, the roads `(source, target, edge_id, columns as JSON)` and the turns `(from_edge, to_edge, weight)`.
#[pyfunction]
#[allow(clippy::type_complexity)]
fn graph_rows(py: Python<'_>, db: PathBuf, mode: String, weight: String)
              -> PyResult<(Vec<(i64, Option<f64>, Option<f64>)>, Vec<(i64, i64, i64, String)>, Vec<(i64, i64, f64)>)> {
    let ((js, roads), turns) = py.detach(|| duckosm_route::graphs::rows_of(&db, &mode, &weight)).map_err(err)?;
    Ok((js.into_iter().map(|j| (j.node_id, j.x, j.y)).collect(),
        roads.into_iter().map(|r| (r.source, r.target, r.edge_id, serde_json::Value::Object(r.columns).to_string())).collect(),
        turns))
}

pub fn add(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(cli, m)?)?;
    m.add_function(wrap_pyfunction!(to_sumo, m)?)?;
    m.add_function(wrap_pyfunction!(to_gis, m)?)?;
    m.add_function(wrap_pyfunction!(to_matsim, m)?)?;
    m.add_function(wrap_pyfunction!(to_matsim_lanes, m)?)?;
    m.add_function(wrap_pyfunction!(to_opendrive, m)?)?;
    m.add_function(wrap_pyfunction!(to_railml, m)?)?;
    m.add_function(wrap_pyfunction!(to_lanelet2, m)?)?;
    m.add_function(wrap_pyfunction!(to_elevation, m)?)?;
    m.add_function(wrap_pyfunction!(route_multimodal, m)?)?;
    m.add_function(wrap_pyfunction!(route_points, m)?)?;
    m.add_function(wrap_pyfunction!(graph_rows, m)?)?;
    m.add_function(wrap_pyfunction!(route_multimodal_points, m)?)?;
    m.add_function(wrap_pyfunction!(directions, m)?)?;
    m.add_function(wrap_pyfunction!(build_lane_graph, m)?)?;
    m.add_function(wrap_pyfunction!(route_lanes, m)?)?;
    m.add_class::<Router>()?;
    Ok(())
}
