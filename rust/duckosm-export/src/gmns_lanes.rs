//! The geometry of the GMNS lanes, the port of the geometry parts of src/duckosm/gmns.py: the runs of pieces
//! (`_chain_runs`), the paired carriageways (`_paired_gaps`), the lanes of a run from one offset curve (`_run_lane_wkts`), and
//! the lane lines moved near a joint or along a varying gap (`_taper_wkt`, `_straight_wkt`, `_vary_wkt`). Lines are in a local
//! metre frame at a first point, as in Python.

use crate::shapely::{self as shp, P};
use std::collections::{HashMap, HashSet};

pub const LANE_TAPER_PER_M: f64 = 30.0 / 3.25;
const M: f64 = 111320.0;

/// `math.cos(math.radians(y)) or 1.0`.
pub fn kx_of(y: f64) -> f64 {
    match y.to_radians().cos() {
        0.0 => 1.0,
        k => k,
    }
}

/// A lon/lat line in the metre frame at (x0, y0).
pub fn to_local(l: &[P], x0: f64, y0: f64) -> Vec<P> {
    let kx = kx_of(y0);
    l.iter().map(|(x, y)| ((x - x0) * kx * M, (y - y0) * M)).collect()
}

/// A line of the metre frame at (x0, y0) back to lon/lat.
pub fn to_lonlat(l: &[P], x0: f64, y0: f64) -> Vec<P> {
    let kx = kx_of(y0);
    l.iter().map(|(x, y)| (x / (kx * M) + x0, y / M + y0)).collect()
}

/// WKT of a line, with the shortest text of each number that reads back as the same number (as shapely writes it).
pub fn wkt(l: &[P]) -> String {
    let n = |v: f64| {
        if v == v.trunc() && v.abs() < 1e15 {
            format!("{}", v as i64)
        } else {
            format!("{v:?}")
        }
    };
    format!("LINESTRING ({})", l.iter().map(|(x, y)| format!("{} {}", n(*x), n(*y))).collect::<Vec<_>>().join(", "))
}

/// `_placement`: OSM `placement` -> where the way's line lies across its lanes, metres from the left edge of lane 1.
pub fn placement(value: Option<&str>, w_each: &[f64]) -> Option<f64> {
    let v = value?;
    let mut it = v.split(':');
    let (kind, n) = (it.next()?, it.next()?);
    if it.next().is_some() {
        return None;
    }
    let n: i64 = n.trim().parse().ok()?;
    if n < 1 || n as usize > w_each.len() {
        return None;
    }
    let left: f64 = w_each[..n as usize - 1].iter().sum();
    let w = w_each[n as usize - 1];
    match kind {
        "left_of" => Some(left),
        "middle_of" => Some(left + w / 2.0),
        "right_of" => Some(left + w),
        _ => None,
    }
}

/// The heading of a way at its start and at its end, degrees, from its first and last two points (`headings`).
pub fn headings(pts: &[P]) -> (f64, f64) {
    if pts.len() < 2 {
        return (0.0, 0.0);
    }
    let k = pts[0].1.to_radians().cos();
    let h = |p: P, q: P| (q.1 - p.1).atan2((q.0 - p.0) * k).to_degrees();
    (h(pts[0], pts[1]), h(pts[pts.len() - 2], pts[pts.len() - 1]))
}

/// The facts of an edge that `_chain_runs` reads: (osm_id, is_reverse, source, target, oneway, lane widths, name, roundabout,
/// heading in, heading out, the other direction's width rounded to 2 decimals).
#[derive(Clone)]
pub struct Info {
    pub osm_id: i64,
    pub is_rev: bool,
    pub source: i64,
    pub target: i64,
    pub oneway: bool,
    pub widths: Vec<f64>,
    pub name: Option<String>,
    pub roundabout: bool,
    pub h0: f64,
    pub h1: f64,
    pub opp: f64,
}

/// Python's `a % b` for floats (the sign of b).
pub fn pymod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 && ((b < 0.0) != (m < 0.0)) {
        m + b
    } else {
        m
    }
}

