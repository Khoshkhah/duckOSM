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
use duckosm_core::util::{FxHashMap, FxHashSet};
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
    // the largest part (an option, on by default as in the whole build): each tile drops the rows of the other parts
    if p.options.keep_largest_component {
        for &m in &p.modes {
            keep_largest_part(tiles, m)?;
        }
        duckosm_core::util::step("union: the largest part");
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
    if p.bus_routes && p.modes.contains(&Mode::Driving) {
        bus_routes(tiles, out)?;
    }
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
/// `multipolygons`: complete the multipolygons of each tile (only raw.*, the features and the building faces need them).
pub fn cut_tiles(region: &Path, boundary: &str, zoom: u8, multipolygons: bool, dir: &Path) -> Result<Vec<Tile>, String> {
    let b = Boundary::parse(boundary)?;
    let list = duckosm_core::tiles::tiles(&b, zoom);
    std::fs::create_dir_all(dir).map_err(io)?;
    let open = || File::open(region).map(|f| std::io::BufReader::with_capacity(1 << 20, f)).map_err(|e| format!("{}: {e}", region.display()));
    let counts = duckosm_core::tilecut::cut_to_tiles(open, &b, &list, dir, multipolygons)?;
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

/// The connected parts of the network of one tile build, for the filter of the largest part after the union
/// (docs/design/build_by_tiles.md, "The largest part"): a union-find on the end nodes of all edges of the tile (its own and
/// the copies of other tiles; the private edges are not in it, as in the whole build). The tile that holds a node has all
/// edges of the node, so the part of each node and edge that the tile owns is complete here; the parts of more tiles join
/// at the nodes on the borders (the links).
#[derive(Default)]
pub struct TileParts {
    index: FxHashMap<i64, u32>,
    parent: Vec<u32>,
    edges: Vec<(i64, u32)>,
    own: Vec<(u32, bool)>, // (a node of the edge, the edge is the tile's own)
    links: Vec<(i64, u32)>,
}

impl TileParts {
    fn node(&mut self, n: i64) -> u32 {
        let next = self.parent.len() as u32;
        let i = *self.index.entry(n).or_insert(next);
        if i == next {
            self.parent.push(i);
        }
        i
    }

    fn root(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            self.parent[x as usize] = self.parent[self.parent[x as usize] as usize];
            x = self.parent[x as usize];
        }
        x
    }

    /// An edge of the tile that is not private: its id, its end nodes, whether the tile owns it, and whether each end node
    /// is held by another tile or by no tile (then it is a link to other tiles).
    pub fn add(&mut self, edge_id: i64, ends: [(i64, bool); 2], own: bool) {
        let (a, b) = (self.node(ends[0].0), self.node(ends[1].0));
        let (ra, rb) = (self.root(a), self.root(b));
        if ra != rb {
            self.parent[ra.max(rb) as usize] = ra.min(rb);
        }
        self.edges.push((edge_id, a));
        self.own.push((a, own));
        for (n, foreign) in ends {
            // a node that another tile holds, or an end of a copy of an edge of another tile: the parts meet there
            if foreign || !own {
                let i = self.index[&n];
                self.links.push((n, i));
            }
        }
    }

    /// Write the side files of the parts: `parts_{m}.bin` (part, own edges, smallest node id), `edge_parts_{m}.bin` and
    /// `node_parts_{m}.bin` (id, part), `links_{m}.bin` (node, part). Each row: little endian numbers.
    pub fn write<W: std::io::Write>(mut self, mut file: impl FnMut(&str) -> Result<W, String>, m: &str) -> Result<(), String> {
        let n = self.parent.len();
        let roots: Vec<u32> = (0..n as u32).map(|i| self.root(i)).collect();
        let (mut count, mut least) = (vec![0u64; n], vec![i64::MAX; n]);
        for (&id, &i) in &self.index {
            let r = roots[i as usize] as usize;
            least[r] = least[r].min(id);
        }
        for &(a, own) in &self.own {
            if own {
                count[roots[a as usize] as usize] += 1;
            }
        }
        let e = |e: std::io::Error| e.to_string();
        let mut w = file(&format!("parts_{m}.bin"))?;
        for r in (0..n).filter(|&r| roots[r] as usize == r) {
            w.write_all(&(r as u32).to_le_bytes()).map_err(e)?;
            w.write_all(&count[r].to_le_bytes()).map_err(e)?;
            w.write_all(&least[r].to_le_bytes()).map_err(e)?;
        }
        let mut pairs = |name: &str, rows: &mut dyn Iterator<Item = (i64, u32)>| -> Result<(), String> {
            let mut w = file(&format!("{name}_{m}.bin"))?;
            for (id, p) in rows {
                w.write_all(&id.to_le_bytes()).map_err(e)?;
                w.write_all(&p.to_le_bytes()).map_err(e)?;
            }
            Ok(())
        };
        pairs("edge_parts", &mut self.edges.iter().map(|&(id, a)| (id, roots[a as usize])))?;
        pairs("node_parts", &mut self.index.iter().map(|(&id, &i)| (id, roots[i as usize])))?;
        self.links.sort_unstable();
        self.links.dedup();
        pairs("links", &mut self.links.iter().map(|&(id, i)| (id, roots[i as usize])))?;
        Ok(())
    }
}

/// The rows of a side file of [`TileParts`]: (id, part).
fn read_pairs(path: &Path) -> Result<Vec<(i64, u32)>, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(b.chunks_exact(12).map(|r| (i64::from_le_bytes(r[..8].try_into().unwrap()), u32::from_le_bytes(r[8..].try_into().unwrap()))).collect())
}

