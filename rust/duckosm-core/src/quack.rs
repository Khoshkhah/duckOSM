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
use geo::orient::{Direction, Orient};
use geo::algorithm::sweep::Intersections;
use geo::algorithm::coordinate_position::CoordPos;
use geo::algorithm::dimensions::Dimensions;
use geo::{BoundingRect, Intersects, Line, LineString, MultiPolygon, Polygon, Relate};
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

/// GEOS `ST_Union_Agg` of polygons: when no two of them meet, the polygons as they are; else the union.
fn union(parts: Vec<MultiPolygon>) -> MultiPolygon {
    let all: Vec<Polygon> = parts.into_iter().flat_map(|m| oriented(m).0).collect();
    let boxes: Vec<_> = all.iter().map(|p| p.bounding_rect()).collect();
    let meet = (0..all.len()).any(|i| {
        (i + 1..all.len()).any(|j| match (boxes[i], boxes[j]) {
            (Some(a), Some(b)) => a.intersects(&b) && all[i].intersects(&all[j]),
            _ => false,
        })
    });
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
    let pieces: Vec<MultiPolygon> = outer
        .into_iter()
        .map(|og| {
            // the inner polygons in this outer polygon, and if each of them is away from its border
            let mut apart = true;
            let holes: Vec<&MultiPolygon> = inner
                .iter()
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
            let boxes: Vec<_> = holes.iter().map(|h| h.bounding_rect()).collect();
            let disjoint = (0..holes.len()).all(|i| {
                (i + 1..holes.len()).all(|j| match (boxes[i], boxes[j]) {
                    (Some(a), Some(b)) => !a.intersects(&b) || !holes[i].intersects(holes[j]),
                    _ => true,
                })
            });
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

/// The elements of one block, as the first step leaves them: the nodes (their rounded points), the features of the
/// nodes, the ways and the relations of the types multipolygon and boundary.
#[derive(Default)]
struct Part {
    node_ids: Vec<i64>,
    node_pts: Vec<Pt>,
    features: Vec<Feature>,
    ways: Vec<(i64, RawTags, Vec<i64>)>,
    relations: Vec<(i64, RawTags, Vec<(i64, String)>)>,
}

impl RawSink for Part {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        let p = (round7(lon), round7(lat));
        self.node_ids.push(id);
        self.node_pts.push(p);
        let tags: RawTags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let tags = clean(&tags);
        if !tags.is_empty() {
            self.features.push(Feature::new(format!("node/{id}"), tags, &Geom::Point(p)));
        }
    }

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

/// The points of the nodes, sorted by id, and the node lists of the valid ways (the ways with all their nodes),
/// which the relations need.
#[derive(Default)]
struct Store {
    ids: Vec<i64>,
    pts: Vec<Pt>,
    way_ids: Vec<i64>,
    way_at: Vec<usize>,
    way_refs: Vec<i64>,
}

impl Store {
    fn point(&self, id: i64) -> Option<Pt> {
        self.ids.binary_search(&id).ok().map(|i| self.pts[i])
    }
    fn points(&self, refs: &[i64]) -> Option<Vec<Pt>> {
        refs.iter().map(|&r| self.point(r)).collect()
    }
    fn way_points(&self, id: i64) -> Option<Vec<Pt>> {
        let i = self.way_ids.binary_search(&id).ok()?;
        let end = self.way_at.get(i + 1).copied().unwrap_or(self.way_refs.len());
        self.points(&self.way_refs[self.way_at[i]..end])
    }
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

/// The features table of the `.osm.pbf` file in `reader` (sorted by type and id). `emit` gets the features in parts,
/// as they are made: the nodes and the ways while the file is read, the relations at the end. The blocks of the file
/// are decoded in parallel, `threads` blocks at a time.
pub fn features<R: Read + Send>(
    reader: R,
    threads: usize,
    mut emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    let threads = threads.max(1);
    let mut store = Store::default();
    let mut relations = Vec::new();
    let mut counts = Counts::default();
    let mut sorted = true;
    crate::pbf::stream_raw(
        reader,
        |_, round| round.len() < 2 * threads,
        Part::default,
        |p: Part| Ok(p),
        |part: Part, _| {
            if let (Some(&last), Some(&first)) = (store.ids.last(), part.node_ids.first()) {
                sorted &= last < first;
            }
            if !part.ways.is_empty() && !sorted {
                return Err("quack: the .osm.pbf file is not sorted by type and id (nodes first)".into());
            }
            store.ids.extend_from_slice(&part.node_ids);
            store.pts.extend_from_slice(&part.node_pts);
            counts.nodes += part.features.len();
            emit(part.features)?;
            let made: Vec<(Option<Vec<i64>>, Option<Feature>)> = part
                .ways
                .par_iter()
                .map(|(id, tags, refs)| match store.points(refs) {
                    Some(pts) => (Some(refs.clone()), way_feature(*id, tags, &pts)),
                    None => (None, None),
                })
                .collect();
            let mut out = Vec::new();
            for ((id, _, _), (refs, f)) in part.ways.iter().zip(made) {
                if let Some(refs) = refs {
                    store.way_ids.push(*id);
                    store.way_at.push(store.way_refs.len());
                    store.way_refs.extend(refs);
                }
                out.extend(f);
            }
            counts.ways += out.len();
            emit(out)?;
            relations.extend(part.relations);
            Ok(())
        },
    )?;
    let made: Vec<Feature> = relations
        .par_iter()
        .filter_map(|(id, tags, members)| {
            let lines: Option<Vec<(&str, Vec<Pt>)>> =
                members.iter().map(|(w, role)| Some((role.as_str(), store.way_points(*w)?))).collect();
            let geometry = relation_geometry(&lines?)?;
            let tags = clean(tags);
            (!tags.is_empty()).then(|| Feature::new(format!("relation/{id}"), tags, &geometry))
        })
        .collect();
    counts.relations = made.len();
    emit(made)?;
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
    let n = centers.len();
    if n == 0 {
        return Vec::new();
    }
    let strips = (n.div_ceil(ROW_GROUP) as f64).sqrt().ceil().max(1.0) as usize;
    let strip_size = n.div_ceil(strips);
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

#[cfg(test)]
mod tests {
    use super::*;

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
