//! The offset curve of a line, in pure Rust: the port of GEOS's `OffsetCurve` (shapely's `offset_curve(d)` with its
//! defaults: 16 segments for a quarter circle, round joins). A positive distance is on the left.
//!
//! GEOS makes the raw offset curve (`OffsetCurveBuilder::getOffsetCurve`) and the buffer polygon of the line, then keeps
//! the parts of the buffer's rings that lie on the raw curve (`OffsetCurve::computeSections`). Here the raw curve and the
//! raw ring of the buffer (`computeLineBufferCurve`) are made with the same steps, and `geos::buffer_polygons` makes
//! the buffer polygon from the raw ring as GEOS's `BufferBuilder` does.

use crate::geos::{dist, intersection_count, orient, point_to_segment, segment_intersection};
use std::f64::consts::{FRAC_PI_2, PI};

pub use crate::geos::P;

/// shapely's default `quad_segs` of `offset_curve`.
const QUADRANT_SEGMENTS: f64 = 16.0;
const CURVE_VERTEX_SNAP_DISTANCE_FACTOR: f64 = 1.0e-4;
const OFFSET_SEGMENT_SEPARATION_FACTOR: f64 = 1.0e-3;
const INSIDE_TURN_VERTEX_SNAP_DISTANCE_FACTOR: f64 = 1.0e-3;
const SIMPLIFY_FACTOR: f64 = 100.0;
const CLOSING_SEG_LEN_FACTOR: f64 = 80.0;
const CW: i32 = -1;
const CCW: i32 = 1;

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

