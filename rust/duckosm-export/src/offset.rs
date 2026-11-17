//! The offset curve of a line, in pure Rust: the port of GEOS's `OffsetCurve` (shapely's `offset_curve(d)` with its
//! defaults: 16 segments for a quarter circle, round joins). A positive distance is on the left.
//!
//! GEOS makes the raw offset curve (`OffsetCurveBuilder::getOffsetCurve`), then keeps the parts of it that are on the
//! ring of the buffer polygon of the line. Here the raw curve and the raw ring of the buffer (`computeLineBufferCurve`)
//! are made with the same steps; the raw curve is cut where it crosses the ring, and a piece stays when it is not inside
//! the buffer (the winding number of the ring). The points of the raw curve are the same as in GEOS; a point where the
//! curve is cut agrees with GEOS's noding to much less than 1 mm.

use std::f64::consts::{FRAC_PI_2, PI};

pub type P = (f64, f64);

/// shapely's default `quad_segs` of `offset_curve`.
const QUADRANT_SEGMENTS: f64 = 16.0;
const CURVE_VERTEX_SNAP_DISTANCE_FACTOR: f64 = 1.0e-4;
const OFFSET_SEGMENT_SEPARATION_FACTOR: f64 = 1.0e-3;
const INSIDE_TURN_VERTEX_SNAP_DISTANCE_FACTOR: f64 = 1.0e-3;
const SIMPLIFY_FACTOR: f64 = 100.0;
const CLOSING_SEG_LEN_FACTOR: f64 = 80.0;
const CW: i32 = -1;
const CCW: i32 = 1;

/// The orientation of (p0, p1, p2): CCW 1, CW -1, collinear 0, as GEOS `CGAlgorithmsDD::orientationIndex`: a fast
/// filter, then double-double arithmetic.
fn orient(p0: P, p1: P, p2: P) -> i32 {
    // GEOS orientationIndexFilter
    let detleft = (p0.0 - p2.0) * (p1.1 - p2.1);
    let detright = (p0.1 - p2.1) * (p1.0 - p2.0);
    let det = detleft - detright;
    let detsum = if detleft > 0.0 {
        (detright <= 0.0).then_some(0.0).unwrap_or(detleft + detright)
    } else if detleft < 0.0 {
        (detright >= 0.0).then_some(0.0).unwrap_or(-detleft - detright)
    } else {
        0.0
    };
    let sign = |d: f64| if d > 0.0 { CCW } else if d < 0.0 { CW } else { 0 };
    if detleft == 0.0 || (detleft > 0.0 && detright <= 0.0) || (detleft < 0.0 && detright >= 0.0) {
        return sign(det);
    }
    if det.abs() >= 1e-15 * detsum {
        return sign(det);
    }
    // (p1 - p0) x (p2 - p1) in double-double: the differences are exact, the products carry their error terms
    let two_diff = |a: f64, b: f64| {
        let s = a - b;
        let bb = s - a;
        (s, (a - (s - bb)) - (b + bb))
    };
    let mul = |a: (f64, f64), b: (f64, f64)| {
        let p = a.0 * b.0;
        let e = a.0.mul_add(b.0, -p);
        (p, e + a.0 * b.1 + a.1 * b.0 + a.1 * b.1)
    };
    let (dx1, dy1, dx2, dy2) = (two_diff(p1.0, p0.0), two_diff(p1.1, p0.1), two_diff(p2.0, p1.0), two_diff(p2.1, p1.1));
    let (m1, m2) = (mul(dx1, dy2), mul(dy1, dx2));
    sign((m1.0 - m2.0) + (m1.1 - m2.1))
}

fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The distance from p to the segment a-b (GEOS `Distance::pointToSegment`).
fn point_to_segment(p: P, a: P, b: P) -> f64 {
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

/// The point where the segments a0-a1 and b0-b1 meet, if they meet (an end point when they only touch).
fn intersection(a0: P, a1: P, b0: P, b1: P) -> Option<P> {
    let (o1, o2, o3, o4) = (orient(a0, a1, b0), orient(a0, a1, b1), orient(b0, b1, a0), orient(b0, b1, a1));
    if (o1 > 0 && o2 > 0) || (o1 < 0 && o2 < 0) || (o3 > 0 && o4 > 0) || (o3 < 0 && o4 < 0) {
        return None;
    }
    if o1 == 0 && o2 == 0 && o3 == 0 && o4 == 0 {
        // collinear: an end point in the other segment
        let within = |p: P, s: P, e: P| p.0 >= s.0.min(e.0) && p.0 <= s.0.max(e.0) && p.1 >= s.1.min(e.1) && p.1 <= s.1.max(e.1);
        return [b0, b1].into_iter().find(|p| within(*p, a0, a1)).or([a0, a1].into_iter().find(|p| within(*p, b0, b1)));
    }
    for (o, p) in [(o1, b0), (o2, b1), (o3, a0), (o4, a1)] {
        if o == 0 {
            return Some(p);
        }
    }
    let d = (a1.0 - a0.0) * (b1.1 - b0.1) - (a1.1 - a0.1) * (b1.0 - b0.0);
    let t = ((b0.0 - a0.0) * (b1.1 - b0.1) - (b0.1 - a0.1) * (b1.0 - b0.0)) / d;
    Some((a0.0 + t * (a1.0 - a0.0), a0.1 + t * (a1.1 - a0.1)))
}

/// GEOS `BufferInputLineSimplifier`: drop the vertices of shallow concavities on one side (tol > 0: the left side).
fn simplify(pts: &[P], tol: f64) -> Vec<P> {
    let dtol = tol.abs();
    let ang = if tol < 0.0 { CW } else { CCW };
    let n = pts.len();
    let mut deleted = vec![false; n];
    let next = |deleted: &[bool], i: usize| {
        let mut j = i + 1;
        while j < n && deleted[j] {
            j += 1;
        }
        j
    };
    let shallow = |p0: P, p1: P, p2: P| point_to_segment(p1, p0, p2) < dtol;
    loop {
        let mut changed = false;
        let mut index = 1;
        let mut mid = next(&deleted, index);
        let mut last = next(&deleted, mid);
        while last < n {
            let (p0, p1, p2) = (pts[index], pts[mid], pts[last]);
            let mut deletable = orient(p0, p1, p2) == ang && shallow(p0, p1, p2);
            if deletable {
                // isShallowSampled(p0, p1, i0, i2): as GEOS, with p1 as its second point
                let inc = ((last - index) / 10).max(1);
                let mut i = index;
                while i < last {
                    if !shallow(p0, p1, pts[i]) {
                        deletable = false;
                        break;
                    }
                    i += inc;
                }
            }
            if deletable {
                deleted[mid] = true;
                changed = true;
                index = last;
            } else {
                index = mid;
            }
            mid = next(&deleted, index);
            last = next(&deleted, mid);
        }
        if !changed {
            break;
        }
    }
    pts.iter().zip(&deleted).filter(|(_, d)| !**d).map(|(p, _)| *p).collect()
}

/// GEOS `OffsetSegmentGenerator`, the left side only (as the offset curve and the line buffer use it).
struct Gen {
    d: f64,
    quantum: f64,
    min_vertex: f64,
    pts: Vec<P>,
    s0: P,
    s1: P,
    s2: P,
    off0: (P, P),
    off1: (P, P),
}

impl Gen {
    fn new(d: f64) -> Gen {
        Gen {
            d,
            quantum: FRAC_PI_2 / QUADRANT_SEGMENTS,
            min_vertex: d * CURVE_VERTEX_SNAP_DISTANCE_FACTOR,
            pts: Vec::new(),
            s0: (0.0, 0.0),
            s1: (0.0, 0.0),
            s2: (0.0, 0.0),
            off0: ((0.0, 0.0), (0.0, 0.0)),
            off1: ((0.0, 0.0), (0.0, 0.0)),
        }
    }
    fn add(&mut self, p: P) {
        if let Some(l) = self.pts.last() {
            if dist(p, *l) < self.min_vertex {
                return;
            }
        }
        self.pts.push(p);
    }
    /// The offset of the segment a-b, `side` 1 = left, -1 = right.
    fn offset(a: P, b: P, side: f64, d: f64) -> (P, P) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = (side * d * dx / len, side * d * dy / len);
        ((a.0 - uy, a.1 + ux), (b.0 - uy, b.1 + ux))
    }
    fn init_side(&mut self, a: P, b: P) {
        self.s1 = a;
        self.s2 = b;
        self.off1 = Gen::offset(a, b, 1.0, self.d);
    }
    fn fillet_angles(&mut self, p: P, start: f64, end: f64, direction: i32, r: f64) {
        let f = if direction == CW { -1.0 } else { 1.0 };
        let total = (start - end).abs();
        let n = (total / self.quantum + 0.5) as i64;
        if n < 1 {
            return;
        }
        let inc = total / n as f64;
        for i in 0..n {
            let a = start + f * i as f64 * inc;
            let (mut s, mut c) = a.sin_cos();
            if s.abs() < 5e-16 {
                s = 0.0;
            }
            if c.abs() < 5e-16 {
                c = 0.0;
            }
            self.add((p.0 + r * c, p.1 + r * s));
        }
    }
    fn fillet(&mut self, p: P, p0: P, p1: P, direction: i32, r: f64) {
        let mut start = (p0.1 - p.1).atan2(p0.0 - p.0);
        let end = (p1.1 - p.1).atan2(p1.0 - p.0);
        if direction == CW {
            if start <= end {
                start += 2.0 * PI;
            }
        } else if start >= end {
            start -= 2.0 * PI;
        }
        self.add(p0);
        self.fillet_angles(p, start, end, direction, r);
        self.add(p1);
    }
    fn next_segment(&mut self, p: P) {
        if self.s2 == p {
            return;
        }
        self.s0 = self.s1;
        self.s1 = self.s2;
        self.s2 = p;
        self.off0 = Gen::offset(self.s0, self.s1, 1.0, self.d);
        self.off1 = Gen::offset(self.s1, self.s2, 1.0, self.d);
        let o = orient(self.s0, self.s1, self.s2);
        let (s1, d) = (self.s1, self.d);
        let (o0, o1) = (self.off0, self.off1);
        if o == 0 {
            // collinear: a fillet only when the line goes back on itself
            let back = (self.s2.0 - s1.0) * (s1.0 - self.s0.0) + (self.s2.1 - s1.1) * (s1.1 - self.s0.1) < 0.0;
            if back {
                self.fillet(s1, o0.1, o1.0, CW, d);
            }
        } else if o == CW {
            // the outside turn (the left side)
            if dist(o0.1, o1.0) < d * OFFSET_SEGMENT_SEPARATION_FACTOR {
                self.add(o0.1);
                return;
            }
            self.add(o0.1);
            self.fillet(s1, o0.1, o1.0, o, d);
            self.add(o1.0);
        } else if let Some(x) = intersection(o0.0, o0.1, o1.0, o1.1) {
            self.add(x);
        } else if dist(o0.1, o1.0) < d * INSIDE_TURN_VERTEX_SNAP_DISTANCE_FACTOR {
            self.add(o0.1);
        } else {
            let k = CLOSING_SEG_LEN_FACTOR;
            self.add(o0.1);
            self.add(((k * o0.1.0 + s1.0) / (k + 1.0), (k * o0.1.1 + s1.1) / (k + 1.0)));
            self.add(((k * o1.0.0 + s1.0) / (k + 1.0), (k * o1.0.1 + s1.1) / (k + 1.0)));
            self.add(o1.0);
        }
    }
    fn end_cap(&mut self, p0: P, p1: P, p2: P) {
        let l = Gen::offset(p0, p1, 1.0, self.d);
        let r = Gen::offset(p2, p1, -1.0, self.d);
        self.add(l.1);
        self.fillet(p1, l.1, r.1, CW, self.d);
        self.add(r.1);
    }
}

