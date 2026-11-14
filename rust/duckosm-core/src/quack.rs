//! duckosm-rust quack: the features table of QuackOSM 0.20.0 (`feature_id`, `tags`, `geometry`), made in Rust from one
//! pass over a `.osm.pbf` file (docs/rust/quack.md). The rules are the ones of QuackOSM's `pbf_file_reader.py` with its
//! default options (no filters, `ignore_metadata_tags`, compact tags):
//!
//! - a node with tags is a POINT;
//! - a way with tags, with 2 or more nodes and all its nodes in the file, is a LINESTRING, or a POLYGON when it is
//!   closed, has 4 or more points and its tags match the polygon rules of QuackOSM (`osm_way_polygon_features.json`);
//! - a relation of the type `multipolygon` or `boundary` with all its member ways in the file is a POLYGON or a
//!   MULTIPOLYGON: the member lines of each role are joined as GEOS `LineMerge` joins them, each closed ring is a
//!   polygon, an outer polygon loses the inner polygons in it, and the union of the outer polygons is the geometry;
//! - the metadata tags (`area`, `created_by`, `source`, `note`, `fixme`, ...) are not in the tags, and an element with
//!   no other tag is not in the table. The polygon rules read the tags before this.
//!
//! The coordinates are rounded to 7 decimals, as QuackOSM rounds them. QuackOSM repairs each geometry with GEOS
//! `ST_MakeValid`; here an invalid polygon is repaired with the even-odd rule of i_overlay, which gives the area of
//! the GEOS repair. The file must be sorted by type and id (nodes, then ways, then relations), as the files of
//! Geofabrik are.
use crate::graph::MemberType;
use crate::raw::{RawSink, RawTags};
use crate::spill::Codec;
use geo::orient::{Direction, Orient};
use geo::algorithm::sweep::Intersections;
use geo::algorithm::coordinate_position::CoordPos;
use geo::algorithm::dimensions::Dimensions;
use geo::{BoundingRect, Line, LineString, MultiPolygon, Polygon, Relate};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::overlay::{FloatOverlay, OverlayOptions};
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::Read;

pub type Pt = (f64, f64);

/// One row of the features table: `node/<id>`, `way/<id>` or `relation/<id>`, the tags, and the geometry as WKB.
#[derive(Clone)]
pub struct Feature {
    pub feature_id: String,
    pub tags: RawTags,
    pub geometry: Vec<u8>,
    /// The center of the bounding box of the geometry: the key of [`str_order`].
    pub center: Pt,
}

impl Feature {
    fn new(feature_id: String, tags: RawTags, g: &Geom) -> Feature {
        Feature { feature_id, tags, geometry: g.wkb(), center: g.center() }
    }
}

/// The numbers of a [`features`] run.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub nodes: usize,
    pub ways: usize,
    pub relations: usize,
}

/// The tags that QuackOSM leaves out (`METADATA_TAGS_TO_IGNORE`, the list of GDAL), and the keys that start with
/// `openGeoDB:`.
const METADATA: &[&str] = &["area", "created_by", "converted_by", "source", "time", "ele", "note", "todo", "fixme", "FIXME"];

// osm_way_polygon_features.json of QuackOSM 0.20.0
const POLYGON_ALL: &[&str] = &["allotments", "area:highway", "boundary", "bridge:support", "building:part", "building", "cemetery", "club", "craft", "demolished:building", "disused:amenity", "disused:leisure", "disused:shop", "healthcare", "historic", "industrial", "internet_access", "junction", "landuse", "leisure", "office", "place", "police", "polling_station", "public_transport", "residential", "ruins", "seamark:type", "shop", "sport", "telecom", "tourism"];
const POLYGON_ALLOWLIST: &[(&str, &[&str])] = &[
    ("advertising", &["sculpture", "sign"]),
    ("aerialway", &["station"]),
    ("barrier", &["city_wall", "hedge", "wall", "toll_booth"]),
    ("highway", &["services", "rest_area", "platform"]),
    ("railway", &["station", "turntable", "roundhouse", "platform"]),
    ("waterway", &["riverbank", "dock", "boatyard", "dam", "fuel"]),
];
const POLYGON_DENYLIST: &[(&str, &[&str])] = &[
    ("aeroway", &["jet_bridge", "parking_position", "taxiway", "no"]),
    ("amenity", &["bench", "weighbridge"]),
    ("attraction", &["river_rafting", "train", "water_slide", "boat_ride"]),
    ("emergency", &["designated", "destination", "no", "official", "private", "yes"]),
    ("geological", &["volcanic_caldera_rim", "fault"]),
    ("golf", &["cartpath", "hole", "path"]),
    ("indoor", &["corridor", "wall"]),
    ("man_made", &["yes", "breakwater", "carpet_hanger", "crane", "cutline", "dyke", "embankment", "goods_conveyor", "groyne", "pier", "pipeline", "torii", "video_wall"]),
    ("military", &["trench"]),
    ("natural", &["bay", "cliff", "coastline", "ridge", "strait", "tree_row", "valley", "no", "arete"]),
    ("piste:type", &["downhill", "hike", "ice_skate", "nordic", "skitour", "sled", "sleigh"]),
    ("playground", &["balancebeam", "rope_traverse", "stepping_stone", "stepping_post", "rope_swing", "climbing_slope"]),
    ("power", &["cable", "line", "minor_line", "insulator", "busbar", "bay", "portal"]),
];

/// DuckDB `round(v, 7)`.
fn round7(v: f64) -> f64 {
    (v * 1e7).round() / 1e7
}

fn tag<'a>(tags: &'a RawTags, key: &str) -> Option<&'a str> {
    tags.iter().find(|t| t.0 == key).map(|t| t.1.as_str())
}

/// The tags without the metadata tags.
fn clean(tags: &RawTags) -> RawTags {
    tags.iter()
        .filter(|(k, _)| !METADATA.contains(&k.as_str()) && !k.starts_with("openGeoDB:"))
        .cloned()
        .collect()
}

/// The polygon rules of QuackOSM for a closed way (from OSMnx `_is_closed_way_a_polygon`), on the tags before
/// [`clean`].
fn polygon_tags(tags: &RawTags) -> bool {
    if tag(tags, "area") == Some("no") {
        return false;
    }
    tag(tags, "area") == Some("yes")
        || POLYGON_ALL.iter().any(|k| tag(tags, k).is_some())
        || POLYGON_ALLOWLIST.iter().any(|(k, vs)| tag(tags, k).is_some_and(|v| vs.contains(&v)))
        || POLYGON_DENYLIST.iter().any(|(k, vs)| tag(tags, k).is_some_and(|v| !vs.contains(&v)))
}

