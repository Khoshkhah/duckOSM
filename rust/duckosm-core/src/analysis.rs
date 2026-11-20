//! Street network analysis (the Python module `duckosm.analysis`): the numbers of OSMnx `basic_stats`, the street count
//! of each node, the bearings and the orientation entropy, the connected components and the nearest node or edge of a
//! point. The input is the edges and the nodes of one mode; the functions do not read a database.
//!
//! Two views of a mode (docs/reference/python-modules.md, "Decisions inside the modules"):
//!
//! | View | Nodes and edges |
//! |---|---|
//! | duckOSM | every row of `<mode>.nodes` and `<mode>.edges` |
//! | OSMnx | [`simplify`]: the nodes that OSMnx's `simplify_graph` removes are removed, and their edges are joined |
//!
//! The rules copy OSMnx 2.0 (`osmnx.stats`, `osmnx.simplification`, `osmnx.bearing`), so the numbers of the OSMnx view
//! are the numbers of `ox.basic_stats(ox.simplify_graph(G))` for the graph `G` of `duckosm.analysis.to_osmnx`.

use rstar::primitives::{GeomWithData, Line};
use rstar::RTree;
use std::collections::{HashMap, HashSet};

/// OSMnx's radius of the earth (`osmnx.distance.EARTH_RADIUS_M`).
pub const EARTH_RADIUS_M: f64 = 6_371_009.0;

/// One edge row of a mode.
#[derive(Clone, Debug)]
pub struct Edge {
    pub edge_id: i64,
    pub osm_id: i64,
    pub source: i64,
    pub target: i64,
    pub length_m: f64,
    /// The points (lon, lat) of the geometry.
    pub line: Vec<(f64, f64)>,
}

/// One node row of a mode: its id and its point (lon, lat).
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub id: i64,
    pub x: f64,
    pub y: f64,
}

/// An edge of a view: node indexes, the OSM ids (sorted, each once), the length and the shape that OSMnx compares to
/// find the two rows of one street, and the networkx key.
#[derive(Clone, Debug)]
pub struct Seg {
    pub u: usize,
    pub v: usize,
    /// The networkx key: 0, 1, ... for the edges from `u` to `v`, in their order.
    pub key: usize,
    pub osm: Vec<i64>,
    pub length: f64,
    pub shape: Vec<(f64, f64)>,
}

/// A view: the points of all nodes, which nodes are in it, and its edges.
#[derive(Clone, Debug)]
pub struct View {
    pub xy: Vec<(f64, f64)>,
    pub keep: Vec<bool>,
    pub segs: Vec<Seg>,
}

/// The duckOSM view: every node and edge. An edge whose end is not in `nodes` adds that end as a node at the end of
/// its line. `ids[i]` is the node id of index `i`.
pub fn duckosm_view(nodes: &[Node], edges: &[Edge]) -> (Vec<i64>, View) {
    let mut ids: Vec<i64> = nodes.iter().map(|n| n.id).collect();
    let mut xy: Vec<(f64, f64)> = nodes.iter().map(|n| (n.x, n.y)).collect();
    let mut at: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut index = |id: i64, p: (f64, f64), ids: &mut Vec<i64>, xy: &mut Vec<(f64, f64)>| -> usize {
        *at.entry(id).or_insert_with(|| {
            ids.push(id);
            xy.push(p);
            ids.len() - 1
        })
    };
    let mut segs = Vec::with_capacity(edges.len());
    for e in edges {
        let first = e.line.first().copied().unwrap_or((f64::NAN, f64::NAN));
        let last = e.line.last().copied().unwrap_or(first);
        let u = index(e.source, first, &mut ids, &mut xy);
        let v = index(e.target, last, &mut ids, &mut xy);
        segs.push(Seg { u, v, key: 0, osm: vec![e.osm_id], length: e.length_m, shape: e.line.clone() });
    }
    set_keys(&mut segs, 0);
    let keep = vec![true; xy.len()];
    (ids, View { xy, keep, segs })
}

/// networkx `new_edge_key` for the edges from `from` on: the first free key of each (u, v).
fn set_keys(segs: &mut [Seg], from: usize) {
    let mut used: HashMap<(usize, usize), HashSet<usize>> = HashMap::new();
    for s in &segs[..from] {
        used.entry((s.u, s.v)).or_default().insert(s.key);
    }
    for s in &mut segs[from..] {
        let k = used.entry((s.u, s.v)).or_default();
        let mut key = k.len();
        while k.contains(&key) {
            key += 1;
        }
        k.insert(key);
        s.key = key;
    }
}