/// The raw offset curve (GEOS `OffsetCurveBuilder::getOffsetCurve`), in the direction of the line.
#[doc(hidden)]
pub fn raw_curve(pts: &[P], d: f64) -> Vec<P> {
    let right = d < 0.0;
    let mut g = Gen::new(d.abs());
    let tol = d.abs() / SIMPLIFY_FACTOR;
    if right {
        let s = simplify(pts, -tol);
        let n = s.len() - 1;
        g.init_side(s[n], s[n - 1]);
        let first = g.off1.0;
        g.add(first);
        for i in (1..n).rev() {
            g.next_segment(s[i - 1]);
        }
    } else {
        let s = simplify(pts, tol);
        g.init_side(s[0], s[1]);
        let first = g.off1.0;
        g.add(first);
        for p in &s[2..] {
            g.next_segment(*p);
        }
    }
    let last = g.off1.1;
    g.add(last);
    // OffsetSegmentString::getCoordinates closes the curve: the chord back to the start is a raw segment too
    if g.pts.first() != g.pts.last() {
        let f = g.pts[0];
        g.pts.push(f);
    }
    if right {
        g.pts.reverse();
    }
    g.pts
}

/// The raw ring of the buffer of the line (GEOS `computeLineBufferCurve`, round caps): clockwise around the line.
#[doc(hidden)]
pub fn buffer_ring(pts: &[P], d: f64) -> Vec<P> {
    let mut g = Gen::new(d);
    let tol = d / SIMPLIFY_FACTOR;
    let s1 = simplify(pts, tol);
    let n1 = s1.len() - 1;
    g.init_side(s1[0], s1[1]);
    for p in &s1[2..] {
        g.next_segment(*p);
    }
    let last = g.off1.1;
    g.add(last);
    let s2 = simplify(pts, -tol);
    let n2 = s2.len() - 1;
    g.end_cap(s1[n1 - 1], s1[n1], s2[n2 - 1]);
    g.init_side(s2[n2], s2[n2 - 1]);
    for i in (1..n2).rev() {
        g.next_segment(s2[i - 1]);
    }
    let last = g.off1.1;
    g.add(last);
    g.end_cap(s2[1], s2[0], s1[1]);
    if g.pts.first() != g.pts.last() {
        let f = g.pts[0];
        g.pts.push(f);
    }
    g.pts
}

