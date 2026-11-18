//! duckOSM `point_routing.py`: a point joins the network at its nearest road point within a radius (the straight walk to
//! it may not cross another road of the mode or a road for cars), and the route in one mode between two points.

use crate::geom::{self, Pt};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// A road meeting the access walk this close to its road point only touches it (duckOSM `TOUCH_M`).
pub const TOUCH_M: f64 = 0.5;
const KY: f64 = 111320.0;

/// A road near a point, as the database gives it (lon, lat).
#[derive(Clone, Debug)]
pub struct Road {
    pub edge_id: i64,
    pub source: i64,
    pub target: i64,
    pub cost_s: Option<f64>,
    pub length_m: Option<f64>,
    pub line: Vec<Pt>,
}

/// A road that a point can join (duckOSM `candidates`): `fraction` along it by length, `access_m` off it, `road_point`.
#[derive(Clone, Debug, PartialEq)]
pub struct Cand {
    pub edge_id: i64,
    pub source: i64,
    pub target: i64,
    pub cost_s: f64,
    pub length_m: f64,
    pub fraction: f64,
    pub access_m: f64,
    pub road_point: Pt,
}

fn scale(lat: f64) -> (f64, f64) {
    (KY * lat.to_radians().cos(), KY)
}

/// The box around `point` (lon, lat) that holds the roads within `radius_m`: (west, south, east, north).
pub fn bbox(point: Pt, radius_m: f64) -> (f64, f64, f64, f64) {
    let (kx, ky) = scale(point.1);
    let (dx, dy) = (radius_m / kx, radius_m / ky);
    (point.0 - dx, point.1 - dy, point.0 + dx, point.1 + dy)
}

/// duckOSM `candidates`: of `roads` (the roads of the mode in [`bbox`], in the order the database gives them), those within
/// `radius_m` that the point can walk straight to; `blockers` are the roads in the box that the walk may not cross (the
/// mode's own and the roads for cars).
pub fn candidates(point: Pt, radius_m: f64, roads: &[Road], blockers: &[Vec<Pt>]) -> Vec<Cand> {
    let (lon, lat) = point;
    let (kx, ky) = scale(lat);
    let local = |l: &[Pt]| -> Vec<Pt> { l.iter().map(|&(x, y)| ((x - lon) * kx, (y - lat) * ky)).collect() };
    let blockers: Vec<Vec<Pt>> = blockers.iter().map(|b| local(b)).collect();
    let mut out = Vec::new();
    for r in roads {
        let l = local(&r.line);
        let d = geom::distance(&l, (0.0, 0.0));
        let len = geom::length(&l);
        if d > radius_m || len == 0.0 {
            continue;
        }
        let at = geom::project(&l, (0.0, 0.0));
        let p = geom::interpolate(&l, at);
        let walk_len = (p.0 * p.0 + p.1 * p.1).sqrt();
        let crossed = walk_len > 0.0 && blockers.iter().any(|b| geom::crosses(p, b, TOUCH_M));
        if !crossed {
            out.push(Cand {
                edge_id: r.edge_id,
                source: r.source,
                target: r.target,
                cost_s: r.cost_s.unwrap_or(0.0),
                length_m: r.length_m.unwrap_or(0.0),
                fraction: at / len,
                access_m: d,
                road_point: (p.0 / kx + lon, p.1 / ky + lat),
            });
        }
    }
    out
}

/// A route between two points: its edges, the part of each that is travelled, and the start and end candidates.
#[derive(Clone, Debug)]
pub struct PointRoute {
    pub path: Vec<i64>,
    pub parts: Vec<(f64, f64)>,
    pub start: Cand,
    pub end: Cand,
}