/// `_chain_runs`: the runs of pieces in `order` (the edges in the order of their rows), the run and place of each edge, and the
/// runs that are rings.
pub fn chain_runs(order: &[i64], info: &HashMap<i64, Info>) -> (Vec<Vec<i64>>, HashMap<i64, (usize, usize)>, HashSet<usize>) {
    let mut at_src: HashMap<i64, Vec<i64>> = HashMap::new();
    for e in order {
        at_src.entry(info[e].source).or_default().push(*e);
    }
    let one_road = |a: i64, b: i64| {
        let (ea, eb) = (&info[&a], &info[&b]);
        if ea.oneway != eb.oneway || ea.widths != eb.widths || ea.opp != eb.opp {
            return false;
        }
        if (ea.osm_id, ea.is_rev) == (eb.osm_id, eb.is_rev) || (ea.roundabout && eb.roundabout) {
            return true;
        }
        let turn = (pymod(eb.h0 - ea.h1 + 180.0, 360.0) - 180.0).abs();
        ea.name.as_deref().is_some_and(|n| !n.is_empty()) && ea.name == eb.name && turn < 30.0
    };
    let mut nxt: Vec<(i64, i64)> = Vec::new();
    for e in order {
        let c: Vec<i64> = at_src.get(&info[e].target).map(|v| v.iter().copied().filter(|f| f != e && one_road(*e, *f)).collect()).unwrap_or_default();
        if c.len() == 1 {
            nxt.push((*e, c[0]));
        }
    }
    let mut taken: HashMap<i64, usize> = HashMap::new();
    for (_, f) in &nxt {
        *taken.entry(*f).or_default() += 1;
    }
    let nxt: HashMap<i64, i64> = nxt.into_iter().filter(|(_, f)| taken[f] == 1).collect();
    let prv: HashSet<i64> = nxt.values().copied().collect();
    let mut runs: Vec<Vec<i64>> = Vec::new();
    let mut seen: HashSet<i64> = HashSet::new();
    for start in order.iter().filter(|e| !prv.contains(e)).chain(order.iter()) {
        if seen.contains(start) {
            continue;
        }
        let mut run = Vec::new();
        let mut x = Some(*start);
        while let Some(e) = x.filter(|e| !seen.contains(e)) {
            seen.insert(e);
            run.push(e);
            x = nxt.get(&e).copied();
        }
        runs.push(run);
    }
    let closed: HashSet<usize> = runs.iter().enumerate().filter(|(_, r)| r.len() > 1 && nxt.get(r.last().unwrap()) == r.first()).map(|(i, _)| i).collect();
    let mut wh = HashMap::new();
    for (i, run) in runs.iter().enumerate() {
        for (k, e) in run.iter().enumerate() {
            wh.insert(*e, (i, k));
        }
    }
    (runs, wh, closed)
}

/// The direction of a line at the distance s along it, as a unit vector, from the points 0.5 m before and after (`tangent`).
fn tangent(l: &[P], len: f64, s: f64) -> P {
    let (a, b) = (shp::interpolate(l, (s - 0.5).max(0.0)), shp::interpolate(l, (s + 0.5).min(len)));
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let n = match dx.hypot(dy) {
        0.0 => 1.0,
        n => n,
    };
    (dx / n, dy / n)
}

