//! The GIS export (`export-gis`): GeoPackage, GeoParquet or ESRI shapefile, the port of src/duckosm/gis.py.
//! docs/design/gis_export.md.
//!
//! The layers are `edges_<mode>`, `nodes_<mode>` and `boundary` (GeoParquet: also `features_<table>`). DuckDB's spatial
//! extension writes each layer (`COPY ... (FORMAT gdal)` or `(FORMAT parquet)`). A GeoPackage gets one file: GDAL writes
//! each layer with no rows (the tables, the SRS and the `gpkg_*` rows, a few KB), SQLite joins them ([`join_gpkg`]), and
//! then the rows go from DuckDB into that file ([`fill_gpkg`]). So the export needs no disk for a second copy.

use crate::{open, Result};
use duckdb::Connection;
use std::path::{Path, PathBuf};

pub struct Summary {
    pub fmt: String,
    /// The GeoPackage file, or the folder of the shapefiles or the GeoParquet files.
    pub path: Option<PathBuf>,
    pub layers: Vec<(String, i64)>,
    pub files: Vec<PathBuf>,
}

fn e(e: duckdb::Error) -> String {
    e.to_string()
}

fn table_exists(con: &Connection, schema: &str, table: &str) -> Result<bool> {
    con.query_row(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema = ? AND table_name = ?",
        [schema, table],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(e)
}

fn strings(con: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut st = con.prepare(sql).map_err(e)?;
    let v = st.query_map([], |r| r.get(0)).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e);
    v
}

