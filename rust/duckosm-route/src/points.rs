//! duckOSM `routing.route_multimodal` and `point_routing.py` on a database: the data for the router, and the results as
//! duckOSM's dicts (JSON).

use crate::{err, Result};
use duckdb::Connection;
use duckosm_router::multimodal::{Network, Points, Prepared, Step, HUB};
use duckosm_router::points::{self, Cand, Road};
use serde_json::{json, Value as J};
use std::collections::{HashMap, HashSet};

fn table_exists(con: &Connection, schema: &str, table: &str) -> Result<bool> {
    let n: i64 = con
        .query_row("SELECT count(*) FROM information_schema.tables WHERE table_schema = ? AND table_name = ?", [schema, table], |r| r.get(0))
        .map_err(err)?;
    Ok(n > 0)
}

/// The roads, the changes of mode and the legal turns of `schema` (duckOSM reads them in `route_multimodal`), made into the
/// router's indexes. `allowed`: only these modes are read (a large area: the other modes are not in memory).
pub fn network(con: &Connection, schema: &str, allowed: Option<&[String]>) -> Result<Prepared> {
    let mut net = Network::default();
    let only = |col: &str| match allowed {
        Some(a) => format!(" WHERE {col} IN ({})", a.iter().map(|m| format!("'{}'", m.replace('\'', "''"))).collect::<Vec<_>>().join(", ")),
        None => String::new(),
    };
    let mut st = con.prepare(&format!("SELECT mode, source, target, edge_id, cost_s::DOUBLE FROM {schema}.edges{}", only("mode"))).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        let m: String = r.get(0).map_err(err)?;
        let m = net.mode(&m);
        net.edges.push((m, r.get(1).map_err(err)?, r.get(2).map_err(err)?, r.get(3).map_err(err)?, r.get(4).map_err(err)?));
    }
    let both = match allowed {
        Some(_) => format!("{} AND {}", only("from_mode"), only("to_mode").replacen(" WHERE ", "", 1)),
        None => String::new(),
    };
    let mut st = con.prepare(&format!("SELECT node_id, from_mode, to_mode, cost_s::DOUBLE, kind FROM {schema}.transfers{both}")).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        let (fm, tm): (String, String) = (r.get(1).map_err(err)?, r.get(2).map_err(err)?);
        let (fm, tm) = (net.mode(&fm), net.mode(&tm));
        net.transfers.push((r.get(0).map_err(err)?, fm, tm, r.get(3).map_err(err)?, r.get(4).map_err(err)?));
    }
    let modes: Vec<(u8, String)> = net.modes.iter().enumerate().map(|(i, m)| (i as u8, m.clone())).collect();
    for (i, m) in modes {
        if !net.edges.iter().any(|e| e.0 == i) || !table_exists(con, &m, "edge_graph")? {
            continue;
        }
        let g = crate::route::routed_graph(con, &m, &[])?;
        let mut st = con.prepare(&format!("SELECT from_edge, to_edge FROM {g}")).map_err(err)?;
        let rows: Vec<(i64, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
        net.turns.insert(i, rows);
    }
    Ok(Prepared::new(net, allowed))
}

