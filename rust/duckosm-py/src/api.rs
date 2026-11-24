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

/// `<mode>.chains` in the database `db` (the Python engine calls it after its build); gives the number of chains.
#[pyfunction]
fn chains(py: Python<'_>, db: PathBuf, mode: String) -> PyResult<usize> {
    py.detach(|| duckosm_build::after::chains(&db, &mode)).map_err(err)
}

#[pyfunction]
#[pyo3(signature = (db, out_dir, mode, net_name, connections, config, netconvert, merge = false))]
#[allow(clippy::too_many_arguments)]
fn to_sumo(py: Python<'_>, db: PathBuf, out_dir: PathBuf, mode: String, net_name: String, connections: bool, config: Option<PathBuf>, netconvert: bool, merge: bool) -> PyResult<String> {
    let o = duckosm_export::sumo::SumoOptions { merge, ..Default::default() };
    let s = py
        .detach(|| duckosm_export::sumo::to_sumo_with(&db, &out_dir, &mode, &net_name, connections, config.as_deref(), netconvert, &o))
        .map_err(err)?;
    let mut v = json!({"nod": s.nod, "edg": s.edg, "n_nodes": s.n_nodes, "n_edges": s.n_edges, "n_connections": s.n_connections,
                       "n_traffic_lights": s.n_traffic_lights, "n_stop_signs": s.n_stop_signs, "n_bus_stops": s.n_bus_stops});
    for (k, p) in [("con", s.con), ("add", s.add), ("netccfg", s.netccfg), ("net", s.net), ("tll", s.tll)] {
        if let Some(p) = p {
            v[k] = json!(p);
        }
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
#[pyo3(signature = (db, out, mode, crs, gzip, merge=false))]
fn to_matsim(py: Python<'_>, db: PathBuf, out: PathBuf, mode: String, crs: Option<String>, gzip: bool, merge: bool) -> PyResult<String> {
    let s = py.detach(|| duckosm_export::matsim::to_matsim(&db, &out, &mode, crs.as_deref(), gzip, merge)).map_err(err)?;
    Ok(text(&json!({"nodes": s.nodes, "links": s.links, "crs": s.crs})))
}

#[pyfunction]
#[pyo3(signature = (gmns_db, out_dir, mode, signals, cycle_s, merge=false))]
fn to_matsim_lanes(py: Python<'_>, gmns_db: PathBuf, out_dir: PathBuf, mode: String, signals: bool, cycle_s: i64, merge: bool) -> PyResult<String> {
    let s = py.detach(|| duckosm_export::matsim_lanes::to_matsim_lanes(&gmns_db, &out_dir, &mode, signals, cycle_s, merge)).map_err(err)?;
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
    /// `weight`: `time`, `length` or a numeric column of `<mode>.edges`; `costs`: your own cost of each edge `(edge_id, cost)`
    /// (then `weight` is only its name). `missing`: an edge with no own cost costs its `time`, or `skip`: it is not used.
    #[new]
    #[pyo3(signature = (db, mode, weight, costs=None, missing="time".to_string()))]
    fn new(py: Python<'_>, db: PathBuf, mode: String, weight: String, costs: Option<Vec<(i64, f64)>>, missing: String) -> PyResult<Self> {
        let (con, graph) = py
            .detach(|| {
                let con = duckosm_route::route::open(&db)?;
                let g = match own_costs(&con, &mode, &weight, costs, &missing)? {
                    None => duckosm_route::route::routing_graph(&con, &mode, &weight, &[])?,
                    Some(o) => duckosm_route::route::routing_graph_own(&con, &mode, &[], &o)?,
                };
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

/// The own costs of `weight` and `costs`: None for `time` and `length` (the router of today), else the costs of a column or
/// the given costs.
fn own_costs(con: &duckdb::Connection, mode: &str, weight: &str, costs: Option<Vec<(i64, f64)>>, missing: &str) -> Result<Option<duckosm_route::route::Own>, String> {
    let skip = match missing {
        "time" => false,
        "skip" => true,
        m => return Err(format!("missing must be 'time' or 'skip', not {m:?}")),
    };
    let costs = match costs {
        Some(c) => c.into_iter().collect(),
        None if weight == "time" || weight == "length" => return Ok(None),
        None => duckosm_route::route::column_costs(con, mode, weight)?,
    };
    Ok(Some(duckosm_route::route::Own { costs, skip }))
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (db, a, b, mode, weight, radius_m, access_kmh, costs=None, missing="time".to_string()))]
fn route_points(
    py: Python<'_>,
    db: PathBuf,
    a: (f64, f64),
    b: (f64, f64),
    mode: String,
    weight: String,
    radius_m: f64,
    access_kmh: f64,
    costs: Option<Vec<(i64, f64)>>,
    missing: String,
) -> PyResult<String> {
    py.detach(|| {
        let con = duckosm_route::route::open(&db)?;
        let own = own_costs(&con, &mode, &weight, costs, &missing)?;
        duckosm_route::points::route_points_own(&con, a, b, &mode, &weight, own.as_ref(), radius_m, access_kmh)
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

/// `to_gmns`: the GMNS db `out` from the duckOSM db `source` (duckosm_export::gmns::to_gmns_with, as `duckosm gmns`).
#[pyfunction]
#[pyo3(signature = (source, out, modes, to_csv, lane_geometry, combined, drive_side, pair_carriageways, csv_extensions, gtfs, gtfs_max_m, walk_frame, walk_clearance_m, tile_zoom, lane_source))]
#[allow(clippy::too_many_arguments)]
fn to_gmns(
    py: Python<'_>,
    source: PathBuf,
    out: PathBuf,
    modes: Vec<String>,
    to_csv: Option<PathBuf>,
    lane_geometry: bool,
    combined: bool,
    drive_side: String,
    pair_carriageways: bool,
    csv_extensions: bool,
    gtfs: Vec<PathBuf>,
    gtfs_max_m: f64,
    walk_frame: bool,
    walk_clearance_m: f64,
    tile_zoom: Option<u8>,
    lane_source: String,
) -> PyResult<String> {
    let o = duckosm_export::gmns::GmnsOptions {
        modes,
        lane_geometry,
        pair_carriageways,
        drive_side,
        combined,
        to_csv: to_csv.clone(),
        csv_extensions,
        gtfs,
        gtfs_max_m,
        walk_frame,
        walk_clearance_m,
        tile_zoom,
        lane_source,
    };
    let s = py.detach(|| duckosm_export::gmns::to_gmns_with(&source, &out, &o)).map_err(err)?;
    let modes: serde_json::Map<String, J> = s
        .modes
        .iter()
        .map(|m| (m.mode.clone(), J::Object(m.counts.iter().map(|(t, n)| (t.to_string(), json!(n))).collect())))
        .collect();
    Ok(text(&json!({
        "path": out,
        "csv": to_csv,
        "combined": s.combined.map(|(n, l)| json!({"node": n, "link": l})),
        "modes": modes,
    })))
}

/// `to_meso`: the meso network of each mode of a GMNS db; per mode its counts.
#[pyfunction]
fn to_meso(py: Python<'_>, gmns_db: PathBuf, modes: Vec<String>, trim_m: f64) -> PyResult<String> {
    let r = py.detach(|| duckosm_export::gmns_meso::to_meso(&gmns_db, &modes, trim_m)).map_err(err)?;
    let v: serde_json::Map<String, J> =
        r.into_iter().map(|(m, n, a, b)| (m, json!({"meso_node": n, "normal": a, "movement": b, "meso_link": a + b}))).collect();
    Ok(text(&J::Object(v)))
}

/// `to_micro`: the micro (cell) network of each mode of a GMNS db; per mode its counts.
#[pyfunction]
fn to_micro(py: Python<'_>, gmns_db: PathBuf, modes: Vec<String>, cell_length_m: f64) -> PyResult<String> {
    let r = py.detach(|| duckosm_export::gmns_meso::to_micro(&gmns_db, &modes, cell_length_m)).map_err(err)?;
    let v: serde_json::Map<String, J> = r
        .into_iter()
        .map(|(m, n, c, l, mv)| (m, json!({"micro_node": n, "cell": c, "lane_change": l, "movement": mv, "micro_link": c + l + mv})))
        .collect();
    Ok(text(&J::Object(v)))
}

/// `check_gmns`: the checks of a GMNS db, as [(schema, check, bad, total)].
#[pyfunction]
fn check_gmns(py: Python<'_>, path: PathBuf, modes: Vec<String>) -> PyResult<String> {
    let r = py.detach(|| duckosm_export::gmns_check::check_gmns(&path, &modes)).map_err(err)?;
    Ok(text(&json!(r.into_iter().map(|(s, c, b, t)| json!([s, c, b, t])).collect::<Vec<_>>())))
}

/// The GMNS viewer page (`duckosm gmns-viz`) of one mode; gives its path, lanes and connectors.
#[pyfunction]
fn gmns_viewer(py: Python<'_>, gmns_db: PathBuf, out: PathBuf, mode: String) -> PyResult<String> {
    let v = py.detach(|| duckosm_export::gmns_viewer::write_viewer(&gmns_db, &out, &mode)).map_err(err)?;
    Ok(text(&json!({"path": v.path, "lanes": v.lanes, "conns": v.conns})))
}

pub fn add(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(cli, m)?)?;
    m.add_function(wrap_pyfunction!(to_gmns, m)?)?;
    m.add_function(wrap_pyfunction!(to_meso, m)?)?;
    m.add_function(wrap_pyfunction!(to_micro, m)?)?;
    m.add_function(wrap_pyfunction!(check_gmns, m)?)?;
    m.add_function(wrap_pyfunction!(gmns_viewer, m)?)?;
    m.add_function(wrap_pyfunction!(to_sumo, m)?)?;
    m.add_function(wrap_pyfunction!(chains, m)?)?;
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