/// A SELECT of the scalar columns and the geometry of `schema.table`, as `_select_sql`. For a shapefile: the long names
/// get a short alias (10 characters in a DBF) and a BIGINT is text (a DBF number cannot hold an int64 exactly).
fn select_sql(con: &Connection, schema: &str, table: &str, shp: bool) -> Result<String> {
    let mut st = con
        .prepare(
            "SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = ? AND table_name = ? \
             ORDER BY ordinal_position",
        )
        .map_err(e)?;
    let cols: Vec<(String, String)> = st
        .query_map([schema, table], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(e)?
        .collect::<std::result::Result<_, _>>()
        .map_err(e)?;
    let mut parts = Vec::new();
    for (name, dtype) in cols {
        let d = dtype.to_uppercase();
        if d.ends_with("[]") || d.contains("STRUCT") || d.contains("MAP") || name.to_uppercase() == "OGC_FID" {
            continue;
        }
        let alias = match (shp, name.as_str()) {
            (true, "maxspeed_kmh") => "spd_kmh",
            (true, "is_reverse") => "is_rev",
            (true, "admin_level") => "adm_level",
            _ => name.as_str(),
        };
        let mut expr = format!("\"{name}\"");
        if shp && (d.contains("BIGINT") || d.contains("HUGEINT")) {
            expr = format!("CAST(\"{name}\" AS VARCHAR)");
        }
        if (name == "length_m" || name == "cost_s") && (d.contains("DOUBLE") || d.contains("FLOAT")) {
            expr = format!("ROUND(CAST({expr} AS DOUBLE), 2)");
        }
        parts.push(format!("{expr} AS \"{alias}\""));
    }
    Ok(format!("SELECT {} FROM \"{schema}\".\"{table}\"", parts.join(", ")))
}

fn count(con: &Connection, schema: &str, table: &str) -> Result<i64> {
    con.query_row(&format!("SELECT count(*) FROM \"{schema}\".\"{table}\""), [], |r| r.get(0)).map_err(e)
}

fn copy_gdal(con: &Connection, sql: &str, out: &Path, driver: &str, srs: &str, layer: Option<&str>) -> Result<()> {
    let layer = layer.map_or(String::new(), |l| format!(", LAYER_NAME '{l}'"));
    con.execute_batch(&format!("COPY ({sql}) TO '{}' (FORMAT gdal, DRIVER '{driver}', SRS '{srs}'{layer})", out.display()))
        .map_err(|x| format!("{}: {x}", out.display()))
}

fn mkdir(d: &Path) -> Result<()> {
    std::fs::create_dir_all(if d.as_os_str().is_empty() { Path::new(".") } else { d }).map_err(|x| format!("{}: {x}", d.display()))
}

/// Export the layers of the database `db`, as `to_gis`. `modes`: empty = each mode of the database. `fmt`: gpkg, shp or
/// geoparquet. `out`: the .gpkg file, else a folder. `name`: the base name of the layer files (default: the file name of `out`).
pub fn to_gis(db: &Path, out: &Path, modes: &[String], fmt: &str, boundary: bool, name: Option<&str>, srs: &str) -> Result<Summary> {
    let con = open(db)?;
    let fmt = fmt.to_lowercase();
    if !["gpkg", "shp", "geoparquet"].contains(&fmt.as_str()) {
        return Err(format!("fmt must be 'gpkg', 'shp' or 'geoparquet', got '{fmt}'"));
    }
    let present = strings(
        &con,
        "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name = 'edges' \
         AND schema_name NOT IN ('information_schema', 'pg_catalog', 'main', 'raw') ORDER BY schema_name",
    )?;
    let chosen: Vec<String> = if modes.is_empty() { present.clone() } else { modes.to_vec() };
    if chosen.is_empty() {
        return Err("no mode schemas with an 'edges' table in the db".into());
    }
    let unknown: Vec<&String> = chosen.iter().filter(|m| !present.contains(m)).collect();
    if !unknown.is_empty() {
        return Err(format!("mode(s) {unknown:?} not found (present: {present:?})"));
    }
    let mut layers: Vec<(String, String, String)> = Vec::new();
    for m in &chosen {
        layers.push((format!("edges_{m}"), m.clone(), "edges".into()));
        if table_exists(&con, m, "nodes")? {
            layers.push((format!("nodes_{m}"), m.clone(), "nodes".into()));
        }
    }
    if boundary && table_exists(&con, "main", "boundary")? {
        layers.push(("boundary".into(), "main".into(), "boundary".into()));
    }
    let stem = || out.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).map(String::from);
    let base = name.map(String::from).or_else(|| if fmt == "gpkg" { stem() } else { None }).unwrap_or_else(|| "network".into());
    match fmt.as_str() {
        "geoparquet" => geoparquet(&con, out, layers),
        "shp" => {
            mkdir(out)?;
            let mut s = Summary { fmt, path: Some(out.to_path_buf()), layers: vec![], files: vec![] };
            for (lyr, schema, table) in layers {
                let path = out.join(format!("{base}_{lyr}.shp"));
                copy_gdal(&con, &select_sql(&con, &schema, &table, true)?, &path, "ESRI Shapefile", srs, None)?;
                s.layers.push((lyr, count(&con, &schema, &table)?));
                s.files.push(path);
            }
            Ok(s)
        }
        _ => gpkg(&con, out, layers, srs),
    }
}

fn geoparquet(con: &Connection, dir: &Path, mut layers: Vec<(String, String, String)>) -> Result<Summary> {
    mkdir(dir)?;
    for t in strings(con, "SELECT table_name FROM information_schema.tables WHERE table_schema = 'features' ORDER BY 1")? {
        layers.push((format!("features_{t}"), "features".into(), t));
    }
    let levels = table_exists(con, "visualization", "edge_levels")?;
    let mut s = Summary { fmt: "geoparquet".into(), path: Some(dir.to_path_buf()), layers: vec![], files: vec![] };
    let mut index = Vec::new();
    for (lyr, schema, table) in &layers {
        let path = dir.join(format!("{lyr}.parquet"));
        let mut sql = select_sql(con, schema, table, false)?;
        if table == "edges" && levels {
            sql = format!(
                "SELECT s.*, v.* EXCLUDE (edge_id) FROM ({sql}) s LEFT JOIN \"visualization\".\"edge_levels\" v ON s.edge_id = v.edge_id"
            );
        }
        con.execute_batch(&format!("COPY ({sql}) TO '{}' (FORMAT parquet)", path.display())).map_err(e)?;
        let n = count(con, schema, table)?;
        // the entry of layers.json, as json.dump(indent=1) of the Python export writes it
        let feat = schema == "features";
        let kind = if feat { "features" } else { table.as_str() };
        let mut entry = format!("  {{\n   \"file\": \"{lyr}.parquet\",\n   \"kind\": \"{kind}\",\n");
        if !feat && (table == "edges" || table == "nodes") {
            entry.push_str(&format!("   \"mode\": \"{schema}\",\n"));
        }
        if feat {
            entry.push_str(&format!("   \"table\": \"{table}\",\n"));
        }
        entry.push_str(&format!("   \"count\": {n}\n  }}"));
        index.push(entry);
        s.layers.push((lyr.clone(), n));
        s.files.push(path);
    }
    let json = if index.is_empty() { "{\n \"layers\": []\n}".to_string() } else { format!("{{\n \"layers\": [\n{}\n ]\n}}", index.join(",\n")) };
    std::fs::write(dir.join("layers.json"), json).map_err(|x| x.to_string())?;
    Ok(s)
}

