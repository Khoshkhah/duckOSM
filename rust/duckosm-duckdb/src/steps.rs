//! The steps that `duckosm bus` and `duckosm multimodal` do on a `.duckdb` file that is already built: the bus routes
//! (`bus.routes`, `bus.route_edges`) and the multimodal graph (`mm.edges`, `mm.transfers`). The build makes the same
//! tables (duckosm-build: `write_bus`, `bus_routes` and `area.rs`); these functions make them for a file built before.

use crate::input::RawInput;
use duckdb::arrow::array::{Array, AsArray};
use duckdb::arrow::datatypes::{DataType, Field, Int64Type};
use duckdb::Connection;
use duckosm_core::graph::MemberType;
use duckosm_core::model::{Attrs, Edge};
use std::path::Path;
use std::sync::Arc;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// A connection that can write `db`: the tables of a build have R-tree indexes, which need the spatial extension.
fn open_rw(db: &Path) -> Result<Connection, String> {
    let con = Connection::open(db).map_err(err)?;
    con.execute_batch("INSTALL spatial; LOAD spatial;").map_err(|e| format!("the spatial extension of DuckDB: {e}"))?;
    Ok(con)
}

fn has_table(con: &Connection, schema: &str, table: &str) -> Result<bool, String> {
    let n: i64 = con
        .query_row(
            "SELECT count(*) FROM duckdb_tables() WHERE database_name = current_database() AND schema_name = ? AND table_name = ?",
            [schema, table],
            |r| r.get(0),
        )
        .map_err(err)?;
    Ok(n > 0)
}

/// The driving edges (`private`: the bus lanes and the other private edges, with `access`) of the ways `ways`, with the
/// columns that `bus::routes` reads.
fn edges(con: &Connection, private: bool, ways: &str) -> Result<Vec<Edge>, String> {
    let (table, access) = if private { ("private_edges", "access") } else { ("edges", "NULL::VARCHAR") };
    let sql = format!("SELECT edge_id, edge_ref, osm_id, refs, {access} FROM driving.{table} WHERE osm_id IN ({ways})");
    let attrs = Arc::new(Attrs {
        highway: String::new(),
        name: None,
        maxspeed: None,
        surface: None,
        junction: None,
        layer: None,
        bridge: None,
        tunnel: None,
        service: None,
    });
    let text = |b: &duckdb::arrow::array::RecordBatch, i| duckdb::arrow::compute::cast(b.column(i), &DataType::Utf8).map_err(err);
    let mut out = Vec::new();
    let mut st = con.prepare(&sql).map_err(err)?;
    for b in st.query_arrow([]).map_err(err)? {
        let id = duckdb::arrow::compute::cast(b.column(0), &DataType::Int64).map_err(err)?;
        let osm = duckdb::arrow::compute::cast(b.column(2), &DataType::Int64).map_err(err)?;
        let list = DataType::List(Arc::new(Field::new("item", DataType::Int64, true)));
        let refs = duckdb::arrow::compute::cast(b.column(3), &list).map_err(err)?;
        let (eref, acc) = (text(&b, 1)?, text(&b, 4)?);
        let (id, osm, refs) = (id.as_primitive::<Int64Type>(), osm.as_primitive::<Int64Type>(), refs.as_list::<i32>());
        let (eref, acc) = (eref.as_string::<i32>(), acc.as_string::<i32>());
        for r in 0..b.num_rows() {
            // edge_ref is `{osm_id}#{seq}{f|r}`: Edge::edge_ref makes it again from seq and is_reverse
            let er = (!eref.is_null(r)).then(|| eref.value(r));
            let seq = er.and_then(|s| s.split_once('#')).and_then(|(_, t)| t[..t.len().saturating_sub(1)].parse().ok());
            out.push(Edge {
                edge_id: id.value(r),
                seq,
                source: 0,
                target: 0,
                osm_id: osm.value(r),
                attrs: attrs.clone(),
                oneway: false,
                lanes: None,
                access: (!acc.is_null(r)).then(|| Arc::from(acc.value(r))),
                refs: refs.value(r).as_primitive::<Int64Type>().values().to_vec(),
                geometry: Vec::new(),
                is_reverse: er.is_some_and(|s| s.ends_with('r')),
                length_m: 0.0,
                dismount: None,
                maxspeed_kmh: None,
                cost_s: None,
                ftype: None,
            });
        }
    }
    Ok(out)
}

