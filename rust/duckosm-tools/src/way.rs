//! `way` (duckOSM `query.way_table_sql`, `way_raw` and `cli.way`): everything a database knows about one OSM way, across
//! all modes, in one table.

use crate::Result;
use duckdb::Connection;
use serde_json::{json, Value as J};
use std::path::Path;

fn err(e: duckdb::Error) -> String {
    e.to_string()
}

fn strings(con: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut st = con.prepare(sql).map_err(err)?;
    let v = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
    Ok(v)
}

fn edge_columns(con: &Connection, mode: &str) -> Result<Vec<String>> {
    strings(
        con,
        &format!("SELECT column_name FROM information_schema.columns WHERE table_schema = '{mode}' AND table_name = 'edges' ORDER BY ordinal_position"),
    )
}

/// The UNION query of the edges of `osm_id` in each mode (duckOSM `way_table_sql`): the columns of all modes in the order
/// they are first seen, NULL where a mode has none; the geometry as WKT only with `geometry`.
pub fn way_table_sql(con: &Connection, osm_id: i64, modes: &[String], geometry: bool) -> Result<String> {
    let schemas = crate::mode_schemas(con, modes)?;
    if schemas.is_empty() {
        return Err("no mode schemas with an edges table in this database".into());
    }
    let mut cols: Vec<String> = Vec::new();
    let mut per_mode: Vec<Vec<String>> = Vec::new();
    for m in &schemas {
        let c = edge_columns(con, m)?;
        for x in &c {
            if !cols.contains(x) && !(x == "geometry" && !geometry) {
                cols.push(x.clone());
            }
        }
        per_mode.push(c);
    }
    let arms: Vec<String> = schemas
        .iter()
        .zip(&per_mode)
        .map(|(m, have)| {
            let sel: Vec<String> = cols
                .iter()
                .map(|c| {
                    if !have.contains(c) {
                        format!("NULL AS \"{c}\"")
                    } else if c == "geometry" {
                        format!("ST_AsText(\"{c}\") AS \"{c}\"")
                    } else {
                        format!("\"{c}\"")
                    }
                })
                .collect();
            format!("SELECT '{m}' AS mode, {}\nFROM {m}.edges WHERE osm_id = {osm_id}", sel.join(", "))
        })
        .collect();
    let order = if per_mode.iter().any(|c| c.iter().any(|x| x == "edge_ref")) {
        "COALESCE(TRY_CAST(regexp_extract(edge_ref, '#(\\d+)', 1) AS INT), 2000000000), is_reverse, mode"
    } else {
        "source, target, is_reverse, mode"
    };
    Ok(format!("SELECT * FROM (\n{}\n) ORDER BY {order}", arms.join("\nUNION ALL\n")))
}

