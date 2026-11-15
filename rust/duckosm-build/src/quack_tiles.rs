//! Tile tables of quack (docs/rust/quack.md, "Tile tables"): QuackOSM's features table of a `.osm.pbf` file as one table
//! for each map tile of one zoom, `<dir>/<z>/<x>/<y>.parquet`, and the union of any set of them.
//!
//! 1. The cut makes the file of each tile (`tilecut::cut_to_store`): the nodes in the tile, each way with a node in the
//!    tile, whole, and each relation with a member in the tile, the multipolygons and boundaries with all their members.
//! 2. Each tile alone: quack on its file (`quack::features_owned` with no `relations`), and its table at once. The table
//!    holds the nodes in the tile, the ways with a node in the tile and the relations that the file has whole, so it is
//!    usable alone; nothing waits for another tile. Its column `owner` is true for the rows that the tile owns (a node by
//!    its place, a way by its first node, a relation by the first node of its first member way).
//! 3. The union of tile tables keeps the rows with `owner`: each row one time, with no search for doubles.
use duckosm_core::quack::{self, Owner, TileSize};
use duckosm_core::tiles::Tile;
use duckosm_parquet::quack::{owned_rows, read_tile_table, tile_batch, QuackWriter};
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};

/// The numbers of [`quack_tile_tables`].
#[derive(Debug, Default, Clone, Copy)]
pub struct TileTables {
    /// the tile tables
    pub tiles: usize,
    /// the rows that the tiles own (each row one time: the rows of the union of all tiles)
    pub rows: usize,
    /// the rows in all tile tables (a row in each tile that has it)
    pub stored: usize,
}

/// The file of the table of tile `t` in `dir`.
pub fn tile_table_path(dir: &Path, t: Tile) -> PathBuf {
    dir.join(format!("{}/{}/{}.parquet", t.z, t.x, t.y))
}

/// The tile tables of zoom `zoom` (0 to 14) of the `.osm.pbf` file that `open` opens (sorted by type and id), into `dir`.
/// `source`: the name of the file, in the metadata of each table (`duckosm_tile`), with the tile and the time of the
/// file. A table in `dir` with the same tile is replaced.
pub fn quack_tile_tables<R: Read + Seek + Send>(open: impl Fn() -> Result<R, String>, zoom: u8, dir: &Path, source: &str) -> Result<TileTables, String> {
    if zoom > 14 {
        return Err(format!("quack tile tables: zoom {zoom}: use 0 to 14"));
    }
    let (timestamp, _) = duckosm_core::cutstream::header(open()?)?;
    let tiles = quack::tiles_of_file(&open, TileSize::Zoom(zoom))?;
    duckosm_core::util::step(&format!("quack tile tables 1: {} tiles of zoom {zoom}", tiles.len()));
    let world = duckosm_core::boundary::Boundary::parse(quack::WORLD)?;
    let files = duckosm_core::tilecut::cut_to_store(&open, &world, &tiles, &["multipolygon", "boundary"], false)?;
    duckosm_core::util::step("quack tile tables 2: the tile cut");
    let todo: Vec<(usize, Tile)> = files.tiles.iter().enumerate().filter(|t| t.1 .1.nodes > 0).map(|(i, t)| (i, t.0)).collect();
    // one tile: its file, quack, its table
    let one = |&(i, tile): &(usize, Tile)| -> Result<(usize, usize), String> {
        let path = tile_table_path(dir, tile);
        std::fs::create_dir_all(path.parent().expect("a folder")).map_err(|e| format!("{}: {e}", path.display()))?;
        let file = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let meta = serde_json::json!({"z": tile.z, "x": tile.x, "y": tile.y, "source": source, "timestamp": timestamp});
        let mut w = QuackWriter::tile(BufWriter::with_capacity(1 << 20, file), vec![("duckosm_tile".to_string(), meta.to_string())])?;
        let owner = Owner { tile, relations: None };
        let mut owned = 0;
        let open_tile = || Ok(std::io::BufReader::with_capacity(1 << 20, files.reader(i)));
        quack::features_owned(open_tile, quack::BUDGET, Some(&owner), |rows| {
            owned += rows.iter().filter(|f| f.owner).count();
            for b in rows.par_chunks(20_000).map(tile_batch).collect::<Result<Vec<_>, _>>()? {
                w.write(&b)?;
            }
            Ok(())
        })?;
        Ok((owned, w.close()?))
    };
    let mut n = TileTables::default();
    for (g, group) in todo.chunks(quack::TILES_AT_ONCE).enumerate() {
        for r in group.par_iter().map(one).collect::<Vec<_>>() {
            let (owned, stored) = r?;
            n.rows += owned;
            n.stored += stored;
            n.tiles += 1;
        }
        duckosm_core::util::step(&format!("quack tile tables 3: {} of {} tiles", (g * quack::TILES_AT_ONCE + group.len()).min(todo.len()), todo.len()));
    }
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