/// `_paired_gaps`: for the one-way lines `edges` (key, lon/lat points, half width), the median gap to an opposite one-way
/// partner on its inner side, and the profile of the gap along it. `side_sign`: -1 for right-hand traffic.
pub fn paired_gaps(edges: &[(usize, Vec<P>, f64)], side_sign: f64, profiles: &mut HashMap<usize, Vec<(f64, f64)>>) -> HashMap<usize, f64> {
    const STEP: f64 = 2.0;
    let mut gaps = HashMap::new();
    if edges.is_empty() {
        return gaps;
    }
    let (x0, y0) = edges[0].1[0];
    let lines: Vec<Vec<P>> = edges.iter().map(|e| to_local(&e.1, x0, y0)).collect();
    let lens: Vec<f64> = lines.iter().map(|l| shp::length(l)).collect();
    let half: Vec<f64> = edges.iter().map(|e| e.2).collect();
    let envs: Vec<(f64, f64, f64, f64)> = lines.iter().map(|l| envelope(l)).collect();
    let tree = shp::strtree_order(&envs);
    let reach = half.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    for (i, a) in lines.iter().enumerate() {
        if lens[i] < STEP {
            continue;
        }
        // tree.query(A.buffer(d)): the lines whose envelope meets the envelope of the buffer
        let be = envelope(&crate::offset::buffer_ring(a, half[i] + reach));
        let cand: Vec<usize> = tree.iter().copied().filter(|&j| j != i && envs_meet(envs[j], be)).collect();
        if cand.is_empty() {
            continue;
        }
        let (mut found, mut prof) = (Vec::new(), Vec::new());
        let nk = (lens[i] / STEP).floor() as i64 + 1;
        for k in 0..nk {
            let s = (k as f64 * STEP).min(lens[i]);
            let p = shp::interpolate(a, s);
            let (tx, ty) = tangent(a, lens[i], s);
            let mut best: Option<f64> = None;
            for &j in &cand {
                let b = &lines[j];
                let sb = shp::project(b, p);
                let q = shp::interpolate(b, sb);
                let d = shp::dist(p, q);
                if d >= half[i] + half[j] || best.is_some_and(|x| d >= x) {
                    continue;
                }
                let (bx, by) = tangent(b, lens[j], sb);
                let cross = tx * (q.1 - p.1) - ty * (q.0 - p.0);
                if tx * bx + ty * by < -0.866 && cross * side_sign < 0.0 {
                    best = Some(d);
                }
            }
            if let Some(b) = best {
                found.push(b);
            }
            prof.push((s, best.unwrap_or(2.0 * half[i])));
        }
        if found.len() >= 3 {
            gaps.insert(edges[i].0, shp::median(&found));
            let g: Vec<f64> = prof.iter().map(|x| x.1).collect();
            let sm = (0..prof.len())
                .map(|k| {
                    let w = &g[k.saturating_sub(1)..(k + 2).min(g.len())];
                    (prof[k].0, w.iter().sum::<f64>() / w.len() as f64)
                })
                .collect();
            profiles.insert(edges[i].0, sm);
        }
    }
    gaps
}

pub fn envelope(l: &[P]) -> (f64, f64, f64, f64) {
    l.iter().fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |e, p| (e.0.min(p.0), e.1.min(p.1), e.2.max(p.0), e.3.max(p.1)))
}

fn envs_meet(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    !(a.0 > b.2 || a.2 < b.0 || a.1 > b.3 || a.3 < b.1)
}

/// The left normals of a line's points, from `numpy.gradient` (a zero tangent counts as length 1).
fn left_normals(pts: &[P]) -> Vec<P> {
    shp::gradient(pts)
        .into_iter()
        .map(|(tx, ty)| {
            let n = match tx.hypot(ty) {
                0.0 => 1.0,
                n => n,
            };
            (-ty / n, tx / n)
        })
        .collect()
}

