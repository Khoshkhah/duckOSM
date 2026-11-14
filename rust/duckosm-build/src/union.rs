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
/// `transfers_mm`), from the boundary of the area (GeoJSON text) and the nodes of the union in `out`. The nodes stay on disk:
/// the point of the time zone comes from passes over the nodes file, and the transfers from a sort on disk.
pub fn write_whole_area_tables(out: &Path, boundary: &str, modes: &[Mode], opts: &Options) -> Result<(), String> {
    let nodes_of = |m: Mode| out.join(format!("nodes_{}.parquet", m.name()));
    let lookup_mode = modes.iter().copied().find(|&m| m == Mode::Driving).or(modes.first().copied());
    let lookup = match lookup_mode {
        Some(m) => lookup_point(&nodes_of(m))?,
        None => None,
    };
    // global_junctions_main is a table of the tiles (the union puts it together)
    let o = Options { global_junctions: false, ..opts.clone() };
    let area = crate::Area {
        boundary: Some(boundary),
        ways: &[],
        nodes_box: [0.0; 4], // the box of the area comes from the boundary
        nodes: &[],
        junctions: &Default::default(),
        opts: &o,
        built_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as i64),
        timezone_entry: None,
        lookup,
    };
    crate::write_area_tables(&area, |name| File::create(out.join(name)).map(BufWriter::new).map_err(io))?;
    if opts.calculate_costs {
        write_transfers(out, modes)?;
    }
    Ok(())
}

/// The key of an f64 in the order of `f64::total_cmp`.
fn order_key(x: f64) -> u64 {
    let b = x.to_bits();
    if b >> 63 == 1 {
        !b
    } else {
        b | 1 << 63
    }
}

fn from_order_key(k: u64) -> f64 {
    f64::from_bits(if k >> 63 == 1 { k & !(1 << 63) } else { !k })
}

/// The point of the time zone (duckOSM `utils.add_timezone`, `area::lookup_point`): the node nearest (median longitude,
/// median latitude) of the real nodes (not the virtual ones) of the nodes file `path`; the first in the order of node_id
/// of two at the same distance. The medians are exact, with a radix select: each pass over the file counts the next 16
/// bits of the values (4 passes), so only small tables are in memory.
fn lookup_point(path: &Path) -> Result<Option<(f64, f64)>, String> {
    let each = |f: &mut dyn FnMut(f64, f64)| duckosm_parquet::read::each_node(path, |n| if n.node_id > 0 { f(n.lon, n.lat) });
    let mut n = 0u64;
    each(&mut |_, _| n += 1)?;
    if n == 0 {
        return Ok(None);
    }
    // the ranks to find: the middle value (an odd count), or the two middle values
    let ranks: Vec<u64> = if n % 2 == 1 { vec![n / 2] } else { vec![n / 2 - 1, n / 2] };
    // the selections (axis, rank left in the bucket, the prefix found so far)
    let mut sel: Vec<(usize, u64, u64)> = (0..2).flat_map(|axis| ranks.iter().map(move |&r| (axis, r, 0u64))).collect();
    for level in 0..4u32 {
        let shift = 48 - 16 * level;
        let mut hist = vec![vec![0u64; 1 << 16]; sel.len()];
        each(&mut |x, y| {
            for (k, &(axis, _, prefix)) in sel.iter().enumerate() {
                let key = order_key(if axis == 0 { x } else { y });
                if level == 0 || key >> (shift + 16) == prefix {
                    hist[k][((key >> shift) & 0xFFFF) as usize] += 1;
                }
            }
        })?;
        for (k, s) in sel.iter_mut().enumerate() {
            let mut d = 0;
            while s.1 >= hist[k][d] {
                s.1 -= hist[k][d];
                d += 1;
            }
            s.2 = (s.2 << 16) | d as u64;
        }
    }
    let value = |axis: usize| -> f64 {
        let v: Vec<f64> = sel.iter().filter(|s| s.0 == axis).map(|s| from_order_key(s.2)).collect();
        if v.len() == 1 {
            v[0]
        } else {
            (v[0] + v[1]) / 2.0
        }
    };
    let (mx, my) = (value(0), value(1));
    let mut best: Option<(f64, i64, (f64, f64))> = None;
    duckosm_parquet::read::each_node(path, |nd| {
        if nd.node_id <= 0 {
            return;
        }
        let d = (nd.lon - mx).powi(2) + (nd.lat - my).powi(2);
        let better = match best {
            None => true,
            Some((bd, bid, _)) => d.total_cmp(&bd).then(nd.node_id.cmp(&bid)) == std::cmp::Ordering::Less,
        };
        if better {
            best = Some((d, nd.node_id, (nd.lon, nd.lat)));
        }
    })?;
    Ok(best.map(|b| b.2))
}

