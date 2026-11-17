//! The last steps of duckOSM's build on the `.duckdb` file: the indexes (`importer._create_indexes`), the build checks
//! (`validate.py`) and the numbers of the build report (`report.py`). The SQL is the SQL of duckOSM.

use duckdb::Connection;
use std::path::Path;

fn err(e: duckdb::Error) -> String {
    e.to_string()
}

fn count(con: &Connection, sql: &str) -> Result<i64, String> {
    con.query_row(sql, [], |r| r.get(0)).map_err(err)
}

fn has_table(con: &Connection, schema: &str, table: &str) -> Result<bool, String> {
    Ok(count(
        con,
        &format!("SELECT count(*) FROM duckdb_tables() WHERE database_name = current_database() AND schema_name = '{schema}' AND table_name = '{table}'"),
    )? > 0)
}

/// duckOSM's indexes on each mode: the ids, the ends of the edges, the geometries (R-tree) and, with the graph, the two
/// ends of `edge_graph`. They come last: DuckDB cannot add a column to a table that has an index.
pub fn create_indexes(db: &Path, modes: &[&str]) -> Result<(), String> {
    let con = Connection::open(db).map_err(err)?;
    con.execute_batch("INSTALL spatial; LOAD spatial;").map_err(err)?;
    for m in modes {
        con.execute_batch(&format!(
            "CREATE INDEX IF NOT EXISTS idx_nodes_id ON {m}.nodes(node_id);
             CREATE INDEX IF NOT EXISTS idx_edges_id ON {m}.edges(edge_id);
             CREATE INDEX IF NOT EXISTS idx_edges_source ON {m}.edges(source);
             CREATE INDEX IF NOT EXISTS idx_edges_target ON {m}.edges(target);
             CREATE INDEX IF NOT EXISTS idx_edges_geom ON {m}.edges USING RTREE (geometry);
             CREATE INDEX IF NOT EXISTS idx_nodes_geom ON {m}.nodes USING RTREE (geom);"
        ))
        .map_err(|e| format!("{m}: indexes: {e}"))?;
        if has_table(&con, m, "edge_graph")? {
            con.execute_batch(&format!(
                "CREATE INDEX IF NOT EXISTS idx_eg_from ON {m}.edge_graph(from_edge);
                 CREATE INDEX IF NOT EXISTS idx_eg_to ON {m}.edge_graph(to_edge);"
            ))
            .map_err(|e| format!("{m}: indexes: {e}"))?;
        }
    }
    con.execute_batch("CHECKPOINT").map_err(err)
}

/// The result of a check: its name, `Some(true)` (OK), `Some(false)` (FAIL) or `None` (WARN, never fatal), and the detail.
pub type Check = (String, Option<bool>, String);

/// The checks of duckOSM's validation (`validate.py`) that are on. The single-component checks are not here: there is no
/// component filter (2026-11-14), so duckOSM skips them too.
#[derive(Clone, Copy, Debug)]
pub struct Checks {
    pub unique_node_id: bool,
    pub way_length_conserved: bool,
    pub layer_without_structure: bool,
}