/// `_taper_wkt`: a lane line moved sideways by `d0` at its start fading to 0 over `l0` metres, and by `d1` at its end fading
/// over the last `l1` metres. None: the input line stays.
pub fn taper(lane: &[P], d0: f64, l0: f64, d1: f64, l1: f64) -> Option<Vec<P>> {
    let (x0, y0) = lane[0];
    let pts = shp::segmentize(&to_local(lane, x0, y0), 1.0);
    if pts.len() < 2 {
        return None;
    }
    let mut s = vec![0.0];
    for w in pts.windows(2) {
        let last = *s.last().unwrap();
        s.push(last + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
    }
    let total = *s.last().unwrap();
    let left = left_normals(&pts);
    let moved: Vec<P> = pts
        .iter()
        .zip(&s)
        .zip(&left)
        .map(|((p, si), n)| {
            let a = if l0 > 0.0 { d0 * (1.0 - si / l0).clamp(0.0, 1.0) } else { 0.0 };
            let b = if l1 > 0.0 { d1 * (1.0 - (total - si) / l1).clamp(0.0, 1.0) } else { 0.0 };
            let d = a + b;
            (p.0 + n.0 * d, p.1 + n.1 * d)
        })
        .collect();
    Some(to_lonlat(&moved, x0, y0))
}

/// `_vary_wkt`: a paired edge's lane line moved so its direction's lanes start from the local midline: each point goes along
/// the left normal by `-side * (g(s) - g0) / 2`, `g(s)` the gap at its place along the run.
pub fn vary(lane: &[P], run: &[P], profile: &[(f64, f64)], g0: f64, side_sign: f64) -> Option<Vec<P>> {
    let (x0, y0) = run[0];
    let ln = shp::segmentize(&to_local(lane, x0, y0), 2.0);
    let rn = to_local(run, x0, y0);
    if ln.len() < 2 || profile.len() < 2 {
        return None;
    }
    let (s, g): (Vec<f64>, Vec<f64>) = profile.iter().copied().unzip();
    let left = left_normals(&ln);
    let moved: Vec<P> = ln
        .iter()
        .zip(&left)
        .map(|(p, n)| {
            let gv = shp::interp(shp::project(&rn, *p), &s, &g);
            let d = -side_sign * (gv - g0) / 2.0;
            (p.0 + n.0 * d, p.1 + n.1 * d)
        })
        .collect();
    Some(to_lonlat(&moved, x0, y0))
}

/// `numpy.linspace(0, 1, n)`.
fn linspace01(n: usize) -> Vec<f64> {
    if n == 1 {
        return vec![0.0];
    }
    let step = 1.0 / (n - 1) as f64;
    (0..n).map(|i| if i == n - 1 { 1.0 } else { i as f64 * step }).collect()
}

/// `_straight_wkt`: a lane line that starts (or ends) at `p0` heading along the unit vector `t` and comes to its own line in one
/// cubic Hermite curve at least `length_m` long.
pub fn straight(lane: &[P], p0: P, t: P, length_m: f64, at_start: bool) -> Option<Vec<P>> {
    let cs: Vec<P> = if at_start { lane.to_vec() } else { lane.iter().rev().copied().collect() };
    let (x0, y0) = p0;
    let own = to_local(&cs, x0, y0);
    let olen = shp::length(&own);
    let e_v = shp::interpolate(&own, length_m.min(olen));
    let shift = (-t.1 * e_v.0 + t.0 * e_v.1).abs();
    let l = length_m.max(shift * LANE_TAPER_PER_M).min(0.8 * olen);
    let e = shp::interpolate(&own, l);
    let (a, b) = (shp::interpolate(&own, (l - 1.0).max(0.0)), shp::interpolate(&own, (l + 1.0).min(olen)));
    let n = match (b.0 - a.0).hypot(b.1 - a.1) {
        0.0 => 1.0,
        n => n,
    };
    let te = ((b.0 - a.0) / n, (b.1 - a.1) / n);
    let mut pts: Vec<P> = linspace01(((l as i64) + 1).max(3) as usize)
        .into_iter()
        .map(|u| {
            // numpy: u**2 is u * u, u**3 calls pow
            let (u2, u3) = (u * u, u.powf(3.0));
            let (h00, h10, h01, h11) = (2.0 * u3 - 3.0 * u2 + 1.0, u3 - 2.0 * u2 + u, -2.0 * u3 + 3.0 * u2, u3 - u2);
            // h10 * L * t0 + h01 * e + h11 * L * te + h00 * 0.0, as numpy adds the arrays left to right
            (h10 * l * t.0 + h01 * e.0 + h11 * l * te.0 + h00 * 0.0, h10 * l * t.1 + h01 * e.1 + h11 * l * te.1 + h00 * 0.0)
        })
        .collect();
    let rest = shp::substring(&own, l, olen);
    pts.extend(rest.into_iter().skip(1));
    let mut out = to_lonlat(&pts, x0, y0);
    if !at_start {
        out.reverse();
    }
    Some(out)
}

/// `_run_lane_wkts`: the lanes of a run, each offset once from the run's merged line and cut back into the pieces at the joints:
/// for each piece, the lon/lat line of each offset in `offs`. None: the geometry will not do (the caller offsets each piece).
pub fn run_lane_wkts(pieces: &[Vec<P>], offs: &[f64], closed: bool) -> crate::Result<Option<Vec<Vec<Vec<P>>>>> {
    if pieces.iter().any(|p| p.len() < 2) {
        return Ok(None);
    }
    let n = pieces.len();
    let (x0, y0) = pieces[0][0];
    let loc: Vec<Vec<P>> = pieces.iter().map(|p| to_local(p, x0, y0)).collect();
    let (first, last) = (loc[0][0], *loc[n - 1].last().unwrap());
    let ring = closed && n > 1 && (first == last || shp::dist(first, last) < 0.01);
    let order: Vec<Vec<P>> = if ring {
        let half = shp::length(&loc[0]) / 2.0;
        let (head, tail) = (shp::substring(&loc[0], 0.0, half), shp::substring(&loc[0], half, shp::length(&loc[0])));
        std::iter::once(tail).chain(loc[1..].iter().cloned()).chain(std::iter::once(head)).collect()
    } else {
        loc.clone()
    };
    let mut coords = order[0].clone();
    let mut joints = Vec::new();
    for ln in &order[1..] {
        let end = *coords.last().unwrap();
        joints.push(end);
        let skip = usize::from(shp::dist(ln[0], end) < 0.01);
        coords.extend_from_slice(&ln[skip..]);
    }
    if ring {
        coords = smooth_ring(&coords, true);
    }
    let merged = coords.clone();
    let mut out: Vec<Vec<Vec<P>>> = vec![Vec::new(); n];
    for &off in offs {
        let line = if off.abs() <= 1e-6 {
            merged.clone()
        } else if ring {
            // the ring's polygon, buffered in or out (left + is inside a counter-clockwise ring)
            if !simple_ring(&coords) || crate::geos::signed_area(&coords).abs() < 1.0 {
                return Ok(None);
            }
            let ccw = crate::geos::is_ccw(&coords);
            let polys = crate::offset::polygon_buffer(&coords, if (off > 0.0) == ccw { -off.abs() } else { off.abs() })?;
            if polys.len() != 1 {
                return Ok(None);
            }
            let mut bd = polys[0].shell.clone();
            if crate::geos::is_ccw(&bd) != ccw {
                bd.reverse();
            }
            let blen = shp::length(&bd);
            let s0 = shp::project(&bd, coords[0]);
            let mut l = shp::substring(&bd, s0, blen);
            l.extend(shp::substring(&bd, 0.0, s0).into_iter().skip(1));
            l
        } else {
            let parts = crate::offset::offset_curve(&merged, off)?;
            // the first of the longest parts
            let mut best: Option<&Vec<P>> = None;
            for p in &parts {
                if best.is_none_or(|b| shp::length(p) > shp::length(b)) {
                    best = Some(p);
                }
            }
            match best {
                Some(b) => b.clone(),
                None => return Ok(None),
            }
        };
        if line.len() < 2 {
            return Ok(None);
        }
        let mut ds = vec![0.0];
        ds.extend(joints.iter().map(|j| shp::project_tol(&line, *j, 1e-9)));
        ds.push(shp::length(&line));
        if ds.windows(2).any(|w| w[1] - w[0] < 0.3) {
            return Ok(None);
        }
        let mut segs: Vec<Vec<P>> = ds.windows(2).map(|w| shp::substring(&line, w[0], w[1])).collect();
        if ring {
            // the first piece: its head (at the end) and its tail (at the start)
            let last = segs.pop().unwrap();
            let first = segs.remove(0);
            let mut joined = last;
            joined.extend(first.into_iter().skip(1));
            segs.insert(0, joined);
        }
        for (i, seg) in segs.iter().enumerate() {
            if seg.len() < 2 {
                return Ok(None);
            }
            out[i].push(to_lonlat(seg, x0, y0));
        }
    }
    Ok(Some(out))
}

/// A polygon shell that GEOS calls valid, as far as a roundabout's ring needs: closed, finite, at least 4 points and 3 distinct
/// ones, and no two segments that meet but where they are neighbours (`Polygon.is_valid`).
pub fn simple_ring(r: &[P]) -> bool {
    if r.len() < 4 || r.first() != r.last() || r.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return false;
    }
    let mut pts: Vec<P> = r.to_vec();
    pts.dedup();
    if pts.len() < 4 {
        return false;
    }
    let n = pts.len() - 1;
    for i in 0..n {
        for j in i + 1..n {
            let adjacent = j == i + 1 || (i == 0 && j == n - 1);
            let k = crate::geos::intersection_count(pts[i], pts[i + 1], pts[j], pts[j + 1]);
            if k == 0 {
                continue;
            }
            if !adjacent || k > 1 {
                return false;
            }
        }
    }
    true
}

