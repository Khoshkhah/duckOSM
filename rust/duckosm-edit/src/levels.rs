//! `duckosm levels DB` (docs/reference/cli.md): the drawing order of the roads of a built database, made again in Rust. The roads of all
//! modes are one level area (roadstyle's `level_edges_sql`, `level_area::make_files`); roadstyle's level editor solves it with your edits
//! and draws the ends with your heads and caps. The database is the master of the edits: `visualization.level_edits`, `level_heads` and
//! `level_caps` are the input, and the result goes to `visualization.edge_levels` and `edge_levels_meta`. A row that names no road of the
//! database is not used and goes to `visualization.level_unmatched`, where the next run tries it again (duckOSM's Python `make_and_solve`).

use crate::native::NativeDb;
use crate::{page, rows, Db};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

/// The level inputs, by table, with the columns that name roads.
const INPUTS: [(&str, &[&str]); 3] = [("level_edits", &["a", "b"]), ("level_heads", &["road"]), ("level_caps", &["road"])];

/// What a run did.
#[derive(Debug)]
pub struct Summary {
    pub roads: usize,
    pub edges: usize,
    pub given_up: usize,
    pub unmatched: usize,
}

/// One input table: its columns and its rows (JSON objects).
struct Table {
    cols: Vec<String>,
    rows: Vec<Value>,
}

