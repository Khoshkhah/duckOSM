//! `duckosm route-map`: the route planner page (duckOSM's route_map.py). The map is roadstyle's page of the database (`maps.rs`);
//! the panel (page/planner.html) routes in the browser with duckosm-planner as WebAssembly, on the page's data (`RM`, made here
//! from the tables), so the page and `duckosm-rust route-points`, `route-multimodal-points` and `directions` give the same routes.

use crate::maps::{env_keys, map_page, MODES};
use crate::native::NativeDb;
use duckdb::Connection;
use serde_json::{json, Map, Value as J};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const PLANNER: &str = include_str!("../page/planner.html");
/// duckosm-planner built for wasm32 (build.rs); empty when the build had no wasm32 target: the page then says so.
const WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/duckosm_planner.wasm"));
pub const WARN_EDGES: i64 = 100_000;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn has(con: &Connection, schema: &str, table: &str) -> Result<bool, String> {
    let n: i64 = con.query_row("SELECT count(*) FROM information_schema.tables WHERE table_schema = ? AND table_name = ?", [schema, table], |r| r.get(0)).map_err(err)?;
    Ok(n > 0)
}

fn has_col(con: &Connection, schema: &str, table: &str, col: &str) -> Result<bool, String> {
    let n: i64 = con.query_row("SELECT count(*) FROM information_schema.columns WHERE table_schema = ? AND table_name = ? AND column_name = ?",
                               [schema, table, col], |r| r.get(0)).map_err(err)?;
    Ok(n > 0)
}

/// The road of the page's data: its edge_id, ends, length, junction, name, highway and line (from the first table that has it).
struct Road {
    eid: i64,
    s: i64,
    t: i64,
    len: Option<f64>,
    rb: bool,
    name: Option<String>,
    hw: Option<String>,
    line: Vec<(f64, f64)>,
}

