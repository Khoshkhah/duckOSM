//! `duckosm quack --osm2pgsql` (docs/concepts/osm2pgsql.md): the four tables of osm2pgsql's `pgsql` output with
//! `default.style` and `--hstore` (`duckosm_core::osm2pgsql`), made by the quack engine in one read of the file, as one
//! GeoParquet file for each table and/or straight into a PostGIS database. The `.duckdb` file is made from the GeoParquet
//! files (`duckosm_duckdb::write_osm2pgsql`).

use duckosm_core::gdal::{self, Parts};
use duckosm_core::osm2pgsql::{self as o2p};
pub use duckosm_core::osm2pgsql::table_name;
use duckosm_core::quack::{self, Feature, Mode, TileSize};
use duckosm_parquet::osm2pgsql::Osm2pgsqlWriter;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// The rows of each table (index: the table, 1 to 4) and the files.
pub struct Osm2pgsqlCounts {
    pub rows: [usize; 5],
    pub files: Vec<PathBuf>,
}

/// Make osm2pgsql's tables of `pbf` into the GeoParquet files `<parquet>_<table>.parquet`, the DuckDB file `duckdb` (made
/// from the GeoParquet files; with no `parquet`, they are made next to it and deleted) and/or the PostGIS database
/// `postgis` (a connection string). `merc`: EPSG:3857 (osm2pgsql's default), else EPSG:4326 (`--latlong`). `tiles`:
/// tile by tile, as quack by tiles. `fixes`: the OSM fixes of `duckosm build --fixes`.
pub fn quack_osm2pgsql(
    pbf: &Path,
    parquet: Option<&Path>,
    duckdb: Option<&Path>,
    postgis: Option<&str>,
    merc: bool,
    tiles: Option<TileSize>,
    fixes: Option<&duckosm_core::fixes::OsmFixes>,
) -> Result<Osm2pgsqlCounts, String> {
    #[cfg(not(feature = "duckdb"))]
    if let Some(db) = duckdb {
        return Err(format!("{}: the .duckdb output needs the cargo feature \"duckdb\"", db.display()));
    }
    let keep = parquet.is_some();
    let db_base = duckdb.map(|d| d.with_extension(""));
    let parquet = parquet.or(db_base.as_deref());
    let open = || {
        let f = duckosm_core::fs::File::open(pbf).map_err(|e| format!("{}: {e}", pbf.display()))?;
        Ok(std::io::BufReader::with_capacity(1 << 20, f))
    };
    let fixes = fixes.map(|f| quack::QuackFixes::new(f, open)).transpose()?;
    let mut files = Vec::new();
    let outs = match parquet {
        Some(base) => {
            let mut w = Vec::new();
            for t in 1..=4u8 {
                let path = PathBuf::from(format!("{}_{}.parquet", base.display(), o2p::table_name(t)));
                w.push(BufWriter::new(std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?));
                files.push(path);
            }
            Some(w)
        }
        None => None,
    };
    let rows = write_tables(merc, outs, postgis, |send| {
        match tiles {
            None => quack::gdal_features(open, quack::BUDGET, Mode::Osm2pgsql, fixes.as_ref(), send)?,
            Some(size) => quack::gdal_features_by_tiles(open, quack::BUDGET, size, Mode::Osm2pgsql, fixes, send)?,
        };
        Ok(())
    })?;
    #[cfg(feature = "duckdb")]
    if let Some(db) = duckdb {
        let tables: Vec<(&str, &Path)> = (1..=4u8).map(table_name).zip(files.iter().map(|p| p.as_path())).collect();
        duckosm_duckdb::write_osm2pgsql(db, &tables)?;
        if !keep {
            files.iter().try_for_each(|p| std::fs::remove_file(p).map_err(|e| format!("{}: {e}", p.display())))?;
            files.clear();
        }
        files.push(db.to_path_buf());
    }
    duckosm_core::util::step("quack --osm2pgsql: the tables");
    Ok(Osm2pgsqlCounts { rows, files })
}

/// The rows of osm2pgsql's tables that one tile owns (a worker of the browser for each tile, as
/// `quack_gdal::quack_gdal_tile_to`), as rows of `gdal::put_row` in Snappy frames; [`quack_osm2pgsql_join_to`] joins the
/// files of all tiles. Gives the rows of the engine (EPSG:4326, before [`o2p::finish`]) of each layer.
pub fn quack_osm2pgsql_tile_to<R: Read + Send, W: Write>(
    open: impl Fn() -> Result<R, String>,
    tile: duckosm_core::tiles::Tile,
    out: W,
    fixes: Option<&quack::QuackFixes>,
) -> Result<[usize; 5], String> {
    let e = |x: std::io::Error| format!("quack --osm2pgsql: the file of the tile: {x}");
    let mut out = snappy::write::FrameEncoder::new(out);
    let owner = quack::Owner { tile, relations: None };
    let (mut n, mut b) = ([0usize; 5], Vec::new());
    quack::gdal_features_owned(open, quack::BUDGET, &owner, Mode::Osm2pgsql, fixes, |rows| {
        for f in rows.iter().filter(|f| f.owner) {
            n[f.layer as usize] += 1;
            b.clear();
            gdal::put_row(f, &mut b);
            out.write_all(&(b.len() as u32).to_le_bytes()).and_then(|_| out.write_all(&b)).map_err(e)?;
        }
        Ok(())
    })?;
    out.into_inner().map_err(|x| format!("quack --osm2pgsql: the file of the tile: {x}"))?.flush().map_err(e)?;
    Ok(n)
}