/// Run the checks on one mode, as `Validator.run` does (without the error: the caller decides with `fail_on_error`).
pub fn validate(db: &Path, mode: &str, c: Checks) -> Result<Vec<Check>, String> {
    let con = Connection::open(db).map_err(err)?;
    let m = mode;
    let mut out: Vec<Check> = Vec::new();
    if c.unique_node_id {
        let dups = count(&con, &format!("SELECT count(*) FROM (SELECT node_id FROM {m}.nodes GROUP BY node_id HAVING count(*) > 1)"))?;
        out.push(("unique_node_id".into(), Some(dups == 0), format!("{dups} duplicate node_id(s) in nodes")));
    }
    if c.way_length_conserved && !has_table(&con, m, "ways")? {
        out.push(("way_length_conserved".into(), Some(true), "skipped (no per-mode ways table — clip build)".into()));
    } else if c.way_length_conserved {
        let lost = count(
            &con,
            &format!(
                "WITH covered AS (
                    SELECT DISTINCT osm_id, unnest(refs) AS node_id
                    FROM {m}.edges WHERE NOT is_reverse AND osm_id > 0
                ), marked AS (
                    SELECT wr.osm_id, wr.node_id, wr.seq, (c.node_id IS NULL) AS missing
                    FROM (SELECT w.osm_id, u.node_id, u.seq
                          FROM {m}.ways w,
                               unnest(w.refs) WITH ORDINALITY AS u(node_id, seq)) wr
                    LEFT JOIN covered c
                      ON c.osm_id = wr.osm_id AND c.node_id = wr.node_id
                ), runs AS (
                    SELECT osm_id, min(seq) AS s0, max(seq) AS s1
                    FROM (SELECT osm_id, seq,
                                 seq - row_number() OVER (PARTITION BY osm_id ORDER BY seq) AS grp
                          FROM marked WHERE missing)
                    GROUP BY osm_id, grp
                )
                SELECT count(*)
                FROM runs r
                JOIN marked a ON a.osm_id = r.osm_id AND a.seq = r.s0 - 1 AND NOT a.missing
                JOIN marked b ON b.osm_id = r.osm_id AND b.seq = r.s1 + 1 AND NOT b.missing
                WHERE a.node_id <> b.node_id
                  AND EXISTS (
                    SELECT 1 FROM {m}.edges e
                    WHERE e.osm_id = r.osm_id AND NOT e.is_reverse
                      AND least(e.source, e.target) = least(a.node_id, b.node_id)
                      AND greatest(e.source, e.target) = greatest(a.node_id, b.node_id))"
            ),
        )?;
        out.push((
            "way_length_conserved".into(),
            Some(lost == 0),
            format!("{lost} deleted parallel arc(s) (way stretch missing while a kept edge bridges its endpoints)"),
        ));
    }
    if c.layer_without_structure {
        let cols = count(
            &con,
            &format!("SELECT count(*) FROM information_schema.columns WHERE table_catalog = current_database() AND table_schema = '{m}' AND table_name = 'edges' AND column_name IN ('layer', 'bridge', 'tunnel')"),
        )?;
        if cols == 3 {
            let n = count(
                &con,
                &format!(
                    "SELECT count(*) FROM {m}.edges WHERE COALESCE(TRY_CAST(layer AS INTEGER), 0) <> 0
                       AND COALESCE(bridge, 'no') = 'no' AND COALESCE(tunnel, 'no') = 'no'"
                ),
            )?;
            out.push((
                "layer_without_structure".into(),
                if n > 0 { None } else { Some(true) },
                format!("{n} edge(s) with layer≠0 but no bridge/tunnel tag (OSM tagging; drawn grade-separated, no 3D deck)"),
            ));
        } else {
            out.push(("layer_without_structure".into(), Some(true), "skipped (no layer/bridge/tunnel columns)".into()));
        }
    }
    Ok(out)
}

/// The numbers of a mode in the build report: nodes, edges, rows of `edge_graph` and the edges of each `highway` (most first).
pub struct ModeCounts {
    pub mode: String,
    pub nodes: i64,
    pub edges: i64,
    pub edge_pairs: i64,
    pub highways: Vec<(String, i64)>,
}

/// The numbers of the build report, and the local time of the build (`%Y%m%d_%H%M%S`, the name of the report files).
pub fn report_counts(db: &Path, modes: &[&str]) -> Result<(Vec<ModeCounts>, String), String> {
    let con = Connection::open(db).map_err(err)?;
    let mut out = Vec::new();
    for m in modes {
        let pairs = if has_table(&con, m, "edge_graph")? { count(&con, &format!("SELECT count(*) FROM {m}.edge_graph"))? } else { 0 };
        let mut st = con
            .prepare(&format!("SELECT coalesce(highway, 'None'), count(*) n FROM {m}.edges GROUP BY highway ORDER BY 2 DESC, 1"))
            .map_err(err)?;
        let highways = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        out.push(ModeCounts {
            mode: m.to_string(),
            nodes: count(&con, &format!("SELECT count(*) FROM {m}.nodes"))?,
            edges: count(&con, &format!("SELECT count(*) FROM {m}.edges"))?,
            edge_pairs: pairs,
            highways,
        });
    }
    let ts: String = con
        .query_row("SELECT strftime(current_localtimestamp(), '%Y%m%d_%H%M%S')", [], |r| r.get(0))
        .map_err(err)?;
    Ok((out, ts))
}

/// duckOSM `multimodal.transfer_s` and `transfer_costs` (`"from->to"`: seconds) on `mm.transfers`: the build writes each
/// transfer with 60 s. Nothing to do when the table is not there or the costs are the default.
pub fn transfer_costs(db: &Path, transfer_s: f64, per_pair: &[(String, f64)]) -> Result<(), String> {
    if transfer_s == 60.0 && per_pair.is_empty() {
        return Ok(());
    }
    let con = Connection::open(db).map_err(err)?;
    if !has_table(&con, "mm", "transfers")? {
        return Ok(());
    }
    let mut cost = String::from("CASE from_mode || '->' || to_mode");
    for (pair, s) in per_pair {
        cost += &format!(" WHEN '{}' THEN {s}", pair.replace('\'', "''"));
    }
    cost += &format!(" ELSE {transfer_s} END");
    let sql = format!("UPDATE mm.transfers SET cost_s = {}; CHECKPOINT;", if per_pair.is_empty() { transfer_s.to_string() } else { cost });
    con.execute_batch(&sql).map_err(|e| format!("mm.transfers: {e}"))
}

/// No `mm.*`: duckOSM makes them only with `multimodal.enabled`.
pub fn drop_multimodal(db: &Path) -> Result<(), String> {
    let con = Connection::open(db).map_err(err)?;
    con.execute_batch("DROP SCHEMA IF EXISTS mm CASCADE; CHECKPOINT;").map_err(err)
}
