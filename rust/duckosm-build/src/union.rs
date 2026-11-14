//! The union of the tiles of a large build: each tile is built alone from its tile file and its junctions file
//! (`duckosm_core::tilecut`), and the union joins their tables into the tables of the whole area.
//!
//! - The edges and the nodes: with the junctions file, a tile cuts its ways at the same nodes as the build of the whole
//!   area, also at the nodes outside the tile. So a way in more tiles has the same edges in each of them: the union keeps
//!   one copy of each edge (by `edge_id`), and counts the copies that are not equal (a check: it must be 0).
//! - The rows of a node (the turns at the node, the lanes of an edge from its source node): a tile has all roads of the
//!   nodes that it holds (inside the area and in the tile), and only some roads of the other nodes. So such a row comes
//!   from the tile that holds the node. No tile holds a node outside the area: there the union makes the turns itself,
//!   with the function of the build (`EdgeGraph`), from the edges that start at those nodes.

use duckosm_core::boundary::Boundary;
use duckosm_core::filter::Mode;
use duckosm_core::geom::Pt;
use duckosm_core::graph::{restricted_label, EdgeGraph, GraphRow, TurnPermission, TurnRestriction};
use duckosm_core::lanes::LaneRow;
use duckosm_core::model::{Attrs, Edge, Node};
use duckosm_core::network::Options;
use duckosm_core::spill::{self, Codec, EdgeCodec, Rows, Writer};
use duckosm_core::tiles::Tile;
use duckosm_core::util::{FxHashMap, FxHashSet};
use duckosm_parquet::read::{read_edge_graph, read_edges, read_lane_profile, read_turn_permission, read_turn_restrictions};
use duckosm_parquet::EdgeCols;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SORT_BUDGET: usize = 64 << 20;

/// The counts of a union.
#[derive(Debug, Default)]
pub struct UnionStats {
    /// The edges of all tiles, with the copies.
    pub rows: usize,
    pub edges: usize,
    pub private_edges: usize,
    pub nodes: usize,
    /// The edges that two tiles give with other values (must be 0).
    pub not_equal: usize,
    pub edge_graph: usize,
    /// The edges whose turns the union made: they end at a node that no tile holds.
    pub turns_made: usize,
    pub lane_profile: usize,
    /// The rows of the tables that the tiles share (ways, way_nodes, raw.*, ...).
    pub shared_rows: usize,
}

fn io(e: std::io::Error) -> String {
    e.to_string()
}

fn put_str(o: &mut Vec<u8>, s: &str) {
    o.extend_from_slice(&(s.len() as u32).to_le_bytes());
    o.extend_from_slice(s.as_bytes());
}

fn get_str(b: &mut &[u8]) -> String {
    let n = u32::from_le_bytes(b[..4].try_into().unwrap()) as usize;
    let s = String::from_utf8_lossy(&b[4..4 + n]).into_owned();
    *b = &b[4 + n..];
    s
}

fn get_i64(b: &mut &[u8]) -> i64 {
    let v = i64::from_le_bytes(b[..8].try_into().unwrap());
    *b = &b[8..];
    v
}

fn get_u32(b: &mut &[u8]) -> u32 {
    let v = u32::from_le_bytes(b[..4].try_into().unwrap());
    *b = &b[4..];
    v
}

/// The values of the turn uses and the lane uses are few: each one is kept once for the run.
fn fixed(v: String) -> &'static str {
    static SEEN: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap();
    match seen.iter().find(|s| **s == v) {
        Some(s) => s,
        None => {
            let s: &'static str = Box::leak(v.into_boxed_str());
            seen.push(s);
            s
        }
    }
}

/// A row of a tile in a temporary file: the number of the tile, then the row.
struct OfTile<C>(C);

impl<T, C: Codec<T>> Codec<(u32, T)> for OfTile<C> {
    fn put(&self, r: &(u32, T), o: &mut Vec<u8>) {
        o.extend_from_slice(&r.0.to_le_bytes());
        self.0.put(&r.1, o);
    }
    fn get(&self, b: &mut &[u8]) -> (u32, T) {
        let t = get_u32(b);
        (t, self.0.get(b))
    }
}

