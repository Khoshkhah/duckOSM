//! Tile tables of quack (docs/rust/quack.md, "Tile tables"): QuackOSM's features table of a `.osm.pbf` file as one table
//! for each map tile of one zoom, `<dir>/<z>/<x>/<y>.parquet`, and the union of any set of them.
//!
//! The table of a tile holds every row whose geometry touches the tile (`quack::row_tiles`, the rule of QuackOSM's
//! `geometry_filter`), whole, so a tile table is usable alone. A row that touches more tiles is in each of them; its
//! column `owner` is true in one tile only, so the union of tile tables (the rows with `owner`) has each row one time.
//!
//! 1. The file is cut to the tiles of the zoom, and each tile makes the rows that it owns (`quack::features_of_tiles`).
//!    A row that touches only its own tile goes at once to the part file of the tile. A row that touches more tiles goes
//!    one time to a temporary file (the side rows), with a pair (tile, row) for each tile that it touches.
//! 2. The table of each tile: its part file, then its side rows (sorted pairs), with `owner` true only in the tile that
//!    made the row.
use duckosm_core::quack::{self, Feature, Numbered, TileSize};
use duckosm_core::spill::{Blob, Codec};
use duckosm_core::tiles::Tile;
use duckosm_parquet::quack::{owned_rows, read_tile_table, tile_batch, QuackWriter};
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The numbers of [`quack_tile_tables`].
#[derive(Debug, Default, Clone, Copy)]
pub struct TileTables {
    /// the tile tables
    pub tiles: usize,
    /// the rows (each one time: the rows of the union of all tiles)
    pub rows: usize,
    /// the rows in all tile tables (a row in each tile that it touches)
    pub stored: usize,
}

/// The file of the table of tile `t` in `dir`.
pub fn tile_table_path(dir: &Path, t: Tile) -> PathBuf {
    dir.join(format!("{}/{}/{}.parquet", t.z, t.x, t.y))
}

fn tile_key(t: Tile) -> u64 {
    ((t.x as u64) << 32) | t.y as u64
}

/// The side rows: each row one time in a temporary file, with its place.
struct Side {
    file: Box<dyn Blob>,
    end: u64,
    /// place, length
    rows: Vec<(u64, u32)>,
    buf: Vec<u8>,
}

impl Side {
    /// Add a row that tile `owner` made; gives its number.
    fn push(&mut self, f: &Feature, owner: Tile) -> std::io::Result<u32> {
        self.buf.clear();
        Numbered::put_row(tile_key(owner), f, &mut self.buf);
        self.file.seek(SeekFrom::Start(self.end))?;
        self.file.write_all(&self.buf)?;
        self.rows.push((self.end, self.buf.len() as u32));
        self.end += self.buf.len() as u64;
        Ok(self.rows.len() as u32 - 1)
    }

    /// Row `i` and the key of the tile that owns it.
    fn get(&mut self, i: u32) -> std::io::Result<(u64, Feature)> {
        let (at, len) = self.rows[i as usize];
        self.buf.resize(len as usize, 0);
        self.file.seek(SeekFrom::Start(at))?;
        self.file.read_exact(&mut self.buf)?;
        Ok(Numbered.get(&mut &self.buf[..]))
    }
}

