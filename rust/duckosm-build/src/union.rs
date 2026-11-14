//! The build by tiles (docs/design/build_by_tiles.md): the cut gives one file for each tile, each tile is built alone, and the
//! union joins the tables of the tiles into the tables of the area.
//!
//! Each row has one owner tile, and the build of a tile writes only its own rows ([`own_rows`]): the tile that holds the
//! node of the row (the node is inside the area and in the tile, or it is a home node of the tile, outside the area; see
//! `duckosm_core::tilecut`). The tile that holds a node has all ways of the node, so its rows there are complete. The
//! few rows that no tile alone owns (outside the area, and the small tables of rules and relations) go to border files.
//!
//! So the union only puts the files together: the own rows of the tiles, copied as blocks of bytes, and the border rows,
//! merged with one copy of each row.

use duckosm_core::boundary::Boundary;
use duckosm_core::filter::Mode;
use duckosm_core::geom::Pt;
use duckosm_core::network::Options;
use duckosm_core::tiles::Tile;
use duckosm_parquet::merge::{concat_files, union_sorted};
use rayon::prelude::*;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

fn io(e: std::io::Error) -> String {
    e.to_string()
}

fn create(p: &Path) -> Result<BufWriter<File>, String> {
    File::create(p).map(BufWriter::new).map_err(|e| format!("{}: {e}", p.display()))
}

/// The tables of the tiles and the key of their rows (the key of the merge of the border rows). `{m}`: the name of the mode.
const TABLES: [(&str, &[&str]); 16] = [
    ("edges_{m}", &["edge_id"]),
    ("private_edges_{m}", &["edge_id"]),
    ("nodes_{m}", &["node_id"]),
    ("edge_graph_{m}", &["from_edge", "to_edge"]),
    ("turn_permission_{m}", &["from_edge", "to_edge", "restriction_id"]),
    ("lane_profile_{m}", &["edge_id", "lane_num"]),
    ("ways_{m}", &["osm_id"]),
    ("way_nodes_{m}", &["way_id", "seq"]),
    ("virtual_nodes_{m}", &["node_id"]),
    ("edge_id_map_{m}", &["new_edge_id", "seq", "old_edge_id"]),
    ("turn_restrictions_{m}", &["restriction_id", "via_node", "from_edge_id", "to_edge_id"]),
    ("turn_path_restrictions_{m}", &["restriction_id", "from_edge", "to_edge"]),
    ("global_junctions_main", &["node_id"]),
    ("raw_nodes", &["osm_id"]),
    ("raw_ways", &["osm_id"]),
    ("raw_relations", &["osm_id"]),
];

