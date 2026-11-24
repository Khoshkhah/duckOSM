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
#[pyo3(signature = (db, out_dir, mode, net_name, connections, config, netconvert, merge=false, bus_edges=false, lanes=None, edge_attrs=None, config_opts=None))]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn to_sumo(
    py: Python<'_>,
    db: PathBuf,
    out_dir: PathBuf,
    mode: String,
    net_name: String,
    connections: bool,
    config: Option<PathBuf>,
    netconvert: bool,
    merge: bool,
    bus_edges: bool,
    lanes: Option<std::collections::HashMap<i64, Vec<(String, f64, Option<String>)>>>,
    edge_attrs: Option<std::collections::HashMap<i64, Vec<(String, String)>>>,
    config_opts: Option<Vec<(String, String)>>,
) -> PyResult<String> {
    // the options of the Python API (the bus-only edges, each edge's lanes and attributes, netconvert options over the built-in ones)
    let o = duckosm_export::sumo::SumoOptions { merge, bus_edges, lanes, edge_attrs: edge_attrs.unwrap_or_default(), config: config_opts.unwrap_or_default() };
    let s = py
        .detach(|| duckosm_export::sumo::to_sumo_with(&db, &out_dir, &mode, &net_name, connections, config.as_deref(), netconvert, &o))
        .map_err(err)?;
    let mut v = json!({"nod": s.nod, "edg": s.edg, "n_nodes": s.n_nodes, "n_edges": s.n_edges, "n_connections": s.n_connections,
                       "n_traffic_lights": s.n_traffic_lights, "n_stop_signs": s.n_stop_signs, "n_bus_stops": s.n_bus_stops,
                       "n_path_restrictions": s.n_path_restrictions});
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
#[pyo3(signature = (db, out, rail_types=None))]
fn to_railml(py: Python<'_>, db: PathBuf, out: PathBuf, rail_types: Option<Vec<String>>) -> PyResult<String> {
    let types: Vec<String> = rail_types.unwrap_or_else(|| duckosm_export::railml::RAIL.iter().map(|t| t.to_string()).collect());
    let types: Vec<&str> = types.iter().map(String::as_str).collect();
    let s = py.detach(|| duckosm_export::railml::to_railml_types(&db, &out, &types)).map_err(err)?;
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
    weight: String,
    own: Option<duckosm_route::route::Own>,
}

impl Router {
    /// The cost of each edge in the weight of the router (`route::edge_weights`).
    fn weights(&self, ids: &[i64]) -> Result<std::collections::HashMap<i64, f64>, String> {
        duckosm_route::route::edge_weights(&self.con, &self.mode, &self.weight, self.own.as_ref(), ids)
    }

    /// The dict of the route along `path`; with own costs also `weight` (its name) and `cost` (the sum of the own costs).
    fn route_dict(&self, path: &[i64]) -> Result<J, String> {
        let mut d = duckosm_route::route::details(&self.con, path, &self.mode)?;
        if self.own.is_some() {
            let w = self.weights(path)?;
            d["weight"] = json!(self.weight);
            d["cost"] = json!(path.iter().map(|e| w.get(e).copied().unwrap_or(f64::NAN)).sum::<f64>());
        }
        Ok(d)
    }

    /// The matrix of `costs` with the cost of the target edge added (a route ends at the end of its last edge); more than
    /// `limit`: NaN.
    fn full_costs(&self, py: Python<'_>, origins: &[i64], targets: &[i64], limit: f64) -> PyResult<Vec<Vec<f64>>> {
        let last = self.weights(targets).map_err(err)?;
        let mut m = self.costs(py, origins.to_vec(), targets.to_vec(), limit)?;
        for row in &mut m {
            for (c, t) in row.iter_mut().zip(targets) {
                *c += last.get(t).copied().unwrap_or(0.0);
                if *c > limit {
                    *c = f64::NAN;
                }
            }
        }
        Ok(m)
    }
}

#[pymethods]
impl Router {
    /// `weight`: `time`, `length` or a numeric column of `<mode>.edges`; `costs`: your own cost of each edge `(edge_id, cost)`
    /// (then `weight` is only its name). `missing`: an edge with no own cost costs its `time`, or `skip`: it is not used.
    #[new]
    #[pyo3(signature = (db, mode, weight, costs=None, missing="time".to_string()))]
    fn new(py: Python<'_>, db: PathBuf, mode: String, weight: String, costs: Option<Vec<(i64, f64)>>, missing: String) -> PyResult<Self> {
        let (con, graph, own) = py
            .detach(|| {
                let con = duckosm_route::route::open(&db)?;
                let own = own_costs(&con, &mode, &weight, costs, &missing)?;
                let g = match &own {
                    None => duckosm_route::route::routing_graph(&con, &mode, &weight, &[])?,
                    Some(o) => duckosm_route::route::routing_graph_own(&con, &mode, &[], o)?,
                };
                Ok::<_, String>((con, g, own))
            })
            .map_err(err)?;
        Ok(Router { con, graph, mode, weight, own })
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

    /// The routes of the pairs `(from_edge, to_edge)` as a JSON list (`null`: not connected), in the order of the pairs.
    fn routes(&self, py: Python<'_>, pairs: Vec<(i64, i64)>) -> PyResult<String> {
        let paths = self.paths(py, pairs)?;
        let out: Vec<J> = paths.iter().map(|p| p.as_ref().map_or(Ok(J::Null), |p| self.route_dict(p))).collect::<Result<_, _>>().map_err(err)?;
        Ok(text(&J::Array(out)))
    }

    /// Yen's `k` shortest loopless routes from `from_edge` to `to_edge`, the shortest first, as a JSON list of route dicts.
    fn k_shortest_routes(&self, py: Python<'_>, from_edge: i64, to_edge: i64, k: usize) -> PyResult<String> {
        let paths = self.k_shortest_paths(py, from_edge, to_edge, k)?;
        let out: Vec<J> = paths.iter().map(|p| self.route_dict(p)).collect::<Result<_, _>>().map_err(err)?;
        Ok(text(&J::Array(out)))
    }

    /// The cost from the start of each origin edge to the end of each target edge (the cost of `route`): NaN when not reached,
    /// more than `limit`, or not in the graph. A row for each origin.
    fn matrix(&self, py: Python<'_>, origins: Vec<i64>, targets: Vec<i64>, limit: f64) -> PyResult<Vec<Vec<f64>>> {
        self.full_costs(py, &origins, &targets, limit)
    }

    /// `duckosm.routing.accessibility`: for each origin, the number of targets within each limit in `limits` (sorted), and the
    /// nearest target (its index, None when none is reached) with its cost.
    fn accessibility(&self, py: Python<'_>, origins: Vec<i64>, targets: Vec<i64>, limits: Vec<f64>)
                     -> PyResult<(Vec<Vec<usize>>, Vec<Option<usize>>, Vec<f64>)> {
        let max = limits.iter().cloned().fold(0.0, f64::max);
        let m = self.full_costs(py, &origins, &targets, max)?;
        let mut counts = Vec::new();
        let (mut nearest, mut best) = (Vec::new(), Vec::new());
        for row in &m {
            counts.push(limits.iter().map(|l| row.iter().filter(|c| **c <= *l).count()).collect());
            let pick = row.iter().enumerate().filter(|(_, c)| !c.is_nan()).min_by(|a, b| a.1.total_cmp(b.1));
            nearest.push(pick.map(|p| p.0));
            best.push(pick.map_or(f64::NAN, |p| *p.1));
        }
        Ok((counts, nearest, best))
    }

    /// `duckosm.routing.isochrones`: for each origin and each limit in `limits` (sorted): `(origin index, limit, the number of
    /// edges reached, area in km², polygon WKB)`. An edge is reached when its whole route cost (with the edge) is at most the
    /// limit; the polygon is the union of the reached edges, each with a buffer of `buffer_m` metres in the UTM zone.
    fn isochrones(&self, py: Python<'_>, origins: Vec<i64>, limits: Vec<f64>, buffer_m: f64)
                  -> PyResult<Vec<(usize, f64, usize, f64, Option<Vec<u8>>)>> {
        let max = limits.iter().cloned().fold(0.0, f64::max);
        let reach = self.reach(py, origins, max)?;
        let every: Vec<i64> = reach.iter().flat_map(|r| r.0.iter().copied()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let own = self.weights(&every).map_err(err)?;
        let utm = duckosm_route::route::utm_of(&self.con, &self.mode).map_err(err)?;
        let mut out = Vec::new();
        for (i, (hit, cost)) in reach.iter().enumerate() {
            for &l in &limits {
                let ids: Vec<i64> = hit.iter().zip(cost).filter(|(e, c)| **c + own.get(e).copied().unwrap_or(0.0) <= l).map(|(e, _)| *e).collect();
                let area = duckosm_route::route::reach_area(&self.con, &self.mode, &ids, buffer_m, &utm).map_err(err)?;
                out.push((i, l, ids.len(), area.as_ref().map_or(0.0, |a| (a.1 * 1000.0).round() / 1000.0), area.map(|a| a.0)));
            }
        }
        Ok(out)
    }

    /// The nearest edge of the graph (an edge that a route can use) to each point `(lon, lat)`.
    fn nearest(&self, points: Vec<(f64, f64)>) -> PyResult<Vec<i64>> {
        let e = |x: duckdb::Error| err(x.to_string());
        let mut st = self.con.prepare(&format!("SELECT edge_id, ST_AsWKB(geometry) FROM {}.edges ORDER BY edge_id", self.mode)).map_err(e)?;
        let mut rows = st.query([]).map_err(e)?;
        let (mut ids, mut lines) = (Vec::new(), Vec::new());
        while let Some(r) = rows.next().map_err(e)? {
            let id: i64 = r.get(0).map_err(e)?;
            if self.graph.contains(id) {
                let wkb: Vec<u8> = r.get(1).map_err(e)?;
                lines.push(duckosm_route::points::line_of_wkb(&wkb).map_err(err)?);
                ids.push(id);
            }
        }
        let ys: Vec<f64> = lines.iter().flatten().map(|p| p.1).collect();
        let flat = duckosm_core::analysis::Flat::new(if ys.is_empty() { 0.0 } else { ys.iter().sum::<f64>() / ys.len() as f64 });
        let near = duckosm_core::analysis::NearestEdges::new(&lines, flat);
        points.iter().map(|&p| near.query(p).map(|(i, _)| ids[i]).ok_or_else(|| err(format!("the {} graph has no edges", self.mode)))).collect()
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

/// The networkx graph of `mode` as a file (duckosm_route::graphml, as `duckosm export-graph`): `kind` `node` (the junction
/// MultiDiGraph) or `edge` (the turn DiGraph), `fmt` `graphml` or `gpickle`. Without `node_attrs` the turn graph has no node
/// attributes. Gives the bytes and the numbers of nodes and edges.
#[pyfunction]
#[pyo3(signature = (db, mode, kind, fmt, weight, geometry, uses=None, node_attrs=true))]
#[allow(clippy::too_many_arguments)]
fn graph_bytes(py: Python<'_>, db: PathBuf, mode: String, kind: String, fmt: String, weight: String, geometry: String,
               uses: Option<String>, node_attrs: bool) -> PyResult<(Vec<u8>, usize, usize)> {
    py.detach(|| -> Result<_, String> {
        let uses = duckosm_route::route::check_uses(uses.as_deref())?;
        let con = duckosm_route::route::open(&db)?;
        let doc = if kind == "node" {
            duckosm_route::graphml::node_graph(&con, &mode, &geometry)?
        } else {
            duckosm_route::graphml::edge_graph(&con, &mode, &weight, &uses, node_attrs)?
        };
        let bytes = if fmt == "gpickle" { doc.pickle(kind == "node") } else { doc.write().into_bytes() };
        Ok((bytes, doc.nodes.len(), doc.edges.len()))
    }).map_err(err)
}

/// `duckosm.fixes.diff`: the edges that are different in `other` (duckosm_tools::diff::diff): (mode, edge_id, change, changed columns).
#[pyfunction]
#[pyo3(signature = (db, other, mode=None))]
fn fixes_diff(py: Python<'_>, db: PathBuf, other: PathBuf, mode: Option<String>) -> PyResult<Vec<(String, i64, String, Vec<String>)>> {
    py.detach(|| duckosm_tools::diff::diff(&db, &other, mode.as_deref())).map_err(err)
}

/// `node.csv` and `link.csv` of a GMNS database, for path4gmns (duckosm_export::gmns::node_link_csv).
#[pyfunction]
fn gmns_node_link_csv(py: Python<'_>, gmns_db: PathBuf, out: PathBuf, mode: String) -> PyResult<()> {
    py.detach(|| duckosm_export::gmns::node_link_csv(&gmns_db, &out, &mode)).map_err(err)
}

/// The links of `duckosm.planning.to_aequilibrae` (duckosm_export::gmns::aequilibrae_links).
#[pyfunction]
fn aequilibrae_links(py: Python<'_>, db: PathBuf, mode: String) -> PyResult<Vec<duckosm_export::gmns::AeqLink>> {
    py.detach(|| duckosm_export::gmns::aequilibrae_links(&db, &mode)).map_err(err)
}

/// `duckosm.routing.geocode`: the `(lon, lat)` of a place name (duckosm_route::route::geocode).
#[pyfunction]
fn geocode(py: Python<'_>, db: PathBuf, mode: String, name: String) -> PyResult<(f64, f64)> {
    py.detach(|| duckosm_route::route::geocode(&duckosm_route::route::open(&db)?, &mode, &name)).map_err(err)
}

/// `duckosm.routing.routed_graph`: the SQL relation of the rows of `<mode>.edge_graph` that the mode routes on.
#[pyfunction]
fn routed_relation(py: Python<'_>, db: PathBuf, mode: String) -> PyResult<String> {
    py.detach(|| duckosm_route::route::routed_graph(&duckosm_route::route::open(&db)?, &mode, &[])).map_err(err)
}

/// The merged line of the edges `ids` of `mode` as WKB (`Route.geometry`).
#[pyfunction]
fn route_line(py: Python<'_>, db: PathBuf, mode: String, ids: Vec<i64>) -> PyResult<Vec<u8>> {
    py.detach(|| duckosm_route::route::line(&duckosm_route::route::open(&db)?, &mode, &ids)).map_err(err)
}

/// `duckosm.analysis.clip`: `duckosm extract` of the box `bbox` or of the circle `point` with `dist_m` (duckosm_tools::extract).
#[pyfunction]
#[pyo3(signature = (source, out, bbox=None, point=None, dist_m=None))]
fn extract_area(py: Python<'_>, source: PathBuf, out: PathBuf, bbox: Option<[f64; 4]>, point: Option<(f64, f64)>, dist_m: Option<f64>) -> PyResult<()> {
    let area = match (bbox, point, dist_m) {
        (Some(b), None, _) => duckosm_tools::extract::Area::Bbox(b),
        (None, Some(p), Some(m)) => duckosm_tools::extract::Area::Circle(p, m),
        _ => return Err(pyo3::exceptions::PyValueError::new_err("give bbox, or point with dist_m")),
    };
    py.detach(|| duckosm_tools::extract::extract(&source, &out, area)).map_err(err)
}

/// One lanelet of `commonroad_lanelets`.
type PyLanelet = (i64, Vec<(f64, f64)>, Vec<(f64, f64)>, Vec<(f64, f64)>, Vec<i64>, Vec<i64>, Option<i64>, Option<i64>, String, String, i64);

/// `duckosm.hdmap.to_commonroad`: the lanelets of the lanes of a GMNS database (duckosm_export::commonroad::lanelets):
/// (origin lon/lat, the numbers of lanes and connectors, the lanelets).
#[pyfunction]
#[allow(clippy::type_complexity)]
fn commonroad_lanelets(py: Python<'_>, gmns_db: PathBuf, mode: String) -> PyResult<((f64, f64), usize, usize, Vec<PyLanelet>)> {
    let m = py.detach(|| duckosm_export::commonroad::lanelets(&gmns_db, &mode)).map_err(err)?;
    let l = m.lanelets.into_iter()
        .map(|x| (x.id, x.left, x.center, x.right, x.pred, x.succ, x.adj_left, x.adj_right, x.kind, x.gmns_id, x.edge_id))
        .collect();
    Ok((m.origin, m.lanes, m.connectors, l))
}

/// `duckosm.hdmap.surfaces`: the lane and junction polygons of a GMNS database (duckosm_export::commonroad::surfaces).
#[pyfunction]
#[allow(clippy::type_complexity)]
fn hdmap_surfaces(py: Python<'_>, gmns_db: PathBuf, mode: String)
                  -> PyResult<(Vec<duckosm_export::commonroad::LaneSurface>, Vec<(i64, i64, Vec<u8>)>)> {
    py.detach(|| duckosm_export::commonroad::surfaces(&gmns_db, &mode)).map_err(err)
}

/// `duckosm.ml.graph_arrays` (duckosm_route::ml::graph_arrays): (src, dst, x, x_names, edge_attr, edge_attr_names, num_nodes,
/// edge_id, node_id); `x` and `edge_attr` row by row.
#[pyfunction]
#[allow(clippy::type_complexity)]
fn graph_arrays(py: Python<'_>, db: PathBuf, mode: String, graph: String, features: Vec<String>, node_features: Vec<String>)
                -> PyResult<(Vec<i64>, Vec<i64>, Vec<f32>, Vec<String>, Vec<f32>, Vec<String>, usize, Vec<i64>, Vec<i64>)> {
    let a = py
        .detach(|| duckosm_route::ml::graph_arrays(&duckosm_route::route::open(&db)?, &mode, &graph, &features, &node_features))
        .map_err(err)?;
    Ok((a.src, a.dst, a.x, a.x_names, a.edge_attr, a.edge_attr_names, a.num_nodes, a.edge_id, a.node_id))
}

/// `Network.plot`: the polygons of each base-map table, cut to the view (duckosm_route::plot::layers).
#[pyfunction]
fn plot_layers(py: Python<'_>, db: PathBuf, tables: Vec<String>, view: [f64; 4]) -> PyResult<Vec<Vec<Vec<u8>>>> {
    py.detach(|| duckosm_route::plot::layers(&duckosm_route::route::open(&db)?, &tables, view)).map_err(err)
}

/// `Network.plot`: the modes label of each road (duckosm_route::plot::modes_labels).
#[pyfunction]
fn modes_labels(py: Python<'_>, db: PathBuf) -> PyResult<Vec<(i64, String)>> {
    py.detach(|| duckosm_route::plot::modes_labels(&duckosm_route::route::open(&db)?)).map_err(err)
}

/// `Network.plot`: the line of a route, its edges found in any mode (duckosm_route::plot::route_union).
#[pyfunction]
fn route_union(py: Python<'_>, db: PathBuf, ids: Vec<i64>) -> PyResult<Option<Vec<u8>>> {
    py.detach(|| duckosm_route::plot::route_union(&duckosm_route::route::open(&db)?, &ids)).map_err(err)
}

/// `Network.schema`: the exact number of rows of each table (duckosm_tools::info::row_counts).
#[pyfunction]
fn row_counts(py: Python<'_>, db: PathBuf, tables: Vec<(String, String)>) -> PyResult<Vec<i64>> {
    py.detach(|| duckosm_tools::info::row_counts(&db, &tables)).map_err(err)
}

/// `duckosm.gis.to_osm_xml` (duckosm_export::osm_xml): the numbers of nodes and ways.
#[pyfunction]
fn to_osm_xml(py: Python<'_>, db: PathBuf, out: PathBuf, mode: String) -> PyResult<(i64, i64)> {
    py.detach(|| duckosm_export::osm_xml::to_osm_xml(&db, &out, &mode)).map_err(err)
}

/// `duckosm.gis.features`: the matching rows of a QuackOSM GeoParquet file (duckosm_export::osm_xml::features).
#[pyfunction]
fn gis_features(py: Python<'_>, parquet: PathBuf, tags: Vec<(String, Option<Vec<String>>)>) -> PyResult<Vec<duckosm_export::osm_xml::Feature>> {
    py.detach(|| duckosm_export::osm_xml::features(&parquet, &tags)).map_err(err)
}

/// `duckosm.dem_sources.sources_for_bbox`: the ids of the DEM sources that cover the box, best first (duckosm_tools::dem_sources).
#[pyfunction]
fn dem_sources_for_bbox(bbox: [f64; 4], include_fallback: bool, include_all: bool) -> Vec<String> {
    duckosm_tools::dem_sources::sources_for_bbox(bbox, include_fallback, include_all)
        .into_iter()
        .filter_map(|s| s["id"].as_str().map(String::from))
        .collect()
}

/// `duckosm.dem_sources.covers`: does the DEM source `id` cover the whole box.
#[pyfunction]
fn dem_source_covers(id: String, bbox: [f64; 4]) -> PyResult<bool> {
    let s = duckosm_tools::dem_sources::get(&id).ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(id.clone()))?;
    Ok(duckosm_tools::dem_sources::covers(s, bbox))
}

/// `Network.summary` (duckosm_tools::info::mode_summary): (mode, edges, nodes, km, oneway_km) for each mode.
#[pyfunction]
#[allow(clippy::type_complexity)]
fn mode_summary(py: Python<'_>, db: PathBuf, modes: Vec<String>) -> PyResult<Vec<(String, i64, i64, Option<f64>, f64)>> {
    py.detach(|| duckosm_tools::info::mode_summary(&db, &modes)).map_err(err)
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
    let mut v = json!({
        "path": out,
        "csv": to_csv,
        "combined": s.combined.map(|(n, l)| json!({"node": n, "link": l})),
        "modes": modes,
    });
    if !s.sumo.is_empty() {
        // the report of the lanes from netconvert, for each mode that used it
        v["sumo"] = J::Object(s.sumo.iter().map(|(m, r)| (m.clone(), J::Object(r.iter().map(|(k, n)| (k.clone(), json!(n))).collect()))).collect());
    }
    Ok(text(&v))
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

/// netconvert (SUMO): the program, or None when it is not found.
#[pyfunction]
fn find_netconvert() -> Option<String> {
    duckosm_export::sumo::find_netconvert().ok().map(|(p, _)| p.display().to_string())
}

/// `lane_diff` (docs/design/gmns_lane_profile.md, decision 2): SUMO from duckOSM's lane profile against SUMO's own OSM import.
#[pyfunction]
fn sumo_lane_diff(py: Python<'_>, db: PathBuf, osm_file: PathBuf, work_dir: PathBuf) -> PyResult<String> {
    let (rows, compared) = py.detach(|| duckosm_export::sumo_compare::lane_diff(&db, &osm_file, &work_dir)).map_err(err)?;
    let lanes = |l: &duckosm_export::sumo_compare::Lanes| json!(l);
    let rows: Vec<J> = rows
        .iter()
        .map(|r| json!({"way": r.way, "dir": r.dir.to_string(), "duckosm": lanes(&r.duckosm), "sumo_osm": lanes(&r.sumo_osm), "sources": r.sources, "cause": r.cause}))
        .collect();
    Ok(text(&json!({"rows": rows, "compared": compared})))
}

pub fn add(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(cli, m)?)?;
    m.add_function(wrap_pyfunction!(find_netconvert, m)?)?;
    m.add_function(wrap_pyfunction!(sumo_lane_diff, m)?)?;
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
    m.add_function(wrap_pyfunction!(graph_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(fixes_diff, m)?)?;
    m.add_function(wrap_pyfunction!(mode_summary, m)?)?;
    m.add_function(wrap_pyfunction!(dem_sources_for_bbox, m)?)?;
    m.add_function(wrap_pyfunction!(dem_source_covers, m)?)?;
    m.add_function(wrap_pyfunction!(gis_features, m)?)?;
    m.add_function(wrap_pyfunction!(to_osm_xml, m)?)?;
    m.add_function(wrap_pyfunction!(row_counts, m)?)?;
    m.add_function(wrap_pyfunction!(plot_layers, m)?)?;
    m.add_function(wrap_pyfunction!(modes_labels, m)?)?;
    m.add_function(wrap_pyfunction!(route_union, m)?)?;
    m.add_function(wrap_pyfunction!(graph_arrays, m)?)?;
    m.add_function(wrap_pyfunction!(commonroad_lanelets, m)?)?;
    m.add_function(wrap_pyfunction!(hdmap_surfaces, m)?)?;
    m.add_function(wrap_pyfunction!(extract_area, m)?)?;
    m.add_function(wrap_pyfunction!(geocode, m)?)?;
    m.add_function(wrap_pyfunction!(routed_relation, m)?)?;
    m.add_function(wrap_pyfunction!(route_line, m)?)?;
    m.add_function(wrap_pyfunction!(gmns_node_link_csv, m)?)?;
    m.add_function(wrap_pyfunction!(aequilibrae_links, m)?)?;
    m.add_function(wrap_pyfunction!(route_multimodal_points, m)?)?;
    m.add_function(wrap_pyfunction!(directions, m)?)?;
    m.add_function(wrap_pyfunction!(build_lane_graph, m)?)?;
    m.add_function(wrap_pyfunction!(route_lanes, m)?)?;
    m.add_class::<Router>()?;
    Ok(())
}
