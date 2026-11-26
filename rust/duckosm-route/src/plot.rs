//! The data of `Network.plot` (the static matplotlib picture): the base-map layers cut to the view, the modes of each road, and
//! the line of a route. Python only draws them.

use crate::{err, Result};
use duckdb::Connection;

/// The mode schemas of the file that have an `edges` table, in the order driving, walking, cycling, then the others.
fn modes(con: &Connection) -> Result<Vec<String>> {
    let mut st = con
        .prepare("SELECT DISTINCT table_schema FROM information_schema.tables WHERE table_catalog = current_database() AND table_name = 'edges'")
        .map_err(err)?;
    let mut m: Vec<String> = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
    let rank = |x: &String| ["driving", "walking", "cycling"].iter().position(|d| d == x).unwrap_or(3);
    m.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
    Ok(m)
}

/// The polygons (WKB) of each table `features.<table>` of `tables`, cut to the box `view` (`x0, y0, x1, y1`); empty for a
/// table that the file does not have.
pub fn layers(con: &Connection, tables: &[String], view: [f64; 4]) -> Result<Vec<Vec<Vec<u8>>>> {
    let [x0, y0, x1, y1] = view;
    let b = format!("ST_MakeEnvelope({x0}, {y0}, {x1}, {y1})");
    let mut out = Vec::new();
    for t in tables {
        let has: i64 = con
            .query_row("SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'features' AND table_name = ?", [t], |r| r.get(0))
            .map_err(err)?;
        if has == 0 {
            out.push(Vec::new());
            continue;
        }
        let mut st = con
            .prepare(&format!("SELECT ST_AsWKB(ST_Intersection(geom, {b}))::BLOB FROM features.\"{t}\" WHERE ST_Intersects(geom, {b})"))
            .map_err(err)?;
        out.push(st.query_map([], |r| r.get(0)).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?);
    }
    Ok(out)
}

/// The dashboard's name of the modes that can use each road (`edge_id`): "all modes", "driving + walking", … or "driving".
pub fn modes_labels(con: &Connection) -> Result<Vec<(i64, String)>> {
    let ms = modes(con)?;
    if ms.is_empty() {
        return Ok(Vec::new());
    }
    let union: Vec<String> = ms.iter().map(|m| format!("SELECT edge_id, '{m}' AS mode FROM \"{m}\".edges")).collect();
    let mut st = con
        .prepare(&format!("SELECT edge_id, list(DISTINCT mode) FROM ({}) GROUP BY edge_id ORDER BY edge_id", union.join(" UNION ALL ")))
        .map_err(err)?;
    let rows = st
        .query_map([], |r| {
            let v: duckdb::types::Value = r.get(1)?;
            let used: Vec<String> = match v {
                duckdb::types::Value::List(l) => l.into_iter().filter_map(|x| if let duckdb::types::Value::Text(t) = x { Some(t) } else { None }).collect(),
                _ => Vec::new(),
            };
            Ok((r.get::<_, i64>(0)?, used))
        })
        .map_err(err)?;
    let main = ["driving", "walking", "cycling"];
    let mut out = Vec::new();
    for r in rows {
        let (e, used) = r.map_err(err)?;
        let label = if main.iter().all(|m| used.iter().any(|u| u == m)) {
            "all modes".to_string()
        } else {
            let known: Vec<&str> = main.iter().copied().filter(|m| used.iter().any(|u| u == m)).collect();
            if known.is_empty() {
                let mut other = used.clone();
                other.sort();
                other.join(" + ")
            } else {
                known.join(" + ")
            }
        };
        out.push((e, label));
    }
    Ok(out)
}

/// The union of the lines of the edges `ids`, found in any mode (a route across modes), as WKB; None when no edge is found.
pub fn route_union(con: &Connection, ids: &[i64]) -> Result<Option<Vec<u8>>> {
    let ms = modes(con)?;
    if ms.is_empty() || ids.is_empty() {
        return Ok(None);
    }
    let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let union: Vec<String> = ms.iter().map(|m| format!("SELECT edge_id, geometry FROM \"{m}\".edges WHERE edge_id IN ({list})")).collect();
    con.query_row(
        &format!("SELECT ST_AsWKB(ST_Union_Agg(geometry))::BLOB FROM (SELECT DISTINCT ON (edge_id) edge_id, geometry FROM ({}))", union.join(" UNION ALL ")),
        [],
        |r| r.get::<_, Option<Vec<u8>>>(0),
    )
    .map_err(err)
}
