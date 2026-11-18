//! duckOSM `directions.directions(con, route, mode)` on a database: the roads of a route (any route result: `route`,
//! `route_points`, `route_multimodal`, `route_multimodal_points`) and its steps as duckOSM's dicts.

use crate::{err, Result};
use duckdb::Connection;
use duckosm_router::directions::{self, Road};
use serde_json::{json, Value as J};
use std::collections::HashMap;

/// duckOSM `_legs`: (mode, edge ids, metres used of each edge) from a route result.
fn legs(route: &J, mode: &str) -> Vec<(String, Vec<i64>, HashMap<i64, f64>)> {
    if let Some(l) = route["legs"].as_array().filter(|l| !l.is_empty()) {
        return l
            .iter()
            .map(|x| (x["mode"].as_str().unwrap_or_default().to_string(), x["edges"].as_array().into_iter().flatten().filter_map(J::as_i64).collect(), HashMap::new()))
            .collect();
    }
    let mut used = HashMap::new();
    for p in route["path"].as_array().into_iter().flatten() {
        if let (Some(e), Some(l)) = (p["edge_id"].as_i64(), p.get("length_m")) {
            used.insert(e, l.as_f64().unwrap_or(f64::NAN));
        }
    }
    vec![(mode.to_string(), route["edges"].as_array().into_iter().flatten().filter_map(J::as_i64).collect(), used)]
}

/// The steps of `route` (a route result as JSON; null or no edges: none).
pub fn directions(con: &Connection, route: &J, mode: &str) -> Result<J> {
    if route.is_null() || route.as_object().is_some_and(|o| o.is_empty()) {
        return Ok(json!([]));
    }
    let legs = legs(route, mode);
    type Info = (Option<String>, Option<String>, Option<f64>, i64, bool, Vec<(f64, f64)>);
    let mut info: HashMap<(String, i64), Info> = HashMap::new();
    let mut outs: HashMap<String, HashMap<i64, Vec<Vec<(f64, f64)>>>> = HashMap::new();
    let mut modes: Vec<&String> = legs.iter().map(|l| &l.0).collect();
    modes.sort();
    modes.dedup();
    for m in modes {
        let ids: Vec<String> = legs.iter().filter(|l| &l.0 == m).flat_map(|l| l.1.iter().map(i64::to_string)).collect();
        if ids.is_empty() {
            continue;
        }
        let mut st = con
            .prepare(&format!(
                "SELECT edge_id, name, highway, length_m::DOUBLE, source, junction, ST_AsWKB(geometry) FROM {m}.edges WHERE edge_id IN ({})",
                ids.join(", ")
            ))
            .map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let jn: Option<String> = r.get(5).map_err(err)?;
            let rb = matches!(jn.as_deref(), Some("roundabout" | "circular"));
            let line = crate::points::line_of_wkb(&r.get::<_, Vec<u8>>(6).map_err(err)?)?;
            info.insert((m.clone(), r.get(0).map_err(err)?), (r.get(1).map_err(err)?, r.get(2).map_err(err)?, r.get(3).map_err(err)?, r.get(4).map_err(err)?, rb, line));
        }
        let mut srcs: Vec<String> = info.iter().filter(|(k, _)| &k.0 == m).map(|(_, v)| v.3.to_string()).collect();
        srcs.sort();
        srcs.dedup();
        let o = outs.entry(m.clone()).or_default();
        if srcs.is_empty() {
            continue;
        }
        let mut st = con.prepare(&format!("SELECT source, ST_AsWKB(geometry) FROM {m}.edges WHERE source IN ({})", srcs.join(", "))).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            o.entry(r.get(0).map_err(err)?).or_default().push(crate::points::line_of_wkb(&r.get::<_, Vec<u8>>(1).map_err(err)?)?);
        }
    }
    let mut roads = Vec::new();
    for (m, ks, used) in &legs {
        for k in ks {
            let (name, hw, len, src, rb, line) = info.get(&(m.clone(), *k)).cloned().ok_or(format!("edge {k} is not in {m}.edges"))?;
            roads.push(Road {
                mode: m.clone(),
                name,
                highway: hw,
                length_m: used.get(k).copied().or(len).unwrap_or(f64::NAN),
                roundabout: rb,
                line,
                out: outs.get(m).and_then(|o| o.get(&src)).cloned().unwrap_or_default(),
            });
        }
    }
    let steps: Vec<J> = directions::directions(&roads)
        .into_iter()
        .map(|s| {
            let mut o = json!({"type": s.kind, "modifier": s.modifier, "name": s.name, "mode": s.mode, "at": [s.at.0, s.at.1], "length_m": s.length_m});
            if let Some(x) = s.exit {
                o["exit"] = json!(x);
            }
            o["text"] = json!(s.text);
            o
        })
        .collect();
    Ok(J::Array(steps))
}