#[derive(PartialEq)]
struct Key(f64, i64);
impl Eq for Key {}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        // Python heapq of (d, edge_id): the least first
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// duckOSM `route_points` (the search): `by_time` (else by length), `v` the walking speed (m/s) of the access walks,
/// `weight` each edge's cost or length (`edge_id -> value`, NULL as 0), `next` the legal turns (`from -> [to]`, in the
/// order of the edge graph). None when no pair of candidates is connected.
pub fn route_points(
    s_cands: &[Cand],
    t_cands: &[Cand],
    by_time: bool,
    v: f64,
    weight: &HashMap<i64, f64>,
    next: &HashMap<i64, Vec<i64>>,
) -> Option<PointRoute> {
    let key = |c: &Cand| if by_time { c.cost_s } else { c.length_m };
    let acc = |c: &Cand| if by_time { c.access_m / v } else { c.access_m };
    let mut tby: HashMap<i64, &Cand> = HashMap::new();
    for c in t_cands {
        tby.insert(c.edge_id, c);
    }
    enum Best<'a> {
        Same(f64, &'a Cand),
        Arrive(f64, i64),
    }
    let total_of = |b: &Best| match b {
        Best::Same(t, _) | Best::Arrive(t, _) => *t,
    };
    let mut best: Option<Best> = None;
    for s in s_cands {
        if let Some(t) = tby.get(&s.edge_id) {
            if s.fraction <= t.fraction {
                let total = acc(s) + (t.fraction - s.fraction) * key(s) + acc(t);
                if best.as_ref().is_none_or(|b| total < total_of(b)) {
                    best = Some(Best::Same(total, s));
                }
            }
        }
    }
    let mut lab: HashMap<i64, f64> = HashMap::new();
    let mut prev: HashMap<i64, Option<i64>> = HashMap::new();
    let mut start_of: HashMap<i64, &Cand> = HashMap::new();
    let mut heap = BinaryHeap::new();
    for s in s_cands {
        let d = acc(s) + (1.0 - s.fraction) * key(s);
        if lab.get(&s.edge_id).is_none_or(|&l| d < l) {
            lab.insert(s.edge_id, d);
            prev.insert(s.edge_id, None);
            start_of.insert(s.edge_id, s);
            heap.push(Key(d, s.edge_id));
        }
    }
    let mut arrive: HashMap<i64, f64> = HashMap::new();
    let mut arrive_from: HashMap<i64, i64> = HashMap::new();
    while let Some(Key(d, e)) = heap.pop() {
        if lab.get(&e).is_some_and(|&l| d > l) {
            continue;
        }
        if best.as_ref().is_some_and(|b| d >= total_of(b)) {
            break;
        }
        for &n in next.get(&e).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(t) = tby.get(&n) {
                if arrive.get(&n).is_none_or(|&a| d < a) {
                    arrive.insert(n, d);
                    arrive_from.insert(n, e);
                    let total = d + t.fraction * key(t) + acc(t);
                    if best.as_ref().is_none_or(|b| total < total_of(b)) {
                        best = Some(Best::Arrive(total, n));
                    }
                }
            }
            let nd = d + weight.get(&n).copied().unwrap_or(0.0);
            if lab.get(&n).is_none_or(|&l| nd < l) {
                lab.insert(n, nd);
                prev.insert(n, Some(e));
                heap.push(Key(nd, n));
            }
        }
    }
    match best? {
        Best::Same(_, s) => {
            let t = tby[&s.edge_id];
            Some(PointRoute { path: vec![s.edge_id], parts: vec![(s.fraction, t.fraction)], start: s.clone(), end: t.clone() })
        }
        Best::Arrive(_, last) => {
            let mut path = vec![last];
            let mut e = Some(arrive_from[&last]);
            while let Some(x) = e {
                path.push(x);
                e = prev[&x];
            }
            path.reverse();
            let (s, t) = (start_of[&path[0]], tby[&last]);
            let n = path.len();
            let parts = (0..n)
                .map(|i| (if i == 0 { s.fraction } else { 0.0 }, if i == n - 1 { t.fraction } else { 1.0 }))
                .collect();
            Some(PointRoute { path, parts, start: s.clone(), end: t.clone() })
        }
    }
}