fn uniq(v: impl Iterator<Item = usize>) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for x in v {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// The OSMnx view of a view: OSMnx 2.0 `simplify_graph` with its defaults (`remove_rings=True`, lengths summed). A node
/// stays (an "endpoint") when it has a self-loop, no edge in or no edge out, or not exactly 2 different neighbouring
/// nodes with 2 or 4 edges. The edges through the other nodes are joined into one edge between two endpoints.
pub fn simplify(view: &View) -> View {
    let n = view.xy.len();
    let mut out_e: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_e: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, s) in view.segs.iter().enumerate() {
        out_e[s.u].push(i);
        in_e[s.v].push(i);
    }
    let succ = |a: usize| uniq(out_e[a].iter().map(|&i| view.segs[i].v));
    let endpoint: Vec<bool> = (0..n)
        .map(|a| {
            if !view.keep[a] {
                return false;
            }
            let nb = uniq(out_e[a].iter().map(|&i| view.segs[i].v).chain(in_e[a].iter().map(|&i| view.segs[i].u)));
            let d = out_e[a].len() + in_e[a].len();
            nb.contains(&a) || out_e[a].is_empty() || in_e[a].is_empty() || !(nb.len() == 2 && (d == 2 || d == 4))
        })
        .collect();
    // osmnx.simplification._build_path
    let build_path = |e: usize, s: usize| -> Vec<usize> {
        let mut path = vec![e, s];
        let mut seen: HashSet<usize> = path.iter().copied().collect();
        if let Some(t) = succ(s).into_iter().find(|t| !seen.contains(t)) {
            path.push(t);
            seen.insert(t);
            let mut cur = t;
            while !endpoint[cur] {
                let next: Vec<usize> = succ(cur).into_iter().filter(|x| !seen.contains(x)).collect();
                match next.len() {
                    1 => {
                        cur = next[0];
                        path.push(cur);
                        seen.insert(cur);
                    }
                    0 => {
                        if succ(cur).contains(&e) {
                            path.push(e);
                        }
                        return path;
                    }
                    _ => return path,
                }
            }
        }
        path
    };
    let first_edge = |a: usize, b: usize| out_e[a].iter().copied().find(|&i| view.segs[i].v == b);
    let mut removed = vec![false; n];
    let mut merged = Vec::new();
    for e in (0..n).filter(|&a| endpoint[a]) {
        for s in succ(e) {
            if endpoint[s] {
                continue;
            }
            let path = build_path(e, s);
            let (mut length, mut osm) = (0.0, Vec::new());
            for w in path.windows(2) {
                if let Some(i) = first_edge(w[0], w[1]) {
                    length += view.segs[i].length;
                    osm.extend_from_slice(&view.segs[i].osm);
                }
            }
            osm.sort_unstable();
            osm.dedup();
            for &k in &path[1..path.len() - 1] {
                removed[k] = true;
            }
            let shape = path.iter().map(|&k| view.xy[k]).collect();
            merged.push(Seg { u: path[0], v: *path.last().unwrap(), key: 0, osm, length, shape });
        }
    }
    let mut keep: Vec<bool> = (0..n).map(|a| view.keep[a] && !removed[a]).collect();
    let mut segs: Vec<Seg> = view.segs.iter().filter(|s| keep[s.u] && keep[s.v]).cloned().collect();
    let kept = segs.len();
    segs.extend(merged);
    set_keys(&mut segs, kept);
    // osmnx.simplification._remove_rings: a weak component with no endpoint goes
    let mut uf = UnionFind::new(n);
    for s in &segs {
        uf.join(s.u, s.v);
    }
    let mut has_end = vec![false; n];
    for a in (0..n).filter(|&a| keep[a] && endpoint[a]) {
        has_end[uf.find(a)] = true;
    }
    for a in 0..n {
        if keep[a] && !has_end[uf.find(a)] {
            keep[a] = false;
        }
    }
    segs.retain(|s| keep[s.u] && keep[s.v]);
    View { xy: view.xy.clone(), keep, segs }
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind((0..n).collect())
    }
    fn find(&mut self, mut a: usize) -> usize {
        while self.0[a] != a {
            self.0[a] = self.0[self.0[a]];
            a = self.0[a];
        }
        a
    }
    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.0[a] = b;
    }
}