/// GEOS `OffsetSegmentGenerator`. `side`: 1 the left side, -1 the right side (a ring's right side).
struct Gen {
    d: f64,
    side: f64,
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
            side: 1.0,
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
        self.off1 = Gen::offset(a, b, self.side, self.d);
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
            // separate sin and cos as GEOS (Angle::sinCosSnap): glibc's sincos, which sin_cos (or LLVM, from sin and
            // cos of the same angle) calls, can differ from sin in the last bit. black_box keeps the two calls apart.
            let (mut s, mut c) = (a.sin(), std::hint::black_box(a).cos());
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
        self.off0 = Gen::offset(self.s0, self.s1, self.side, self.d);
        self.off1 = Gen::offset(self.s1, self.s2, self.side, self.d);
        let o = orient(self.s0, self.s1, self.s2);
        let (s1, d) = (self.s1, self.d);
        let (o0, o1) = (self.off0, self.off1);
        if o == 0 {
            // collinear (GEOS addCollinear): a fillet only when the segments overlap (the line goes back on itself)
            if intersection_count(self.s0, s1, s1, self.s2) >= 2 {
                self.fillet(s1, o0.1, o1.0, CW, d);
            }
        } else if (o == CW && self.side > 0.0) || (o == CCW && self.side < 0.0) {
            // the outside turn
            if dist(o0.1, o1.0) < d * OFFSET_SEGMENT_SEPARATION_FACTOR {
                self.add(o0.1);
                return;
            }
            self.add(o0.1);
            self.fillet(s1, o0.1, o1.0, o, d);
            self.add(o1.0);
        } else if let Some(x) = segment_intersection(o0.0, o0.1, o1.0, o1.1) {
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
    /// GEOS `addLineEndCap`, round: the fillet from the angle of p0-p1 plus 90 degrees to the angle minus 90 degrees; flat: the
    /// two offset points only.
    fn end_cap(&mut self, p0: P, p1: P, flat: bool) {
        let l = Gen::offset(p0, p1, 1.0, self.d);
        let r = Gen::offset(p0, p1, -1.0, self.d);
        self.add(l.1);
        if !flat {
            let angle = (p1.1 - p0.1).atan2(p1.0 - p0.0);
            self.fillet_angles(p1, angle + FRAC_PI_2, angle - FRAC_PI_2, CW, self.d);
        }
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
    buffer_ring_cap(pts, d, false)
}

/// The buffer of a line by `d` (> 0), as shapely's `line.buffer(d)` (round joins, 16 segments for a quarter circle; `flat`:
/// `cap_style="flat"`, else round caps): GEOS `BufferOp` of a line.
pub fn line_buffer(pts: &[P], d: f64, flat: bool) -> Result<Vec<crate::geos::Polygon>, String> {
    let mut clean: Vec<P> = Vec::with_capacity(pts.len());
    for p in pts {
        if p.0.is_finite() && p.1.is_finite() && clean.last() != Some(p) {
            clean.push(*p);
        }
    }
    if clean.len() < 2 || d <= 0.0 {
        return Ok(vec![]);
    }
    let curves = if clean.len() >= 4 && clean.first() == clean.last() { ring_sides(&clean, d) } else { vec![(buffer_ring_cap(&clean, d, flat), -1)] };
    crate::geos::buffer_polygons(&curves)
}

fn buffer_ring_cap(pts: &[P], d: f64, flat: bool) -> Vec<P> {
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
    g.end_cap(s1[n1 - 1], s1[n1], flat);
    g.init_side(s2[n2], s2[n2 - 1]);
    for i in (1..n2).rev() {
        g.next_segment(s2[i - 1]);
    }
    let last = g.off1.1;
    g.add(last);
    g.end_cap(s2[1], s2[0], flat);
    if g.pts.first() != g.pts.last() {
        let f = g.pts[0];
        g.pts.push(f);
    }
    g.pts
}

/// GEOS `computeRingBufferCurve`: the curve of one side of a closed ring (`side` 1 left, -1 right), closed.
fn ring_curve(pts: &[P], d: f64, side: f64) -> Vec<P> {
    let tol = if side < 0.0 { -d / SIMPLIFY_FACTOR } else { d / SIMPLIFY_FACTOR };
    let s = simplify(pts, tol);
    let n = s.len() - 1;
    let mut g = Gen::new(d);
    g.side = side;
    g.init_side(s[n - 1], s[0]);
    for p in &s[1..=n] {
        g.next_segment(*p);
    }
    if g.pts.first() != g.pts.last() {
        let f = g.pts[0];
        g.pts.push(f);
    }
    g.pts
}

/// GEOS `isRingFullyEroded`.
fn fully_eroded(pts: &[P], is_hole: bool, d: f64) -> bool {
    let n = pts.len();
    if n < 4 {
        return true;
    }
    if n == 4 {
        let (p0, p1, p2) = (pts[0], pts[1], pts[2]);
        let (l0, l1, l2) = (dist(p1, p2), dist(p0, p2), dist(p0, p1));
        let c = l0 + l1 + l2;
        let centre = ((l0 * p0.0 + l1 * p1.0 + l2 * p2.0) / c, (l0 * p0.1 + l1 * p1.1 + l2 * p2.1) / c);
        return point_to_segment(centre, p0, p1) < d.abs();
    }
    let (minx, miny, maxx, maxy) = pts.iter().fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |e, p| (e.0.min(p.0), e.1.min(p.1), e.2.max(p.0), e.3.max(p.1)));
    let erodable = (is_hole && d > 0.0) || (!is_hole && d < 0.0);
    erodable && 2.0 * d.abs() > (maxy - miny).min(maxx - minx)
}

/// GEOS `Orientation::isCCWArea`.
fn is_ccw_area(pts: &[P]) -> bool {
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let x0 = pts[0].0;
    (1..n - 1).map(|i| (pts[i].0 - x0) * (pts[i - 1].1 - pts[i + 1].1)).sum::<f64>() / 2.0 < 0.0
}

/// The buffer of the polygon whose shell is `shell` (closed, no holes) by `d` (negative: inwards), as shapely's
/// `Polygon(shell).buffer(d)` (GEOS `BufferCurveSetBuilder::addPolygon`, round joins, 16 segments for a quarter circle).
pub fn polygon_buffer(shell: &[P], d: f64) -> Result<Vec<crate::geos::Polygon>, String> {
    let (od, mut side) = if d < 0.0 { (-d, -1.0) } else { (d, 1.0) };
    if d < 0.0 && fully_eroded(shell, false, d) {
        return Ok(vec![]);
    }
    let mut clean: Vec<P> = Vec::with_capacity(shell.len());
    for p in shell {
        if p.0.is_finite() && p.1.is_finite() && clean.last() != Some(p) {
            clean.push(*p);
        }
    }
    if d <= 0.0 && clean.len() < 3 {
        return Ok(vec![]);
    }
    let mut delta = -1;
    if clean.len() >= 4 && is_ccw_area(&clean) {
        delta = 1;
        side = -side;
    }
    let curve = ring_curve(&clean, od, side);
    if ring_curve_inverted(&clean, od, &curve) {
        return Ok(vec![]);
    }
    crate::geos::buffer_polygons(&[(curve, delta)])
}

/// GEOS `BufferCurveSetBuilder::addLinearRingSides` for a closed line: the curve of each side that the buffer has, with its
/// depth delta (left: -1, right: +1).
fn ring_sides(pts: &[P], d: f64) -> Vec<(Vec<P>, i32)> {
    let eroded = fully_eroded(pts, true, d);
    let hole = !eroded;
    let ccw = is_ccw_area(pts);
    let mut out = Vec::new();
    for (side, delta, want) in [(1.0, -1, !ccw || hole), (-1.0, 1, ccw || hole)] {
        if !want {
            continue;
        }
        let curve = ring_curve(pts, d, side);
        if !ring_curve_inverted(pts, d, &curve) {
            out.push((curve, delta));
        }
    }
    out
}

/// GEOS `isRingCurveInverted`: a small ring whose curve has no point at the buffer distance.
fn ring_curve_inverted(input: &[P], d: f64, curve: &[P]) -> bool {
    if d == 0.0 || input.len() <= 3 || input.len() >= 9 || curve.len() > 4 * input.len() {
        return false;
    }
    let tol = 0.99 * d.abs();
    let to_ring = |p: P| input.windows(2).map(|w| point_to_segment(p, w[0], w[1])).fold(dist(p, input[0]), f64::min);
    for i in 0..curve.len() {
        let v = curve[i];
        if to_ring(v) > tol {
            return false;
        }
        let nx = curve[if i < curve.len() - 1 { i + 1 } else { 0 }];
        if to_ring(((v.0 + nx.0) / 2.0, (v.1 + nx.1) / 2.0)) > tol {
            return false;
        }
    }
    true
}

const MATCH_DISTANCE_FACTOR: f64 = 10000.0;

/// GEOS `LineSegment::segmentFraction`: the projection factor of p on a-b, in 0..1.
fn segment_fraction(p: P, a: P, b: P) -> f64 {
    let f = if p == a {
        0.0
    } else if p == b || a == b {
        if p == b { 1.0 } else { 0.0 }
    } else {
        let len2 = (b.0 - a.0) * (b.0 - a.0) + (b.1 - a.1) * (b.1 - a.1);
        ((p.0 - a.0) * (b.0 - a.0) + (p.1 - a.1) * (b.1 - a.1)) / len2
    };
    f.clamp(0.0, 1.0)
}

/// GEOS `OffsetCurve::computeCurveSections` and `extractSections`: the runs of the buffer ring `ring` (closed) whose
/// segments lie on a segment of the raw curve (both ends within `md`), as (location on the raw curve, points).
fn curve_sections(ring: &[P], raw: &[P], md: f64, sections: &mut Vec<(f64, Vec<P>)>) {
    const NOT_IN_CURVE: f64 = -1.0;
    let n = ring.len() - 1;
    let mut loc = vec![NOT_IN_CURVE; n];
    let mut first: Option<usize> = None;
    let mut min_pos = -1.0;
    for (i, w) in raw.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let mut best: Option<usize> = None;
        let mut best_loc = -1.0;
        for j in 0..n {
            if md < point_to_segment(ring[j], a, b) || md < point_to_segment(ring[j + 1], a, b) {
                continue;
            }
            let l = i as f64 + segment_fraction(ring[j], a, b);
            loc[j] = l;
            if best_loc < 0.0 || l < best_loc {
                best_loc = l;
                best = Some(j);
            }
        }
        if let Some(j) = best {
            let pos = loc[j];
            if first.is_none() || pos < min_pos {
                min_pos = pos;
                first = Some(j);
            }
        }
    }
    let Some(start) = first else { return };
    let next = |i: usize| if i >= n - 1 { 0 } else { i + 1 };
    let prev = |i: usize| if i == 0 { n - 1 } else { i - 1 };
    let mut sec_start = start;
    loop {
        // findSectionEnd
        let mut end = sec_start;
        loop {
            let nx = next(end);
            if loc[nx] == NOT_IN_CURVE {
                end = nx;
                break;
            }
            end = nx;
            if end == sec_start || end == start {
                break;
            }
        }
        // OffsetCurveSection::create
        let len = if end <= sec_start { n + 1 - sec_start + end } else { end - sec_start + 1 };
        sections.push((loc[sec_start], (0..len).map(|i| ring[(sec_start + i) % n]).collect()));
        // findSectionStart
        let mut s = end;
        loop {
            let nx = next(s);
            if loc[s] != NOT_IN_CURVE && loc[prev(s)] == NOT_IN_CURVE {
                break;
            }
            s = nx;
            if s == end {
                break;
            }
        }
        sec_start = s;
        if sec_start == start || end == start {
            break;
        }
    }
}

