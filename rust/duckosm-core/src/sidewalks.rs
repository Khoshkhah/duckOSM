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
const GROUP: usize = 16; // the segments of a line in groups of this many, each with a box (Line::locate)

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
    /// the segments in groups of `GROUP`: (the box of the group's points, its first segment, its end)
    groups: Vec<((f64, f64, f64, f64), usize, usize)>,
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
        // the first segment that ends at or after `d` (else the last): a binary search on the sorted distances, not a
        // walk from the start of the line, which made the rule take points x spots on each road
        let i = self.cum[1..n - 1].partition_point(|&c| d > c);
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        let seg = self.cum[i + 1] - self.cum[i];
        if d >= self.cum[i + 1] || seg == 0.0 {
            return if d >= self.cum[i + 1] { b } else { a };
        }
        let t = (d - self.cum[i]) / seg;
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    }

    /// The nearest point to `p` on the segments `first..end`: `best` gets (squared distance, distance along the line)
    /// when a segment is strictly nearer, so the first nearest segment wins.
    fn nearest_in(&self, p: P, first: usize, end: usize, best: &mut (f64, f64)) {
        for i in first..end {
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
                *best = (d2, self.cum[i] + t * l2.sqrt());
            }
        }
    }

}

/// A group of segments of a line (`Line::groups`) in a tree: a spot reads only the groups near it, not whole lines.
struct Group {
    env: AABB<P>,
    line: usize,
    group: usize,
}

impl RTreeObject for Group {
    type Envelope = AABB<P>;
    fn envelope(&self) -> Self::Envelope {
        self.env
    }
}

/// The lines with a group in `env`, each once, sorted by line, with their groups in `env` in the order of the line.
fn groups_in(tree: &RTree<Group>, env: &AABB<P>, buf: &mut Vec<(usize, usize)>) {
    buf.clear();
    buf.extend(tree.locate_in_envelope_intersecting(env).map(|g| (g.line, g.group)));
    buf.sort_unstable();
}

/// For each line in `found` (from [`groups_in`]): the line, its distance to `p` and the distance along it of its
/// nearest point. Only the segments of the found groups are read. When a line is within `r` of `p`, its nearest
/// segments are in found groups (their boxes are within `r`), so the first nearest segment is that of the whole line.
fn nearest_lines<'a>(
    lines: &'a [Line],
    found: &'a [(usize, usize)],
    p: P,
) -> impl Iterator<Item = (usize, f64, f64)> + 'a {
    found.chunk_by(|a, b| a.0 == b.0).map(move |gs| {
        let line = &lines[gs[0].0];
        let mut best = (f64::INFINITY, 0.0);
        for &(_, g) in gs {
            let (_, first, end) = line.groups[g];
            line.nearest_in(p, first, end, &mut best);
        }
        let q = line.interp(best.1);
        (gs[0].0, ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt(), best.1)
    })
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

/// The degrees of a line to metres in the flat frame of a road at the cosine `c` of its latitude.
const M_PER_DEG: f64 = 111320.0;

/// A line in a flat frame: its points scaled by `kx` and `ky`, with its distances and its groups.
fn line_in(pts: &[P], kind: u8, ow: i32, lvl: i32, kx: f64, ky: f64) -> Line {
    let pts: Vec<P> = pts.iter().map(|p| (p.0 * kx, p.1 * ky)).collect();
    let mut cum = vec![0.0];
    for w in pts.windows(2) {
        let l = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        cum.push(cum.last().unwrap() + l);
    }
    let groups = (0..pts.len() - 1)
        .step_by(GROUP)
        .map(|first| {
            let end = (first + GROUP).min(pts.len() - 1);
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for q in &pts[first..=end] {
                (x0, y0, x1, y1) = (x0.min(q.0), y0.min(q.1), x1.max(q.0), y1.max(q.1));
            }
            ((x0, y0, x1, y1), first, end)
        })
        .collect();
    Line { groups, pts, cum, kind, ow, lvl }
}

/// The trees of the groups of the sidewalk lines and of the car roads.
fn trees(lines: &[Line]) -> (RTree<Group>, RTree<Group>) {
    let tree = |side: bool| {
        RTree::bulk_load(
            (0..lines.len())
                .filter(|&i| (lines[i].kind == 2) == side)
                .flat_map(|i| {
                    lines[i].groups.iter().enumerate().map(move |(g, &((x0, y0, x1, y1), ..))| Group {
                        env: AABB::from_corners((x0, y0), (x1, y1)),
                        line: i,
                        group: g,
                    })
                })
                .collect(),
        )
    };
    (tree(true), tree(false))
}

