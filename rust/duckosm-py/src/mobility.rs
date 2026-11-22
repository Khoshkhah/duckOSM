//! The Rust part of `duckosm.mobility` (src/duckosm/mobility.py): snap points, match GPS traces and map edge ids between
//! two builds, on the edges of one mode (`duckosm_core::matching`). Python releases the GIL while Rust works.

use duckosm_core::matching::{self as mm, MatchOptions, Net, Road};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::path::PathBuf;

fn err(e: impl ToString) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

fn roads(db: &std::path::Path, mode: &str) -> Result<Vec<Road>, String> {
    let (_, edges) = crate::analysis::read(db, mode)?;
    Ok(edges.into_iter().map(|e| Road { edge_id: e.edge_id, osm_id: e.osm_id, source: e.source, target: e.target, length_m: e.length_m, line: e.line }).collect())
}

/// The legal turns of the mode (`edge_graph`) as edge indexes of `r` (the edges sorted by `edge_id`); None without
/// the table.
fn turns(db: &std::path::Path, mode: &str, r: &[Road]) -> Result<Option<Vec<(usize, usize)>>, String> {
    let con = duckosm_route::route::open(db)?;
    let n: i64 = con
        .query_row("SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? AND table_name = 'edge_graph'", [mode], |x| x.get(0))
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    let index: std::collections::HashMap<i64, usize> = r.iter().enumerate().map(|(i, e)| (e.edge_id, i)).collect();
    let mut st = con.prepare(&format!("SELECT DISTINCT from_edge, to_edge FROM {mode}.edge_graph")).map_err(|e| e.to_string())?;
    let rows = st.query_map([], |x| Ok((x.get::<_, i64>(0)?, x.get::<_, i64>(1)?))).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (a, b) = row.map_err(|e| e.to_string())?;
        if let (Some(&a), Some(&b)) = (index.get(&a), index.get(&b)) {
            out.push((a, b));
        }
    }
    Ok(Some(out))
}

/// The points on the edges, as columns: edge ids (-1: none), fractions, distances, lon, lat.
type Snapped = (Vec<i64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>);

fn columns(r: &[Road], on: impl Iterator<Item = Option<mm::OnEdge>>) -> Snapped {
    let mut out: Snapped = Default::default();
    for p in on {
        match p {
            Some(p) => {
                out.0.push(r[p.edge].edge_id);
                out.1.push(p.fraction);
                out.2.push(p.distance_m);
                out.3.push(p.at.0);
                out.4.push(p.at.1);
            }
            None => {
                out.0.push(-1);
                for c in [&mut out.1, &mut out.2, &mut out.3, &mut out.4] {
                    c.push(f64::NAN);
                }
            }
        }
    }
    out
}

/// The nearest edge of each point (`xs`, `ys` in lon, lat) and the point on it. `radius_m`: no edge further away
/// (edge id -1).
#[pyfunction]
#[pyo3(signature = (db, mode, xs, ys, radius_m = None))]
fn mobility_snap(py: Python<'_>, db: PathBuf, mode: String, xs: Vec<f64>, ys: Vec<f64>, radius_m: Option<f64>) -> PyResult<Snapped> {
    if xs.len() != ys.len() {
        return Err(PyValueError::new_err("x and y must have the same length"));
    }
    py.detach(|| {
        let r = roads(&db, &mode)?;
        let net = Net::new(&r);
        let on = xs.iter().zip(&ys).map(|(&x, &y)| net.nearest((x, y)).filter(|p| radius_m.is_none_or(|m| p.distance_m <= m)));
        Ok::<_, String>(columns(&r, on))
    })
    .map_err(err)
}