/// GEOS `RemoveRepeatedPoints`: the points without a point that equals the point before it.
fn dedupe(pts: &[Pt]) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(pts.len());
    for &p in pts {
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

/// A geometry of the table.
#[derive(Debug, Clone, PartialEq)]
pub enum Geom {
    Point(Pt),
    Line(Vec<Pt>),
    /// One polygon is a POLYGON, more are a MULTIPOLYGON; each polygon is its rings (the shell first).
    Polygons(Vec<Vec<Vec<Pt>>>),
}

impl Geom {
    /// The center of the bounding box, as QuackOSM computes it for its sort: `(ST_XMin + ST_XMax) / 2`, and so for y.
    pub fn center(&self) -> Pt {
        let pts: Box<dyn Iterator<Item = &Pt>> = match self {
            Geom::Point(p) => return *p,
            Geom::Line(ps) => Box::new(ps.iter()),
            Geom::Polygons(ps) => Box::new(ps.iter().flat_map(|p| p[0].iter())),
        };
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in pts {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
    }

    pub fn wkb(&self) -> Vec<u8> {
        fn head(b: &mut Vec<u8>, kind: u32, n: usize) {
            b.push(1u8);
            b.extend_from_slice(&kind.to_le_bytes());
            b.extend_from_slice(&(n as u32).to_le_bytes());
        }
        fn pts(b: &mut Vec<u8>, ps: &[Pt]) {
            b.extend_from_slice(&(ps.len() as u32).to_le_bytes());
            for p in ps {
                b.extend_from_slice(&p.0.to_le_bytes());
                b.extend_from_slice(&p.1.to_le_bytes());
            }
        }
        fn polygon(b: &mut Vec<u8>, rings: &[Vec<Pt>]) {
            head(b, 3, rings.len());
            rings.iter().for_each(|r| pts(b, r));
        }
        let mut b = Vec::new();
        match self {
            Geom::Point(p) => {
                b.push(1u8);
                b.extend_from_slice(&1u32.to_le_bytes());
                b.extend_from_slice(&p.0.to_le_bytes());
                b.extend_from_slice(&p.1.to_le_bytes());
            }
            Geom::Line(ps) => {
                b.push(1u8);
                b.extend_from_slice(&2u32.to_le_bytes());
                pts(&mut b, ps);
            }
            Geom::Polygons(ps) if ps.len() == 1 => polygon(&mut b, &ps[0]),
            Geom::Polygons(ps) => {
                head(&mut b, 6, ps.len());
                ps.iter().for_each(|p| polygon(&mut b, p));
            }
        }
        b
    }
}

fn to_geo(polys: &[Vec<Vec<Pt>>]) -> MultiPolygon {
    let ring = |r: &Vec<Pt>| LineString::from(r.iter().map(|&(x, y)| geo::coord! { x: x, y: y }).collect::<Vec<_>>());
    MultiPolygon::new(
        polys
            .iter()
            .map(|p| Polygon::new(ring(&p[0]), p[1..].iter().map(ring).collect()))
            .collect(),
    )
}

fn from_geo(mp: &MultiPolygon) -> Vec<Vec<Vec<Pt>>> {
    let ring = |l: &LineString| l.0.iter().map(|c| (c.x, c.y)).collect::<Vec<Pt>>();
    mp.0.iter()
        .map(|p| std::iter::once(ring(p.exterior())).chain(p.interiors().iter().map(ring)).collect())
        .collect()
}

/// `geo`'s `Validation` of a polygon of one closed ring: 4 or more points, and no two segments meet other than at the
/// point that they share. `geo` compares each segment with each other segment (a boundary ring of 50 000 points took
/// seconds); here a sweep (`geo::algorithm::sweep::Intersections`) finds the pairs that meet.
fn ring_is_valid(ring: &[Pt]) -> bool {
    if ring.len() < 4 {
        return false;
    }
    let lines: Vec<Line> =
        ring.windows(2).map(|w| Line::new(geo::coord! { x: w[0].0, y: w[0].1 }, geo::coord! { x: w[1].0, y: w[1].1 })).collect();
    !Intersections::from_iter(lines.iter()).any(|(a, b, _)| a.start != b.end && a.end != b.start)
}

/// GEOS `ST_MakeValid(ST_MakePolygon(ring))` for a closed ring of 4 or more points (no repeated points): the polygon
/// when it is valid, else its repair with the even-odd rule of i_overlay (the area of GEOS's `linework` repair; GEOS
/// also keeps the lines that enclose no area, as a GEOMETRYCOLLECTION). A ring with no area stays a line.
fn ring_polygon(ring: Vec<Pt>) -> Geom {
    if ring_is_valid(&ring) {
        return Geom::Polygons(vec![vec![ring]]);
    }
    let open = vec![ring[..ring.len() - 1].iter().map(|p| [p.0, p.1]).collect::<Vec<_>>()];
    let v = run_overlay(&open, &vec![], OverlayRule::Subject, FillRule::EvenOdd);
    if v.0.is_empty() {
        Geom::Line(ring)
    } else {
        Geom::Polygons(from_geo(&v))
    }
}

/// GEOS `LineMerger` (not directed): the lines are joined at the points where exactly two line ends meet. Lines with
/// fewer than 2 points (after [`dedupe`]) are left out. A closed chain of lines is one closed line.
fn line_merge(lines: &[Vec<Pt>]) -> Vec<Vec<Pt>> {
    let key = |p: &Pt| (p.0.to_bits(), p.1.to_bits());
    let lines: Vec<Vec<Pt>> = lines.iter().map(|l| dedupe(l)).filter(|l| l.len() >= 2).collect();
    // the nodes (the line ends) and, for each node, the edge ends at it: (line, the line starts here)
    let mut ends: HashMap<(u64, u64), Vec<(usize, bool)>> = HashMap::new();
    for (i, l) in lines.iter().enumerate() {
        ends.entry(key(&l[0])).or_default().push((i, true));
        ends.entry(key(l.last().unwrap())).or_default().push((i, false));
    }
    let mut used = vec![false; lines.len()];
    let mut out = Vec::new();
    // one chain from the node `at` along the line `first`, on through the nodes of degree 2
    let chain = |first: (usize, bool), used: &mut Vec<bool>| {
        let mut pts: Vec<Pt> = Vec::new();
        let (mut line, mut forward) = first;
        loop {
            used[line] = true;
            let l = &lines[line];
            let seq: Box<dyn Iterator<Item = &Pt>> = if forward { Box::new(l.iter()) } else { Box::new(l.iter().rev()) };
            for &p in seq {
                if pts.last() != Some(&p) {
                    pts.push(p);
                }
            }
            let end = *pts.last().unwrap();
            let at = &ends[&key(&end)];
            if at.len() != 2 {
                break;
            }
            // the other edge end at this node
            let me = (line, !forward);
            let next = if at[0] == me { at[1] } else { at[0] };
            if used[next.0] {
                break;
            }
            line = next.0;
            forward = next.1;
        }
        pts
    };
    // the chains that start at a node of a degree other than 2, in the order of the nodes (as GEOS: sorted by x, y)
    let mut nodes: Vec<&(u64, u64)> = ends.keys().collect();
    nodes.sort_by(|a, b| {
        (f64::from_bits(a.0), f64::from_bits(a.1))
            .partial_cmp(&(f64::from_bits(b.0), f64::from_bits(b.1)))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for n in &nodes {
        let at = &ends[*n];
        if at.len() == 2 {
            continue;
        }
        for &e in at {
            if !used[e.0] {
                out.push(chain(e, &mut used));
            }
        }
    }
    // the rings: all their nodes have degree 2
    for n in &nodes {
        for &e in &ends[*n] {
            if !used[e.0] {
                out.push(chain(e, &mut used));
            }
        }
    }
    out
}

/// The parts of a geometry as `geo` polygons (a line has none).
fn polys(g: &Geom) -> MultiPolygon {
    match g {
        Geom::Polygons(p) => to_geo(p),
        _ => MultiPolygon::new(vec![]),
    }
}

fn as_geom(mp: &MultiPolygon) -> Option<Geom> {
    (!mp.0.is_empty()).then(|| Geom::Polygons(from_geo(mp)))
}

/// The polygons with their shells counterclockwise and their holes clockwise. The boolean operations of `geo`
/// (i_overlay) take the fill rule from the direction of the first ring, so all rings must have the same direction;
/// the rings of OSM come in both directions.
fn oriented(mp: MultiPolygon) -> MultiPolygon {
    mp.orient(Direction::Default)
}

/// A result of a boolean operation on the grid of the input (7 decimals): i_overlay computes on integers and gives
/// the input points back with a small error. A new point (where two edges cross) is not near the grid and stays. The
/// rings lose the repeated points that this makes, and a ring with fewer than 4 points goes.
fn snap(mp: MultiPolygon) -> MultiPolygon {
    let near = |v: f64| if (v - round7(v)).abs() < 1e-10 { round7(v) } else { v };
    let polys: Vec<Vec<Vec<Pt>>> = from_geo(&mp)
        .into_iter()
        .filter_map(|p| {
            let rings: Vec<Vec<Pt>> = p
                .into_iter()
                .map(|r| dedupe(&r.into_iter().map(|(x, y)| (near(x), near(y))).collect::<Vec<_>>()))
                .collect();
            if rings[0].len() < 4 {
                return None;
            }
            Some(rings.into_iter().enumerate().filter(|(i, r)| *i == 0 || r.len() >= 4).map(|(_, r)| r).collect())
        })
        .collect();
    to_geo(&polys)
}

/// A boolean operation of i_overlay (`Union` or `Difference`) on `subj` and `clip`. The rings are [`oriented`] and
/// the fill rule is `Positive`. All points stay, also the points on a straight edge, as in GEOS (the options of
/// `geo`'s `BooleanOps` remove them). The result is [`snap`]ped.
fn overlay(subj: &MultiPolygon, clip: &MultiPolygon, rule: OverlayRule) -> MultiPolygon {
    let paths = |mp: &MultiPolygon| -> Vec<Vec<[f64; 2]>> {
        let mp = oriented(mp.clone());
        mp.0.iter()
            .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
            .map(|r| r.0[..r.0.len().saturating_sub(1)].iter().map(|c| [c.x, c.y]).collect())
            .collect()
    };
    run_overlay(&paths(subj), &paths(clip), rule, FillRule::Positive)
}

/// i_overlay on rings (open: the last point is not the first point again), with all points kept.
fn run_overlay(subj: &Vec<Vec<[f64; 2]>>, clip: &Vec<Vec<[f64; 2]>>, rule: OverlayRule, fill: FillRule) -> MultiPolygon {
    let mut opt = OverlayOptions::ogc();
    opt.preserve_input_collinear = true;
    opt.preserve_output_collinear = true;
    let shapes = FloatOverlay::with_subj_and_clip_custom(subj, clip, opt, Default::default()).overlay(rule, fill);
    let polys: Vec<Vec<Vec<Pt>>> = shapes
        .into_iter()
        .map(|s| {
            s.into_iter()
                .map(|c| {
                    let mut r: Vec<Pt> = c.iter().map(|p| (p[0], p[1])).collect();
                    if let Some(&f) = r.first() {
                        r.push(f);
                    }
                    r
                })
                .collect()
        })
        .collect();
    snap(to_geo(&polys))
}

/// Do two of the boxes overlap (or touch)? A sweep over the boxes sorted by their left side.
fn boxes_meet(boxes: &[Option<geo::Rect>]) -> bool {
    let mut b: Vec<geo::Rect> = boxes.iter().flatten().copied().collect();
    b.sort_by(|p, q| p.min().x.total_cmp(&q.min().x));
    (0..b.len()).any(|i| {
        b[i + 1..].iter().take_while(|q| q.min().x <= b[i].max().x).any(|q| q.min().y <= b[i].max().y && b[i].min().y <= q.max().y)
    })
}

/// An edge of a ring of polygon `poly`, for the sweep of [`polygons_meet`].
struct Edge {
    line: Line,
    poly: usize,
}

impl geo::algorithm::sweep::Cross for Edge {
    type Scalar = f64;
    fn line(&self) -> Line {
        self.line
    }
}

/// Do two of the polygons meet (an edge crosses or touches an edge of another polygon, or one polygon is in another)?
/// The exact answer of `geo`'s `Intersects` for each pair, which compares each segment with each segment (a relation of
/// France took 100 s); here one sweep over all edges (`geo::algorithm::sweep::Intersections`), and for the pairs whose
/// boxes meet, one vertex of each in the other.
fn polygons_meet(polys: &[&Polygon]) -> bool {
    use geo::algorithm::coordinate_position::CoordinatePosition;
    let boxes: Vec<Option<geo::Rect>> = polys.iter().map(|p| p.bounding_rect()).collect();
    if !boxes_meet(&boxes) {
        return false;
    }
    let edges: Vec<Edge> = polys
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            std::iter::once(p.exterior()).chain(p.interiors()).flat_map(move |r| r.lines().map(move |line| Edge { line, poly: i }))
        })
        .collect();
    if Intersections::from_iter(edges.iter()).any(|(a, b, _)| a.poly != b.poly) {
        return true;
    }
    // no edges meet: a polygon is in another one only if all of it is, so one vertex tells
    let inside = |i: usize, j: usize| {
        polys[i].exterior().0.first().is_some_and(|c| polys[j].coordinate_position(c) != CoordPos::Outside)
    };
    (0..polys.len()).any(|i| {
        (0..polys.len()).any(|j| {
            i != j
                && match (boxes[i], boxes[j]) {
                    (Some(a), Some(b)) => a.min().x <= b.max().x && b.min().x <= a.max().x && a.min().y <= b.max().y && b.min().y <= a.max().y,
                    _ => false,
                }
                && inside(i, j)
        })
    })
}

/// GEOS `ST_Union_Agg` of polygons: when no two of them meet ([`polygons_meet`]), the polygons as they are; else the
/// union by i_overlay.
fn union(parts: Vec<MultiPolygon>) -> MultiPolygon {
    let all: Vec<Polygon> = parts.into_iter().flat_map(|m| oriented(m).0).collect();
    let meet = polygons_meet(&all.iter().collect::<Vec<_>>());
    if !meet {
        return MultiPolygon::new(all);
    }
    overlay(&MultiPolygon::new(all), &MultiPolygon::new(vec![]), OverlayRule::Union)
}

/// The geometry of a relation of the type multipolygon or boundary, from the lines of its member ways (each with
/// its role), as QuackOSM `_get_filtered_relations_with_geometry` makes it. None: the relation is not in the table.
fn relation_geometry(members: &[(&str, Vec<Pt>)]) -> Option<Geom> {
    let any_outer = members.iter().any(|m| m.0 == "outer");
    // the roles in the order of their first member
    let mut roles: Vec<&str> = Vec::new();
    for m in members {
        if !roles.contains(&m.0) {
            roles.push(m.0);
        }
    }
    let mut parts: Vec<(&str, Vec<Pt>)> = Vec::new();
    for role in roles {
        let lines: Vec<Vec<Pt>> = members.iter().filter(|m| m.0 == role).map(|m| m.1.clone()).collect();
        for l in line_merge(&lines) {
            if dedupe(&l).len() >= 4 {
                parts.push((if any_outer { role } else { "outer" }, l));
            }
        }
    }
    if parts.is_empty() || parts.iter().any(|p| p.1.first() != p.1.last()) {
        return None;
    }
    let ring = |r: &str| -> Vec<MultiPolygon> {
        parts.iter().filter(|p| p.0 == r).map(|p| polys(&ring_polygon(dedupe(&p.1)))).collect()
    };
    let (inner, outer) = (ring("inner"), ring("outer"));
    // a polygon in another polygon has its box in the box of the other: the exact (slow) relate runs only for these
    let inner_boxes: Vec<Option<geo::Rect>> = inner.iter().map(|ig| ig.bounding_rect()).collect();
    let pieces: Vec<MultiPolygon> = outer
        .into_iter()
        .map(|og| {
            let ob = og.bounding_rect();
            let in_box = |i: usize| match (inner_boxes[i], ob) {
                (Some(a), Some(b)) => a.min().x >= b.min().x && a.min().y >= b.min().y && a.max().x <= b.max().x && a.max().y <= b.max().y,
                _ => false,
            };
            // the inner polygons in this outer polygon, and if each of them is away from its border
            let mut apart = true;
            let holes: Vec<&MultiPolygon> = inner
                .iter()
                .enumerate()
                .filter(|(i, _)| in_box(*i))
                .map(|(_, ig)| ig)
                .filter(|ig| {
                    if ig.0.is_empty() || og.0.is_empty() {
                        return false;
                    }
                    let m = ig.relate(&og);
                    let within = m.is_within();
                    apart &= !within || m.get(CoordPos::OnBoundary, CoordPos::OnBoundary) == Dimensions::Empty;
                    within
                })
                .collect();
            if holes.is_empty() {
                return og;
            }
            // one shell, and holes that are single shells, away from the shell and from each other: the polygon
            // with the holes is the difference
            let single = og.0.len() == 1
                && og.0[0].interiors().is_empty()
                && holes.iter().all(|h| h.0.len() == 1 && h.0[0].interiors().is_empty());
            // the holes do not meet each other (else the exact difference below)
            let disjoint = !polygons_meet(&holes.iter().flat_map(|h| h.0.iter()).collect::<Vec<_>>());
            if single && apart && disjoint {
                let p = Polygon::new(og.0[0].exterior().clone(), holes.iter().map(|h| h.0[0].exterior().clone()).collect());
                return MultiPolygon::new(vec![p]);
            }
            let cut = MultiPolygon::new(holes.into_iter().flat_map(|h| h.0.clone()).collect());
            overlay(&og, &cut, OverlayRule::Difference)
        })
        .collect();
    as_geom(&union(pieces))
}

/// The ways and the relations of one block (pass 1): each way with 2 or more nodes, and each relation of the type
/// multipolygon or boundary with its member ways.
#[derive(Default)]
struct WaysPart {
    ways: Vec<(i64, RawTags, Vec<i64>)>,
    relations: Vec<(i64, RawTags, Vec<(i64, String)>)>,
}

impl RawSink for WaysPart {
    fn node<'a>(&mut self, _: i64, _: f64, _: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {}

    fn way<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        let refs: Vec<i64> = refs.collect();
        if refs.len() >= 2 {
            self.ways.push((id, tags.map(|(k, v)| (k.to_string(), v.to_string())).collect(), refs));
        }
    }

    fn relation<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        let tags: RawTags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        if !matches!(tag(&tags, "type"), Some("multipolygon" | "boundary")) {
            return;
        }
        let ways: Vec<(i64, String)> =
            // QuackOSM: COALESCE(ref_role, 'outer'), and ST_READOSM gives NULL for an empty role
            members
                .filter(|m| m.1 == MemberType::Way)
                .map(|m| (m.0, if m.2.is_empty() { "outer" } else { m.2 }.to_string()))
                .collect();
        if !ways.is_empty() {
            self.relations.push((id, tags, ways));
        }
    }
}