fn gpkg(con: &Connection, out: &Path, layers: Vec<(String, String, String)>, srs: &str) -> Result<Summary> {
    let dir = out.parent().unwrap_or(Path::new(""));
    mkdir(dir)?;
    let mut s = Summary { fmt: "gpkg".into(), path: None, layers: vec![], files: vec![] };
    if out.exists() {
        std::fs::remove_file(out).map_err(|x| format!("{}: {x}", out.display()))?;
    }
    let tmp = std::env::temp_dir().join(format!("duckosm-gis-{}", std::process::id()));
    mkdir(&tmp)?;
    let r = (|| {
        let mut parts = Vec::new();
        for (lyr, schema, table) in &layers {
            let part = tmp.join(format!("{lyr}.gpkg"));
            copy_gdal(con, &format!("{} LIMIT 0", select_sql(con, schema, table, false)?), &part, "GPKG", srs, Some(lyr))?;
            parts.push(part);
        }
        join_gpkg(&parts, out)?;
        let db = rusqlite::Connection::open(out).map_err(|x| format!("{}: {x}", out.display()))?;
        for (lyr, schema, table) in &layers {
            let n = fill_gpkg(con, &db, lyr, &select_sql(con, schema, table, false)?).map_err(|x| format!("{}: {x}", out.display()))?;
            s.layers.push((lyr.clone(), n));
        }
        Ok::<(), String>(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r?;
    s.path = Some(out.to_path_buf());
    s.files.push(out.to_path_buf());
    Ok(s)
}

/// Join GeoPackage files of one layer each into the file `out`: the first file becomes `out`, then SQLite copies the
/// tables, the R-tree, the `gpkg_*` rows, the indexes and the triggers of each other file into it, as GDAL wrote them.
fn join_gpkg(parts: &[PathBuf], out: &Path) -> Result<()> {
    let Some((first, rest)) = parts.split_first() else { return Ok(()) };
    std::fs::copy(first, out).map_err(|x| format!("{}: {x}", out.display()))?;
    let db = rusqlite::Connection::open(out).map_err(|x| format!("{}: {x}", out.display()))?;
    let x = |x: rusqlite::Error| format!("{}: {x}", out.display());
    for part in rest {
        db.execute("ATTACH ?1 AS p", [part.to_string_lossy()]).map_err(x)?;
        let objects = |kind: &str| -> Result<Vec<(String, String)>> {
            let mut q = db
                .prepare("SELECT name, sql FROM p.sqlite_master WHERE type = ?1 AND sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY rootpage")
                .map_err(x)?;
            let rows = q.query_map([kind], |r| Ok((r.get(0)?, r.get(1)?))).map_err(x)?;
            rows.collect::<rusqlite::Result<_>>().map_err(x)
        };
        let tables = objects("table")?;
        // an R-tree makes its own tables (`<name>_node`, `_parent`, `_rowid`)
        let virt: Vec<&String> = tables.iter().filter(|t| t.1.starts_with("CREATE VIRTUAL")).map(|t| &t.0).collect();
        let shadow = |n: &str| virt.iter().any(|v| ["_node", "_parent", "_rowid"].iter().any(|s| n == format!("{v}{s}")));
        db.execute_batch("BEGIN").map_err(x)?;
        // the `gpkg_*` tables, and their indexes and triggers, are in `out` already: only their rows come
        let have = |name: &str| -> Result<bool> {
            db.query_row("SELECT count(*) FROM main.sqlite_master WHERE name = ?1", [name], |r| r.get::<_, i64>(0)).map(|n| n > 0).map_err(x)
        };
        for (name, sql) in tables.iter().filter(|t| !shadow(&t.0)) {
            if !have(name)? {
                db.execute_batch(sql).map_err(x)?;
            }
            let q = name.replace('"', "\"\"");
            db.execute_batch(&format!("INSERT OR IGNORE INTO main.\"{q}\" SELECT * FROM p.\"{q}\"")).map_err(x)?;
        }
        // the indexes and the triggers after the rows, as GDAL writes them
        for (name, sql) in objects("index")?.into_iter().chain(objects("trigger")?) {
            if !have(&name)? {
                db.execute_batch(&sql).map_err(x)?;
            }
        }
        db.execute_batch("COMMIT; DETACH p").map_err(x)?;
    }
    Ok(())
}

/// Write the rows of `sql` into the empty layer `lyr` of the GeoPackage `db`, as GDAL writes them: the geometry as a
/// GeoPackage blob (the header, the envelope for a geometry that is not a point, then the WKB), the R-tree, the extent
/// in `gpkg_contents` and the count in `gpkg_ogr_contents`. Returns the number of rows.
fn fill_gpkg(con: &Connection, db: &rusqlite::Connection, lyr: &str, sql: &str) -> Result<i64> {
    let x = |x: rusqlite::Error| x.to_string();
    let (gcol, srs_id): (String, i32) = db
        .query_row("SELECT column_name, srs_id FROM gpkg_geometry_columns WHERE table_name = ?1", [lyr], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(x)?;
    let mut st = con.prepare(&format!("DESCRIBE {sql}")).map_err(e)?;
    let cols: Vec<(String, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
    let geoms: Vec<&String> = cols.iter().filter(|c| c.1.starts_with("GEOMETRY")).map(|c| &c.0).collect();
    let [g] = geoms[..] else { return Err(format!("{lyr}: {} geometry columns, the export takes one", geoms.len())) };
    let others: Vec<&(String, String)> = cols.iter().filter(|c| &c.0 != g).collect();
    let q = |n: &str| format!("\"{}\"", n.replace('"', "\"\""));
    // DuckDB gives these types as SQLite values; another type goes as text, as GDAL reads it
    let plain = ["BOOLEAN", "TINYINT", "SMALLINT", "INTEGER", "BIGINT", "UTINYINT", "USMALLINT", "UINTEGER", "FLOAT", "DOUBLE", "VARCHAR"];
    let mut select: Vec<String> =
        others.iter().map(|(n, t)| if plain.contains(&t.as_str()) { q(n) } else { format!("CAST({} AS VARCHAR)", q(n)) }).collect();
    let gq = q(g);
    select.push(format!(
        "ST_AsWKB({gq})::BLOB, ST_XMin({gq}), ST_XMax({gq}), ST_YMin({gq}), ST_YMax({gq}), \
         ST_GeometryType({gq})::VARCHAR = 'POINT', ST_IsEmpty({gq})"
    ));
    let k = others.len();
    let mut rows_st = con.prepare(&format!("SELECT {} FROM ({sql}) t", select.join(", "))).map_err(e)?;
    let mut rows = rows_st.query([]).map_err(e)?;

    // the R-tree trigger calls ST_IsEmpty and ST_MinX, which only GDAL gives to SQLite: this function fills the R-tree
    let rtree = format!("rtree_{lyr}_{gcol}");
    let trigger = format!("{rtree}_insert");
    let tsql: String = db.query_row("SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1", [&trigger], |r| r.get(0)).map_err(x)?;
    db.execute_batch(&format!("BEGIN; DROP TRIGGER {}", q(&trigger))).map_err(x)?;
    let names: Vec<String> = std::iter::once(q(&gcol)).chain(others.iter().map(|c| q(&c.0))).collect();
    let marks = vec!["?"; names.len()].join(", ");
    let mut ins = db.prepare(&format!("INSERT INTO {} ({}) VALUES ({marks})", q(lyr), names.join(", "))).map_err(x)?;
    let mut idx = db.prepare(&format!("INSERT INTO {} VALUES (?1, ?2, ?3, ?4, ?5)", q(&rtree))).map_err(x)?;
    let (mut n, mut ext) = (0i64, [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]);
    use duckdb::types::ValueRef as D;
    use rusqlite::types::Value as S;
    while let Some(r) = rows.next().map_err(e)? {
        let mut vals: Vec<S> = Vec::with_capacity(k + 1);
        let mut env = None;
        let geom = match r.get_ref(k).map_err(e)? {
            D::Blob(wkb) => {
                let (empty, point): (bool, bool) = (r.get(k + 6).map_err(e)?, r.get(k + 5).map_err(e)?);
                let mut b = vec![b'G', b'P', 0, if empty { 0x11 } else if point { 0x01 } else { 0x03 }];
                b.extend_from_slice(&srs_id.to_le_bytes());
                if !empty {
                    // minx, maxx, miny, maxy: the order of the GeoPackage envelope and of the R-tree
                    let e4: [f64; 4] = [r.get(k + 1).map_err(e)?, r.get(k + 2).map_err(e)?, r.get(k + 3).map_err(e)?, r.get(k + 4).map_err(e)?];
                    if !point {
                        e4.iter().for_each(|v| b.extend_from_slice(&v.to_le_bytes()));
                    }
                    ext = [ext[0].min(e4[0]), ext[1].min(e4[2]), ext[2].max(e4[1]), ext[3].max(e4[3])];
                    env = Some(e4);
                }
                b.extend_from_slice(wkb);
                S::Blob(b)
            }
            _ => S::Null,
        };
        vals.push(geom);
        for i in 0..k {
            vals.push(match r.get_ref(i).map_err(e)? {
                D::Null => S::Null,
                D::Boolean(v) => S::Integer(v as i64),
                D::TinyInt(v) => S::Integer(v.into()),
                D::SmallInt(v) => S::Integer(v.into()),
                D::Int(v) => S::Integer(v.into()),
                D::BigInt(v) => S::Integer(v),
                D::UTinyInt(v) => S::Integer(v.into()),
                D::USmallInt(v) => S::Integer(v.into()),
                D::UInt(v) => S::Integer(v.into()),
                D::Float(v) => S::Real(v.into()),
                D::Double(v) => S::Real(v),
                D::Text(v) => S::Text(String::from_utf8_lossy(v).into_owned()),
                v => return Err(format!("{lyr}.{}: the type {:?} is not exported", others[i].0, v.data_type())),
            });
        }
        ins.execute(rusqlite::params_from_iter(vals)).map_err(x)?;
        n += 1;
        if let Some([a, b, c, d]) = env {
            idx.execute(rusqlite::params![db.last_insert_rowid(), a, b, c, d]).map_err(x)?;
        }
    }
    drop((ins, idx));
    db.execute_batch(&tsql).map_err(x)?;
    let ext: Vec<Option<f64>> = ext.iter().map(|v| v.is_finite().then_some(*v)).collect();
    db.execute(
        "UPDATE gpkg_contents SET min_x = ?1, min_y = ?2, max_x = ?3, max_y = ?4 WHERE table_name = ?5",
        rusqlite::params![ext[0], ext[1], ext[2], ext[3], lyr],
    )
    .map_err(x)?;
    db.execute("UPDATE gpkg_ogr_contents SET feature_count = ?1 WHERE table_name = ?2", rusqlite::params![n, lyr]).map_err(x)?;
    db.execute_batch("COMMIT").map_err(x)?;
    Ok(n)
}