/// The page's data (`RM`; duckosm-planner's `Area::from_data` reads it): every road of the page by number (`eid`, `src`, `tgt`
/// as numbers of `nid`, `name`, `hw`, `len`, `rb`, `geom`), each mode's roads in the order of `<mode>.edges` with their cost,
/// length and legal turns, walk + drive (`mm`) when the database has it, and the first route's ends.
pub fn planner_data(con: &Connection) -> Result<J, String> {
    let mut modes = Vec::new();
    for m in MODES {
        if has(con, m, "edges")? && has(con, m, "edge_graph")? {
            modes.push(m.to_string());
        }
    }
    if modes.is_empty() {
        return Err("no mode with edges + edge_graph to route on".into());
    }
    let mut roads: Vec<Road> = Vec::new();
    let mut k_of: HashMap<i64, usize> = HashMap::new();
    for tb in ["edges", "private_edges"] {
        for m in MODES {
            if !has(con, m, tb)? {
                continue;
            }
            let jn = if has_col(con, m, tb, "junction")? { "junction" } else { "NULL" };
            let mut st = con.prepare(&format!("SELECT edge_id, source, target, length_m::DOUBLE, {jn}::VARCHAR, name, highway, ST_AsWKB(geometry) FROM {m}.{tb}")).map_err(err)?;
            let mut rows = st.query([]).map_err(err)?;
            while let Some(r) = rows.next().map_err(err)? {
                let eid: i64 = r.get(0).map_err(err)?;
                if k_of.contains_key(&eid) {
                    continue;
                }
                let j: Option<String> = r.get(4).map_err(err)?;
                k_of.insert(eid, roads.len());
                roads.push(Road { eid, s: r.get(1).map_err(err)?, t: r.get(2).map_err(err)?, len: r.get(3).map_err(err)?,
                                  rb: matches!(j.as_deref(), Some("roundabout" | "circular")), name: r.get(5).map_err(err)?, hw: r.get(6).map_err(err)?,
                                  line: duckosm_route::points::line_of_wkb(&r.get::<_, Vec<u8>>(7).map_err(err)?)? });
            }
        }
    }
    // node ids become small numbers (some pass 2**53, which JavaScript does not keep)
    let mut nodes: Vec<i64> = roads.iter().flat_map(|r| [r.s, r.t]).collect();
    nodes.sort_unstable();
    nodes.dedup();
    let n_of: HashMap<i64, usize> = nodes.iter().enumerate().map(|(i, &n)| (n, i)).collect();
    let mut graphs = Map::new();
    for m in &modes {
        let mut k = Vec::new();
        let (mut cost, mut len) = (Vec::new(), Vec::new());
        let mut st = con.prepare(&format!("SELECT edge_id, cost_s::DOUBLE, length_m::DOUBLE FROM {m}.edges")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            k.push(k_of[&r.get::<_, i64>(0).map_err(err)?]);
            cost.push(r.get::<_, Option<f64>>(1).map_err(err)?);
            len.push(r.get::<_, Option<f64>>(2).map_err(err)?);
        }
        let pos: HashMap<usize, usize> = k.iter().enumerate().map(|(i, &r)| (r, i)).collect();
        let mut next: Vec<Vec<usize>> = vec![Vec::new(); k.len()];
        let g = duckosm_route::route::routed_graph(con, m, &[])?;
        let mut st = con.prepare(&format!("SELECT from_edge, to_edge FROM {g}")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let (f, t): (i64, i64) = (r.get(0).map_err(err)?, r.get(1).map_err(err)?);
            if let (Some(&fk), Some(&tk)) = (k_of.get(&f), k_of.get(&t)) {
                if let Some(&i) = pos.get(&fk) {
                    next[i].push(tk);
                }
            }
        }
        graphs.insert(m.clone(), json!({"k": k, "cost": cost, "len": len, "next": next}));
    }
    // walk + drive over duckOSM's intermodal graph (`duckosm multimodal`); walk + cycle needs bike stations to mean anything
    let mut mm = J::Null;
    if modes.iter().any(|m| m == "walking") && modes.iter().any(|m| m == "driving") && has(con, "mm", "edges")? && has(con, "mm", "transfers")? {
        let mi = |m: &str| modes.iter().position(|x| x == m).filter(|_| m == "walking" || m == "driving");
        let mut edges = Vec::new();
        let mut st = con.prepare("SELECT mode, source, target, edge_id, cost_s::DOUBLE FROM mm.edges").map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let (md, s, t, e, c): (String, i64, i64, i64, Option<f64>) = (r.get(0).map_err(err)?, r.get(1).map_err(err)?, r.get(2).map_err(err)?, r.get(3).map_err(err)?, r.get(4).map_err(err)?);
            if let (Some(i), Some(k), Some(s), Some(t)) = (mi(&md), k_of.get(&e), n_of.get(&s), n_of.get(&t)) {
                edges.push(json!([i, s, t, k, c]));
            }
        }
        let mut transfers = Vec::new();
        let mut st = con.prepare("SELECT node_id, from_mode, to_mode, cost_s::DOUBLE, kind FROM mm.transfers").map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let (n, fm, tm, c, kind): (i64, String, String, Option<f64>, Option<String>) =
                (r.get(0).map_err(err)?, r.get(1).map_err(err)?, r.get(2).map_err(err)?, r.get(3).map_err(err)?, r.get(4).map_err(err)?);
            if let (Some(n), Some(f), Some(t)) = (n_of.get(&n), mi(&fm), mi(&tm)) {
                transfers.push(json!([n, f, t, c, kind]));
            }
        }
        mm = json!({"edges": edges, "transfers": transfers});
    }
    // the page opens with a route: markers on the roads nearest 30 % and 70 % along the diagonal
    let m0 = &modes[0];
    let (x0, y0, x1, y1): (f64, f64, f64, f64) = con.query_row(&format!("SELECT min(ST_XMin(geometry)), min(ST_YMin(geometry)), max(ST_XMax(geometry)), max(ST_YMax(geometry)) FROM {m0}.edges"),
                                                               [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(err)?;
    let near = |f: f64| -> Result<[f64; 2], String> {
        let sql = format!("SELECT ST_X(p), ST_Y(p) FROM (SELECT ST_LineInterpolatePoint(geometry, 0.5) AS p FROM {m0}.edges \
                           ORDER BY ST_Distance(ST_Centroid(geometry), ST_Point(?, ?)) LIMIT 1)");
        con.query_row(&sql, [x0 + (x1 - x0) * f, y0 + (y1 - y0) * f], |r| Ok([r.get(0)?, r.get(1)?])).map_err(err)
    };
    Ok(json!({
        "modes": modes, "n": roads.len(),
        "eid": roads.iter().map(|r| r.eid.to_string()).collect::<Vec<_>>(),
        "nid": nodes.iter().map(i64::to_string).collect::<Vec<_>>(),
        "src": roads.iter().map(|r| n_of[&r.s]).collect::<Vec<_>>(),
        "tgt": roads.iter().map(|r| n_of[&r.t]).collect::<Vec<_>>(),
        "name": roads.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
        "hw": roads.iter().map(|r| r.hw.clone().unwrap_or_default()).collect::<Vec<_>>(),
        "len": roads.iter().map(|r| r.len.unwrap_or(0.0)).collect::<Vec<_>>(),
        "rb": roads.iter().enumerate().filter(|(_, r)| r.rb).map(|(k, _)| k).collect::<Vec<_>>(),
        "geom": roads.iter().map(|r| r.line.iter().flat_map(|p| [p.0, p.1]).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "graphs": graphs, "mm": mm, "start": near(0.3)?, "end": near(0.7)?,
    }))
}

/// RFC 4648 base64 (the page's WebAssembly as a data string).
fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            s.push(if i <= c.len() { A[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    s
}

/// Write the route planner of the built database `db` to `out`; return its path. `mode`: the first choice and the network in
/// front (driving, walking, cycling, or walk+drive: every mode in front; needs `mm.*`). Default: walking in front.
pub fn write_route_map(db: &Path, out: &Path, mode: Option<&str>, basemap: Option<&str>, google_3d: Option<bool>) -> Result<PathBuf, String> {
    if let Some(m) = mode.filter(|m| !MODES.contains(m) && *m != "walk+drive") {
        return Err(format!("unknown mode {m:?} (one of {}, walk+drive)", MODES.join(", ")));
    }
    let con = duckosm_route::route::open(db)?;
    let mut data = planner_data(&con)?;
    let present: Vec<&str> = data["modes"].as_array().into_iter().flatten().filter_map(J::as_str).collect();
    if mode == Some("walk+drive") && data["mm"].is_null() {
        return Err(format!("walk+drive needs walking, driving and the mm tables in {}: run `duckosm multimodal` on it", db.display()));
    }
    if let Some(m) = mode.filter(|m| MODES.contains(m) && !present.contains(m)) {
        return Err(format!("mode {m:?} has no edge_graph in {} (present: {present:?})", db.display()));
    }
    let n: i64 = present.iter().map(|m| con.query_row(&format!("SELECT count(*) FROM {m}.edges"), [], |r| r.get::<_, i64>(0)).unwrap_or(0)).sum();
    drop(con);
    if n > WARN_EDGES {
        eprintln!("route-map: {n} edges make a heavy page; clip an area first (duckosm extract) for a lighter one");
    }
    data["first"] = match mode {
        Some("walk+drive") => json!(["walking", "driving"]),
        Some(m) => json!([m]),
        None => J::Null,
    };
    let stem = db.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    // the planner draws its route over the roads in the page: no tiles; its panel is on the right, so roadstyle's toolbox is on the left
    let mut args = json!({"name": format!("{stem}: route planner"), "tiles": false, "dashboard": {"panel": null, "toolbar": {"position": "left"}}});
    if google_3d.unwrap_or_else(|| !env_keys().is_empty()) {
        args["google_3d"] = json!(true);
    }
    if let Some(b) = basemap {
        args["basemap"] = json!(b);
    }
    let front = if mode == Some("walk+drive") { "all" } else { mode.unwrap_or("walking") };
    let mut ndb = NativeDb::new(db);
    crate::Db::open(&mut ndb)?;
    let html = map_page(&mut ndb, &stem, front, false, args)?;
    let panel = PLANNER.replacen("__RM__", &data.to_string().replace("</", "<\\/"), 1).replacen("__WASM__", &base64(WASM), 1);
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(out, html.replacen("</body>", &(panel + "</body>"), 1)).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("route-map: {n} edges -> {}", out.display());
    Ok(out.to_path_buf())
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_of_rfc4648() {
        for (a, b) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(super::base64(a.as_bytes()), b);
        }
    }
}
