//! duckOSM `lane_routing.py`: the lane graph of a GMNS database (lane to lane turns from `movement`, lane changes between
//! adjacent lanes) as `lane_<mode>.lane_edges`, and the shortest lane route (networkx `dijkstra_path`).

use crate::graph::Graph;
use crate::{err, Result};
use duckdb::Connection;
use serde_json::{json, Value as J};
use std::collections::HashMap;
use std::path::Path;

const LC_PEN: f64 = 25.0;
const TURN_PEN: [(&str, f64); 6] = [("left", 30.0), ("uturn", 60.0), ("right", 10.0), ("thru", 3.0), ("merge", 3.0), ("diverge", 3.0)];

fn metres(g: &str) -> String {
    format!("ST_Length_Spheroid(ST_FlipCoordinates({g}))")
}

fn lc_sql(g: &str) -> String {
    format!(
        "SELECT a.lane_id, b.lane_id, 'lane_change', {} + {LC_PEN:?} FROM {g}.lane a JOIN {g}.lane b ON a.link_id = b.link_id AND abs(a.lane_num - b.lane_num) = 1",
        metres("b.geom")
    )
}

fn turn_sql(g: &str) -> String {
    let case: Vec<String> = TURN_PEN.iter().map(|(k, v)| format!("WHEN '{k}' THEN {v:?}")).collect();
    format!(
        "SELECT il.lane_id, ol.lane_id, m.type, {} + CASE m.type {} ELSE 15.0 END FROM {g}.movement m \
         JOIN {g}.lane il ON il.link_id = m.ib_link_id   AND (m.start_ib_lane IS NULL OR il.lane_num BETWEEN m.start_ib_lane AND m.end_ib_lane) \
         JOIN {g}.lane ol ON ol.link_id = m.ob_link_id   AND (m.start_ob_lane IS NULL OR m.start_ib_lane IS NULL        OR ol.lane_num = m.start_ob_lane + il.lane_num - m.start_ib_lane) \
         WHERE m.ib_link_id IS NOT NULL AND m.ob_link_id IS NOT NULL AND COALESCE(m.allowed_uses, '') NOT IN ('bus', 'bus,bike')",
        metres("ol.geom"),
        case.join(" ")
    )
}

fn label(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "left" => "left turn",
        "right" => "right turn",
        "uturn" => "U-turn",
        "lane_change" => "lane change",
        "merge" => "merge",
        "diverge" => "fork",
        _ => return None,
    })
}

fn count(con: &Connection, sql: &str) -> Result<i64> {
    con.query_row(sql, [], |r| r.get(0)).map_err(err)
}

fn require_lane(con: &Connection, g: &str, db: &Path) -> Result<()> {
    if count(con, &format!("SELECT count(*) FROM duckdb_tables() WHERE schema_name='{g}' AND table_name='lane'"))? == 0 {
        return Err(format!("no '{g}.lane' in {} — build a GMNS db first (duckosm gmns)", db.display()));
    }
    Ok(())
}

/// duckOSM `build_lane_graph`: `lane_<mode>.lane_edges(from_lane, to_lane, kind, cost)`; the counts (lanes, lane changes,
/// turns, edges).
pub fn build_lane_graph(db: &Path, mode: &str) -> Result<(i64, i64, i64)> {
    let con = Connection::open(db).map_err(|e| format!("{}: {e}", db.display()))?;
    con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;").map_err(err)?;
    let (g, ls) = (format!("gmns_{mode}"), format!("lane_{mode}"));
    require_lane(&con, &g, db)?;
    con.execute_batch(&format!(
        "DROP SCHEMA IF EXISTS {ls} CASCADE; CREATE SCHEMA {ls};
         CREATE TABLE {ls}.lane_edges(from_lane VARCHAR, to_lane VARCHAR, kind VARCHAR, cost DOUBLE);
         INSERT INTO {ls}.lane_edges {}; INSERT INTO {ls}.lane_edges {}; CHECKPOINT;",
        lc_sql(&g),
        turn_sql(&g)
    ))
    .map_err(err)?;
    Ok((
        count(&con, &format!("SELECT count(*) FROM {g}.lane"))?,
        count(&con, &format!("SELECT count(*) FROM {ls}.lane_edges WHERE kind='lane_change'"))?,
        count(&con, &format!("SELECT count(*) FROM {ls}.lane_edges WHERE kind!='lane_change'"))?,
    ))
}