/// An id of a cell: a number, or text of a number (a table of the Python package has text).
fn id_of(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// A cell as CSV text, with quotes as pandas writes them.
fn cell(v: &Value) -> String {
    let s = match v {
        Value::Null => return String::new(),
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

fn csv(t: &Table) -> String {
    let mut out = t.cols.join(",") + "\n";
    for r in &t.rows {
        out += &t.cols.iter().map(|c| cell(&r[c])).collect::<Vec<_>>().join(",");
        out.push('\n');
    }
    out
}

/// The input tables of the database, with the rows of `level_unmatched` put back into their tables. A table that the database does not
/// have is not in the list; `level_edits` is there always (an empty edits.csv, as roadstyle's make_area writes it).
fn read_inputs(db: &mut dyn Db) -> Result<Vec<(&'static str, Table)>, String> {
    let have: HashSet<String> = db
        .texts("SELECT table_name FROM information_schema.tables WHERE table_schema = 'visualization'")?
        .into_iter()
        .flatten()
        .collect();
    let mut out = Vec::new();
    for (name, _) in INPUTS {
        if have.contains(name) {
            let cols = db
                .texts(&format!("SELECT column_name FROM information_schema.columns WHERE table_schema = 'visualization' AND table_name = '{name}' ORDER BY ordinal_position"))?
                .into_iter()
                .flatten()
                .collect();
            out.push((name, Table { cols, rows: rows(db, &format!("SELECT * FROM visualization.{name}"))? }));
        } else if name == "level_edits" {
            let cols = ["relation", "a", "b", "a_end", "b_end", "enabled"].map(String::from).to_vec();
            out.push((name, Table { cols, rows: Vec::new() }));
        }
    }
    if have.contains("level_unmatched") {
        for r in rows(db, "SELECT kind, row FROM visualization.level_unmatched")? {
            let kind = r["kind"].as_str().unwrap_or_default();
            let row: Value = serde_json::from_str(r["row"].as_str().unwrap_or("null")).map_err(|e| format!("level_unmatched: {e}"))?;
            let Some(name) = INPUTS.iter().map(|(n, _)| *n).find(|n| *n == kind) else { continue };
            if !out.iter().any(|(n, _)| *n == name) {
                let cols = row.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                out.push((name, Table { cols, rows: Vec::new() }));
            }
            out.iter_mut().find(|(n, _)| *n == name).map(|(_, t)| t.rows.push(row));
        }
    }
    Ok(out)
}

/// Solve the level area of `db` again with its edits and write the result into it. The file must not be open in another process.
pub fn levels(db_path: &Path) -> Result<Summary, String> {
    let name = db_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut db = NativeDb::new(db_path);
    db.open()?;
    let edges = page::level_edges(&mut db)?;
    let ids: HashSet<i64> = page::level_edge_ids(&mut db)?.into_iter().collect();
    let mut inputs = read_inputs(&mut db)?;
    db.close();

    let table = roadstyle::read::read_roads(edges, "geometry").map_err(|e| e.to_string())?;
    // the rows whose roads are all in this database are used; the others wait in level_unmatched
    let mut unmatched = Vec::new();
    for (name, t) in inputs.iter_mut() {
        let id_cols = INPUTS.iter().find(|(n, _)| n == name).map_or(&[][..], |(_, c)| *c);
        let (keep, out): (Vec<Value>, Vec<Value>) = std::mem::take(&mut t.rows)
            .into_iter()
            .partition(|r| id_cols.iter().all(|c| r[*c].is_null() || id_of(&r[*c]).is_some_and(|i| ids.contains(&i))));
        t.rows = keep;
        unmatched.extend(out.into_iter().map(|r| json!({"kind": *name, "row": r.to_string()})));
    }
    let text = |n: &str| inputs.iter().find(|(t, _)| *t == n).map(|(_, t)| csv(t)).unwrap_or_default();
    let (edits, heads, caps) = (text("level_edits"), text("level_heads"), text("level_caps"));

    let files = roadstyle::level_area::make_files(&table, &roadstyle::level_area::MakeOptions::default(), &heads).map_err(|e| e.to_string())?;
    let mut area = roadstyle::level_editor::Editor::new(files.roads, &files.pairs).map_err(|e| e.to_string())?;
    let answer = area.solve(&edits).map_err(|e| e.to_string())?;
    let numbers: Vec<i64> = answer.levels.iter().flatten().copied().collect();
    let levels_csv = area.levels_csv(&numbers, &heads, &caps).map_err(|e| e.to_string())?;

    // the result into the database: the files go to a temporary folder, which the SQL reads
    let dir = std::env::temp_dir().join(format!("duckosm-levels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let q = |s: &str| s.replace('\'', "''");
    let write = |f: &str, text: &str| -> Result<String, String> {
        let p = dir.join(f);
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok(q(&p.display().to_string()))
    };
    let lv = write("levels.csv", &levels_csv)?;
    let created = {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros());
        format!("strftime(make_timestamp({now}), '%Y-%m-%dT%H:%M:%S') || '+00:00'")
    };
    let n_edges: usize = levels_csv.lines().count().saturating_sub(1);
    let mut sql = format!(
        "BEGIN;
CREATE SCHEMA IF NOT EXISTS visualization;
CREATE OR REPLACE TABLE visualization.edge_levels AS SELECT edge AS edge_id, casing_start, casing_level, casing_end, fill_level, head_start_m, head_end_m,
  CAST(cap_start AS VARCHAR) AS cap_start, CAST(cap_end AS VARCHAR) AS cap_end
  FROM read_csv('{lv}', header = true, types = {{'edge': 'BIGINT', 'casing_start': 'BIGINT', 'casing_level': 'BIGINT', 'casing_end': 'BIGINT',
                                               'fill_level': 'BIGINT', 'head_start_m': 'DOUBLE', 'head_end_m': 'DOUBLE'}});
CREATE OR REPLACE TABLE visualization.edge_levels_meta AS SELECT 'area' AS source, '{}' AS area, {n_edges}::BIGINT AS n_edges, '{}' AS edge_hash,
  '{}' AS roadstyle_version, {created} AS created;
",
        q(&name),
        q(&area.edge_hash()),
        roadstyle::levels::ROADSTYLE_VERSION
    );
    for (table, t) in &inputs {
        let path = write(&format!("{table}.csv"), &csv(t))?;
        let ids = INPUTS.iter().find(|(n, _)| n == table).map_or(&[][..], |(_, c)| *c);
        let cast = ids.iter().map(|c| format!("CAST(NULLIF(\"{c}\", '') AS BIGINT) AS \"{c}\"")).collect::<Vec<_>>().join(", ");
        sql += &format!("CREATE OR REPLACE TABLE visualization.{table} AS SELECT * REPLACE ({cast}) FROM read_csv('{path}', header = true, all_varchar = true);\n");
    }
    if unmatched.is_empty() {
        sql += "DROP TABLE IF EXISTS visualization.level_unmatched;\n";
    } else {
        let t = Table { cols: vec!["kind".into(), "row".into()], rows: unmatched.clone() };
        let path = write("level_unmatched.csv", &csv(&t))?;
        sql += &format!("CREATE OR REPLACE TABLE visualization.level_unmatched AS SELECT * FROM read_csv('{path}', header = true, all_varchar = true);\n");
    }
    sql += "COMMIT;";
    let con = duckdb::Connection::open(db_path).map_err(|e| format!("{}: {e}", db_path.display()))?;
    let r = con.execute_batch(&sql).map_err(|e| format!("levels: {e}"));
    let _ = std::fs::remove_dir_all(&dir);
    r?;
    Ok(Summary { roads: answer.roads, edges: n_edges, given_up: answer.given_up.len(), unmatched: unmatched.len() })
}
