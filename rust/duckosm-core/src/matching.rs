//! Mobility data on the edges (the Python module `duckosm.mobility`): snap points to edges, match GPS traces to the
//! edges (map matching), and map the edge ids of one build to the edge ids of another build.
//!
//! The map matching is the hidden Markov model of Newson and Krumm (2009, "Hidden Markov map matching through noise
//! and sparseness"), solved with the Viterbi algorithm, as in leuvenmapmatching, Valhalla and GraphHopper:
//!
//! | Part | Rule |
//! |---|---|
//! | States | for each GPS point, the edges within `radius_m`, each at the point's projection on the edge |
//! | Emission | a normal distribution of the distance from the point to the edge, `sigma_m` |
//! | Transition | an exponential distribution of the difference between the route distance (along the edge directions) and the straight distance of the two points, `beta_m` |
//! | Break | when no state of a point can follow a state of the point before, the trace starts again at that point |
//!
//! The second matcher is GPS-DTW (the gps-dtw crate, [`match_trace_dtw`]): dynamic time warping of the trace,
//! interpolated each second, against the points of the road. It needs the time of each point.

use crate::analysis::Flat;
use rstar::primitives::{GeomWithData, Line};
use rstar::{PointDistance, RTree, AABB};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// One edge of a mode: its id, its ends (node ids), its length and its points (lon, lat).
#[derive(Clone, Debug)]
pub struct Road {
    pub edge_id: i64,
    pub osm_id: i64,
    pub source: i64,
    pub target: i64,
    pub length_m: f64,
    pub line: Vec<(f64, f64)>,
}

/// A point on an edge: the edge index, the share of the edge's length from its start (0 to 1), the distance from the
/// point in metres, and the point on the edge (lon, lat).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnEdge {
    pub edge: usize,
    pub fraction: f64,
    pub distance_m: f64,
    pub at: (f64, f64),
}

/// The edges of a mode in a search tree, with the node graph for the route distances.
pub struct Net<'a> {
    pub roads: &'a [Road],
    flat: Flat,
    tree: RTree<GeomWithData<Line<[f64; 2]>, (usize, usize)>>,
    /// metres of the line before each vertex, for each edge (flat)
    before: Vec<Vec<f64>>,
    node: HashMap<i64, usize>,
    out: Vec<Vec<usize>>,
    /// the edge of the other direction (target to source, same OSM way), for the U-turn cost
    twin: Vec<Option<usize>>,
}

fn unflat(flat: &Flat, p: [f64; 2]) -> (f64, f64) {
    let one = flat.at((1.0, 1.0));
    (p[0] / one[0], p[1] / one[1])
}

impl<'a> Net<'a> {
    pub fn new(roads: &'a [Road]) -> Self {
        let ys: Vec<f64> = roads.iter().flat_map(|r| r.line.iter().map(|p| p.1)).collect();
        let flat = Flat::new(if ys.is_empty() { 0.0 } else { ys.iter().sum::<f64>() / ys.len() as f64 });
        let mut items = Vec::new();
        let mut before = Vec::with_capacity(roads.len());
        let (mut node, mut out): (HashMap<i64, usize>, Vec<Vec<usize>>) = (HashMap::new(), Vec::new());
        for (i, r) in roads.iter().enumerate() {
            let mut acc = vec![0.0];
            for (k, w) in r.line.windows(2).enumerate() {
                let (a, b) = (flat.at(w[0]), flat.at(w[1]));
                acc.push(acc[k] + ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt());
                items.push(GeomWithData::new(Line::new(a, b), (i, k)));
            }
            before.push(acc);
            let n = out.len();
            let s = *node.entry(r.source).or_insert(n);
            if s == out.len() {
                out.push(Vec::new());
            }
            let n = out.len();
            if *node.entry(r.target).or_insert(n) == out.len() {
                out.push(Vec::new());
            }
            out[s].push(i);
        }
        let twin = roads
            .iter()
            .map(|r| {
                let back = &out[node[&r.target]];
                let rev = |e: &&usize| roads[**e].target == r.source;
                back.iter().filter(rev).find(|e| roads[**e].osm_id == r.osm_id).or_else(|| back.iter().find(rev)).copied()
            })
            .collect();
        Net { roads, flat, tree: RTree::bulk_load(items), before, node, out, twin }
    }

