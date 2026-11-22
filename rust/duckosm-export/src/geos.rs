//! The buffer polygon of a line, in pure Rust: the port of the parts of GEOS 3.13.1 that `BufferOp` uses for a line with
//! the full precision of its input (`BufferBuilder::buffer`). These are:
//!
//! - `CGAlgorithmsDD` (double-double orientation and intersection point) and `LineIntersector`;
//! - the noder of the buffer: `MCIndexNoder` with an `IntersectionAdder`, and `SegmentNodeList` (the split edges);
//! - `BufferBuilder::insertUniqueEdge` (equal edges merge, their depth deltas add);
//! - the planar graph (`geomgraph`): nodes, directed edges sorted around each node, the depths (`BufferSubgraph`,
//!   `RightmostEdgeFinder`, `DirectedEdgeStar::computeDepths`) and the result edges;
//! - `buffer::PolygonBuilder`: the maximal and minimal edge rings, shells and holes.
//!
//! The order of the steps and of each loop is as in GEOS, so the rings have the same points in the same order. Where
//! GEOS throws a `TopologyException`, `buffer_polygons` tries again with less precision, as `BufferOp`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};

pub type P = (f64, f64);

// ---------------------------------------------------------------------------------------------------------------
// double-double arithmetic (geos::math::DD)

#[derive(Clone, Copy)]
struct DD {
    hi: f64,
    lo: f64,
}

const SPLIT: f64 = 134217729.0;

impl DD {
    fn new(x: f64) -> DD {
        DD { hi: x, lo: 0.0 }
    }
    fn add(self, y: DD) -> DD {
        let (hi, lo, yhi, ylo) = (self.hi, self.lo, y.hi, y.lo);
        let s0 = hi + yhi;
        let t0 = lo + ylo;
        let mut e = s0 - hi;
        let f = t0 - lo;
        let mut s = s0 - e;
        let mut t = t0 - f;
        s = (yhi - e) + (hi - s);
        t = (ylo - f) + (lo - t);
        e = s + t0;
        let hh = s0 + e;
        let h = e + (s0 - hh);
        e = t + h;
        let zhi = hh + e;
        DD { hi: zhi, lo: e + (hh - zhi) }
    }
    fn sub(self, y: DD) -> DD {
        self.add(DD { hi: -y.hi, lo: -y.lo })
    }
    fn mul(self, y: DD) -> DD {
        let (hi, lo, yhi, ylo) = (self.hi, self.lo, y.hi, y.lo);
        let mut cc = SPLIT * hi;
        let mut hx = cc - hi;
        let mut c = SPLIT * yhi;
        hx = cc - hx;
        let tx = hi - hx;
        let mut hy = c - yhi;
        cc = hi * yhi;
        hy = c - hy;
        let ty = yhi - hy;
        c = ((((hx * hy - cc) + hx * ty) + tx * hy) + tx * ty) + (hi * ylo + lo * yhi);
        let zhi = cc + c;
        hx = cc - zhi;
        DD { hi: zhi, lo: c + hx }
    }
    fn div(self, y: DD) -> DD {
        let (hi, lo, yhi, ylo) = (self.hi, self.lo, y.hi, y.lo);
        let cc = hi / yhi;
        let mut c = SPLIT * cc;
        let mut hc = c - cc;
        let mut u = SPLIT * yhi;
        hc = c - hc;
        let tc = cc - hc;
        let mut hy = u - yhi;
        let uu = cc * yhi;
        hy = u - hy;
        let ty = yhi - hy;
        u = (((hc * hy - uu) + hc * ty) + tc * hy) + tc * ty;
        c = ((((hi - uu) - u) + lo) - cc * ylo) / yhi;
        u = cc + c;
        DD { hi: u, lo: (cc - u) + c }
    }
    fn value(self) -> f64 {
        self.hi + self.lo
    }
    fn signum(self) -> i32 {
        if self.hi < 0.0 || (self.hi == 0.0 && self.lo < 0.0) {
            -1
        } else if self.hi > 0.0 || (self.hi == 0.0 && self.lo > 0.0) {
            1
        } else {
            0
        }
    }
}

/// The orientation of q to the segment p1-p2: counter-clockwise 1, clockwise -1, collinear 0
/// (`CGAlgorithmsDD::orientationIndex`).
pub fn orient(p1: P, p2: P, q: P) -> i32 {
    let detleft = (p1.0 - q.0) * (p2.1 - q.1);
    let detright = (p1.1 - q.1) * (p2.0 - q.0);
    let det = detleft - detright;
    let sign = |d: f64| if d > 0.0 { 1 } else if d < 0.0 { -1 } else { 0 };
    let detsum = if detleft > 0.0 {
        if detright <= 0.0 {
            return sign(det);
        }
        detleft + detright
    } else if detleft < 0.0 {
        if detright >= 0.0 {
            return sign(det);
        }
        -detleft - detright
    } else {
        return sign(det);
    };
    let errbound = 1e-15 * detsum;
    if det >= errbound || -det >= errbound {
        return sign(det);
    }
    let dx1 = DD::new(p2.0).add(DD::new(-p1.0));
    let dy1 = DD::new(p2.1).add(DD::new(-p1.1));
    let dx2 = DD::new(q.0).add(DD::new(-p2.0));
    let dy2 = DD::new(q.1).add(DD::new(-p2.1));
    dx1.mul(dy2).sub(dy1.mul(dx2)).signum()
}

/// `Coordinate::distance`.
pub fn dist(a: P, b: P) -> f64 {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    (dx * dx + dy * dy).sqrt()
}

/// `Distance::pointToSegment`.
pub fn point_to_segment(p: P, a: P, b: P) -> f64 {
    if a == b {
        return dist(p, a);
    }
    let len2 = (b.0 - a.0) * (b.0 - a.0) + (b.1 - a.1) * (b.1 - a.1);
    let r = ((p.0 - a.0) * (b.0 - a.0) + (p.1 - a.1) * (b.1 - a.1)) / len2;
    if r <= 0.0 {
        return dist(p, a);
    }
    if r >= 1.0 {
        return dist(p, b);
    }
    let s = ((a.1 - p.1) * (b.0 - a.0) - (a.0 - p.0) * (b.1 - a.1)) / len2;
    s.abs() * len2.sqrt()
}

