//! Step 0 of a build by tiles: one read of the region file gives one file for each tile ([`crate::tiles`]). The cut is
//! the pbfcut library (`pbfcut::tilecut`, docs/design/build_by_tiles.md). This module keeps the names that duckOSM and
//! its hosts call, and gives pbfcut the rule of duckOSM's junctions ([`Junctions`]) for the junctions file of each tile.

use crate::boundary::Boundary;
use crate::cutstream::CutCounts;
use crate::tiles::Tile;
use pbfcut::tilecut::NodeUses;
use std::io::{Read, Seek};
use std::path::Path;

pub use crate::tiles::near_tile;
pub use pbfcut::tilecut::{border_m, read_junctions, Sink, TileFiles, HOME, JUNCTIONS_MAGIC};

/// The node is a junction in the whole area (`main.global_junctions`).
pub const JUNCTION: u8 = 1;
/// The junction is a road junction (`is_road_junction`).
pub const ROAD: u8 = 2;

/// The rule of `main.global_junctions`, for the junctions files of the cut: a tile does not have all ways of a node
/// outside it, so the file tells it which of these nodes are junctions in the whole area.
struct Junctions;

impl NodeUses for Junctions {
    fn of_way(&self, tags: &[(String, String)], refs: &[i64], out: &mut Vec<(i64, u8)>) {
        let tags = crate::model::Tags::from_pairs(tags.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        crate::network::JunctionUses::of_way(&tags, refs, out);
    }
    fn of_node(&self, uses: &mut dyn Iterator<Item = u8>) -> u8 {
        match crate::network::global_junction(&crate::network::JunctionUses::of(uses)) {
            Some(true) => JUNCTION | ROAD,
            Some(false) => JUNCTION,
            None => 0,
        }
    }
}

/// The steps of pbfcut go to [`crate::util::step`] (the probes and the progress of the page).
fn steps() {
    let _ = pbfcut::util::STEP_HOOK.set(crate::util::step);
}

/// The positions of the nodes of `ids` in a PBF file (the home nodes of a tile: the build of the tile holds them).
pub fn node_positions<R: Read + Send>(r: R, ids: &crate::util::FxHashSet<i64>) -> Result<Vec<(i64, (f64, f64))>, String> {
    pbfcut::tilecut::node_positions(r, ids)
}

/// Cut the region file that `open` reads (a new reader for each pass) to the tiles of `tiles` inside `boundary`: the file
/// of a tile goes into `dir` as `z_x_y.osm.pbf` with its junctions file `z_x_y.junctions`; a tile with no node inside has
/// no file. Gives the numbers of elements of each tile. `multipolygons`: complete the multipolygons of each tile with all
/// their members (osmium's smart cut). Only `raw.*`, the features and the building faces read them.
pub fn cut_to_tiles<R: Read + Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    tiles: &[Tile],
    dir: &Path,
    multipolygons: bool,
    border_m: f64,
) -> Result<Vec<CutCounts>, String> {
    steps();
    let complete: &[&str] = if multipolygons { &["multipolygon"] } else { &[] };
    // zlib: the tile files can stay (--keep-tiles), and the cut of France keeps its peak memory
    pbfcut::tilecut::cut_to_tiles(open, boundary, tiles, dir, complete, border_m, Some(&Junctions), true)
}

/// [`cut_to_tiles`] into the temporary store (`spill::temp_store`: a folder natively, OPFS in the browser), with no
/// junctions files and with no zlib in the tile files: quack by tiles. `complete`: the types of the relations to complete
/// with all their members in each tile with a member (quack: `multipolygon` and `boundary`).
pub fn cut_to_store<R: Read + Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    tiles: &[Tile],
    complete: &[&str],
) -> Result<TileFiles, String> {
    steps();
    pbfcut::tilecut::cut_to_store(open, boundary, tiles, complete, false)
}

/// The cut of the build by tiles ([`cut_to_tiles`]) into the temporary store: the cut where no folder is (the browser).
/// The tile files are the part "tile" of each tile, the junctions the part "junctions".
pub fn cut_to_store_build<R: Read + Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    tiles: &[Tile],
    multipolygons: bool,
    border_m: f64,
) -> Result<TileFiles, String> {
    steps();
    pbfcut::tilecut::cut_to_store_build(open, boundary, tiles, multipolygons, border_m, Some(&Junctions), true)
}

/// [`cut_to_store`], with each tile file given to `sink` (in pieces, tile by tile): the browser writes each tile into a
/// file of its own, so that a worker for each tile can read it. The tile files have no zlib (less CPU; Sweden in
/// Chromium at zoom 8: the cut 17.8 s, with zlib 24.0 s). `memory`: the tile files with zlib, so they take less space
/// where the page keeps them in memory. pbfcut keeps its temporary files in the store (OPFS in the browser).
pub fn cut_to_sink<R: Read + Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    tiles: &[Tile],
    complete: &[&str],
    memory: bool,
    sink: Sink,
) -> Result<Vec<CutCounts>, String> {
    steps();
    pbfcut::tilecut::cut_to_sink(open, boundary, tiles, complete, 0.0, memory, sink)
}
