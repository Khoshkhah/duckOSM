//! `extract` (duckOSM `extract.py`): slice a sub-area out of a built database into a new, self-contained database. An edge
//! is kept whole if its geometry intersects the area; each mode keeps the nodes, the graph, the turn tables and the ways of
//! its edges. `main.boundary`, a new `main.visualization_metadata` (with the time zone) and the admin boundaries inside the
//! area are added; `raw.*` is not copied.

use crate::Result;
use duckdb::Connection;
use std::path::Path;

fn err(e: duckdb::Error) -> String {
    e.to_string()
}

/// The tables of a mode and how each is cut to the kept edges (duckOSM `MODE_TABLES`, in its order).
const MODE_TABLES: [(&str, &str); 7] = [
    ("nodes", "node_id IN (SELECT source FROM {sch}.edges UNION SELECT target FROM {sch}.edges)"),
    ("edge_graph", "from_edge IN ({live}) AND to_edge IN ({live})"),
    ("turn_restrictions", "from_edge_id IN (SELECT edge_id FROM {sch}.edges) AND to_edge_id IN (SELECT edge_id FROM {sch}.edges)"),
    ("turn_permission", "from_edge IN (SELECT edge_id FROM {sch}.edges) AND to_edge IN (SELECT edge_id FROM {sch}.edges)"),
    ("turn_path_restrictions", "from_edge IN (SELECT edge_id FROM {sch}.edges) AND to_edge IN (SELECT edge_id FROM {sch}.edges)"),
    ("ways", "osm_id IN (SELECT DISTINCT osm_id FROM {sch}.edges)"),
    ("way_nodes", "way_id IN (SELECT DISTINCT osm_id FROM {sch}.edges)"),
];