/// duckOSM `route_multimodal`: the dict of the trip (time_s, length_m, edges, legs, transfers, nodes; with points also
/// access_s), or null.
#[allow(clippy::too_many_arguments)]
pub fn route_multimodal(
    con: &Connection,
    net: &Prepared,
    src: Option<i64>,
    dst: Option<i64>,
    schema: &str,
    start_mode: &str,
    end_mode: &str,
    enforce_sequence: bool,
    points: Option<&Points>,
) -> Result<J> {
    let steps = net
        .route(src.unwrap_or(0), dst.unwrap_or(0), start_mode, end_mode, enforce_sequence, points)
        .map_err(|e| e.replace("mm.edges", &format!("{schema}.edges")))?;
    let Some(steps) = steps else {
        return Ok(J::Null);
    };
    let mut edges_seq: Vec<(String, i64)> = Vec::new();
    let mut legs: Vec<(String, Vec<i64>, f64, Vec<f64>)> = Vec::new();
    let mut transfers: Vec<J> = Vec::new();
    let mut transfer_costs: Vec<f64> = Vec::new();
    let mut access_s = 0.0f64;
    let mut nodes: Vec<i64> = if points.is_some() { Vec::new() } else { vec![src.unwrap_or(0)] };
    for step in &steps {
        match step {
            Step::Edge(e, m, cost, frac, acc) => {
                edges_seq.push((m.clone(), *e));
                if legs.last().is_none_or(|l| &l.0 != m) {
                    legs.push((m.clone(), Vec::new(), 0.0, Vec::new()));
                }
                let l = legs.last_mut().expect("pushed");
                l.1.push(*e);
                l.3.push(*frac);
                l.2 += cost;
                access_s += acc;
                // the end of the road (duckOSM: info[(mode, edge_id)][1])
                let t = net.target(m, *e).ok_or(format!("edge {e} is not a {m} road"))?;
                if nodes.last() != Some(&t) {
                    nodes.push(t);
                }
            }
            Step::Transfer(n, fm, tm, cost, kind) => {
                transfers.push(json!({"node_id": n, "from_mode": fm, "to_mode": tm, "cost_s": cost, "kind": kind}));
                transfer_costs.push(*cost);
            }
        }
    }
    let mut detail: HashMap<(String, i64), (Option<String>, Option<String>, Option<f64>, Option<f64>, Option<String>)> = HashMap::new();
    if !edges_seq.is_empty() {
        let ids: Vec<String> = edges_seq.iter().map(|x| x.1.to_string()).collect::<HashSet<_>>().into_iter().collect();
        let mut st = con
            .prepare(&format!(
                "SELECT mode, edge_id, name, highway, length_m::DOUBLE, cost_s::DOUBLE, ST_AsText(geometry) FROM {schema}.edges WHERE edge_id IN ({})",
                ids.join(", ")
            ))
            .map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            detail.insert(
                (r.get(0).map_err(err)?, r.get(1).map_err(err)?),
                (r.get(2).map_err(err)?, r.get(3).map_err(err)?, r.get(4).map_err(err)?, r.get(5).map_err(err)?, r.get(6).map_err(err)?),
            );
        }
    }
    let mut length_m = 0.0f64;
    let mut legs_out = Vec::new();
    let mut leg_times = Vec::new();
    for (mode, es, time, fr) in legs {
        let mut path = Vec::new();
        for (e, frac) in es.iter().zip(&fr) {
            let Some((name, hw, len, cost, geom)) = detail.get(&(mode.clone(), *e)).cloned() else { continue };
            let mut row = json!({"edge_id": e, "name": name, "highway": hw, "length_m": len, "cost_s": cost, "geometry": geom});
            if *frac != 1.0 {
                row["fraction"] = json!(frac);
                row["length_m"] = json!(len.unwrap_or(0.0) * frac);
                row["cost_s"] = json!(cost.unwrap_or(0.0) * frac);
            }
            path.push(row);
            if let Some(l) = len {
                length_m += l * frac;
            }
        }
        leg_times.push(time);
        legs_out.push(json!({"mode": mode, "edges": es, "time_s": time, "path": path}));
    }
    // Python: sum(leg times) + sum(transfer costs) + access_s, each sum from 0
    let time_s = leg_times.iter().fold(0.0, |a, b| a + b) + transfer_costs.iter().fold(0.0, |a, b| a + b) + access_s;
    let mut out = json!({
        "time_s": time_s,
        "length_m": length_m,
        "edges": edges_seq.iter().map(|(m, e)| json!([m, e])).collect::<Vec<_>>(),
        "legs": legs_out,
        "transfers": transfers,
        "nodes": nodes,
    });
    if points.is_some() {
        out["access_s"] = json!(access_s);
    }
    Ok(out)
}

/// duckOSM `candidates` on the database: the roads of `schema` near `point` and the blockers (the roads of `schema` and of
/// `driving`).
pub fn candidates(con: &Connection, schema: &str, point: (f64, f64), radius_m: f64) -> Result<Vec<Cand>> {
    let (x0, y0, x1, y1) = points::bbox(point, radius_m);
    let env = format!("ST_MakeEnvelope({}, {}, {}, {})", duckosm_build::place::py_float(x0), duckosm_build::place::py_float(y0), duckosm_build::place::py_float(x1), duckosm_build::place::py_float(y1));
    let line = |wkb: Vec<u8>| -> Result<Vec<(f64, f64)>> { line_of_wkb(&wkb) };
    let mut roads = Vec::new();
    let mut st = con
        .prepare(&format!(
            "SELECT edge_id, source, target, cost_s::DOUBLE, length_m::DOUBLE, ST_AsWKB(geometry) FROM {schema}.edges WHERE ST_Intersects(geometry, {env})"
        ))
        .map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        roads.push(Road {
            edge_id: r.get(0).map_err(err)?,
            source: r.get(1).map_err(err)?,
            target: r.get(2).map_err(err)?,
            cost_s: r.get(3).map_err(err)?,
            length_m: r.get(4).map_err(err)?,
            line: line(r.get(5).map_err(err)?)?,
        });
    }
    let mut blockers = Vec::new();
    let mut seen = Vec::new();
    for sch in [schema, "driving"] {
        if seen.contains(&sch) || !table_exists(con, sch, "edges")? {
            continue;
        }
        seen.push(sch);
        let mut st = con.prepare(&format!("SELECT ST_AsWKB(geometry) FROM {sch}.edges WHERE ST_Intersects(geometry, {env})")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            blockers.push(line(r.get(0).map_err(err)?)?);
        }
    }
    Ok(points::candidates(point, radius_m, &roads, &blockers))
}