/// The offset curve of the line `pts` at the distance `d` (left: positive), as shapely's `offset_curve(d)`: the
/// sections of the curve (more than one: a MultiLineString). Empty: GEOS gives an empty line. An error: GEOS's buffer
/// with the full precision fails (GEOS then uses less precision; not ported).
pub fn offset_curve(pts: &[P], d: f64) -> Result<Vec<Vec<P>>, String> {
    let length: f64 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
    if pts.len() < 2 || length == 0.0 {
        return Ok(vec![]);
    }
    if d == 0.0 {
        return Ok(vec![pts.to_vec()]);
    }
    if pts.len() == 2 {
        // GEOS LineSegment::offset
        let (a, b) = (pts[0], pts[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = (d * dx / len, d * dy / len);
        return Ok(vec![vec![(a.0 - uy, a.1 + ux), (b.0 - uy, b.1 + ux)]]);
    }
    let mut clean: Vec<P> = Vec::with_capacity(pts.len());
    for p in pts {
        if p.0.is_finite() && p.1.is_finite() && clean.last() != Some(p) {
            clean.push(*p);
        }
    }
    let raw = raw_curve(&clean, d);
    if raw.is_empty() {
        return Ok(vec![]);
    }
    // a closed line (GEOS's CoordinateSequence::isRing): the buffer of both sides of the ring
    let curves = if clean.len() >= 4 && clean.first() == clean.last() {
        ring_sides(&clean, d.abs())
    } else {
        vec![(buffer_ring(&clean, d.abs()), -1)]
    };
    let polys = crate::geos::buffer_polygons(&curves)?;
    // extractMaxAreaPolygon (the first of the largest)
    let mut best: Option<(f64, &crate::geos::Polygon)> = None;
    for p in &polys {
        let a = p.area();
        if best.is_none_or(|b| a > b.0) {
            best = Some((a, p));
        }
    }
    let Some((_, poly)) = best else { return Ok(vec![]) };
    let md = d.abs() / MATCH_DISTANCE_FACTOR;
    let orient_ring = |r: &Vec<P>| if d < 0.0 { r.iter().rev().copied().collect() } else { r.clone() };
    let mut sections = Vec::new();
    curve_sections(&orient_ring(&poly.shell), &raw, md, &mut sections);
    for h in &poly.holes {
        curve_sections(&orient_ring(h), &raw, md, &mut sections);
    }
    if sections.len() > 1 {
        sections.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    Ok(sections.into_iter().map(|s| s.1).collect())
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
            let got = offset_curve(pts, *d).unwrap();
            let ok = got.len() == want.len()
                && got.iter().zip(want.iter()).all(|(g, w)| g.len() == w.len() && g.iter().zip(w.iter()).all(|(p, q)| dist(*p, *q) < 1e-3));
            assert!(ok, "d = {d}, line {pts:?}\n rust:    {got:?}\n shapely: {want:?}");
        }
    }
}
