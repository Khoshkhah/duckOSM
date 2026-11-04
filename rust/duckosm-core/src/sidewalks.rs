//! The sidewalk rule (`SeparateSidewalks` in duckOSM): a car road with no sidewalk tag gets
//! `sidewalk=separate` when both its sides are covered. A side is covered by a `footway=sidewalk`
//! line beside the road, or by the other half of a dual carriageway.
//!
//! Distances are in metres in a local flat frame, as in duckOSM. A "spot" is a point on the road, every
//! `STEP_M` metres. The rule looks at the spots, not at the whole line.

use crate::model::Way;
use crate::network::Coords;
use rstar::{RTree, RTreeObject, AABB};

const SIDE_M: f64 = 12.0; // a sidewalk line this near the road line can be its sidewalk
const STEP_M: f64 = 5.0; // one spot every this many metres along the road
const ANGLE_DEG: f64 = 30.0; // the sidewalk runs within this angle of the road
const SHARE: f64 = 0.6; // a side has a sidewalk when this share of the spots find one there

const CAR_ROADS: [&str; 15] = [
    "residential",
    "service",
    "unclassified",
    "road",
    "tertiary",
    "tertiary_link",
    "secondary",
    "secondary_link",
    "primary",
    "primary_link",
    "trunk",
    "trunk_link",
    "motorway",
    "motorway_link",
    "living_street",
];
/// The classes walked on their carriageway without a sidewalk tag.
const WALKED: [&str; 12] = [
    "residential",
    "service",
    "unclassified",
    "road",
    "tertiary",
    "tertiary_link",
    "secondary",
    "secondary_link",
    "primary",
    "primary_link",
    "trunk",
    "trunk_link",
];

type P = (f64, f64);

struct Line {
    pts: Vec<P>,
    cum: Vec<f64>, // distance along the line at each vertex
    kind: u8,      // 2 sidewalk line, 1 road without a sidewalk tag, 0 other car road
    ow: i32,       // one-way: 1, -1 (against the line), 0
    lvl: i32,      // level: layer, else tunnel -1, bridge 1
}

impl Line {
    fn len(&self) -> f64 {
        *self.cum.last().unwrap()
    }

    /// The point at distance `d` along the line (clamped to the line).
    fn interp(&self, d: f64) -> P {
        let n = self.pts.len();
        let d = d.clamp(0.0, self.len());
        let mut i = 0;
        while i + 2 < n && d > self.cum[i + 1] {
            i += 1;
        }
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        let seg = self.cum[i + 1] - self.cum[i];
        if d >= self.cum[i + 1] || seg == 0.0 {
            return if d >= self.cum[i + 1] { b } else { a };
        }
        let t = (d - self.cum[i]) / seg;
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    }

