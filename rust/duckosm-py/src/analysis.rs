//! The Rust part of `duckosm.analysis` (src/duckosm/analysis.py): read the edges and the nodes of one mode from a built
//! database and run the algorithms of `duckosm_core::analysis`. Python releases the GIL while Rust works.

use duckosm_core::analysis::{self as an, Edge, Flat, Node, View};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use serde_json::{json, Value as J};
use std::path::PathBuf;

fn err(e: impl ToString) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// The nodes and the edges of `mode`, ordered by id.
pub fn read(db: &std::path::Path, mode: &str) -> Result<(Vec<Node>, Vec<Edge>), String> {
    if mode.is_empty() || !mode.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{mode:?} is not a mode name"));
    }
    let con = duckosm_route::route::open(db)?;
    let mut nodes = Vec::new();
    let mut st = con.prepare(&format!("SELECT node_id, ST_X(geom), ST_Y(geom) FROM {mode}.nodes ORDER BY node_id")).map_err(|e| e.to_string())?;
    let mut rows = st.query([]).map_err(|e| e.to_string())?;
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        nodes.push(Node { id: r.get(0).map_err(|e| e.to_string())?, x: r.get(1).map_err(|e| e.to_string())?, y: r.get(2).map_err(|e| e.to_string())? });
    }
    let mut edges = Vec::new();
    let mut st = con
        .prepare(&format!("SELECT edge_id, osm_id, source, target, length_m::DOUBLE, ST_AsWKB(geometry) FROM {mode}.edges ORDER BY edge_id"))
        .map_err(|e| e.to_string())?;
    let mut rows = st.query([]).map_err(|e| e.to_string())?;
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        let wkb: Vec<u8> = r.get(5).map_err(|e| e.to_string())?;
        edges.push(Edge {
            edge_id: r.get(0).map_err(|e| e.to_string())?,
            osm_id: r.get::<_, Option<i64>>(1).map_err(|e| e.to_string())?.unwrap_or(0),
            source: r.get(2).map_err(|e| e.to_string())?,
            target: r.get(3).map_err(|e| e.to_string())?,
            length_m: r.get(4).map_err(|e| e.to_string())?,
            line: duckosm_route::points::line_of_wkb(&wkb)?,
        });
    }
    Ok((nodes, edges))
}

fn view(db: &std::path::Path, mode: &str) -> Result<(Vec<i64>, Vec<Edge>, View), String> {
    let (nodes, edges) = read(db, mode)?;
    let (ids, v) = an::duckosm_view(&nodes, &edges);
    Ok((ids, edges, v))
}

fn stats_json(s: &an::Stats) -> J {
    let mut v = json!({
        "n": s.n, "m": s.m, "k_avg": s.k_avg, "edge_length_total": s.edge_length_total, "edge_length_avg": s.edge_length_avg,
        "streets_per_node_avg": s.streets_per_node_avg, "streets_per_node_counts": s.streets_per_node_counts,
        "streets_per_node_proportions": s.streets_per_node_proportions, "intersection_count": s.intersection_count,
        "street_length_total": s.street_length_total, "street_segment_count": s.street_segment_count,
        "street_length_avg": s.street_length_avg, "circuity_avg": s.circuity_avg, "self_loop_proportion": s.self_loop_proportion,
    });
    if let Some([nd, id, ed, sd]) = s.densities {
        v["node_density_km"] = json!(nd);
        v["intersection_density_km"] = json!(id);
        v["edge_density_km"] = json!(ed);
        v["street_density_km"] = json!(sd);
    }
    v
}

/// The OSMnx `basic_stats` of the two views of `mode`, as JSON: `{"duckosm": {...}, "osmnx": {...}}`.
#[pyfunction]
#[pyo3(signature = (db, mode, area_m2 = None))]
fn analysis_stats(py: Python<'_>, db: PathBuf, mode: String, area_m2: Option<f64>) -> PyResult<String> {
    py.detach(|| {
        let (_, _, v) = view(&db, &mode)?;
        let (o, spn) = (an::simplify(&v), an::streets_per_node(&v));
        Ok::<_, String>(json!({"duckosm": stats_json(&an::stats(&v, &spn, area_m2)), "osmnx": stats_json(&an::stats(&o, &spn, area_m2))}).to_string())
    })
    .map_err(err)
}

