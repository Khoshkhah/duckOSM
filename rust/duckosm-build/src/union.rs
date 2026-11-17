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
use duckosm_core::util::FxHashSet;
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
const TABLES: [(&str, &[&str]); 21] = [
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
    ("h3_nodes_{m}", &["node_id"]),
    ("h3_edges_{m}", &["edge_id"]),
    ("elevation_nodes_{m}", &["node_id"]),
    ("elevation_edges_{m}", &["edge_id"]),
    // the nodes with no height: only for the count of `elevation_metadata_main` (the union deletes the table)
    ("elevation_nodata_{m}", &["node_id"]),
    ("global_junctions_main", &["node_id"]),
    ("raw_nodes", &["osm_id"]),
    ("raw_ways", &["osm_id"]),
    ("raw_relations", &["osm_id"]),
];

/// The last step of a build by tiles: the union of the tile builds (each tile with its output folder, in the order of the
/// tiles) into `out`, for the modes of `p`, and the tables of the whole area. `boundary`: the GeoJSON text of the area.
/// `consume`: each file of a tile goes when the union has read it. Returns the rows of each table.
/// One step of a build by tiles with its own line: `tiles: {name}: {time} s, peak {memory} MB`. The peak is that of this
/// process during the step (Linux: `/proc/self/clear_refs` resets the peak, `VmHWM` reads it; elsewhere no peak).
pub fn timed<T>(name: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
    let t = std::time::Instant::now();
    let r = f()?;
    let peak = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()));
    match peak {
        Some(kb) => eprintln!("tiles: {name}: {:.1} s, peak {} MB", t.elapsed().as_secs_f64(), kb >> 10),
        None => eprintln!("tiles: {name}: {:.1} s", t.elapsed().as_secs_f64()),
    }
    Ok(r)
}

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
    let rows: Vec<(String, usize)> = timed("union", || names
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
            // the merged border file is temporary; the files of the tiles stay unless `consume` (--keep-tiles keeps them)
            let n = concat_files(&parts, consume, &out.join(format!("{name}.parquet")))?;
            Ok((name.clone(), n))
        })
        .collect::<Result<_, _>>())?;
    let _ = std::fs::remove_dir_all(&tmp);
    if p.bus_routes && p.modes.contains(&Mode::Driving) {
        timed("bus routes", || bus_routes(tiles, out))?;
    }
    if p.levels != crate::LevelsMethod::None {
        timed("levels", || levels(&p.modes, p.levels, out))?;
    }
    if let Some(dem) = &p.dem {
        timed("elevation metadata and grid", || elevation_main(dem, p, &rows, out))?;
    }
    timed("area tables", || write_whole_area_tables(out, boundary, &p.modes, &p.options))?;
    Ok(rows.into_iter().filter(|(name, _)| !name.starts_with("elevation_nodata_")).collect())
}

/// The elevation tables of the whole area after the union (as the whole build writes them): `elevation_metadata_main`, with the
/// nodes with a height and with no height of all modes, and `elevation_grid_main`, the pixels over the box of the nodes of every
/// mode with 3 pixels more on each side, in strips of rows ([`crate::elevation::write_grid`]).
fn elevation_main(dem: &Path, p: &crate::Params, rows: &[(String, usize)], out: &Path) -> Result<(), String> {
    let count = |prefix: &str| rows.iter().filter(|(n, _)| n.starts_with(prefix)).map(|(_, r)| *r as i64).sum::<i64>();
    let (n_nodes, n_nodata) = (count("elevation_nodes_"), count("elevation_nodata_"));
    for m in &p.modes {
        let _ = std::fs::remove_file(out.join(format!("elevation_nodata_{}.parquet", m.name())));
    }
    let head = crate::elevation::Dem::open_pixels(dem, None)?;
    crate::write_elevation_metadata(p, head.resolution_m, n_nodes, n_nodata, |name| create(&out.join(name)))?;
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for m in &p.modes {
        let path = out.join(format!("nodes_{}.parquet", m.name()));
        if path.exists() {
            duckosm_parquet::read::each_node(&path, |n| b = [b[0].min(n.lon), b[1].min(n.lat), b[2].max(n.lon), b[3].max(n.lat)])?;
        }
    }
    if b[0] <= b[2] && b[1] <= b[3] {
        crate::elevation::write_grid(dem, b, 3, create(&out.join("elevation_grid_main.parquet"))?)?;
    }
    Ok(())
}