/// `CGAlgorithmsDD::intersection`: the point where the lines p1-p2 and q1-q2 cross (None: parallel).
fn dd_intersection(p1: P, p2: P, q1: P, q2: P) -> Option<P> {
    let (p1x, p1y, p2x, p2y) = (DD::new(p1.0), DD::new(p1.1), DD::new(p2.0), DD::new(p2.1));
    let (q1x, q1y, q2x, q2y) = (DD::new(q1.0), DD::new(q1.1), DD::new(q2.0), DD::new(q2.1));
    let px = p1y.sub(p2y);
    let py = p2x.sub(p1x);
    let pw = p1x.mul(p2y).sub(p2x.mul(p1y));
    let qx = q1y.sub(q2y);
    let qy = q2x.sub(q1x);
    let qw = q1x.mul(q2y).sub(q2x.mul(q1y));
    let x = py.mul(qw).sub(qy.mul(pw));
    let y = qx.mul(pw).sub(px.mul(qw));
    let w = px.mul(qy).sub(qx.mul(py));
    let (xi, yi) = (x.div(w).value(), y.div(w).value());
    (xi.is_finite() && yi.is_finite()).then_some((xi, yi))
}

/// `Envelope::intersects(p1, p2, q)`.
fn env_has(p1: P, p2: P, q: P) -> bool {
    q.0 >= p1.0.min(p2.0) && q.0 <= p1.0.max(p2.0) && q.1 >= p1.1.min(p2.1) && q.1 <= p1.1.max(p2.1)
}

/// `Envelope::intersects(p1, p2, q1, q2)`.
fn envs_meet(p1: P, p2: P, q1: P, q2: P) -> bool {
    !(p1.0.min(p2.0) > q1.0.max(q2.0)
        || p1.0.max(p2.0) < q1.0.min(q2.0)
        || p1.1.min(p2.1) > q1.1.max(q2.1)
        || p1.1.max(p2.1) < q1.1.min(q2.1))
}

/// `LineIntersector::nearestEndpoint`.
fn nearest_endpoint(p1: P, p2: P, q1: P, q2: P) -> P {
    let mut best = p1;
    let mut min = point_to_segment(p1, q1, q2);
    for (p, a, b) in [(p2, q1, q2), (q1, p1, p2), (q2, p1, p2)] {
        let d = point_to_segment(p, a, b);
        if d < min {
            min = d;
            best = p;
        }
    }
    best
}

/// The result of `LineIntersector::computeIntersection` for two segments: 0, 1 or 2 points (2: collinear).
struct Li {
    n: usize,
    pts: [P; 2],
}

/// `LineIntersector::computeIntersect` and `computeCollinearIntersection`, with no precision model.
fn line_intersection(p1: P, p2: P, q1: P, q2: P) -> Li {
    let none = Li { n: 0, pts: [(0.0, 0.0); 2] };
    let one = |p: P| Li { n: 1, pts: [p, (0.0, 0.0)] };
    if !envs_meet(p1, p2, q1, q2) {
        return none;
    }
    let pq1 = orient(p1, p2, q1);
    let pq2 = orient(p1, p2, q2);
    if (pq1 > 0 && pq2 > 0) || (pq1 < 0 && pq2 < 0) {
        return none;
    }
    let qp1 = orient(q1, q2, p1);
    let qp2 = orient(q1, q2, p2);
    if (qp1 > 0 && qp2 > 0) || (qp1 < 0 && qp2 < 0) {
        return none;
    }
    if pq1 == 0 && pq2 == 0 && qp1 == 0 && qp2 == 0 {
        let (q1in, q2in, p1in, p2in) = (env_has(p1, p2, q1), env_has(p1, p2, q2), env_has(q1, q2, p1), env_has(q1, q2, p2));
        let two = |a: P, b: P, point: bool| if point { one(a) } else { Li { n: 2, pts: [a, b] } };
        if q1in && q2in {
            return two(q1, q2, false);
        }
        if p1in && p2in {
            return two(p1, p2, false);
        }
        if q1in && p1in {
            return two(q1, p1, q1 == p1 && !q2in && !p2in);
        }
        if q1in && p2in {
            return two(q1, p2, q1 == p2 && !q2in && !p1in);
        }
        if q2in && p1in {
            return two(q2, p1, q2 == p1 && !q1in && !p2in);
        }
        if q2in && p2in {
            return two(q2, p2, q2 == p2 && !q1in && !p1in);
        }
        return none;
    }
    if pq1 == 0 || pq2 == 0 || qp1 == 0 || qp2 == 0 {
        let p = if p1 == q1 || p1 == q2 {
            p1
        } else if p2 == q1 || p2 == q2 {
            p2
        } else if pq1 == 0 {
            q1
        } else if pq2 == 0 {
            q2
        } else if qp1 == 0 {
            p1
        } else {
            p2
        };
        return one(p);
    }
    // a proper intersection: LineIntersector::intersection
    let mut p = dd_intersection(p1, p2, q1, q2).unwrap_or_else(|| nearest_endpoint(p1, p2, q1, q2));
    if !(env_has(p1, p2, p) && env_has(q1, q2, p)) {
        p = nearest_endpoint(p1, p2, q1, q2);
    }
    one(p)
}

/// The number of points of `LineIntersector::computeIntersection` of the segments p1-p2 and q1-q2 (2: they overlap).
pub fn intersection_count(p1: P, p2: P, q1: P, q2: P) -> usize {
    line_intersection(p1, p2, q1, q2).n
}

/// The points of `LineIntersector::computeIntersection` of the segments p1-p2 and q1-q2 (none, one, or two where they overlap).
pub fn intersection_points(p1: P, p2: P, q1: P, q2: P) -> Vec<P> {
    let li = line_intersection(p1, p2, q1, q2);
    li.pts[..li.n].to_vec()
}

/// The point p is inside the ring or on it (`RayCrossingCounter`, not EXTERIOR).
pub fn in_or_on_ring(p: P, ring: &[P]) -> bool {
    in_ring(p, ring)
}

/// The first point of `LineIntersector::computeIntersection` of the segments p1-p2 and q1-q2, if they meet.
pub fn segment_intersection(p1: P, p2: P, q1: P, q2: P) -> Option<P> {
    let li = line_intersection(p1, p2, q1, q2);
    (li.n > 0).then_some(li.pts[0])
}

// ---------------------------------------------------------------------------------------------------------------
// noding (MCIndexNoder, IntersectionAdder, SegmentNodeList)