struct GraphCodec;

impl Codec<GraphRow> for GraphCodec {
    fn put(&self, r: &GraphRow, o: &mut Vec<u8>) {
        o.extend_from_slice(&r.from_edge.to_le_bytes());
        o.extend_from_slice(&r.to_edge.to_le_bytes());
        o.push(r.cost.is_some() as u8);
        o.extend_from_slice(&r.cost.unwrap_or(0.0).to_le_bytes());
        put_str(o, r.uses);
    }
    fn get(&self, b: &mut &[u8]) -> GraphRow {
        let (from_edge, to_edge) = (get_i64(b), get_i64(b));
        let has = b[0] != 0;
        let c = f32::from_le_bytes(b[1..5].try_into().unwrap());
        *b = &b[5..];
        GraphRow { from_edge, to_edge, cost: has.then_some(c), uses: fixed(get_str(b)) }
    }
}

struct LaneCodec;

impl Codec<LaneRow> for LaneCodec {
    fn put(&self, r: &LaneRow, o: &mut Vec<u8>) {
        o.extend_from_slice(&r.edge_id.to_le_bytes());
        o.extend_from_slice(&r.lane_num.to_le_bytes());
        o.extend_from_slice(&r.width_m.to_le_bytes());
        put_str(o, r.use_);
        o.push(r.turn.is_some() as u8);
        put_str(o, r.turn.as_deref().unwrap_or(""));
        put_str(o, &r.source);
        put_str(o, &r.width_source);
    }
    fn get(&self, b: &mut &[u8]) -> LaneRow {
        let edge_id = get_i64(b);
        let lane_num = get_u32(b) as i32;
        let width_m = f64::from_le_bytes(b[..8].try_into().unwrap());
        *b = &b[8..];
        let use_ = fixed(get_str(b));
        let has_turn = b[0] != 0;
        *b = &b[1..];
        let turn = Some(get_str(b)).filter(|_| has_turn);
        LaneRow { edge_id, lane_num, use_, width_m, turn, source: get_str(b), width_source: get_str(b) }
    }
}

/// The tile that holds a point: the tile of the point, when the point is inside the area and the tile is built.
struct Holder<'a> {
    boundary: &'a Boundary,
    z: u8,
    index: FxHashMap<(u32, u32), u32>,
}

impl Holder<'_> {
    fn of(&self, p: Pt) -> Option<u32> {
        if !self.boundary.contains(p.0, p.1) {
            return None;
        }
        let t = Tile::of(self.z, p.0, p.1);
        self.index.get(&(t.x, t.y)).copied()
    }
}

/// The key of a sort of rows of tiles: the first two numbers of the row, then the tile.
fn key_of_tile_row(b: &[u8]) -> (i64, i64, u32) {
    let mut x = &b[4..];
    (get_i64(&mut x), get_i64(&mut x), u32::from_le_bytes(b[..4].try_into().unwrap()))
}