fn coords(wkt: &str) -> Vec<(f64, f64)> {
    let (Some(a), Some(b)) = (wkt.find('('), wkt.rfind(')')) else { return Vec::new() };
    wkt[a + 1..b]
        .split(',')
        .filter_map(|p| {
            let v: Vec<f64> = p.split_whitespace().take(2).filter_map(|x| x.parse().ok()).collect();
            (v.len() == 2).then(|| (v[0], v[1]))
        })
        .collect()
}

/// duckOSM `route_lanes`: `{lanes, cost, geometry, maneuvers}` (empty when the lanes are not connected). `from` and `to` are
/// lane ids, or edge ids for their lane 1.
pub fn route_lanes(db: &Path, from: &str, to: &str, mode: &str) -> Result<J> {
    let con = crate::route::open(db)?;
    let (g, ls) = (format!("gmns_{mode}"), format!("lane_{mode}"));
    require_lane(&con, &g, db)?;
    let sql = if count(&con, &format!("SELECT count(*) FROM duckdb_tables() WHERE schema_name='{ls}' AND table_name='lane_edges'"))? > 0 {
        format!("SELECT from_lane, to_lane, kind, cost::DOUBLE FROM {ls}.lane_edges")
    } else {
        format!("{} UNION ALL {}", lc_sql(&g), turn_sql(&g))
    };
    // the lane ids as numbers for the graph, in the order they come
    let mut id: HashMap<String, i64> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut intern = |s: &str, names: &mut Vec<String>| -> i64 {
        *id.entry(s.to_string()).or_insert_with(|| {
            names.push(s.to_string());
            names.len() as i64 - 1
        })
    };
    let mut graph = Graph::default();
    let mut kind: HashMap<(i64, i64), Option<String>> = HashMap::new();
    let mut st = con.prepare(&sql).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        let (f, t): (String, String) = (r.get(0).map_err(err)?, r.get(1).map_err(err)?);
        let (a, b) = (intern(&f, &mut names), intern(&t, &mut names));
        let c: Option<f64> = r.get(3).map_err(err)?;
        graph.add_arc(a, b, c.unwrap_or(f64::NAN));
        kind.insert((a, b), r.get(2).map_err(err)?);
    }
    let lanes: std::collections::HashSet<String> = {
        let mut st = con.prepare(&format!("SELECT lane_id FROM {g}.lane")).map_err(err)?;
        st.query_map([], |r| r.get(0)).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?
    };
    let resolve = |x: &str| -> String {
        if lanes.contains(x) {
            x.to_string()
        } else if lanes.contains(&format!("{x}_1")) {
            format!("{x}_1")
        } else {
            x.to_string()
        }
    };
    let (src, dst) = (resolve(from), resolve(to));
    let none = json!({"lanes": [], "cost": null, "geometry": null, "maneuvers": []});
    let (Some(&a), Some(&b)) = (id.get(&src), id.get(&dst)) else { return Ok(none) };
    if !graph.has_path(a, b) {
        return Ok(none);
    }
    let Some((cost, path)) = graph.dijkstra(a, b)? else { return Ok(none) };
    let maneuvers: Vec<&str> = path.windows(2).filter_map(|w| kind.get(&(w[0], w[1])).cloned().flatten().and_then(|k| label(&k))).collect();
    let path: Vec<String> = path.iter().map(|&i| names[i as usize].clone()).collect();
    let list = path.iter().map(|p| format!("'{}'", p.replace('\'', "''"))).collect::<Vec<_>>().join(",");
    let mut st = con.prepare(&format!("SELECT lane_id, ST_AsText(geom) FROM {g}.lane WHERE lane_id IN ({list})")).map_err(err)?;
    let geom: HashMap<String, String> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(err)?;
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for lid in &path {
        for p in coords(geom.get(lid).map(String::as_str).unwrap_or("LINESTRING()")) {
            if pts.last() != Some(&p) {
                pts.push(p);
            }
        }
    }
    let f = duckosm_build::place::py_float;
    let wkt = (pts.len() >= 2).then(|| format!("LINESTRING ({})", pts.iter().map(|(x, y)| format!("{} {}", f(*x), f(*y))).collect::<Vec<_>>().join(", ")));
    eprintln!("route_lanes[{mode}]: {src} -> {dst}: {} lanes, cost {cost:.0}, {} maneuvers", path.len(), maneuvers.len());
    Ok(json!({"lanes": path, "cost": cost, "geometry": wkt, "maneuvers": maneuvers}))
}