/// The area: a name or an OSM id in the source's `main.admin_boundaries`, or a GeoJSON file.
pub enum Area<'a> {
    Name(&'a str),
    OsmId(i64),
    Boundary(&'a Path),
}

/// The temporary table `_clip(geom, osm_id, name, admin_level)` with the area (duckOSM `build_clip`); its label.
fn build_clip(con: &Connection, area: &Area) -> Result<String> {
    let label = match area {
        Area::Boundary(b) => {
            let b = b.canonicalize().map_err(|e| format!("{}: {e}", b.display()))?;
            con.execute_batch(&format!(
                "CREATE TEMP TABLE _clip AS SELECT ST_Union_Agg(geom)::GEOMETRY AS geom, NULL::BIGINT AS osm_id,
                        NULL::VARCHAR AS name, NULL::INTEGER AS admin_level FROM ST_Read('{}')",
                b.display().to_string().replace('\'', "''")
            ))
            .map_err(err)?;
            b.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
        }
        _ => {
            let mut st = con
                .prepare("SELECT column_name FROM duckdb_columns() WHERE database_name='src' AND schema_name='main' AND table_name='admin_boundaries'")
                .map_err(err)?;
            let cols: Vec<String> = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
            if cols.is_empty() {
                return Err("Source has no main.admin_boundaries table — use --boundary instead.".into());
            }
            match area {
                Area::OsmId(id) => con
                    .execute_batch(&format!(
                        "CREATE TEMP TABLE _clip AS SELECT geometry AS geom, osm_id, name, admin_level FROM src.main.admin_boundaries WHERE osm_id = {id}"
                    ))
                    .map_err(err)?,
                Area::Name(n) => {
                    // duckOSM area.admin_match_sql
                    let norm = |x: &str| format!("replace(strip_accents(lower({x})), '-', ' ')");
                    let (q, nm) = (norm("$1"), norm("name"));
                    let en = if cols.iter().any(|c| c == "name_en") { norm("coalesce(name_en, '')") } else { "''".into() };
                    con.execute(
                        &format!(
                            "CREATE TEMP TABLE _clip AS SELECT geometry AS geom, osm_id, name, admin_level FROM src.main.admin_boundaries
                             WHERE {nm} LIKE '%' || {q} || '%' OR {en} LIKE '%' || {q} || '%'
                             ORDER BY ({nm} = {q} OR {en} = {q}) DESC, ST_Area(geometry) DESC LIMIT 1"
                        ),
                        [n],
                    )
                    .map_err(err)?;
                }
                Area::Boundary(_) => unreachable!(),
            }
            let row: Option<(Option<String>, Option<i32>)> = {
                let mut st = con.prepare("SELECT name, admin_level FROM _clip").map_err(err)?;
                let mut rows = st.query([]).map_err(err)?;
                match rows.next().map_err(err)? {
                    Some(r) => Some((r.get(0).map_err(err)?, r.get(1).map_err(err)?)),
                    None => None,
                }
            };
            let Some((name, level)) = row else {
                let what = match area {
                    Area::Name(n) => n.to_string(),
                    Area::OsmId(id) => id.to_string(),
                    Area::Boundary(_) => unreachable!(),
                };
                return Err(format!("No admin boundary matched '{what}'."));
            };
            let s = |v: Option<String>| v.unwrap_or_else(|| "None".into());
            format!("{} (admin_level {})", s(name), s(level.map(|l| l.to_string())))
        }
    };
    let n: i64 = con.query_row("SELECT count(*) FROM _clip WHERE geom IS NOT NULL", [], |r| r.get(0)).map_err(err)?;
    if n == 0 {
        return Err("Resolved boundary is empty.".into());
    }
    Ok(label)
}

/// `duckosm extract --source SRC --db DB (--name N | --osm-id ID | --boundary FILE)`.
pub fn extract(source: &Path, db: &Path, area: Area) -> Result<()> {
    if !source.exists() {
        return Err(format!("Source not found: {}", source.display()));
    }
    let _ = std::fs::remove_file(db);
    if let Some(d) = db.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let con = Connection::open(db).map_err(|e| format!("{}: {e}", db.display()))?;
    let src = source.canonicalize().map_err(|e| e.to_string())?;
    con.execute_batch(&format!(
        "INSTALL spatial; LOAD spatial; SET geometry_always_xy = false; ATTACH '{}' AS src (READ_ONLY);",
        src.display().to_string().replace('\'', "''")
    ))
    .map_err(err)?;
    let label = build_clip(&con, &area)?;
    println!("Area: {label}");
    let (x0, y0, x1, y1): (f64, f64, f64, f64) = con
        .query_row("SELECT ST_XMin(geom), ST_YMin(geom), ST_XMax(geom), ST_YMax(geom) FROM _clip", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(err)?;
    let bbox = format!(
        "ST_XMax(geometry) >= {} AND ST_XMin(geometry) <= {} AND ST_YMax(geometry) >= {} AND ST_YMin(geometry) <= {}",
        duckosm_build::place::py_float(x0),
        duckosm_build::place::py_float(x1),
        duckosm_build::place::py_float(y0),
        duckosm_build::place::py_float(y1)
    );
    let mut st = con.prepare("SELECT schema_name, table_name FROM duckdb_tables() WHERE database_name='src'").map_err(err)?;
    let src_tables: Vec<(String, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
    // the schemas in the order they come first (duckOSM: a dict)
    let mut schemas: Vec<String> = Vec::new();
    for (s, _) in &src_tables {
        if !schemas.contains(s) {
            schemas.push(s.clone());
        }
    }
    let has = |s: &str, t: &str| src_tables.iter().any(|x| x.0 == s && x.1 == t);
    let modes: Vec<String> = schemas
        .iter()
        .filter(|s| !["information_schema", "pg_catalog", "main", "raw"].contains(&s.as_str()) && has(s, "edges"))
        .cloned()
        .collect();
    let mut summary = Vec::new();
    for sch in &modes {
        let mut sql = format!(
            "CREATE SCHEMA IF NOT EXISTS {sch};
             CREATE OR REPLACE TABLE {sch}.edges AS SELECT * FROM src.{sch}.edges WHERE {bbox} AND ST_Intersects(geometry, (SELECT geom FROM _clip));"
        );
        let private = has(sch, "private_edges");
        if private {
            sql += &format!(
                "CREATE OR REPLACE TABLE {sch}.private_edges AS SELECT * FROM src.{sch}.private_edges WHERE {bbox} AND ST_Intersects(geometry, (SELECT geom FROM _clip));"
            );
        }
        con.execute_batch(&sql).map_err(|e| format!("{sch}: {e}"))?;
        let n: i64 = con.query_row(&format!("SELECT count(*) FROM {sch}.edges"), [], |r| r.get(0)).map_err(err)?;
        let live = format!("SELECT edge_id FROM {sch}.edges{}", if private { format!(" UNION ALL SELECT edge_id FROM {sch}.private_edges") } else { String::new() });
        for (t, pred) in MODE_TABLES {
            if has(sch, t) {
                con.execute_batch(&format!(
                    "CREATE OR REPLACE TABLE {sch}.{t} AS SELECT * FROM src.{sch}.{t} WHERE {}",
                    pred.replace("{sch}", sch).replace("{live}", &live)
                ))
                .map_err(|e| format!("{sch}.{t}: {e}"))?;
            }
        }
        summary.push((sch.clone(), n));
    }
    con.execute_batch("CREATE SCHEMA IF NOT EXISTS main; CREATE OR REPLACE TABLE main.boundary AS SELECT geom FROM _clip;").map_err(err)?;
    if has("main", "admin_boundaries") {
        con.execute_batch(
            "CREATE OR REPLACE TABLE main.admin_boundaries AS SELECT a.* FROM src.main.admin_boundaries a
             WHERE ST_Within(ST_PointOnSurface(a.geometry), (SELECT geom FROM _clip))
               AND ((SELECT osm_id FROM _clip) IS NULL OR a.osm_id <> (SELECT osm_id FROM _clip))",
        )
        .map_err(err)?;
    }
    con.execute_batch(
        "CREATE OR REPLACE TABLE main.visualization_metadata AS
         SELECT ST_AsGeoJSON(geom) AS boundary_geojson, ST_Y(ST_Centroid(geom)) AS center_lat, ST_X(ST_Centroid(geom)) AS center_lon,
                CAST(LEAST(16, GREATEST(1, LOG2(360.0 / NULLIF(ST_XMax(geom) - ST_XMin(geom), 0)))) AS INTEGER) AS initial_zoom
         FROM _clip",
    )
    .map_err(err)?;
    // duckOSM utils.add_timezone: at the node nearest the median of the nodes of driving (else of another mode)
    let mode: Option<String> = con
        .query_row(
            "SELECT table_schema FROM information_schema.tables WHERE table_name = 'nodes' AND table_schema IN ('driving', 'walking', 'cycling')
             AND table_catalog = current_database() ORDER BY table_schema = 'driving' DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok();
    let mode = mode.ok_or("cannot set the time zone: no <mode>.nodes table")?;
    let (lon, lat): (f64, f64) = con
        .query_row(
            &format!(
                "WITH n AS (SELECT ST_X(geom) AS x, ST_Y(geom) AS y FROM {mode}.nodes WHERE node_id > 0),
                      m AS (SELECT median(x) AS mx, median(y) AS my FROM n)
                 SELECT x, y FROM n, m ORDER BY (x - mx) * (x - mx) + (y - my) * (y - my) LIMIT 1"
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    let tz = duckosm_build::area::timezone_at(lon, lat).ok_or(format!("no time zone found at lat={lat}, lon={lon}"))?;
    con.execute_batch("ALTER TABLE main.visualization_metadata ADD COLUMN IF NOT EXISTS timezone VARCHAR;").map_err(err)?;
    con.execute("UPDATE main.visualization_metadata SET timezone = ?", [&tz]).map_err(err)?;
    println!("  Timezone: {tz}");
    // duckOSM utils.stamp_build
    con.execute_batch(
        "ALTER TABLE main.visualization_metadata ADD COLUMN IF NOT EXISTS built_at TIMESTAMP;
         ALTER TABLE main.visualization_metadata ADD COLUMN IF NOT EXISTS duckosm_version VARCHAR;",
    )
    .map_err(err)?;
    con.execute(
        "UPDATE main.visualization_metadata SET built_at = now()::TIMESTAMP, duckosm_version = ?",
        [format!("duckosm-rust {}", env!("CARGO_PKG_VERSION"))],
    )
    .map_err(err)?;
    con.execute_batch(&format!("DROP TABLE _clip; DETACH src; {} CHECKPOINT;", duckosm_duckdb::MACROS)).map_err(err)?;
    drop(con);
    println!("Wrote {}", db.display());
    for (sch, n) in summary {
        println!("  {sch}: {} edges", crate::info::thousands(n));
    }
    Ok(())
}