/// `Quadrant::quadrant`: NE 0, NW 1, SW 2, SE 3.
fn quadrant(dx: f64, dy: f64) -> i32 {
    match (dx >= 0.0, dy >= 0.0) {
        (true, true) => 0,
        (true, false) => 3,
        (false, true) => 1,
        (false, false) => 2,
    }
}

/// `Octant::octant`.
fn octant(dx: f64, dy: f64) -> i32 {
    let (adx, ady) = (dx.abs(), dy.abs());
    match (dx >= 0.0, dy >= 0.0) {
        (true, true) => (adx < ady) as i32,
        (true, false) => 7 - (adx < ady) as i32,
        (false, true) => 3 - (adx < ady) as i32,
        (false, false) => 4 + (adx < ady) as i32,
    }
}

#[derive(Clone, Copy)]
struct SegNode {
    p: P,
    seg: usize,
    octant: i32,
    interior: bool,
}

/// `SegmentNode::compareTo` (with `SegmentPointComparator::compare`).
fn cmp_node(a: &SegNode, b: &SegNode) -> Ordering {
    if a.seg != b.seg {
        return a.seg.cmp(&b.seg);
    }
    if a.p == b.p {
        return Ordering::Equal;
    }
    if !a.interior {
        return Ordering::Less;
    }
    if !b.interior {
        return Ordering::Greater;
    }
    let rel = |x0: f64, x1: f64| if x0 < x1 { -1 } else if x0 > x1 { 1 } else { 0 };
    let (xs, ys) = (rel(a.p.0, b.p.0), rel(a.p.1, b.p.1));
    let (s0, s1) = match a.octant {
        0 => (xs, ys),
        1 => (ys, xs),
        2 => (ys, -xs),
        3 => (-xs, ys),
        4 => (-xs, -ys),
        5 => (-ys, -xs),
        6 => (-ys, xs),
        _ => (xs, -ys),
    };
    (if s0 != 0 { s0 } else { s1 }).cmp(&0)
}

/// A segment string with its nodes (`NodedSegmentString`).
struct Noded<'a> {
    pts: &'a [P],
    nodes: Vec<SegNode>,
}

impl Noded<'_> {
    /// `SegmentNodeList::add`.
    fn add(&mut self, p: P, seg: usize) {
        let octant = if seg == self.pts.len() - 1 {
            -1
        } else {
            let (a, b) = (self.pts[seg], self.pts[seg + 1]);
            if a == b {
                0
            } else {
                octant(b.0 - a.0, b.1 - a.1)
            }
        };
        let interior = p != self.pts[seg];
        self.nodes.push(SegNode { p, seg, octant, interior });
    }
    /// `NodedSegmentString::addIntersection`.
    fn add_intersection(&mut self, p: P, seg: usize) {
        let s = if p == self.pts[seg + 1] { seg + 1 } else { seg };
        self.add(p, s);
    }
    /// `SegmentNodeList::prepare`: sort, then drop the equal nodes.
    fn prepare(&mut self) {
        self.nodes.sort_by(cmp_node);
        self.nodes.dedup_by(|b, a| cmp_node(a, b) == Ordering::Equal);
    }
    /// `SegmentNodeList::createSplitEdgePts`.
    fn split_pts(&self, a: &SegNode, b: &SegNode) -> Vec<P> {
        if a.seg == b.seg {
            return vec![a.p, b.p];
        }
        let use_b = b.interior || b.p != self.pts[b.seg];
        let mut v = vec![a.p];
        v.extend_from_slice(&self.pts[a.seg + 1..=b.seg]);
        if use_b {
            v.push(b.p);
        }
        v
    }
    /// `SegmentNodeList::addSplitEdges`.
    fn split_edges(mut self) -> Vec<Vec<P>> {
        let last = self.pts.len() - 1;
        let (p0, pn) = (self.pts[0], self.pts[last]);
        self.add(p0, 0);
        self.add(pn, last);
        self.prepare();
        // addCollapsedNodes
        let mut collapsed = Vec::new();
        for w in self.nodes.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            if a.p == b.p {
                let mut between = b.seg - a.seg;
                if !b.interior {
                    between = between.wrapping_sub(1);
                }
                if between == 1 {
                    collapsed.push(a.seg + 1);
                }
            }
        }
        for i in 0..self.pts.len().saturating_sub(2) {
            if self.pts[i] == self.pts[i + 2] {
                collapsed.push(i + 1);
            }
        }
        for i in collapsed {
            let p = self.pts[i];
            self.add(p, i);
        }
        self.prepare();
        let mut out = Vec::new();
        let mut prev = 0;
        for i in 1..self.nodes.len() {
            if cmp_node(&self.nodes[i], &self.nodes[prev]) == Ordering::Equal {
                continue;
            }
            out.push(self.split_pts(&self.nodes[prev], &self.nodes[i]));
            prev = i;
        }
        out
    }
}

/// The monotone chain of each segment (`MonotoneChainBuilder`): the noder only tests segments of two different chains.
fn chain_ids(pts: &[P]) -> Vec<usize> {
    let mut ids = Vec::with_capacity(pts.len());
    let (mut id, mut quad) = (0, -1);
    for i in 1..pts.len() {
        let (a, b) = (pts[i - 1], pts[i]);
        if a != b {
            let q = quadrant(b.0 - a.0, b.1 - a.1);
            if quad < 0 {
                quad = q;
            }
            if q != quad {
                id += 1;
                quad = q;
            }
        }
        ids.push(id);
    }
    ids
}

