//! The network step for the driving mode: ways in, edges and nodes out. It follows
//! `GlobalJunctions`, `RoadFilter`, `GraphSimplifier`, `_split_private` and `ComponentFilter` of duckOSM.
//! Not in this version: `merge_segments`, OSM overrides, speeds, costs, turn restrictions, H3.

use crate::filter::{self, Mode, ModeWay, NON_ROAD};
use crate::fixes::OsmFixes;
use crate::geom::{self, Pt};
use crate::graph::{Graph, Relation};
use crate::hash;
use crate::model::{Attrs, Edge, MapRow, Network, Node, Stats, Way};
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
    let mut stats = Stats {
        ways_read: all_ways.len(),
        ..Default::default()
    };
    let gj = if opts.global_junctions {
        global_junctions(all_ways)
    } else {
        FxHashMap::default()
    };
    let mut mw: Vec<ModeWay> = all_ways
        .iter()
        .filter_map(|w| filter::mode_way(w, mode, opts.cycling_dismount))
        .collect();
    // duckOSM: after RoadFilter and before GraphBuilder, so oneway and the lanes act on the edges
    stats.osm_fixes = fixes.apply_to_mode(&mut mw, mode);
    stats.ways_kept = mw.len();
    let lj = local_junctions(&mw);

    // 1. Cut every way into forward edges. Step 2 runs on each edge as it is made, so the forward
    // edges are not copied to a second list.
    let mut fwd: Vec<Fwd> = Vec::new();
    let mut virtual_nodes: Vec<Node> = Vec::new();
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
            };
            if push_split_loop(&mut fwd, f, &mut virtual_nodes) {
                stats.self_loops_split += 1;
            }
        }
    }

    // 2b. Merge the chains of edges that are one road.
    let mut edge_id_map: Vec<MapRow> = Vec::new();
    if opts.merge_segments {
        let (merged, map, n_chains, n_pieces) = merge_chains(fwd, &gj, coords);
        fwd = merged;
        edge_id_map = map;
        stats.merged_chains = n_chains;
        stats.merged_pieces = n_pieces;
    }

    // 3. Split same-direction parallel arcs (all but the shortest of each group).
    let mut groups: FxHashMap<(i64, i64, i64), Vec<usize>> = FxHashMap::default();
    for (i, f) in fwd.iter().enumerate() {
        if f.e.source != f.e.target {
            groups
                .entry((f.e.osm_id, f.e.source, f.e.target))
                .or_default()
                .push(i);
        }
    }
    let mut sel = FxHashSet::default();
    for idx in groups.values().filter(|v| v.len() > 1) {
        let key = |i: usize| (fwd[i].e.length_m, refs_key(&fwd[i].e));
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
    stats.parallel_split = sel.len();
    fwd = split_arcs(fwd, &sel, &mut virtual_nodes);

    // 4. Split two-way antiparallel pairs (the longer arc of each pair).
    let mut by_key: FxHashMap<(i64, i64, i64), Vec<usize>> = FxHashMap::default();
    for (i, f) in fwd.iter().enumerate() {
        by_key
            .entry((f.e.osm_id, f.e.source, f.e.target))
            .or_default()
            .push(i);
    }
    let mut sel = FxHashSet::default();
    for (i, f) in fwd.iter().enumerate() {
        if f.e.oneway || f.e.source == f.e.target {
            continue;
        }
        if let Some(others) = by_key.get(&(f.e.osm_id, f.e.target, f.e.source)) {
            let mine = (f.e.length_m, refs_key(&f.e));
            for &j in others {
                if fwd[j].e.oneway {
                    continue;
                }
                let theirs = (fwd[j].e.length_m, refs_key(&fwd[j].e));
                if mine.partial_cmp(&theirs) == Some(std::cmp::Ordering::Greater) {
                    sel.insert(i);
                }
            }
        }
    }
    stats.antiparallel_split = sel.len();
    fwd = split_arcs(fwd, &sel, &mut virtual_nodes);

    // 5. Add the reverse edge of each two-way way (and of a one-way way with a bus lane).
    let has_rev = |f: &Fwd| !f.e.oneway || mw[f.way].bus_back;
    let mut rev: Vec<Edge> = Vec::with_capacity(fwd.iter().filter(|f| has_rev(f)).count());
    for f in &fwd {
        let w = &mw[f.way];
        if has_rev(f) {
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
            r.ftype = if opts.functional_types {
                w.type_rev
            } else {
                None
            };
            rev.push(r);
        }
    }
    let mut edges: Vec<Edge> = Vec::with_capacity(fwd.len() + rev.len());
    edges.extend(fwd.into_iter().map(|f| f.e));
    edges.extend(rev);

    // 6. The stable edge_id, edge_ref, and the check that (osm_id, source, target) is unique.
    let mut seen: FxHashSet<(i64, i64, i64)> = FxHashSet::default();
    for e in &edges {
        if !seen.insert((e.osm_id, e.source, e.target)) {
            return Err(format!(
                "duplicate (osm_id, source, target) after the splits: {} {}->{}",
                e.osm_id, e.source, e.target
            ));
        }
    }
    let raw_refs: FxHashMap<i64, &Vec<i64>> = all_ways.iter().map(|w| (w.id, &w.refs)).collect();
    set_refs_and_ids(&mut edges, &raw_refs);

    // 7. Correct the length of the edges whose halves were wrong by more than 0.1 %.
    for e in &mut edges {
        let m = geom::line_length_m(&e.geometry);
        if m > 0.0 && (((e.length_m / m) - 1.0f32).abs() as f64) > 0.001 {
            e.length_m = m;
        }
    }

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
    nodes.extend(virtual_nodes);

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

    // 8b2. Edges that are walked get the cycle_type 'dismount'.
    if opts.functional_types && mode == Mode::Cycling {
        for e in edges.iter_mut().filter(|e| e.dismount == Some(true)) {
            e.ftype = Some("dismount");
        }
    }

    // 8c. Speeds and travel times.
    if opts.calculate_costs && !opts.process_speeds {
        return Err("calculate_costs needs process_speeds".to_string());
    }
    if opts.process_speeds {
        crate::speeds::apply(&mut edges, mode, opts.calculate_costs);
    }

    // 9. Move private roads, bus-only roads and pedestrian streets to private_edges.
    let moved = |e: &Edge| {
        matches!(e.access.as_deref(), Some("private") | Some("bus"))
            || (mode == Mode::Driving && e.attrs.highway == "pedestrian")
    };
    // extract_if keeps the order, and it does not make a second copy of all edges like partition
    let private_edges: Vec<Edge> = edges.extract_if(.., |e| moved(e)).collect();
    stats.private_edges = private_edges.len();
    if !private_edges.is_empty() {
        prune_nodes(&mut nodes, &edges);
    }

    // 9b. Turn restrictions (driving) and the edge graph, before the component filter as in duckOSM.
    let mut graph = Graph::default();
    if opts.extract_restrictions && mode == Mode::Driving {
        let (tr, paths) =
            crate::graph::restrictions(relations, &fixes.turn_restrictions, &mw, &edges);
        graph.turn_restrictions = Some(tr);
        graph.turn_path_restrictions = Some(paths);
    }
    if opts.build_graph {
        let (rows, permission) = crate::graph::edge_graph(
            mode,
            &edges,
            &private_edges,
            graph.turn_restrictions.as_deref(),
        );
        graph.edge_graph = Some(rows);
        graph.turn_permission = Some(permission);
    }

    // 10. Keep the largest connected part. It also removes the graph rows of the dropped edges.
    if opts.keep_largest_component && !edges.is_empty() {
        let before = edges.len();
        let ids: FxHashSet<i64> = edges.iter().map(|e| e.edge_id).collect();
        keep_largest(&mut edges);
        stats.dropped_by_component_filter = before - edges.len();
        if before != edges.len() {
            prune_nodes(&mut nodes, &edges);
            let kept: FxHashSet<i64> = edges.iter().map(|e| e.edge_id).collect();
            graph.prune(&ids.difference(&kept).copied().collect());
        }
    }

    edges.sort_by_key(|e| e.edge_id);
    let mut private_edges = private_edges;
    private_edges.sort_by_key(|e| e.edge_id);
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
    });
    fwd.push(Fwd {
        e: half(&f.e, v, f.e.source, r2, geom::substring(&rr, 0.5, 1.0)),
        way: f.way,
    });
    true
}