/// The filter of the largest part after the tile builds (docs/design/build_by_tiles.md, "The largest part"): the parts of
/// the tiles join at their links (a union-find on the parts, not on the nodes), the largest part has the most own edges
/// and then the smallest node id (the rule of the whole build), and each tile drops the rows of the other parts: its
/// edges, nodes and lanes, and its turns and turn rules that use a dropped edge. Returns the edges dropped.
pub fn keep_largest_part(tiles: &[(Tile, PathBuf)], mode: Mode) -> Result<u64, String> {
    let m = mode.name();
    // the parts of all tiles, one number each: the first number of tile k is base[k]
    let (mut base, mut count, mut least, mut local): (Vec<u32>, Vec<u64>, Vec<i64>, Vec<FxHashMap<u32, u32>>) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (_, dir) in tiles {
        base.push(count.len() as u32);
        let b = std::fs::read(dir.join(format!("parts_{m}.bin"))).unwrap_or_default();
        let mut map = FxHashMap::default();
        for r in b.chunks_exact(20) {
            map.insert(u32::from_le_bytes(r[..4].try_into().unwrap()), count.len() as u32);
            count.push(u64::from_le_bytes(r[4..12].try_into().unwrap()));
            least.push(i64::from_le_bytes(r[12..20].try_into().unwrap()));
        }
        local.push(map);
    }
    let mut parent: Vec<u32> = (0..count.len() as u32).collect();
    fn root(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    // the links of all tiles, sorted by node: the parts at one node join
    let mut pairs: Vec<(i64, u32)> = Vec::new();
    for (k, (_, dir)) in tiles.iter().enumerate() {
        let path = dir.join(format!("links_{m}.bin"));
        if path.exists() {
            pairs.extend(read_pairs(&path)?.into_iter().map(|(n, p)| (n, local[k][&p])));
        }
    }
    pairs.sort_unstable(); // ponytail: the links are the border nodes only (France: a few million); a sort on disk when they grow
    for run in pairs.chunk_by(|a, b| a.0 == b.0) {
        for w in run.windows(2) {
            let (a, b) = (root(&mut parent, w[0].1), root(&mut parent, w[1].1));
            if a != b {
                parent[a.max(b) as usize] = a.min(b);
            }
        }
    }
    drop(pairs);
    let n = count.len();
    let (mut total, mut small) = (vec![0u64; n], vec![i64::MAX; n]);
    for i in 0..n as u32 {
        let r = root(&mut parent, i) as usize;
        total[r] += count[i as usize];
        small[r] = small[r].min(least[i as usize]);
    }
    let Some(best) = (0..n).filter(|&r| parent[r] as usize == r).max_by_key(|&r| (total[r], std::cmp::Reverse(small[r]))) else {
        return Ok(0);
    };
    let dropped: u64 = (0..n).filter(|&r| parent[r] as usize == r && r != best).map(|r| total[r]).sum();
    // each tile: the parts that stay, then the rows of the other parts go (the tiles at the same time)
    let roots: Vec<u32> = (0..n as u32).map(|i| root(&mut parent, i)).collect();
    let keep: Vec<FxHashSet<u32>> = (0..tiles.len())
        .map(|k| local[k].iter().filter(|(_, &g)| roots[g as usize] as usize == best).map(|(&p, _)| p).collect())
        .collect();
    tiles
        .par_iter()
        .enumerate()
        .map(|(k, (_, dir))| drop_other_parts(dir, m, &keep[k]))
        .collect::<Result<Vec<()>, String>>()?;
    Ok(dropped)
}

/// The rows of one tile of the parts that do not stay (`keep`: the parts of the tile that stay).
fn drop_other_parts(dir: &Path, m: &str, keep: &FxHashSet<u32>) -> Result<(), String> {
    use arrow_array::cast::AsArray;
    use arrow_array::types::Int64Type;
    use duckosm_parquet::merge::{split_file, MINE, OTHER};
    let edges: FxHashMap<i64, u32> = read_pairs(&dir.join(format!("edge_parts_{m}.bin"))).unwrap_or_default().into_iter().collect();
    let nodes: FxHashMap<i64, u32> = read_pairs(&dir.join(format!("node_parts_{m}.bin"))).unwrap_or_default().into_iter().collect();
    // an edge goes when its part goes; a private edge (not in a part) stays
    let gone = |map: &FxHashMap<i64, u32>, id: i64| map.get(&id).is_some_and(|p| !keep.contains(p));
    let tables: [(&str, &[&str], &FxHashMap<i64, u32>); 7] = [
        ("edges", &["edge_id"], &edges),
        ("lane_profile", &["edge_id"], &edges),
        ("edge_graph", &["from_edge", "to_edge"], &edges),
        ("turn_permission", &["from_edge", "to_edge"], &edges),
        ("turn_restrictions", &["from_edge_id", "to_edge_id"], &edges),
        ("turn_path_restrictions", &["from_edge", "to_edge"], &edges),
        ("nodes", &["node_id"], &nodes),
    ];
    for (name, cols, map) in tables {
        for suffix in ["", ".border"] {
            let path = dir.join(format!("{name}_{m}{suffix}.parquet"));
            if !path.exists() {
                continue;
            }
            split_file(&path, |r| {
                let cs: Vec<arrow_array::Int64Array> = cols
                    .iter()
                    .map(|c| r.column_by_name(c).map(|a| a.as_primitive::<Int64Type>().clone()).ok_or(format!("no column {c}")))
                    .collect::<Result<_, _>>()?;
                Ok((0..r.num_rows()).map(|i| if cs.iter().any(|c| gone(map, c.value(i))) { OTHER } else { MINE }).collect())
            })?;
        }
    }
    Ok(())
}

/// The member ways of the routes of one batch of the bus step (see [`bus_routes`]).
const BUS_BATCH_WAYS: usize = 20_000;

/// `bus.routes` and `bus.route_edges` after the union (docs/design/build_by_tiles.md, "The bus routes"): a bus route crosses
/// tiles, so the walk of the routes (`bus::routes`, the function of the whole build) runs after the union, on the route
/// relations and their member ways of all tiles (each tile's `bus.osm.pbf`, one copy of each), and on the driving edges
/// and bus lanes of those ways, read from the tables of the union without their lines. The routes go in batches of about
/// [`BUS_BATCH_WAYS`] member ways, in the order of their ids (the order of the whole build): one read of the edges puts each
/// member edge into the temporary file of each batch that needs it, and each batch holds only its own edges (France:
/// 7.5 million route edges, 6.5 GB at once). Returns the number of routes.
pub fn bus_routes(tiles: &[(Tile, PathBuf)], out: &Path) -> Result<usize, String> {
    use duckosm_core::graph::MemberType;
    use duckosm_core::raw::{read_part, Want};
    use duckosm_core::spill::{self, EdgeCodec, Writer};
    let (mut relations, mut ways) = (Vec::new(), Vec::new());
    let (mut seen_rel, mut seen_way): (FxHashSet<i64>, FxHashSet<i64>) = Default::default();
    let all = |_: i64, _: &[(&str, &str)]| true;
    let want = Want { relation: &all, way: &all, node: &|_| false, way_nodes: false, tag: &|_, _, _| true };
    for (_, dir) in tiles {
        let path = dir.join("bus.osm.pbf");
        if !path.exists() {
            continue;
        }
        let open = || File::open(&path).map(std::io::BufReader::new).map_err(|e| format!("{}: {e}", path.display()));
        // one copy of each relation and way: a route is in each tile of its ways
        let part = read_part(open, &want)?;
        relations.extend(part.relations.into_iter().filter(|r| seen_rel.insert(r.id)));
        ways.extend(part.ways.into_iter().filter(|w| seen_way.insert(w.id)));
    }
    drop((seen_rel, seen_way));
    relations.sort_by_key(|r| r.id);
    ways.sort_by_key(|w| w.id);
    duckosm_core::util::step(&format!("bus 1: {} route relations, {} member ways", relations.len(), ways.len()));
    // the batches of the routes: consecutive relations with about BUS_BATCH_WAYS member ways
    let mut batch_of: Vec<u32> = Vec::with_capacity(relations.len());
    let (mut batch, mut n) = (0u32, 0usize);
    for r in &relations {
        let k = r.members.iter().filter(|m| m.1 == MemberType::Way).count();
        if n > 0 && n + k > BUS_BATCH_WAYS {
            batch += 1;
            n = 0;
        }
        n += k;
        batch_of.push(batch);
    }
    let batches = if relations.is_empty() { 0 } else { batch as usize + 1 };
    // the batches of each member way
    let mut way_batches: Vec<(i64, u32)> = relations
        .iter()
        .zip(&batch_of)
        .flat_map(|(r, &b)| r.members.iter().filter(|m| m.1 == MemberType::Way).map(move |m| (m.0, b)))
        .collect();
    way_batches.sort_unstable();
    way_batches.dedup();
    let batches_of = |osm: i64| {
        let i = way_batches.partition_point(|x| x.0 < osm);
        way_batches[i..].iter().take_while(move |x| x.0 == osm).map(|x| x.1)
    };
    // one read of the union edges: each member edge, without its line, to the files of its batches
    // one temporary file, sorted by (batch, private, edge_id) after the read: one row for each batch of an edge
    let store = spill::temp_store();
    let store = &*store;
    let codec = BatchEdge(EdgeCodec::default());
    let mut w = Writer::new(store).map_err(io)?;
    let mut attrs = FxHashSet::default();
    for (k, name) in ["edges", "private_edges"].into_iter().enumerate() {
        let path = out.join(format!("{name}_driving.parquet"));
        if !path.exists() {
            continue;
        }
        duckosm_parquet::read::read_edges(&path, &mut attrs, |osm| batches_of(osm).next().is_some(), |mut e| {
            e.geometry = Vec::new(); // the walk reads the nodes of an edge, not its line
            for b in batches_of(e.osm_id) {
                w.push(&codec, &(b, k as u8, e.clone())).map_err(io)?;
            }
            Ok(())
        })?;
    }
    drop(attrs);
    let key = |b: &[u8]| (u32::from_le_bytes(b[..4].try_into().unwrap()), b[4], EdgeCodec::edge_id(&b[5..]));
    let sorted = spill::sort_bytes(w.finish().map_err(io)?, key, 64 << 20, store).map_err(io)?;
    let mut rows = sorted.read(&codec).map_err(io)?.peekable();
    duckosm_core::util::step(&format!("bus 2: the member edges to {batches} batches"));
    // each batch: its routes, its member ways and its edges (in the order of edge_id, as the whole build gives them)
    let tmp = std::env::temp_dir().join(format!("duckosm-bus-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(io)?;
    let (mut routes, mut parts) = (Vec::new(), Vec::new());
    let mut start = 0;
    for b in 0..batches as u32 {
        let end = start + batch_of[start..].iter().take_while(|&&x| x == b).count();
        let rels = &relations[start..end];
        start = end;
        // the edges of the batch come sorted by (private, edge_id)
        let (mut edges, mut private_edges) = (Vec::new(), Vec::new());
        while let Some(r) = rows.next_if(|r| r.as_ref().is_ok_and(|r| r.0 == b)) {
            let (_, k, e) = r.map_err(io)?;
            let list: &mut Vec<duckosm_core::model::Edge> = if k == 0 { &mut edges } else { &mut private_edges };
            if list.last().is_none_or(|l| l.edge_id != e.edge_id) {
                list.push(e);
            }
        }
        let members: FxHashSet<i64> =
            rels.iter().flat_map(|r| r.members.iter()).filter(|m| m.1 == MemberType::Way).map(|m| m.0).collect();
        let own_ways: Vec<duckosm_core::raw::RawWay> = ways.iter().filter(|w| members.contains(&w.id)).cloned().collect();
        let (r, rows) = duckosm_core::bus::routes(rels, &own_ways, &edges, &private_edges);
        routes.extend(r);
        let part = tmp.join(format!("{b}.parquet"));
        duckosm_parquet::write_route_edges_to(create(&part)?, &rows)?;
        parts.push(part);
    }
    duckosm_core::util::step("bus 3: the batches");
    duckosm_parquet::write_routes_to(create(&out.join("routes_bus.parquet"))?, &routes)?;
    if parts.is_empty() {
        duckosm_parquet::write_route_edges_to(create(&out.join("route_edges_bus.parquet"))?, &[])?;
    } else {
        concat_files(&parts, true, &out.join("route_edges_bus.parquet"))?;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(routes.len())
}

/// An edge of a batch of the bus step in a temporary file: the batch, 1 for a private edge (else 0), the edge.
struct BatchEdge(duckosm_core::spill::EdgeCodec);

impl duckosm_core::spill::Codec<(u32, u8, duckosm_core::model::Edge)> for BatchEdge {
    fn put(&self, r: &(u32, u8, duckosm_core::model::Edge), o: &mut Vec<u8>) {
        o.extend_from_slice(&r.0.to_le_bytes());
        o.push(r.1);
        self.0.put(&r.2, o);
    }
    fn get(&self, b: &mut &[u8]) -> (u32, u8, duckosm_core::model::Edge) {
        let batch = u32::from_le_bytes(b[..4].try_into().unwrap());
        let k = b[4];
        *b = &b[5..];
        (batch, k, self.0.get(b))
    }
}