/// The noded edges of the curves (`MCIndexNoder` with an `IntersectionAdder`, then the split edges of each curve, in order):
/// (curve index, points).
fn node_curves(curves: &[&[P]]) -> Vec<(usize, Vec<P>)> {
    // every segment: (curve, segment index, chain)
    let mut segs: Vec<(usize, usize, usize)> = Vec::new();
    for (c, pts) in curves.iter().enumerate() {
        for (i, ch) in chain_ids(pts).into_iter().enumerate() {
            segs.push((c, i, ch));
        }
    }
    let seg = |k: usize| {
        let (c, i, _) = segs[k];
        (curves[c][i], curves[c][i + 1])
    };
    let xmin = |k: usize| seg(k).0 .0.min(seg(k).1 .0);
    let xmax = |k: usize| seg(k).0 .0.max(seg(k).1 .0);
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|a, b| xmin(*a).total_cmp(&xmin(*b)));
    let mut pairs = Vec::new();
    for (k, &a) in order.iter().enumerate() {
        for &b in &order[k + 1..] {
            if xmin(b) > xmax(a) {
                break;
            }
            let (sa, sb) = (segs[a], segs[b]);
            if sa.0 != sb.0 || sa.2 != sb.2 {
                pairs.push((a.min(b), a.max(b)));
            }
        }
    }
    pairs.sort_unstable();
    let mut noded: Vec<Noded> = curves.iter().map(|pts| Noded { pts, nodes: Vec::new() }).collect();
    for (a, b) in pairs {
        let ((ca, i, _), (cb, j, _)) = (segs[a], segs[b]);
        let (p, q) = (seg(a), seg(b));
        let li = line_intersection(p.0, p.1, q.0, q.1);
        if li.n == 0 {
            continue;
        }
        // IntersectionAdder::isTrivialIntersection (GEOS compares the last point index, not the last segment index, for a
        // closed string, so the first and last segments of a curve are never "adjacent")
        if ca == cb && li.n == 1 && j.abs_diff(i) == 1 {
            continue;
        }
        for k in 0..li.n {
            noded[ca].add_intersection(li.pts[k], i);
        }
        for k in 0..li.n {
            noded[cb].add_intersection(li.pts[k], j);
        }
    }
    noded.into_iter().enumerate().flat_map(|(c, n)| n.split_edges().into_iter().map(move |e| (c, e))).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// the planar graph and the polygons (geomgraph, buffer::BufferSubgraph, buffer::PolygonBuilder)

const LEFT: usize = 1;
const RIGHT: usize = 2;
const UNSET: i32 = -999;

struct Edge {
    pts: Vec<P>,
    delta: i32,
}

struct DirEdge {
    edge: usize,
    fwd: bool,
    p0: P,
    p1: P,
    quad: i32,
    node: usize,
    depth: [i32; 3],
    in_result: bool,
    visited: bool,
    next: Option<usize>,
    next_min: Option<usize>,
    ring: Option<usize>,
    min_ring: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
struct Key(f64, f64);
impl Eq for Key {}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        self.partial_cmp(o).unwrap_or(Ordering::Equal)
    }
}

struct Node {
    p: P,
    star: Vec<usize>,
    result_area: Option<Vec<usize>>,
}

struct Graph {
    edges: Vec<Edge>,
    des: Vec<DirEdge>,
    nodes: Vec<Node>,
}

pub type Err = String;

fn topo<T>(what: &str, p: P) -> Result<T, Err> {
    Err(format!("TopologyException: {what} at {} {}", p.0, p.1))
}