/// Join the files of the tiles of [`quack_osm2pgsql_tile_to`] (`parts`) into the GeoParquet files of the four tables
/// (`outs`, in the order of the tables). Gives the rows of each table.
pub fn quack_osm2pgsql_join_to<R: Read, W: Write + Send>(parts: Vec<R>, merc: bool, outs: Vec<W>) -> Result<[usize; 5], String> {
    write_tables(merc, Some(outs), None, |send| {
        let e = |x: std::io::Error| format!("quack --osm2pgsql: the file of a tile: {x}");
        for r in parts {
            // as quack_gdal_join_to: a buffer, because in the browser each read is a read of the file
            let mut r = snappy::read::FrameDecoder::new(std::io::BufReader::with_capacity(1 << 20, r));
            let (mut batch, mut len) = (Vec::new(), [0u8; 4]);
            loop {
                match r.read_exact(&mut len) {
                    Ok(()) => {}
                    Err(x) if x.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(x) => return Err(e(x)),
                }
                let mut b = vec![0u8; u32::from_le_bytes(len) as usize];
                r.read_exact(&mut b).map_err(e)?;
                batch.push(gdal::get_row(&mut &b[..]));
                if batch.len() >= 20_000 {
                    send(std::mem::take(&mut batch))?;
                }
            }
            send(batch)?;
        }
        Ok(())
    })
}

/// Write the rows of the engine that `produce` gives (to its `send`) into the GeoParquet files of the four tables `outs`
/// and/or the PostGIS database `postgis`: the parts of the relation lines wait in `Parts` until `produce` is done, and
/// [`o2p::finish`] makes the rows of the tables. Gives the rows of each table.
// ponytail: this thread makes and writes the rows; a writer thread as quack --gdal's when the time on large areas matters
pub fn write_tables<W: Write + Send>(
    merc: bool,
    outs: Option<Vec<W>>,
    postgis: Option<&str>,
    produce: impl FnOnce(&mut dyn FnMut(Vec<Feature>) -> Result<(), String>) -> Result<(), String>,
) -> Result<[usize; 5], String> {
    let mut parquet = match outs {
        Some(ws) => Some(ws.into_iter().zip(1..=4u8).map(|(w, t)| Osm2pgsqlWriter::new(t, merc, w)).collect::<Result<Vec<_>, _>>()?),
        None => None,
    };
    let mut pg = postgis.map(|_| Copy::new()).transpose()?;
    let mut rows = [0usize; 5];
    let mut write = |batch: Vec<Feature>| -> Result<(), String> {
        let mut out = Vec::new();
        batch.into_iter().for_each(|f| o2p::finish(f, merc, &mut out));
        for f in out {
            rows[f.layer as usize] += 1;
            if let Some(c) = pg.as_mut() {
                c.write(&f, merc)?;
            }
            if let Some(w) = parquet.as_mut() {
                w[f.layer as usize - 1].write(f)?;
            }
        }
        Ok(())
    };
    let mut parts = Parts::new()?;
    produce(&mut |batch: Vec<Feature>| {
        let (part, whole): (Vec<Feature>, Vec<Feature>) = batch.into_iter().partition(|f| f.part.is_some());
        part.iter().try_for_each(|f| parts.push(f))?;
        write(whole)
    })?;
    let mut batch = Vec::new();
    parts.rows(quack::BUDGET, |f| {
        batch.push(f);
        if batch.len() >= 10_000 {
            write(std::mem::take(&mut batch))?;
        }
        Ok(())
    })?;
    write(batch)?;
    if let Some(w) = parquet {
        w.into_iter().try_for_each(|w| w.close().map(|_| ()))?;
    }
    if let (Some(c), Some(conn)) = (pg, postgis) {
        c.load(conn, merc)?;
    }
    Ok(rows)
}

/// The rows of the four tables in PostgreSQL's COPY text format, in temporary files, as osm2pgsql sends them: `osm_id`,
/// the columns, `tags` as hstore text, and the geometry as hex EWKB.
struct Copy {
    files: Vec<(PathBuf, BufWriter<std::fs::File>)>,
}

/// A value in COPY text: `\` before `\`, and the tab, the new line and the carriage return as `\t`, `\n`, `\r`.
fn copy_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
}

/// The hstore text of the tags: `"key"=>"value"`, with `\` before `"` and `\`.
fn hstore(tags: &[(String, String)]) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    tags.iter().map(|(k, v)| format!("{}=>{}", q(k), q(v))).collect::<Vec<_>>().join(",")
}