/// The points of a LINESTRING in WKB (little or big endian, 2D).
pub fn line_of_wkb(b: &[u8]) -> Result<Vec<(f64, f64)>> {
    let le = b.first() == Some(&1);
    let u32_at = |i: usize| -> Option<u32> {
        let a: [u8; 4] = b.get(i..i + 4)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    };
    let f64_at = |i: usize| -> Option<f64> {
        let a: [u8; 8] = b.get(i..i + 8)?.try_into().ok()?;
        Some(if le { f64::from_le_bytes(a) } else { f64::from_be_bytes(a) })
    };
    if u32_at(1) != Some(2) {
        return Err("a road geometry is not a LINESTRING".into());
    }
    let n = u32_at(5).ok_or("a short WKB")? as usize;
    (0..n).map(|k| Some((f64_at(9 + 16 * k)?, f64_at(17 + 16 * k)?))).collect::<Option<Vec<_>>>().ok_or_else(|| "a short WKB".into())
}

fn ends(con: &Connection, schema: &str, a: (f64, f64), b: (f64, f64), radius_m: f64) -> Result<(Vec<Cand>, Vec<Cand>)> {
    let (s, t) = (candidates(con, schema, a, radius_m)?, candidates(con, schema, b, radius_m)?);
    let r = duckosm_build::place::py_float(radius_m);
    // Python's f"{radius_m:g}"
    let r = r.strip_suffix(".0").unwrap_or(&r).to_string();
    if s.is_empty() {
        return Err(format!("no road within {r} m of the start"));
    }
    if t.is_empty() {
        return Err(format!("no road within {r} m of the end"));
    }
    Ok((s, t))
}

fn end_info(point: (f64, f64), c: &Cand, v: f64) -> J {
    json!({"point": [point.0, point.1], "road_point": [c.road_point.0, c.road_point.1], "edge_id": c.edge_id,
           "fraction": c.fraction, "access_m": c.access_m, "access_s": c.access_m / v})
}

/// duckOSM `route_points`: the dict of the route between two points (lon, lat) in one mode, or null.
pub fn route_points(con: &Connection, a: (f64, f64), b: (f64, f64), mode: &str, weight: &str, radius_m: f64, access_kmh: f64) -> Result<J> {
    route_points_own(con, a, b, mode, weight, None, radius_m, access_kmh)
}