/// OSMnx `count_streets_per_node`: for each node of the view, the streets that meet at it. Between two nodes, the
/// streets are the larger of the two counts of edges (a two-way road: 1); a node with a self-loop gets 2 more.
pub fn streets_per_node(view: &View) -> Vec<usize> {
    let mut pairs: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    let mut looped = vec![false; view.xy.len()];
    for s in &view.segs {
        if s.u == s.v {
            looped[s.u] = true;
            continue;
        }
        let c = pairs.entry((s.u.min(s.v), s.u.max(s.v))).or_default();
        if s.u < s.v {
            c.0 += 1
        } else {
            c.1 += 1
        }
    }
    let mut spn = vec![0usize; view.xy.len()];
    for (&(a, b), &(f, r)) in &pairs {
        spn[a] += f.max(r);
        spn[b] += f.max(r);
    }
    for (a, l) in looped.iter().enumerate() {
        if *l {
            spn[a] += 2;
        }
    }
    spn
}

fn shape_bits(s: &[(f64, f64)]) -> Vec<u64> {
    s.iter().flat_map(|p| [p.0.to_bits(), p.1.to_bits()]).collect()
}

/// OSMnx `to_undirected`: one edge of each street, in OSMnx's two steps. 1: the edges between the same two nodes
/// with the same key share one slot of networkx's MultiGraph, and the last one stays; when their shapes differ, OSMnx
/// gives the first one a new key (a slot of its own). 2: two slots with the same OSM ids and the same shape (or the
/// reverse shape) are one street. Gives the index of the edge that stays for each street.
pub fn undirected(view: &View) -> Vec<usize> {
    let canon = |s: &Seg| {
        let fwd = shape_bits(&s.shape);
        let rev: Vec<(f64, f64)> = s.shape.iter().rev().copied().collect();
        fwd.min(shape_bits(&rev))
    };
    // the order of networkx's edges: by the node u, then as added
    let mut order: Vec<usize> = (0..view.segs.len()).collect();
    order.sort_by_key(|&i| (view.segs[i].u, i));
    let mut groups: HashMap<(usize, usize, usize), Vec<usize>> = HashMap::new();
    for &i in &order {
        let s = &view.segs[i];
        groups.entry((s.u.min(s.v), s.u.max(s.v), s.key)).or_default().push(i);
    }
    // in key order, so that the same edge stays at each run
    let mut groups: Vec<((usize, usize, usize), Vec<usize>)> = groups.into_iter().collect();
    groups.sort_unstable_by_key(|g| g.0);
    let mut slots: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for ((a, b, _), list) in groups {
        let shapes: Vec<Vec<u64>> = list.iter().map(|&i| canon(&view.segs[i])).collect();
        let mut rest = &list[..];
        if shapes.iter().any(|x| *x != shapes[0]) {
            slots.entry((a, b)).or_default().push(list[0]);
            rest = &list[1..];
        }
        if let Some(&last) = rest.last() {
            slots.entry((a, b)).or_default().push(last);
        }
    }
    let mut out = Vec::new();
    for list in slots.values() {
        let mut seen: HashSet<(Vec<i64>, Vec<u64>)> = HashSet::new();
        for &i in list {
            if seen.insert((view.segs[i].osm.clone(), canon(&view.segs[i]))) {
                out.push(i);
            }
        }
    }
    out.sort_unstable();
    out
}

/// OSMnx `great_circle`, in metres.
pub fn great_circle(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (y1, y2) = (a.1.to_radians(), b.1.to_radians());
    let (dy, dx) = (y2 - y1, b.0.to_radians() - a.0.to_radians());
    let h = ((dy / 2.0).sin().powi(2) + y1.cos() * y2.cos() * (dx / 2.0).sin().powi(2)).min(1.0);
    2.0 * h.sqrt().asin() * EARTH_RADIUS_M
}

/// OSMnx `calculate_bearing`: the compass bearing from `a` to `b`, 0 to 360 degrees.
pub fn bearing(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat1, lat2) = (a.1.to_radians(), b.1.to_radians());
    let dlon = (b.0 - a.0).to_radians();
    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

/// The numbers of OSMnx `basic_stats` (the keys of its result). The densities need the area.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub n: usize,
    pub m: usize,
    pub k_avg: f64,
    pub edge_length_total: f64,
    pub edge_length_avg: f64,
    pub streets_per_node_avg: f64,
    pub streets_per_node_counts: Vec<usize>,
    pub streets_per_node_proportions: Vec<f64>,
    pub intersection_count: usize,
    pub street_length_total: f64,
    pub street_segment_count: usize,
    pub street_length_avg: f64,
    pub circuity_avg: Option<f64>,
    pub self_loop_proportion: f64,
    /// node, intersection, edge and street density per km² (with an area)
    pub densities: Option<[f64; 4]>,
}

