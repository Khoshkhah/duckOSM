//! The network step for the driving mode: ways in, edges and nodes out. It follows
//! `GlobalJunctions`, `RoadFilter`, `GraphSimplifier`, `_split_private` and `ComponentFilter` of duckOSM.
//! Not in this version: `merge_segments`, OSM overrides, speeds, costs, turn restrictions, H3.

use crate::filter::{self, Mode, ModeWay, NON_ROAD};
use crate::fixes::OsmFixes;
use crate::geom::{self, Pt};
use crate::graph::{Graph, Relation};
use crate::hash;
use crate::model::{Attrs, Edge, MapRow, Network, Node, Stats, Way};
use crate::spill::{
    self, get_i64, get_u32, put_i64, put_u32, take, Codec, EdgeCodec, Rows, Store, Writer,
};
use crate::util::{FxHashMap, FxHashSet};
use rstar::{primitives::GeomWithData, RTree, AABB};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Options {
    /// Keep only the largest connected part of the graph (duckOSM does this when a boundary is set).
    pub keep_largest_component: bool,
    /// Cycling also gets footways and pedestrian streets, as edges that are walked (`dismount`).
    pub cycling_dismount: bool,
    /// Cut the roads at one set of junctions for all modes (duckOSM `options.global_junctions`, on by default).
    pub global_junctions: bool,
    /// Add a straight connector edge from each dangling path end to the nearest node within `snap_m`
    /// (duckOSM `clip.connectivity_rescue`, on by default; walking and cycling only).
    pub connect_paths: bool,
    pub snap_m: f64,
    /// Set `maxspeed_kmh` and replace the raw `maxspeed` (duckOSM `options.process_speeds`, on by default).
    pub process_speeds: bool,
    /// Set `cost_s`, the travel time in seconds (duckOSM `options.calculate_costs`, on by default; needs the speeds).
    pub calculate_costs: bool,
    /// Add `walk_type` (walking) or `cycle_type` (cycling) (duckOSM `options.functional_types`, on by default).
    pub functional_types: bool,
    /// Join the chains of edges that are one road (duckOSM `options.merge_segments`, on by default).
    pub merge_segments: bool,
    /// Build `edge_graph` and `turn_permission` (duckOSM `options.build_graph`, on by default).
    pub build_graph: bool,
    /// Build `turn_restrictions` and `turn_path_restrictions` in the driving mode (duckOSM
    /// `options.extract_restrictions`, on by default). They need the relations of the PBF.
    pub extract_restrictions: bool,
    /// Keep the ways of the mode for the tables `ways` and `way_nodes` (duckOSM writes them always). Off saves
    /// the memory of their node lists in a large area.
    pub ways_tables: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            keep_largest_component: true,
            cycling_dismount: true,
            merge_segments: true,
            global_junctions: true,
            connect_paths: true,
            snap_m: 10.0,
            process_speeds: true,
            calculate_costs: true,
            functional_types: true,
            build_graph: true,
            extract_restrictions: true,
            ways_tables: true,
        }
    }
}

/// Node id -> (longitude, latitude), as a list sorted by node id. It uses 24 bytes for each node,
/// with no unused capacity. A hash map uses about twice that, and more while it grows.
#[derive(Default, Debug)]
pub struct Coords(Vec<(i64, Pt)>);

impl Coords {
    pub fn new(mut nodes: Vec<(i64, Pt)>) -> Coords {
        nodes.sort_unstable_by_key(|n| n.0);
        nodes.dedup_by_key(|n| n.0);
        nodes.shrink_to_fit();
        Coords(nodes)
    }

    pub fn get(&self, id: &i64) -> Option<&Pt> {
        let i = self.0.binary_search_by_key(id, |n| n.0).ok()?;
        Some(&self.0[i].1)
    }

    /// Add nodes (the new nodes of the OSM fixes). A node that is already here keeps its place.
    pub fn extend(&mut self, nodes: impl IntoIterator<Item = (i64, Pt)>) {
        let mut all = std::mem::take(&mut self.0);
        all.extend(nodes);
        *self = Coords::new(all);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &(i64, Pt)> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A forward edge while the graph is built. `way` is the index of its way in the filtered list.
struct Fwd {
    e: Edge,
    way: usize,
    pos: Pos,
}

/// The place of a forward edge in the one list of all edges that steps 1 to 7 used before part B2
/// (docs/design/network_stream.md). The steps now read the edges one way at a time, from files, and `Pos` keeps the old
/// order where it decides a tie (step 3, the largest part in step 10) or the order of a result (the edges, the virtual
/// nodes, `edge_id_map`). The fields compare in their order:
/// - after step 2b, the merged chains in the order (first edge, start, end, order of the walk), then the other edges in
///   the order of the cut;
/// - `half`: a split edge of step 3 gives its halves 0 and 2, step 4 adds 0 and 1 (the halves come in place of the edge).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Pos {
    group: u8,  // 0: a merged chain, 1: an edge that is not merged
    first: u32, // the number of the edge in the cut (of a chain: of its first edge)
    start: i64,
    end: i64,
    walk: u32,
    half: u8,
}

impl Pos {
    fn cut(n: u32) -> Pos {
        Pos {
            group: 1,
            first: n,
            ..Pos::default()
        }
    }
}

/// [`Fwd`] in a temporary file.
#[derive(Default)]
struct FwdCodec(EdgeCodec);

impl Codec<Fwd> for FwdCodec {
    fn put(&self, f: &Fwd, o: &mut Vec<u8>) {
        self.0.put(&f.e, o);
        let p = &f.pos;
        put_u32(o, f.way as u32);
        o.push(p.group);
        put_u32(o, p.first);
        put_i64(o, p.start);
        put_i64(o, p.end);
        put_u32(o, p.walk);
        o.push(p.half);
    }

    fn get(&self, b: &mut &[u8]) -> Fwd {
        let e = self.0.get(b);
        let way = get_u32(b) as usize;
        let group = take::<1>(b)[0];
        let (first, start, end, walk) = (get_u32(b), get_i64(b), get_i64(b), get_u32(b));
        let half = take::<1>(b)[0];
        Fwd {
            e,
            way,
            pos: Pos {
                group,
                first,
                start,
                end,
                walk,
                half,
            },
        }
    }
}

/// One end of a forward edge, for the merge of chains: the node, the number of the edge in the cut, its way.
struct End {
    node: i64,
    edge: u32,
    way: u32,
    at_source: bool,
}

struct EndCodec;

impl Codec<End> for EndCodec {
    fn put(&self, h: &End, o: &mut Vec<u8>) {
        put_i64(o, h.node);
        put_u32(o, h.edge);
        put_u32(o, h.way);
        o.push(h.at_source as u8);
    }

