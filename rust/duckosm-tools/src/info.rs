//! `info` (duckOSM `query.db_info` and `cli.info`): what a built database holds.

use crate::Result;
use serde_json::{json, Map, Value as J};
use std::path::Path;

fn err(e: duckdb::Error) -> String {
    e.to_string()
}

/// The dictionary of `db_info`, with its keys in its order.
pub fn db_info(db: &Path) -> Result<J> {
    let con = crate::open_read_only(db)?;
    let mut st = con
        .prepare("SELECT table_schema, table_name FROM information_schema.tables WHERE table_catalog = current_database()")
        .map_err(err)?;
    let tables: Vec<(String, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
    let has = |s: &str, t: &str| tables.iter().any(|x| x.0 == s && x.1 == t);
    let count = |s: &str, t: &str| -> Result<J> {
        if !has(s, t) {
            return Ok(J::Null);
        }
        con.query_row(&format!("SELECT count(*) FROM {s}.{t}"), [], |r| r.get::<_, i64>(0)).map(|n| json!(n)).map_err(err)
    };
    let mut modes = Vec::new();
    for m in crate::mode_schemas(&con, &[])? {
        let (edges, km): (i64, f64) = con
            .query_row(&format!("SELECT count(*), round(coalesce(sum(length_m), 0) / 1000, 1)::DOUBLE FROM {m}.edges"), [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(err)?;
        modes.push(json!({
            "mode": m, "edges": edges, "nodes": count(&m, "nodes")?, "private_edges": count(&m, "private_edges")?, "km": km,
            "edge_graph": count(&m, "edge_graph")?, "turn_restrictions": count(&m, "turn_restrictions")?,
        }));
    }
    let mut meta: Map<String, J> = Map::new();
    if has("main", "visualization_metadata") {
        let mut st = con.prepare("SELECT column_name FROM (DESCRIBE main.visualization_metadata)").map_err(err)?;
        let cols: Vec<String> = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
        let want: Vec<&str> = ["timezone", "built_at", "duckosm_version"].into_iter().filter(|c| cols.iter().any(|x| x == c)).collect();
        if !want.is_empty() {
            let sql = format!("SELECT {} FROM main.visualization_metadata LIMIT 1", want.join(", "));
            let mut st = con.prepare(&sql).map_err(err)?;
            let mut rows = st.query([]).map_err(err)?;
            if let Some(r) = rows.next().map_err(err)? {
                for (i, c) in want.iter().enumerate() {
                    // Python: str(value), so a time is its text
                    let v = match crate::json::value(r.get::<_, duckdb::types::Value>(i).map_err(err)?) {
                        J::Null => J::Null,
                        J::String(s) => J::String(s),
                        o => J::String(o.to_string()),
                    };
                    meta.insert(c.to_string(), v);
                }
            }
        }
    }
    let mut features: Vec<&str> = tables.iter().filter(|x| x.0 == "features").map(|x| x.1.as_str()).collect();
    features.sort_unstable();
    let admin = match count("main", "admin_boundaries")? {
        J::Number(n) if n.as_i64() != Some(0) => J::Number(n),
        _ => json!(0),
    };
    let get = |k: &str| meta.get(k).cloned().unwrap_or(J::Null);
    Ok(json!({
        "modes": modes,
        "timezone": get("timezone"), "built_at": get("built_at"), "duckosm_version": get("duckosm_version"),
        "raw": has("raw", "ways"),
        "features": features,
        "multimodal": tables.iter().any(|x| x.0 == "mm"),
        "boundary": has("main", "boundary"),
        "admin_boundaries": admin,
        "elevation": has("main", "elevation_metadata"),
        "visualization": has("visualization", "edge_levels"),
        "bus": has("bus", "routes"),
    }))
}

/// Python's `f"{v:,}"` for a whole number.
pub fn thousands(n: i64) -> String {
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Python's `str()` of a float: the shortest text, with `.0` for a whole number.
pub fn py_float(f: f64) -> String {
    let s = format!("{f:?}");
    s
}

/// The text of `duckosm info`.
pub fn text(db: &Path, d: &J) -> String {
    let name = db.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let built = match d["built_at"].as_str() {
        Some(b) => format!("built {}, duckOSM {}", b.chars().take(16).collect::<String>(), d["duckosm_version"].as_str().unwrap_or("None")),
        None => "build date not recorded".into(),
    };
    let mut out = format!("{name}  ({built}; time zone {})\n", d["timezone"].as_str().unwrap_or("not recorded"));
    let modes = d["modes"].as_array().cloned().unwrap_or_default();
    if modes.is_empty() {
        out += "no mode schemas (no <mode>.edges table)\n";
    } else {
        let cols = ["mode", "edges", "nodes", "private_edges", "km", "edge_graph", "turn_restrictions"];
        let fmt = |v: &J| match v {
            J::Null => "-".to_string(),
            J::Number(n) if n.is_i64() => thousands(n.as_i64().unwrap()),
            J::Number(n) => py_float(n.as_f64().unwrap()),
            J::String(s) => s.clone(),
            o => o.to_string(),
        };
        let mut rows: Vec<Vec<String>> = vec![cols.iter().map(|c| c.to_string()).collect()];
        for m in &modes {
            rows.push(cols.iter().map(|c| fmt(&m[*c])).collect());
        }
        let w: Vec<usize> = (0..cols.len()).map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0)).collect();
        for r in &rows {
            let cells: Vec<String> =
                r.iter().enumerate().map(|(i, v)| if i == 0 { format!("{v:<w$}", w = w[i]) } else { format!("{v:>w$}", w = w[i]) }).collect();
            out += &cells.join("  ");
            out.push('\n');
        }
    }
    let nfeat = d["features"].as_array().map_or(0, Vec::len);
    let admin = d["admin_boundaries"].as_i64().unwrap_or(0);
    let also: Vec<String> = [
        ("raw (OSM data)".to_string(), d["raw"] == J::Bool(true)),
        (format!("features ({nfeat} layers)"), nfeat > 0),
        ("mm (across modes)".to_string(), d["multimodal"] == J::Bool(true)),
        ("boundary".to_string(), d["boundary"] == J::Bool(true)),
        (format!("admin_boundaries ({admin})"), admin != 0),
        ("elevation".to_string(), d["elevation"] == J::Bool(true)),
        ("visualization (drawing order)".to_string(), d["visualization"] == J::Bool(true)),
        ("bus (bus routes)".to_string(), d["bus"] == J::Bool(true)),
    ]
    .into_iter()
    .filter(|x| x.1)
    .map(|x| x.0)
    .collect();
    out += &format!("also: {}\n", if also.is_empty() { "nothing else".to_string() } else { also.join(", ") });
    out
}
