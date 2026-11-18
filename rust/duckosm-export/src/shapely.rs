//! The shapely (GEOS 3.13.1) functions of lines that the GMNS export uses, in pure Rust, with GEOS's formulas: the length,
//! `interpolate` and `project` (GEOS's linear referencing), and the distance of a point or a line to a line (`DistanceOp`).
//! Also a WKT reader of a LINESTRING and Python's `round`.

pub use crate::geos::P;

pub use crate::geos::{dist, point_to_segment};

/// `Distance::segmentToSegment`.
pub fn segment_to_segment(a: P, b: P, c: P, d: P) -> f64 {
    if a == b {
        return point_to_segment(a, c, d);
    }
    if c == d {
        return point_to_segment(d, a, b);
    }
    let envs_meet = !(a.0.min(b.0) > c.0.max(d.0) || a.0.max(b.0) < c.0.min(d.0) || a.1.min(b.1) > c.1.max(d.1) || a.1.max(b.1) < c.1.min(d.1));
    let mut no_intersection = !envs_meet;
    if envs_meet {
        let denom = (b.0 - a.0) * (d.1 - c.1) - (b.1 - a.1) * (d.0 - c.0);
        if denom == 0.0 {
            no_intersection = true;
        } else {
            let r_num = (a.1 - c.1) * (d.0 - c.0) - (a.0 - c.0) * (d.1 - c.1);
            let s_num = (a.1 - c.1) * (b.0 - a.0) - (a.0 - c.0) * (b.1 - a.1);
            let (s, r) = (s_num / denom, r_num / denom);
            if !(0.0..=1.0).contains(&r) || !(0.0..=1.0).contains(&s) {
                no_intersection = true;
            }
        }
    }
    if no_intersection {
        point_to_segment(a, c, d).min(point_to_segment(b, c, d).min(point_to_segment(c, a, b).min(point_to_segment(d, a, b))))
    } else {
        0.0
    }
}