impl Graph {
    fn sym(de: usize) -> usize {
        de ^ 1
    }
    /// `EdgeEnd::compareDirection`.
    fn cmp_dir(&self, a: usize, b: usize) -> Ordering {
        let (ea, eb) = (&self.des[a], &self.des[b]);
        let (dxa, dya, dxb, dyb) = (ea.p1.0 - ea.p0.0, ea.p1.1 - ea.p0.1, eb.p1.0 - eb.p0.0, eb.p1.1 - eb.p0.1);
        if dxa == dxb && dya == dyb {
            return Ordering::Equal;
        }
        if ea.quad != eb.quad {
            return ea.quad.cmp(&eb.quad);
        }
        orient(eb.p0, eb.p1, ea.p1).cmp(&0)
    }
    /// `PlanarGraph::addEdges`: a directed edge each way, put into the star of its node (a set: an edge end of the same
    /// direction as one already there is not put in).
    fn new(edges: Vec<Edge>) -> Graph {
        let mut g = Graph { edges, des: Vec::new(), nodes: Vec::new() };
        let mut node_of: BTreeMap<Key, usize> = BTreeMap::new();
        for e in 0..g.edges.len() {
            for fwd in [true, false] {
                let pts = &g.edges[e].pts;
                let n = pts.len();
                let (p0, p1) = if fwd { (pts[0], pts[1]) } else { (pts[n - 1], pts[n - 2]) };
                let key = Key(p0.0 + 0.0, p0.1 + 0.0);
                let next_id = node_of.len();
                let node = *node_of.entry(key).or_insert(next_id);
                if node == g.nodes.len() {
                    g.nodes.push(Node { p: p0, star: Vec::new(), result_area: None });
                }
                let de = g.des.len();
                g.des.push(DirEdge {
                    edge: e,
                    fwd,
                    p0,
                    p1,
                    quad: quadrant(p1.0 - p0.0, p1.1 - p0.1),
                    node,
                    depth: [0, UNSET, UNSET],
                    in_result: false,
                    visited: false,
                    next: None,
                    next_min: None,
                    ring: None,
                    min_ring: None,
                });
                let star = &g.nodes[node].star;
                if let Err(pos) = star.binary_search_by(|x| g.cmp_dir(*x, de)) {
                    g.nodes[node].star.insert(pos, de);
                }
            }
        }
        // the node list in the order of GEOS's NodeMap (by x, then y)
        let order: Vec<usize> = node_of.values().copied().collect();
        let mut new_id = vec![0; order.len()];
        for (k, &old) in order.iter().enumerate() {
            new_id[old] = k;
        }
        let mut nodes: Vec<Option<Node>> = g.nodes.into_iter().map(Some).collect();
        g.nodes = order.iter().map(|&o| nodes[o].take().unwrap()).collect();
        for de in &mut g.des {
            de.node = new_id[de.node];
        }
        g
    }
    fn depth_delta(&self, de: usize) -> i32 {
        let d = self.edges[self.des[de].edge].delta;
        if self.des[de].fwd {
            d
        } else {
            -d
        }
    }
    fn set_depth(&mut self, de: usize, pos: usize, d: i32) -> Result<(), Err> {
        let old = self.des[de].depth[pos];
        if old != UNSET && old != d {
            return topo("assigned depths do not match", self.des[de].p0);
        }
        self.des[de].depth[pos] = d;
        Ok(())
    }
    /// `DirectedEdge::setEdgeDepths`.
    fn set_edge_depths(&mut self, de: usize, pos: usize, d: i32) -> Result<(), Err> {
        let factor = if pos == LEFT { -1 } else { 1 };
        let opp = if pos == LEFT { RIGHT } else { LEFT };
        let od = d + self.depth_delta(de) * factor;
        self.set_depth(de, pos, d)?;
        self.set_depth(de, opp, od)
    }
    fn copy_sym_depths(&mut self, de: usize) -> Result<(), Err> {
        let (l, r) = (self.des[de].depth[LEFT], self.des[de].depth[RIGHT]);
        let s = Graph::sym(de);
        self.set_depth(s, LEFT, r)?;
        self.set_depth(s, RIGHT, l)
    }
    /// `DirectedEdgeStar::computeDepths(de)`.
    fn star_depths(&mut self, node: usize, de: usize) -> Result<(), Err> {
        let star = self.nodes[node].star.clone();
        let at = star.iter().position(|x| *x == de).unwrap();
        let start = self.des[de].depth[LEFT];
        let target = self.des[de].depth[RIGHT];
        let mut cur = start;
        for &x in star[at + 1..].iter().chain(&star[..at]) {
            self.set_edge_depths(x, RIGHT, cur)?;
            cur = self.des[x].depth[LEFT];
        }
        if cur != target {
            return topo("depth mismatch", self.des[de].p0);
        }
        Ok(())
    }
    /// `DirectedEdgeStar::getRightmostEdge`.
    fn rightmost_edge(&self, node: usize) -> Option<usize> {
        let star = &self.nodes[node].star;
        let (&de0, &del) = (star.first()?, star.last()?);
        if star.len() == 1 {
            return Some(de0);
        }
        let north = |q: i32| q == 0 || q == 1;
        let (q0, q1) = (self.des[de0].quad, self.des[del].quad);
        if north(q0) && north(q1) {
            Some(de0)
        } else if !north(q0) && !north(q1) {
            Some(del)
        } else if self.des[de0].p1.1 - self.des[de0].p0.1 != 0.0 {
            Some(de0)
        } else if self.des[del].p1.1 - self.des[del].p0.1 != 0.0 {
            Some(del)
        } else {
            None
        }
    }
    /// `RightmostEdgeFinder::findEdge`: the directed edge with the outside of the buffer on its right.
    fn rightmost(&self, de_list: &[usize]) -> Result<(usize, P), Err> {
        let mut min: Option<(usize, usize, P)> = None; // (de, index, coord)
        let check = |de: usize, min: &mut Option<(usize, usize, P)>| {
            let pts = &self.edges[self.des[de].edge].pts;
            for (i, p) in pts[..pts.len() - 1].iter().enumerate() {
                if min.is_none_or(|m| p.0 > m.2 .0) {
                    *min = Some((de, i, *p));
                }
            }
        };
        for &de in de_list {
            if self.des[de].fwd {
                check(de, &mut min);
            }
        }
        let Some((mut min_de, mut min_index, min_coord)) = min else {
            return topo("no forward edges found in buffer subgraph", (0.0, 0.0));
        };
        if min_index == 0 {
            // findRightmostEdgeAtNode
            let Some(de) = self.rightmost_edge(self.des[min_de].node) else {
                return topo("no rightmost edge", min_coord);
            };
            min_de = de;
            if !self.des[min_de].fwd {
                min_de = Graph::sym(min_de);
                min_index = self.edges[self.des[min_de].edge].pts.len() - 1;
            }
        } else {
            // findRightmostEdgeAtVertex
            let pts = &self.edges[self.des[min_de].edge].pts;
            let (prev, next) = (pts[min_index - 1], pts[min_index + 1]);
            let o = orient(min_coord, next, prev);
            if (prev.1 < min_coord.1 && next.1 < min_coord.1 && o == 1) || (prev.1 > min_coord.1 && next.1 > min_coord.1 && o == -1) {
                min_index -= 1;
            }
        }
        // getRightmostSide
        let side_of = |de: usize, i: isize| -> i32 {
            let pts = &self.edges[self.des[de].edge].pts;
            if i < 0 || i as usize + 1 >= pts.len() {
                return -1;
            }
            let (p0, p1) = (pts[i as usize], pts[i as usize + 1]);
            if p0.1 == p1.1 {
                -1
            } else if p0.1 < p1.1 {
                RIGHT as i32
            } else {
                LEFT as i32
            }
        };
        let mut side = side_of(min_de, min_index as isize);
        if side < 0 {
            side = side_of(min_de, min_index as isize - 1);
        }
        let mut coord = min_coord;
        if side < 0 {
            // GEOS resets the rightmost coordinate to that of this edge
            let mut m: Option<(usize, usize, P)> = None;
            check(min_de, &mut m);
            coord = m.map(|x| x.2).unwrap_or(coord);
        }
        Ok((if side == LEFT as i32 { Graph::sym(min_de) } else { min_de }, coord))
    }
    /// `BufferSubgraph::create` (`addReachable`): the nodes and directed edges reached from `start`, in GEOS's order
    /// (a node pushed twice on the stack is added twice, as in GEOS).
    fn subgraph(&mut self, start: usize, visited: &mut [bool]) -> (Vec<usize>, Vec<usize>) {
        let (mut nodes, mut des) = (Vec::new(), Vec::new());
        let mut stack = vec![start];
        while let Some(n) = stack.pop() {
            visited[n] = true;
            nodes.push(n);
            for &de in &self.nodes[n].star {
                des.push(de);
                let sn = self.des[Graph::sym(de)].node;
                if !visited[sn] {
                    stack.push(sn);
                }
            }
        }
        (nodes, des)
    }
    /// `BufferSubgraph::computeDepth`.
    fn compute_depth(&mut self, de_list: &[usize], start: usize, outside: i32) -> Result<(), Err> {
        for &de in de_list {
            self.des[de].visited = false;
        }
        self.set_edge_depths(start, RIGHT, outside)?;
        self.copy_sym_depths(start)?;
        let start_node = self.des[start].node;
        let mut seen = vec![false; self.nodes.len()];
        let mut queue = VecDeque::from([start_node]);
        seen[start_node] = true;
        self.des[start].visited = true;
        while let Some(n) = queue.pop_front() {
            // computeNodeDepth
            let star = self.nodes[n].star.clone();
            let Some(&s) = star.iter().find(|&&de| self.des[de].visited || self.des[Graph::sym(de)].visited) else {
                return topo("unable to find edge to compute depths", self.nodes[n].p);
            };
            self.star_depths(n, s)?;
            for &de in &star {
                self.des[de].visited = true;
                self.copy_sym_depths(de)?;
            }
            for &de in &star {
                let sym = Graph::sym(de);
                if self.des[sym].visited {
                    continue;
                }
                let adj = self.des[sym].node;
                if !seen[adj] {
                    seen[adj] = true;
                    queue.push_back(adj);
                }
            }
        }
        Ok(())
    }
    fn result_area(&mut self, n: usize) -> Vec<usize> {
        if self.nodes[n].result_area.is_none() {
            let v = self.nodes[n].star.iter().copied().filter(|&de| self.des[de].in_result || self.des[Graph::sym(de)].in_result).collect();
            self.nodes[n].result_area = Some(v);
        }
        self.nodes[n].result_area.clone().unwrap()
    }
    /// `DirectedEdgeStar::linkResultDirectedEdges`.
    fn link_result(&mut self, n: usize) -> Result<(), Err> {
        let list = self.result_area(n);
        let (mut first_out, mut incoming) = (None, None);
        let mut linking = false;
        for out in list {
            let inc = Graph::sym(out);
            if first_out.is_none() && self.des[out].in_result {
                first_out = Some(out);
            }
            if !linking {
                if !self.des[inc].in_result {
                    continue;
                }
                incoming = Some(inc);
                linking = true;
            } else {
                if !self.des[out].in_result {
                    continue;
                }
                self.des[incoming.unwrap()].next = Some(out);
                linking = false;
            }
        }
        if linking {
            let Some(f) = first_out else {
                return topo("no outgoing dirEdge found", self.nodes[n].p);
            };
            self.des[incoming.unwrap()].next = Some(f);
        }
        Ok(())
    }
    /// `DirectedEdgeStar::linkMinimalDirectedEdges`.
    fn link_minimal(&mut self, n: usize, er: usize) {
        let list = self.result_area(n);
        let (mut first_out, mut incoming) = (None, None);
        let mut linking = false;
        for &out in list.iter().rev() {
            let inc = Graph::sym(out);
            if first_out.is_none() && self.des[out].ring == Some(er) {
                first_out = Some(out);
            }
            if !linking {
                if self.des[inc].ring != Some(er) {
                    continue;
                }
                incoming = Some(inc);
                linking = true;
            } else {
                if self.des[out].ring != Some(er) {
                    continue;
                }
                self.des[incoming.unwrap()].next_min = Some(out);
                linking = false;
            }
        }
        if linking {
            self.des[incoming.unwrap()].next_min = first_out;
        }
    }
    /// `EdgeRing::computePoints` (maximal: `next`, minimal: `next_min`), then the ring's points.
    fn ring(&mut self, start: usize, id: usize, minimal: bool) -> Result<(Vec<usize>, Vec<P>), Err> {
        let (mut des, mut pts): (Vec<usize>, Vec<P>) = (Vec::new(), Vec::new());
        let mut de = Some(start);
        loop {
            let Some(d) = de else {
                return topo("EdgeRing::computePoints: found null Directed Edge", (0.0, 0.0));
            };
            let r = if minimal { &mut self.des[d].min_ring } else { &mut self.des[d].ring };
            if *r == Some(id) {
                return topo("Directed Edge visited twice during ring-building", self.des[d].p0);
            }
            *r = Some(id);
            des.push(d);
            let e = &self.edges[self.des[d].edge].pts;
            let skip = usize::from(!pts.is_empty());
            if self.des[d].fwd {
                pts.extend_from_slice(&e[skip..]);
            } else {
                pts.extend(e[..e.len() - skip].iter().rev());
            }
            de = if minimal { self.des[d].next_min } else { self.des[d].next };
            if de == Some(start) {
                break;
            }
        }
        Ok((des, pts))
    }
}