/// The counts of `duckosm bus`: the routes, the rows of `bus.route_edges`, the routes with gaps.
pub struct BusCounts {
    pub routes: usize,
    pub edges: usize,
    pub with_gaps: usize,
}

/// `bus.routes` and `bus.route_edges` of `db` from its `raw.relations`, `raw.ways` and driving network, in place of the
/// tables that are there (docs/concepts/networks.md#bus-routes).
pub fn bus_routes(db: &Path) -> Result<BusCounts, String> {
    {
        let con = Connection::open(db).map_err(err)?;
        for (s, t) in [("raw", "relations"), ("raw", "ways"), ("driving", "edges"), ("driving", "private_edges")] {
            if !has_table(&con, s, t)? {
                return Err(format!("needs raw.relations, raw.ways and the driving network: {s}.{t} is not in {}", db.display()));
            }
        }
    }
    let input = RawInput::open(db)?;
    let raw = input.raw_part(&[Vec::new(), Vec::new(), Vec::new()], &duckosm_core::bus::ROUTES)?;
    input.close();
    let mut ways: Vec<String> = raw
        .relations
        .iter()
        .flat_map(|r| r.members.iter())
        .filter(|m| m.1 == MemberType::Way)
        .map(|m| m.0.to_string())
        .collect();
    ways.push("NULL".into());
    let ways = ways.join(",");
    let con = open_rw(db)?;
    let (e, p) = (edges(&con, false, &ways)?, edges(&con, true, &ways)?);
    let (routes, rows) = duckosm_core::bus::routes(&raw.relations, &raw.ways, &e, &p);
    // the tables through Parquet files, as the build loads them (the same column types)
    let dir = std::env::temp_dir().join(format!("duckosm-bus-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(err)?;
    let file = |n: &str| std::fs::File::create(dir.join(n)).map_err(|e| format!("{n}: {e}"));
    let written = duckosm_parquet::write_routes_to(file("routes_bus.parquet")?, &routes)
        .and_then(|_| duckosm_parquet::write_route_edges_to(file("route_edges_bus.parquet")?, &rows))
        .and_then(|_| con.execute_batch("DROP TABLE IF EXISTS bus.routes; DROP TABLE IF EXISTS bus.route_edges;").map_err(err))
        .and_then(|_| crate::write_schema(&con, &dir, "bus", &["routes", "route_edges"]))
        .and_then(|_| con.execute_batch("CHECKPOINT;").map_err(err));
    let _ = std::fs::remove_dir_all(&dir);
    written?;
    Ok(BusCounts { routes: routes.len(), edges: rows.len(), with_gaps: routes.iter().filter(|r| !r.gaps.is_empty()).count() })
}

/// The schemas that are never the layer of a mode.
const NOT_MODES: [&str; 4] = ["information_schema", "pg_catalog", "main", "raw"];

/// `{schema}.edges` (a view: the edges of each mode with a `mode` column) and `{schema}.transfers` (the coarse transfers:
/// at each node that walking shares with another mode, a change to that mode and a change back) of `db`, in place of what
/// is there (docs/concepts/multimodal.md). `transfer_s` is the cost of a change in seconds; `per_pair` (`"from->to"`,
/// seconds) gives another cost to a direction. A database without walking or without a second mode gets an empty
/// `transfers`. Returns the number of rows of the two tables.
pub fn multimodal(db: &Path, transfer_s: f64, per_pair: &[(String, f64)], schema: &str) -> Result<(i64, i64), String> {
    if !schema.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || schema.is_empty() {
        return Err(format!("not a schema name: {schema}"));
    }
    let con = open_rw(db)?;
    let mut st = con
        .prepare("SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name = 'edges' ORDER BY schema_name")
        .map_err(err)?;
    let modes: Vec<String> = st
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?
        .into_iter()
        .filter(|m| !NOT_MODES.contains(&m.as_str()) && m != schema)
        .collect();
    if let Some(m) = modes.first() {
        let n: i64 = con
            .query_row("SELECT count(*) FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'edges' AND column_name = 'cost_s'", [m], |r| r.get(0))
            .map_err(err)?;
        if n == 0 {
            return Err("multimodal routing is weighted in seconds but `edges.cost_s` is missing: build again with \
                        options.calculate_costs = true (and process_speeds)"
                .into());
        }
    }
    // a view replaces a view only; an old file can have a table here
    let mut sql = format!("CREATE SCHEMA IF NOT EXISTS {schema};\n");
    let kind: Option<String> = con
        .query_row(
            "SELECT table_type FROM information_schema.tables WHERE table_catalog = current_database() AND table_schema = ? AND table_name = 'edges'",
            [schema],
            |r| r.get(0),
        )
        .ok();
    match kind.as_deref() {
        Some("VIEW") => sql += &format!("DROP VIEW {schema}.edges;\n"),
        Some(_) => sql += &format!("DROP TABLE {schema}.edges;\n"),
        None => {}
    }
    let cols = "edge_id, source, target, cost_s, length_m, highway, name, geometry";
    if modes.is_empty() {
        sql += &format!(
            "CREATE VIEW {schema}.edges AS SELECT NULL::VARCHAR AS mode, NULL::BIGINT AS edge_id, NULL::BIGINT AS source, \
             NULL::BIGINT AS target, NULL::DOUBLE AS cost_s, NULL::DOUBLE AS length_m, NULL::VARCHAR AS highway, \
             NULL::VARCHAR AS name, NULL::GEOMETRY AS geometry WHERE FALSE;\n"
        );
    } else {
        let parts: Vec<String> = modes.iter().map(|m| format!("SELECT '{m}' AS mode, {cols} FROM {m}.edges")).collect();
        sql += &format!("CREATE VIEW {schema}.edges AS {};\n", parts.join(" UNION ALL "));
    }
    sql += &format!(
        "CREATE OR REPLACE TABLE {schema}.transfers (node_id BIGINT, from_mode VARCHAR, to_mode VARCHAR, cost_s DOUBLE, kind VARCHAR);\n"
    );
    let cost = |from: &str, to: &str| {
        let pair = format!("{from}->{to}");
        per_pair.iter().find(|p| p.0 == pair).map_or(transfer_s, |p| p.1)
    };
    if modes.iter().any(|m| m == "walking") {
        for v in modes.iter().filter(|m| *m != "walking") {
            let (to, back) = match v.as_str() {
                "driving" => ("park", "retrieve"),
                "cycling" => ("bike_park", "bike_unpark"),
                _ => ("enter", "exit"),
            };
            let shared = format!("(SELECT node_id FROM walking.nodes INTERSECT SELECT node_id FROM {v}.nodes)");
            sql += &format!(
                "INSERT INTO {schema}.transfers SELECT node_id, 'walking', '{v}', {:?}, '{to}' FROM {shared};\n\
                 INSERT INTO {schema}.transfers SELECT node_id, '{v}', 'walking', {:?}, '{back}' FROM {shared};\n",
                cost("walking", v),
                cost(v, "walking")
            );
        }
    }
    sql += "CHECKPOINT;";
    con.execute_batch(&sql).map_err(|e| format!("{schema}: {e}"))?;
    let count = |t: &str| con.query_row(&format!("SELECT count(*) FROM {schema}.{t}"), [], |r| r.get::<_, i64>(0)).map_err(err);
    Ok((count("edges")?, count("transfers")?))
}