    fn on(&self, q: [f64; 2], edge: usize, k: usize) -> OnEdge {
        let r = &self.roads[edge];
        let (a, b) = (self.flat.at(r.line[k]), self.flat.at(r.line[k + 1]));
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 { (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let p = [a[0] + t * dx, a[1] + t * dy];
        let total = *self.before[edge].last().unwrap();
        let along = self.before[edge][k] + t * len2.sqrt();
        OnEdge {
            edge,
            fraction: if total > 0.0 { along / total } else { 0.0 },
            distance_m: ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt(),
            at: unflat(&self.flat, p),
        }
    }

    /// The nearest edge of a point, and the point on it. Edges within 1 mm of the nearest are equal: the first wins.
    pub fn nearest(&self, p: (f64, f64)) -> Option<OnEdge> {
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
        Some(self.on(q, best.0, best.1))
    }

    /// Each edge within `radius_m` of a point, at its nearest point to the point.
    pub fn within(&self, p: (f64, f64), radius_m: f64) -> Vec<OnEdge> {
        let q = self.flat.at(p);
        let env = AABB::from_corners([q[0] - radius_m, q[1] - radius_m], [q[0] + radius_m, q[1] + radius_m]);
        let mut best: HashMap<usize, (f64, usize)> = HashMap::new();
        for g in self.tree.locate_in_envelope_intersecting(&env) {
            let d = g.geom().distance_2(&q).sqrt();
            if d <= radius_m {
                let e = best.entry(g.data.0).or_insert((d, g.data.1));
                if d < e.0 {
                    *e = (d, g.data.1);
                }
            }
        }
        let mut out: Vec<OnEdge> = best.into_iter().map(|(e, (_, k))| self.on(q, e, k)).collect();
        out.sort_by(|a, b| a.edge.cmp(&b.edge));
        out
    }

    /// The shortest paths from the end of edge `from`, over the edges (Dijkstra on the edges, so that a U-turn, an
    /// edge followed by its other direction, costs `uturn_m` more anywhere on a path). For each edge reached: the metres
    /// from the end of `from` to the end of that edge, and the edge before it (None: `from`). Only edges that start
    /// within `limit` metres.
    fn search(&self, from: usize, limit: f64, uturn_m: f64) -> HashMap<usize, (f64, Option<usize>)> {
        let mut dist: HashMap<usize, (f64, Option<usize>)> = HashMap::new();
        let mut heap = BinaryHeap::new();
        let step = |e: usize, f: usize| self.roads[f].length_m + if self.twin[e] == Some(f) { uturn_m } else { 0.0 };
        for &f in &self.out[self.node[&self.roads[from].target]] {
            let nd = step(from, f);
            if dist.get(&f).is_none_or(|old| nd < old.0) {
                dist.insert(f, (nd, None));
                heap.push(Item(nd, f));
            }
        }
        while let Some(Item(d, e)) = heap.pop() {
            if d > dist[&e].0 || d > limit {
                continue;
            }
            for &f in &self.out[self.node[&self.roads[e].target]] {
                let nd = d + step(e, f);
                if dist.get(&f).is_none_or(|old| nd < old.0) {
                    dist.insert(f, (nd, Some(e)));
                    heap.push(Item(nd, f));
                }
            }
        }
        dist
    }
}

#[derive(PartialEq)]
struct Item(f64, usize);
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal).then(o.1.cmp(&self.1))
    }
}

/// The options of the map matching.
#[derive(Clone, Copy, Debug)]
pub struct MatchOptions {
    /// the edges within this distance of a point are its states
    pub radius_m: f64,
    /// the GPS error (the standard deviation of the emission)
    pub sigma_m: f64,
    /// the scale of the transition: a larger value allows routes that go further around
    pub beta_m: f64,
    /// a route between two points longer than `max_route_factor` × their distance + `radius_m` × 2 is not possible
    pub max_route_factor: f64,
    /// the extra metres of a U-turn (onto the other direction of the edge just driven), as in GraphHopper and Valhalla:
    /// without it, a noisy point next to a short dead end makes a route into the dead end and back
    pub uturn_m: f64,
}