/// The least-squares solution x of A x = b for an n x 3 matrix A (Householder QR).
fn lstsq3(a: &[[f64; 3]], b: &[f64]) -> Option<[f64; 3]> {
    let n = a.len();
    let mut m: Vec<[f64; 4]> = a.iter().zip(b).map(|(r, v)| [r[0], r[1], r[2], *v]).collect();
    for k in 0..3 {
        let norm = (k..n).map(|i| m[i][k] * m[i][k]).sum::<f64>().sqrt();
        if norm == 0.0 {
            return None;
        }
        let alpha = if m[k][k] > 0.0 { -norm } else { norm };
        let mut v: Vec<f64> = (k..n).map(|i| m[i][k]).collect();
        v[0] -= alpha;
        let vv: f64 = v.iter().map(|x| x * x).sum();
        if vv == 0.0 {
            continue;
        }
        for c in k..4 {
            let dot: f64 = (k..n).map(|i| v[i - k] * m[i][c]).sum();
            let f = 2.0 * dot / vv;
            for i in k..n {
                m[i][c] -= f * v[i - k];
            }
        }
    }
    let mut x = [0.0; 3];
    for k in (0..3).rev() {
        let s: f64 = (k + 1..3).map(|j| m[k][j] * x[j]).sum();
        x[k] = (m[k][3] - s) / m[k][k];
    }
    Some(x)
}