/// The kind of a way in the rule: 2 a sidewalk line, 1 a road without a sidewalk tag (the rule decides it), 0 another car
/// road; None when the rule does not read it.
pub fn kind(w: &Way) -> Option<u8> {
    let t = &w.tags;
    let hw = t.highway().unwrap_or("");
    let is_sidewalk = hw == "footway" && t.footway() == Some("sidewalk");
    if !(CAR_ROADS.contains(&hw) || is_sidewalk) {
        return None;
    }
    Some(if is_sidewalk {
        2
    } else if WALKED.contains(&hw)
        && !matches!(t.foot(), Some("yes") | Some("designated") | Some("permissive"))
        && t.sidewalk().is_none()
        && t.sidewalk_both().is_none()
        && t.sidewalk_left().is_none()
        && t.sidewalk_right().is_none()
    {
        1
    } else {
        0
    })
}

/// Set `sidewalk=separate` on the ways `found` (the result of [`separate_sidewalks`]).
pub fn mark(ways: &mut [Way], found: &[usize]) {
    for &i in found {
        // duckOSM: map_concat(tags, MAP {'sidewalk': 'separate', 'duckosm:sidewalk': 'inferred'})
        ways[i].set_tag("sidewalk", "separate");
        ways[i].set_tag("duckosm:sidewalk", "inferred");
    }
}

/// Find the roads that get `sidewalk=separate`, set the tag on them, and return how many.
pub fn infer_separate_sidewalks(ways: &mut [Way], coords: &Coords) -> usize {
    let found = separate_sidewalks(ways, coords, |_| true);
    mark(ways, &found);
    found.len()
}