/// The nodes of one block (pass 2): each node for the join, and the features of the nodes with tags.
#[derive(Default)]
struct NodesPart {
    nodes: Vec<(i64, f64, f64)>,
    features: Vec<Feature>,
}

impl RawSink for NodesPart {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        self.nodes.push((id, lon, lat));
        let tags: RawTags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let tags = clean(&tags);
        if !tags.is_empty() {
            self.features.push(Feature::new(format!("node/{id}"), tags, &Geom::Point((round7(lon), round7(lat)))));
        }
    }
    fn way<'a>(&mut self, _: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = i64>) {}
    fn relation<'a>(
        &mut self,
        _: i64,
        _: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
    }
}

fn put_u32(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&(n as u32).to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len());
    out.extend_from_slice(s.as_bytes());
}

fn put_tags(out: &mut Vec<u8>, tags: &RawTags) {
    put_u32(out, tags.len());
    for (k, v) in tags {
        put_str(out, k);
        put_str(out, v);
    }
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> &'a [u8] {
    let (x, rest) = b.split_at(n);
    *b = rest;
    x
}

fn get_u32(b: &mut &[u8]) -> u32 {
    u32::from_le_bytes(take(b, 4).try_into().expect("4 bytes"))
}

fn get_i64(b: &mut &[u8]) -> i64 {
    i64::from_le_bytes(take(b, 8).try_into().expect("8 bytes"))
}