    fn get(&self, b: &mut &[u8]) -> End {
        End {
            node: get_i64(b),
            edge: get_u32(b),
            way: get_u32(b),
            at_source: take::<1>(b)[0] != 0,
        }
    }
}

/// The memory of each sorted block of a sort of a temporary file (`spill::sort`).
const SORT_BUDGET: usize = 64 << 20;

fn io(e: std::io::Error) -> String {
    format!("temporary file of the network step: {e}")
}

/// Call `f` with each run of rows that have the same `key`. The rows are sorted by `key`.
fn for_groups<T, K: PartialEq, E>(
    rows: impl Iterator<Item = Result<T, E>>,
    key: impl Fn(&T) -> K,
    mut f: impl FnMut(Vec<T>) -> Result<(), E>,
) -> Result<(), E> {
    let mut g: Vec<T> = Vec::new();
    for r in rows {
        let r = r?;
        if g.last().is_some_and(|l| key(l) != key(&r)) {
            f(std::mem::take(&mut g))?;
        }
        g.push(r);
    }
    if !g.is_empty() {
        f(g)?;
    }
    Ok(())
}
/// Count the uses of each node with a sort, not a hash map of all nodes: the entries use 16 bytes
/// for each node of each way, and only the result map is a hash map. `uses` gives
/// (node, flags) for each node of each way; `keep` gets the flags of all uses of one node.
fn junction_map(
    mut uses: Vec<(i64, u8)>,
    keep: impl Fn(&[(i64, u8)]) -> Option<bool>,
) -> FxHashMap<i64, bool> {
    uses.sort_unstable();
    let mut out = FxHashMap::default();
    for run in uses.chunk_by(|a, b| a.0 == b.0) {
        if let Some(v) = keep(run) {
            out.insert(run[0].0, v);
        }
    }
    out
}

const ROAD: u8 = 1;
const END: u8 = 2;
const ROAD_END: u8 = ROAD | END;

/// Each node of each way, with ROAD if the way is a road and END at the first and last node.
fn node_uses<'a>(ways: impl Iterator<Item = (&'a [i64], bool)> + Clone) -> Vec<(i64, u8)> {
    let mut uses = Vec::with_capacity(ways.clone().map(|w| w.0.len()).sum());
    for (refs, road) in ways {
        let n = refs.len();
        uses.extend(refs.iter().enumerate().map(|(pos, &node)| {
            let end = if pos == 0 || pos + 1 == n { END } else { 0 };
            (node, if road { ROAD } else { 0 } | end)
        }));
    }
    uses
}

/// `main.global_junctions`: node id -> is a road junction (false: only paths are cut there).
pub fn global_junctions(ways: &[Way]) -> FxHashMap<i64, bool> {
    let built = ways.iter().filter(|w| filter::is_built_highway(&w.tags));
    let uses = node_uses(built.map(|w| (&w.refs[..], filter::is_road_raw(&w.tags))));
    junction_map(uses, |run| {
        let road = run.iter().filter(|u| u.1 & ROAD != 0).count();
        let road_end = run.iter().filter(|u| u.1 & ROAD_END == ROAD_END).count();
        let ways = run.len();
        ((road > 0 && (ways > 1 || road_end > 0)) || (road == 0 && ways > 1)).then_some(road > 0)
    })
}

/// `junctions`: the nodes where the ways of this mode meet, or end. The value is `is_road_junction`.
fn local_junctions(mw: &[ModeWay]) -> FxHashMap<i64, bool> {
    let uses = node_uses(
        mw.iter()
            .map(|w| (&w.refs[..], !NON_ROAD.contains(&w.attrs.highway.as_str()))),
    );
    junction_map(uses, |run| {
        let ways = run.len();
        let ends = run.iter().filter(|u| u.1 & END != 0).count();
        let road_ends = run.iter().filter(|u| u.1 & ROAD_END == ROAD_END).count();
        (ways > 1 || ends > 0).then_some(ways > 1 || road_ends > 0)
    })
}

/// Cut the nodes of a way into groups of node positions, one group for each edge (`_segment_ways`).
fn segments(split: &[bool]) -> Vec<Vec<usize>> {
    let n = split.len();
    let mut next: Vec<Option<usize>> = vec![None; n]; // nearest split strictly after
    let mut nx = None;
    for i in (0..n).rev() {
        next[i] = nx;
        if split[i] {
            nx = Some(i);
        }
    }
    let mut groups: Vec<(Option<usize>, Vec<usize>)> = Vec::new();
    let push =
        |key: Option<usize>, i: usize, groups: &mut Vec<(Option<usize>, Vec<usize>)>| match groups
            .iter_mut()
            .rev()
            .take(2)
            .find(|g| g.0 == key)
        {
            Some(g) => g.1.push(i),
            None => groups.push((key, vec![i])),
        };
    let mut last: Option<usize> = None; // latest split at or before i
    let mut prev: Option<usize>; // latest split strictly before i
    for i in 0..n {
        prev = last;
        if split[i] {
            last = Some(i);
        }
        if !split[i] {
            push(last, i, &mut groups);
        } else {
            if next[i].is_some() {
                push(Some(i), i, &mut groups);
            }
            if prev.is_some() {
                push(prev, i, &mut groups);
            }
        }
    }
    groups.into_iter().map(|g| g.1).collect()
}

/// The two parts of a node list for a split edge. DuckDB: `refs[1 : len/2 + 1]` and `refs[len/2 + 1 :]`,
/// where `/` is a float division and the index is rounded half to even.
fn split_refs(refs: &[i64]) -> (Vec<i64>, Vec<i64>) {
    let k = ((refs.len() as f64) / 2.0 + 1.0).round_ties_even() as usize;
    let k = k.clamp(1, refs.len().max(1));
    (refs[..k].to_vec(), refs[k - 1..].to_vec())
}

fn half(src: &Edge, source: i64, target: i64, refs: Vec<i64>, geometry: Vec<Pt>) -> Edge {
    Edge {
        source,
        target,
        refs,
        geometry,
        length_m: src.length_m / 2.0,
        ..src.clone()
    }
}

fn refs_key(e: &Edge) -> String {
    hash::refs_text(&e.refs)
}

pub fn build_driving(all_ways: &[Way], coords: &Coords, opts: &Options) -> Result<Network, String> {
    build_network(all_ways, coords, Mode::Driving, opts)
}

/// The network of one mode.
pub fn build_network(
    all_ways: &[Way],
    coords: &Coords,
    mode: Mode,
    opts: &Options,
) -> Result<Network, String> {
    build_network_with(all_ways, coords, mode, opts, &OsmFixes::default(), &[])
}