impl Default for MatchOptions {
    fn default() -> Self {
        MatchOptions { radius_m: 50.0, sigma_m: 5.0, beta_m: 10.0, max_route_factor: 4.0, uturn_m: 50.0 }
    }
}

/// The options of GPS-DTW: `GpsDtwParams` of the gps-dtw crate (its `Default` gives its defaults).
pub use gps_dtw::core::GpsDtwParams as DtwOptions;

/// The network of GPS-DTW for the edges of `net`, in the same metres; `turns`: the allowed (from edge, to edge)
/// pairs as edge indexes (`edge_graph`), None: each edge that starts where an edge ends.
pub fn dtw_network(net: &Net, turns: Option<Vec<(usize, usize)>>) -> gps_dtw::core::Network {
    let shapes = net.roads.iter().map(|r| r.line.iter().map(|&p| net.flat.at(p)).collect()).collect();
    let from = net.roads.iter().map(|r| net.node[&r.source]).collect();
    let to = net.roads.iter().map(|r| net.node[&r.target]).collect();
    gps_dtw::core::Network::new(shapes, from, to, turns)
}

/// Match one trace with GPS-DTW: the route of [`gps_dtw::core::match_gps_dtw`], and for each point the nearest edge
/// of that route within `radius_m` (None: no route edge so near). `ts`: the time of each point in whole seconds,
/// in increasing order. `dtw`: [`dtw_network`] of `net`.
pub fn match_trace_dtw(net: &Net, dtw: &gps_dtw::core::Network, points: &[(f64, f64)], ts: &[i64], radius_m: f64, o: &DtwOptions) -> Result<Matched, String> {
    if points.is_empty() {
        return Ok(Matched::default());
    }
    let (x, y): (Vec<f64>, Vec<f64>) = points.iter().map(|&p| net.flat.at(p)).map(|q| (q[0], q[1])).unzip();
    let route = gps_dtw::core::match_gps_dtw(dtw, &x, &y, ts, o)?;
    let on: std::collections::HashSet<usize> = route.iter().copied().collect();
    let points = points
        .iter()
        .map(|&p| net.within(p, radius_m).into_iter().filter(|c| on.contains(&c.edge)).min_by(|a, b| a.distance_m.partial_cmp(&b.distance_m).unwrap_or(Ordering::Equal)))
        .collect();
    Ok(Matched { points, route })
}

/// The result of one trace: the edge of each point (None: no edge within the radius), and the edges of the route
/// through the trace, in order (an edge once for each pass).
#[derive(Clone, Debug, Default)]
pub struct Matched {
    pub points: Vec<Option<OnEdge>>,
    pub route: Vec<usize>,
}

fn straight(a: (f64, f64), b: (f64, f64)) -> f64 {
    crate::analysis::great_circle(a, b)
}

/// The route metres from state `a` to state `b` (None: not reached). `found` is the [`Net::search`] from `a`'s edge.
fn route_m(net: &Net, a: &OnEdge, b: &OnEdge, found: &HashMap<usize, (f64, Option<usize>)>) -> Option<f64> {
    let (ra, rb) = (&net.roads[a.edge], &net.roads[b.edge]);
    if a.edge == b.edge && b.fraction >= a.fraction - 1e-9 {
        return Some((b.fraction - a.fraction).max(0.0) * ra.length_m);
    }
    let (to_end, _) = found.get(&b.edge)?;
    Some((1.0 - a.fraction) * ra.length_m + to_end - (1.0 - b.fraction) * rb.length_m)
}

/// The edges between edge `a` and edge `b` on the path of `found` (both not included).
fn between(found: &HashMap<usize, (f64, Option<usize>)>, b: usize) -> Vec<usize> {
    let mut edges = Vec::new();
    let mut e = found.get(&b).and_then(|x| x.1);
    while let Some(p) = e {
        if edges.len() > found.len() {
            break; // a loop: never for the paths of a search
        }
        edges.push(p);
        e = found.get(&p).and_then(|x| x.1);
    }
    edges.reverse();
    edges
}