/// The raw way (`raw.ways`): its id, refs and tags; None for a synthetic id or a clip with no `raw.*`.
fn way_raw(con: &Connection, osm_id: i64) -> Result<Option<(i64, Vec<i64>, Vec<(String, String)>)>> {
    let has: i64 = con
        .query_row(
            "SELECT count(*) FROM information_schema.tables WHERE table_catalog = current_database() AND table_schema = 'raw' AND table_name = 'ways'",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if has == 0 {
        return Ok(None);
    }
    let mut st = con.prepare("SELECT osm_id, refs, map_keys(tags), map_values(tags) FROM raw.ways WHERE osm_id = ?").map_err(err)?;
    let mut rows = st.query([osm_id]).map_err(err)?;
    let Some(r) = rows.next().map_err(err)? else { return Ok(None) };
    let list = |v: duckdb::types::Value| -> Vec<duckdb::types::Value> {
        match v {
            duckdb::types::Value::List(l) => l,
            _ => Vec::new(),
        }
    };
    let text = |v: &duckdb::types::Value| match v {
        duckdb::types::Value::Text(s) => s.clone(),
        o => format!("{o:?}"),
    };
    let refs = list(r.get(1).map_err(err)?).into_iter().filter_map(|v| if let duckdb::types::Value::BigInt(i) = v { Some(i) } else { None }).collect();
    let (k, v) = (list(r.get(2).map_err(err)?), list(r.get(3).map_err(err)?));
    let tags = k.iter().zip(&v).map(|(a, b)| (text(a), text(b))).collect();
    Ok(Some((r.get(0).map_err(err)?, refs, tags)))
}

/// Python's `repr` of a text.
fn py_str(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::from(q);
    for c in s.chars() {
        match c {
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out += &format!("\\x{:02x}", c as u32),
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

/// The name of a type in DuckDB's table print (`duckbox`).
fn box_type(t: &str) -> String {
    if let Some(inner) = t.strip_suffix("[]") {
        return format!("{}[]", box_type(inner));
    }
    match t {
        "BIGINT" => "int64",
        "INTEGER" => "int32",
        "SMALLINT" => "int16",
        "TINYINT" => "int8",
        "HUGEINT" => "int128",
        "UBIGINT" => "uint64",
        "UINTEGER" => "uint32",
        "USMALLINT" => "uint16",
        "UTINYINT" => "uint8",
        _ => return t.to_lowercase(),
    }
    .to_string()
}

fn numeric(t: &str) -> bool {
    matches!(t, "BIGINT" | "INTEGER" | "SMALLINT" | "TINYINT" | "HUGEINT" | "UBIGINT" | "UINTEGER" | "USMALLINT" | "UTINYINT" | "FLOAT" | "DOUBLE")
        || t.starts_with("DECIMAL")
}

/// The rows of `sql` as DuckDB's table print shows them (`DuckDBPyRelation.show` with no limit of width): a header, the
/// types, the values as DuckDB's text of them.
fn duckbox(con: &Connection, sql: &str) -> Result<String> {
    con.execute_batch(&format!("CREATE OR REPLACE TEMP VIEW way_rows AS {sql}")).map_err(err)?;
    let mut st = con.prepare("SELECT column_name, column_type FROM (DESCRIBE way_rows)").map_err(err)?;
    let cols: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
    let sel = cols.iter().map(|(c, _)| format!("CAST(\"{c}\" AS VARCHAR)")).collect::<Vec<_>>().join(", ");
    let mut st = con.prepare(&format!("SELECT {sel} FROM way_rows")).map_err(err)?;
    let n = cols.len();
    let rows: Vec<Vec<String>> = st
        .query_map([], |r| (0..n).map(|i| r.get::<_, Option<String>>(i).map(|v| v.unwrap_or_else(|| "NULL".into()))).collect())
        .map_err(err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(err)?;
    let types: Vec<String> = cols.iter().map(|(_, t)| box_type(t)).collect();
    let len = |s: &str| s.chars().count();
    let w: Vec<usize> = (0..n)
        .map(|i| rows.iter().map(|r| len(&r[i])).chain([len(&cols[i].0), len(&types[i])]).max().unwrap_or(0) + 2)
        .collect();
    let line = |l: &str, m: &str, r: &str| format!("{l}{}{r}\n", w.iter().map(|&x| "─".repeat(x)).collect::<Vec<_>>().join(m));
    let center = |s: &str, x: usize| {
        let pad = x - len(s);
        format!("{}{s}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2))
    };
    let mut out = line("┌", "┬", "┐");
    out += &format!("│{}│\n", (0..n).map(|i| center(&cols[i].0, w[i])).collect::<Vec<_>>().join("│"));
    out += &format!("│{}│\n", (0..n).map(|i| center(&types[i], w[i])).collect::<Vec<_>>().join("│"));
    out += &line("├", "┼", "┤");
    for r in &rows {
        let cells: Vec<String> = (0..n)
            .map(|i| {
                let pad = w[i] - 2 - len(&r[i]);
                if numeric(&cols[i].1) {
                    format!(" {}{} ", " ".repeat(pad), r[i])
                } else {
                    format!(" {}{} ", r[i], " ".repeat(pad))
                }
            })
            .collect();
        out += &format!("│{}│\n", cells.join("│"));
    }
    out += &line("└", "┴", "┘");
    Ok(out)
}

/// `duckosm way`: the text (or with `as_json` one JSON object), or with `out` a file (.csv, .parquet or .json). The lines of
/// the raw way are written first, also before an error.
pub fn way(db: &Path, osm_id: i64, modes: &[String], geometry: bool, out: Option<&Path>, as_json: bool, w: &mut dyn std::io::Write) -> Result<()> {
    let io = |e: std::io::Error| e.to_string();
    let con = crate::open_read_only(db)?;
    con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;").map_err(err)?;
    let raw = way_raw(&con, osm_id)?;
    if as_json {
        let sql = way_table_sql(&con, osm_id, modes, geometry)?;
        con.execute_batch(&format!("CREATE OR REPLACE TEMP VIEW way_rows AS {sql}")).map_err(err)?;
        let cols = strings(&con, "SELECT column_name FROM (DESCRIBE way_rows)")?;
        let mut st = con.prepare("SELECT * FROM way_rows").map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        let mut edges = Vec::new();
        while let Some(r) = rows.next().map_err(err)? {
            let mut o = serde_json::Map::new();
            for (i, c) in cols.iter().enumerate() {
                o.insert(c.clone(), crate::json::value(r.get(i).map_err(err)?));
            }
            edges.push(J::Object(o));
        }
        let raw = match raw {
            Some((id, refs, tags)) => json!({"osm_id": id, "refs": refs, "tags": J::Object(tags.into_iter().map(|(k, v)| (k, J::String(v))).collect())}),
            None => J::Null,
        };
        return writeln!(w, "{}", crate::json::dumps(&json!({"osm_id": osm_id, "raw": raw, "edges": edges}))).map_err(io);
    }
    let mut text = String::new();
    match &raw {
        Some((id, refs, tags)) => {
            text += &format!("raw way {id}: {} node refs\n", refs.len());
            let t: Vec<String> = tags.iter().map(|(k, v)| format!("{}: {}", py_str(k), py_str(v))).collect();
            text += &format!("  tags: {{{}}}\n", t.join(", "));
            text += &format!("  refs: [{}]\n", refs.iter().map(i64::to_string).collect::<Vec<_>>().join(", "));
        }
        None if osm_id > 0 => text += &format!("raw way {osm_id}: not in raw.ways (clip build, or id not in this extract)\n"),
        None => text += &format!("osm_id {osm_id} is synthetic (PathConnector connector; -min of the two joined ways)\n"),
    }
    // the lines about the raw way come first, also before an error (as duckOSM prints them)
    w.write_all(text.as_bytes()).map_err(io)?;
    text.clear();
    let sql = way_table_sql(&con, osm_id, modes, geometry)?;
    let n: i64 = con.query_row(&format!("SELECT count(*) FROM ({sql})"), [], |r| r.get(0)).map_err(err)?;
    if n == 0 {
        return Err(format!("osm_id {osm_id} has no edges in any requested mode"));
    }
    match out {
        Some(path) => {
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let opts = match ext.as_str() {
                "csv" => "(FORMAT CSV, HEADER)",
                "parquet" => "(FORMAT PARQUET)",
                "json" => "(FORMAT JSON, ARRAY true)",
                _ => return Err(format!("unsupported extension .{ext} — use .csv, .parquet or .json")),
            };
            con.execute_batch(&format!("COPY ({sql}) TO '{}' {opts}", path.display().to_string().replace('\'', "''"))).map_err(err)?;
            text += &format!("wrote {n} rows -> {}\n", path.display());
        }
        // DuckDB's show() ends with an empty line
        None => text += &(duckbox(&con, &sql)? + "\n"),
    }
    w.write_all(text.as_bytes()).map_err(io)
}
