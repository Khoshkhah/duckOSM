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

/// Write the comments of the tables and the columns (COMMENT ON). Each item is (schema, table, column, text); the
/// column is `None` for the comment of the table. The step skips an item for a table or a column that is not in the
/// file, and a view. It does not change a row or a type.
pub fn comments(db: &Path, items: &[(String, String, Option<String>, String)]) -> Result<(), String> {
    use std::collections::HashSet;
    let con = Connection::open(db).map_err(err)?;
    let mut stmt = con
        .prepare(
            "SELECT c.schema_name, c.table_name, c.column_name FROM duckdb_columns() c JOIN duckdb_tables() t
             USING (database_name, schema_name, table_name) WHERE c.database_name = current_database()",
        )
        .map_err(err)?;
    let mut tables = HashSet::new();
    let mut columns = HashSet::new();
    for r in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).map_err(err)? {
        let (s, t, c) = r.map_err(err)?;
        tables.insert((s.clone(), t.clone()));
        columns.insert((s, t, c));
    }
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let id = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut sql = String::from("BEGIN;\n");
    for (s, t, c, text) in items {
        match c {
            None if tables.contains(&(s.clone(), t.clone())) => {
                sql += &format!("COMMENT ON TABLE {}.{} IS {};\n", id(s), id(t), q(text));
            }
            Some(c) if columns.contains(&(s.clone(), t.clone(), c.clone())) => {
                sql += &format!("COMMENT ON COLUMN {}.{}.{} IS {};\n", id(s), id(t), id(c), q(text));
            }
            _ => {}
        }
    }
    sql += "COMMIT;";
    con.execute_batch(&sql).map_err(|e| format!("comments: {e}"))
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

/// duckOSM `multimodal.node_costs` and `only_listed_nodes` on `mm.transfers` (see [`crate::steps::apply_node_costs`]).
pub fn node_costs(db: &Path, file: &Path, only_listed: bool) -> Result<(), String> {
    let con = Connection::open(db).map_err(err)?;
    if !has_table(&con, "mm", "transfers")? {
        return Ok(());
    }
    crate::steps::apply_node_costs(&con, "mm", file, only_listed)?;
    con.execute_batch("CHECKPOINT;").map_err(err)
}

/// No `mm.*`: duckOSM makes them only with `multimodal.enabled`.
pub fn drop_multimodal(db: &Path) -> Result<(), String> {
    let con = Connection::open(db).map_err(err)?;
    con.execute_batch("DROP SCHEMA IF EXISTS mm CASCADE; CHECKPOINT;").map_err(err)
}