/// Match one trace (points in time order) to the edges: Newson and Krumm's hidden Markov model, Viterbi.
pub fn match_trace(net: &Net, points: &[(f64, f64)], o: &MatchOptions) -> Matched {
    let cands: Vec<Vec<OnEdge>> = points.iter().map(|&p| net.within(p, o.radius_m)).collect();
    let emit = |c: &OnEdge| -0.5 * (c.distance_m / o.sigma_m).powi(2);
    // the Viterbi lattice: for each point, the score and the back pointer of each state
    let mut score: Vec<Vec<f64>> = Vec::with_capacity(points.len());
    let mut back: Vec<Vec<Option<usize>>> = Vec::with_capacity(points.len());
    let mut prev: Option<usize> = None;                  // the last point with states
    for (t, cs) in cands.iter().enumerate() {
        let mut sc = vec![f64::NEG_INFINITY; cs.len()];
        let mut bk = vec![None; cs.len()];
        if let Some(p) = prev {
            let d = straight(points[p], points[t]);
            let limit = d * o.max_route_factor + 2.0 * o.radius_m + o.uturn_m;
            let mut found_of: HashMap<usize, HashMap<usize, (f64, Option<usize>)>> = HashMap::new();
            for (i, a) in cands[p].iter().enumerate() {
                if score[p][i] == f64::NEG_INFINITY {
                    continue;
                }
                let found = found_of.entry(a.edge).or_insert_with(|| net.search(a.edge, limit, o.uturn_m));
                for (j, b) in cs.iter().enumerate() {
                    if let Some(r) = route_m(net, a, b, found).filter(|&r| r <= limit) {
                        let s = score[p][i] - (r - d).abs() / o.beta_m + emit(b);
                        if s > sc[j] {
                            sc[j] = s;
                            bk[j] = Some(i);
                        }
                    }
                }
            }
        }
        if sc.iter().all(|s| *s == f64::NEG_INFINITY) {
            // the first point, or a break: the trace starts again here
            sc = cs.iter().map(emit).collect();
            bk = vec![None; cs.len()];
        }
        if !cs.is_empty() {
            prev = Some(t);
        }
        score.push(sc);
        back.push(bk);
    }
    // back from the best state of the last point; a state with no back pointer starts a piece
    let mut chosen: Vec<Option<usize>> = vec![None; points.len()];
    let mut t = points.len();
    let mut next: Option<usize> = None;
    while t > 0 {
        t -= 1;
        if cands[t].is_empty() {
            continue;
        }
        let j = match next {
            Some(j) => j,
            None => (0..cands[t].len()).max_by(|&a, &b| score[t][a].partial_cmp(&score[t][b]).unwrap_or(Ordering::Equal)).unwrap(),
        };
        chosen[t] = Some(j);
        next = back[t][j];
    }
    let pts: Vec<Option<OnEdge>> = (0..points.len()).map(|t| chosen[t].map(|j| cands[t][j])).collect();
    // the route: the edges of each point, and the shortest path between two points on different edges
    let mut route: Vec<usize> = Vec::new();
    let mut last: Option<(usize, OnEdge)> = None;
    for (t, p) in pts.iter().enumerate() {
        let Some(b) = p else { continue };
        if let Some((s, a)) = last {
            let joined = back[t][chosen[t].unwrap()].is_some();
            let ahead = a.edge == b.edge && b.fraction >= a.fraction - 1e-9;
            if joined && !ahead {
                // the path of the Viterbi step: the same search, the same limit
                let limit = straight(points[s], points[t]) * o.max_route_factor + 2.0 * o.radius_m + o.uturn_m;
                route.extend(between(&net.search(a.edge, limit, o.uturn_m), b.edge));
                route.push(b.edge);
            }
        }
        if route.last() != Some(&b.edge) {
            route.push(b.edge);
        }
        last = Some((t, *b));
    }
    Matched { points: pts, route }
}