/// The network of one mode, with the per-mode rules of the OSM fixes (the other rules act on the
/// ways before: `OsmFixes::apply_to_ways`), and the `type=restriction` relations of the PBF
/// (`pbf::read_restrictions`).
pub fn build_network_with(
    all_ways: &[Way],
    coords: &Coords,
    mode: Mode,
    opts: &Options,
    fixes: &OsmFixes,
    relations: &[Relation],
) -> Result<Network, String> {
    let gj = if opts.global_junctions {
        global_junctions(all_ways)
    } else {
        FxHashMap::default()
    };
    build_network_junctions(all_ways, coords, mode, opts, fixes, relations, &gj, None)
}

/// Where the rows of `edge_graph` go while the network step makes them: a file writer (docs/design/network_stream.md,
/// part C). Then `Graph::edge_graph` is `None` and `Stats::edge_graph_rows` has the number of rows.
pub type GraphRows<'a> = &'a mut dyn FnMut(crate::graph::GraphRow) -> Result<(), String>;

/// [`build_network_with`] with the global junctions given: `global_junctions(all_ways)`, or an empty map when
/// `opts.global_junctions` is off. A build of several modes computes them one time for all modes and for the table
/// `main.global_junctions` (they depend only on the ways, not on the mode). With `graph_rows`, the rows of `edge_graph` go
/// there while they are made, and the network does not keep them.
#[allow(clippy::too_many_arguments)]
pub fn build_network_junctions(
    all_ways: &[Way],
    coords: &Coords,
    mode: Mode,
    opts: &Options,
    fixes: &OsmFixes,
    relations: &[Relation],
    gj: &FxHashMap<i64, bool>,
    graph_rows: Option<GraphRows>,
) -> Result<Network, String> {
    let mut stats = Stats {
        ways_read: all_ways.len(),
        ..Default::default()
    };
    let mut mw: Vec<ModeWay> = all_ways
        .iter()
        .filter_map(|w| filter::mode_way(w, mode, opts.cycling_dismount))
        .collect();
    // duckOSM: after RoadFilter and before GraphBuilder, so oneway and the lanes act on the edges
    stats.osm_fixes = fixes.apply_to_mode(&mut mw, mode);
    stats.ways_kept = mw.len();
    let lj = local_junctions(&mw);

    crate::util::step("start of step 1: Cut every way into forward edges. Step 2");
    // 1. Cut every way into forward edges, and write them to a temporary file (docs/design/network_stream.md, part
    // B2). Step 2 runs on each edge as it is made. For the merge of chains, the two ends of each edge go to a second file.
    let store = spill::temp_store();
    let store = &*store;
    let codec = FwdCodec::default();
    let mut cut = Writer::new(store).map_err(io)?;
    let mut ends = Writer::new(store).map_err(io)?;
    let mut virtual_nodes: Vec<Node> = Vec::new();
    let mut made: Vec<Fwd> = Vec::new();
    let mut n_cut = 0u32;
    for (wi, w) in mw.iter().enumerate() {
        let is_road = !NON_ROAD.contains(&w.attrs.highway.as_str());
        let split: Vec<bool> = w
            .refs
            .iter()
            .map(|n| {
                if opts.global_junctions {
                    if is_road {
                        gj.get(n) == Some(&true)
                    } else {
                        lj.contains_key(n) || gj.contains_key(n)
                    }
                } else if is_road {
                    lj.get(n) == Some(&true)
                } else {
                    lj.contains_key(n)
                }
            })
            .collect();
        for g in segments(&split) {
            // a node without coordinates leaves the list (an inner join in the SQL)
            let mut refs = Vec::with_capacity(g.len());
            let mut geometry = Vec::with_capacity(g.len());
            for &i in &g {
                if let Some(&p) = coords.get(&w.refs[i]) {
                    refs.push(w.refs[i]);
                    geometry.push(p);
                }
            }
            if refs.len() < 2 {
                continue;
            }
            let length_m = geom::line_length_m(&geometry);
            stats.forward_edges += 1;
            let f = Fwd {
                e: Edge {
                    edge_id: 0,
                    seq: None,
                    source: refs[0],
                    target: refs[refs.len() - 1],
                    osm_id: w.id,
                    attrs: w.attrs.clone(),
                    oneway: w.oneway,
                    lanes: w.lanes_fwd,
                    access: w.access.clone(),
                    refs,
                    geometry,
                    is_reverse: false,
                    length_m,
                    dismount: w.dismount,
                    maxspeed_kmh: None,
                    cost_s: None,
                    ftype: if opts.functional_types {
                        w.type_fwd
                    } else {
                        None
                    },
                },
                way: wi,
                pos: Pos::default(),
            };
            if push_split_loop(&mut made, f, &mut virtual_nodes) {
                stats.self_loops_split += 1;
            }
            for mut f in made.drain(..) {
                f.pos = Pos::cut(n_cut);
                if opts.merge_segments {
                    for (node, at_source) in [(f.e.source, true), (f.e.target, false)] {
                        let end = End {
                            node,
                            edge: n_cut,
                            way: wi as u32,
                            at_source,
                        };
                        ends.push(&EndCodec, &end).map_err(io)?;
                    }
                }
                cut.push(&codec, &f).map_err(io)?;
                n_cut = n_cut.checked_add(1).ok_or("more than 2^32 forward edges")?;
            }
        }
    }
    drop(lj); // the junction maps are as large as the nodes of the area, and the later steps do not read them
    let cut = cut.finish().map_err(io)?;
    let ends = ends.finish().map_err(io)?;

    crate::util::step("start of step 2b: Merge the chains of edges that are one r");
    // 2b. Merge the chains of edges that are one road.
    let mut edge_id_map: Vec<MapRow> = Vec::new();
    let fwd = if opts.merge_segments {
        let (merged, map, n_chains, n_pieces) =
            merge_chains(cut, ends, &mw, gj, coords, &codec, store).map_err(io)?;
        edge_id_map = map;
        stats.merged_chains = n_chains;
        stats.merged_pieces = n_pieces;
        merged
    } else {
        cut
    };

    crate::util::step("start of steps 3 to 7: Split the arcs, add the reverse edg");
    // 3 to 7. Split the parallel and antiparallel arcs, add the reverse edges, set edge_id, edge_ref and the length. The
    // steps read the forward edges of one way (one osm_id) at a time: the keys of steps 3, 4 and 6 contain osm_id.
    let raw_refs: FxHashMap<i64, &Vec<i64>> = all_ways.iter().map(|w| (w.id, &w.refs)).collect();
    let by_way = spill::sort(
        fwd,
        &codec,
        |f: &Fwd| (f.e.osm_id, f.pos),
        SORT_BUDGET,
        store,
    )
    .map_err(io)?;
    let mut done = Writer::new(store).map_err(io)?;
    let mut split_nodes: Vec<(u8, Pos, Node)> = Vec::new();
    let rows = by_way.read(&codec).map_err(io)?.map(|r| r.map_err(io));
    for_groups(
        rows,
        |f| f.e.osm_id,
        |g| {
            for f in way_steps(
                g,
                &mw,
                &raw_refs,
                opts.functional_types,
                &mut split_nodes,
                &mut stats,
            )? {
                done.push(&codec, &f).map_err(io)?;
            }
            Ok(())
        },
    )?;
    drop(raw_refs);
    // the virtual nodes in the old order: of step 1 (in the order of the cut), then of step 3, then of step 4
    split_nodes.sort_by_key(|v| (v.0, v.1));
    virtual_nodes.extend(split_nodes.into_iter().map(|v| v.2));

    crate::util::step("start of the sort of the edges into the order of step 8");
    // Step 8 and later still take a list of all edges (part B3 changes them), in the old order: the reverse edges, then
    // the forward edges, each in the order of `Pos`. They are all in memory after this, so the sort is in memory too.
    let done = done.finish().map_err(io)?;
    let mut all: Vec<Fwd> = Vec::with_capacity(done.len);
    for f in done.read(&codec).map_err(io)? {
        all.push(f.map_err(io)?);
    }
    all.sort_unstable_by_key(|f| (!f.e.is_reverse, f.pos)); // the keys are unique
    // collected in place: the block of `all` stays, with free places at its end (a `Fwd` is larger than an `Edge`)
    let mut edges: Vec<Edge> = all.into_iter().map(|f| f.e).collect();
    edges.shrink_to_fit();
    drop(codec); // its table of the shared tags of all ways

    crate::util::step("start of step 8: The nodes");
    // 8. The nodes.
    let used: FxHashSet<i64> = edges.iter().flat_map(|e| [e.source, e.target]).collect();
    let mut nodes: Vec<Node> = Vec::new();
    for &n in &used {
        if n > 0 {
            if let Some(&(lon, lat)) = coords.get(&n) {
                nodes.push(Node {
                    node_id: n,
                    lon,
                    lat,
                });
            }
        }
    }
    drop(used);
    nodes.extend(virtual_nodes);

    crate::util::step("start of step 8b: Connect the dangling path ends to the ne");
    // 8b. Connect the dangling path ends to the network (walking and cycling).
    if opts.connect_paths && mode != Mode::Driving {
        stats.connectors = connect_paths(
            &mut edges,
            &nodes,
            opts.snap_m,
            mode == Mode::Cycling && opts.cycling_dismount,
            if opts.functional_types {
                Some(mode)
            } else {
                None
            },
        );
    }

    crate::util::step("start of step 8b2: Edges that are walked get the cycle_type");
    // 8b2. Edges that are walked get the cycle_type 'dismount'.
    if opts.functional_types && mode == Mode::Cycling {
        for e in edges.iter_mut().filter(|e| e.dismount == Some(true)) {
            e.ftype = Some("dismount");
        }
    }

    crate::util::step("start of step 8c: Speeds and travel times");
    // 8c. Speeds and travel times.
    if opts.calculate_costs && !opts.process_speeds {
        return Err("calculate_costs needs process_speeds".to_string());
    }
    if opts.process_speeds {
        crate::speeds::apply(&mut edges, mode, opts.calculate_costs);
    }

    crate::util::step("start of step 9: Move private roads, bus-only roads and p");
    // 9. Move private roads, bus-only roads and pedestrian streets to private_edges.
    let moved = |e: &Edge| {
        matches!(e.access.as_deref(), Some("private") | Some("bus"))
            || (mode == Mode::Driving && e.attrs.highway == "pedestrian")
    };
    // extract_if keeps the order, and it does not make a second copy of all edges like partition
    let mut private_edges: Vec<Edge> = edges.extract_if(.., |e| moved(e)).collect();
    stats.private_edges = private_edges.len();
    if !private_edges.is_empty() {
        prune_nodes(&mut nodes, &edges);
    }
    // The vectors grew by pushes (the connectors, the collect above), so each has up to half of its capacity unused: an Edge has
    // 160 bytes, so 60,000 unused places are 9.6 MiB (Tartu, walking). Give the memory back before the edge graph needs its own.
    edges.shrink_to_fit();
    private_edges.shrink_to_fit();
    nodes.shrink_to_fit();

    crate::util::step("start of step 10: Keep the largest connected part. It also");
    // 10. Keep the largest connected part. It runs before the turn restrictions and the edge graph (duckOSM: after them, and it
    // removes their rows of the dropped edges): the dropped edges are in other connected parts, so they share no node with the
    // kept edges, and the rows of the kept edges are the same. Then the graph rows can go to a file while they are made.
    if opts.keep_largest_component && !edges.is_empty() {
        let before = edges.len();
        keep_largest(&mut edges);
        stats.dropped_by_component_filter = before - edges.len();
        if before != edges.len() {
            prune_nodes(&mut nodes, &edges);
        }
    }
    // The edges in the order of their edge_id (the order of the tables), here and not at the end: the edge graph takes the edges
    // in this order, so it reads them one after the other in memory. After the filter, whose tie rule uses the order before.
    edges.sort_by_key(|e| e.edge_id);
    private_edges.sort_by_key(|e| e.edge_id);

    crate::util::step("start of step 9b: Turn restrictions (driving) and the edge");
    // 9b. Turn restrictions (driving) and the edge graph.
    let mut graph = Graph::default();
    if opts.extract_restrictions && mode == Mode::Driving {
        let (tr, paths) =
            crate::graph::restrictions(relations, &fixes.turn_restrictions, &mw, &edges);
        graph.turn_restrictions = Some(tr);
        graph.turn_path_restrictions = Some(paths);
    }
    crate::util::step("start of step 9b2: The edge graph and the turn permissions");
    if opts.build_graph {
        let restrictions = graph.turn_restrictions.as_deref();
        match graph_rows {
            Some(sink) => {
                let mut n = 0;
                let permission = crate::graph::edge_graph_each(
                    mode,
                    &edges,
                    &private_edges,
                    restrictions,
                    |r| {
                        n += 1;
                        sink(r)
                    },
                )?;
                stats.edge_graph_rows = n;
                graph.turn_permission = Some(permission);
            }
            None => {
                let (rows, permission) =
                    crate::graph::edge_graph(mode, &edges, &private_edges, restrictions);
                stats.edge_graph_rows = rows.len();
                graph.edge_graph = Some(rows);
                graph.turn_permission = Some(permission);
            }
        }
    }

    nodes.sort_by_key(|n| n.node_id);
    nodes.dedup_by_key(|n| n.node_id);
    Ok(Network {
        edges,
        private_edges,
        nodes,
        edge_id_map,
        graph,
        stats,
        ways: if opts.ways_tables { mw } else { Vec::new() },
    })
}

