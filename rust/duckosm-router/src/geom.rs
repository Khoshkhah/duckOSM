//! The measures of a line that duckOSM's point routing takes from shapely (GEOS 3.13), with GEOS's own formulas, so that the
//! values are the same to the last bit: the distance from a point to a line, `project`, `interpolate` and the length.

pub type Pt = (f64, f64);

fn dist(a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    (dx * dx + dy * dy).sqrt()
}

/// GEOS `Distance::pointToSegment`.
pub fn point_to_segment(p: Pt, a: Pt, b: Pt) -> f64 {
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
    s.abs() * ((b.0 - a.0) * (b.0 - a.0) + (b.1 - a.1) * (b.1 - a.1)).sqrt()
}

/// shapely `line.distance(point)` (GEOS `DistanceOp`: the least distance to a segment).
pub fn distance(line: &[Pt], p: Pt) -> f64 {
    let mut d = f64::INFINITY;
    for w in line.windows(2) {
        let x = point_to_segment(p, w[0], w[1]);
        if x < d {
            d = x;
        }
    }
    d
}

/// shapely `line.length` (GEOS `Length::ofLine`).
pub fn length(line: &[Pt]) -> f64 {
    let mut len = 0.0;
    for w in line.windows(2) {
        let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        len += (dx * dx + dy * dy).sqrt();
    }
    len
}

/// GEOS `LineSegment::projectionFactor`.
fn projection_factor(p0: Pt, p1: Pt, q: Pt) -> f64 {
    if q == p0 {
        return 0.0;
    }
    if q == p1 {
        return 1.0;
    }
    if p0 == p1 {
        return 0.0;
    }
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len2 = dx * dx + dy * dy;
    ((q.0 - p0.0) * dx + (q.1 - p0.1) * dy) / len2
}

/// shapely `line.project(point)` (GEOS `LengthIndexOfPoint::indexOfFromStart` with no minimum).
pub fn project(line: &[Pt], p: Pt) -> f64 {
    let (mut min_d, mut measure, mut start) = (f64::INFINITY, -1.0, 0.0);
    for w in line.windows(2) {
        let (p0, p1) = (w[0], w[1]);
        let seg_len = dist(p1, p0);
        let d = point_to_segment(p, p0, p1);
        let f = projection_factor(p0, p1, p);
        let m = if f <= 0.0 {
            start
        } else if f <= 1.0 {
            start + f * seg_len
        } else {
            start + seg_len
        };
        if d < min_d && m > -1.0 {
            measure = m;
            min_d = d;
        }
        start += dist(p0, p1);
    }
    measure
}

/// shapely `line.interpolate(at)` (GEOS `LengthLocationMap::getLocationForward` and `pointAlongSegmentByFraction`).
pub fn interpolate(line: &[Pt], at: f64) -> Pt {
    if at <= 0.0 {
        return line[0];
    }
    let mut total = 0.0;
    for w in line.windows(2) {
        let (p0, p1) = (w[0], w[1]);
        let seg = dist(p1, p0);
        if total + seg > at {
            let f = (at - total) / seg;
            if f <= 0.0 {
                return p0;
            }
            if f >= 1.0 {
                return p1;
            }
            return ((p1.0 - p0.0) * f + p0.0, (p1.1 - p0.1) * f + p0.1);
        }
        total += seg;
    }
    *line.last().expect("a line has points")
}

/// The parts of segment `a b` and segment `c d` in common: a point, the two ends of a shared stretch, or nothing.
fn segment_intersection(a: Pt, b: Pt, c: Pt, d: Pt) -> Vec<Pt> {
    let cross = |u: Pt, v: Pt| u.0 * v.1 - u.1 * v.0;
    let sub = |u: Pt, v: Pt| (u.0 - v.0, u.1 - v.1);
    let (r, s) = (sub(b, a), sub(d, c));
    let den = cross(r, s);
    let qp = sub(c, a);
    if den == 0.0 {
        if cross(qp, r) != 0.0 {
            return Vec::new(); // parallel, apart
        }
        // on one line: the ends of the overlap
        let rr = r.0 * r.0 + r.1 * r.1;
        if rr == 0.0 {
            return if a == c || a == d { vec![a] } else { Vec::new() };
        }
        let t = |p: Pt| ((p.0 - a.0) * r.0 + (p.1 - a.1) * r.1) / rr;
        let (t0, t1) = (t(c).min(t(d)), t(c).max(t(d)));
        let (lo, hi) = (t0.max(0.0), t1.min(1.0));
        if lo > hi {
            return Vec::new();
        }
        return vec![(a.0 + lo * r.0, a.1 + lo * r.1), (a.0 + hi * r.0, a.1 + hi * r.1)];
    }
    let t = cross(qp, s) / den;
    let u = cross(qp, r) / den;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        vec![(a.0 + t * r.0, a.1 + t * r.1)]
    } else {
        Vec::new()
    }
}

/// duckOSM's test that the straight walk from (0, 0) to `p` crosses the line `blocker`: `not walk.intersection(blocker)
/// .difference(p.buffer(touch)).is_empty`, a part of the meeting farther than `touch` from `p`. (GEOS's buffer is a polygon of
/// 64 sides inside the circle: a meeting between that polygon and the circle counts as touching here.)
pub fn crosses(p: Pt, blocker: &[Pt], touch: f64) -> bool {
    blocker.windows(2).any(|w| segment_intersection((0.0, 0.0), p, w[0], w[1]).iter().any(|&q| dist(q, p) > touch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_values_of_shapely() {
        // shapely 2.1.2 (GEOS 3.13.1): l = LineString([(0, 0), (3, 0), (3, 4)]); p = Point(4, 1)
        let l = [(0.0, 0.0), (3.0, 0.0), (3.0, 4.0)];
        assert_eq!(distance(&l, (4.0, 1.0)), 1.0);
        assert_eq!(project(&l, (4.0, 1.0)), 4.0);
        assert_eq!(interpolate(&l, 4.0), (3.0, 1.0));
        assert_eq!(length(&l), 7.0);
        assert!(crosses((0.0, 10.0), &[(-1.0, 5.0), (1.0, 5.0)], 0.5));
        assert!(!crosses((0.0, 10.0), &[(-1.0, 9.8), (1.0, 9.8)], 0.5));
    }
}