fn get_str(b: &mut &[u8]) -> String {
    let n = get_u32(b) as usize;
    String::from_utf8_lossy(take(b, n)).into_owned()
}

fn get_tags(b: &mut &[u8]) -> RawTags {
    (0..get_u32(b)).map(|_| (get_str(b), get_str(b))).collect()
}

/// The header of a way in the temporary file of pass 1: its id, its number of nodes and its tags (before [`clean`]).
struct Header;

impl Codec<(i64, u32, RawTags)> for Header {
    fn put(&self, r: &(i64, u32, RawTags), out: &mut Vec<u8>) {
        out.extend_from_slice(&r.0.to_le_bytes());
        put_u32(out, r.1 as usize);
        put_tags(out, &r.2);
    }
    fn get(&self, b: &mut &[u8]) -> (i64, u32, RawTags) {
        (get_i64(b), get_u32(b), get_tags(b))
    }
}

/// A way with no tags (pass 1) and its nodes: it needs its points only when it is a member of a relation, and the
/// relations come after the ways in the file.
struct WayRefs;

impl Codec<(i64, Vec<i64>)> for WayRefs {
    fn put(&self, r: &(i64, Vec<i64>), out: &mut Vec<u8>) {
        out.extend_from_slice(&r.0.to_le_bytes());
        put_u32(out, r.1.len());
        r.1.iter().for_each(|n| out.extend_from_slice(&n.to_le_bytes()));
    }
    fn get(&self, b: &mut &[u8]) -> (i64, Vec<i64>) {
        let id = get_i64(b);
        let n = get_u32(b);
        (id, (0..n).map(|_| get_i64(b)).collect())
    }
}

/// A relation (pass 1): its id, its tags and the role of each member way, in the order of the members.
struct RelHeader;

impl Codec<(i64, RawTags, Vec<String>)> for RelHeader {
    fn put(&self, r: &(i64, RawTags, Vec<String>), out: &mut Vec<u8>) {
        out.extend_from_slice(&r.0.to_le_bytes());
        put_tags(out, &r.1);
        put_u32(out, r.2.len());
        r.2.iter().for_each(|s| put_str(out, s));
    }
    fn get(&self, b: &mut &[u8]) -> (i64, RawTags, Vec<String>) {
        let id = get_i64(b);
        let tags = get_tags(b);
        let n = get_u32(b);
        (id, tags, (0..n).map(|_| get_str(b)).collect())
    }
}

/// A line of points on the grid with a key: the id of a member way, or the relation and the place of the member
/// (`relation << 32 | place`). The key comes first, so a sort of the bytes reads it with [`line_key`].
struct KeyedLine;

impl Codec<(i64, Vec<Grid>)> for KeyedLine {
    fn put(&self, r: &(i64, Vec<Grid>), out: &mut Vec<u8>) {
        out.extend_from_slice(&r.0.to_le_bytes());
        put_u32(out, r.1.len());
        for g in &r.1 {
            out.extend_from_slice(&g[0].to_le_bytes());
            out.extend_from_slice(&g[1].to_le_bytes());
        }
    }
    fn get(&self, b: &mut &[u8]) -> (i64, Vec<Grid>) {
        let key = get_i64(b);
        let n = get_u32(b);
        let pts = (0..n)
            .map(|_| {
                let x = i32::from_le_bytes(take(b, 4).try_into().expect("4 bytes"));
                let y = i32::from_le_bytes(take(b, 4).try_into().expect("4 bytes"));
                [x, y]
            })
            .collect();
        (key, pts)
    }
}

fn line_key(b: &[u8]) -> i64 {
    i64::from_le_bytes(b[..8].try_into().expect("8 bytes"))
}

/// A point on the grid of 7 decimals, as two i32 (the lines of the member ways wait in memory for the relations).
type Grid = [i32; 2];

fn to_grid(p: Pt) -> Grid {
    [(p.0 * 1e7).round() as i32, (p.1 * 1e7).round() as i32]
}

fn from_grid(g: &Grid) -> Pt {
    (g[0] as f64 / 1e7, g[1] as f64 / 1e7)
}

/// The feature of a way that has all its nodes (`pts`), or None.
fn way_feature(id: i64, raw: &RawTags, pts: &[Pt]) -> Option<Feature> {
    let tags = clean(raw);
    if tags.is_empty() {
        return None;
    }
    let line = dedupe(pts);
    let closed = pts.first() == pts.last();
    let geometry = if closed && line.len() >= 4 && polygon_tags(raw) {
        ring_polygon(line)
    } else if line.len() == 1 {
        Geom::Point(line[0])
    } else {
        Geom::Line(line)
    };
    Some(Feature::new(format!("way/{id}"), tags, &geometry))
}

/// The memory that each sort of the temporary files holds (the default of [`features`]).
pub const BUDGET: usize = 512 << 20;