/// duckOSM `importer._raw_way_fixes`: the OSM fixes on the `raw.*` of the file (a build with no boundary: `read_osm` wrote
/// them as the PBF has them). In duckOSM's order: `remove`, `keep_nodes` and `tags` of each rule, `extra.tag_edits`, then
/// `new_ways` (only when all their OSM nodes are in the file). Run it after the network step has read `raw.*`: that step
/// applies the fixes itself. Then the roads of the sidewalk rule (`sidewalks`, duckOSM `SeparateSidewalks`) get
/// `sidewalk=separate` and `duckosm:sidewalk=inferred`. Last, duckOSM's `OSMOverrides` (per mode): the arrows of a rule (`turn_lanes_*`) as the tags
/// `turn:lanes:*` of the raw way, once for each mode whose `ways` has it.
pub fn raw_fixes(db: &Path, f: &duckosm_core::fixes::OsmFixes, sidewalks: &[i64], modes: &[&str]) -> Result<(), String> {
    use duckosm_core::fixes::{Elem, NewNode};
    let s = |x: &str| format!("'{}'", x.replace('\'', "''"));
    let map = |kv: &[(String, String)]| {
        format!(
            "MAP([{}]::VARCHAR[], [{}]::VARCHAR[])",
            kv.iter().map(|(k, _)| s(k)).collect::<Vec<_>>().join(", "),
            kv.iter().map(|(_, v)| s(v)).collect::<Vec<_>>().join(", ")
        )
    };
    let con = Connection::open(db).map_err(err)?;
    let mut sql = String::new();
    // tags_from: a way's OSM tags before any fix changes them
    sql += "CREATE OR REPLACE TEMP TABLE fix_tags_from AS SELECT osm_id, tags FROM raw.ways WHERE osm_id IN (";
    sql += &f.new_ways.iter().filter_map(|n| n.tags_from).map(|i| i.to_string()).chain(["NULL".to_string()]).collect::<Vec<_>>().join(", ");
    sql += ");\n";
    let removed: Vec<String> = f.ways.iter().filter(|r| r.remove).map(|r| r.osm_id.to_string()).collect();
    if !removed.is_empty() {
        sql += &format!("DELETE FROM raw.ways WHERE osm_id IN ({});\n", removed.join(", "));
    }
    for r in f.ways.iter().filter(|r| !r.remove) {
        let id = r.osm_id;
        if let Some((a, b)) = r.keep_nodes {
            sql += &format!(
                "UPDATE raw.ways SET refs = refs[list_position(refs, {a}):list_position(refs, {b})] WHERE osm_id = {id} AND list_position(refs, {a}) < list_position(refs, {b});\n"
            );
        }
        if !r.tags.is_empty() {
            sql += &format!("UPDATE raw.ways SET tags = map_concat(tags, {}) WHERE osm_id = {id};\n", map(&r.tags));
        }
    }
    for e in &f.tag_edits {
        let table = match e.elem {
            Elem::Way => "ways",
            Elem::Node => "nodes",
            Elem::Relation => "relations",
        };
        let del = e.delete.iter().map(|k| s(k)).collect::<Vec<_>>().join(", ");
        sql += &format!(
            "UPDATE raw.{table} SET tags = map_from_entries(list_filter(map_entries(map_concat(coalesce(tags, MAP {{}}), {})), x -> NOT list_contains([{del}]::VARCHAR[], x.key))) WHERE osm_id = {};\n",
            map(&e.set),
            e.id
        );
    }
    con.execute_batch(&sql).map_err(|e| format!("raw.* fixes: {e}"))?;
    for n in &f.new_ways {
        let osm: Vec<i64> = n.nodes.iter().filter_map(|x| if let NewNode::Osm(i) = x { Some(*i) } else { None }).collect();
        let mut distinct = osm.clone();
        distinct.sort_unstable();
        distinct.dedup();
        let list = distinct.iter().map(i64::to_string).chain(["NULL".to_string()]).collect::<Vec<_>>().join(", ");
        if count(&con, &format!("SELECT count(DISTINCT osm_id) FROM raw.nodes WHERE osm_id IN ({list})"))? < distinct.len() as i64 {
            continue; // not in this area: nothing to add
        }
        let mut refs = Vec::new();
        let mut sql = String::new();
        for (pos, node) in n.nodes.iter().enumerate() {
            match node {
                NewNode::At((lon, lat)) => {
                    let nid = n.id * 1000 + pos as i64 + 1;
                    sql += &format!("INSERT INTO raw.nodes (osm_id, lat, lon) VALUES ({nid}, {lat:?}, {lon:?});\n");
                    refs.push(nid);
                }
                NewNode::Osm(i) => refs.push(*i),
            }
        }
        let from = n.tags_from.map_or("NULL".to_string(), |i| i.to_string());
        let refs = refs.iter().map(i64::to_string).collect::<Vec<_>>().join(", ");
        sql += &format!(
            "INSERT INTO raw.ways SELECT {}, map_concat(coalesce((SELECT tags FROM fix_tags_from WHERE osm_id = {from}), MAP {{}}), {}), [{refs}]::BIGINT[];\n",
            n.id,
            map(&n.tags)
        );
        con.execute_batch(&sql).map_err(|e| format!("raw.* fixes, new way {}: {e}", n.id))?;
    }
    let mut sql = String::new();
    if !sidewalks.is_empty() {
        sql += &format!(
            "UPDATE raw.ways SET tags = map_concat(tags, MAP {{'sidewalk': 'separate', 'duckosm:sidewalk': 'inferred'}}) WHERE osm_id IN ({});\n",
            sidewalks.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
        );
    }
    for m in modes {
        for r in f.ways.iter().filter(|r| !r.arrows.is_empty() && !r.exclude_modes.iter().any(|x| x.name() == *m)) {
            let id = r.osm_id;
            sql += &format!(
                "UPDATE raw.ways SET tags = map_concat(tags, {}) WHERE osm_id = {id} AND EXISTS (SELECT 1 FROM {m}.ways WHERE osm_id = {id});\n",
                map(&r.arrows)
            );
        }
    }
    con.execute_batch(&sql).map_err(|e| format!("raw.* arrows: {e}"))?;
    con.execute_batch("CHECKPOINT").map_err(err)
}