/// Step 2: split a self-loop with a virtual node in the middle, and push the two halves. Push any other
/// edge as it is. Returns true for a self-loop that is split.
fn push_split_loop(fwd: &mut Vec<Fwd>, f: Fwd, vnodes: &mut Vec<Node>) -> bool {
    if f.e.source != f.e.target {
        fwd.push(f);
        return false;
    }
    let rr = geom::remove_repeated(&f.e.geometry);
    if rr.len() < 3 {
        return false; // a loop of fewer than 3 distinct points disappears
    }
    let v = hash::virtual_node_loop(f.e.osm_id, f.e.source, &f.e.refs);
    let mid = geom::interpolate(&rr, 0.5);
    let (r1, r2) = split_refs(&f.e.refs);
    vnodes.push(Node {
        node_id: v,
        lon: mid.0,
        lat: mid.1,
    });
    fwd.push(Fwd {
        e: half(&f.e, f.e.source, v, r1, geom::substring(&rr, 0.0, 0.5)),
        way: f.way,
        pos: f.pos,
    });
    fwd.push(Fwd {
        e: half(&f.e, v, f.e.source, r2, geom::substring(&rr, 0.5, 1.0)),
        way: f.way,
        pos: f.pos,
    });
    true
}

/// Steps 3 to 7 on the forward edges of one way (one `osm_id`), in the order of `Pos`. Returns the forward and the
/// reverse edges of the way, with `edge_id`, `seq` and the length.
fn way_steps(
    mut g: Vec<Fwd>,
    mw: &[ModeWay],
    raw_refs: &FxHashMap<i64, &Vec<i64>>,
    types: bool,
    vnodes: &mut Vec<(u8, Pos, Node)>,
    stats: &mut Stats,
) -> Result<Vec<Fwd>, String> {
    if g.len() > 1 {
        // 3. Split same-direction parallel arcs (all but the shortest of each group).
        let mut groups: FxHashMap<(i64, i64), Vec<usize>> = FxHashMap::default();
        for (i, f) in g.iter().enumerate() {
            if f.e.source != f.e.target {
                groups.entry((f.e.source, f.e.target)).or_default().push(i);
            }
        }
        let mut sel = FxHashSet::default();
        for idx in groups.values().filter(|v| v.len() > 1) {
            // `Pos` (the index in the old list) is the last tie-break: a way that walks one segment twice makes two arcs
            // with the same length and the same refs, and duckOSM keeps both (it stops with an error). Here the first
            // stays whole.
            let key = |i: usize| (g[i].e.length_m, refs_key(&g[i].e), g[i].pos);
            let min = idx
                .iter()
                .map(|&i| key(i))
                .min_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap();
            for &i in idx {
                if key(i).partial_cmp(&min) == Some(std::cmp::Ordering::Greater) {
                    sel.insert(i);
                }
            }
        }
        stats.parallel_split += sel.len();
        g = split_arcs(g, &sel, 3, vnodes);

        // 4. Split two-way antiparallel pairs (the longer arc of each pair).
        let mut by_key: FxHashMap<(i64, i64), Vec<usize>> = FxHashMap::default();
        for (i, f) in g.iter().enumerate() {
            by_key.entry((f.e.source, f.e.target)).or_default().push(i);
        }
        let mut sel = FxHashSet::default();
        for (i, f) in g.iter().enumerate() {
            if f.e.oneway || f.e.source == f.e.target {
                continue;
            }
            if let Some(others) = by_key.get(&(f.e.target, f.e.source)) {
                let mine = (f.e.length_m, refs_key(&f.e));
                for &j in others {
                    if g[j].e.oneway {
                        continue;
                    }
                    let theirs = (g[j].e.length_m, refs_key(&g[j].e));
                    if mine.partial_cmp(&theirs) == Some(std::cmp::Ordering::Greater) {
                        sel.insert(i);
                    }
                }
            }
        }
        stats.antiparallel_split += sel.len();
        g = split_arcs(g, &sel, 4, vnodes);
    }

    // 5. Add the reverse edge of each two-way way (and of a one-way way with a bus lane).
    let mut out: Vec<Fwd> = Vec::with_capacity(2 * g.len());
    for f in g {
        let w = &mw[f.way];
        if !f.e.oneway || w.bus_back {
            let mut r = f.e.clone();
            r.source = f.e.target;
            r.target = f.e.source;
            r.lanes = w.lanes_bwd;
            if f.e.oneway {
                r.access = Some(Arc::from("bus"));
            }
            r.refs.reverse();
            r.geometry.reverse();
            r.is_reverse = true;
            r.ftype = if types { w.type_rev } else { None };
            out.push(Fwd {
                e: r,
                way: f.way,
                pos: f.pos,
            });
        }
        out.push(f);
    }

    // 6. The stable edge_id, edge_ref, and the check that (osm_id, source, target) is unique.
    if out.len() > 1 {
        let mut seen: FxHashSet<(i64, i64)> = FxHashSet::default();
        for f in &out {
            if !seen.insert((f.e.source, f.e.target)) {
                return Err(format!(
                    "duplicate (osm_id, source, target) after the splits: {} {}->{}",
                    f.e.osm_id, f.e.source, f.e.target
                ));
            }
        }
    }
    set_refs_and_ids(&mut out, raw_refs);

    // 7. Correct the length of the edges whose halves were wrong by more than 0.1 %.
    for f in &mut out {
        let m = geom::line_length_m(&f.e.geometry);
        if m > 0.0 && (((f.e.length_m / m) - 1.0f32).abs() as f64) > 0.001 {
            f.e.length_m = m;
        }
    }
    Ok(out)
}