/// The features table of a `.osm.pbf` file sorted by type and id (`open` opens it; it is read twice). Each table goes to
/// a temporary file (`crate::spill`, OPFS in the browser) as soon as a pass makes it, and a pass reads only what it
/// needs, so the memory is one bucket, one batch and the threads' work, for any size of area (France: a footprint of
/// 3.5 GB):
///
/// 1. Pass 1 reads the ways and the relations. A way with tags (and 2 or more nodes) gets a number; its header goes to
///    a file, and its nodes go into buckets by node id (`spill::Buckets`, 2^27 ids each) as requests (node, way, place).
///    A way with no tags goes to a file of its own: it needs its points only as a member of a relation. A relation of
///    the type multipolygon or boundary goes to a file of headers, and each member way to a file of (way, relation,
///    place). After the pass, the member ways with no tags get numbers and requests too. In memory: the sorted ids of
///    the member ways (an index).
/// 2. Pass 2 reads the nodes in the order of their ids. A node with tags is a feature. The buckets of requests are read
///    one at a time in the order of the ids, each sorted in memory on all threads, and each request gets its point; the
///    points go into buckets by way number.
/// 3. A reader thread reads the buckets of points (each sorted by way and place) with the headers: a way with all its
///    points and with tags is a feature, and the line of a member way goes to a file.
/// 4. The members sorted by way meet the lines sorted by way; the lines, sorted by relation and place, meet the
///    relation headers. The relations with all their lines go to all threads one at a time, and each row goes to `emit`.
///
/// `emit` gets the features in parts: the nodes, the ways, then the relations. Each pass calls [`crate::util::step`],
/// and a relation that takes more than 2 s is written to the error output.
pub fn features<R: Read + Send>(
    open: impl Fn() -> Result<R, String>,
    budget: usize,
    mut emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    use crate::spill::join::{Point, Request};
    use crate::spill::{sort_bytes, temp_store, Buckets, SortedParts, Writer};
    let io = |e: std::io::Error| format!("quack: a temporary file: {e}");
    let store = temp_store();
    let store = &*store;
    let mut counts = Counts::default();
    // pass 1: the ways and the relations. The requests go into buckets by node id (2^27 ids each: France about 100).
    let mut headers = Writer::new(store).map_err(io)?;
    let mut requests = Buckets::new(27);
    let mut untagged = Writer::new(store).map_err(io)?;
    let mut rel_headers = Writer::new(store).map_err(io)?;
    let mut rel_members = Writer::new(store).map_err(io)?;
    let mut member_ids: Vec<i64> = Vec::new();
    let (mut ways, mut rels) = (0u32, 0u32);
    let mut way = |id: i64, refs: &[i64], tags: RawTags, headers: &mut Writer, requests: &mut Buckets| -> Result<(), String> {
        for (k, &n) in refs.iter().enumerate() {
            requests.push(store, n, &Request, &(n, ways, k as u32)).map_err(io)?;
        }
        headers.push(&Header, &(id, refs.len() as u32, tags)).map_err(io)?;
        ways = ways.checked_add(1).ok_or("quack: more than 4 294 967 295 ways")?;
        Ok(())
    };
    crate::cutstream::each_block(
        &mut open()?,
        |b, buf| {
            let mut part = WaysPart::default();
            crate::pbfraw::decode_block(b, [false, true, true], buf, &mut part)?;
            Ok(part)
        },
        |part, _| {
            for (id, tags, refs) in part.ways {
                if tags.is_empty() {
                    untagged.push(&WayRefs, &(id, refs)).map_err(io)?;
                } else {
                    way(id, &refs, tags, &mut headers, &mut requests)?;
                }
            }
            for (id, tags, members) in part.relations {
                for (place, (w, _)) in members.iter().enumerate() {
                    rel_members.push(&Request, &(*w, rels, place as u32)).map_err(io)?;
                    member_ids.push(*w);
                }
                rel_headers.push(&RelHeader, &(id, tags, members.into_iter().map(|m| m.1).collect())).map_err(io)?;
                rels = rels.checked_add(1).ok_or("quack: more than 4 294 967 295 relations")?;
            }
            Ok(true)
        },
    )?;
    member_ids.sort_unstable();
    member_ids.dedup();
    for w in untagged.finish().map_err(io)?.read(&WayRefs).map_err(io)? {
        let (id, refs) = w.map_err(io)?;
        if member_ids.binary_search(&id).is_ok() {
            way(id, &refs, Vec::new(), &mut headers, &mut requests)?;
        }
    }
    let ways_count = ways as u64;
    crate::util::step("quack 1: the ways and the relations");
    // pass 2: the nodes in the order of their ids meet the requests, one bucket at a time (sorted in memory). The points
    // go into buckets by way number, about 64 of them.
    let way_shift = (64 - (ways_count / 64).max(1).leading_zeros()).max(10);
    let mut points = Buckets::new(way_shift);
    let mut parts = SortedParts::new(requests.finish().map_err(io)?, &Request, |r: &(i64, u32, u32)| r.0);
    let (mut part, mut at): (Vec<(i64, u32, u32)>, usize) = (Vec::new(), 0);
    let (mut last, mut seen, mut more) = (i64::MIN, false, true);
    crate::cutstream::each_block(
        &mut open()?,
        |b, buf| {
            let mut part = NodesPart::default();
            crate::pbfraw::decode_block(b, [true, false, false], buf, &mut part)?;
            Ok(part)
        },
        |nodes, _| {
            if nodes.nodes.is_empty() {
                // the nodes of a sorted file come first: a block with no nodes after them ends the pass
                return Ok(!seen);
            }
            seen = true;
            for &(id, lon, lat) in &nodes.nodes {
                if id <= last {
                    return Err("quack: the nodes of the .osm.pbf file are not in the order of their ids (the file must be sorted by type and id)".into());
                }
                last = id;
                // the requests before this node have no node in the file
                loop {
                    while at < part.len() && part[at].0 < id {
                        at += 1;
                    }
                    if at < part.len() || !more {
                        break;
                    }
                    more = parts.next_into(&mut part).map_err(io)?;
                    at = 0;
                }
                while at < part.len() && part[at].0 == id {
                    let (_, w, place) = part[at];
                    points.push(store, w as i64, &Point, &(w, place, lon, lat)).map_err(io)?;
                    at += 1;
                }
            }
            counts.nodes += nodes.features.len();
            emit(nodes.features)?;
            Ok(true)
        },
    )?;
    drop((part, parts));
    let points = points.finish().map_err(io)?;
    crate::util::step("quack 2: the nodes and the points");
    // pass 3: the ways. A reader thread joins the headers with their points into batches; this thread makes the features of
    // each batch on all threads and writes them, and writes the lines of the member ways to a file.
    type WayBatch = Vec<(i64, RawTags, Vec<Pt>)>;
    let mut member_lines = Writer::new(store).map_err(io)?;
    let headers = headers.finish().map_err(io)?;
    std::thread::scope(|scope| -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<WayBatch, String>>(4);
        scope.spawn(move || {
            let run = || -> Result<(), String> {
                // the points of each bucket, sorted, one bucket in memory at a time (no copy)
                let mut parts = SortedParts::new(points, &Point, |p: &(u32, u32, f64, f64)| (p.0, p.1));
                let (mut part, mut at): (Vec<(u32, u32, f64, f64)>, usize) = (Vec::new(), 0);
                let mut points = std::iter::from_fn(move || loop {
                    if at < part.len() {
                        at += 1;
                        return Some(Ok(part[at - 1]));
                    }
                    match parts.next_into(&mut part) {
                        Ok(true) => at = 0,
                        Ok(false) => return None,
                        Err(e) => return Some(Err(e)),
                    }
                })
                .peekable();
                let mut batch: WayBatch = Vec::new();
                for (number, h) in headers.read(&Header).map_err(io)?.enumerate() {
                    let (id, refs, tags) = h.map_err(io)?;
                    let mut pts = Vec::with_capacity(refs as usize);
                    while let Some(p) = points.next_if(|p| p.as_ref().is_ok_and(|p| p.0 == number as u32)) {
                        let p = p.map_err(io)?;
                        pts.push((round7(p.2), round7(p.3)));
                    }
                    if let Some(Err(_)) = points.peek() {
                        return Err(io(points.next().expect("peeked").expect_err("an error")));
                    }
                    // a way with a node that is not in the file is not a feature, and not a member of a relation
                    if pts.len() == refs as usize {
                        batch.push((id, tags, pts));
                    }
                    if batch.len() >= 50_000 {
                        tx.send(Ok(std::mem::take(&mut batch))).map_err(|_| "quack: pass 3 stopped".to_string())?;
                    }
                }
                tx.send(Ok(batch)).map_err(|_| "quack: pass 3 stopped".to_string())
            };
            if let Err(e) = run() {
                let _ = tx.send(Err(e));
            }
        });
        for batch in rx {
            let batch = batch?;
            for (id, _, pts) in &batch {
                if member_ids.binary_search(id).is_ok() {
                    member_lines.push(&KeyedLine, &(*id, pts.iter().map(|&p| to_grid(p)).collect())).map_err(io)?;
                }
            }
            // the member ways with no tags are not features
            let made: Vec<Feature> = batch
                .par_iter()
                .filter(|w| !w.1.is_empty())
                .filter_map(|(id, tags, pts)| way_feature(*id, tags, pts))
                .collect();
            counts.ways += made.len();
            emit(made)?;
        }
        Ok(())
    })?;
    drop(member_ids);
    crate::util::step("quack 3: the ways");
    // pass 4: the relations. The members and the lines, each sorted by way, give the lines of each relation; sorted by
    // relation and place, they meet the relation headers.
    let members = sort_bytes(rel_members.finish().map_err(io)?, crate::spill::join::node_id, budget, store).map_err(io)?;
    let lines = sort_bytes(member_lines.finish().map_err(io)?, line_key, budget, store).map_err(io)?;
    let mut rel_lines = Writer::new(store).map_err(io)?;
    {
        let mut lines = lines.read(&KeyedLine).map_err(io)?.peekable();
        for m in members.read(&Request).map_err(io)? {
            let (w, r, place) = m.map_err(io)?;
            while lines.next_if(|l| l.as_ref().is_ok_and(|l| l.0 < w)).is_some() {}
            match lines.peek() {
                Some(Ok(l)) if l.0 == w => {
                    rel_lines.push(&KeyedLine, &(((r as i64) << 32) | place as i64, l.1.clone())).map_err(io)?;
                }
                Some(Err(_)) => return Err(io(lines.next().expect("peeked").expect_err("an error"))),
                _ => {}
            }
        }
    }
    let rel_lines = sort_bytes(rel_lines.finish().map_err(io)?, line_key, budget, store).map_err(io)?;
    crate::util::step("quack 4a: the lines of the relations");
    // the relations with all their lines, streamed into all threads (a slow relation delays only itself); this thread
    // writes each part of rows that the threads make
    type Rel = (i64, RawTags, Vec<(String, Vec<Pt>)>);
    std::thread::scope(|scope| -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Option<Feature>, String>>(4096);
        scope.spawn(move || {
            let read = || -> Result<(), String> {
                let mut lines = rel_lines.read(&KeyedLine).map_err(io)?.peekable();
                let headers = rel_headers.finish().map_err(io)?;
                let rels = headers.read(&RelHeader).map_err(io)?.enumerate().map(|(r, h)| -> Result<Option<Rel>, String> {
                    let (id, tags, roles) = h.map_err(io)?;
                    let mut got: Vec<(String, Vec<Pt>)> = Vec::with_capacity(roles.len());
                    while let Some(l) = lines.next_if(|l| l.as_ref().is_ok_and(|l| (l.0 >> 32) as usize == r)) {
                        let (key, pts) = l.map_err(io)?;
                        got.push((roles[(key & 0xffff_ffff) as usize].clone(), pts.iter().map(from_grid).collect()));
                    }
                    if let Some(Err(_)) = lines.peek() {
                        return Err(io(lines.next().expect("peeked").expect_err("an error")));
                    }
                    // a relation with a member way that is not a valid way is not a feature
                    Ok((got.len() == roles.len()).then_some((id, tags, got)))
                });
                rels.par_bridge().try_for_each_with(tx.clone(), |tx, rel| {
                    let made = rel.map(|rel| {
                        rel.and_then(|(id, tags, members)| {
                            let t = std::time::Instant::now();
                            let m: Vec<(&str, Vec<Pt>)> = members.iter().map(|(role, l)| (role.as_str(), l.clone())).collect();
                            let geometry = relation_geometry(&m);
                            let secs = t.elapsed().as_secs_f64();
                            if secs > 2.0 {
                                eprintln!("quack: relation {id} ({} members) took {secs:.1} s", m.len());
                            }
                            let tags = clean(&tags);
                            (geometry.is_some() && !tags.is_empty()).then(|| Feature::new(format!("relation/{id}"), tags, &geometry.unwrap()))
                        })
                    });
                    // a full channel: this thread helps with the other parallel work (the writer makes its batches
                    // on these threads too) and tries again; a plain send would wait for ever when all threads wait
                    let mut item = made;
                    loop {
                        match tx.try_send(item) {
                            Ok(()) => return Ok(()),
                            Err(std::sync::mpsc::TrySendError::Full(back)) => {
                                item = back;
                                if rayon::yield_now() != Some(rayon::Yield::Executed) {
                                    std::thread::sleep(std::time::Duration::from_micros(200));
                                }
                            }
                            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return Err("quack: pass 4 stopped".to_string()),
                        }
                    }
                })
            };
            if let Err(e) = read() {
                let _ = tx.send(Err(e));
            }
        });
        let mut part = Vec::new();
        for f in rx {
            if let Some(f) = f? {
                part.push(f);
                if part.len() >= 10_000 {
                    counts.relations += part.len();
                    emit(std::mem::take(&mut part))?;
                }
            }
        }
        counts.relations += part.len();
        emit(part)
    })?;
    crate::util::step("quack 4b: the relations");
    Ok(counts)
}

