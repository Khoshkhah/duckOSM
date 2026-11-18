//! `duckosm-rust quack --gdal` (docs/rust/quack.md, "GDAL layers"): the five layers of GDAL's OSM driver
//! (`duckosm_core::gdal`), made by the quack engine in one read of the file, as a GeoPackage with the tables that GDAL's
//! GeoPackage driver writes (`ogr2ogr -f GPKG out.gpkg in.osm.pbf`) or as one GeoParquet file for each layer.

use duckosm_core::gdal::{self, Parts};
#[cfg(feature = "gpkg")]
use duckosm_core::gdal::{columns, values, Value};
use duckosm_core::quack::{self, Feature, TileSize};
#[cfg(feature = "gpkg")]
use duckosm_core::spill::Codec;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// The numbers of rows of the layers (index: the layer, 1 to 5) and the files.
pub struct GdalCounts {
    pub rows: [usize; 6],
    pub files: Vec<PathBuf>,
}

/// Make GDAL's layers of `pbf` into the GeoPackage `gpkg` and the GeoParquet files `<parquet>_<layer>.parquet`
/// (either or both). `tiles`: tile by tile, as quack by tiles.
pub fn quack_gdal(pbf: &Path, gpkg: Option<&Path>, parquet: Option<&Path>, tiles: Option<TileSize>) -> Result<GdalCounts, String> {
    let open = || {
        let f = duckosm_core::fs::File::open(pbf).map_err(|e| format!("{}: {e}", pbf.display()))?;
        Ok(std::io::BufReader::with_capacity(1 << 20, f))
    };
    let mut files: Vec<PathBuf> = gpkg.iter().map(|p| p.to_path_buf()).collect();
    let parquet = match parquet {
        Some(base) => {
            let mut w = Vec::new();
            for layer in 1..=5u8 {
                let path = PathBuf::from(format!("{}_{}.parquet", base.display(), gdal::layer_name(layer)));
                let f = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                w.push(BufWriter::new(f));
                files.push(path);
            }
            Some(w)
        }
        None => None,
    };
    let rows = write_layers(gpkg, parquet, |send| {
        match tiles {
            None => quack::gdal_features(open, quack::BUDGET, send)?,
            Some(size) => quack::gdal_features_by_tiles(open, quack::BUDGET, size, send)?,
        };
        Ok(())
    })?;
    duckosm_core::util::step("quack --gdal: the files");
    Ok(GdalCounts { rows, files })
}

/// The rows of GDAL's layers of one tile alone (a worker of the browser for each tile, as `quack_tile_to`), from the file
/// of the tile that `open` opens (a file of the tile cut), to `out` as rows of `gdal::put_row` (a u32 of the length before
/// each row), compressed as Snappy frames (France: about 30 GB of rows): only the rows that the tile owns. [`quack_gdal_join_to`] joins the files of all tiles. Gives the rows of
/// each layer (the parts of the relations in their layer).
pub fn quack_gdal_tile_to<R: Read + Send, W: Write>(
    open: impl Fn() -> Result<R, String>,
    tile: duckosm_core::tiles::Tile,
    out: W,
) -> Result<[usize; 6], String> {
    let mut out = snappy::write::FrameEncoder::new(out);
    let owner = quack::Owner { tile, relations: None };
    let (mut n, mut b) = ([0usize; 6], Vec::new());
    quack::gdal_features_owned(open, quack::BUDGET, &owner, |rows| {
        for f in rows.iter().filter(|f| f.owner) {
            n[f.layer as usize] += 1;
            b.clear();
            gdal::put_row(f, &mut b);
            out.write_all(&(b.len() as u32).to_le_bytes()).and_then(|_| out.write_all(&b)).map_err(|e| format!("quack --gdal: the file of the tile: {e}"))?;
        }
        Ok(())
    })?;
    out.into_inner().map_err(|e| format!("quack --gdal: the file of the tile: {e}"))?.flush().map_err(|e| format!("quack --gdal: the file of the tile: {e}"))?;
    Ok(n)
}