/// Split the forward edges in `sel` at their midpoint with a virtual node (`_split_arcs_at_midpoint`). `stage` is the
/// step (3 or 4); the virtual nodes get it and the `Pos` of their edge, for the old order.
fn split_arcs(
    fwd: Vec<Fwd>,
    sel: &FxHashSet<usize>,
    stage: u8,
    vnodes: &mut Vec<(u8, Pos, Node)>,
) -> Vec<Fwd> {
    if sel.is_empty() {
        return fwd;
    }
    let add = if stage == 3 { 2 } else { 1 }; // see `Pos::half`
    let mut out = Vec::with_capacity(fwd.len() + sel.len());
    for (i, f) in fwd.into_iter().enumerate() {
        if !sel.contains(&i) {
            out.push(f);
            continue;
        }
        let rr = geom::remove_repeated(&f.e.geometry);
        if rr.len() < 2 {
            continue;
        }
        let v = hash::virtual_node_arc(f.e.osm_id, f.e.source, f.e.target, &f.e.refs);
        let mid = geom::interpolate(&rr, 0.5);
        let (r1, r2) = split_refs(&f.e.refs);
        vnodes.push((
            stage,
            f.pos,
            Node {
                node_id: v,
                lon: mid.0,
                lat: mid.1,
            },
        ));
        out.push(Fwd {
            e: half(&f.e, f.e.source, v, r1, geom::substring(&rr, 0.0, 0.5)),
            way: f.way,
            pos: f.pos,
        });
        out.push(Fwd {
            e: half(&f.e, v, f.e.target, r2, geom::substring(&rr, 0.5, 1.0)),
            way: f.way,
            pos: Pos {
                half: f.pos.half + add,
                ..f.pos
            },
        });
    }
    out
}