/// Split the forward edges in `sel` at their midpoint with a virtual node (`_split_arcs_at_midpoint`).
fn split_arcs(fwd: Vec<Fwd>, sel: &FxHashSet<usize>, vnodes: &mut Vec<Node>) -> Vec<Fwd> {
    if sel.is_empty() {
        return fwd;
    }
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
        vnodes.push(Node {
            node_id: v,
            lon: mid.0,
            lat: mid.1,
        });
        out.push(Fwd {
            e: half(&f.e, f.e.source, v, r1, geom::substring(&rr, 0.0, 0.5)),
            way: f.way,
        });
        out.push(Fwd {
            e: half(&f.e, v, f.e.target, r2, geom::substring(&rr, 0.5, 1.0)),
            way: f.way,
        });
    }
    out
}

/// `edge_id` and `edge_ref` (`{osm_id}#{seq}{f|r}`). seq numbers the forward edges of a way by the
/// position of their source node in the original node list; a reverse edge takes the seq of its twin.
fn set_refs_and_ids(edges: &mut [Edge], raw_refs: &FxHashMap<i64, &Vec<i64>>) {
    let mut keys: FxHashMap<i64, Vec<(usize, i64, i64)>> = FxHashMap::default();
    for e in edges.iter().filter(|e| !e.is_reverse) {
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
    for e in edges.iter_mut() {
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
        let keep = roots[i] == largest;
        i += 1;
        keep
    });
}