/// `Orientation::isCCW`.
pub fn is_ccw(ring: &[P]) -> bool {
    if ring.len() < 4 {
        return false;
    }
    let n = ring.len() - 1;
    let mut up_hi = ring[0];
    let mut up_low: Option<P> = None;
    let mut prev_y = up_hi.1;
    let mut i_up_hi = 0;
    for i in 1..=n {
        let py = ring[i].1;
        if py > prev_y && py >= up_hi.1 {
            i_up_hi = i;
            up_hi = ring[i];
            up_low = Some(ring[i - 1]);
        }
        prev_y = py;
    }
    if i_up_hi == 0 {
        return false;
    }
    let mut i_down_low = i_up_hi;
    loop {
        i_down_low = (i_down_low + 1) % n;
        if !(i_down_low != i_up_hi && ring[i_down_low].1 == up_hi.1) {
            break;
        }
    }
    let down_low = ring[i_down_low];
    let i_down_hi = if i_down_low > 0 { i_down_low - 1 } else { n - 1 };
    let down_hi = ring[i_down_hi];
    if up_hi == down_hi {
        let up_low = up_low.unwrap();
        if up_low == up_hi || down_low == up_hi || up_low == down_low {
            return false;
        }
        orient(up_low, up_hi, down_low) == 1
    } else {
        down_hi.0 - up_hi.0 < 0.0
    }
}

/// `Area::ofRingSigned`.
pub fn signed_area(ring: &[P]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let x0 = ring[0].0;
    let mut sum = 0.0;
    for i in 1..n - 1 {
        sum += (ring[i].0 - x0) * (ring[i - 1].1 - ring[i + 1].1);
    }
    sum / 2.0
}

type Env = (f64, f64, f64, f64);

fn envelope(ring: &[P]) -> Env {
    ring.iter().fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |e, p| {
        (e.0.min(p.0), e.1.min(p.1), e.2.max(p.0), e.3.max(p.1))
    })
}

fn env_contains(a: Env, b: Env) -> bool {
    b.0 >= a.0 && b.2 <= a.2 && b.1 >= a.1 && b.3 <= a.3
}

/// `RayCrossingCounter::locatePointInRing` is not EXTERIOR.
fn in_ring(p: P, ring: &[P]) -> bool {
    let mut crossings = 0;
    for w in ring.windows(2) {
        let (p1, p2) = (w[0], w[1]);
        if p1.0 < p.0 && p2.0 < p.0 {
            continue;
        }
        if p == p2 {
            return true;
        }
        if p1.1 == p.1 && p2.1 == p.1 {
            if p.0 >= p1.0.min(p2.0) && p.0 <= p1.0.max(p2.0) {
                return true;
            }
            continue;
        }
        if (p1.1 > p.1 && p2.1 <= p.1) || (p2.1 > p.1 && p1.1 <= p.1) {
            let mut sign = orient(p1, p2, p);
            if sign == 0 {
                return true;
            }
            if p2.1 < p1.1 {
                sign = -sign;
            }
            if sign > 0 {
                crossings += 1;
            }
        }
    }
    crossings % 2 == 1
}

/// A polygon: the shell (clockwise) and its holes.
pub struct Polygon {
    pub shell: Vec<P>,
    pub holes: Vec<Vec<P>>,
}