/// The clockwise winding number of the closed ring around p.
fn winding_cw(ring: &[P], p: P) -> i32 {
    let mut w = 0;
    for s in ring.windows(2) {
        let (a, b) = (s[0], s[1]);
        if a.1 <= p.1 {
            if b.1 > p.1 && orient(a, b, p) == CCW {
                w -= 1;
            }
        } else if b.1 <= p.1 && orient(a, b, p) == CW {
            w += 1;
        }
    }
    w
}

const MATCH_DISTANCE_FACTOR: f64 = 10000.0;

/// GEOS `LineSegment::segmentFraction`: the projection factor of p on a-b, in 0..1.
fn segment_fraction(p: P, a: P, b: P) -> f64 {
    let len2 = (b.0 - a.0) * (b.0 - a.0) + (b.1 - a.1) * (b.1 - a.1);
    let f = ((p.0 - a.0) * (b.0 - a.0) + (p.1 - a.1) * (b.1 - a.1)) / len2;
    if f < 0.0 {
        0.0
    } else if f > 1.0 || f.is_nan() {
        1.0
    } else {
        f
    }
}

/// The sections as GEOS `computeCurveSections` and `extractSections` make them from the runs of the ring that lie on
/// the raw curve. Each segment of a run gets the location of the last raw segment that matches it (both ends within
/// `md`), the closing chord too. The walk starts at the segment of the smallest location (raw segment by raw segment),
/// so the run that holds it is cut there. The sections are sorted by the location of their first segment.
fn geos_sections(runs: Vec<Vec<P>>, raw: &[P], md: f64) -> Vec<Vec<P>> {
    let mut loc: Vec<Vec<f64>> = runs.iter().map(|r| vec![-1.0; r.len() - 1]).collect();
    let mut first: Option<(usize, usize)> = None;
    let mut min_pos = -1.0;
    for (i, w) in raw.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        if a == b {
            continue;
        }
        let mut best: Option<((usize, usize), f64)> = None;
        for (r, run) in runs.iter().enumerate() {
            for j in 0..run.len() - 1 {
                if point_to_segment(run[j], a, b) > md || point_to_segment(run[j + 1], a, b) > md {
                    continue;
                }
                let l = i as f64 + segment_fraction(run[j], a, b);
                loc[r][j] = l;
                if best.is_none_or(|x| l < x.1) {
                    best = Some(((r, j), l));
                }
            }
        }
        if let Some((k, pos)) = best {
            if first.is_none() || pos < min_pos {
                first = Some(k);
                min_pos = pos;
            }
        }
    }
    let mut out: Vec<(f64, Vec<P>)> = Vec::new();
    for (r, run) in runs.into_iter().enumerate() {
        // a segment that no raw segment matches ends a section
        let mut cuts: Vec<usize> = vec![0];
        for j in 0..run.len() - 1 {
            if loc[r][j] < 0.0 {
                cuts.push(j);
                cuts.push(j + 1);
            }
        }
        if let Some((fr, fj)) = first {
            if fr == r && fj > 0 {
                cuts.push(fj);
            }
        }
        cuts.push(run.len() - 1);
        cuts.sort_unstable();
        cuts.dedup();
        for c in cuts.windows(2) {
            let (s, e) = (c[0], c[1]);
            if e > s && loc[r][s] >= 0.0 {
                out.push((loc[r][s], run[s..=e].to_vec()));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|s| s.1).collect()
}

/// The offset curve of the line `pts` at the distance `d` (left: positive), as shapely's `offset_curve(d)`: the
/// sections of the curve (more than one: a MultiLineString). Empty: GEOS gives an empty line.
pub fn offset_curve(pts: &[P], d: f64) -> Vec<Vec<P>> {
    let length: f64 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
    if pts.len() < 2 || length == 0.0 {
        return vec![];
    }
    if d == 0.0 {
        return vec![pts.to_vec()];
    }
    if pts.len() == 2 {
        // GEOS LineSegment::offset
        let (a, b) = (pts[0], pts[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = (d * dx / len, d * dy / len);
        return vec![vec![(a.0 - uy, a.1 + ux), (b.0 - uy, b.1 + ux)]];
    }
    let mut clean: Vec<P> = Vec::with_capacity(pts.len());
    for p in pts {
        if p.0.is_finite() && p.1.is_finite() && clean.last() != Some(p) {
            clean.push(*p);
        }
    }
    let raw_closed = raw_curve(&clean, d);
    if raw_closed.len() < 2 {
        return vec![];
    }
    // the curve itself, without the closing chord (the first segment on the right side, else the last)
    let raw: &[P] = if raw_closed.len() < 3 {
        &raw_closed
    } else if d < 0.0 {
        &raw_closed[1..]
    } else {
        &raw_closed[..raw_closed.len() - 1]
    };
    let ring = buffer_ring(&clean, d.abs());
    // a piece of the raw curve stays when a point just outside it (away from the line) is outside the buffer
    let eps = 1e-6 * d.abs().max(1e-9);
    let md = d.abs() / MATCH_DISTANCE_FACTOR;
    let side = if d > 0.0 { 1.0 } else { -1.0 };
    // the sections: (location of the start on the raw curve, points). A section follows the ring of the buffer: where the
    // raw curve leaves the ring at a point and comes back to the same point later (a loop inside the buffer), the
    // section goes on through that point.
    let mut sections: Vec<(f64, Vec<P>)> = Vec::new();
    let mut cur: Vec<P> = Vec::new();
    let mut cur_loc = 0.0;
    let close = |cur: &mut Vec<P>, cur_loc: f64, sections: &mut Vec<(f64, Vec<P>)>| {
        if cur.is_empty() {
            return;
        }
        let end = *cur.last().unwrap();
        let mut run = std::mem::take(cur);
        // the run ends where an earlier section starts: the ring goes on into that section
        if let Some(i) = sections.iter().position(|s| dist(s.1[0], end) <= eps) {
            let (_, tail) = sections.remove(i);
            run.extend_from_slice(&tail[1..]);
        }
        sections.push((cur_loc, run));
    };
    for (i, w) in raw.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let len = dist(a, b);
        if len == 0.0 {
            continue;
        }
        let mut ts: Vec<f64> = ring
            .windows(2)
            .filter_map(|r| {
                let x = intersection(a, b, r[0], r[1])?;
                let t = ((x.0 - a.0) * (b.0 - a.0) + (x.1 - a.1) * (b.1 - a.1)) / (len * len);
                (t * len > eps && (1.0 - t) * len > eps).then_some(t)
            })
            .collect();
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y) * len <= eps);
        let at = |t: f64| if t == 0.0 { a } else if t == 1.0 { b } else { (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)) };
        let (nx, ny) = (-(b.1 - a.1) / len * side, (b.0 - a.0) / len * side);
        let mut t0 = 0.0;
        for t1 in ts.into_iter().chain([1.0]) {
            let m = at((t0 + t1) / 2.0);
            let outside = winding_cw(&ring, (m.0 + nx * eps, m.1 + ny * eps)) <= 0;
            if outside {
                if cur.is_empty() {
                    // the run starts where an earlier section ends: that section goes on
                    match sections.iter().position(|s| dist(*s.1.last().unwrap(), at(t0)) <= eps) {
                        Some(k) => (cur_loc, cur) = sections.remove(k),
                        None => {
                            cur_loc = i as f64 + t0;
                            cur.push(at(t0));
                        }
                    }
                }
                cur.push(at(t1));
            } else {
                close(&mut cur, cur_loc, &mut sections);
            }
            t0 = t1;
        }
    }
    close(&mut cur, cur_loc, &mut sections);
    sections.retain(|s| s.1.len() >= 2);
    geos_sections(sections.into_iter().map(|s| s.1).collect(), &raw_closed, md)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The results of shapely 2.1.2 (GEOS 3.13.1), `LineString(pts).offset_curve(d)`: the parts and their points.
    const CASES: &[(&[P], f64, &[&[P]])] = &[
        (&[(0.0, 0.0), (10.0, 0.0)], 1.0, &[&[(0.0, 1.0), (10.0, 1.0)]]),
        (&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], -1.0, &[&[(0.0, -1.0), (10.0, -1.0), (10.09801714032956, -0.9951847266721968), (10.195090322016128, -0.9807852804032304), (10.290284677254462, -0.9569403357322089), (10.38268343236509, -0.9238795325112867), (10.471396736825998, -0.8819212643483549), (10.555570233019603, -0.8314696123025452), (10.634393284163645, -0.773010453362737), (10.707106781186548, -0.7071067811865475), (10.773010453362737, -0.6343932841636455), (10.831469612302545, -0.5555702330196022), (10.881921264348355, -0.47139673682599764), (10.923879532511286, -0.3826834323650898), (10.95694033573221, -0.29028467725446233), (10.98078528040323, -0.19509032201612825), (10.995184726672196, -0.0980171403295606), (11.0, 0.0), (11.0, 10.0)]]),
        (&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], 1.0, &[&[(0.0, 1.0), (9.0, 1.0), (9.0, 10.0)]]),
        (&[(0.0, 0.0), (10.0, 0.0), (10.5, 0.4), (11.0, 0.0), (20.0, 0.0), (20.0, 1.0), (20.2, -3.0), (25.0, 5.0)], 1.6, &[&[(0.0, 1.6), (9.43875030502686, 1.6), (9.500487923912921, 1.6493900951088487), (9.625371987064094, 1.7397857436873214), (9.75837622053984, 1.8177426316998542), (9.898265794202386, 1.8825369963589829), (10.043741953440662, 1.9335672776079496), (10.19345407699202, 1.9703597030894504), (10.346012216301293, 1.9925726867152909), (10.5, 2.0), (10.653987783698707, 1.9925726867152909), (10.80654592300798, 1.9703597030894504), (10.956258046559338, 1.9335672776079496), (11.101734205797614, 1.8825369963589829), (11.241623779460161, 1.8177426316998537), (11.374628012935906, 1.7397857436873219), (11.499512076087079, 1.6493900951088487), (11.56124969497314, 1.6), (18.51781352201637, 1.6), (18.525630695732374, 1.6214782012535869), (18.594834147134797, 1.76518555000837), (18.67800193905648, 1.9012885924394967), (18.774307561684168, 2.0284347556628903), (18.88279394329174, 2.145360478999684), (19.00238296148434, 2.2509037710644435), (19.131886157427015, 2.3440157574720493), (19.2700165465814, 2.4237711044037438), (19.41540140857696, 2.4893772144444126), (19.56659592911287, 2.540182103304173), (19.722097558319238, 2.5756808791464945), (19.880360942886256, 2.5955207601322314), (20.03981328356798, 2.5995045803158976), (20.198869965439894, 2.587592749053086), (20.355950305579057, 2.5599036444467256), (20.509493261668567, 2.516712436922149), (20.657972945416457, 2.458448354622128), (20.79991378661881, 2.3856904177979867), (20.93390519716882, 2.2991616845878236), (21.05861558928291, 2.199722065366476), (21.172805608632746, 2.0883597770772173), (21.275340450874175, 1.966181522470833), (21.295801085813356, 1.9361761536064184), (23.62801131885993, 5.823193208684042)]]),
        (&[(0.0, 0.0), (10.0, 0.0), (10.5, 0.4), (11.0, 0.0), (20.0, 0.0), (20.0, 1.0), (20.2, -3.0), (25.0, 5.0)], -1.6, &[&[(0.0, -1.6), (10.0, -1.6), (10.153987783698707, -1.592572686715291), (10.30654592300798, -1.5703597030894503), (10.456258046559338, -1.5335672776079494), (10.5, -1.5182234288416363), (10.543741953440662, -1.5335672776079494), (10.69345407699202, -1.5703597030894503), (10.846012216301293, -1.592572686715291), (11.0, -1.6), (18.52800124843994, -1.6), (18.60199625779545, -3.079900187110227), (18.617510599274862, -3.2360662970281755), (18.648249322488212, -3.389961324802491), (18.693916704724185, -3.54010471918985), (18.754073401706833, -3.6850520216365745), (18.828140674318984, -3.823408762730899), (18.915405956377427, -3.953843877733358), (19.01502970989501, -4.075102512120821), (19.12605350187836, -4.186018093948093), (19.247409224958208, -4.285523556884617), (19.37792937314499, -4.372661605954727), (19.516358273851544, -4.446593927219647), (19.66136416812492, -4.506609252799226), (19.81155202286911, -4.552130203643664), (19.965476951798387, -4.582718844224148), (20.121658116004557, -4.598080895703357), (20.278592970407423, -4.598068567052909), (20.43477171903003, -4.582681976880916), (20.588691840031366, -4.552069152290912), (20.738872540758045, -4.506524604783131), (20.883869003749748, -4.446486496898719), (21.022286286644366, -4.372532426865251), (21.152792742258427, -4.285373871797489), (21.274132829734523, -4.185849342912709), (21.385139193505843, -4.074916318611043), (21.4847438938717, -3.953642033028844), (21.57198868114007, -3.8231932086840423), (26.37198868114007, 4.176806791315958)]]),
        (&[(0.0, 0.0), (5.0, 0.01), (10.0, 0.0), (10.2, 3.0), (10.4, 0.0), (30.0, 0.5), (30.0, 0.6), (29.0, 0.2)], 2.0, &[&[(-0.003999992000024001, 1.999996000012), (4.996000007999976, 2.009996000012), (5.003999992000024, 2.009996000012), (8.129143530412623, 2.003745712935175), (8.204429684286781, 3.1330380210475473), (8.226710248939046, 3.325772249213775), (8.267560903876037, 3.5154407238195313), (8.326597214754056, 3.700258526711917), (8.403263606993908, 3.878486388083198), (8.496838594141344, 4.048447054263384), (8.606441567578806, 4.208541071895906), (8.731041083692343, 4.357261839955811), (8.869464570505427, 4.493209787960049), (9.02040936243321, 4.6151055469426066), (9.182454959312143, 4.7218019892460195), (9.354076394338612, 4.812295023826098), (9.53365858511454, 4.885733045478188), (9.719511532746234, 4.941424949060902), (9.909886225961857, 4.978846633297625), (10.102991100578276, 4.997645932950328), (10.297008899421721, 4.997645932950328), (10.490113774038141, 4.978846633297625), (10.680488467253763, 4.941424949060902), (10.866341414885456, 4.885733045478189), (11.045923605661386, 4.812295023826098), (11.217545040687854, 4.7218019892460195), (11.379590637566787, 4.6151055469426066), (11.53053542949457, 4.49320978796005), (11.668958916307655, 4.357261839955813), (11.793558432421193, 4.208541071895908), (11.903161405858654, 4.048447054263386), (11.996736393006088, 3.8784863880831995), (12.073402785245941, 3.7002585267119183), (12.132439096123962, 3.5154407238195327), (12.173289751060953, 3.3257722492137765), (12.195570315713217, 3.1330380210475486), (12.267886129048092, 2.0483008210244256), (29.33481517015973, 2.483681663909927)], &[(30.15624126283894, -1.491569417718226), (29.742781352708207, -1.6569533817705189)]]),
        (&[(0.0, 0.0), (5.0, 0.01), (10.0, 0.0), (10.2, 3.0), (10.4, 0.0), (30.0, 0.5), (30.0, 0.6), (29.0, 0.2)], -2.0, &[&[(0.003999992000024001, -1.999996000012), (5.0, -1.9900039999960002), (9.996000007999976, -1.999996000012), (10.196512845255269, -1.9903222607531874), (10.20674584906416, -1.9887901051055088), (10.259907554174639, -1.995087493475579), (10.451003814973017, -1.9993495469422546), (28.033498683509144, -1.550816514581639)], &[(30.15624126283894, -1.491569417718226), (30.243564698953065, -1.4851136585656506), (30.43385541656713, -1.452375342375375), (30.620102346314457, -1.401439738749379), (30.800569557177543, -1.3327815974961537), (30.97357498852106, -1.2470408529070562), (31.1375061278871, -1.145016659192026), (31.290835040560463, -1.0276599418919332), (31.432132610819618, -0.8960645346927858), (31.56008186213645, -0.7514569842519039), (31.673490232172732, -0.5951851180610779), (31.77130068816311, -0.4287054819009626), (31.852601579082673, -0.25356976397702036), (31.916635132771162, -0.07141033227206217), (31.96280451881553, 0.11607498008422817), (31.990679411360738, 0.30713869961947404), (32.0, 0.5), (32.0, 0.6), (31.990488592875156, 0.7948208449675949), (31.96204483836611, 0.9877886695622964), (31.91493927660217, 1.1770680782423888), (31.849619947759106, 1.3608587574915656), (31.766708130566666, 1.5374125993337362), (31.666992433073638, 1.7050503282996834), (31.551421291875606, 1.8621774736997287), (31.42109395114839, 2.0072995352836065), (31.277250007289148, 2.139036198040796), (31.121257618610368, 2.2561344609385454), (30.95460049222915, 2.3574805547248214), (30.77886377192476, 2.44211053544111), (30.595718961190805, 2.5092194528858514), (30.40690802488497, 2.5581690068235203), (30.21422682069209, 2.5884936181180365), (30.019508017990542, 2.5999048570454746), (29.82460366658769, 2.592294186666597), (29.63136758112013, 2.565733995165871), (29.44163770866795, 2.5204769073379727), (29.33481517015973, 2.483681663909927)]]),
        (&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.2)], 0.4, &[&[(0.4000000000000001, 0.4), (0.6, 0.4), (0.6, 0.6), (0.4000000000000001, 0.6), (0.4000000000000001, 0.4)]]),
        (&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.2)], -0.4, &[&[(0.0, -0.4), (1.0, -0.4), (1.0392068561318244, -0.39807389066887877), (1.0780361288064513, -0.3923141121612922), (1.1161138709017848, -0.3827761342928836), (1.1530733729460358, -0.36955181300451473), (1.1885586947303992, -0.352768505739342), (1.2222280932078409, -0.3325878449210181), (1.2537573136654583, -0.3092041813450948), (1.2828427124746191, -0.282842712474619), (1.3092041813450948, -0.25375731366545823), (1.3325878449210182, -0.22222809320784087), (1.3527685057393422, -0.18855869473039907), (1.3695518130045148, -0.15307337294603593), (1.3827761342928835, -0.11611387090178493), (1.392314112161292, -0.0780361288064513), (1.3980738906688788, -0.039206856131824244), (1.4, 0.0), (1.4, 1.0), (1.3980738906688788, 1.0392068561318242), (1.392314112161292, 1.0780361288064513), (1.3827761342928837, 1.1161138709017848), (1.3695518130045148, 1.1530733729460358), (1.352768505739342, 1.188558694730399), (1.3325878449210182, 1.2222280932078409), (1.3092041813450948, 1.2537573136654583), (1.2828427124746191, 1.282842712474619), (1.2537573136654583, 1.3092041813450948), (1.2222280932078409, 1.3325878449210182), (1.1885586947303992, 1.352768505739342), (1.1530733729460358, 1.3695518130045148), (1.1161138709017848, 1.3827761342928837), (1.0780361288064513, 1.392314112161292), (1.0392068561318244, 1.3980738906688788), (1.0, 1.4), (0.0, 1.4), (-0.03920685613182426, 1.3980738906688788), (-0.07803612880645128, 1.392314112161292), (-0.11611387090178488, 1.3827761342928837), (-0.1530733729460359, 1.3695518130045148), (-0.1885586947303991, 1.3527685057393422), (-0.2222280932078408, 1.3325878449210182), (-0.2537573136654582, 1.3092041813450948), (-0.282842712474619, 1.2828427124746191), (-0.3092041813450948, 1.2537573136654583), (-0.33258784492101817, 1.2222280932078409), (-0.352768505739342, 1.1885586947303992), (-0.36955181300451473, 1.1530733729460358), (-0.38277613429288354, 1.116113870901785), (-0.3923141121612922, 1.0780361288064515), (-0.39807389066887877, 1.0392068561318244), (-0.4000000000000001, 1.0), (-0.4000000000000001, 0.2)]]),
    ];

    /// The same parts and points as shapely, each point within 1 mm (these lines are in metres).
    #[test]
    fn as_shapely() {
        for (pts, d, want) in CASES {
            let got = offset_curve(pts, *d);
            let ok = got.len() == want.len()
                && got.iter().zip(want.iter()).all(|(g, w)| g.len() == w.len() && g.iter().zip(w.iter()).all(|(p, q)| dist(*p, *q) < 1e-3));
            assert!(ok, "d = {d}, line {pts:?}\n rust:    {got:?}\n shapely: {want:?}");
        }
    }
}