/// The `.duckdb` file of a build by tiles (`{name}.duckdb` in `out`): the tables of the union in `out`, as the whole build loads
/// its files (`duckosm_duckdb::write_database`), and `raw.*` when the union has the raw files. Gives the path of the file.
#[cfg(feature = "duckdb")]
pub fn write_duckdb(out: &Path, p: &crate::Params) -> Result<PathBuf, String> {
    let db = out.join(format!("{}.duckdb", p.name));
    let names: Vec<&str> = p.modes.iter().map(|m| m.name()).collect();
    duckosm_duckdb::write_database(&db, out, &names, p.memory_limit.as_deref())?;
    if out.join("raw_nodes.parquet").exists() {
        duckosm_duckdb::write_raw_from_parquet(&db, out, p.memory_limit.as_deref())?;
    }
    Ok(db)
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
/// The border of each tile in metres (`tilecut::border_m`): the ways within it go to the tile too, for the connector of the
/// dangling paths. 0 when no connector runs (driving only, or `connect_paths` off).
pub fn border_m(p: &crate::Params) -> f64 {
    if p.options.connect_paths && p.modes.iter().any(|&m| m != Mode::Driving) {
        duckosm_core::tilecut::border_m(p.options.snap_m)
    } else {
        0.0
    }
}

pub fn cut_tiles(region: &Path, boundary: &str, zoom: u8, multipolygons: bool, border_m: f64, dir: &Path) -> Result<Vec<Tile>, String> {
    let b = Boundary::parse(boundary)?;
    let list = duckosm_core::tiles::tiles(&b, zoom);
    std::fs::create_dir_all(dir).map_err(io)?;
    let open = || File::open(region).map(|f| std::io::BufReader::with_capacity(1 << 20, f)).map_err(|e| format!("{}: {e}", region.display()));
    let counts = duckosm_core::tilecut::cut_to_tiles(open, &b, &list, dir, multipolygons, border_m)?;
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

/// The class of each row of a part of a raw table of a tile build (0 `raw_nodes`, 1 `raw_ways`, 2 `raw_relations`;
/// `duckosm_parquet::write_area_raw_split`): a node by its place, a way by the place of its first node inside the area (a way with
/// no line: a border row), a relation always a border row (one copy after the union).
pub fn raw_classes(o: &Owner, table: usize, r: &arrow_array::RecordBatch) -> Result<Vec<u8>, String> {
    use arrow_array::cast::AsArray;
    use arrow_array::Array;
    use duckosm_parquet::merge::BORDER;
    let col = |n: &str| r.column_by_name(n).cloned().ok_or(format!("no column {n}"));
    match table {
        0 => {
            let (x, y) = (col("lon")?, col("lat")?);
            let (x, y) = (x.as_primitive::<arrow_array::types::Float64Type>(), y.as_primitive::<arrow_array::types::Float64Type>());
            Ok((0..r.num_rows()).map(|i| o.place((x.value(i), y.value(i)))).collect())
        }
        1 => {
            let g = col("line")?;
            let g = g.as_binary::<i32>();
            Ok((0..r.num_rows()).map(|i| if g.is_valid(i) { o.way(line_points(g.value(i))) } else { BORDER }).collect())
        }
        _ => Ok(vec![BORDER; r.num_rows()]),
    }
}

/// The last step of the build of tile `tile`: the virtual nodes, `edge_id_map` and the raw tables keep only the rows that the
/// tile owns, and the rows that no tile alone owns go to a border file (the network tables are written so at once:
/// `write_tables_owned`, the edge graph, `ways`, `lane_profile`, `global_junctions`). `b`: the boundary of the tile build.
pub fn own_rows(dir: &Path, modes: &[Mode], b: &Boundary, tile: Tile) -> Result<(), String> {
    use arrow_array::cast::AsArray;
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
    // the raw tables are split as they are written (raw_classes); the tables of the whole area: the union writes them
    let raw = |n: &str| dir.join(format!("{n}.parquet"));
    for n in ["boundary_main", "visualization_metadata_main", "transfers_mm"] {
        let _ = std::fs::remove_file(raw(n));
    }
    Ok(())
}

/// The member ways of the routes of one batch of the bus step (see [`bus_routes`]).
const BUS_BATCH_WAYS: usize = 20_000;

/// `bus.routes` and `bus.route_edges` after the union (docs/design/build_by_tiles.md, "The bus routes", and
/// docs/design/tail_steps_by_tiles.md): a bus route crosses tiles, so the walk of the routes (`bus::routes`, the function of
/// the whole build) runs after the union, on the route relations and their member ways of all tiles (each tile's
/// `bus.osm.pbf`, one copy of each), and on the driving edges and bus lanes of those ways, read from the tables of the union
/// without their lines. The routes go in batches of about [`BUS_BATCH_WAYS`] member ways, in the order of their ids (the
/// order of the whole build). The relations, the member ways and the member edges go to temporary files sorted by batch,
/// and each batch holds only its own data: only the list of (member way, batch) stays in memory (16 bytes for each).
/// Returns the number of routes.
pub fn bus_routes(tiles: &[(Tile, PathBuf)], out: &Path) -> Result<usize, String> {
    use duckosm_core::graph::MemberType;
    use duckosm_core::raw::{read_part, Want};
    use duckosm_core::spill::{self, EdgeCodec, Writer};
    let store = spill::temp_store();
    let store = &*store;
    // 1. the relations and the member ways of all tiles to two temporary files, sorted by id (a route is in each tile of its
    // ways: the sorted files keep one copy of each id)
    let (mut rel_w, mut way_w) = (Writer::new(store).map_err(io)?, Writer::new(store).map_err(io)?);
    // the files of one tile for each thread at a time (the files of all tiles at once can be large)
    for group in tiles.chunks(rayon::current_num_threads().max(1)) {
        let parts = group
            .par_iter()
            .map(|(_, dir)| {
                let path = dir.join("bus.osm.pbf");
                if !path.exists() {
                    return Ok(None);
                }
                let open = || File::open(&path).map(std::io::BufReader::new).map_err(|e| format!("{}: {e}", path.display()));
                let all = |_: i64, _: &[(&str, &str)]| true;
                let want = Want { relation: &all, way: &all, node: &|_| false, way_nodes: false, tag: &|_, _, _| true };
                read_part(open, &want).map(Some)
            })
            .collect::<Result<Vec<_>, String>>()?;
        for part in parts.into_iter().flatten() {
            for r in &part.relations {
                rel_w.push(&RelationCodec, r).map_err(io)?;
            }
            for w in part.ways {
                way_w.push(&WayCodec, &(0, w)).map_err(io)?;
            }
        }
    }
    duckosm_core::util::step("bus 1a: the files of the tiles");
    let id_at = |b: &[u8], at: usize| i64::from_le_bytes(b[at..at + 8].try_into().unwrap());
    let mut rels = spill::sort_bytes(rel_w.finish().map_err(io)?, |b| id_at(b, 0), 64 << 20, store).map_err(io)?;
    let ways = spill::sort_bytes(way_w.finish().map_err(io)?, |b| id_at(b, 4), 64 << 20, store).map_err(io)?;
    duckosm_core::util::step("bus 1b: sorted by id");
    // 2. the batch of each relation (consecutive relations with about BUS_BATCH_WAYS member ways), and the batches of each
    // member way. The relations stay in the file, in the order of their ids (the order of the batches)
    let (mut batch, mut n, mut n_rel) = (0u32, 0usize, 0usize);
    let mut way_batches: Vec<(i64, u32)> = Vec::new();
    let mut last = None;
    for r in rels.read_ref(&RelationCodec).map_err(io)? {
        let r = r.map_err(io)?;
        if last.replace(r.id) == Some(r.id) {
            continue;
        }
        let k = r.members.iter().filter(|m| m.1 == MemberType::Way).count();
        if n > 0 && n + k > BUS_BATCH_WAYS {
            batch += 1;
            n = 0;
        }
        n += k;
        n_rel += 1;
        way_batches.extend(r.members.iter().filter(|m| m.1 == MemberType::Way).map(|m| (m.0, batch)));
    }
    let batches = if n_rel == 0 { 0 } else { batch as usize + 1 };
    way_batches.sort_unstable();
    way_batches.dedup();
    way_batches.shrink_to_fit();
    let batches_of = |osm: i64| {
        let i = way_batches.partition_point(|x| x.0 < osm);
        way_batches[i..].iter().take_while(move |x| x.0 == osm).map(|x| x.1)
    };
    duckosm_core::util::step("bus 1c: the batches");
    // 3. each member way (one copy) to the file of its batches, sorted by batch, then id
    let mut w = Writer::new(store).map_err(io)?;
    let (mut n_way, mut last) = (0usize, None);
    for r in ways.read(&WayCodec).map_err(io)? {
        let (_, way) = r.map_err(io)?;
        if last.replace(way.id) == Some(way.id) {
            continue;
        }
        n_way += 1;
        for b in batches_of(way.id) {
            w.push(&WayCodec, &(b, way.clone())).map_err(io)?;
        }
    }
    let ways = spill::sort_bytes(w.finish().map_err(io)?, |b| (u32::from_le_bytes(b[..4].try_into().unwrap()), id_at(b, 4)), 64 << 20, store)
        .map_err(io)?;
    duckosm_core::util::step(&format!("bus 1: {n_rel} route relations, {n_way} member ways"));
    // 4. one read of the union edges: each member edge, without its line, to the file of its batches, sorted by
    // (batch, private, edge_id): one row for each batch of an edge
    let codec = BatchEdge(EdgeCodec::default());
    let mut w = Writer::new(store).map_err(io)?;
    for (k, name) in ["edges", "private_edges"].into_iter().enumerate() {
        let path = out.join(format!("{name}_driving.parquet"));
        if !path.exists() {
            continue;
        }
        // only the columns of the walk (it reads the nodes of an edge, not its line or its tags)
        duckosm_parquet::read::read_route_edges(&path, |osm| batches_of(osm).next().is_some(), |e| {
            for b in batches_of(e.osm_id) {
                w.push(&codec, &(b, k as u8, e.clone())).map_err(io)?;
            }
            Ok(())
        })?;
    }
    let key = |b: &[u8]| (u32::from_le_bytes(b[..4].try_into().unwrap()), b[4], EdgeCodec::edge_id(&b[5..]));
    let sorted = spill::sort_bytes(w.finish().map_err(io)?, key, 64 << 20, store).map_err(io)?;
    drop(way_batches);
    duckosm_core::util::step(&format!("bus 2: the member edges to {batches} batches"));
    // 5. each batch: its relations, its member ways and its edges, from the three sorted files (in the order of the ids, and
    // the edges in the order of edge_id, as the whole build gives them). The rows of each batch go to part files
    let mut edge_rows = sorted.read(&codec).map_err(io)?.peekable();
    let mut way_rows = ways.read(&WayCodec).map_err(io)?.peekable();
    let mut rel_rows = rels.read(&RelationCodec).map_err(io)?;
    let mut pending = None;
    let tmp = std::env::temp_dir().join(format!("duckosm-bus-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(io)?;
    let (mut n_routes, mut route_parts, mut parts) = (0usize, Vec::new(), Vec::new());
    let (mut n, mut last) = (0usize, None);
    for b in 0..batches as u32 {
        // the relations of the batch: the next relations up to about BUS_BATCH_WAYS member ways (the rule of step 2)
        let mut rels = Vec::new();
        loop {
            let r = match pending.take() {
                Some(r) => r,
                None => match rel_rows.next() {
                    Some(r) => r.map_err(io)?,
                    None => break,
                },
            };
            if last == Some(r.id) {
                continue;
            }
            let k = r.members.iter().filter(|m| m.1 == MemberType::Way).count();
            if n > 0 && n + k > BUS_BATCH_WAYS {
                n = 0;
                pending = Some(r);
                break;
            }
            n += k;
            last = Some(r.id);
            rels.push(r);
        }
        let mut own_ways = Vec::new();
        while let Some(r) = way_rows.next_if(|r| r.as_ref().is_ok_and(|r| r.0 == b)) {
            own_ways.push(r.map_err(io)?.1);
        }
        // the edges of the batch come sorted by (private, edge_id)
        let (mut edges, mut private_edges) = (Vec::new(), Vec::new());
        while let Some(r) = edge_rows.next_if(|r| r.as_ref().is_ok_and(|r| r.0 == b)) {
            let (_, k, e) = r.map_err(io)?;
            let list: &mut Vec<duckosm_core::model::Edge> = if k == 0 { &mut edges } else { &mut private_edges };
            if list.last().is_none_or(|l| l.edge_id != e.edge_id) {
                list.push(e);
            }
        }
        let (routes, rows) = duckosm_core::bus::routes(&rels, &own_ways, &edges, &private_edges);
        n_routes += routes.len();
        let part = tmp.join(format!("{b}.routes.parquet"));
        duckosm_parquet::write_routes_to(create(&part)?, &routes)?;
        route_parts.push(part);
        let part = tmp.join(format!("{b}.parquet"));
        duckosm_parquet::write_route_edges_to(create(&part)?, &rows)?;
        parts.push(part);
    }
    duckosm_core::util::step("bus 3: the batches");
    if parts.is_empty() {
        duckosm_parquet::write_routes_to(create(&out.join("routes_bus.parquet"))?, &[])?;
        duckosm_parquet::write_route_edges_to(create(&out.join("route_edges_bus.parquet"))?, &[])?;
    } else {
        concat_files(&route_parts, true, &out.join("routes_bus.parquet"))?;
        concat_files(&parts, true, &out.join("route_edges_bus.parquet"))?;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(n_routes)
}

fn put_str(s: &str, o: &mut Vec<u8>) {
    o.extend_from_slice(&(s.len() as u32).to_le_bytes());
    o.extend_from_slice(s.as_bytes());
}

fn get_u32(b: &mut &[u8]) -> u32 {
    let v = u32::from_le_bytes(b[..4].try_into().unwrap());
    *b = &b[4..];
    v
}

fn get_i64(b: &mut &[u8]) -> i64 {
    let v = i64::from_le_bytes(b[..8].try_into().unwrap());
    *b = &b[8..];
    v
}

fn get_str(b: &mut &[u8]) -> String {
    let n = get_u32(b) as usize;
    let s = String::from_utf8_lossy(&b[..n]).into_owned();
    *b = &b[n..];
    s
}

fn put_tags(tags: &[(String, String)], o: &mut Vec<u8>) {
    o.extend_from_slice(&(tags.len() as u32).to_le_bytes());
    for (k, v) in tags {
        put_str(k, o);
        put_str(v, o);
    }
}

fn get_tags(b: &mut &[u8]) -> Vec<(String, String)> {
    (0..get_u32(b)).map(|_| (get_str(b), get_str(b))).collect()
}

/// A route relation of the bus step in a temporary file: its id first (the sort key), its tags, its members.
struct RelationCodec;

impl duckosm_core::spill::Codec<duckosm_core::graph::Relation> for RelationCodec {
    fn put(&self, r: &duckosm_core::graph::Relation, o: &mut Vec<u8>) {
        use duckosm_core::graph::MemberType;
        o.extend_from_slice(&r.id.to_le_bytes());
        put_tags(&r.tags, o);
        o.extend_from_slice(&(r.members.len() as u32).to_le_bytes());
        for (id, t, role) in &r.members {
            o.extend_from_slice(&id.to_le_bytes());
            o.push(match t {
                MemberType::Node => 0,
                MemberType::Way => 1,
                MemberType::Relation => 2,
            });
            put_str(role, o);
        }
    }
    fn get(&self, b: &mut &[u8]) -> duckosm_core::graph::Relation {
        use duckosm_core::graph::MemberType;
        let id = get_i64(b);
        let tags = get_tags(b);
        let members = (0..get_u32(b))
            .map(|_| {
                let id = get_i64(b);
                let t = match b[0] {
                    0 => MemberType::Node,
                    1 => MemberType::Way,
                    _ => MemberType::Relation,
                };
                *b = &b[1..];
                (id, t, get_str(b))
            })
            .collect();
        duckosm_core::graph::Relation { id, tags, members }
    }
}

/// A member way of the bus step in a temporary file: its batch and its id first (the sort keys), its tags, its nodes.
struct WayCodec;

impl duckosm_core::spill::Codec<(u32, duckosm_core::raw::RawWay)> for WayCodec {
    fn put(&self, (batch, w): &(u32, duckosm_core::raw::RawWay), o: &mut Vec<u8>) {
        o.extend_from_slice(&batch.to_le_bytes());
        o.extend_from_slice(&w.id.to_le_bytes());
        put_tags(&w.tags, o);
        o.extend_from_slice(&(w.refs.len() as u32).to_le_bytes());
        for r in &w.refs {
            o.extend_from_slice(&r.to_le_bytes());
        }
    }
    fn get(&self, b: &mut &[u8]) -> (u32, duckosm_core::raw::RawWay) {
        let batch = get_u32(b);
        let id = get_i64(b);
        let tags = get_tags(b);
        let refs = (0..get_u32(b)).map(|_| get_i64(b)).collect();
        (batch, duckosm_core::raw::RawWay { id, tags, refs })
    }
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

/// The drawing levels after the union (docs/design/build_by_tiles.md, "The levels"), from the edge tables of the union: each
/// pass reads only the columns of the levels, and each tile of the levels sorts its edges by `edge_id` (the order of the whole
/// build). `solve`: by tiles in their order, each with the numbers of the earlier tiles fixed (`levels::tiled`), with memory
/// that does not grow with the area. `tags`: the rule of the whole build by tiles too (`roadstyle_levels::tags_tile`): the
/// rule reads only the edges at the ends of each edge.
fn levels(modes: &[Mode], method: crate::LevelsMethod, out: &Path) -> Result<(), String> {
    // the tags of the levels: few sets of values, one copy of each
    let mut attrs = FxHashSet::default();
    let rank = |m: Mode, private: bool| {
        (match m {
            Mode::Driving => 0,
            Mode::Walking => 1,
            Mode::Cycling => 2,
        }) + if private { 3 } else { 0 }
    };
    let built_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as i64);
    let file = |name: &str| create(&out.join(name));
    // each pass of the levels reads the columns that it needs from the edge tables (no temporary copy)
    let mut each = |f: &mut dyn FnMut(u8, i64, &std::sync::Arc<duckosm_core::model::Attrs>, Vec<(f64, f64)>) -> Result<(), String>| -> Result<(), String> {
        for &m in modes {
            for private in [false, true] {
                let name = if private { "private_edges" } else { "edges" };
                let path = out.join(format!("{name}_{}.parquet", m.name()));
                if path.exists() {
                    let r = rank(m, private);
                    duckosm_parquet::read::read_level_edges(&path, &mut attrs, |id, a, g| f(r, id, a, g))?;
                }
            }
        }
        Ok(())
    };
    crate::levels::tiled::write_levels_tiled_from(&mut each, method, built_at, file)?;
    Ok(())
}