/// A node id in a temporary file.
struct Id;

impl duckosm_core::spill::Codec<i64> for Id {
    fn put(&self, n: &i64, o: &mut Vec<u8>) {
        o.extend_from_slice(&n.to_le_bytes());
    }
    fn get(&self, b: &mut &[u8]) -> i64 {
        let v = i64::from_le_bytes(b[..8].try_into().unwrap());
        *b = &b[8..];
        v
    }
}

/// `transfers_mm` (`area::transfers`): the nodes of walking that cycling or driving has too, each with a change of mode in
/// both directions. The node ids of each mode are sorted on disk.
fn write_transfers(out: &Path, modes: &[Mode]) -> Result<(), String> {
    use duckosm_core::spill::{self, Writer};
    use duckosm_parquet::Transfer;
    let store = spill::temp_store();
    let store = &*store;
    let ids = |m: Mode| -> Result<Option<spill::Rows>, String> {
        if !modes.contains(&m) {
            return Ok(None);
        }
        let mut w = Writer::new(store).map_err(io)?;
        let mut err = None;
        duckosm_parquet::read::each_node(&out.join(format!("nodes_{}.parquet", m.name())), |n| {
            if let Err(e) = w.push(&Id, &n.node_id) {
                err.get_or_insert(e);
            }
        })?;
        if let Some(e) = err {
            return Err(io(e));
        }
        let rows = w.finish().map_err(io)?;
        spill::sort_bytes(rows, |b| i64::from_le_bytes(b[..8].try_into().unwrap()), 64 << 20, store).map(Some).map_err(io)
    };
    let Some(mut walk) = ids(Mode::Walking)? else {
        // no walking: an empty table, as the build writes it
        return duckosm_parquet::write_transfers_to(create(&out.join("transfers_mm.parquet"))?, &[]);
    };
    let path = out.join("transfers_mm.parquet");
    let mut parts: Vec<(Mode, &'static str, &'static str, &'static str)> = Vec::new();
    for (m, name, to, back) in [(Mode::Cycling, "cycling", "bike_park", "bike_unpark"), (Mode::Driving, "driving", "park", "retrieve")] {
        if modes.contains(&m) {
            parts.push((m, name, to, back));
        }
    }
    let mut chunks: Vec<PathBuf> = Vec::new();
    for (k, (m, name, to, back)) in parts.into_iter().enumerate() {
        let Some(other) = ids(m)? else { continue };
        let mut a = walk.read_ref(&Id).map_err(io)?.peekable();
        let mut b = other.read(&Id).map_err(io)?.peekable();
        let chunk = out.join(format!("transfers_mm.{k}.part"));
        let mut last = None;
        let pairs = std::iter::from_fn(|| loop {
            let x = match a.next()? {
                Ok(x) => x,
                Err(_) => return None,
            };
            if last == Some(x) {
                continue;
            }
            last = Some(x);
            while b.next_if(|y| y.as_ref().is_ok_and(|y| *y < x)).is_some() {}
            if b.peek().is_some_and(|y| y.as_ref().is_ok_and(|y| *y == x)) {
                return Some(x);
            }
        });
        let made = pairs.flat_map(|n| {
            [
                Transfer { node_id: n, from_mode: "walking", to_mode: name, cost_s: TRANSFER_S, kind: to },
                Transfer { node_id: n, from_mode: name, to_mode: "walking", cost_s: TRANSFER_S, kind: back },
            ]
        });
        duckosm_parquet::write_transfers_iter_to(create(&chunk)?, made)?;
        chunks.push(chunk);
    }
    if chunks.is_empty() {
        return duckosm_parquet::write_transfers_to(create(&path)?, &[]);
    }
    concat_files(&chunks, true, &path)?;
    Ok(())
}

/// duckOSM `multimodal.transfer_s` (as `area::TRANSFER_S`): the seconds of a change of mode.
const TRANSFER_S: f64 = 60.0;

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
