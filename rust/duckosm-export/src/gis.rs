//! The GIS export (`export-gis`): GeoPackage, GeoParquet or ESRI shapefile, the port of src/duckosm/gis.py.
//! docs/design/gis_export.md.
//!
//! The layers are `edges_<mode>`, `nodes_<mode>` and `boundary` (GeoParquet: also `features_<table>`). DuckDB's spatial
//! extension writes each layer (`COPY ... (FORMAT gdal)` or `(FORMAT parquet)`). A GeoPackage has one layer for each
//! file, so `ogr2ogr` puts the layers into one file; without `ogr2ogr`, each layer stays in its own file.

use crate::{open, Result};
use duckdb::Connection;
use std::path::{Path, PathBuf};

pub struct Summary {
    pub fmt: String,
    /// The GeoPackage file (None: one file for each layer), or the folder of the shapefiles or the GeoParquet files.
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

fn copy_gdal(con: &Connection, sql: &str, out: &Path, driver: &str, srs: &str) -> Result<()> {
    con.execute_batch(&format!("COPY ({sql}) TO '{}' (FORMAT gdal, DRIVER '{driver}', SRS '{srs}')", out.display()))
        .map_err(|x| format!("{}: {x}", out.display()))
}

fn mkdir(d: &Path) -> Result<()> {
    std::fs::create_dir_all(if d.as_os_str().is_empty() { Path::new(".") } else { d }).map_err(|x| format!("{}: {x}", d.display()))
}

/// The `ogr2ogr` program on the PATH.
fn find_ogr2ogr() -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|d| Path::new(d).join("ogr2ogr")).find(|p| p.is_file())
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
                copy_gdal(&con, &select_sql(&con, &schema, &table, true)?, &path, "ESRI Shapefile", srs)?;
                s.layers.push((lyr, count(&con, &schema, &table)?));
                s.files.push(path);
            }
            Ok(s)
        }
        _ => gpkg(&con, out, &base, layers, srs),
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

fn gpkg(con: &Connection, out: &Path, base: &str, layers: Vec<(String, String, String)>, srs: &str) -> Result<Summary> {
    let dir = out.parent().unwrap_or(Path::new(""));
    mkdir(dir)?;
    let mut s = Summary { fmt: "gpkg".into(), path: None, layers: vec![], files: vec![] };
    let Some(ogr) = find_ogr2ogr() else {
        eprintln!("ogr2ogr not found — writing one GeoPackage per layer (install GDAL for a single multi-layer .gpkg)");
        for (lyr, schema, table) in layers {
            let path = dir.join(format!("{base}_{lyr}.gpkg"));
            copy_gdal(con, &select_sql(con, &schema, &table, false)?, &path, "GPKG", srs)?;
            s.layers.push((lyr, count(con, &schema, &table)?));
            s.files.push(path);
        }
        return Ok(s);
    };
    if out.exists() {
        std::fs::remove_file(out).map_err(|x| format!("{}: {x}", out.display()))?;
    }
    let tmp = std::env::temp_dir().join(format!("duckosm-gis-{}", std::process::id()));
    mkdir(&tmp)?;
    let r = (|| {
        for (lyr, schema, table) in &layers {
            let part = tmp.join(format!("{lyr}.gpkg"));
            copy_gdal(con, &select_sql(con, schema, table, false)?, &part, "GPKG", srs)?;
            let mut cmd = std::process::Command::new(&ogr);
            cmd.args(["-f", "GPKG"]);
            if out.exists() {
                cmd.args(["-update", "-append"]);
            }
            let res = cmd.arg(out).arg(&part).args(["-nln", lyr]).output().map_err(|x| format!("ogr2ogr: {x}"))?;
            if !res.status.success() {
                let err = String::from_utf8_lossy(&res.stderr);
                return Err(format!("ogr2ogr failed on layer {lyr} ({}):\n{}", res.status, &err[err.len().saturating_sub(2000)..]));
            }
            s.layers.push((lyr.clone(), count(con, schema, table)?));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r?;
    s.path = Some(out.to_path_buf());
    s.files.push(out.to_path_buf());
    Ok(s)
}