/// Map matching of traces. `trace`: the trace number of each point (the points of a trace in time order, the traces one
/// after the other). Returns (the columns of the points, the route rows: trace number and edge id).
#[pyfunction]
#[pyo3(signature = (db, mode, trace, xs, ys, radius_m = 50.0, sigma_m = 5.0, beta_m = 10.0, max_route_factor = 4.0, uturn_m = 50.0, method = "hmm".to_string(), ts = None))]
#[allow(clippy::too_many_arguments)]
fn mobility_match(
    py: Python<'_>,
    db: PathBuf,
    mode: String,
    trace: Vec<i64>,
    xs: Vec<f64>,
    ys: Vec<f64>,
    radius_m: f64,
    sigma_m: f64,
    beta_m: f64,
    max_route_factor: f64,
    uturn_m: f64,
    method: String,
    ts: Option<Vec<i64>>,
) -> PyResult<(Snapped, (Vec<i64>, Vec<i64>))> {
    if xs.len() != ys.len() || xs.len() != trace.len() {
        return Err(PyValueError::new_err("trace, x and y must have the same length"));
    }
    let dtw = match method.as_str() {
        "hmm" => false,
        "dtw" => true,
        m => return Err(PyValueError::new_err(format!("method {m:?}: give \"hmm\" or \"dtw\""))),
    };
    if dtw && ts.as_ref().is_none_or(|t| t.len() != xs.len()) {
        return Err(PyValueError::new_err("method \"dtw\" needs the time of each point (whole seconds)"));
    }
    if radius_m <= 0.0 || sigma_m <= 0.0 || beta_m <= 0.0 {
        return Err(PyValueError::new_err("radius_m, sigma_m and beta_m must be larger than 0"));
    }
    let o = MatchOptions { radius_m, sigma_m, beta_m, max_route_factor, uturn_m };
    py.detach(|| {
        let r = roads(&db, &mode)?;
        let net = Net::new(&r);
        let dtw_net = if dtw { Some(mm::dtw_network(&net, turns(&db, &mode, &r)?)) } else { None };
        let mut points = Vec::with_capacity(xs.len());
        let mut route = (Vec::new(), Vec::new());
        let mut start = 0;
        while start < trace.len() {
            let end = (start..trace.len()).find(|&i| trace[i] != trace[start]).unwrap_or(trace.len());
            let pts: Vec<(f64, f64)> = (start..end).map(|i| (xs[i], ys[i])).collect();
            let m = match (&dtw_net, &ts) {
                (Some(d), Some(t)) => mm::match_trace_dtw(&net, d, &pts, &t[start..end], radius_m, &mm::DtwOptions::default())
                    .map_err(|e| format!("trace {}: {e}", trace[start]))?,
                _ => mm::match_trace(&net, &pts, &o),
            };
            points.extend(m.points);
            for e in m.route {
                route.0.push(trace[start]);
                route.1.push(r[e].edge_id);
            }
            start = end;
        }
        Ok::<_, String>((columns(&r, points.into_iter()), route))
    })
    .map_err(err)
}

/// The edge ids of the old build `old` on the edges of the new build `new` (same mode): (old ids, new ids, shares).
#[pyfunction]
#[pyo3(signature = (old, new, mode, tol_m = 2.0))]
fn mobility_edge_id_map(py: Python<'_>, old: PathBuf, new: PathBuf, mode: String, tol_m: f64) -> PyResult<(Vec<i64>, Vec<i64>, Vec<f64>)> {
    py.detach(|| {
        let (a, b) = (roads(&old, &mode)?, roads(&new, &mode)?);
        let rows = mm::edge_id_map(&a, &b, tol_m);
        Ok::<_, String>((rows.iter().map(|r| r.0).collect(), rows.iter().map(|r| r.1).collect(), rows.iter().map(|r| r.2).collect()))
    })
    .map_err(err)
}

pub fn add(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(mobility_snap, m)?)?;
    m.add_function(wrap_pyfunction!(mobility_match, m)?)?;
    m.add_function(wrap_pyfunction!(mobility_edge_id_map, m)?)?;
    Ok(())
}