/// `LineString::getLength`.
pub fn length(l: &[P]) -> f64 {
    l.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// `line.interpolate(d)` (GEOS `LengthIndexedLine::extractPoint`): the point at the distance `d` along the line.
pub fn interpolate(l: &[P], d: f64) -> P {
    let d = if d < 0.0 { length(l) + d } else { d };
    if d <= 0.0 {
        return l[0];
    }
    let mut total = 0.0;
    for w in l.windows(2) {
        let seg = dist(w[1], w[0]);
        if total + seg > d {
            let f = (d - total) / seg;
            return if f <= 0.0 {
                w[0]
            } else if f >= 1.0 {
                w[1]
            } else {
                ((w[1].0 - w[0].0) * f + w[0].0, (w[1].1 - w[0].1) * f + w[0].1)
            };
        }
        total += seg;
    }
    l[l.len() - 1]
}

/// `LineSegment::projectionFactor`.
fn projection_factor(p: P, a: P, b: P) -> f64 {
    if p == a {
        return 0.0;
    }
    if p == b {
        return 1.0;
    }
    if a == b {
        return 0.0;
    }
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    ((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2
}

/// `line.project(p)` (GEOS `LengthIndexOfPoint::indexOf`): the distance along the line of its point nearest to p.
pub fn project(l: &[P], p: P) -> f64 {
    let (mut min, mut measure, mut start) = (f64::INFINITY, -1.0, 0.0);
    for w in l.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = point_to_segment(p, a, b);
        let len = dist(a, b);
        let f = projection_factor(p, a, b);
        let m = if f <= 0.0 {
            start
        } else if f <= 1.0 {
            start + f * len
        } else {
            start + len
        };
        if d < min && m > -1.0 {
            measure = m;
            min = d;
        }
        start += len;
    }
    measure
}

/// The distance of the point p to the line.
pub fn point_line_distance(p: P, l: &[P]) -> f64 {
    if l.len() == 1 {
        return dist(p, l[0]);
    }
    l.windows(2).map(|w| point_to_segment(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

/// The distance of two lines (`DistanceOp`): the least distance of two of their segments.
pub fn line_distance(a: &[P], b: &[P]) -> f64 {
    let mut min = f64::INFINITY;
    for s in a.windows(2) {
        for t in b.windows(2) {
            let d = segment_to_segment(s[0], s[1], t[0], t[1]);
            if d < min {
                min = d;
                if min <= 0.0 {
                    return 0.0;
                }
            }
        }
    }
    min
}

/// The points of a WKT `LINESTRING (x y, ...)` (a `POINT` gives one point).
pub fn wkt_points(wkt: &str) -> Vec<P> {
    let body = wkt.trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == ' ').trim_start_matches('(').trim_end_matches(')');
    body.split(',')
        .filter_map(|p| {
            let mut it = p.split_whitespace().map(|v| v.parse::<f64>());
            match (it.next(), it.next()) {
                (Some(Ok(x)), Some(Ok(y))) => Some((x, y)),
                _ => None,
            }
        })
        .collect()
}

/// The order in which shapely's `STRtree(geoms)` (GEOS `TemplateSTRtree`, 10 children for each node) gives its items: the
/// leaves of the tree, depth first. A query gives the items whose envelope meets the query, in this order. `envs`: the
/// envelope (min x, min y, max x, max y) of each item.
pub fn strtree_order(envs: &[(f64, f64, f64, f64)]) -> Vec<usize> {
    const CAP: usize = 10;
    // (envelope, children range in `nodes`, item)
    let mut nodes: Vec<((f64, f64, f64, f64), (usize, usize), usize)> = envs.iter().enumerate().map(|(i, e)| (*e, (0, 0), i)).collect();
    if nodes.is_empty() {
        return vec![];
    }
    let mut begin = 0;
    let mut number = nodes.len();
    while number > 1 {
        let min_leaf = (number as f64 / CAP as f64).ceil();
        let slices = min_leaf.sqrt().ceil() as usize;
        let per_slice = (number as f64 / slices as f64).ceil() as usize;
        let end = begin + number;
        cpp_sort(&mut nodes[begin..end], &|a, b| a.0 .0 + a.0 .2 < b.0 .0 + b.0 .2);
        let mut start = begin;
        for _ in 0..slices {
            let in_slice = (end - start).min(per_slice);
            let slice_end = start + in_slice;
            cpp_sort(&mut nodes[start..slice_end], &|a, b| a.0 .1 + a.0 .3 < b.0 .1 + b.0 .3);
            let mut first = start;
            while first != slice_end {
                let last = first + CAP.min(slice_end - first);
                let env = nodes[first..last].iter().fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |e, n| {
                    (e.0.min(n.0 .0), e.1.min(n.0 .1), e.2.max(n.0 .2), e.3.max(n.0 .3))
                });
                nodes.push((env, (first, last), usize::MAX));
                first = last;
            }
            start = slice_end;
        }
        begin = end;
        number = nodes.len() - begin;
    }
    let mut out = Vec::with_capacity(envs.len());
    let mut stack = vec![nodes.len() - 1];
    while let Some(n) = stack.pop() {
        let (_, (a, b), item) = nodes[n];
        if item != usize::MAX {
            out.push(item);
        } else {
            stack.extend((a..b).rev());
        }
    }
    out
}

/// C++ `std::sort` as libstdc++ does it (introsort, then insertion sort), so that equal items end in the same order as in
/// GEOS. `less`: the comparator.
pub fn cpp_sort<T: Copy>(v: &mut [T], less: &dyn Fn(&T, &T) -> bool) {
    let n = v.len();
    if n < 2 {
        return;
    }
    let depth = 2 * (usize::BITS - 1 - n.leading_zeros()) as usize;
    introsort_loop(v, 0, n, depth, less);
    // __final_insertion_sort
    if n > 16 {
        insertion_sort(v, 0, 16, less);
        for i in 16..n {
            unguarded_linear_insert(v, i, less);
        }
    } else {
        insertion_sort(v, 0, n, less);
    }
}

fn introsort_loop<T: Copy>(v: &mut [T], first: usize, mut last: usize, mut depth: usize, less: &dyn Fn(&T, &T) -> bool) {
    while last - first > 16 {
        if depth == 0 {
            // partial_sort(first, last, last): a heap sort
            make_heap(v, first, last, less);
            let mut l = last;
            while l - first > 1 {
                l -= 1;
                let value = v[l];
                v[l] = v[first];
                adjust_heap(v, first, 0, l - first, value, less);
            }
            return;
        }
        depth -= 1;
        // __unguarded_partition_pivot
        let mid = first + (last - first) / 2;
        let (a, b, c) = (first + 1, mid, last - 1);
        let pick = if less(&v[a], &v[b]) {
            if less(&v[b], &v[c]) {
                b
            } else if less(&v[a], &v[c]) {
                c
            } else {
                a
            }
        } else if less(&v[a], &v[c]) {
            a
        } else if less(&v[b], &v[c]) {
            c
        } else {
            b
        };
        v.swap(first, pick);
        let (mut lo, mut hi) = (first + 1, last);
        let cut = loop {
            while less(&v[lo], &v[first]) {
                lo += 1;
            }
            hi -= 1;
            while less(&v[first], &v[hi]) {
                hi -= 1;
            }
            if lo >= hi {
                break lo;
            }
            v.swap(lo, hi);
            lo += 1;
        };
        introsort_loop(v, cut, last, depth, less);
        last = cut;
    }
}

fn insertion_sort<T: Copy>(v: &mut [T], first: usize, last: usize, less: &dyn Fn(&T, &T) -> bool) {
    for i in first + 1..last {
        if less(&v[i], &v[first]) {
            let val = v[i];
            v.copy_within(first..i, first + 1);
            v[first] = val;
        } else {
            unguarded_linear_insert(v, i, less);
        }
    }
}

fn unguarded_linear_insert<T: Copy>(v: &mut [T], mut last: usize, less: &dyn Fn(&T, &T) -> bool) {
    let val = v[last];
    while less(&val, &v[last - 1]) {
        v[last] = v[last - 1];
        last -= 1;
    }
    v[last] = val;
}

fn adjust_heap<T: Copy>(v: &mut [T], first: usize, mut hole: usize, len: usize, value: T, less: &dyn Fn(&T, &T) -> bool) {
    let top = hole;
    let mut second = hole;
    while second < (len.saturating_sub(1)) / 2 {
        second = 2 * (second + 1);
        if less(&v[first + second], &v[first + second - 1]) {
            second -= 1;
        }
        v[first + hole] = v[first + second];
        hole = second;
    }
    if len & 1 == 0 && len >= 2 && second == (len - 2) / 2 {
        second = 2 * (second + 1);
        v[first + hole] = v[first + second - 1];
        hole = second - 1;
    }
    // __push_heap
    while hole > top {
        let parent = (hole - 1) / 2;
        if !less(&v[first + parent], &value) {
            break;
        }
        v[first + hole] = v[first + parent];
        hole = parent;
    }
    v[first + hole] = value;
}

fn make_heap<T: Copy>(v: &mut [T], first: usize, last: usize, less: &dyn Fn(&T, &T) -> bool) {
    let len = last - first;
    if len < 2 {
        return;
    }
    let mut parent = (len - 2) / 2;
    loop {
        let value = v[first + parent];
        adjust_heap(v, first, parent, len, value, less);
        if parent == 0 {
            return;
        }
        parent -= 1;
    }
}

/// Python's `round(x, n)`: the decimal rounding of the binary value, half to even.
pub fn py_round(x: f64, n: usize) -> f64 {
    format!("{x:.n$}").parse().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_shapely() {
        let l = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert_eq!(length(&l), 20.0);
        assert_eq!(interpolate(&l, 15.0), (10.0, 5.0));
        assert_eq!(interpolate(&l, 25.0), (10.0, 10.0));
        assert_eq!(project(&l, (12.0, 4.0)), 14.0);
        assert_eq!(line_distance(&l, &[(12.0, 3.0), (15.0, 3.0)]), 2.0);
        assert_eq!(wkt_points("LINESTRING (1 2, 3.5 -4)"), vec![(1.0, 2.0), (3.5, -4.0)]);
        assert_eq!(py_round(2.675, 2), 2.67);
        assert_eq!(py_round(0.125, 2), 0.12);
        let mut v: Vec<i32> = (0..100).map(|i| (i * 37) % 23).collect();
        cpp_sort(&mut v, &|a, b| a < b);
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
    }
}