/// The last step of a build by tiles: the union of the tile builds (each tile with its output folder, in the order of the
/// tiles) into `out`, for the modes of `p`, and the tables of the whole area. `boundary`: the GeoJSON text of the area.
/// `consume`: each file of a tile goes when the union has read it. Returns the rows of each table.
pub fn union_all(tiles: &[(Tile, PathBuf)], boundary: &str, p: &crate::Params, consume: bool, out: &Path) -> Result<Vec<(String, usize)>, String> {
    std::fs::create_dir_all(out).map_err(io)?;
    let tmp = std::env::temp_dir().join(format!("duckosm-union-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(io)?;
    let mut names: Vec<(String, &[&str])> = Vec::new();
    for (name, key) in TABLES {
        if name.contains("{m}") {
            names.extend(p.modes.iter().map(|m| (name.replace("{m}", m.name()), key)));
        } else {
            names.push((name.to_string(), key));
        }
    }
    // the tables at the same time (each one is a copy of files, and a small merge)
    let rows: Vec<(String, usize)> = names
        .par_iter()
        .map(|(name, key)| -> Result<(String, usize), String> {
            let of_tiles = |suffix: &str| -> Vec<PathBuf> {
                tiles.iter().map(|t| t.1.join(format!("{name}{suffix}"))).filter(|p| p.exists()).collect()
            };
            let mut parts = of_tiles(".parquet");
            let border = of_tiles(".border.parquet");
            if !border.is_empty() {
                let merged = tmp.join(format!("{name}.border.parquet"));
                union_sorted(&border, key, consume, create(&merged)?)?;
                parts.push(merged);
            }
            if parts.is_empty() {
                return Ok((name.clone(), 0));
            }
            let n = concat_files(&parts, consume || parts.last().is_some_and(|p| p.starts_with(&tmp)), &out.join(format!("{name}.parquet")))?;
            Ok((name.clone(), n))
        })
        .collect::<Result<_, _>>()?;
    let _ = std::fs::remove_dir_all(&tmp);
    write_whole_area_tables(out, boundary, &p.modes, &p.options)?;
    Ok(rows)
}

/// The tables of the whole area after the union of the tiles (`boundary_main`, `visualization_metadata_main`,
/// `transfers_mm`), from the boundary of the area (GeoJSON text) and the nodes of the union in `out`.
pub fn write_whole_area_tables(out: &Path, boundary: &str, modes: &[Mode], opts: &Options) -> Result<(), String> {
    let mut nodes = Vec::new();
    for &m in modes {
        nodes.push((m, duckosm_parquet::read::read_nodes(&out.join(format!("nodes_{}.parquet", m.name())))?));
    }
    let nodes_box = nodes.iter().flat_map(|(_, ns)| ns).fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |[w, s, e, n], p| {
        [w.min(p.lon), s.min(p.lat), e.max(p.lon), n.max(p.lat)]
    });
    // global_junctions_main is a table of the tiles (the union merges it)
    let opts = Options { global_junctions: false, ..opts.clone() };
    let area = crate::Area {
        boundary: Some(boundary),
        ways: &[],
        nodes_box,
        nodes: &nodes,
        junctions: &Default::default(),
        opts: &opts,
        built_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as i64),
        timezone_entry: None,
    };
    crate::write_area_tables(&area, |name| File::create(out.join(name)).map(BufWriter::new).map_err(io))
}

/// Step 0 of a build by tiles: cut the region file to the tiles of zoom `zoom` of the boundary (GeoJSON text), with the
/// junctions file of each tile, into `dir`. Returns the tiles that have a file.
pub fn cut_tiles(region: &Path, boundary: &str, zoom: u8, dir: &Path) -> Result<Vec<Tile>, String> {
    let b = Boundary::parse(boundary)?;
    let list = duckosm_core::tiles::tiles(&b, zoom);
    std::fs::create_dir_all(dir).map_err(io)?;
    let open = || File::open(region).map(|f| std::io::BufReader::with_capacity(1 << 20, f)).map_err(|e| format!("{}: {e}", region.display()));
    let counts = duckosm_core::tilecut::cut_to_tiles(open, &b, &list, dir)?;
    Ok(list.into_iter().zip(counts).filter(|(_, c)| c.nodes > 0).map(|(t, _)| t).collect())
}

/// The points of a WKB LineString.
fn line_points(w: &[u8]) -> impl Iterator<Item = Pt> + '_ {
    let n = u32::from_le_bytes(w[5..9].try_into().unwrap()) as usize;
    (0..n).map(move |k| (f64::from_le_bytes(w[9 + 16 * k..17 + 16 * k].try_into().unwrap()), f64::from_le_bytes(w[17 + 16 * k..25 + 16 * k].try_into().unwrap())))
}

/// A WKB Point.
fn point(w: &[u8]) -> Pt {
    (f64::from_le_bytes(w[5..13].try_into().unwrap()), f64::from_le_bytes(w[13..21].try_into().unwrap()))
}

/// The owner of the rows of the build of one tile (docs/design/build_by_tiles.md): `b` is the boundary of the tile build
/// (the area in the tile, and the home nodes of the tile).
pub struct Owner<'a> {
    pub b: &'a Boundary,
    pub tile: Tile,
}

