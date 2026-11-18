//! The exports of a built duckOSM database (docs/design/all_in_rust.md, group D). Each module is the port of one
//! Python export in src/duckosm/ and writes the same file. `cli` has the commands of `duckosm`.

pub mod cli;
pub mod gis;
pub mod gis_debug;
pub mod geos;
pub mod lanelet2;
pub mod gmns;
pub mod gmns_lanes;
pub mod matsim;
pub mod matsim_lanes;
pub mod offset;
pub mod opendrive;
pub mod railml;
pub mod shapely;
pub mod sumo;

use duckdb::{AccessMode, Config, Connection};
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

/// Open a duckOSM database read-only, with DuckDB's spatial extension (the first `INSTALL` downloads it).
pub fn open(db: &Path) -> Result<Connection> {
    let cfg = Config::default().access_mode(AccessMode::ReadOnly).map_err(|e| e.to_string())?;
    let con = Connection::open_with_flags(db, cfg).map_err(|e| format!("{}: {e}", db.display()))?;
    con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;")
        .map_err(|e| format!("the spatial extension of DuckDB: {e}"))?;
    Ok(con)
}

/// True when the database has the table `schema.table`.
pub fn has_table(con: &Connection, schema: &str, table: &str) -> Result<bool> {
    con.query_row(
        "SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? AND table_name = ?",
        [schema, table],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| e.to_string())
}

/// The EPSG code of the WGS84 UTM zone that holds (lon, lat), as duckosm.utils.utm_epsg.
pub fn utm_epsg(lon: f64, lat: f64) -> String {
    let zone = (((lon + 180.0) / 6.0).floor() as i64).rem_euclid(60) + 1;
    format!("EPSG:{}", if lat >= 0.0 { 32600 } else { 32700 } + zone)
}

/// The UTM zone of the centre of `table`'s geometries, as duckosm.utils.data_utm_crs.
pub fn data_utm_crs(con: &Connection, table: &str, geom: &str) -> Result<String> {
    let (lon, lat): (f64, f64) = con
        .query_row(
            &format!("SELECT avg(ST_X(ST_Centroid({geom}))), avg(ST_Y(ST_Centroid({geom}))) FROM {table} WHERE {geom} IS NOT NULL"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    Ok(utm_epsg(lon, lat))
}

/// XML text escape, as Python's xml.sax.saxutils.escape: `&`, `<`, `>`.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// An XML attribute value with its quotes, as Python's xml.sax.saxutils.quoteattr.
pub fn quoteattr(s: &str) -> String {
    let d = escape(s).replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;");
    if d.contains('"') {
        if d.contains('\'') {
            format!("\"{}\"", d.replace('"', "&quot;"))
        } else {
            format!("'{d}'")
        }
    } else {
        format!("\"{d}\"")
    }
}

/// Write `text` to `path` (and make its folder), gzip-compressed when `gz`.
pub fn write_text(path: &Path, text: &str, gz: bool) -> Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    if gz {
        let f = std::fs::File::create(path).map_err(err)?;
        let mut w = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        w.write_all(text.as_bytes()).map_err(err)?;
        w.finish().map_err(err)?;
        Ok(())
    } else {
        std::fs::write(path, text).map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoteattr_as_python() {
        assert_eq!(quoteattr("car,bike"), "\"car,bike\"");
        assert_eq!(quoteattr("a\"b"), "'a\"b'");
        assert_eq!(quoteattr("a\"b'c&"), "\"a&quot;b'c&amp;\"");
        assert_eq!(utm_epsg(7.42, 43.73), "EPSG:32632");
        assert_eq!(utm_epsg(-70.0, -33.0), "EPSG:32719");
    }
}
