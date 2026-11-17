//! `source.type: duckdb` (duckOSM `processors/duckdb_clipper.py`): the area is a clip of a parent database, with no PBF.
//! Each mode gets the rows of the parent's tables in the boundary, copied as they are, so `edge_id` does not change. Then
//! `main.boundary`, `main.visualization_metadata` (area.rs), the drawing order (levels.rs) and the steps of after.rs.

use crate::{Params, Summary};
use std::path::Path;

#[cfg(not(feature = "duckdb"))]
pub fn build(_: &Path, _: &Path, _: &Params, _: &str) -> Result<Summary, String> {
    Err("source.type: duckdb needs the cargo feature duckdb".into())
}

#[cfg(feature = "duckdb")]
pub fn build(parent: &Path, out: &Path, p: &Params, predicate: &str) -> Result<Summary, String> {
    use crate::levels::{LevelEdges, LevelTags};
    use duckosm_core::filter::Mode;
    use duckosm_core::model::Node;
    use duckosm_duckdb::duckdb::Connection;

    let err = |e: duckosm_duckdb::duckdb::Error| e.to_string();
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let mut log = crate::StepLog::system();
    let bpath = p.area_boundary.as_ref().or(p.boundary.as_ref()).ok_or("a boundary is required to clip from a duckdb source")?;
    let db = out.join(format!("{}.duckdb", p.name));
    for f in [db.clone(), out.join(format!("{}.duckdb.wal", p.name))] {
        let _ = std::fs::remove_file(f);
    }
    let pred = match predicate {
        "within" => "ST_Within(e.geometry, b.geom)",
        "centroid" => "ST_Within(ST_Centroid(e.geometry), b.geom)",
        _ => "ST_Intersects(e.geometry, b.geom)",
    };
    let modes: Vec<&str> = p.modes.iter().map(|m| m.name()).collect();
    let con = Connection::open(&db).map_err(err)?;
    con.execute_batch(&format!(
        "INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;
         ATTACH '{}' AS parent (READ_ONLY);
         CREATE TEMP TABLE clip_area AS SELECT geom::GEOMETRY AS geom FROM ST_Read('{}');",
        q(parent),
        q(bpath)
    ))
    .map_err(|e| format!("clip: {e}"))?;
    let has = |schema: &str, table: &str| -> Result<bool, String> {
        con.query_row(
            "SELECT count(*) FROM duckdb_tables() WHERE database_name = 'parent' AND schema_name = ? AND table_name = ?",
            [schema, table],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(err)
    };
    let mut sum = Summary::default();
    for m in &modes {
        if !has(m, "edges")? {
            return Err(format!("clip: the parent database has no {m}.edges"));
        }
        // the SQL of DuckdbClipper: the edges in the boundary, the private edges, the nodes of the edges, and the rows of the
        // graph and of the turn tables whose edges are all kept
        let mut sql = format!(
            "CREATE SCHEMA {m};
             CREATE TABLE {m}.edges AS SELECT e.* FROM parent.{m}.edges e WHERE EXISTS (SELECT 1 FROM clip_area b WHERE {pred});"
        );
        let mut live = format!("SELECT edge_id FROM {m}.edges");
        if has(m, "private_edges")? {
            sql += &format!(
                "CREATE TABLE {m}.private_edges AS SELECT e.* FROM parent.{m}.private_edges e WHERE EXISTS (SELECT 1 FROM clip_area b WHERE {pred});"
            );
            live += &format!(" UNION ALL SELECT edge_id FROM {m}.private_edges");
        }
        sql += &format!(
            "CREATE TABLE {m}.nodes AS SELECT n.* FROM parent.{m}.nodes n
               WHERE n.node_id IN (SELECT source FROM {m}.edges UNION SELECT target FROM {m}.edges);"
        );
        let kept = format!("(SELECT edge_id FROM {m}.edges)");
        if has(m, "edge_graph")? {
            sql += &format!("CREATE TABLE {m}.edge_graph AS SELECT g.* FROM parent.{m}.edge_graph g WHERE g.from_edge IN ({live}) AND g.to_edge IN ({live});");
        }
        if has(m, "turn_restrictions")? {
            sql += &format!(
                "CREATE TABLE {m}.turn_restrictions AS SELECT t.* FROM parent.{m}.turn_restrictions t WHERE t.from_edge_id IN {kept} AND t.to_edge_id IN {kept};"
            );
        }
        if has(m, "turn_permission")? {
            sql += &format!("CREATE TABLE {m}.turn_permission AS SELECT t.* FROM parent.{m}.turn_permission t WHERE t.from_edge IN {kept} AND t.to_edge IN {kept};");
        }
        // duckOSM's SQL (list_bool_and over a lambda with a subquery) fails in DuckDB, and duckOSM drops the error: its clip
        // has no turn_path_restrictions. This one keeps the paths whose edges are all in the clip.
        if has(m, "turn_path_restrictions")? {
            sql += &format!(
                "CREATE TABLE {m}.turn_path_restrictions AS SELECT t.* FROM parent.{m}.turn_path_restrictions t
                   WHERE t.from_edge IN {kept} AND t.to_edge IN {kept}
                     AND NOT EXISTS (SELECT 1 FROM unnest(t.via_edges) AS v(id) WHERE v.id NOT IN {kept});"
            );
        }
        con.execute_batch(&sql).map_err(|e| format!("clip {m}: {e}"))?;
        let n = |t: &str| con.query_row(&format!("SELECT count(*) FROM {m}.{t}"), [], |r| r.get::<_, i64>(0)).map_err(err);
        eprintln!("  [{m}] clipped {} edges / {} nodes from parent.{m} (predicate={predicate}, edge_ids preserved)", n("edges")?, n("nodes")?);
    }
    log.step("clip");

    // the nodes of driving (else of the first mode): the point of the time zone
    let first = if modes.contains(&"driving") { "driving" } else { modes[0] };
    let mut st = con.prepare(&format!("SELECT node_id, ST_X(geom), ST_Y(geom) FROM {first}.nodes")).map_err(err)?;
    let nodes: Vec<Node> = st
        .query_map([], |r| Ok(Node { node_id: r.get(0)?, lon: r.get(1)?, lat: r.get(2)? }))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let mode_nodes = vec![(Mode::parse_all(&[first])?[0], nodes)];

    // the drawing order from the clipped edges, in duckOSM's order of the tables (driving, walking, cycling, then their
    // private edges)
    let mut level_edges = LevelEdges::default();
    if p.levels != crate::LevelsMethod::None {
        for (rank, m) in ["driving", "walking", "cycling"].iter().enumerate() {
            if !modes.contains(m) {
                continue;
            }
            for (t, add) in [("edges", 0u8), ("private_edges", 3)] {
                let exists: i64 = con
                    .query_row("SELECT count(*) FROM duckdb_tables() WHERE database_name = current_database() AND schema_name = ? AND table_name = ?", [m, &t], |r| r.get(0))
                    .map_err(err)?;
                if exists == 0 {
                    continue;
                }
                let col = |c: &str| -> Result<String, String> {
                    let n: i64 = con
                        .query_row(
                            "SELECT count(*) FROM information_schema.columns WHERE table_catalog = current_database() AND table_schema = ? AND table_name = ? AND column_name = ?",
                            [m, &t, c],
                            |r| r.get(0),
                        )
                        .map_err(err)?;
                    Ok(if n > 0 { format!("{c}::VARCHAR") } else { "NULL::VARCHAR".into() })
                };
                let mut st = con
                    .prepare(&format!(
                        "SELECT edge_id, highway, {}, {}, {}, {}, ST_AsWKB(geometry) FROM {m}.{t}",
                        col("tunnel")?,
                        col("bridge")?,
                        col("layer")?,
                        col("junction")?
                    ))
                    .map_err(err)?;
                let mut rows = st.query([]).map_err(err)?;
                while let Some(r) = rows.next().map_err(err)? {
                    let wkb: Vec<u8> = r.get(6).map_err(err)?;
                    let tags = LevelTags {
                        highway: r.get::<_, Option<String>>(1).map_err(err)?.unwrap_or_default(),
                        tunnel: r.get(2).map_err(err)?,
                        bridge: r.get(3).map_err(err)?,
                        layer: r.get(4).map_err(err)?,
                        junction: r.get(5).map_err(err)?,
                        band: None,
                    };
                    level_edges.add_row(rank as u8 + add, r.get(0).map_err(err)?, tags, line_of_wkb(&wkb)?);
                }
            }
        }
    }
    drop(st);
    con.execute_batch("DETACH parent; CHECKPOINT;").map_err(err)?;
    drop(con);

    // main.boundary, main.visualization_metadata and the levels: files in a temporary folder, then into the database
    let tmp = std::env::temp_dir().join(format!("duckosm-clip-{}-{}", std::process::id(), p.name));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let res = (|| -> Result<(), String> {
        let file = |name: &str| std::fs::File::create(tmp.join(name)).map(std::io::BufWriter::new).map_err(|e| e.to_string());
        let text = std::fs::read_to_string(bpath).map_err(|e| format!("{}: {e}", bpath.display()))?;
        let opts = duckosm_core::network::Options { global_junctions: false, calculate_costs: false, ..Default::default() };
        let built_at = p.built_at.unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as i64)
        });
        let area = crate::Area {
            boundary: Some(&text),
            ways: &[],
            nodes_box: [0.0; 4],
            nodes: &mode_nodes,
            junctions: &Default::default(),
            opts: &opts,
            built_at,
            lookup: None,
            timezone_entry: p.timezone_entry.as_deref(),
        };
        crate::write_area_tables(&area, &file)?;
        duckosm_duckdb::write_area_tables(&db, &tmp)?;
        if let Some(line) = crate::write_levels(&level_edges, p.levels, built_at, &file)? {
            sum.levels = Some(line);
            duckosm_duckdb::write_levels(&db, &tmp)?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    res?;
    log.step("area tables and levels");
    crate::after::run(&db, &p.name, &modes, &p.after)?;
    log.step("indexes and checks");
    sum.files.push(("duckdb".to_string(), db));
    sum.steps = log.rows.iter().map(|(s, ms, _)| (s.clone(), ms / 1000.0)).collect();
    Ok(sum)
}

/// The points of a LINESTRING in WKB (as DuckDB's `ST_AsWKB` writes it: little-endian, 2D).
#[cfg(feature = "duckdb")]
fn line_of_wkb(b: &[u8]) -> Result<Vec<duckosm_core::geom::Pt>, String> {
    let le = b.first() == Some(&1);
    let u32_at = |i: usize| -> Option<u32> {
        let a: [u8; 4] = b.get(i..i + 4)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    };
    let f64_at = |i: usize| -> Option<f64> {
        let a: [u8; 8] = b.get(i..i + 8)?.try_into().ok()?;
        Some(if le { f64::from_le_bytes(a) } else { f64::from_be_bytes(a) })
    };
    if u32_at(1) != Some(2) {
        return Err("clip: an edge geometry is not a LINESTRING".into());
    }
    let n = u32_at(5).ok_or("clip: a short WKB")? as usize;
    (0..n)
        .map(|k| Some((f64_at(9 + 16 * k)?, f64_at(17 + 16 * k)?)))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "clip: a short WKB".into())
}