/// Join the tables of mode `mode` of the tile builds (each tile with its output folder) into `out`: `edges`,
/// `private_edges`, `nodes`, `edge_graph`, `turn_permission`, `turn_restrictions`, `lane_profile`, and the tables that the
/// tiles share (`ways`, `way_nodes`, `raw.*`, `global_junctions`, ...). Not here: the tables of the whole area (boundary,
/// metadata) and of the global steps (bus routes, levels, the largest part), which run after the union. `boundary`: the area of the whole build;
/// `opts`: the options of the tile builds.
pub fn union_tiles(tiles: &[(Tile, PathBuf)], boundary: &Boundary, mode: Mode, opts: &Options, out: &Path) -> Result<UnionStats, String> {
    let m = mode.name();
    let store = spill::temp_store();
    let store = &*store;
    let holder = Holder {
        boundary,
        z: tiles.first().map_or(0, |t| t.0.z),
        index: tiles.iter().enumerate().map(|(k, t)| ((t.0.x, t.0.y), k as u32)).collect(),
    };
    let table = |name: &str| -> Vec<(u32, PathBuf)> {
        let files = tiles.iter().enumerate().map(|(k, t)| (k as u32, t.1.join(format!("{name}_{m}.parquet"))));
        files.filter(|t| t.1.exists()).collect()
    };
    let codec = OfTile(EdgeCodec::default());
    let mut attrs: FxHashSet<Arc<Attrs>> = FxHashSet::default();
    let mut st = UnionStats::default();
    std::fs::create_dir_all(out).map_err(io)?;
    let file = |name: &str| File::create(out.join(format!("{name}_{m}.parquet"))).map(BufWriter::new).map_err(io);

    // the turn restrictions: one copy of each row (the turns at the nodes that no tile holds read them)
    let mut restrictions: Vec<TurnRestriction> = Vec::new();
    for (_, path) in table("turn_restrictions") {
        restrictions.extend(read_turn_restrictions(&path)?);
    }
    restrictions.sort();
    restrictions.dedup();
    duckosm_parquet::write_turn_restrictions_to(file("turn_restrictions")?, &restrictions)?;

    // 1. the edges and the private edges: one copy of each; the edges that start at a node that no tile holds go to the
    // graph of the union
    let mut graph = EdgeGraph::new(mode, (mode == Mode::Driving && opts.extract_restrictions).then_some(&restrictions[..]));
    let mut ends = Writer::new(store).map_err(io)?;
    let mut kept_files: Vec<Rows> = Vec::new();
    for name in ["edges", "private_edges"] {
        let private = name == "private_edges";
        let mut rows = Writer::new(store).map_err(io)?;
        for (k, path) in table(name) {
            read_edges(&path, &mut attrs, |e| {
                st.rows += 1;
                rows.push(&codec, &(k, e)).map_err(io)
            })?;
        }
        // the copies of an edge come one after the other; the bytes after the tile number are the edge
        let sorted = spill::sort_bytes(rows.finish().map_err(io)?, |b| EdgeCodec::edge_id(&b[4..]), SORT_BUDGET, store).map_err(io)?;
        let mut kept = Writer::new(store).map_err(io)?;
        let mut last: Option<Edge> = None;
        for r in sorted.read(&codec).map_err(io)? {
            let (_, e) = r.map_err(io)?;
            if let Some(l) = &last {
                if l.edge_id == e.edge_id {
                    if !same(l, &e) {
                        st.not_equal += 1;
                    }
                    continue;
                }
            }
            if private {
                st.private_edges += 1;
            } else {
                st.edges += 1;
                for (n, p) in [(e.source, e.geometry[0]), (e.target, *e.geometry.last().expect("a line"))] {
                    ends.push(&spill::join::Position, &(n, p.0, p.1)).map_err(io)?;
                }
            }
            let label = if private { restricted_label(&e) } else { None };
            if holder.of(e.geometry[0]).is_none() && (!private || label.is_some()) {
                graph.add(&e, label);
            }
            kept.push(&codec.0, &e).map_err(io)?;
            last = Some(e);
        }
        let mut kept = kept.finish().map_err(io)?;
        let rows = kept.read_ref(&codec.0).map_err(io)?.map(|r| r.map_err(io));
        duckosm_parquet::write_edges_iter_to(file(name)?, rows, EdgeCols::of(mode, opts))?;
        kept_files.push(kept);
    }
    drop(attrs);
    graph.ready();
    duckosm_core::util::step("union 1: the edges");

    // 2. the nodes: the ends of the edges (the build's nodes table), one time each, by node_id
    let ends = spill::sort_bytes(ends.finish().map_err(io)?, |b| i64::from_le_bytes(b[..8].try_into().unwrap()), SORT_BUDGET, store).map_err(io)?;
    let mut last = None;
    let nodes = ends.read(&spill::join::Position).map_err(io)?.filter_map(|r| match r {
        Ok((id, lon, lat)) if last != Some(id) => {
            last = Some(id);
            Some(Ok(Node { node_id: id, lon, lat }))
        }
        Ok(_) => None,
        Err(e) => Some(Err(io(e))),
    });
    st.nodes = duckosm_parquet::write_nodes_iter_to(file("nodes")?, nodes)?;
    duckosm_core::util::step("union 2: the nodes");

    // 3. the turns, in the order of the build: the edges and the private edges with a label, by edge_id. The turns of an edge
    // are at its target node: the rows of the tile that holds it, else made here.
    let gcodec = OfTile(GraphCodec);
    let mut rows = Writer::new(store).map_err(io)?;
    for (k, path) in table("edge_graph") {
        read_edge_graph(&path, |g| rows.push(&gcodec, &(k, g)).map_err(io))?;
    }
    let tile_rows = spill::sort_bytes(rows.finish().map_err(io)?, key_of_tile_row, SORT_BUDGET, store).map_err(io)?;
    duckosm_core::util::step("union 3a: the turns of the tiles, sorted");
    let mut tile_rows = tile_rows.read(&gcodec).map_err(io)?.peekable();
    let mut w = duckosm_parquet::EdgeGraphWriter::new(file("edge_graph")?)?;
    // the turn permissions are rows of the turns too (a small table): from the tile that holds the node, else made here
    let mut permission: FxHashMap<i64, Vec<(u32, TurnPermission)>> = FxHashMap::default();
    for (k, path) in table("turn_permission") {
        for p in read_turn_permission(&path)? {
            permission.entry(p.from_edge).or_default().push((k, p));
        }
    }
    let mut permission_out: Vec<TurnPermission> = Vec::new();
    let (edges_file, private_file) = kept_files.split_at_mut(1);
    let mut a = edges_file[0].read_ref(&codec.0).map_err(io)?.peekable();
    let mut b = private_file[0]
        .read_ref(&codec.0)
        .map_err(io)?
        .filter_map(|e| match e {
            Ok(e) => restricted_label(&e).map(|l| Ok((e, Some(l)))),
            Err(err) => Some(Err(err)),
        })
        .peekable();
    loop {
        let take_a = match (a.peek(), b.peek()) {
            (None, None) => break,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(Ok(x)), Some(Ok((y, _)))) => x.edge_id < y.edge_id,
            (Some(Err(_)), _) => true,
            (_, Some(Err(_))) => false,
        };
        let (e, label) = if take_a { (a.next().unwrap().map_err(io)?, None) } else { b.next().unwrap().map_err(io)? };
        let at = holder.of(*e.geometry.last().expect("a line"));
        while tile_rows.next_if(|r| r.as_ref().is_ok_and(|r| r.1.from_edge < e.edge_id)).is_some() {}
        // the rows of the edge come sorted by (to_edge, tile)
        while let Some(r) = tile_rows.next_if(|r| r.as_ref().is_ok_and(|r| r.1.from_edge == e.edge_id)) {
            let (t, g) = r.map_err(io)?;
            if at == Some(t) {
                st.edge_graph += 1;
                w.push(g)?;
            }
        }
        if let (Some(t), Some(rows)) = (at, permission.get(&e.edge_id)) {
            permission_out.extend(rows.iter().filter(|r| r.0 == t).map(|r| r.1.clone()));
        }
        if at.is_none() {
            st.turns_made += 1;
            graph.moves(&e, label, &mut |g| {
                st.edge_graph += 1;
                w.push(g)
            })?;
        }
    }
    drop((a, b));
    w.finish()?;
    permission_out.extend(graph.finish());
    permission_out.sort();
    duckosm_parquet::write_turn_permission_to(file("turn_permission")?, &permission_out)?;
    duckosm_core::util::step("union 3: the turns");

    // 4. the lanes of an edge: the rows of the tile that holds its source node (the lane count of an edge can come from the
    // edge before it), else of the first tile with the edge
    let lcodec = OfTile(LaneCodec);
    let mut rows = Writer::new(store).map_err(io)?;
    for (k, path) in table("lane_profile") {
        read_lane_profile(&path, |r| rows.push(&lcodec, &(k, r)).map_err(io))?;
    }
    let lane_key = |b: &[u8]| (i64::from_le_bytes(b[4..12].try_into().unwrap()), u32::from_le_bytes(b[..4].try_into().unwrap()));
    let lanes = spill::sort_bytes(rows.finish().map_err(io)?, lane_key, SORT_BUDGET, store).map_err(io)?;
    duckosm_core::util::step("union 4a: the lanes of the tiles, sorted");
    let mut lanes = lanes.read(&lcodec).map_err(io)?.peekable();
    let mut lw = duckosm_parquet::LaneProfileWriter::new(file("lane_profile")?)?;
    let mut edges = kept_files[0].read_ref(&codec.0).map_err(io)?.peekable();
    let mut group: Vec<(u32, LaneRow)> = Vec::new();
    let mut out_rows: Vec<LaneRow> = Vec::with_capacity(65_536);
    while let Some(r) = lanes.next() {
        group.clear();
        group.push(r.map_err(io)?);
        let id = group[0].1.edge_id;
        while let Some(r) = lanes.next_if(|r| r.as_ref().is_ok_and(|r| r.1.edge_id == id)) {
            group.push(r.map_err(io)?);
        }
        while edges.next_if(|e| e.as_ref().is_ok_and(|e| e.edge_id < id)).is_some() {}
        let at = match edges.peek() {
            Some(Ok(e)) if e.edge_id == id => holder.of(e.geometry[0]),
            _ => None,
        };
        let t = at.filter(|t| group.iter().any(|r| r.0 == *t)).unwrap_or(group[0].0);
        let n = out_rows.len();
        out_rows.extend(group.drain(..).filter(|r| r.0 == t).map(|r| r.1));
        st.lane_profile += out_rows.len() - n;
        // the writer takes a block of rows at a time (one write for each edge was 2.7 GB on Kentucky)
        if out_rows.len() >= 65_536 {
            lw.write(&out_rows)?;
            out_rows.clear();
        }
    }
    lw.write(&out_rows)?;
    lw.close()?;
    duckosm_core::util::step("union 4: the lanes");

    // 5. the tables that the tiles share: the same rows for one key in each tile; one row for each key
    let shared: [(&str, &[&str]); 9] = [
        (&format!("ways_{m}"), &["osm_id"]),
        (&format!("way_nodes_{m}"), &["way_id", "seq"]),
        (&format!("virtual_nodes_{m}"), &["node_id"]),
        (&format!("edge_id_map_{m}"), &["new_edge_id", "seq", "old_edge_id"]),
        (&format!("turn_path_restrictions_{m}"), &["restriction_id", "from_edge", "to_edge"]),
        ("global_junctions_main", &["node_id"]),
        ("raw_nodes", &["osm_id"]),
        ("raw_ways", &["osm_id"]),
        ("raw_relations", &["osm_id"]),
    ];
    for (name, key) in shared {
        let inputs: Vec<PathBuf> = tiles.iter().map(|t| t.1.join(format!("{name}.parquet"))).filter(|p| p.exists()).collect();
        if !inputs.is_empty() {
            let f = File::create(out.join(format!("{name}.parquet"))).map(BufWriter::new).map_err(io)?;
            st.shared_rows += duckosm_parquet::merge::union_sorted(&inputs, key, f)?;
        }
    }
    duckosm_core::util::step("union 5: the shared tables");
    Ok(st)
}

/// Two copies of an edge have the same values.
fn same(a: &Edge, b: &Edge) -> bool {
    a.seq == b.seq
        && (a.source, a.target, a.osm_id, a.oneway, a.lanes, a.is_reverse) == (b.source, b.target, b.osm_id, b.oneway, b.lanes, b.is_reverse)
        && a.attrs == b.attrs
        && a.access == b.access
        && a.refs == b.refs
        && a.geometry == b.geometry
        && a.length_m.to_bits() == b.length_m.to_bits()
        && a.maxspeed_kmh.map(f32::to_bits) == b.maxspeed_kmh.map(f32::to_bits)
        && a.cost_s.map(f32::to_bits) == b.cost_s.map(f32::to_bits)
        && a.dismount == b.dismount
        && a.ftype == b.ftype
}