    /// Distance along the line of the point nearest to `p` (the first nearest segment).
    fn locate(&self, p: P) -> f64 {
        let mut best = (f64::INFINITY, 0.0);
        for i in 0..self.pts.len() - 1 {
            let (a, b) = (self.pts[i], self.pts[i + 1]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l2 = dx * dx + dy * dy;
            let t = if l2 == 0.0 {
                0.0
            } else {
                (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
            };
            let q = (a.0 + dx * t, a.1 + dy * t);
            let d2 = (p.0 - q.0).powi(2) + (p.1 - q.1).powi(2);
            if d2 < best.0 {
                best = (d2, self.cum[i] + t * l2.sqrt());
            }
        }
        best.1
    }

    fn dist(&self, p: P) -> f64 {
        let q = self.interp(self.locate(p));
        ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()
    }

    fn envelope(&self) -> AABB<P> {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &self.pts {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
        AABB::from_corners((x0, y0), (x1, y1))
    }

    /// Does the segment `a-b` touch the line?
    fn touched_by(&self, a: P, b: P) -> bool {
        self.pts.windows(2).any(|w| segments_meet(a, b, w[0], w[1]))
    }
}

struct Item {
    env: AABB<P>,
    idx: usize,
}

impl RTreeObject for Item {
    type Envelope = AABB<P>;
    fn envelope(&self) -> Self::Envelope {
        self.env
    }
}

fn orient(a: P, b: P, c: P) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn on_segment(a: P, b: P, p: P) -> bool {
    p.0 >= a.0.min(b.0) && p.0 <= a.0.max(b.0) && p.1 >= a.1.min(b.1) && p.1 <= a.1.max(b.1)
}

/// Do the closed segments `a-b` and `c-d` meet (cross or touch)?
fn segments_meet(a: P, b: P, c: P, d: P) -> bool {
    let (o1, o2, o3, o4) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
        return true;
    }
    (o1 == 0.0 && on_segment(a, b, c))
        || (o2 == 0.0 && on_segment(a, b, d))
        || (o3 == 0.0 && on_segment(c, d, a))
        || (o4 == 0.0 && on_segment(c, d, b))
}

fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// The unit vector of `v`; the length is at least 1e-9 (as `norm.clip(1e-9)`).
fn unit(v: P) -> P {
    let n = (v.0 * v.0 + v.1 * v.1).sqrt().max(1e-9);
    (v.0 / n, v.1 / n)
}

fn level(w: &Way) -> i32 {
    let t = &w.tags;
    if let Some(l) = t.layer().and_then(|l| l.trim().parse::<i32>().ok()) {
        return l;
    }
    if t.tunnel().unwrap_or("no") != "no" {
        -1
    } else if t.bridge().unwrap_or("no") != "no" {
        1
    } else {
        0
    }
}

/// Find the roads that get `sidewalk=separate`, set the tag on them, and return how many.
pub fn infer_separate_sidewalks(ways: &mut [Way], coords: &Coords) -> usize {
    // 1. The lines: car roads and sidewalk lines (nodes without coordinates leave the line).
    let mut raw: Vec<(usize, Vec<P>, u8)> = Vec::new(); // (index in ways, points, kind)
    for (wi, w) in ways.iter().enumerate() {
        let t = &w.tags;
        let hw = t.highway().unwrap_or("");
        let is_sidewalk = hw == "footway" && t.footway() == Some("sidewalk");
        if !(CAR_ROADS.contains(&hw) || is_sidewalk) {
            continue;
        }
        let kind = if is_sidewalk {
            2
        } else if WALKED.contains(&hw)
            && !matches!(
                t.foot(),
                Some("yes") | Some("designated") | Some("permissive")
            )
            && t.sidewalk().is_none()
            && t.sidewalk_both().is_none()
            && t.sidewalk_left().is_none()
            && t.sidewalk_right().is_none()
        {
            1
        } else {
            0
        };
        let pts: Vec<P> = w
            .refs
            .iter()
            .filter_map(|n| coords.get(n).copied())
            .collect();
        if pts.len() >= 2 {
            raw.push((wi, pts, kind));
        }
    }
    if !raw.iter().any(|r| r.2 == 2) || !raw.iter().any(|r| r.2 == 1) {
        return 0;
    }
    // the local frame: metres, from the mean latitude of the lines (the mean of the means)
    let lat0 = raw
        .iter()
        .map(|r| r.1.iter().map(|p| p.1).sum::<f64>() / r.1.len() as f64)
        .sum::<f64>()
        / raw.len() as f64;
    let kx = 111320.0 * lat0.to_radians().cos();
    let ky = 111320.0;
    let lines: Vec<Line> = raw
        .iter()
        .map(|(wi, pts, kind)| {
            let pts: Vec<P> = pts.iter().map(|p| (p.0 * kx, p.1 * ky)).collect();
            let mut cum = vec![0.0];
            for w in pts.windows(2) {
                let l = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
                cum.push(cum.last().unwrap() + l);
            }
            let t = &ways[*wi].tags;
            let ow = if matches!(t.oneway(), Some("yes") | Some("true") | Some("1"))
                || t.junction() == Some("roundabout")
            {
                1
            } else if t.oneway() == Some("-1") {
                -1
            } else {
                0
            };
            Line {
                pts,
                cum,
                kind: *kind,
                ow,
                lvl: level(&ways[*wi]),
            }
        })
        .collect();
    let item = |i: usize| Item {
        env: lines[i].envelope(),
        idx: i,
    };
    let side_tree = RTree::bulk_load(
        (0..lines.len())
            .filter(|&i| lines[i].kind == 2)
            .map(item)
            .collect(),
    );
    let car_tree = RTree::bulk_load(
        (0..lines.len())
            .filter(|&i| lines[i].kind != 2)
            .map(item)
            .collect(),
    );
    let around = |p: P, r: f64| AABB::from_corners((p.0 - r, p.1 - r), (p.0 + r, p.1 + r));
    let cos_a = ANGLE_DEG.to_radians().cos();

    // 2. The spots: every STEP_M along each road without a sidewalk tag (at least its middle).
    let roads: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].kind == 1).collect();
    let mut found: Vec<usize> = Vec::new();
    for &k in &roads {
        let road = &lines[k];
        let n = road.len();
        let ats: Vec<f64> = if n > STEP_M {
            let mut v = Vec::new();
            let mut i = 0.0;
            while STEP_M / 2.0 + i * STEP_M < n {
                v.push(STEP_M / 2.0 + i * STEP_M);
                i += 1.0;
            }
            v
        } else {
            vec![n / 2.0]
        };
        let (mut left_n, mut right_n) = (0usize, 0usize);
        for &at in &ats {
            let pt = road.interp(at);
            let ahead = road.interp((at + 0.5).min(n));
            let back = road.interp((at - 0.5).max(0.0));
            let t = unit((ahead.0 - back.0, ahead.1 - back.1));
            let (mut left, mut right) = (false, false);

            // each sidewalk line near the spot
            for it in side_tree.locate_in_envelope_intersecting(&around(pt, SIDE_M)) {
                let sw = &lines[it.idx];
                if sw.dist(pt) > SIDE_M {
                    continue;
                }
                let loc = sw.locate(pt);
                let near = sw.interp(loc);
                let v = (near.0 - pt.0, near.1 - pt.1);
                let side = sign(t.0 * v.1 - t.1 * v.0);
                let d = unit({
                    let a = sw.interp(loc + 0.5);
                    let b = sw.interp((loc - 0.5).max(0.0));
                    (a.0 - b.0, a.1 - b.1)
                });
                let ok =
                    (d.0 * t.0 + d.1 * t.1).abs() >= cos_a && side != 0.0 && sw.lvl == road.lvl;
                if !ok {
                    continue;
                }
                // no other car road between the spot and the sidewalk
                let blocked = car_tree
                    .locate_in_envelope_intersecting(&AABB::from_corners(
                        (pt.0.min(near.0), pt.1.min(near.1)),
                        (pt.0.max(near.0), pt.1.max(near.1)),
                    ))
                    .any(|c| c.idx != k && lines[c.idx].touched_by(pt, near));
                if !blocked {
                    if side > 0.0 {
                        left = true;
                    } else {
                        right = true;
                    }
                }
            }

            // the other half of a dual carriageway
            for it in car_tree.locate_in_envelope_intersecting(&around(pt, SIDE_M)) {
                let c = &lines[it.idx];
                if it.idx == k || c.dist(pt) > SIDE_M {
                    continue;
                }
                let cl = c.locate(pt);
                let cn = c.interp(cl);
                let cv = (cn.0 - pt.0, cn.1 - pt.1);
                let cside = sign(t.0 * cv.1 - t.1 * cv.0);
                let cd = unit({
                    let a = c.interp(cl + 0.5);
                    let b = c.interp((cl - 0.5).max(0.0));
                    (a.0 - b.0, a.1 - b.1)
                });
                let (oc, oo) = (road.ow as f64, c.ow as f64);
                let twin = (cd.0 * t.0 + cd.1 * t.1) * oc * oo <= -cos_a
                    && road.ow != 0
                    && c.ow != 0
                    && c.lvl == road.lvl
                    && cside * oc > 0.0
                    && (cv.0 * cv.0 + cv.1 * cv.1).sqrt() > 1.0;
                if twin {
                    if cside > 0.0 {
                        left = true;
                    } else if cside < 0.0 {
                        right = true;
                    }
                }
            }
            left_n += left as usize;
            right_n += right as usize;
        }
        let spots = ats.len().max(1) as f64;
        if left_n as f64 / spots >= SHARE && right_n as f64 / spots >= SHARE {
            found.push(k);
        }
    }

    // 3. Tag the roads. (lines[k] is made from raw[k])
    let n = found.len();
    for k in found {
        // duckOSM: map_concat(tags, MAP {'sidewalk': 'separate', 'duckosm:sidewalk': 'inferred'})
        ways[raw[k].0].set_tag("sidewalk", "separate");
        ways[raw[k].0].set_tag("duckosm:sidewalk", "inferred");
    }
    n
}