/// `edge_id` and `edge_ref` (`{osm_id}#{seq}{f|r}`). seq numbers the forward edges of a way by the
/// position of their source node in the original node list; a reverse edge takes the seq of its twin.
fn set_refs_and_ids(edges: &mut [Fwd], raw_refs: &FxHashMap<i64, &Vec<i64>>) {
    let mut keys: FxHashMap<i64, Vec<(usize, i64, i64)>> = FxHashMap::default();
    for e in edges.iter().map(|f| &f.e).filter(|e| !e.is_reverse) {
        let pos = raw_refs
            .get(&e.osm_id)
            .and_then(|r| r.iter().position(|&n| n == e.source))
            .map_or(2_000_000_000usize, |p| p + 1);
        keys.entry(e.osm_id)
            .or_default()
            .push((pos, e.source, e.target));
    }
    for v in keys.values_mut() {
        v.sort();
        v.dedup();
    }
    for e in edges.iter_mut().map(|f| &mut f.e) {
        e.edge_id = hash::edge_id(e.osm_id, e.source, e.target);
        let (s, t) = if e.is_reverse {
            (e.target, e.source)
        } else {
            (e.source, e.target)
        };
        let seq = keys.get(&e.osm_id).and_then(|v| {
            let pos = raw_refs
                .get(&e.osm_id)
                .and_then(|r| r.iter().position(|&n| n == s))
                .map_or(2_000_000_000usize, |p| p + 1);
            v.binary_search(&(pos, s, t)).ok().map(|i| i + 1)
        });
        e.seq = seq.map(|q| q as u32);
    }
}
fn prune_nodes(nodes: &mut Vec<Node>, edges: &[Edge]) {
    let used: FxHashSet<i64> = edges.iter().flat_map(|e| [e.source, e.target]).collect();
    nodes.retain(|n| used.contains(&n.node_id));
}

/// Keep the largest weakly connected component (union-find over the edge ends).
fn keep_largest(edges: &mut Vec<Edge>) {
    let mut id: FxHashMap<i64, usize> = FxHashMap::default();
    let mut parent: Vec<usize> = Vec::new();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut ends = Vec::with_capacity(edges.len());
    for e in edges.iter() {
        let mut get = |n: i64, parent: &mut Vec<usize>| {
            *id.entry(n).or_insert_with(|| {
                parent.push(parent.len());
                parent.len() - 1
            })
        };
        let a = get(e.source, &mut parent);
        let b = get(e.target, &mut parent);
        ends.push(a);
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent[ra] = rb;
        }
    }
    let mut sizes: FxHashMap<usize, usize> = FxHashMap::default();
    let roots: Vec<usize> = ends.iter().map(|&a| find(&mut parent, a)).collect();
    for &r in &roots {
        *sizes.entry(r).or_default() += 1;
    }
    let largest = *sizes
        .iter()
        .max_by_key(|(r, n)| (**n, std::cmp::Reverse(**r)))
        .unwrap()
        .0;
    let mut i = 0;
    edges.retain(|_| {
        i += 1;
        roots[i - 1] == largest
    });
}

/// Do two ways agree on every attribute that the merge compares? Before the merge, a forward edge has these values of
/// its way.
fn same_road(a: &ModeWay, b: &ModeWay) -> bool {
    a.attrs == b.attrs && a.oneway == b.oneway && a.lanes_fwd == b.lanes_fwd && a.access == b.access
}

/// Is the node of `run` (the ends of the forward edges at one node) a contraction node: real, exactly two half-edges
/// (after the junction rule), agreeing edges?
fn contraction(run: &[End], mw: &[ModeWay], gj: &FxHashMap<i64, bool>) -> bool {
    let node = run[0].node;
    if node <= 0 || gj.get(&node) == Some(&true) {
        return false;
    }
    let rows: Vec<&End> = run
        .iter()
        .filter(|h| {
            !(NON_ROAD.contains(&mw[h.way as usize].attrs.highway.as_str())
                && gj.contains_key(&node))
        })
        .collect();
    if rows.len() != 2 {
        return false;
    }
    let (a, b) = (&mw[rows[0].way as usize], &mw[rows[1].way as usize]);
    let one_in_one_out = rows[0].at_source != rows[1].at_source;
    if (a.oneway || b.oneway) && !one_in_one_out {
        return false;
    }
    same_road(a, b)
}

fn find(p: &mut [u32], mut x: u32) -> u32 {
    while p[x as usize] != x {
        p[x as usize] = p[p[x as usize] as usize];
        x = p[x as usize];
    }
    x
}

const SRC: u8 = 1;
const TGT: u8 = 2;

struct Chain {
    start: i64,
    end: i64,
    edges: Vec<usize>,
    refs: Vec<i64>,
}

/// Join each maximal chain of forward edges that are the same road into one edge (`_contract_chains`). The forward
/// edges are in the file `fwd`, and their ends in the file `ends`:
/// 1. the ends sorted by node give the contraction nodes; all edges at a contraction node join one set (a union-find on
///    the edge numbers), because the walk of step 3 follows each of them;
/// 2. the edges at no contraction node go to the result as they are, the others to a file sorted by their set;
/// 3. each set is merged in memory.
///
/// Returns the forward edges, the rows of `edge_id_map`, the number of chains and of pieces.
fn merge_chains(
    fwd: Rows,
    ends: Rows,
    mw: &[ModeWay],
    gj: &FxHashMap<i64, bool>,
    coords: &Coords,
    codec: &FwdCodec,
    store: &dyn Store,
) -> std::io::Result<(Rows, Vec<MapRow>, usize, usize)> {
    let n = fwd.len;
    let mut parent: Vec<u32> = (0..n as u32).collect();
    let mut at: Vec<u8> = vec![0; n]; // SRC, TGT: the source, the target of the edge is a contraction node
    let ends = spill::sort(ends, &EndCodec, |h: &End| h.node, SORT_BUDGET, store)?;
    for_groups(
        ends.read(&EndCodec)?,
        |h| h.node,
        |run| {
            if contraction(&run, mw, gj) {
                let r = find(&mut parent, run[0].edge);
                for h in &run {
                    at[h.edge as usize] |= if h.at_source { SRC } else { TGT };
                    let rh = find(&mut parent, h.edge);
                    parent[rh as usize] = r;
                }
            }
            Ok(())
        },
    )?;
    if at.iter().all(|&a| a == 0) {
        return Ok((fwd, Vec::new(), 0, 0));
    }
    let set: Vec<u32> = (0..n as u32).map(|i| find(&mut parent, i)).collect();
    drop(parent);

    let mut out = Writer::new(store)?;
    let mut in_sets = Writer::new(store)?;
    for f in fwd.read(codec)? {
        let f = f?;
        if at[f.pos.first as usize] == 0 {
            out.push(codec, &f)?;
        } else {
            in_sets.push(codec, &f)?;
        }
    }
    let key = |f: &Fwd| set[f.pos.first as usize];
    let in_sets = spill::sort(in_sets.finish()?, codec, key, SORT_BUDGET, store)?;
    let mut map: Vec<(MapRow, Pos, usize)> = Vec::new();
    let (mut n_chains, mut n_pieces) = (0, 0);
    for_groups(in_sets.read(codec)?, key, |g| {
        let (merged, pieces) = merge_set(g, &at, coords, &mut map);
        n_chains += merged.iter().filter(|f| f.pos.group == 0).count();
        n_pieces += pieces;
        for f in &merged {
            out.push(codec, f)?;
        }
        Ok(())
    })?;
    // the old order of the rows (the chains in the order of `Pos`, the rows of a chain in their order) breaks the ties
    map.sort_by_key(|(m, p, k)| (m.new_edge_id, m.seq, m.old_edge_id, *p, *k));
    Ok((
        out.finish()?,
        map.into_iter().map(|m| m.0).collect(),
        n_chains,
        n_pieces,
    ))
}