/// Join the files of the tiles of [`quack_gdal_tile_to`] (`parts`) into the GeoParquet files of the five layers (`outs`,
/// in the order of the layers) and/or the GeoPackage `gpkg`. Gives the rows of each layer.
pub fn quack_gdal_join_to<R: Read, W: Write + Send>(parts: Vec<R>, gpkg: Option<&Path>, outs: Option<Vec<W>>) -> Result<[usize; 6], String> {
    write_layers(gpkg, outs, |send| {
        for r in parts {
            // a buffer: in the browser, each read of a part is a read of its file (two for each row took 14 s on Monaco)
            let mut r = snappy::read::FrameDecoder::new(std::io::BufReader::with_capacity(1 << 20, r));
            let mut batch = Vec::new();
            let mut len = [0u8; 4];
            loop {
                match r.read_exact(&mut len) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => return Err(format!("quack --gdal: the file of a tile: {e}")),
                }
                let mut b = vec![0u8; u32::from_le_bytes(len) as usize];
                r.read_exact(&mut b).map_err(|e| format!("quack --gdal: the file of a tile: {e}"))?;
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

/// Write the rows that `produce` gives (to its `send`) into the GeoPackage `gpkg` and the GeoParquet files of the five
/// layers `parquet` (either or both). Natively, a writer thread writes while `produce` makes the rows; in the browser this
/// thread does both. The parts of the relations wait in `Parts` until `produce` is done. Gives the rows of each layer.
pub fn write_layers<W: Write + Send>(
    gpkg: Option<&Path>,
    parquet: Option<Vec<W>>,
    produce: impl FnOnce(&mut dyn FnMut(Vec<Feature>) -> Result<(), String>) -> Result<(), String>,
) -> Result<[usize; 6], String> {
    #[cfg(not(feature = "gpkg"))]
    if let Some(p) = gpkg {
        return Err(format!("{}: the GeoPackage output needs the cargo feature \"gpkg\"", p.display()));
    }
    let make_out = move || -> Result<Out<W>, String> {
        let parquet = match parquet {
            Some(ws) => Some(ws.into_iter().zip(1..=5u8).map(|(w, layer)| duckosm_parquet::gdal::GdalWriter::new(layer, w)).collect::<Result<_, _>>()?),
            None => None,
        };
        Ok(Out {
            // the GeoPackage is made on the writer thread (its temporary files belong to that thread)
            #[cfg(feature = "gpkg")]
            gpkg: match gpkg {
                Some(p) => Some(Gpkg::new(p)?),
                None => None,
            },
            parquet,
            rows: [0; 6],
        })
    };
    // the rows to the writer, the parts to `Parts`; then the joined parts
    let feed = |send: &mut dyn FnMut(Vec<Feature>) -> Result<(), String>| -> Result<(), String> {
        let mut parts = Parts::new()?;
        produce(&mut |rows: Vec<Feature>| {
            let (part, whole): (Vec<Feature>, Vec<Feature>) = rows.into_iter().partition(|f| f.part.is_some());
            part.iter().try_for_each(|f| parts.push(f))?;
            send(whole)
        })?;
        let mut batch = Vec::new();
        parts.rows(quack::BUDGET, |f| {
            batch.push(f);
            if batch.len() >= 10_000 {
                send(std::mem::take(&mut batch))?;
            }
            Ok(())
        })?;
        send(batch)?;
        duckosm_core::util::step("quack --gdal: the relations of the tiles");
        Ok(())
    };
    #[cfg(target_arch = "wasm32")]
    {
        let mut out = make_out()?;
        feed(&mut |rows| out.write(rows))?;
        out.close()
    }
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::scope(|scope| -> Result<[usize; 6], String> {
        // one batch waits for the writer: with 8, the waiting rows were 200 MB of the memory on Kentucky
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<Feature>>(1);
        let writer = scope.spawn(move || -> Result<[usize; 6], String> {
            let mut out = make_out()?;
            for batch in rx {
                out.write(batch)?;
            }
            out.close()
        });
        let made = feed(&mut |rows| tx.send(rows).map_err(|_| "quack --gdal: the writer stopped".to_string()));
        drop(tx);
        // the error of the writer explains a stop of the engine
        let written = writer.join().unwrap_or_else(|_| Err("quack --gdal: the writer stopped (panic)".into()));
        made.and(written)
    })
}

/// The writers of the rows (on the writer thread).
struct Out<W: Write + Send> {
    #[cfg(feature = "gpkg")]
    gpkg: Option<Gpkg>,
    parquet: Option<Vec<duckosm_parquet::gdal::GdalWriter<W>>>,
    rows: [usize; 6],
}

impl<W: Write + Send> Out<W> {
    fn write(&mut self, batch: Vec<Feature>) -> Result<(), String> {
        batch.iter().for_each(|f| self.rows[f.layer as usize] += 1);
        #[cfg(feature = "gpkg")]
        if let Some(g) = self.gpkg.as_mut() {
            g.write(&batch)?;
        }
        if let Some(w) = self.parquet.as_mut() {
            for f in batch {
                w[f.layer as usize - 1].write(f)?;
            }
        }
        Ok(())
    }

    fn close(self) -> Result<[usize; 6], String> {
        #[cfg(feature = "gpkg")]
        if let Some(g) = self.gpkg {
            g.close()?;
        }
        if let Some(w) = self.parquet {
            for w in w {
                w.close()?;
            }
        }
        Ok(self.rows)
    }
}

/// The envelope (min x, max x, min y, max y) of a WKB geometry (little endian, 2D).
#[cfg(feature = "gpkg")]
fn envelope(wkb: &[u8]) -> [f64; 4] {
    fn walk(b: &mut &[u8], e: &mut [f64; 4]) {
        let u32_ = |b: &mut &[u8]| {
            let v = u32::from_le_bytes(b[..4].try_into().expect("4 bytes"));
            *b = &b[4..];
            v
        };
        let pt = |b: &mut &[u8], e: &mut [f64; 4]| {
            let x = f64::from_le_bytes(b[..8].try_into().expect("8 bytes"));
            let y = f64::from_le_bytes(b[8..16].try_into().expect("8 bytes"));
            *b = &b[16..];
            *e = [e[0].min(x), e[1].max(x), e[2].min(y), e[3].max(y)];
        };
        *b = &b[1..];
        match u32_(b) {
            1 => pt(b, e),
            2 => (0..u32_(b)).for_each(|_| pt(b, e)),
            3 => (0..u32_(b)).for_each(|_| (0..u32_(b)).for_each(|_| pt(b, e))),
            _ => (0..u32_(b)).for_each(|_| walk(b, e)),
        }
    }
    let mut e = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
    walk(&mut &wkb[..], &mut e);
    e
}

/// The GeoPackage of GDAL's layers: the tables of GDAL's driver (`gpkg_*`, a table and an R-tree index for each layer,
/// `gpkg_ogr_contents`); the triggers of the index and of the counts come after the rows, as GDAL leaves them. The
/// boxes of the rows wait in temporary files; [`Gpkg::close`] writes each R-tree in one go ([`bulk_rtree`]): an insert
/// into SQLite's R-tree for each row took 10 s for each million rows.
#[cfg(feature = "gpkg")]
struct Gpkg {
    con: rusqlite::Connection,
    /// the envelope of each layer
    env: [[f64; 4]; 6],
    rows: [i64; 6],
    /// the INSERT of each layer
    insert: Vec<String>,
    store: Box<dyn duckosm_core::spill::Store>,
    /// the boxes of the rows of each layer: (fid, box)
    boxes: Vec<duckosm_core::spill::Writer>,
}

/// A box of the R-tree: its id and min x, max x, min y, max y as SQLite's R-tree keeps them (32-bit floats, the minimums
/// rounded down and the maximums up).
#[cfg(feature = "gpkg")]
type RBox = (i64, [f32; 4]);

#[cfg(feature = "gpkg")]
struct BoxCodec;

#[cfg(feature = "gpkg")]
impl duckosm_core::spill::Codec<RBox> for BoxCodec {
    fn put(&self, r: &RBox, o: &mut Vec<u8>) {
        o.extend_from_slice(&r.0.to_le_bytes());
        r.1.iter().for_each(|v| o.extend_from_slice(&v.to_le_bytes()));
    }
    fn get(&self, b: &mut &[u8]) -> RBox {
        let f = |i: usize| f32::from_le_bytes(b[8 + 4 * i..12 + 4 * i].try_into().expect("4 bytes"));
        let r = (i64::from_le_bytes(b[..8].try_into().expect("8 bytes")), [f(0), f(1), f(2), f(3)]);
        *b = &b[24..];
        r
    }
}

/// `rtreeValueDown` and `rtreeValueUp` of SQLite.
#[cfg(feature = "gpkg")]
fn down(d: f64) -> f32 {
    let f = d as f32;
    if (f as f64) > d {
        return (d * if d < 0.0 { 1.0 + 1.0 / 8388608.0 } else { 1.0 - 1.0 / 8388608.0 }) as f32;
    }
    f
}

#[cfg(feature = "gpkg")]
fn up(d: f64) -> f32 {
    let f = d as f32;
    if (f as f64) < d {
        return (d * if d < 0.0 { 1.0 - 1.0 / 8388608.0 } else { 1.0 + 1.0 / 8388608.0 }) as f32;
    }
    f
}

/// The distance along a Hilbert curve of 2^16 x 2^16 cells.
#[cfg(feature = "gpkg")]
fn hilbert(mut x: u32, mut y: u32) -> u64 {
    let mut d = 0u64;
    let mut s = 1u32 << 15;
    while s > 0 {
        let rx = (x & s > 0) as u32;
        let ry = (y & s > 0) as u32;
        d += s as u64 * s as u64 * ((3 * rx) ^ ry) as u64;
        if ry == 0 {
            if rx == 1 {
                x = 0xffff - x;
                y = 0xffff - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s >>= 1;
    }
    d
}

/// The bytes of a node of SQLite's R-tree in 2 dimensions: the depth (the root only), the number of cells, and each
/// cell (the id of a row or of a child node, the box), big endian, in [`NODE`] bytes.
#[cfg(feature = "gpkg")]
const NODE: usize = 1228;
#[cfg(feature = "gpkg")]
const CELLS: usize = (NODE - 4) / 24;

#[cfg(feature = "gpkg")]
fn node(depth: u16, cells: &[RBox]) -> Vec<u8> {
    let mut b = vec![0u8; NODE];
    b[..2].copy_from_slice(&depth.to_be_bytes());
    b[2..4].copy_from_slice(&(cells.len() as u16).to_be_bytes());
    for (i, (id, r)) in cells.iter().enumerate() {
        let at = 4 + 24 * i;
        b[at..at + 8].copy_from_slice(&id.to_be_bytes());
        for (k, v) in r.iter().enumerate() {
            b[at + 8 + 4 * k..at + 12 + 4 * k].copy_from_slice(&v.to_be_bytes());
        }
    }
    b
}

#[cfg(feature = "gpkg")]
fn union(cells: &[RBox]) -> [f32; 4] {
    cells.iter().fold([f32::MAX, f32::MIN, f32::MAX, f32::MIN], |a, (_, r)| [a[0].min(r[0]), a[1].max(r[1]), a[2].min(r[2]), a[3].max(r[3])])
}

/// Write the R-tree `rtree_<t>_geom` from its boxes in the order of the Hilbert curve (a packed R-tree): the shadow tables
/// of SQLite's R-tree (`_node`, `_rowid`, `_parent`), with full nodes, the root as node 1. As GDAL's bulk load of its
/// R-trees.
#[cfg(feature = "gpkg")]
fn bulk_rtree(con: &rusqlite::Connection, t: &str, n: i64, boxes: impl Iterator<Item = Result<RBox, String>>) -> Result<(), String> {
    let e = |x: rusqlite::Error| format!("the GeoPackage: {x}");
    if n == 0 {
        return Ok(());
    }
    let mut put_node = con.prepare(&format!("INSERT OR REPLACE INTO \"rtree_{t}_geom_node\" VALUES (?,?)")).map_err(e)?;
    let mut put_rowid = con.prepare(&format!("INSERT INTO \"rtree_{t}_geom_rowid\" VALUES (?,?)")).map_err(e)?;
    let mut put_parent = con.prepare(&format!("INSERT INTO \"rtree_{t}_geom_parent\" VALUES (?,?)")).map_err(e)?;
    let mut next = 2i64;
    // the leaves; with no more rows than a node holds, the root is the only leaf
    let root_leaf = n as usize <= CELLS;
    let mut level: Vec<RBox> = Vec::new();
    let mut cells: Vec<RBox> = Vec::with_capacity(CELLS);
    let mut leaf = |cells: &mut Vec<RBox>, level: &mut Vec<RBox>| -> Result<(), String> {
        let no = if root_leaf { 1 } else { next };
        next += 1;
        put_node.execute(rusqlite::params![no, node(0, cells)]).map_err(e)?;
        for (id, _) in cells.iter() {
            put_rowid.execute(rusqlite::params![id, no]).map_err(e)?;
        }
        level.push((no, union(cells)));
        cells.clear();
        Ok(())
    };
    for b in boxes {
        cells.push(b?);
        if cells.len() == CELLS {
            leaf(&mut cells, &mut level)?;
        }
    }
    if !cells.is_empty() {
        leaf(&mut cells, &mut level)?;
    }
    if root_leaf {
        return Ok(());
    }
    let mut depth = 1u16;
    while level.len() > CELLS {
        let mut up = Vec::new();
        for chunk in level.chunks(CELLS) {
            let no = next;
            next += 1;
            put_node.execute(rusqlite::params![no, node(0, chunk)]).map_err(e)?;
            for (child, _) in chunk {
                put_parent.execute(rusqlite::params![child, no]).map_err(e)?;
            }
            up.push((no, union(chunk)));
        }
        level = up;
        depth += 1;
    }
    put_node.execute(rusqlite::params![1, node(depth, &level)]).map_err(e)?;
    for (child, _) in &level {
        put_parent.execute(rusqlite::params![child, 1]).map_err(e)?;
    }
    Ok(())
}

/// A row ready for SQLite: the layer, the GeoPackage geometry, the values of the columns and the box.
#[cfg(feature = "gpkg")]
struct Encoded {
    layer: u8,
    params: Vec<rusqlite::types::Value>,
    env: [f64; 4],
}

#[cfg(feature = "gpkg")]
fn encode(f: &Feature) -> Encoded {
    let env = envelope(&f.geometry);
    let point = f.layer == gdal::POINTS;
    // the GeoPackage header: GDAL writes no envelope for a point
    let mut blob = Vec::with_capacity(8 + 32 + f.geometry.len());
    blob.extend_from_slice(b"GP");
    blob.push(0);
    blob.push(if point { 1 } else { 3 });
    blob.extend_from_slice(&4326i32.to_le_bytes());
    if !point {
        env.iter().for_each(|v| blob.extend_from_slice(&v.to_le_bytes()));
    }
    blob.extend_from_slice(&f.geometry);
    let mut params = vec![rusqlite::types::Value::Blob(blob)];
    params.extend(values(f).into_iter().map(|v| match v {
        Some(Value::Text(s)) => rusqlite::types::Value::Text(s),
        Some(Value::Int(n)) => rusqlite::types::Value::Integer(n),
        None => rusqlite::types::Value::Null,
    }));
    Encoded { layer: f.layer, params, env }
}

#[cfg(feature = "gpkg")]
const WGS84: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AXIS["Latitude",NORTH],AXIS["Longitude",EAST],AUTHORITY["EPSG","4326"]]"#;

#[cfg(feature = "gpkg")]
fn geometry_name(layer: u8) -> &'static str {
    ["", "POINT", "LINESTRING", "MULTILINESTRING", "MULTIPOLYGON", "GEOMETRYCOLLECTION"][layer as usize]
}

#[cfg(feature = "gpkg")]
impl Gpkg {
    fn new(path: &Path) -> Result<Gpkg, String> {
        let e = |x: rusqlite::Error| format!("{}: {x}", path.display());
        if path.exists() {
            std::fs::remove_file(path).map_err(|x| format!("{}: {x}", path.display()))?;
        }
        let con = rusqlite::Connection::open(path).map_err(e)?;
        let mut sql = String::from(
            "PRAGMA application_id = 1196444487; PRAGMA user_version = 10400; PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
            CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL,srs_id INTEGER NOT NULL PRIMARY KEY,organization TEXT NOT NULL,organization_coordsys_id INTEGER NOT NULL,definition  TEXT NOT NULL,description TEXT);
            CREATE TABLE gpkg_contents (table_name TEXT NOT NULL PRIMARY KEY,data_type TEXT NOT NULL,identifier TEXT UNIQUE,description TEXT DEFAULT '',last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),min_x DOUBLE, min_y DOUBLE,max_x DOUBLE, max_y DOUBLE,srs_id INTEGER,CONSTRAINT fk_gc_r_srs_id FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id));
            CREATE TABLE gpkg_ogr_contents(table_name TEXT NOT NULL PRIMARY KEY,feature_count INTEGER DEFAULT NULL);
            CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL,column_name TEXT NOT NULL,geometry_type_name TEXT NOT NULL,srs_id INTEGER NOT NULL,z TINYINT NOT NULL,m TINYINT NOT NULL,CONSTRAINT pk_geom_cols PRIMARY KEY (table_name, column_name),CONSTRAINT uk_gc_table_name UNIQUE (table_name),CONSTRAINT fk_gc_tn FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name),CONSTRAINT fk_gc_srs FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys (srs_id));
            CREATE TABLE gpkg_extensions (table_name TEXT,column_name TEXT,extension_name TEXT NOT NULL,definition TEXT NOT NULL,scope TEXT NOT NULL,CONSTRAINT ge_tce UNIQUE (table_name, column_name, extension_name));
            INSERT INTO gpkg_spatial_ref_sys VALUES ('Undefined Cartesian SRS', -1, 'NONE', -1, 'undefined', 'undefined Cartesian coordinate reference system');
            INSERT INTO gpkg_spatial_ref_sys VALUES ('Undefined geographic SRS', 0, 'NONE', 0, 'undefined', 'undefined geographic coordinate reference system');
            ",
        );
        sql += &format!(
            "INSERT INTO gpkg_spatial_ref_sys VALUES ('WGS 84 geodetic', 4326, 'EPSG', 4326, '{WGS84}', 'longitude/latitude coordinates in decimal degrees on the WGS 84 spheroid');\n"
        );
        for layer in 1..=5u8 {
            let t = gdal::layer_name(layer);
            let cols: Vec<String> =
                columns(layer).iter().map(|(c, int)| format!("\"{c}\" {}", if *int { "MEDIUMINT" } else { "TEXT" })).collect();
            sql += &format!(
                "CREATE TABLE \"{t}\" ( \"fid\" INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, \"geom\" {}, {});
                CREATE VIRTUAL TABLE \"rtree_{t}_geom\" USING rtree(id, minx, maxx, miny, maxy);\n",
                geometry_name(layer),
                cols.join(", ")
            );
        }
        sql += "BEGIN;";
        con.execute_batch(&sql).map_err(e)?;
        let insert = (0..=5u8)
            .map(|layer| {
                if layer == 0 {
                    return String::new();
                }
                let cols = columns(layer);
                let names: Vec<String> = cols.iter().map(|c| format!("\"{}\"", c.0)).collect();
                let marks = vec!["?"; cols.len() + 1].join(",");
                format!("INSERT INTO \"{}\" (\"geom\",{}) VALUES ({marks})", gdal::layer_name(layer), names.join(","))
            })
            .collect();
        let store = duckosm_core::spill::temp_store();
        let io = |x: std::io::Error| format!("quack --gdal: a temporary file: {x}");
        let boxes = (0..6).map(|_| duckosm_core::spill::Writer::new(&*store).map_err(io)).collect::<Result<_, _>>()?;
        Ok(Gpkg { con, env: [[f64::MAX, f64::MIN, f64::MAX, f64::MIN]; 6], rows: [0; 6], insert, store, boxes })
    }

    /// Write a batch of rows: encoded on all threads, inserted on this thread.
    fn write(&mut self, batch: &[Feature]) -> Result<(), String> {
        use rayon::prelude::*;
        let e = |x: rusqlite::Error| format!("the GeoPackage: {x}");
        let io = |x: std::io::Error| format!("quack --gdal: a temporary file: {x}");
        let rows: Vec<Encoded> = batch.par_iter().map(encode).collect();
        for r in rows {
            let layer = r.layer as usize;
            self.con.prepare_cached(&self.insert[layer]).map_err(e)?.execute(rusqlite::params_from_iter(r.params)).map_err(e)?;
            let fid = self.con.last_insert_rowid();
            let env = r.env;
            self.boxes[layer].push(&BoxCodec, &(fid, [down(env[0]), up(env[1]), down(env[2]), up(env[3])])).map_err(io)?;
            let l = &mut self.env[layer];
            *l = [l[0].min(env[0]), l[1].max(env[1]), l[2].min(env[2]), l[3].max(env[3])];
            self.rows[layer] += 1;
        }
        Ok(())
    }

    fn close(self) -> Result<(), String> {
        let e = |x: rusqlite::Error| format!("the GeoPackage: {x}");
        let io = |x: std::io::Error| format!("quack --gdal: a temporary file: {x}");
        for (layer, boxes) in self.boxes.into_iter().enumerate().skip(1) {
            // the boxes in the order of the Hilbert curve of their centers in the envelope of the layer
            let env = self.env[layer];
            let (w, h) = ((env[1] - env[0]).max(1e-12), (env[3] - env[2]).max(1e-12));
            let key = |b: &[u8]| {
                let (_, r) = BoxCodec.get(&mut &b[..]);
                let cx = ((r[0] as f64 + r[1] as f64) / 2.0 - env[0]) / w;
                let cy = ((r[2] as f64 + r[3] as f64) / 2.0 - env[2]) / h;
                hilbert((cx.clamp(0.0, 1.0) * 65535.0) as u32, (cy.clamp(0.0, 1.0) * 65535.0) as u32)
            };
            let sorted = duckosm_core::spill::sort_bytes(boxes.finish().map_err(io)?, key, quack::BUDGET, &*self.store).map_err(io)?;
            let rows = sorted.read(&BoxCodec).map_err(io)?.map(|r| r.map_err(io));
            bulk_rtree(&self.con, gdal::layer_name(layer as u8), self.rows[layer], rows)?;
        }
        duckosm_core::util::step("quack --gdal: the R-trees");
        let mut sql = String::new();
        for layer in 1..=5u8 {
            let t = gdal::layer_name(layer);
            let env = self.env[layer as usize];
            let ext = if self.rows[layer as usize] > 0 { format!("{}, {}, {}, {}", env[0], env[2], env[1], env[3]) } else { "NULL, NULL, NULL, NULL".into() };
            sql += &format!(
                "INSERT INTO gpkg_contents (table_name, data_type, identifier, description, min_x, min_y, max_x, max_y, srs_id) VALUES ('{t}', 'features', '{t}', '', {ext}, 4326);
                INSERT INTO gpkg_geometry_columns VALUES ('{t}', 'geom', '{}', 4326, 0, 0);
                INSERT INTO gpkg_extensions VALUES ('{t}', 'geom', 'gpkg_rtree_index', 'http://www.geopackage.org/spec120/#extension_rtree', 'write-only');
                INSERT INTO gpkg_ogr_contents VALUES ('{t}', {});
                CREATE TRIGGER \"rtree_{t}_geom_insert\" AFTER INSERT ON \"{t}\" WHEN (new.\"geom\" NOT NULL AND NOT ST_IsEmpty(NEW.\"geom\")) BEGIN INSERT OR REPLACE INTO \"rtree_{t}_geom\" VALUES (NEW.\"fid\",ST_MinX(NEW.\"geom\"), ST_MaxX(NEW.\"geom\"),ST_MinY(NEW.\"geom\"), ST_MaxY(NEW.\"geom\")); END;
                CREATE TRIGGER \"rtree_{t}_geom_update6\" AFTER UPDATE OF \"geom\" ON \"{t}\" WHEN OLD.\"fid\" = NEW.\"fid\" AND (NEW.\"geom\" NOTNULL AND NOT ST_IsEmpty(NEW.\"geom\")) AND (OLD.\"geom\" NOTNULL AND NOT ST_IsEmpty(OLD.\"geom\")) BEGIN UPDATE \"rtree_{t}_geom\" SET minx = ST_MinX(NEW.\"geom\"), maxx = ST_MaxX(NEW.\"geom\"),miny = ST_MinY(NEW.\"geom\"), maxy = ST_MaxY(NEW.\"geom\") WHERE id = NEW.\"fid\";END;
                CREATE TRIGGER \"rtree_{t}_geom_update7\" AFTER UPDATE OF \"geom\" ON \"{t}\" WHEN OLD.\"fid\" = NEW.\"fid\" AND (NEW.\"geom\" NOTNULL AND NOT ST_IsEmpty(NEW.\"geom\")) AND (OLD.\"geom\" ISNULL OR ST_IsEmpty(OLD.\"geom\")) BEGIN INSERT INTO \"rtree_{t}_geom\" VALUES (NEW.\"fid\",ST_MinX(NEW.\"geom\"), ST_MaxX(NEW.\"geom\"),ST_MinY(NEW.\"geom\"), ST_MaxY(NEW.\"geom\")); END;
                CREATE TRIGGER \"rtree_{t}_geom_update2\" AFTER UPDATE OF \"geom\" ON \"{t}\" WHEN OLD.\"fid\" = NEW.\"fid\" AND (NEW.\"geom\" ISNULL OR ST_IsEmpty(NEW.\"geom\")) BEGIN DELETE FROM \"rtree_{t}_geom\" WHERE id = OLD.\"fid\"; END;
                CREATE TRIGGER \"rtree_{t}_geom_update5\" AFTER UPDATE ON \"{t}\" WHEN OLD.\"fid\" != NEW.\"fid\" AND (NEW.\"geom\" NOTNULL AND NOT ST_IsEmpty(NEW.\"geom\")) BEGIN DELETE FROM \"rtree_{t}_geom\" WHERE id = OLD.\"fid\"; INSERT OR REPLACE INTO \"rtree_{t}_geom\" VALUES (NEW.\"fid\",ST_MinX(NEW.\"geom\"), ST_MaxX(NEW.\"geom\"),ST_MinY(NEW.\"geom\"), ST_MaxY(NEW.\"geom\")); END;
                CREATE TRIGGER \"rtree_{t}_geom_update4\" AFTER UPDATE ON \"{t}\" WHEN OLD.\"fid\" != NEW.\"fid\" AND (NEW.\"geom\" ISNULL OR ST_IsEmpty(NEW.\"geom\")) BEGIN DELETE FROM \"rtree_{t}_geom\" WHERE id IN (OLD.\"fid\", NEW.\"fid\"); END;
                CREATE TRIGGER \"rtree_{t}_geom_delete\" AFTER DELETE ON \"{t}\" WHEN old.\"geom\" NOT NULL BEGIN DELETE FROM \"rtree_{t}_geom\" WHERE id = OLD.\"fid\"; END;
                CREATE TRIGGER \"trigger_insert_feature_count_{t}\" AFTER INSERT ON \"{t}\" BEGIN UPDATE gpkg_ogr_contents SET feature_count = feature_count + 1 WHERE lower(table_name) = lower('{t}'); END;
                CREATE TRIGGER \"trigger_delete_feature_count_{t}\" AFTER DELETE ON \"{t}\" BEGIN UPDATE gpkg_ogr_contents SET feature_count = feature_count - 1 WHERE lower(table_name) = lower('{t}'); END;\n",
                geometry_name(layer),
                self.rows[layer as usize]
            );
        }
        sql += "COMMIT;";
        self.con.execute_batch(&sql).map_err(e)
    }
}