/// The edge ids of an old build on the edges of a new build: for each old edge, the new edges of the same OSM way
/// that lie on it in the same direction, with the share of the old edge on each. `tol_m`: the largest distance
/// between the two lines. An old edge id that is in the new build maps to itself with share 1.
pub fn edge_id_map(old: &[Road], new: &[Road], tol_m: f64) -> Vec<(i64, i64, f64)> {
    let ids: std::collections::HashSet<i64> = new.iter().map(|r| r.edge_id).collect();
    let mut by_osm: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, r) in new.iter().enumerate() {
        by_osm.entry(r.osm_id).or_default().push(i);
    }
    let net = Net::new(new);
    let flat = net.flat;
    const SAMPLES: usize = 10;
    let mut out = Vec::new();
    for r in old {
        if ids.contains(&r.edge_id) {
            out.push((r.edge_id, r.edge_id, 1.0));
            continue;
        }
        let Some(group) = by_osm.get(&r.osm_id) else { continue };
        let line: Vec<[f64; 2]> = r.line.iter().map(|&p| flat.at(p)).collect();
        let mut acc = vec![0.0];
        for w in line.windows(2) {
            acc.push(acc.last().unwrap() + ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt());
        }
        let total = *acc.last().unwrap();
        let mut count: HashMap<usize, usize> = HashMap::new();
        for s in 0..SAMPLES {
            // a point and the direction of the old edge at the middle of each tenth
            let want = total * (s as f64 + 0.5) / SAMPLES as f64;
            let k = acc.windows(2).position(|w| want <= w[1]).unwrap_or(line.len().saturating_sub(2));
            if line.len() < 2 {
                break;
            }
            let (a, b) = (line[k], line[k + 1]);
            let seg = acc[k + 1] - acc[k];
            let t = if seg > 0.0 { (want - acc[k]) / seg } else { 0.0 };
            let q = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            let dir = [b[0] - a[0], b[1] - a[1]];
            let mut best: Option<(f64, usize)> = None;
            for &e in group {
                let nl = &new[e].line;
                for w in nl.windows(2) {
                    let (c, d) = (flat.at(w[0]), flat.at(w[1]));
                    if (d[0] - c[0]) * dir[0] + (d[1] - c[1]) * dir[1] <= 0.0 {
                        continue; // the other direction
                    }
                    let dist = Line::new(c, d).distance_2(&q).sqrt();
                    if dist <= tol_m && best.is_none_or(|b| dist < b.0) {
                        best = Some((dist, e));
                    }
                }
            }
            if let Some((_, e)) = best {
                *count.entry(e).or_default() += 1;
            }
        }
        let mut rows: Vec<(i64, i64, f64)> = count.into_iter().map(|(e, c)| (r.edge_id, new[e].edge_id, c as f64 / SAMPLES as f64)).collect();
        rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(Ordering::Equal).then(a.1.cmp(&b.1)));
        out.extend(rows);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-way street 0-1-2 along the equator (two rows each), and a side street 1-3 to the north.
    fn roads() -> Vec<Road> {
        let p = |x: f64, y: f64| (x * 0.001, y * 0.001);
        let mut v = Vec::new();
        let mut add = |id: i64, osm: i64, s: i64, t: i64, a: (f64, f64), b: (f64, f64)| {
            v.push(Road { edge_id: id, osm_id: osm, source: s, target: t, length_m: crate::analysis::great_circle(a, b), line: vec![a, b] });
        };
        add(1, 10, 0, 1, p(0.0, 0.0), p(1.0, 0.0));
        add(2, 10, 1, 0, p(1.0, 0.0), p(0.0, 0.0));
        add(3, 10, 1, 2, p(1.0, 0.0), p(2.0, 0.0));
        add(4, 10, 2, 1, p(2.0, 0.0), p(1.0, 0.0));
        add(5, 20, 1, 3, p(1.0, 0.0), p(1.0, 1.0));
        add(6, 20, 3, 1, p(1.0, 1.0), p(1.0, 0.0));
        v
    }

    #[test]
    fn matches_a_trace_in_its_direction() {
        let r = roads();
        let net = Net::new(&r);
        // east along the street, then north up the side street; about 2 m off the lines
        let pts: Vec<(f64, f64)> = [(0.1, 0.02), (0.4, -0.02), (0.7, 0.02), (0.95, 0.02), (1.02, 0.3), (0.98, 0.6), (1.02, 0.9)]
            .iter()
            .map(|&(x, y)| (x * 0.001, y * 0.001))
            .collect();
        let m = match_trace(&net, &pts, &MatchOptions::default());
        let route: Vec<i64> = m.route.iter().map(|&e| r[e].edge_id).collect();
        assert_eq!(route, vec![1, 5]);
        assert_eq!(r[m.points[0].unwrap().edge].edge_id, 1);
        // back the other way: the reverse rows
        let back: Vec<(f64, f64)> = pts.iter().rev().copied().collect();
        let m = match_trace(&net, &back, &MatchOptions::default());
        assert_eq!(m.route.iter().map(|&e| r[e].edge_id).collect::<Vec<_>>(), vec![6, 2]);
    }

    #[test]
    fn gps_dtw_matches_the_same_trace() {
        let r = roads();
        let net = Net::new(&r);
        let dtw = dtw_network(&net, None);
        let pts: Vec<(f64, f64)> = [(0.1, 0.02), (0.4, -0.02), (0.7, 0.02), (0.95, 0.02), (1.02, 0.3), (0.98, 0.6), (1.02, 0.9)]
            .iter()
            .map(|&(x, y)| (x * 0.001, y * 0.001))
            .collect();
        let ts: Vec<i64> = (0..pts.len() as i64).map(|i| i * 3).collect();
        let m = match_trace_dtw(&net, &dtw, &pts, &ts, 50.0, &DtwOptions::default()).unwrap();
        assert_eq!(m.route.iter().map(|&e| r[e].edge_id).collect::<Vec<_>>(), vec![1, 5]);
        assert_eq!(r[m.points[0].unwrap().edge].edge_id, 1);
        assert_eq!(r[m.points[6].unwrap().edge].edge_id, 5);
    }

    /// A noisy point next to a 5.5 m dead end: the route does not go into the dead end and back (the U-turn cost).
    #[test]
    fn no_detour_into_a_dead_end() {
        let mut r = roads();
        let (a, b) = ((0.001, 0.0), (0.001, -0.00005));
        let len = crate::analysis::great_circle(a, b);
        r.push(Road { edge_id: 7, osm_id: 30, source: 1, target: 4, length_m: len, line: vec![a, b] });
        r.push(Road { edge_id: 8, osm_id: 30, source: 4, target: 1, length_m: len, line: vec![b, a] });
        let net = Net::new(&r);
        let pts: Vec<(f64, f64)> =
            [(0.2, 0.02), (0.6, -0.02), (1.003, -0.1), (1.4, 0.02), (1.8, -0.02)].iter().map(|&(x, y)| (x * 0.001, y * 0.001)).collect();
        let m = match_trace(&net, &pts, &MatchOptions::default());
        assert_eq!(m.route.iter().map(|&e| r[e].edge_id).collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn maps_split_edges() {
        let old = roads();
        // the new build splits edge 1 (0 -> 1) at its middle: edges 11 and 12; the others keep their ids
        let mut new: Vec<Road> = old.iter().filter(|r| r.edge_id != 1).cloned().collect();
        let (a, m, b) = ((0.0, 0.0), (0.0005, 0.0), (0.001, 0.0));
        new.push(Road { edge_id: 11, osm_id: 10, source: 0, target: 9, length_m: 55.0, line: vec![a, m] });
        new.push(Road { edge_id: 12, osm_id: 10, source: 9, target: 1, length_m: 55.0, line: vec![m, b] });
        let map = edge_id_map(&old, &new, 1.0);
        let one: Vec<_> = map.iter().filter(|r| r.0 == 1).collect();
        assert_eq!(one, vec![&(1, 11, 0.5), &(1, 12, 0.5)]);
        assert!(map.contains(&(2, 2, 1.0)));
    }
}