/// The rows of a row group of the GeoParquet file, as in QuackOSM.
pub const ROW_GROUP: usize = 100_000;

/// QuackOSM's sort of the rows (`sort_result=True`, the default; `rq_geo_toolkit` `_str_index_select_sql`):
/// Sort-Tile-Recursive packing on the centers of the rows ([`Feature::center`]). The rows sorted by x (then y) are cut
/// into `ceil(sqrt(ceil(rows / ROW_GROUP)))` vertical strips; each strip is sorted by y, up in the even strips and down
/// in the odd strips. So each row group of the file holds the rows of a small area, and a reader with a box can skip the
/// others. Returns the row numbers in the new order; the order before the sort breaks the ties.
pub fn str_order(centers: &[Pt]) -> Vec<usize> {
    str_order_by(centers, ROW_GROUP)
}

/// The number of strips and the rows in each strip (the last strip can have fewer) of `n` rows.
fn strips(n: usize, group: usize) -> (usize, usize) {
    let strips = (n.div_ceil(group) as f64).sqrt().ceil().max(1.0) as usize;
    (strips, n.div_ceil(strips))
}

fn str_order_by(centers: &[Pt], group: usize) -> Vec<usize> {
    let n = centers.len();
    if n == 0 {
        return Vec::new();
    }
    let (_, strip_size) = strips(n, group);
    let cmp = |a: f64, b: f64| a.total_cmp(&b);
    let mut by_x: Vec<usize> = (0..n).collect();
    by_x.par_sort_by(|&a, &b| {
        let (pa, pb) = (centers[a], centers[b]);
        cmp(pa.0, pb.0).then(cmp(pa.1, pb.1)).then(a.cmp(&b))
    });
    let mut strip = vec![0usize; n];
    for (rank, &i) in by_x.iter().enumerate() {
        strip[i] = rank / strip_size;
    }
    let mut order = by_x;
    order.par_sort_by(|&a, &b| {
        let (pa, pb) = (centers[a], centers[b]);
        let y = |s: usize, y: f64| if s % 2 == 0 { y } else { -y };
        strip[a]
            .cmp(&strip[b])
            .then(cmp(y(strip[a], pa.1), y(strip[b], pb.1)))
            .then(cmp(pa.0, pb.0))
            .then(a.cmp(&b))
    });
    order
}

/// An `i64` in the order of `f64::total_cmp`.
fn ordered(x: f64) -> i64 {
    let b = x.to_bits() as i64;
    b ^ (((b >> 63) as u64) >> 1) as i64
}

/// The key of a row of [`Numbered`] from its bytes: (x, y, row number), the order of the strips of [`str_order`].
fn numbered_key(b: &[u8]) -> (i64, i64, u64) {
    let f = |i: usize| f64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    (ordered(f(8)), ordered(f(16)), u64::from_le_bytes(b[..8].try_into().unwrap()))
}