impl Polygon {
    pub fn area(&self) -> f64 {
        signed_area(&self.shell).abs() - self.holes.iter().map(|h| signed_area(h).abs()).sum::<f64>()
    }
}

/// The rightmost coordinate of a subgraph and the depth outside it (`SubgraphDepthLocater::getDepth`): the left depth of the
/// lowest segment of the processed subgraphs that a ray from p to the right crosses, 0 when none.
fn outside_depth(g: &Graph, processed: &[Vec<usize>], p: P) -> i32 {
    // (low, high, depth)
    let mut stabbed: Vec<(P, P, i32)> = Vec::new();
    for des in processed {
        // BufferSubgraph::getEnvelope: the points of its edges, the last point of each left out
        let mut env = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &de in des {
            let pts = &g.edges[g.des[de].edge].pts;
            for q in &pts[..pts.len() - 1] {
                env = (env.0.min(q.0), env.1.min(q.1), env.2.max(q.0), env.3.max(q.1));
            }
        }
        if p.1 < env.1 || p.1 > env.3 || p.0 > env.2 {
            continue;
        }
        for &de in des {
            if !g.des[de].fwd {
                continue;
            }
            let pts = &g.edges[g.des[de].edge].pts;
            let e = envelope(pts);
            if p.1 < e.1 || p.1 > e.3 || p.0 > e.2 {
                continue;
            }
            for w in pts.windows(2) {
                let (mut low, mut high) = (w[0], w[1]);
                let swap = low.1 > high.1;
                if swap {
                    std::mem::swap(&mut low, &mut high);
                }
                if low.0.max(high.0) < p.0 || low.1 == high.1 || p.1 < low.1 || p.1 > high.1 || orient(low, high, p) == -1 {
                    continue;
                }
                let depth = if swap { g.des[de].depth[RIGHT] } else { g.des[de].depth[LEFT] };
                stabbed.push((low, high, depth));
            }
        }
    }
    // DepthSegment::compareTo
    let cmp_pt = |a: P, b: P| a.0.partial_cmp(&b.0).unwrap().then(a.1.partial_cmp(&b.1).unwrap());
    let seg_orient = |s: (P, P), t: (P, P)| {
        let (o0, o1) = (orient(s.0, s.1, t.0), orient(s.0, s.1, t.1));
        if o0 >= 0 && o1 >= 0 {
            o0.max(o1)
        } else if o0 <= 0 && o1 <= 0 {
            o0.min(o1)
        } else {
            0
        }
    };
    let less = |a: &(P, P, i32), b: &(P, P, i32)| {
        let (s, t) = ((a.0, a.1), (b.0, b.1));
        let (sminx, smaxx, sminy, smaxy) = (s.0 .0.min(s.1 .0), s.0 .0.max(s.1 .0), s.0 .1.min(s.1 .1), s.0 .1.max(s.1 .1));
        let (tminx, tmaxx, tminy, tmaxy) = (t.0 .0.min(t.1 .0), t.0 .0.max(t.1 .0), t.0 .1.min(t.1 .1), t.0 .1.max(t.1 .1));
        let c = if sminx >= tmaxx || smaxx <= tminx || sminy >= tmaxy || smaxy <= tminy {
            match cmp_pt(s.0, t.0) {
                std::cmp::Ordering::Equal => cmp_pt(s.1, t.1) as i32,
                o => o as i32,
            }
        } else {
            let o = seg_orient(s, t);
            if o != 0 {
                o
            } else {
                -seg_orient(t, s)
            }
        };
        c < 0
    };
    let mut best: Option<&(P, P, i32)> = None;
    for x in &stabbed {
        if best.is_none_or(|b| less(x, b)) {
            best = Some(x);
        }
    }
    best.map(|b| b.2).unwrap_or(0)
}

/// The polygons of the buffer whose raw curves are `curves` (closed), each with its depth delta (-1: the left side is outside,
/// +1: the right side is outside). First GEOS's `BufferBuilder::buffer` with no precision model. Where it gives a
/// `TopologyException`, it tries again as `BufferOp::bufferReducedPrecision`: the points rounded to a grid of 12, 11, ... 0
/// digits of precision of the extent of the curves. The error of the first try when each try fails.
// ponytail: GEOS rounds the curves while it makes them and nodes them with snap rounding; here the made curves are rounded
// and the same noder runs again. The result can differ from GEOS's by less than the grid, only where the first try fails.
pub fn buffer_polygons(curves: &[(Vec<P>, i32)]) -> Result<Vec<Polygon>, Err> {
    let first = match buffer_full_precision(curves) {
        Ok(p) => return Ok(p),
        Err(e) => e,
    };
    let max = curves.iter().flat_map(|c| c.0.iter()).fold(0.0f64, |m, p| m.max(p.0.abs()).max(p.1.abs()));
    // BufferOp::precisionScaleFactor: the digits of the extent are taken from the precision
    let env_digits = if max > 0.0 { (max.log10() + 1.0) as i32 } else { 1 };
    for digits in (0..=12).rev() {
        let scale = 10f64.powi(digits - env_digits);
        let rounded: Vec<(Vec<P>, i32)> =
            curves.iter().map(|(c, d)| (c.iter().map(|p| ((p.0 * scale).round() / scale, (p.1 * scale).round() / scale)).collect(), *d)).collect();
        if let Ok(p) = buffer_full_precision(&rounded) {
            return Ok(p);
        }
    }
    Err(first)
}