impl Owner<'_> {
    /// A row at a place (a raw node, a junction, the first point of a way inside the area): the tile of the place inside
    /// the area; outside the area, a border row.
    pub fn place(&self, p: Pt) -> u8 {
        use duckosm_parquet::merge::{BORDER, MINE, OTHER};
        if !self.b.in_area(p.0, p.1) {
            BORDER
        } else if Tile::of(self.tile.z, p.0, p.1) == self.tile {
            MINE
        } else {
            OTHER
        }
    }

    /// A row of a node of the network (an edge at its source node, a turn at the target node of its from edge, a node):
    /// the tile that holds the node. A virtual node (a negative id: the middle of a split loop or parallel road) joins two
    /// halves of one way, and every tile with the way has both: the tile of its place owns it.
    pub fn node(&self, id: i64, p: Pt) -> u8 {
        use duckosm_parquet::merge::{MINE, OTHER};
        if id < 0 {
            self.place(p)
        } else if self.b.contains(p.0, p.1) {
            MINE
        } else {
            OTHER
        }
    }

    /// A way: the place of its first node inside the area (`pts`: the places of its nodes).
    pub fn way(&self, mut pts: impl Iterator<Item = Pt>) -> u8 {
        match pts.find(|p| self.b.in_area(p.0, p.1)) {
            Some(p) => self.place(p),
            None => duckosm_parquet::merge::BORDER,
        }
    }
}

/// The last step of the build of tile `tile`: the virtual nodes, `edge_id_map` and the raw tables keep only the rows that the
/// tile owns, and the rows that no tile alone owns go to a border file (the network tables are written so at once:
/// `write_tables_owned`, the edge graph, `ways`, `lane_profile`, `global_junctions`). `b`: the boundary of the tile build.
pub fn own_rows(dir: &Path, modes: &[Mode], b: &Boundary, tile: Tile) -> Result<(), String> {
    use arrow_array::cast::AsArray;
    use arrow_array::Array;
    use duckosm_parquet::merge::{split_file, BORDER};
    let o = Owner { b, tile };
    let col = |r: &arrow_array::RecordBatch, n: &str| r.column_by_name(n).cloned().ok_or(format!("no column {n}"));
    for &mode in modes {
        let m = mode.name();
        let path = |n: &str| dir.join(format!("{n}_{m}.parquet"));
        if path("virtual_nodes").exists() {
            split_file(&path("virtual_nodes"), |r| {
                let g = col(r, "geometry")?;
                let g = g.as_binary::<i32>();
                Ok((0..r.num_rows()).map(|i| o.place(point(g.value(i)))).collect())
            })?;
        }
        if path("edge_id_map").exists() {
            split_file(&path("edge_id_map"), |r| Ok(vec![BORDER; r.num_rows()]))?;
        }
    }
    // the raw tables of the area
    let raw = |n: &str| dir.join(format!("{n}.parquet"));
    if raw("raw_nodes").exists() {
        split_file(&raw("raw_nodes"), |r| {
            let (x, y) = (col(r, "lon")?, col(r, "lat")?);
            let (x, y) = (x.as_primitive::<arrow_array::types::Float64Type>(), y.as_primitive::<arrow_array::types::Float64Type>());
            Ok((0..r.num_rows()).map(|i| o.place((x.value(i), y.value(i)))).collect())
        })?;
    }
    if raw("raw_ways").exists() {
        split_file(&raw("raw_ways"), |r| {
            let g = col(r, "line")?;
            let g = g.as_binary::<i32>();
            Ok((0..r.num_rows()).map(|i| if g.is_valid(i) { o.way(line_points(g.value(i))) } else { BORDER }).collect())
        })?;
    }
    if raw("raw_relations").exists() {
        split_file(&raw("raw_relations"), |r| Ok(vec![BORDER; r.num_rows()]))?;
    }
    // the tables of the whole area: the union writes them
    for n in ["boundary_main", "visualization_metadata_main", "transfers_mm"] {
        let _ = std::fs::remove_file(raw(n));
    }
    Ok(())
}