/// OSMnx `basic_stats` of a view. `spn_all`: the streets of each node (index as `view.xy`). OSMnx stores them on the
/// nodes (`street_count`) before it simplifies the graph, so give the [`streets_per_node`] of the duckOSM view for both
/// views. `area_m2`: the area for the densities.
pub fn stats(view: &View, spn_all: &[usize], area_m2: Option<f64>) -> Stats {
    let nodes: Vec<usize> = (0..view.xy.len()).filter(|&a| view.keep[a]).collect();
    let (n, m) = (nodes.len(), view.segs.len());
    let spn: Vec<usize> = nodes.iter().map(|&a| spn_all[a]).collect();
    let max = spn.iter().copied().max().unwrap_or(0);
    let mut counts = vec![0usize; max + 1];
    for &c in &spn {
        counts[c] += 1;
    }
    let div = |a: f64, b: usize| if b == 0 { f64::NAN } else { a / b as f64 };
    let edge_length_total: f64 = view.segs.iter().map(|s| s.length).sum();
    let streets = undirected(view);
    let street_length_total: f64 = streets.iter().map(|&i| view.segs[i].length).sum();
    let straight: f64 = streets.iter().map(|&i| great_circle(view.xy[view.segs[i].u], view.xy[view.segs[i].v])).filter(|d| !d.is_nan()).sum();
    let loops = streets.iter().filter(|&&i| view.segs[i].u == view.segs[i].v).count();
    let intersection_count = spn.iter().filter(|&&c| c >= 2).count();
    let densities = area_m2.map(|a| {
        let km2 = a / 1e6;
        [n as f64 / km2, intersection_count as f64 / km2, edge_length_total / km2, street_length_total / km2]
    });
    Stats {
        n,
        m,
        k_avg: div(2.0 * m as f64, n),
        edge_length_total,
        edge_length_avg: div(edge_length_total, m),
        streets_per_node_avg: div(spn.iter().sum::<usize>() as f64, n),
        streets_per_node_proportions: counts.iter().map(|&c| div(c as f64, n)).collect(),
        streets_per_node_counts: counts,
        intersection_count,
        street_length_total,
        street_segment_count: streets.len(),
        street_length_avg: div(street_length_total, streets.len()),
        circuity_avg: if straight > 0.0 { Some(street_length_total / straight) } else { None },
        self_loop_proportion: div(loops as f64, streets.len()),
        densities,
    }
}

/// OSMnx `orientation_entropy` and the bins of `plot_orientation` on the streets of a view (both directions of each
/// street). `weight_by_length`: weight each bearing by the length (OSMnx `weight="length"`). Gives the count of each of
/// `num_bins` bins (the first is centred on north) and the entropy (natural log).
pub fn orientation(view: &View, num_bins: usize, min_length: f64, weight_by_length: bool) -> (Vec<f64>, f64) {
    let split = num_bins * 2;
    let width = 360.0 / split as f64;
    let mut bins = vec![0.0f64; split];
    for i in undirected(view) {
        let s = &view.segs[i];
        if s.u == s.v || s.length < min_length {
            continue;
        }
        let b = bearing(view.xy[s.u], view.xy[s.v]);
        if b.is_nan() {
            continue;
        }
        let w = if weight_by_length { s.length } else { 1.0 };
        for x in [b, (b - 180.0).rem_euclid(360.0)] {
            bins[((x / width) as usize).min(split - 1)] += w;
        }
    }
    bins.rotate_right(1);
    let counts: Vec<f64> = (0..num_bins).map(|j| bins[2 * j] + bins[2 * j + 1]).collect();
    let total: f64 = counts.iter().sum();
    let entropy = if total > 0.0 { -counts.iter().filter(|&&c| c > 0.0).map(|&c| (c / total) * (c / total).ln()).sum::<f64>() } else { f64::NAN };
    (counts, entropy)
}