/// The streets at each node of the duckOSM view: (node ids, counts).
#[pyfunction]
fn analysis_streets_per_node(py: Python<'_>, db: PathBuf, mode: String) -> PyResult<(Vec<i64>, Vec<usize>)> {
    py.detach(|| {
        let (ids, _, v) = view(&db, &mode)?;
        Ok::<_, String>((ids, an::streets_per_node(&v)))
    })
    .map_err(err)
}

/// The bearing of each edge, from its source node to its target node (NaN for a self-loop): (edge ids, degrees).
#[pyfunction]
fn analysis_bearings(py: Python<'_>, db: PathBuf, mode: String) -> PyResult<(Vec<i64>, Vec<f64>)> {
    py.detach(|| {
        let (_, edges, v) = view(&db, &mode)?;
        let b = v.segs.iter().map(|s| if s.u == s.v { f64::NAN } else { an::bearing(v.xy[s.u], v.xy[s.v]) }).collect();
        Ok::<_, String>((edges.iter().map(|e| e.edge_id).collect(), b))
    })
    .map_err(err)
}

/// The bins of the street bearings and the orientation entropy of the duckOSM view: (counts, entropy).
#[pyfunction]
#[pyo3(signature = (db, mode, num_bins = 36, min_length = 0.0, weight_by_length = false))]
fn analysis_orientation(py: Python<'_>, db: PathBuf, mode: String, num_bins: usize, min_length: f64, weight_by_length: bool) -> PyResult<(Vec<f64>, f64)> {
    if num_bins == 0 {
        return Err(PyValueError::new_err("num_bins must be 1 or more"));
    }
    py.detach(|| {
        let (_, _, v) = view(&db, &mode)?;
        Ok::<_, String>(an::orientation(&v, num_bins, min_length, weight_by_length))
    })
    .map_err(err)
}

/// The component of each node (0: the largest): (node ids, labels).
#[pyfunction]
#[pyo3(signature = (db, mode, strong = true))]
fn analysis_components(py: Python<'_>, db: PathBuf, mode: String, strong: bool) -> PyResult<(Vec<i64>, Vec<usize>)> {
    py.detach(|| {
        let (ids, _, v) = view(&db, &mode)?;
        Ok::<_, String>((ids, an::components(&v, strong)))
    })
    .map_err(err)
}

fn flat_of(xy: &[(f64, f64)]) -> Flat {
    let ys: Vec<f64> = xy.iter().map(|p| p.1).filter(|y| !y.is_nan()).collect();
    Flat::new(if ys.is_empty() { 0.0 } else { ys.iter().sum::<f64>() / ys.len() as f64 })
}

/// The nearest node to each point (`xs`, `ys` in lon, lat): (node ids, distances in metres).
#[pyfunction]
fn analysis_nearest_nodes(py: Python<'_>, db: PathBuf, mode: String, xs: Vec<f64>, ys: Vec<f64>) -> PyResult<(Vec<i64>, Vec<f64>)> {
    if xs.len() != ys.len() {
        return Err(PyValueError::new_err("x and y must have the same length"));
    }
    py.detach(|| {
        let (ids, _, v) = view(&db, &mode)?;
        let nn = an::NearestNodes::new(&v.xy, flat_of(&v.xy));
        let mut out = (Vec::with_capacity(xs.len()), Vec::with_capacity(xs.len()));
        for (&x, &y) in xs.iter().zip(&ys) {
            let (i, d) = nn.query((x, y)).ok_or("the mode has no nodes")?;
            out.0.push(ids[i]);
            out.1.push(d);
        }
        Ok::<_, String>(out)
    })
    .map_err(err)
}