/// The hex EWKB of a WKB with the SRID.
fn ewkb_hex(wkb: &[u8], srid: u32) -> String {
    let mut b = Vec::with_capacity(wkb.len() + 4);
    b.push(wkb[0]);
    let kind = u32::from_le_bytes(wkb[1..5].try_into().expect("4 bytes")) | 0x2000_0000;
    b.extend_from_slice(&kind.to_le_bytes());
    b.extend_from_slice(&srid.to_le_bytes());
    b.extend_from_slice(&wkb[5..]);
    b.iter().map(|x| format!("{x:02X}")).collect()
}

impl Copy {
    fn new() -> Result<Copy, String> {
        let dir = std::env::temp_dir();
        let mut files = Vec::new();
        for t in 1..=4u8 {
            let p = dir.join(format!("duckosm-{}-{}.copy", std::process::id(), o2p::table_name(t)));
            files.push((p.clone(), BufWriter::new(std::fs::File::create(&p).map_err(|e| format!("{}: {e}", p.display()))?)));
        }
        Ok(Copy { files })
    }

    fn write(&mut self, f: &Feature, merc: bool) -> Result<(), String> {
        let mut line = o2p::osm_id(f).to_string();
        let (vals, tags) = o2p::values(f);
        for v in vals {
            line.push('\t');
            match v {
                None => line.push_str("\\N"),
                Some(o2p::Cell::Text(s)) => copy_text(&s, &mut line),
                Some(o2p::Cell::Int(n)) => line.push_str(&n.to_string()),
                Some(o2p::Cell::Real(n)) => line.push_str(&n.to_string()),
            }
        }
        line.push('\t');
        copy_text(&hstore(&tags), &mut line);
        line.push('\t');
        line.push_str(&ewkb_hex(&f.geometry, if merc { 3857 } else { 4326 }));
        line.push('\n');
        let p = &mut self.files[f.layer as usize - 1];
        p.1.write_all(line.as_bytes()).map_err(|e| format!("{}: {e}", p.0.display()))
    }

    /// The tables as osm2pgsql makes them (`table_t::start` and `table_t::stop`): CREATE TABLE, COPY, the rows sorted by
    /// the geometry, a GiST index with fillfactor 100, ANALYZE.
    #[cfg(feature = "postgis")]
    fn load(self, conn: &str, merc: bool) -> Result<(), String> {
        let e = |x: postgres::Error| format!("PostGIS: {x}");
        let mut c = postgres::Client::connect(conn, postgres::NoTls).map_err(e)?;
        c.batch_execute("CREATE EXTENSION IF NOT EXISTS postgis; CREATE EXTENSION IF NOT EXISTS hstore;").map_err(e)?;
        let srid = if merc { 3857 } else { 4326 };
        for (i, (path, w)) in self.files.into_iter().enumerate() {
            let table = (i + 1) as u8;
            let name = o2p::table_name(table);
            drop(w.into_inner().map_err(|x| format!("{}: {x}", path.display()))?);
            let cols: Vec<String> = o2p::columns(table)
                .iter()
                .map(|(c, k)| format!("\"{c}\" {}", match k {
                    o2p::Kind::Text => "text",
                    o2p::Kind::Int => "int4",
                    o2p::Kind::Real => "real",
                }))
                .collect();
            let kind = match table {
                o2p::POINT => "POINT",
                o2p::POLYGON => "GEOMETRY",
                _ => "LINESTRING",
            };
            c.batch_execute(&format!(
                "DROP TABLE IF EXISTS {name}; DROP TABLE IF EXISTS {name}_tmp;
                 CREATE UNLOGGED TABLE {name}_tmp (osm_id int8,{},\"tags\" hstore,way geometry({kind},{srid}))",
                cols.join(",")
            ))
            .map_err(e)?;
            let mut copy = c.copy_in(&format!("COPY {name}_tmp FROM STDIN")).map_err(e)?;
            std::io::copy(&mut std::fs::File::open(&path).map_err(|x| format!("{}: {x}", path.display()))?, &mut copy)
                .map_err(|x| format!("PostGIS: COPY {name}: {x}"))?;
            copy.finish().map_err(e)?;
            let _ = std::fs::remove_file(&path);
            c.batch_execute(&format!(
                "CREATE TABLE {name} AS SELECT * FROM {name}_tmp ORDER BY way; DROP TABLE {name}_tmp;
                 CREATE INDEX ON {name} USING GIST (way) WITH (fillfactor = 100); ANALYZE {name};"
            ))
            .map_err(e)?;
        }
        Ok(())
    }

    #[cfg(not(feature = "postgis"))]
    fn load(self, _: &str, _: bool) -> Result<(), String> {
        Err("the PostGIS output needs the cargo feature \"postgis\"".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_values() {
        assert_eq!(hstore(&[("a\"b".into(), "c\\d".into())]), r#""a\"b"=>"c\\d""#);
        let mut s = String::new();
        copy_text("a\tb\\", &mut s);
        assert_eq!(s, "a\\tb\\\\");
        // POINT(1 2) as EWKB with SRID 3857
        let wkb = gdal::wkb_point((1.0, 2.0));
        assert!(ewkb_hex(&wkb, 3857).starts_with("0101000020110F0000"));
    }
}
