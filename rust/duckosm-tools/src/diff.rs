//! `duckosm.fixes.diff`: the edges that are different in two builds of an area, by `edge_id` (DuckDB joins the two files).

use crate::Result;
use std::path::Path;

/// The columns not compared as values: the id, the cells and tiles (from the geometry), and the geometry (compared as WKB).
const SKIP: [&str; 8] = ["edge_id", "geometry", "from_cell", "to_cell", "lca_res", "from_tile", "to_tile", "lca_zoom"];

/// One row for each edge that was added, removed or changed in `other`: `(mode, edge_id, change, changed columns)`. The
/// modes of both files are compared, or only `mode`.
pub fn diff(db: &Path, other: &Path, mode: Option<&str>) -> Result<Vec<(String, i64, String, Vec<String>)>> {
    let con = crate::open_read_only(db)?;
    let e = |e: duckdb::Error| e.to_string();
    con.execute_batch(&format!("LOAD spatial; ATTACH '{}' AS other (READ_ONLY)", other.display().to_string().replace('\'', "''"))).map_err(e)?;
    let modes = match mode {
        Some(m) => vec![m.to_string()],
        None => {
            let theirs = {
                let mut st = con.prepare("SELECT DISTINCT schema_name FROM duckdb_tables() WHERE database_name = 'other' AND table_name = 'edges'").map_err(e)?;
                st.query_map([], |r| r.get::<_, String>(0)).map_err(e)?.collect::<std::result::Result<Vec<_>, _>>().map_err(e)?
            };
            crate::mode_schemas(&con, &[])?.into_iter().filter(|m| theirs.contains(m)).collect()
        }
    };
    let mut out = Vec::new();
    for m in modes {
        let mut st = con
            .prepare(
                "SELECT column_name FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'edges' AND database_name = current_database() \
                 INTERSECT SELECT column_name FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'edges' AND database_name = 'other'",
            )
            .map_err(e)?;
        let mut cols: Vec<String> =
            st.query_map([&m, &m], |r| r.get(0)).map_err(e)?.collect::<std::result::Result<Vec<String>, _>>().map_err(e)?;
        cols.retain(|c| !SKIP.contains(&c.as_str()));
        cols.sort();
        let mut cases: Vec<String> = cols.iter().map(|c| format!("CASE WHEN a.\"{c}\" IS DISTINCT FROM b.\"{c}\" THEN '{c}' END")).collect();
        cases.push("CASE WHEN ST_AsWKB(a.geometry) IS DISTINCT FROM ST_AsWKB(b.geometry) THEN 'geometry' END".into());
        let changed = format!("list_filter([{}], x -> x IS NOT NULL)", cases.join(", "));
        let sql = format!(
            "SELECT coalesce(a.edge_id, b.edge_id), \
                    CASE WHEN a.edge_id IS NULL THEN 'added' WHEN b.edge_id IS NULL THEN 'removed' ELSE 'changed' END AS change, \
                    CASE WHEN a.edge_id IS NULL OR b.edge_id IS NULL THEN [] ELSE {changed} END \
             FROM \"{m}\".edges a FULL JOIN other.\"{m}\".edges b ON a.edge_id = b.edge_id \
             WHERE a.edge_id IS NULL OR b.edge_id IS NULL OR len({changed}) > 0 ORDER BY change, 1"
        );
        let mut st = con.prepare(&sql).map_err(e)?;
        let rows = st
            .query_map([], |r| {
                let list: duckdb::types::Value = r.get(2)?;
                let names = match list {
                    duckdb::types::Value::List(v) => v.into_iter().filter_map(|x| if let duckdb::types::Value::Text(t) = x { Some(t) } else { None }).collect(),
                    _ => Vec::new(),
                };
                Ok((m.clone(), r.get::<_, i64>(0)?, r.get::<_, String>(1)?, names))
            })
            .map_err(e)?;
        for r in rows {
            out.push(r.map_err(e)?);
        }
    }
    Ok(out)
}

/// `duckosm.analysis.largest_component`: copy the database `db` to `out` and keep only the nodes `keep` of `mode`, with
/// the edges between them and the turns of those edges. The other modes do not change.
pub fn keep_nodes(db: &Path, out: &Path, mode: &str, keep: &[i64]) -> Result<()> {
    if mode.is_empty() || !mode.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{mode:?} is not a mode name"));
    }
    std::fs::copy(db, out).map_err(|e| format!("{}: {e}", out.display()))?;
    let con = duckdb::Connection::open(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let e = |e: duckdb::Error| e.to_string();
    con.execute_batch("LOAD spatial; CREATE TEMP TABLE _keep(node_id BIGINT)").map_err(e)?;
    {
        let mut app = con.appender("_keep").map_err(e)?;
        for id in keep {
            app.append_row([*id]).map_err(e)?;
        }
    }
    let mut sql = format!(
        "DELETE FROM {mode}.edges WHERE source NOT IN (SELECT node_id FROM _keep) OR target NOT IN (SELECT node_id FROM _keep);
         DELETE FROM {mode}.nodes WHERE node_id NOT IN (SELECT node_id FROM _keep);"
    );
    let mut st = con.prepare("SELECT table_name FROM information_schema.tables WHERE table_schema = ?").map_err(e)?;
    let tables: Vec<String> = st.query_map([mode], |r| r.get(0)).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
    for (t, a, b) in [("edge_graph", "from_edge", "to_edge"), ("turn_restrictions", "from_edge_id", "to_edge_id"),
                      ("turn_permission", "from_edge", "to_edge"), ("turn_path_restrictions", "from_edge", "to_edge")] {
        if tables.iter().any(|x| x == t) {
            sql += &format!("DELETE FROM {mode}.{t} WHERE {a} NOT IN (SELECT edge_id FROM {mode}.edges) OR {b} NOT IN (SELECT edge_id FROM {mode}.edges);");
        }
    }
    sql += "CHECKPOINT;";
    con.execute_batch(&sql).map_err(e)
}