/// `_smooth_ring`: the closed ring (local metres) as points on its circle when it is one: the vertices kept, and points put on
/// the circle between them at most 3 degrees apart. `free_first`: the first point is a seam, put on the circle, not fitted.
pub fn smooth_ring(coords: &[P], free_first: bool) -> Vec<P> {
    const STEP_DEG: f64 = 3.0;
    const MIN_RADIUS: f64 = 3.0;
    let n = coords.len();
    if n < 5 || (coords[0].0 - coords[n - 1].0).hypot(coords[0].1 - coords[n - 1].1) > 0.01 {
        return coords.to_vec();
    }
    let mut pts = coords.to_vec();
    let mut xy: Vec<P> = coords[..n - 1].to_vec();
    let fit: Vec<P> = if free_first { xy[1..].to_vec() } else { xy.clone() };
    let a: Vec<[f64; 3]> = fit.iter().map(|p| [2.0 * p.0, 2.0 * p.1, 1.0]).collect();
    let b: Vec<f64> = fit.iter().map(|p| p.0 * p.0 + p.1 * p.1).collect();
    let Some([cx, cy, k]) = lstsq3(&a, &b) else { return coords.to_vec() };
    let r2 = k + cx * cx + cy * cy;
    if r2 <= MIN_RADIUS * MIN_RADIUS {
        return coords.to_vec();
    }
    let r = r2.sqrt();
    let worst = fit.iter().map(|p| ((p.0 - cx).hypot(p.1 - cy) - r).abs()).fold(f64::NEG_INFINITY, f64::max);
    if worst > (0.04 * r).max(0.25) {
        return coords.to_vec();
    }
    if free_first {
        let v = (xy[0].0 - cx, xy[0].1 - cy);
        let h = v.0.hypot(v.1);
        xy[0] = (cx + r * v.0 / h, cy + r * v.1 / h);
        pts[0] = xy[0];
        pts[n - 1] = xy[0];
    }
    let ang: Vec<f64> = xy.iter().map(|p| (p.1 - cy).atan2(p.0 - cx)).collect();
    let tau = 2.0 * std::f64::consts::PI;
    let d: Vec<f64> = (0..ang.len()).map(|i| pymod(ang[(i + 1) % ang.len()] - ang[i] + std::f64::consts::PI, tau) - std::f64::consts::PI).collect();
    let sum: f64 = d.iter().sum();
    let sign = |x: f64| if x > 0.0 { 1 } else if x < 0.0 { -1 } else { 0 };
    if ((sum.abs() - tau).abs() > 1e-6) || d.iter().any(|x| sign(*x) != sign(sum)) || d.iter().fold(0.0f64, |m, x| m.max(x.abs())) > std::f64::consts::PI / 3.0 {
        return pts;
    }
    let mut out = Vec::new();
    for ((p, a0), da) in xy.iter().zip(&ang).zip(&d) {
        out.push(*p);
        let m = (da.to_degrees().abs() / STEP_DEG).ceil() as i64;
        for j in 1..m {
            let t = a0 + da * j as f64 / m as f64;
            out.push((cx + r * std::hint::black_box(t).cos(), cy + r * t.sin()));
        }
    }
    out.push(pts[n - 1]);
    out
}