/// The tile tables of zoom `zoom` (0 to 14) of the `.osm.pbf` file that `open` opens (sorted by type and id), into `dir`.
/// `source`: the name of the file, in the metadata of each table (`duckosm_tile`), with the tile and the time of the
/// file. A table in `dir` with the same tile is replaced.
// ponytail: the pairs (tile, side row) stay in memory (16 bytes each; France at zoom 9: about 10 million); on disk when a
// larger area needs it
pub fn quack_tile_tables<R: Read + Seek + Send>(open: impl Fn() -> Result<R, String>, zoom: u8, dir: &Path, source: &str) -> Result<TileTables, String> {
    let io = |e: std::io::Error| format!("quack tile tables: {e}");
    if zoom > 14 {
        return Err(format!("quack tile tables: zoom {zoom}: use 0 to 14"));
    }
    let (timestamp, _) = duckosm_core::cutstream::header(open()?)?;
    let meta = |t: Tile| {
        let v = serde_json::json!({"z": t.z, "x": t.x, "y": t.y, "source": source, "timestamp": timestamp});
        vec![("duckosm_tile".to_string(), v.to_string())]
    };
    let part_path = |t: Tile| dir.join(format!("{}/{}/{}.part.parquet", t.z, t.x, t.y));
    let create = |p: &Path| -> Result<BufWriter<File>, String> {
        std::fs::create_dir_all(p.parent().expect("a folder")).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok(BufWriter::with_capacity(1 << 20, File::create(p).map_err(|e| format!("{}: {e}", p.display()))?))
    };
    let store = duckosm_core::spill::temp_store();
    let mut side = Side { file: store.create().map_err(io)?, end: 0, rows: Vec::new(), buf: Vec::new() };
    let mut pairs: Vec<(u64, u32)> = Vec::new();
    let mut part: Option<(Tile, QuackWriter<BufWriter<File>>)> = None;
    let mut tiles: Vec<Tile> = Vec::new();
    let mut n = TileTables::default();
    // 1. the rows of each tile: to its part file, or one time to the side rows
    let counts = quack::features_of_tiles(&open, quack::BUDGET, TileSize::Zoom(zoom), |p, rows| {
        let touched: Vec<Vec<Tile>> = rows
            .par_iter()
            .map(|f| {
                let mut v = Vec::new();
                quack::row_tiles(&f.geometry, zoom, &mut v)?;
                // the tile that made the row has it (a point on its side can count for the next tile only)
                if !v.contains(&p) {
                    v.push(p);
                }
                Ok(v)
            })
            .collect::<Result<_, String>>()?;
        let mut mine = Vec::new();
        for (f, ts) in rows.into_iter().zip(touched) {
            if ts.len() == 1 {
                mine.push((f, true));
            } else {
                let i = side.push(&f, p).map_err(io)?;
                pairs.extend(ts.into_iter().map(|t| (tile_key(t), i)));
            }
        }
        if part.as_ref().map(|w| w.0) != Some(p) {
            if let Some((_, w)) = part.take() {
                w.close()?;
            }
            part = Some((p, QuackWriter::tile(create(&part_path(p))?, Vec::new())?));
            tiles.push(p);
        }
        let w = &mut part.as_mut().expect("a part file").1;
        for b in mine.par_chunks(20_000).map(tile_batch).collect::<Result<Vec<_>, _>>()? {
            w.write(&b)?;
        }
        Ok(())
    })?;
    if let Some((_, w)) = part.take() {
        w.close()?;
    }
    n.rows = counts.nodes + counts.ways + counts.relations;
    duckosm_core::util::step(&format!("quack tile tables 1: {} side rows in {} pairs", side.rows.len(), pairs.len()));
    // 2. the table of each tile: its part file and its side rows
    pairs.par_sort_unstable();
    let key_tile = |k: u64| Tile { z: zoom, x: (k >> 32) as u32, y: k as u32 };
    tiles.extend(pairs.iter().map(|p| key_tile(p.0)));
    // the order of the pairs (x, then y)
    tiles.sort_by_key(|&t| tile_key(t));
    tiles.dedup();
    let mut at = 0;
    for &t in &tiles {
        let path = tile_table_path(dir, t);
        let mut w = QuackWriter::tile(create(&path)?, meta(t))?;
        let part = part_path(t);
        if part.exists() {
            read_tile_table(&part, |b| w.write(&b))?;
            std::fs::remove_file(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        }
        let mut rows = Vec::new();
        while at < pairs.len() && pairs[at].0 == tile_key(t) {
            let (owner, f) = side.get(pairs[at].1).map_err(io)?;
            rows.push((f, owner == tile_key(t)));
            at += 1;
            if rows.len() == 20_000 || at == pairs.len() || pairs[at].0 != tile_key(t) {
                w.write(&tile_batch(&std::mem::take(&mut rows))?)?;
            }
        }
        n.stored += w.close()?;
        n.tiles += 1;
    }
    duckosm_core::util::step(&format!("quack tile tables 2: {} tile tables", n.tiles));
    Ok(n)
}

/// The tile tables under `dir` (`<z>/<x>/<y>.parquet`), sorted by path.
pub fn tile_table_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))? {
            let p = e.map_err(|e| format!("{}: {e}", d.display()))?.path();
            if p.is_dir() {
                todo.push(p);
            } else if p.extension().is_some_and(|x| x == "parquet") && !p.to_string_lossy().ends_with(".part.parquet") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The union of tile tables (`files`): the rows with `owner`, with QuackOSM's columns, as one GeoParquet file through
/// `out`; with `sort`, in QuackOSM's order. The tiles of one cut give each row one time. Gives the number of rows.
pub fn quack_union<W: Write + Send>(files: &[PathBuf], out: W, sort: bool) -> Result<usize, String> {
    let mut w = QuackWriter::new(out)?;
    let mut rows = duckosm_parquet::quack::Rows::new(sort);
    for f in files {
        read_tile_table(f, |b| {
            let owned = owned_rows(&b)?;
            if sort {
                rows.push(duckosm_parquet::quack::features_of(&owned)?)?;
                Ok(())
            } else {
                w.write(&owned)
            }
        })?;
    }
    rows.sorted(|b| w.write(&b))?;
    w.close()
}