/// A row on its way through [`StrSort`]: the number of the row (the ties of [`str_order`]) and the feature.
struct Numbered;

impl Numbered {
    fn put_row(i: u64, f: &Feature, o: &mut Vec<u8>) {
        let s = |o: &mut Vec<u8>, b: &[u8]| {
            o.extend_from_slice(&(b.len() as u32).to_le_bytes());
            o.extend_from_slice(b);
        };
        o.extend_from_slice(&i.to_le_bytes());
        o.extend_from_slice(&f.center.0.to_le_bytes());
        o.extend_from_slice(&f.center.1.to_le_bytes());
        s(o, f.feature_id.as_bytes());
        o.extend_from_slice(&(f.tags.len() as u32).to_le_bytes());
        for (k, v) in &f.tags {
            s(o, k.as_bytes());
            s(o, v.as_bytes());
        }
        s(o, &f.geometry);
    }
}

impl Codec<(u64, Feature)> for Numbered {
    fn put(&self, (i, f): &(u64, Feature), o: &mut Vec<u8>) {
        Numbered::put_row(*i, f, o);
    }

    fn get(&self, b: &mut &[u8]) -> (u64, Feature) {
        use crate::spill::{get_f64, get_u32, take};
        fn bytes<'a>(b: &mut &'a [u8]) -> &'a [u8] {
            let n = get_u32(b) as usize;
            let (v, rest) = b.split_at(n);
            *b = rest;
            v
        }
        let s = |b: &mut &[u8]| String::from_utf8_lossy(bytes(b)).into_owned();
        let i = u64::from_le_bytes(take::<8>(b));
        let center = (get_f64(b), get_f64(b));
        let feature_id = s(b);
        let tags = (0..get_u32(b)).map(|_| (s(b), s(b))).collect();
        let geometry = bytes(b).to_vec();
        (i, Feature { feature_id, tags, geometry, center })
    }
}

/// The rows in a block of a run of [`StrSort`] (about 700 kB on France): the merge keeps one block of each run.
const BLOCK_ROWS: usize = 2048;

/// A block of a run: compressed with zstd level 1 (natively), else as it is.
fn pack(raw: Vec<u8>) -> std::io::Result<Vec<u8>> {
    #[cfg(feature = "zstd")]
    return zstd::bulk::compress(&raw, 1);
    #[cfg(not(feature = "zstd"))]
    Ok(raw)
}

fn unpack(b: &[u8], raw_len: usize, out: &mut Vec<u8>) -> std::io::Result<()> {
    out.clear();
    #[cfg(feature = "zstd")]
    {
        *out = zstd::bulk::decompress(b, raw_len)?;
    }
    #[cfg(not(feature = "zstd"))]
    {
        let _ = raw_len;
        out.extend_from_slice(b);
    }
    Ok(())
}

/// A run of [`StrSort`] in the merge: its next blocks (place, length and rows' bytes in the file) and its block in
/// memory, with the place of the next row.
struct Cursor {
    blocks: std::collections::VecDeque<(u64, usize, usize)>,
    buf: Vec<u8>,
    pos: usize,
    row: (usize, usize),
}

impl Cursor {
    /// Go to the next row (read the next block when the block is done); false at the end of the run.
    fn advance(&mut self, file: &mut dyn crate::spill::Blob, tmp: &mut Vec<u8>) -> std::io::Result<bool> {
        use std::io::SeekFrom;
        if self.pos == self.buf.len() {
            let Some((at, len, raw_len)) = self.blocks.pop_front() else {
                return Ok(false);
            };
            file.seek(SeekFrom::Start(at))?;
            tmp.resize(len, 0);
            file.read_exact(tmp)?;
            unpack(tmp, raw_len, &mut self.buf)?;
            self.pos = 0;
        }
        let n = u32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap()) as usize;
        self.row = (self.pos + 4, self.pos + 4 + n);
        self.pos += 4 + n;
        Ok(true)
    }

    fn bytes(&self) -> &[u8] {
        &self.buf[self.row.0..self.row.1]
    }
}

/// A row in the bytes of [`StrSort`]: its sort key (two `ordered` values and the row number) and the place of its bytes.
type Placed = (i64, i64, u64, usize, usize);

/// QuackOSM's sort ([`str_order`]) with the rows on disk, for areas whose rows do not fit in memory (France: 99 million
/// rows). [`StrSort::push`] writes the rows as bytes into one buffer until it holds `budget` bytes, then sorts them by
/// (x, y, row number) and writes them as a run of blocks (compressed on all threads) at the end of one temporary file.
/// [`StrSort::finish`] merges the runs, so the rows come in the order of x, into the bytes of a strip; each time a strip
/// is full, it sorts the strip by y and gives its rows, a row group at a time. The order is the one of [`str_order`].
/// The disk holds the rows one time, compressed. The memory is `budget` while the rows come, then the bytes of one
/// strip (`sqrt(rows * ROW_GROUP)` rows, 3.2 million on France), one block of each run and one row group.
pub struct StrSort {
    file: Box<dyn crate::spill::Blob>,
    end: u64,
    /// the blocks of each run: place, compressed length, length
    runs: Vec<Vec<(u64, usize, usize)>>,
    bytes: Vec<u8>,
    rows: Vec<Placed>,
    written: u64,
    budget: usize,
    group: usize,
}

impl Default for StrSort {
    fn default() -> Self {
        StrSort::new(BUDGET, ROW_GROUP).expect("quack sort: a temporary file")
    }
}

impl StrSort {
    fn new(budget: usize, group: usize) -> std::io::Result<Self> {
        let file = crate::spill::temp_store().create()?;
        Ok(StrSort { file, end: 0, runs: Vec::new(), bytes: Vec::new(), rows: Vec::new(), written: 0, budget, group })
    }

    /// Add the rows of a part (written as bytes on all threads).
    pub fn push(&mut self, part: Vec<Feature>) -> Result<(), String> {
        const PIECE: usize = 4096;
        let first = self.written + self.rows.len() as u64;
        let pieces: Vec<(Vec<u8>, Vec<Placed>)> = part
            .par_chunks(PIECE)
            .enumerate()
            .map(|(p, rows)| {
                let (mut b, mut placed) = (Vec::new(), Vec::with_capacity(rows.len()));
                for (j, f) in rows.iter().enumerate() {
                    let n = first + (p * PIECE + j) as u64;
                    let start = b.len();
                    Numbered::put_row(n, f, &mut b);
                    placed.push((ordered(f.center.0), ordered(f.center.1), n, start, b.len() - start));
                }
                (b, placed)
            })
            .collect();
        drop(part);
        for (b, placed) in pieces {
            let base = self.bytes.len();
            self.bytes.extend_from_slice(&b);
            self.rows.extend(placed.into_iter().map(|(x, y, n, start, len)| (x, y, n, base + start, len)));
        }
        if self.bytes.len() + self.rows.len() * std::mem::size_of::<Placed>() >= self.budget {
            self.flush().map_err(|e| format!("quack sort: {e}"))?;
        }
        Ok(())
    }