/// The connected components of a view: one label for each node, 0 for the largest component, 1 for the next, ...
/// `strong`: strongly connected (each node reaches each other along the edges' directions); else weakly connected.
pub fn components(view: &View, strong: bool) -> Vec<usize> {
    let n = view.xy.len();
    let raw: Vec<usize> = if strong {
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for s in &view.segs {
            adj[s.u].push(s.v);
        }
        tarjan(&adj)
    } else {
        let mut uf = UnionFind::new(n);
        for s in &view.segs {
            uf.join(s.u, s.v);
        }
        (0..n).map(|a| uf.find(a)).collect()
    };
    // relabel by size: the largest is 0; equal sizes keep the order of their first node
    let mut size: HashMap<usize, (usize, usize)> = HashMap::new();
    for (a, &c) in raw.iter().enumerate() {
        size.entry(c).or_insert((0, a)).0 += 1;
    }
    let mut order: Vec<(usize, usize, usize)> = size.into_iter().map(|(c, (k, first))| (c, k, first)).collect();
    order.sort_by(|x, y| y.1.cmp(&x.1).then(x.2.cmp(&y.2)));
    let label: HashMap<usize, usize> = order.iter().enumerate().map(|(i, x)| (x.0, i)).collect();
    raw.iter().map(|c| label[c]).collect()
}

/// Tarjan's strongly connected components, without recursion. A component id for each node.
fn tarjan(adj: &[Vec<usize>]) -> Vec<usize> {
    let n = adj.len();
    const NONE: usize = usize::MAX;
    let (mut index, mut low, mut comp) = (vec![NONE; n], vec![0usize; n], vec![NONE; n]);
    let (mut on, mut stack, mut next, mut ncomp) = (vec![false; n], Vec::new(), 0usize, 0usize);
    for root in 0..n {
        if index[root] != NONE {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on[root] = true;
        while let Some(&mut (v, ref mut k)) = call.last_mut() {
            if *k < adj[v].len() {
                let w = adj[v][*k];
                *k += 1;
                if index[w] == NONE {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on[w] = true;
                    call.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(p, _)) = call.last() {
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        comp[w] = ncomp;
                        if w == v {
                            break;
                        }
                    }
                    ncomp += 1;
                }
            }
        }
    }
    comp
}

/// Metres east and north of (0, 0) on a flat map with the scale of the WGS84 ellipsoid at latitude `lat0` (the area is
/// small, so the error is small: under 0.1 % in 50 km).
#[derive(Clone, Copy, Debug)]
pub struct Flat {
    kx: f64,
    ky: f64,
}

impl Flat {
    pub fn new(lat0: f64) -> Self {
        const A: f64 = 6_378_137.0;
        const E2: f64 = 6.694_379_990_141_316e-3;
        let s2 = lat0.to_radians().sin().powi(2);
        let w = 1.0 - E2 * s2;
        // the radius of the meridian (north) and of the prime vertical (east)
        let (m, n) = (A * (1.0 - E2) / w.powf(1.5), A / w.sqrt());
        Flat { kx: (n * lat0.to_radians().cos()).to_radians(), ky: m.to_radians() }
    }
    pub fn at(&self, p: (f64, f64)) -> [f64; 2] {
        [p.0 * self.kx, p.1 * self.ky]
    }
}

/// The nearest node to each point. Gives (the node index, the distance in metres) for each point.
pub struct NearestNodes {
    flat: Flat,
    tree: RTree<GeomWithData<[f64; 2], usize>>,
}

impl NearestNodes {
    pub fn new(points: &[(f64, f64)], flat: Flat) -> Self {
        let items = points.iter().enumerate().filter(|(_, p)| !p.0.is_nan()).map(|(i, &p)| GeomWithData::new(flat.at(p), i)).collect();
        NearestNodes { flat, tree: RTree::bulk_load(items) }
    }
    pub fn query(&self, p: (f64, f64)) -> Option<(usize, f64)> {
        let q = self.flat.at(p);
        self.tree.nearest_neighbor(&q).map(|g| {
            let c = g.geom();
            (g.data, ((c[0] - q[0]).powi(2) + (c[1] - q[1]).powi(2)).sqrt())
        })
    }
}

/// The nearest edge to each point: the edge whose line is the nearest. Equal distances (the two rows of a two-way
/// road) give the edge that comes first.
pub struct NearestEdges {
    flat: Flat,
    tree: RTree<GeomWithData<Line<[f64; 2]>, usize>>,
}