/// `route_points` with your own cost of each edge (`own`; `weight` then only names it in the result as `weight`, and the
/// result also has `cost`: the own cost of the used parts of the edges; the access legs cost 0).
#[allow(clippy::too_many_arguments)]
pub fn route_points_own(
    con: &Connection,
    a: (f64, f64),
    b: (f64, f64),
    mode: &str,
    weight: &str,
    own: Option<&crate::route::Own>,
    radius_m: f64,
    access_kmh: f64,
) -> Result<J> {
    if own.is_none() && weight != "time" && weight != "length" {
        return Err("weight must be 'time' or 'length'".into());
    }
    let (mut s, mut t) = ends(con, mode, a, b, radius_m)?;
    let v = access_kmh / 3.6;
    let key = if weight == "length" && own.is_none() { "length_m" } else { "cost_s" };
    let mut w: HashMap<i64, f64> = HashMap::new();
    let mut st = con.prepare(&format!("SELECT edge_id, {key}::DOUBLE FROM {mode}.edges")).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        let (e, x): (i64, Option<f64>) = (r.get(0).map_err(err)?, r.get(1).map_err(err)?);
        match own {
            Some(o) => match o.costs.get(&e) {
                Some(&c) => _ = w.insert(e, c),
                None if o.skip => {}
                None => _ = w.insert(e, x.unwrap_or(0.0)),
            },
            None => _ = w.insert(e, x.unwrap_or(0.0)),
        }
    }
    let g = crate::route::routed_graph(con, mode, &[])?;
    let mut next: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut st = con.prepare(&format!("SELECT from_edge, to_edge FROM {g}")).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        let (x, y): (i64, i64) = (r.get(0).map_err(err)?, r.get(1).map_err(err)?);
        if own.is_some() && !(w.contains_key(&x) && w.contains_key(&y)) {
            continue;
        }
        next.entry(x).or_default().push(y);
    }
    // own costs: each candidate costs its own cost, and the access legs cost nothing (an infinite access speed)
    let mut v_route = v;
    if own.is_some() {
        for c in s.iter_mut().chain(t.iter_mut()) {
            c.cost_s = w.get(&c.edge_id).copied().unwrap_or(f64::NAN);
        }
        s.retain(|c| !c.cost_s.is_nan());
        t.retain(|c| !c.cost_s.is_nan());
        v_route = f64::INFINITY;
    }
    let Some(r) = points::route_points(&s, &t, weight == "time" || own.is_some(), v_route, &w, &next) else { return Ok(J::Null) };
    let ids: Vec<String> = r.path.iter().map(i64::to_string).collect::<HashSet<_>>().into_iter().collect();
    let mut info: HashMap<i64, (Option<String>, Option<String>, Option<f64>, Option<f64>)> = HashMap::new();
    let mut st = con
        .prepare(&format!("SELECT edge_id, name, highway, length_m::DOUBLE, cost_s::DOUBLE FROM {mode}.edges WHERE edge_id IN ({})", ids.join(", ")))
        .map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(x) = rows.next().map_err(err)? {
        info.insert(x.get(0).map_err(err)?, (x.get(1).map_err(err)?, x.get(2).map_err(err)?, x.get(3).map_err(err)?, x.get(4).map_err(err)?));
    }
    let mut rows_out = Vec::new();
    let (mut sum_cost, mut sum_len, mut sum_own) = (0.0f64, 0.0f64, 0.0f64);
    for (e, (f0, f1)) in r.path.iter().zip(&r.parts) {
        let (name, hw, len, cost) = info[e].clone();
        let share = f1 - f0;
        let (l, c) = (len.unwrap_or(0.0) * share, cost.unwrap_or(0.0) * share);
        sum_cost += c;
        sum_len += l;
        sum_own += w.get(e).copied().unwrap_or(0.0) * share;
        rows_out.push(json!({"edge_id": e, "name": name, "highway": hw, "from_fraction": f0, "to_fraction": f1, "length_m": l, "cost_s": c}));
    }
    let (start, end) = (end_info(a, &r.start, v), end_info(b, &r.end, v));
    if own.is_some() {
        return Ok(json!({
            "time_s": start["access_s"].as_f64().unwrap_or(0.0) + sum_cost + end["access_s"].as_f64().unwrap_or(0.0),
            "length_m": r.start.access_m + sum_len + r.end.access_m,
            "weight": weight,
            "cost": sum_own,
            "edges": r.path,
            "start": start,
            "end": end,
            "path": rows_out,
        }));
    }
    Ok(json!({
        "time_s": start["access_s"].as_f64().unwrap_or(0.0) + sum_cost + end["access_s"].as_f64().unwrap_or(0.0),
        "length_m": r.start.access_m + sum_len + r.end.access_m,
        "edges": r.path,
        "start": start,
        "end": end,
        "path": rows_out,
    }))
}

/// duckOSM `route_multimodal_points`: the trip between two points, starting and ending walking, or null.
#[allow(clippy::too_many_arguments)]
pub fn route_multimodal_points(
    con: &Connection,
    net: &Prepared,
    a: (f64, f64),
    b: (f64, f64),
    radius_m: f64,
    access_kmh: f64,
    schema: &str,
    allowed: Option<&[String]>,
    enforce_sequence: bool,
) -> Result<J> {
    if allowed.is_some_and(|m| !m.is_empty() && !m.iter().any(|x| x == HUB)) {
        return Err("a trip between two points starts and ends walking: allowed_modes needs 'walking'".into());
    }
    let (s, t) = ends(con, HUB, a, b, radius_m)?;
    let v = access_kmh / 3.6;
    let pts = Points {
        starts: s.iter().map(|c| (c.edge_id, c.fraction, c.access_m / v)).collect(),
        ends: t.iter().map(|c| (c.edge_id, (c.fraction, c.access_m / v))).collect(),
    };
    let mut tby: HashMap<i64, &Cand> = HashMap::new();
    for c in &t {
        tby.insert(c.edge_id, c);
    }
    let mut r = route_multimodal(con, net, None, None, schema, HUB, HUB, enforce_sequence, Some(&pts))?;
    if r.is_null() {
        return Ok(r);
    }
    let edges = r["edges"].as_array().cloned().unwrap_or_default();
    let (first, last) = (edges[0][1].as_i64().unwrap_or(0), edges[edges.len() - 1][1].as_i64().unwrap_or(0));
    let sc = s.iter().find(|c| c.edge_id == first).ok_or("the first road is not a start candidate")?;
    let tc = tby.get(&last).copied().or_else(|| t.iter().find(|c| c.edge_id == last)).ok_or("the last road is not an end candidate")?;
    r["start"] = end_info(a, sc, v);
    r["end"] = end_info(b, tc, v);
    // Python: r["length_m"] += start access_m + end access_m (the two first)
    let l = r["length_m"].as_f64().unwrap_or(0.0) + (sc.access_m + tc.access_m);
    r["length_m"] = json!(l);
    Ok(r)
}