/// Do two edges agree on every attribute that the merge compares?
fn same_road(a: &Edge, b: &Edge) -> bool {
    a.attrs == b.attrs && a.oneway == b.oneway && a.lanes == b.lanes && a.access == b.access
}

struct Chain {
    start: i64,
    end: i64,
    edges: Vec<usize>,
    refs: Vec<i64>,
}

/// Join each maximal chain of forward edges that are the same road into one edge (`_contract_chains`).
/// Returns the new forward edges, the rows of `edge_id_map`, the number of chains and of pieces.
fn merge_chains(
    fwd: Vec<Fwd>,
    gj: &FxHashMap<i64, bool>,
    coords: &Coords,
) -> (Vec<Fwd>, Vec<MapRow>, usize, usize) {
    // the two half-edges of each forward edge: (edge index, true at its source)
    let mut inc: FxHashMap<i64, Vec<(usize, bool)>> = FxHashMap::default();
    for (i, f) in fwd.iter().enumerate() {
        inc.entry(f.e.source).or_default().push((i, true));
        inc.entry(f.e.target).or_default().push((i, false));
    }
    // contraction nodes: real, exactly two half-edges (after the junction rule), agreeing edges
    let mut node2: FxHashSet<i64> = FxHashSet::default();
    for (&node, halves) in &inc {
        if node <= 0 || gj.get(&node) == Some(&true) {
            continue;
        }
        let rows: Vec<&(usize, bool)> = halves
            .iter()
            .filter(|(i, _)| {
                !(NON_ROAD.contains(&fwd[*i].e.attrs.highway.as_str()) && gj.contains_key(&node))
            })
            .collect();
        if rows.len() != 2 {
            continue;
        }
        let (a, b) = (&fwd[rows[0].0].e, &fwd[rows[1].0].e);
        let one_in_one_out = (rows[0].1 as u8 + rows[1].1 as u8) == 1;
        if (a.oneway || b.oneway) && !one_in_one_out {
            continue;
        }
        if same_road(a, b) {
            node2.insert(node);
        }
    }
    if node2.is_empty() {
        return (fwd, Vec::new(), 0, 0);
    }
    let oriented = |i: usize, at_source: bool| -> Vec<i64> {
        let r = &fwd[i].e.refs;
        if at_source {
            r.clone()
        } else {
            r.iter().rev().copied().collect()
        }
    };
    let mut chains: Vec<Chain> = Vec::new();
    for (i, f) in fwd.iter().enumerate() {
        for (at_source, node, other) in [
            (true, f.e.source, f.e.target),
            (false, f.e.target, f.e.source),
        ] {
            if node2.contains(&node) || !node2.contains(&other) {
                continue;
            }
            // walk from this terminal through the contraction nodes
            let mut stack: Vec<(Vec<usize>, i64, Vec<i64>)> =
                vec![(vec![i], other, oriented(i, at_source))];
            while let Some((edges, cur, refs)) = stack.pop() {
                if !node2.contains(&cur) {
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
                        fwd[j].e.target
                    } else {
                        fwd[j].e.source
                    };
                    stack.push((e2, next, r2));
                }
            }
        }
    }
    if chains.is_empty() {
        return (fwd, Vec::new(), 0, 0);
    }
    chains.sort_by_key(|c| (c.edges[0], c.start, c.end));
    let removed: FxHashSet<usize> = chains
        .iter()
        .flat_map(|c| c.edges.iter().copied())
        .collect();
    let n_pieces = removed.len();
    let mut map: Vec<MapRow> = Vec::new();
    let mut merged: Vec<Fwd> = Vec::with_capacity(fwd.len());
    for c in &chains {
        let first = &fwd[c.edges[0]];
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
            let p = &fwd[pi].e;
            let pos = |n: i64| c.refs.iter().position(|&x| x == n).map_or(0, |x| x + 1);
            let against = pos(p.source) > pos(p.target);
            let (a, b) = if pos(p.source) < pos(p.target) {
                (p.source, p.target)
            } else {
                (p.target, p.source)
            };
            map.push(MapRow {
                old_edge_id: hash::edge_id(p.osm_id, a, b),
                new_edge_id: new_fwd,
                seq: k as i32 + 1,
                is_reverse: against,
                osm_id: p.osm_id,
            });
            if !e.oneway {
                map.push(MapRow {
                    old_edge_id: hash::edge_id(p.osm_id, b, a),
                    new_edge_id: new_rev,
                    seq: k as i32 + 1,
                    is_reverse: !against,
                    osm_id: p.osm_id,
                });
            }
        }
        merged.push(Fwd { e, way: first.way });
    }
    let n_chains = merged.len();
    for (i, f) in fwd.into_iter().enumerate() {
        if !removed.contains(&i) {
            merged.push(f);
        }
    }
    map.sort_by_key(|m| (m.new_edge_id, m.seq, m.old_edge_id));
    (merged, map, n_chains, n_pieces)
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