/// The checks of the lane geometry of a GMNS db (docs/design/all_in_rust.md, "How the lane geometry is checked"): for each
/// `gmns_<mode>` schema, one line with the number of lanes, their total length in metres (to 1 mm), the lanes that cross
/// themselves, the pairs of lanes of one link that cross, and the lanes whose distance to the link's line is the same within
/// 1 mm along the middle half of the lane (the others are near a join, a taper or a varying gap).
pub fn lane_report(gmns_db: &std::path::Path) -> crate::Result<Vec<String>> {
    let con = duckdb::Connection::open(gmns_db).map_err(|e| e.to_string())?;
    con.execute_batch("LOAD spatial").map_err(|e| e.to_string())?;
    let q = |sql: &str| -> crate::Result<Vec<String>> {
        let mut st = con.prepare(sql).map_err(|e| format!("{e}\n{sql}"))?;
        let v = st.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<Result<Vec<String>, _>>();
        v.map_err(|e| e.to_string())
    };
    let schemas = q("SELECT DISTINCT table_schema FROM information_schema.tables WHERE table_name = 'lane' AND table_schema LIKE 'gmns\\_%' ESCAPE '\\' ORDER BY 1")?;
    let mut out = vec!["schema\tlanes\tlength_m\tnot_simple\tcrossing_pairs\tconstant_offset\tconnectors\tconnector_ends_off".to_string()];
    for s in schemas {
        let sql = format!(
            "SELECT l.lane_id, ST_AsText(l.geom), ST_AsText(k.geom) FROM {s}.lane l JOIN {s}.link k USING (link_id) WHERE l.geom IS NOT NULL ORDER BY l.lane_id"
        );
        let mut st = con.prepare(&sql).map_err(|e| e.to_string())?;
        let rows: Vec<(String, String, String)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let (mut total, mut constant) = (0.0, 0usize);
        for (_, lw, kw) in &rows {
            let (lane, link) = (shp::wkt_points(lw), shp::wkt_points(kw));
            if lane.len() < 2 || link.len() < 2 {
                continue;
            }
            let (x0, y0) = link[0];
            let (lane, link) = (to_local(&lane, x0, y0), to_local(&link, x0, y0));
            let len = shp::length(&lane);
            total += len;
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            let mut t = 0.25 * len;
            while t <= 0.75 * len {
                let d = shp::point_line_distance(shp::interpolate(&lane, t), &link);
                lo = lo.min(d);
                hi = hi.max(d);
                t += 1.0;
            }
            if hi - lo <= 1e-3 {
                constant += 1;
            }
        }
        let n = |sql: String| -> crate::Result<i64> { con.query_row(&sql, [], |r| r.get(0)).map_err(|e| format!("{e}\n{sql}")) };
        let not_simple = n(format!("SELECT count(*) FROM {s}.lane WHERE geom IS NOT NULL AND NOT ST_IsSimple(geom)"))?;
        let crossing = n(format!(
            "SELECT count(*) FROM {s}.lane a JOIN {s}.lane b ON a.link_id = b.link_id AND a.lane_id < b.lane_id WHERE ST_Intersects(a.geom, b.geom)"
        ))?;
        // the lane connectors meet the ends of the lanes they join (geom_cut: the lane line to the junction) within 1 mm
        let has_conn = n(format!("SELECT count(*) FROM information_schema.tables WHERE table_schema = '{s}' AND table_name = 'lane_connector'"))? > 0
            && n(format!("SELECT count(*) FROM information_schema.columns WHERE table_schema = '{s}' AND table_name = 'lane' AND column_name = 'geom_cut'"))? > 0;
        let (conns, off) = if has_conn {
            let sql = format!(
                "SELECT ST_AsText(c.geom), ST_AsText(a.geom_cut), ST_AsText(b.geom_cut) FROM {s}.lane_connector c
                 JOIN {s}.lane a ON a.lane_id = c.from_lane_id JOIN {s}.lane b ON b.lane_id = c.to_lane_id ORDER BY c.connector_id"
            );
            let mut st = con.prepare(&sql).map_err(|e| e.to_string())?;
            let rows: Vec<(String, Option<String>, Option<String>)> =
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
            let mut off = 0;
            for (cw, aw, bw) in &rows {
                let cp = shp::wkt_points(cw);
                let (Some(a), Some(b)) = (aw.as_deref().map(shp::wkt_points), bw.as_deref().map(shp::wkt_points)) else {
                    off += 1;
                    continue;
                };
                let k = |p: P, q: P| ((p.0 - q.0) * kx_of(p.1) * M).hypot((p.1 - q.1) * M);
                if k(cp[0], *a.last().unwrap()) > 1e-3 || k(*cp.last().unwrap(), b[0]) > 1e-3 {
                    off += 1;
                }
            }
            (rows.len().to_string(), off.to_string())
        } else {
            ("-".to_string(), "-".to_string())
        };
        out.push(format!("{s}\t{}\t{:.3}\t{not_simple}\t{crossing}\t{constant}\t{conns}\t{off}", rows.len(), total));
    }
    // the crossings: each lane stretch inside its lane (5 cm and 0.5 %: the stretch is in the flat metre frame of the crossings, the
    // lane length on the ellipsoid), each part across inside the zebra (1 mm), the width the tag's or
    // 2.5-4 m
    let xs = q("SELECT DISTINCT table_schema FROM information_schema.tables WHERE table_name = 'lane_crossing' AND table_schema LIKE 'gmns\\_%' ESCAPE '\\' ORDER BY 1")?;
    if !xs.is_empty() {
        out.push("schema\tcrossings\tlane_rows\tstretch_outside\tacross_outside\twidth_outside".to_string());
    }
    for s in xs {
        let n = |sql: String| -> crate::Result<i64> { con.query_row(&sql, [], |r| r.get(0)).map_err(|e| format!("{e}\n{sql}")) };
        let len = "ST_Length_Spheroid(ST_FlipCoordinates(g))";
        let row = |what: &str| format!("(SELECT lane_id, geom AS g FROM {s}.lane UNION ALL SELECT connector_id, geom FROM {s}.lane_connector) {what}");
        out.push(format!(
            "{s}\t{}\t{}\t{}\t{}\t{}",
            n(format!("SELECT count(*) FROM {s}.crossing"))?,
            n(format!("SELECT count(*) FROM {s}.lane_crossing"))?,
            n(format!(
                "SELECT count(*) FROM {s}.lane_crossing x JOIN {} USING (lane_id) WHERE x.start_lr < -0.05 OR x.end_lr > 1.005 * {len} + 0.05 OR x.start_lr >= x.end_lr",
                row("r(lane_id, g)")
            ))?,
            n(format!(
                "SELECT count(*) FROM {s}.lane_crossing x JOIN {s}.crossing c USING (crossing_id) WHERE x.across_from < -0.001 OR x.across_to > c.length + 0.001 OR x.across_from >= x.across_to"
            ))?,
            n(format!("SELECT count(*) FROM {s}.crossing WHERE width < 0.5 OR width > 20 OR (width NOT BETWEEN 2.5 AND 4.0 AND source = 'node')"))?,
        ));
    }
    Ok(out)
}