/// Merge the chains of one set of forward edges (in the order of the cut). Returns the merged chains and the edges that
/// are in no chain, and the number of pieces. The rows of `edge_id_map` go to `map`, with the `Pos` of their chain and
/// their order in it.
fn merge_set(
    set: Vec<Fwd>,
    at: &[u8],
    coords: &Coords,
    map: &mut Vec<(MapRow, Pos, usize)>,
) -> (Vec<Fwd>, usize) {
    let end_at = |f: &Fwd, src: bool| at[f.pos.first as usize] & if src { SRC } else { TGT } != 0;
    // the two half-edges of each edge at a contraction node, in the order of the edges, the source first
    let mut inc: FxHashMap<i64, Vec<(usize, bool)>> = FxHashMap::default();
    for (i, f) in set.iter().enumerate() {
        if end_at(f, true) {
            inc.entry(f.e.source).or_default().push((i, true));
        }
        if end_at(f, false) {
            inc.entry(f.e.target).or_default().push((i, false));
        }
    }
    let node2 = |n: &i64| inc.contains_key(n);
    let oriented = |i: usize, at_source: bool| -> Vec<i64> {
        let r = &set[i].e.refs;
        if at_source {
            r.clone()
        } else {
            r.iter().rev().copied().collect()
        }
    };
    let mut chains: Vec<Chain> = Vec::new();
    for (i, f) in set.iter().enumerate() {
        for (at_source, node, other) in [
            (true, f.e.source, f.e.target),
            (false, f.e.target, f.e.source),
        ] {
            if node2(&node) || !node2(&other) {
                continue;
            }
            // walk from this terminal through the contraction nodes
            let mut stack: Vec<(Vec<usize>, i64, Vec<i64>)> =
                vec![(vec![i], other, oriented(i, at_source))];
            while let Some((edges, cur, refs)) = stack.pop() {
                if !node2(&cur) {
                    let ok = edges.len() > 1
                        && node != cur
                        && if f.e.oneway { at_source } else { node < cur };
                    if ok {
                        chains.push(Chain {
                            start: node,
                            end: cur,
                            edges,
                            refs,
                        });
                    }
                    continue;
                }
                let last = *edges.last().unwrap();
                for &(j, j_at_source) in &inc[&cur] {
                    if j == last || edges.contains(&j) {
                        continue;
                    }
                    let mut e2 = edges.clone();
                    e2.push(j);
                    let mut r2 = refs.clone();
                    r2.extend(oriented(j, j_at_source).into_iter().skip(1));
                    let next = if j_at_source {
                        set[j].e.target
                    } else {
                        set[j].e.source
                    };
                    stack.push((e2, next, r2));
                }
            }
        }
    }
    let mut removed = vec![false; set.len()];
    for c in &chains {
        for &i in &c.edges {
            removed[i] = true;
        }
    }
    let n_pieces = removed.iter().filter(|&&r| r).count();
    let mut merged: Vec<Fwd> = Vec::with_capacity(set.len());
    for (walk, c) in chains.iter().enumerate() {
        let first = &set[c.edges[0]];
        let pos = Pos {
            group: 0,
            first: first.pos.first,
            start: c.start,
            end: c.end,
            walk: walk as u32,
            half: 0,
        };
        let geometry: Vec<Pt> = c
            .refs
            .iter()
            .filter_map(|n| coords.get(n).copied())
            .collect();
        if geometry.len() < 2 {
            continue;
        }
        let mut e = first.e.clone();
        e.source = c.start;
        e.target = c.end;
        e.refs = c.refs.clone();
        e.length_m = geom::line_length_m(&geometry);
        e.geometry = geometry;
        e.is_reverse = false;
        let new_fwd = hash::edge_id(e.osm_id, e.source, e.target);
        let new_rev = hash::edge_id(e.osm_id, e.target, e.source);
        for (k, &pi) in c.edges.iter().enumerate() {
            let p = &set[pi].e;
            let pos_of = |n: i64| c.refs.iter().position(|&x| x == n).map_or(0, |x| x + 1);
            let against = pos_of(p.source) > pos_of(p.target);
            let (a, b) = if pos_of(p.source) < pos_of(p.target) {
                (p.source, p.target)
            } else {
                (p.target, p.source)
            };
            let row = MapRow {
                old_edge_id: hash::edge_id(p.osm_id, a, b),
                new_edge_id: new_fwd,
                seq: k as i32 + 1,
                is_reverse: against,
                osm_id: p.osm_id,
            };
            map.push((row, pos, 2 * k));
            if !e.oneway {
                let row = MapRow {
                    old_edge_id: hash::edge_id(p.osm_id, b, a),
                    new_edge_id: new_rev,
                    seq: k as i32 + 1,
                    is_reverse: !against,
                    osm_id: p.osm_id,
                };
                map.push((row, pos, 2 * k + 1));
            }
        }
        merged.push(Fwd {
            e,
            way: first.way,
            pos,
        });
    }
    merged.extend(
        set.into_iter()
            .zip(removed)
            .filter(|(_, r)| !r)
            .map(|(f, _)| f),
    );
    (merged, n_pieces)
}
const PATH_CLASSES: [&str; 6] = [
    "cycleway",
    "footway",
    "path",
    "steps",
    "pedestrian",
    "track",
];