    /// Write the rows of the buffer as a new run, sorted.
    fn flush(&mut self) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        self.rows.par_sort_unstable_by_key(|r| (r.0, r.1, r.2));
        let bytes = &self.bytes;
        let blocks: Vec<(Vec<u8>, usize)> = self
            .rows
            .par_chunks(BLOCK_ROWS)
            .map(|rows| {
                let mut raw = Vec::new();
                for &(_, _, _, start, len) in rows {
                    raw.extend_from_slice(&(len as u32).to_le_bytes());
                    raw.extend_from_slice(&bytes[start..start + len]);
                }
                let n = raw.len();
                Ok((pack(raw)?, n))
            })
            .collect::<std::io::Result<_>>()?;
        self.file.seek(SeekFrom::Start(self.end))?;
        let mut run = Vec::with_capacity(blocks.len());
        let mut w = std::io::BufWriter::with_capacity(1 << 20, &mut self.file);
        for (b, raw_len) in blocks {
            w.write_all(&b)?;
            run.push((self.end, b.len(), raw_len));
            self.end += b.len() as u64;
        }
        w.flush()?;
        drop(w);
        self.runs.push(run);
        self.written += self.rows.len() as u64;
        self.rows.clear();
        self.bytes.clear();
        Ok(())
    }

    /// Give the rows in the order of the sort, in pieces of a row group.
    pub fn finish(mut self, mut emit: impl FnMut(Vec<Feature>) -> Result<(), String>) -> Result<(), String> {
        use std::cmp::Reverse;
        let io = |e: std::io::Error| format!("quack sort: {e}");
        let group = self.group;
        let n = (self.written + self.rows.len() as u64) as usize;
        let (_, strip_size) = strips(n.max(1), group);
        if !self.rows.is_empty() {
            self.flush().map_err(io)?;
        }
        // the buffer of the rows is the buffer of the strips now
        let (mut bytes, mut strip) = (std::mem::take(&mut self.bytes), std::mem::take(&mut self.rows));
        // the merge of the runs: the smallest (x, y, row number) first
        let mut tmp = Vec::new();
        let mut cursors: Vec<Cursor> = Vec::with_capacity(self.runs.len());
        let mut heap = std::collections::BinaryHeap::new();
        for run in std::mem::take(&mut self.runs) {
            let mut c = Cursor { blocks: run.into(), buf: Vec::new(), pos: 0, row: (0, 0) };
            if c.advance(&mut *self.file, &mut tmp).map_err(io)? {
                heap.push(Reverse((numbered_key(c.bytes()), cursors.len())));
            }
            cursors.push(c);
        }
        let mut out: Vec<Feature> = Vec::with_capacity(group);
        let mut up = true;
        loop {
            let next = heap.pop();
            if let Some(Reverse(((x, _, n), i))) = next {
                // the strip: by y (up in the even strips, down in the odd strips), then x, then the row number
                let b = cursors[i].bytes();
                let y = f64::from_le_bytes(b[16..24].try_into().unwrap());
                strip.push((ordered(if up { y } else { -y }), x, n, bytes.len(), b.len()));
                bytes.extend_from_slice(b);
                if cursors[i].advance(&mut *self.file, &mut tmp).map_err(io)? {
                    heap.push(Reverse((numbered_key(cursors[i].bytes()), i)));
                }
            }
            let more = next.is_some();
            if strip.len() == strip_size || (!more && !strip.is_empty()) {
                strip.par_sort_unstable_by_key(|r| (r.0, r.1, r.2));
                // the rows of the strip are read on all threads, a row group at a time
                let mut i = 0;
                while i < strip.len() {
                    let take = (group - out.len()).min(strip.len() - i);
                    let rows: Vec<Feature> =
                        strip[i..i + take].par_iter().map(|r| Numbered.get(&mut &bytes[r.3..r.3 + r.4]).1).collect();
                    out.extend(rows);
                    i += take;
                    if out.len() == group {
                        emit(std::mem::replace(&mut out, Vec::with_capacity(group)))?;
                    }
                }
                strip.clear();
                bytes.clear();
                up = !up;
            }
            if !more {
                break;
            }
        }
        if !out.is_empty() {
            emit(out)?;
        }
        crate::util::step("quack 5: the sort");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_sort_on_disk_gives_the_order_of_str_order() {
        // 5 000 rows in groups of 100: 50 groups, 8 strips; a small budget gives about 80 runs; repeated centers test the
        // ties, and NaN the ends
        let pts: Vec<Pt> = (0..5000u64)
            .map(|i| match i % 97 {
                0 => (f64::NAN, 1.0),
                1 => (-f64::NAN, 2.0),
                _ => (((i * 7919) % 613) as f64 / 37.0 - 8.0, ((i * 104729) % 211) as f64 / 13.0 % 5.0),
            })
            .collect();
        let mut sort = StrSort::new(10_000, 100).unwrap();
        let rows: Vec<Feature> = pts
            .iter()
            .enumerate()
            .map(|(i, &c)| Feature { feature_id: format!("node/{i}"), tags: vec![("k".into(), i.to_string())], geometry: vec![i as u8], center: c })
            .collect();
        // parts of other sizes, larger and smaller than a piece of push
        let mut rest = &rows[..];
        for size in [7, 5000, 1, 4500].iter().cycle() {
            if rest.is_empty() {
                break;
            }
            let (part, r) = rest.split_at((*size).min(rest.len()));
            sort.push(part.to_vec()).unwrap();
            rest = r;
        }
        let mut got = Vec::new();
        sort.finish(|g| {
            assert!(g.len() == 100 || got.len() + g.len() == 5000);
            got.extend(g.into_iter().map(|f| f.feature_id));
            Ok(())
        })
        .unwrap();
        let want: Vec<String> = str_order_by(&pts, 100).into_iter().map(|i| format!("node/{i}")).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn str_order_sorts_one_strip_by_y_then_x() {
        // 16 points: one row group, so one strip
        let pts: Vec<Pt> = (0..16).rev().map(|i| ((i % 4) as f64, (i / 4) as f64)).collect();
        let c: Vec<Pt> = str_order(&pts).into_iter().map(|i| pts[i]).collect();
        assert_eq!((c[0], c[1], c[4], c[15]), ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (3.0, 3.0)));
    }

    fn sq(x: f64, y: f64, s: f64) -> Vec<Pt> {
        vec![(x, y), (x + s, y), (x + s, y + s), (x, y + s), (x, y)]
    }

    #[test]
    fn merge_joins_two_halves_into_a_ring() {
        let a = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)];
        let b = vec![(0.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
        let m = line_merge(&[a, b]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].len(), 5);
        assert_eq!(m[0].first(), m[0].last());
    }

    #[test]
    fn merge_stops_at_a_node_of_degree_three() {
        let m = line_merge(&[vec![(0.0, 0.0), (1.0, 0.0)], vec![(1.0, 0.0), (2.0, 0.0)], vec![(1.0, 0.0), (1.0, 1.0)]]);
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn multipolygon_with_a_hole() {
        let g = relation_geometry(&[("outer", sq(0.0, 0.0, 10.0)), ("inner", sq(2.0, 2.0, 2.0))]).unwrap();
        match g {
            Geom::Polygons(p) => assert_eq!((p.len(), p[0].len()), (1, 2)),
            _ => panic!("not a polygon"),
        }
    }

    #[test]
    fn union_of_rings_in_both_directions() {
        use geo::Area;
        let mut cw = sq(1.0, 0.0, 2.0);
        cw.reverse();
        let u = union(vec![to_geo(&[vec![sq(0.0, 0.0, 2.0)]]), to_geo(&[vec![cw]])]);
        assert_eq!(u.unsigned_area(), 6.0);
    }

    #[test]
    fn open_ring_drops_the_relation() {
        assert!(relation_geometry(&[("outer", vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])]).is_none());
    }

    #[test]
    fn no_outer_role_makes_all_rings_outer() {
        let g = relation_geometry(&[("part", sq(0.0, 0.0, 1.0)), ("inner", sq(5.0, 5.0, 1.0))]).unwrap();
        match g {
            Geom::Polygons(p) => assert_eq!(p.len(), 2),
            _ => panic!("not a polygon"),
        }
    }

    #[test]
    fn bowtie_is_repaired_into_two_polygons() {
        let g = ring_polygon(vec![(0.0, 0.0), (0.0, 10.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.0)]);
        match g {
            Geom::Polygons(p) => assert_eq!(p.len(), 2),
            _ => panic!("not a polygon"),
        }
    }

    #[test]
    fn metadata_tags_and_polygon_rules() {
        let t = |v: &[(&str, &str)]| v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<RawTags>();
        assert!(clean(&t(&[("source", "x"), ("openGeoDB:id", "1")])).is_empty());
        assert!(polygon_tags(&t(&[("building", "yes")])));
        assert!(!polygon_tags(&t(&[("building", "yes"), ("area", "no")])));
        assert!(polygon_tags(&t(&[("barrier", "wall")])));
        assert!(!polygon_tags(&t(&[("barrier", "fence")])));
        assert!(!polygon_tags(&t(&[("natural", "tree_row")])));
        assert!(polygon_tags(&t(&[("natural", "wood")])));
        assert!(!polygon_tags(&t(&[("highway", "residential")])));
    }
}
