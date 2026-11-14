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
use duckosm_core::util::FxHashMap;
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

/// The ends of a WKB LineString.
fn line_ends(w: &[u8]) -> (Pt, Pt) {
    let n = u32::from_le_bytes(w[5..9].try_into().unwrap()) as usize;
    let pt = |k: usize| (f64::from_le_bytes(w[9 + 16 * k..17 + 16 * k].try_into().unwrap()), f64::from_le_bytes(w[17 + 16 * k..25 + 16 * k].try_into().unwrap()));
    (pt(0), pt(n.max(1) - 1))
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

/// The last step of the build of tile `tile` (option A of docs/design/build_by_tiles.md): each table of the tile in `dir`
/// keeps only the rows that the tile owns, and the rows that no tile alone owns go to a border file. Then the union only
/// puts the files of the tiles together. `b`: the boundary of the tile build (the area in the tile, and the home nodes).
/// The owners:
/// - an edge and its lanes: the tile that holds its source node; a turn: the tile that holds the target node of its from
///   edge; a node: the tile that holds it;
/// - a way of the mode (`ways`, `way_nodes`): the owner of its first edge;
/// - a raw node, a raw way, a virtual node: the tile of the node (of the first point of the way) inside the area; outside
///   the area, a border row;
/// - the small tables of rules and relations: border rows.
pub fn own_rows(dir: &Path, modes: &[Mode], b: &Boundary, tile: Tile) -> Result<(), String> {
    use arrow_array::cast::AsArray;
    use arrow_array::Array;
    use arrow_array::types::Int64Type;
    use duckosm_parquet::merge::{split_file, BORDER, MINE, OTHER};
    let col = |r: &arrow_array::RecordBatch, n: &str| r.column_by_name(n).cloned().ok_or(format!("no column {n}"));
    let in_tile = |p: Pt| if b.in_area(p.0, p.1) { if Tile::of(tile.z, p.0, p.1) == tile { MINE } else { OTHER } } else { BORDER };
    for &mode in modes {
        let m = mode.name();
        let path = |n: &str| dir.join(format!("{n}_{m}.parquet"));
        // the edges: owned by the holder of the source node; the turns at their target node; the first edge of each way. A
        // virtual node (a negative id: the middle of a split loop or of a split parallel road) joins two halves of one way:
        // every tile with the way has both, so the tile of its place owns it, and outside the area it is a border row.
        let class = |id: i64, p: Pt| if id < 0 { in_tile(p) } else if b.contains(p.0, p.1) { MINE } else { OTHER };
        let (mut own, mut turns): (FxHashMap<i64, u8>, FxHashMap<i64, u8>) = Default::default();
        let mut first: FxHashMap<i64, (u32, u8)> = FxHashMap::default(); // osm_id -> (seq of the first edge, its class)
        for name in ["edges", "private_edges"] {
            if !path(name).exists() {
                continue;
            }
            split_file(&path(name), |r| {
                let (ids, osm) = (col(r, "edge_id")?, col(r, "osm_id")?);
                let (ids, osm) = (ids.as_primitive::<Int64Type>(), osm.as_primitive::<Int64Type>());
                let (g, refs) = (col(r, "geometry")?, col(r, "edge_ref")?);
                let (g, refs) = (g.as_binary::<i32>(), refs.as_string::<i32>());
                let (src, tgt) = (col(r, "source")?, col(r, "target")?);
                let (src, tgt) = (src.as_primitive::<Int64Type>(), tgt.as_primitive::<Int64Type>());
                Ok((0..r.num_rows())
                    .map(|i| {
                        let (a, z) = line_ends(g.value(i));
                        let c = class(src.value(i), a);
                        turns.insert(ids.value(i), class(tgt.value(i), z));
                        // edge_ref: {osm_id}#{seq}{f|r}; the first edge is the forward edge with the least seq
                        let er = refs.value(i);
                        if er.ends_with('f') {
                            let seq = er[er.find('#').map_or(0, |k| k + 1)..er.len() - 1].parse::<u32>().unwrap_or(u32::MAX);
                            let e = first.entry(osm.value(i)).or_insert((u32::MAX, OTHER));
                            if seq < e.0 {
                                *e = (seq, c);
                            }
                        }
                        own.insert(ids.value(i), c);
                        c
                    })
                    .collect())
            })?;
        }
        // the rows of an edge: of the class of its node (the turns and the rules of turns: the target node of the from edge)
        let by_id = |name: &str, column: &'static str, classes: &FxHashMap<i64, u8>| -> Result<(), String> {
            if path(name).exists() {
                split_file(&path(name), |r| {
                    let c = col(r, column)?;
                    let c = c.as_primitive::<Int64Type>();
                    Ok(c.values().iter().map(|v| classes.get(v).copied().unwrap_or(OTHER)).collect())
                })?;
            }
            Ok(())
        };
        by_id("edge_graph", "from_edge", &turns)?;
        by_id("turn_permission", "from_edge", &turns)?;
        by_id("turn_restrictions", "from_edge_id", &turns)?;
        by_id("turn_path_restrictions", "from_edge", &turns)?;
        by_id("lane_profile", "edge_id", &own)?;
        if path("nodes").exists() {
            split_file(&path("nodes"), |r| {
                let g = col(r, "geometry")?;
                let g = g.as_binary::<i32>();
                let ids = col(r, "node_id")?;
                let ids = ids.as_primitive::<Int64Type>();
                Ok((0..r.num_rows()).map(|i| class(ids.value(i), point(g.value(i)))).collect())
            })?;
        }
        // a way of the mode: the owner of its first edge; a way with no edge is a border row
        for (name, column) in [("ways", "osm_id"), ("way_nodes", "way_id")] {
            if path(name).exists() {
                split_file(&path(name), |r| {
                    let c = col(r, column)?;
                    let c = c.as_primitive::<Int64Type>();
                    Ok(c.values().iter().map(|v| first.get(v).map_or(BORDER, |f| f.1)).collect())
                })?;
            }
        }
        if path("virtual_nodes").exists() {
            split_file(&path("virtual_nodes"), |r| {
                let g = col(r, "geometry")?;
                let g = g.as_binary::<i32>();
                Ok((0..r.num_rows()).map(|i| in_tile(point(g.value(i)))).collect())
            })?;
        }
        for name in ["edge_id_map"] {
            if path(name).exists() {
                split_file(&path(name), |r| Ok(vec![BORDER; r.num_rows()]))?;
            }
        }
    }
    // the raw tables of the area
    let raw = |n: &str| dir.join(format!("{n}.parquet"));
    if raw("raw_nodes").exists() {
        split_file(&raw("raw_nodes"), |r| {
            let (x, y) = (col(r, "lon")?, col(r, "lat")?);
            let (x, y) = (x.as_primitive::<arrow_array::types::Float64Type>(), y.as_primitive::<arrow_array::types::Float64Type>());
            Ok((0..r.num_rows()).map(|i| in_tile((x.value(i), y.value(i)))).collect())
        })?;
    }
    if raw("raw_ways").exists() {
        split_file(&raw("raw_ways"), |r| {
            let g = col(r, "line")?;
            let g = g.as_binary::<i32>();
            Ok((0..r.num_rows())
                .map(|i| match g.is_valid(i).then(|| line_points(g.value(i)).find(|p| b.in_area(p.0, p.1))).flatten() {
                    Some(p) if Tile::of(tile.z, p.0, p.1) == tile => MINE,
                    Some(_) => OTHER,
                    None => BORDER,
                })
                .collect())
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