impl NearestEdges {
    pub fn new(lines: &[Vec<(f64, f64)>], flat: Flat) -> Self {
        let mut items = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            for w in l.windows(2) {
                items.push(GeomWithData::new(Line::new(flat.at(w[0]), flat.at(w[1])), i));
            }
        }
        NearestEdges { flat, tree: RTree::bulk_load(items) }
    }
    /// (the edge index, the distance in metres). Lines within 1 mm of the nearest one are equal: the first edge wins.
    pub fn query(&self, p: (f64, f64)) -> Option<(usize, f64)> {
        let q = self.flat.at(p);
        let mut it = self.tree.nearest_neighbor_iter_with_distance_2(&q);
        let (g, d0) = it.next()?;
        let mut best = g.data;
        for (g, d2) in it {
            if d2.sqrt() > d0.sqrt() + 1e-3 {
                break;
            }
            best = best.min(g.data);
        }
        Some((best, d0.sqrt()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(u: usize, v: usize, osm: i64, xy: &[(f64, f64)]) -> Seg {
        Seg { u, v, key: 0, osm: vec![osm], length: 1.0, shape: vec![xy[u], xy[v]] }
    }

    /// A two-way road a-b-c (b only splits it) and a one-way spur b->d. b has 3 neighbours: it stays; a two-way road
    /// through a node that has 2 neighbours goes in the OSMnx view.
    #[test]
    fn views_and_streets() {
        let xy = vec![(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (1.0, 1.0)];
        // 0 - 1 - 2 - 3 two-way, 1 -> 4 one-way: 1 is a junction, 2 is not
        let segs = vec![
            seg(0, 1, 1, &xy),
            seg(1, 0, 1, &xy),
            seg(1, 2, 1, &xy),
            seg(2, 1, 1, &xy),
            seg(2, 3, 2, &xy),
            seg(3, 2, 2, &xy),
            seg(1, 4, 3, &xy),
        ];
        let segs = segs.into_iter().map(|mut s| { if s.u > s.v { s.shape.reverse(); } s }).collect::<Vec<_>>();
        let v = View { xy: xy.clone(), keep: vec![true; 5], segs };
        assert_eq!(streets_per_node(&v), vec![1, 3, 2, 1, 1]);
        let spn = streets_per_node(&v);
        let s = stats(&v, &spn, None);
        assert_eq!((s.n, s.m, s.street_segment_count, s.intersection_count), (5, 7, 4, 2));
        let o = simplify(&v);
        let so = stats(&o, &spn, None);
        assert_eq!((so.n, so.m, so.street_segment_count, so.intersection_count), (4, 5, 3, 1));
        assert!(o.segs.iter().any(|s| s.u == 1 && s.v == 3 && s.osm == vec![1, 2] && s.length == 2.0));
    }

    #[test]
    fn strong_components() {
        let xy = vec![(0.0, 0.0); 4];
        let segs = vec![seg(0, 1, 1, &xy), seg(1, 0, 1, &xy), seg(1, 2, 2, &xy), seg(2, 3, 3, &xy), seg(3, 2, 3, &xy)];
        let v = View { xy, keep: vec![true; 4], segs };
        let c = components(&v, true);
        assert_eq!(c[0], c[1]);
        assert_eq!(c[2], c[3]);
        assert_ne!(c[0], c[2]);
        assert_eq!(components(&v, false), vec![0, 0, 0, 0]);
    }

    #[test]
    fn bearings_and_entropy() {
        assert!((bearing((0.0, 0.0), (0.0, 1.0)) - 0.0).abs() < 1e-9);
        assert!((bearing((0.0, 0.0), (1.0, 0.0)) - 90.0).abs() < 1e-9);
        // a grid: all bearings in 4 bins of 36, entropy ln 4
        let xy = vec![(0.0, 0.0), (0.0, 0.001), (0.001, 0.0)];
        let v = View { xy: xy.clone(), keep: vec![true; 3], segs: vec![seg(0, 1, 1, &xy), seg(0, 2, 2, &xy)] };
        let (counts, h) = orientation(&v, 36, 0.0, false);
        assert_eq!(counts.iter().filter(|&&c| c > 0.0).count(), 4);
        assert!((h - 4f64.ln()).abs() < 1e-9);
    }

    #[test]
    fn nearest() {
        let flat = Flat::new(0.0);
        let lines = vec![vec![(0.0, 0.0), (0.001, 0.0)], vec![(0.0, 0.001), (0.001, 0.001)]];
        let ne = NearestEdges::new(&lines, flat);
        assert_eq!(ne.query((0.0005, 0.0002)).unwrap().0, 0);
        assert_eq!(ne.query((0.0005, 0.0009)).unwrap().0, 1);
        let nn = NearestNodes::new(&[(0.0, 0.0), (1.0, 1.0)], flat);
        assert_eq!(nn.query((0.9, 0.8)).unwrap().0, 1);
    }
}