/// The roads of `ways` that get `sidewalk=separate` (their places in `ways`), of the roads for which `decide` is true.
///
/// Each road has its own flat frame, from the mean latitude of its points: the result of a road depends only on the road
/// and on the lines within `SIDE_M` of it (a build by tiles gives the same result as the whole build).
pub fn separate_sidewalks(ways: &[Way], coords: &Coords, decide: impl Fn(&Way) -> bool) -> Vec<usize> {
    // 1. The lines: car roads and sidewalk lines (nodes without coordinates leave the line), in degrees.
    // (index in ways, points, kind, one-way, level)
    let mut raw: Vec<(usize, Vec<P>, u8, i32, i32)> = Vec::new();
    for (wi, w) in ways.iter().enumerate() {
        let Some(kind) = kind(w) else { continue };
        let t = &w.tags;
        let pts: Vec<P> = w
            .refs
            .iter()
            .filter_map(|n| coords.get(n).copied())
            .collect();
        if pts.len() >= 2 {
            let ow = if matches!(t.oneway(), Some("yes") | Some("true") | Some("1"))
                || t.junction() == Some("roundabout")
            {
                1
            } else if t.oneway() == Some("-1") {
                -1
            } else {
                0
            };
            raw.push((wi, pts, kind, ow, level(w)));
        }
    }
    if !raw.iter().any(|r| r.2 == 1) {
        return Vec::new();
    }
    // the lines in degrees times M_PER_DEG on both axes: only to find the lines near a road (a box of SIDE_M / cos(lat)
    // along x holds all the points within SIDE_M in the frame of the road)
    let flat: Vec<Line> = raw.iter().map(|(_, pts, kind, ow, lvl)| line_in(pts, *kind, *ow, *lvl, M_PER_DEG, M_PER_DEG)).collect();
    let (side_all, car_all) = trees(&flat);
    let cos_a = ANGLE_DEG.to_radians().cos();

    // 2. The spots: every STEP_M along each road without a sidewalk tag (at least its middle).
    let roads: Vec<usize> = (0..raw.len()).filter(|&i| raw[i].2 == 1 && decide(&ways[raw[i].0])).collect();
    // A road reads only the lines near it, so the roads run on all threads; the result keeps their order.
    let covered = |k: usize, near_groups: &mut Vec<(usize, usize)>, crossing: &mut Vec<(usize, usize)>| -> bool {
        // the frame of the road, and the lines within reach in it (the road first)
        let c = (raw[k].1.iter().map(|p| p.1).sum::<f64>() / raw[k].1.len() as f64).to_radians().cos();
        let (rx, ry) = (SIDE_M / c.max(1e-9) + 1.0, SIDE_M + 1.0);
        let b = flat[k].groups.iter().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |a, &((x0, y0, x1, y1), ..)| {
            (a.0.min(x0), a.1.min(y0), a.2.max(x1), a.3.max(y1))
        });
        let reach = AABB::from_corners((b.0 - rx, b.1 - ry), (b.2 + rx, b.3 + ry));
        let mut near: Vec<usize> = side_all
            .locate_in_envelope_intersecting(&reach)
            .chain(car_all.locate_in_envelope_intersecting(&reach))
            .map(|g| g.line)
            .filter(|&i| i != k)
            .collect();
        near.sort_unstable();
        near.dedup();
        near.insert(0, k);
        let lines: Vec<Line> = near.iter().map(|&i| {
            let (_, pts, kind, ow, lvl) = &raw[i];
            line_in(pts, *kind, *ow, *lvl, M_PER_DEG * c, M_PER_DEG)
        }).collect();
        let (side_tree, car_tree) = trees(&lines);
        let k = 0;
        let around = |p: P, r: f64| AABB::from_corners((p.0 - r, p.1 - r), (p.0 + r, p.1 + r));
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
            groups_in(&side_tree, &around(pt, SIDE_M), near_groups);
            for (i, dd, loc) in nearest_lines(&lines, near_groups, pt) {
                let sw = &lines[i];
                if dd > SIDE_M {
                    continue;
                }
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
                groups_in(
                    &car_tree,
                    &AABB::from_corners((pt.0.min(near.0), pt.1.min(near.1)), (pt.0.max(near.0), pt.1.max(near.1))),
                    crossing,
                );
                let blocked = crossing.iter().any(|&(c, g)| {
                    let (_, first, end) = lines[c].groups[g];
                    c != k && lines[c].pts[first..=end].windows(2).any(|w| segments_meet(pt, near, w[0], w[1]))
                });
                if !blocked {
                    if side > 0.0 {
                        left = true;
                    } else {
                        right = true;
                    }
                }
            }

            // the other half of a dual carriageway
            groups_in(&car_tree, &around(pt, SIDE_M), near_groups);
            for (i, cdd, cl) in nearest_lines(&lines, near_groups, pt) {
                let c = &lines[i];
                if i == k || cdd > SIDE_M {
                    continue;
                }
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
        left_n as f64 / spots >= SHARE && right_n as f64 / spots >= SHARE
    };
    #[cfg(feature = "pbf")]
    let flags: Vec<bool> = {
        use rayon::prelude::*;
        roads
            .par_iter()
            .map_init(|| (Vec::new(), Vec::new()), |(g, c), &k| covered(k, g, c))
            .collect()
    };
    #[cfg(not(feature = "pbf"))]
    let flags: Vec<bool> = {
        let (mut g, mut c) = (Vec::new(), Vec::new());
        roads.iter().map(|&k| covered(k, &mut g, &mut c)).collect()
    };
    roads.iter().zip(flags).filter(|(_, f)| *f).map(|(&k, _)| raw[k].0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{OtherTags, Tags};

    fn way(id: i64, refs: &[i64], tags: &[(&str, &str)]) -> Way {
        Way { id, refs: refs.to_vec(), tags: Tags::from_pairs(tags.iter().copied()), other: OtherTags::of(tags.iter().copied()) }
    }

    /// A street with a sidewalk line 6 m on each side gets `sidewalk=separate`, and a line far away at another latitude
    /// does not change the answer (each road has its own frame: a tile with other lines gives the same result).
    #[test]
    fn a_road_reads_only_the_lines_near_it() {
        let (lat, dy) = (45.0, 6.0 / 111_320.0);
        let mut nodes = Vec::new();
        for (i, y) in [(1, 0.0), (2, dy), (3, -dy)] {
            nodes.push((10 * i, (7.0, lat + y)));
            nodes.push((10 * i + 1, (7.001, lat + y)));
        }
        nodes.extend([(100, (8.0, 60.0)), (101, (8.001, 60.0))]);
        let mut ways = vec![
            way(1, &[10, 11], &[("highway", "residential")]),
            way(2, &[20, 21], &[("highway", "footway"), ("footway", "sidewalk")]),
            way(3, &[30, 31], &[("highway", "footway"), ("footway", "sidewalk")]),
        ];
        let coords = Coords::new(nodes);
        assert_eq!(separate_sidewalks(&ways, &coords, |_| true), vec![0]);
        ways.push(way(4, &[100, 101], &[("highway", "residential")]));
        assert_eq!(separate_sidewalks(&ways, &coords, |_| true), vec![0]);
        assert_eq!(separate_sidewalks(&ways, &coords, |w| w.id != 1), Vec::<usize>::new());
    }
}