/// Write `<mode>.chains` (`duckosm_core::chains`): one row for each chain of the mode's edges, with the joined line. The
/// edges and the other tables do not change.
pub fn chains(db: &Path, mode: &str) -> Result<usize, String> {
    use duckosm_core::chains::{chain_id, chains as find, ChainEdge};
    use std::collections::HashSet;
    let con = Connection::open(db).map_err(err)?;
    con.execute_batch("INSTALL spatial; LOAD spatial;").map_err(err)?;
    let cols: HashSet<String> = {
        let mut st = con.prepare("SELECT column_name FROM information_schema.columns WHERE table_schema = ? AND table_name = 'edges'").map_err(err)?;
        let r = st.query_map([mode], |r| r.get::<_, String>(0)).map_err(err)?.collect::<Result<_, _>>().map_err(err)?;
        r
    };
    // the values that must be the same on the two sides of a join (the columns that the file has)
    let same = ["highway", "name", "lanes", "maxspeed_kmh", "maxspeed", "oneway", "layer", "bridge", "tunnel", "junction", "access", "service"];
    let same: Vec<&str> = same.into_iter().filter(|c| cols.contains(*c)).collect();
    let mut key: Vec<String> = same.iter().map(|c| format!("coalesce(e.\"{c}\"::VARCHAR, '-')")).collect();
    let profile = has_table(&con, mode, "lane_profile")?;
    let mut from = format!("{mode}.edges e");
    if profile {
        from += &format!(
            " LEFT JOIN (SELECT edge_id, string_agg(coalesce(use, '') || ':' || width_m::VARCHAR, ',' ORDER BY lane_num) AS p \
             FROM {mode}.lane_profile GROUP BY edge_id) p USING (edge_id)"
        );
        key.push("coalesce(p.p, '-')".into());
    }
    let mut st = con.prepare(&format!("SELECT e.edge_id, e.source, e.target, concat_ws('|', {}) FROM {from} ORDER BY e.edge_id", key.join(", "))).map_err(err)?;
    let edges: Vec<ChainEdge> = st
        .query_map([], |r| Ok(ChainEdge { id: r.get(0)?, source: r.get(1)?, target: r.get(2)?, key: r.get(3)? }))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let pairs = |sql: &str| -> Result<HashSet<(i64, i64)>, String> {
        let mut st = con.prepare(sql).map_err(err)?;
        let r = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?.collect::<Result<_, _>>().map_err(err);
        r
    };
    let ids = |sql: &str| -> Result<HashSet<i64>, String> {
        let mut st = con.prepare(sql).map_err(err)?;
        let r = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<Result<_, _>>().map_err(err);
        r
    };
    let mut turns = HashSet::new();
    if has_table(&con, mode, "edge_graph")? {
        let uses = count(&con, &format!("SELECT count(*) FROM information_schema.columns WHERE table_schema = '{mode}' AND table_name = 'edge_graph' AND column_name = 'uses'"))? > 0;
        let own = match mode { "walking" => "walk", "cycling" => "bike", _ => "car" };
        let w = if uses { format!(" WHERE uses = '{own}'") } else { String::new() };
        turns = pairs(&format!("SELECT from_edge, to_edge FROM {mode}.edge_graph{w}"))?;
    }
    let mut keep_turns = HashSet::new();
    if has_table(&con, mode, "turn_restrictions")? {
        keep_turns.extend(pairs(&format!("SELECT from_edge_id, to_edge_id FROM {mode}.turn_restrictions"))?);
    }
    if has_table(&con, mode, "turn_permission")? {
        keep_turns.extend(pairs(&format!("SELECT from_edge, to_edge FROM {mode}.turn_permission"))?);
    }
    let mut keep_edges = HashSet::new();
    if has_table(&con, mode, "turn_path_restrictions")? {
        keep_edges = ids(&format!(
            "SELECT from_edge FROM {mode}.turn_path_restrictions UNION SELECT to_edge FROM {mode}.turn_path_restrictions \
             UNION SELECT unnest(via_edges) FROM {mode}.turn_path_restrictions"
        ))?;
    }
    // a traffic light or a stop sign stays a node of the export (the SUMO node types: the same nodes as the SUMO export)
    let mut stop_nodes = HashSet::new();
    if has_table(&con, "raw", "nodes")? {
        if has_table(&con, mode, "way_nodes")? {
            stop_nodes = ids(&format!("SELECT node_id FROM ({})", duckosm_core::chains::NODE_TYPES_SQL.replace("{mode}", mode)))?;
        }
    }
    let cs = find(&edges, &turns, &stop_nodes, &keep_turns, &keep_edges);
    con.execute_batch("CREATE OR REPLACE TEMP TABLE chain_members (chain_id BIGINT, edge_id BIGINT, seq INTEGER)").map_err(err)?;
    {
        let mut ap = con.appender("chain_members").map_err(err)?;
        for c in &cs {
            let id = chain_id(c);
            for (i, e) in c.iter().enumerate() {
                ap.append_row(duckdb::params![id, e, (i + 1) as i32]).map_err(err)?;
            }
        }
    }
    let carried: Vec<String> = ["highway", "name", "lanes", "maxspeed_kmh", "oneway", "layer", "bridge", "tunnel", "junction", "access", "service"]
        .into_iter()
        .filter(|c| cols.contains(*c))
        .map(|c| format!("arg_min(e.\"{c}\", m.seq) AS \"{c}\""))
        .collect();
    let cost = if cols.contains("cost_s") { ", sum(e.cost_s)::FLOAT AS cost_s" } else { "" };
    con.execute_batch(&format!(
        "CREATE OR REPLACE TABLE {mode}.chains AS
         WITH p AS (
             SELECT m.chain_id, m.seq, unnest(range(CASE WHEN m.seq = 1 THEN 1 ELSE 2 END, ST_NPoints(e.geometry) + 1)) AS i, e.geometry
             FROM chain_members m JOIN {mode}.edges e USING (edge_id)),
         g AS (SELECT chain_id, ST_MakeLine(list(ST_PointN(geometry, i::INTEGER) ORDER BY seq, i)) AS geometry FROM p GROUP BY chain_id)
         SELECT m.chain_id, arg_min(e.source, m.seq) AS source, arg_max(e.target, m.seq) AS target,
                list(m.edge_id ORDER BY m.seq) AS edge_ids, sum(e.length_m)::FLOAT AS length_m{cost}, {carried}, any_value(g.geometry) AS geometry
         FROM chain_members m JOIN {mode}.edges e USING (edge_id) JOIN g USING (chain_id)
         GROUP BY m.chain_id ORDER BY min(e.rowid);
         DROP TABLE chain_members;",
        carried = carried.join(", ")
    ))
    .map_err(|e| format!("{mode}.chains: {e}"))?;
    Ok(cs.len())
}