/// The nearest edge to each point (`xs`, `ys` in lon, lat): (edge ids, distances in metres).
#[pyfunction]
fn analysis_nearest_edges(py: Python<'_>, db: PathBuf, mode: String, xs: Vec<f64>, ys: Vec<f64>) -> PyResult<(Vec<i64>, Vec<f64>)> {
    if xs.len() != ys.len() {
        return Err(PyValueError::new_err("x and y must have the same length"));
    }
    py.detach(|| {
        let (_, edges, v) = view(&db, &mode)?;
        let lines: Vec<Vec<(f64, f64)>> = edges.iter().map(|e| e.line.clone()).collect();
        let ne = an::NearestEdges::new(&lines, flat_of(&v.xy));
        let mut out = (Vec::with_capacity(xs.len()), Vec::with_capacity(xs.len()));
        for (&x, &y) in xs.iter().zip(&ys) {
            let (i, d) = ne.query((x, y)).ok_or("the mode has no edges")?;
            out.0.push(edges[i].edge_id);
            out.1.push(d);
        }
        Ok::<_, String>(out)
    })
    .map_err(err)
}

/// `n` random points on the streets of `mode` (duckosm_core::analysis::sample_points): (edge ids, lons, lats).
#[pyfunction]
fn analysis_sample_points(py: Python<'_>, db: PathBuf, mode: String, n: usize, seed: u64) -> PyResult<(Vec<i64>, Vec<f64>, Vec<f64>)> {
    py.detach(|| {
        let (_, edges) = read(&db, &mode)?;
        let p = an::sample_points(&edges, n, seed);
        Ok::<_, String>((p.iter().map(|q| q.0).collect(), p.iter().map(|q| q.1).collect(), p.iter().map(|q| q.2).collect()))
    })
    .map_err(err)
}

/// The grade of each edge of `mode`, `(z_to - z_from) / length_m` (None for a length of 0): (edge ids, grades). Needs the heights.
#[pyfunction]
fn analysis_grades(py: Python<'_>, db: PathBuf, mode: String) -> PyResult<(Vec<i64>, Vec<Option<f64>>)> {
    py.detach(|| {
        if mode.is_empty() || !mode.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("{mode:?} is not a mode name"));
        }
        let con = duckosm_route::route::open(&db)?;
        let e = |e: duckdb::Error| e.to_string();
        let n: i64 = con
            .query_row("SELECT count(*) FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'edges' AND column_name IN ('z_from', 'z_to')",
                       [&mode], |r| r.get(0))
            .map_err(e)?;
        if n < 2 {
            return Err(format!("{mode}.edges has no z_from and z_to: run duckosm.analysis.add_elevation(net) first"));
        }
        let mut st = con.prepare(&format!("SELECT edge_id, (z_to - z_from) / nullif(length_m, 0) FROM {mode}.edges ORDER BY edge_id")).map_err(e)?;
        let rows: Vec<(i64, Option<f64>)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
        Ok(rows.into_iter().unzip())
    })
    .map_err(err)
}

/// `duckosm.analysis.largest_component`: the file `out` with only the largest component of `mode` (duckosm_tools::diff::keep_nodes).
#[pyfunction]
fn analysis_largest_component(py: Python<'_>, db: PathBuf, mode: String, strong: bool, out: PathBuf) -> PyResult<()> {
    py.detach(|| {
        let (ids, _, v) = view(&db, &mode)?;
        let keep: Vec<i64> = ids.iter().zip(an::components(&v, strong)).filter(|(_, c)| *c == 0).map(|(i, _)| *i).collect();
        duckosm_tools::diff::keep_nodes(&db, &out, &mode, &keep)
    })
    .map_err(err)
}

/// `duckosm.analysis.to_osmnx`: the pickle of the OSMnx graph of `mode` (duckosm_route::graphml::osmnx_graph); `name` names the graph.
#[pyfunction]
fn analysis_osmnx_pickle(py: Python<'_>, db: PathBuf, mode: String, name: String) -> PyResult<Vec<u8>> {
    py.detach(|| {
        let (ids, _, v) = view(&db, &mode)?;
        let streets = ids.into_iter().zip(an::streets_per_node(&v)).collect();
        let con = duckosm_route::route::open(&db)?;
        Ok::<_, String>(duckosm_route::graphml::osmnx_graph(&con, &mode, &name, &streets)?.pickle(true))
    })
    .map_err(err)
}

pub fn add(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(analysis_osmnx_pickle, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_sample_points, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_grades, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_largest_component, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_stats, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_streets_per_node, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_bearings, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_orientation, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_components, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_nearest_nodes, m)?)?;
    m.add_function(wrap_pyfunction!(analysis_nearest_edges, m)?)?;
    Ok(())
}