fn buffer_full_precision(curves: &[(Vec<P>, i32)]) -> Result<Vec<Polygon>, Err> {
    // computeNodedEdges and insertUniqueEdge
    let mut edges: Vec<Edge> = Vec::new();
    let mut index: HashMap<Vec<(u64, u64)>, usize> = HashMap::new();
    let bits = |p: &P| ((p.0 + 0.0).to_bits(), (p.1 + 0.0).to_bits());
    let refs: Vec<&[P]> = curves.iter().map(|c| c.0.as_slice()).collect();
    for (c, mut pts) in node_curves(&refs) {
        let d = curves[c].1;
        pts.dedup();
        if pts.len() < 2 {
            continue;
        }
        let fwd: Vec<(u64, u64)> = pts.iter().map(bits).collect();
        let rev: Vec<(u64, u64)> = fwd.iter().rev().copied().collect();
        let key = fwd.clone().min(rev);
        match index.get(&key) {
            Some(&e) => {
                let same_way = edges[e].pts.iter().map(bits).eq(fwd.iter().copied());
                edges[e].delta += if same_way { d } else { -d };
            }
            None => {
                index.insert(key, edges.len());
                edges.push(Edge { pts, delta: d });
            }
        }
    }
    if edges.is_empty() {
        return Ok(vec![]);
    }
    let mut g = Graph::new(edges);
    // createSubgraphs, sorted by their rightmost x, largest first
    let mut visited = vec![false; g.nodes.len()];
    let mut subgraphs: Vec<(Vec<usize>, Vec<usize>, usize, P)> = Vec::new();
    for n in 0..g.nodes.len() {
        if !visited[n] {
            let (nodes, des) = g.subgraph(n, &mut visited);
            let (start, coord) = g.rightmost(&des)?;
            subgraphs.push((nodes, des, start, coord));
        }
    }
    let mut idx: Vec<usize> = (0..subgraphs.len()).collect();
    crate::shapely::cpp_sort(&mut idx, &|a, b| subgraphs[*a].3 .0 > subgraphs[*b].3 .0);
    let mut slots: Vec<Option<(Vec<usize>, Vec<usize>, usize, P)>> = subgraphs.into_iter().map(Some).collect();
    let subgraphs: Vec<(Vec<usize>, Vec<usize>, usize, P)> = idx.iter().map(|&i| slots[i].take().unwrap()).collect();
    struct Ring {
        pts: Vec<P>,
        hole: bool,
        shell: Option<usize>,
    }
    let mut rings: Vec<Ring> = Vec::new();
    let mut shells: Vec<usize> = Vec::new();
    let mut hole_order: Vec<usize> = Vec::new(); // the holes, in the order GEOS gives them to a shell
    let (mut max_id, mut min_id) = (0, 0);
    let mut processed: Vec<Vec<usize>> = Vec::new();
    for (nodes, des, start, coord) in subgraphs {
        let outside = outside_depth(&g, &processed, coord);
        g.compute_depth(&des, start, outside)?;
        for &de in &des {
            let d = &g.des[de];
            if d.depth[RIGHT] >= 1 && d.depth[LEFT] <= 0 {
                g.des[de].in_result = true;
            }
        }
        processed.push(des.clone());
        // PolygonBuilder::add
        for &n in &nodes {
            g.link_result(n)?;
        }
        let mut max_rings: Vec<(usize, Vec<usize>, Vec<P>)> = Vec::new(); // (id, des, pts)
        for &de in &des {
            if g.des[de].in_result && g.des[de].ring.is_none() {
                let (rd, pts) = g.ring(de, max_id, false)?;
                max_rings.push((max_id, rd, pts));
                max_id += 1;
            }
        }
        let mut free_holes: Vec<usize> = Vec::new();
        let mut plain: Vec<usize> = Vec::new();
        for (id, rd, pts) in &max_rings {
            let id = *id;
            let degree = rd
                .iter()
                .map(|&de| g.nodes[g.des[de].node].star.iter().filter(|&&x| g.des[x].ring == Some(id)).count())
                .max()
                .unwrap_or(0);
            if degree > 2 {
                for &de in rd {
                    let n = g.des[de].node;
                    g.link_minimal(n, id);
                }
                let mut mins = Vec::new();
                for &de in rd {
                    if g.des[de].min_ring.is_none() {
                        let (_, mp) = g.ring(de, min_id, true)?;
                        min_id += 1;
                        let hole = is_ccw(&mp);
                        rings.push(Ring { pts: mp, hole, shell: None });
                        mins.push(rings.len() - 1);
                    }
                }
                let shell_list: Vec<usize> = mins.iter().copied().filter(|&r| !rings[r].hole).collect();
                if shell_list.len() > 1 {
                    return topo("found two shells in MinimalEdgeRing list", rings[shell_list[1]].pts[0]);
                }
                if let Some(&s) = shell_list.first() {
                    for &r in &mins {
                        if rings[r].hole {
                            rings[r].shell = Some(s);
                            hole_order.push(r);
                        }
                    }
                    shells.push(s);
                } else {
                    free_holes.extend(mins);
                }
            } else {
                rings.push(Ring { pts: pts.clone(), hole: is_ccw(pts), shell: None });
                plain.push(rings.len() - 1);
            }
        }
        // sortShellsAndHoles
        for r in plain {
            if rings[r].hole {
                free_holes.push(r);
            } else {
                shells.push(r);
            }
        }
        // placeFreeHoles (findEdgeRingContaining)
        for &h in &free_holes {
            if rings[h].shell.is_some() {
                continue;
            }
            let test_env = envelope(&rings[h].pts);
            let mut min_shell: Option<usize> = None;
            for &s in &shells {
                let env = envelope(&rings[s].pts);
                if env == test_env || !env_contains(env, test_env) {
                    continue;
                }
                let shell_pts = &rings[s].pts;
                let test_pt = rings[h].pts.iter().find(|p| !shell_pts.contains(p)).copied();
                let contained = test_pt.is_some_and(|p| in_ring(p, shell_pts));
                if contained && min_shell.is_none_or(|m| env_contains(envelope(&rings[m].pts), env)) {
                    min_shell = Some(s);
                }
            }
            if min_shell.is_some() {
                rings[h].shell = min_shell;
                hole_order.push(h);
            }
        }
    }
    Ok(shells
        .iter()
        .map(|&s| Polygon {
            shell: rings[s].pts.clone(),
            holes: hole_order.iter().filter(|&&h| rings[h].shell == Some(s)).map(|&h| rings[h].pts.clone()).collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dd_intersection_and_orientation() {
        assert_eq!(dd_intersection((0.0, 0.0), (2.0, 2.0), (0.0, 2.0), (2.0, 0.0)), Some((1.0, 1.0)));
        assert_eq!(orient((0.0, 0.0), (1.0, 0.0), (0.5, 1.0)), 1);
        assert_eq!(orient((0.0, 0.0), (1.0, 0.0), (0.5, -1.0)), -1);
        assert_eq!(orient((0.0, 0.0), (1.0, 1.0), (3.0, 3.0)), 0);
    }

    /// A square ring around the origin, clockwise, with a loop that crosses itself: the buffer is the square.
    #[test]
    fn square_with_a_loop() {
        let ring = [(-1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)];
        let p = buffer_polygons(&[(ring.to_vec(), -1)]).unwrap();
        assert_eq!(p.len(), 1);
        assert!((p[0].area() - 4.0).abs() < 1e-12);
        assert!(p[0].holes.is_empty());
    }
}