/// `PathConnector`: reconnect dangling path ends. Returns the number of connectors (each has two edges).
fn connect_paths(
    edges: &mut Vec<Edge>,
    nodes: &[Node],
    snap_m: f64,
    dismount: bool,
    types_for: Option<Mode>,
) -> usize {
    // 1. the undirected segments: (a, b) -> (smallest osm_id, a highway)
    let mut und: FxHashMap<(i64, i64), (i64, String)> = FxHashMap::default();
    for e in edges.iter() {
        let key = (e.source.min(e.target), e.source.max(e.target));
        und.entry(key)
            .and_modify(|v| v.0 = v.0.min(e.osm_id))
            .or_insert((e.osm_id, e.attrs.highway.clone()));
    }
    // 2. for each node: how many segments touch it, the other end and way of the last one, the smallest osm_id
    struct Touch {
        count: u32,
        other: i64,
        osm: i64,
        hw: String,
        min_osm: i64,
    }
    let mut touch: FxHashMap<i64, Touch> = FxHashMap::default();
    let mut keys: Vec<&(i64, i64)> = und.keys().collect();
    keys.sort();
    for &(a, b) in keys {
        let (osm, hw) = &und[&(a, b)];
        for (node, other) in [(a, b), (b, a)] {
            let t = touch.entry(node).or_insert(Touch {
                count: 0,
                other,
                osm: *osm,
                hw: hw.clone(),
                min_osm: *osm,
            });
            t.count += 1;
            t.other = other;
            t.osm = *osm;
            t.hw = hw.clone();
            t.min_osm = t.min_osm.min(*osm);
        }
    }
    let by_id: FxHashMap<i64, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.node_id, i))
        .collect();
    let mut dangles: Vec<(i64, &Touch)> = touch
        .iter()
        .filter(|(n, t)| {
            t.count == 1 && PATH_CLASSES.contains(&t.hw.as_str()) && by_id.contains_key(n)
        })
        .map(|(n, t)| (*n, t))
        .collect();
    dangles.sort_by_key(|d| d.0);
    if dangles.is_empty() {
        return 0;
    }
    // 3. the nearest node within snap_m (a local metric at the latitude of the dangle)
    let tree = RTree::bulk_load(
        nodes
            .iter()
            .enumerate()
            .map(|(i, n)| GeomWithData::new([n.lon, n.lat], i))
            .collect(),
    );
    let mut pairs: Vec<(i64, i64, String, f64, i64)> = Vec::new();
    let mut seen: FxHashSet<(i64, i64)> = FxHashSet::default();
    for (dnode, t) in dangles {
        let d = nodes[by_id[&dnode]];
        let coslat = d.lat.to_radians().cos();
        let coslat = if coslat == 0.0 { 1e-9 } else { coslat };
        let dlat = snap_m / 111320.0 * 1.5;
        let dlon = dlat / coslat.abs().max(1e-9);
        let mut best: Option<(f64, usize)> = None;
        for c in tree.locate_in_envelope(&AABB::from_corners(
            [d.lon - dlon, d.lat - dlat],
            [d.lon + dlon, d.lat + dlat],
        )) {
            let n = nodes[c.data];
            if n.node_id == dnode || n.node_id == t.other {
                continue;
            }
            let dist = ((n.lon - d.lon) * 111320.0 * coslat).hypot((n.lat - d.lat) * 111320.0);
            if best.is_none_or(|b| (dist, c.data) < b) {
                best = Some((dist, c.data));
            }
        }
        if let Some((dist, j)) = best {
            if dist <= snap_m {
                let target = nodes[j].node_id;
                let key = (dnode.min(target), dnode.max(target));
                if seen.insert(key) {
                    let other_osm = touch.get(&target).map_or(t.osm, |x| x.min_osm);
                    let osm_id = -(t.osm.min(other_osm));
                    pairs.push((
                        dnode,
                        target,
                        t.hw.clone(),
                        (dist * 100.0).round() / 100.0,
                        osm_id,
                    ));
                }
            }
        }
    }
    // 4. the connector edges, both directions
    let mut rank: FxHashMap<i64, Vec<(i64, i64)>> = FxHashMap::default();
    for p in &pairs {
        rank.entry(p.4)
            .or_default()
            .push((p.0.min(p.1), p.0.max(p.1)));
    }
    for v in rank.values_mut() {
        v.sort();
        v.dedup();
    }
    let first_new = edges.len();
    // Two edges for each pair. Without this, the first push into the full vector doubles its capacity: at the size of Estonia
    // walking (1.4 million edges of 160 bytes) it holds the old and the new block at one time (+400 MiB at the peak).
    edges.reserve_exact(pairs.len() * 2);
    for (a, b, hw, dist, osm_id) in &pairs {
        let seq = rank[osm_id].binary_search(&(*a.min(b), *a.max(b))).unwrap() + 1;
        let attrs = Arc::new(Attrs {
            highway: hw.clone(),
            name: None,
            maxspeed: None,
            surface: None,
            junction: None,
            layer: None,
            bridge: None,
            tunnel: None,
            service: None,
        });
        for (s, t, is_reverse) in [(*a, *b, false), (*b, *a, true)] {
            let (ns, nt) = (nodes[by_id[&s]], nodes[by_id[&t]]);
            edges.push(Edge {
                edge_id: hash::edge_id(*osm_id, s, t),
                seq: Some(seq as u32),
                source: s,
                target: t,
                osm_id: *osm_id,
                attrs: attrs.clone(),
                oneway: false,
                lanes: None,
                access: None,
                refs: vec![s, t],
                geometry: vec![(ns.lon, ns.lat), (nt.lon, nt.lat)],
                is_reverse,
                length_m: *dist as f32,
                dismount: if dismount { Some(false) } else { None },
                maxspeed_kmh: None,
                cost_s: None,
                // a connector has no way: the class comes from the highway only
                ftype: match types_for {
                    Some(Mode::Walking) => Some(filter::walk_type(hw, None)),
                    Some(Mode::Cycling) => Some(filter::cycle_type(hw, None, is_reverse)),
                    _ => None,
                },
            });
        }
    }
    // 5. a connector that extends a footway or pedestrian way that is walked is walked too
    if dismount {
        let walked: Vec<(String, i64, i64)> = edges[..first_new]
            .iter()
            .filter(|e| e.osm_id > 0 && e.dismount == Some(true))
            .map(|e| (e.attrs.highway.clone(), e.source, e.target))
            .collect();
        let mut by_node: FxHashMap<(&str, i64), ()> = FxHashMap::default();
        for (hw, s, t) in &walked {
            by_node.insert((hw.as_str(), *s), ());
            by_node.insert((hw.as_str(), *t), ());
        }
        let flags: Vec<bool> = edges[first_new..]
            .iter()
            .map(|e| {
                let hw = e.attrs.highway.as_str();
                matches!(hw, "footway" | "pedestrian")
                    && (by_node.contains_key(&(hw, e.source))
                        || by_node.contains_key(&(hw, e.target)))
            })
            .collect();
        for (e, f) in edges[first_new..].iter_mut().zip(flags) {
            e.dismount = Some(f);
        }
    }
    pairs.len()
}
