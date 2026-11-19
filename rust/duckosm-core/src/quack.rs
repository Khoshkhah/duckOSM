//! duckosm quack: the features table of QuackOSM 0.20.0 (`feature_id`, `tags`, `geometry`), made in Rust from one
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
use crate::tiles::Tile;
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
#[derive(Clone, Default)]
pub struct Feature {
    pub feature_id: String,
    pub tags: RawTags,
    pub geometry: Vec<u8>,
    /// The center of the bounding box of the geometry: the key of [`str_order`].
    pub center: Pt,
    /// The tile of a tile table owns the row ([`Owner`] with no `relations`); true in all other runs.
    pub owner: bool,
    /// quack --gdal: the layer of the row (`crate::gdal::POINTS` to `OTHER_RELATIONS`); 0 in the table of quack.
    pub layer: u8,
    /// quack --gdal by tiles: the row is one member of a relation of `multilinestrings` or `other_relations`, at this
    /// place in the members (`crate::gdal::Parts` joins the members of all tiles into the row). None: a whole row.
    pub part: Option<u32>,
}

impl Feature {
    fn new(feature_id: String, tags: RawTags, g: &Geom) -> Feature {
        Feature { feature_id, tags, geometry: g.wkb(), center: g.center(), owner: true, ..Default::default() }
    }

    /// A row of quack --gdal.
    fn gdal(feature_id: String, layer: u8, tags: RawTags, geometry: Vec<u8>, center: Pt) -> Feature {
        Feature { feature_id, tags, geometry, center, owner: true, layer, part: None }
    }
}

/// The rules of a layer run of the engine ([`gdal_features`]): GDAL's five layers (`duckosm quack --gdal`), or the rows
/// that the Geofabrik layers come from (`duckosm quack --geofabrik`, [`crate::geofabrik`]). The Geofabrik mode reads the
/// file as GDAL does, with two differences: a multipolygon with no tags of its own does not take the tags of its outer
/// member, and the polygon of an outer member way is a row also when a complete multipolygon uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Quack,
    Gdal,
    Osm2pgsql,
    Geofabrik,
}

/// The numbers of a [`features`] run.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub nodes: usize,
    pub ways: usize,
    pub relations: usize,
}

/// The OSM fixes (`crate::fixes::OsmFixes`, the file of `duckosm build --fixes`) as changes of the OSM data, for quack and
/// quack --gdal (docs/concepts/quackosm.md, "OSM fixes"). The reader applies them to each node, way and relation of the
/// file, in each pass and in each tile, so quack by tiles gives the same rows:
///
/// - `remove`, `keep_nodes` and `tags` of a rule, and the tag edits (`extra.tag_edits`) of nodes, ways and relations;
/// - the network rules as OSM tags: `oneway` (`oneway=yes` / `no`), `lanes`, `lanes_forward` (`lanes:forward`),
///   `lanes_backward` (`lanes:backward`) and `layer`, with the value of the rule;
/// - the new ways, with the ids of the network, from the points that [`QuackFixes::new`] finds in the file.
///
/// `exclude_modes`, the arrows of the lanes and the turn restrictions are not OSM data of a way, and quack leaves them out.
/// The type goes to the workers of the browser as JSON.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct QuackFixes {
    ways: crate::util::FxHashMap<i64, Change>,
    nodes: crate::util::FxHashMap<i64, Change>,
    relations: crate::util::FxHashMap<i64, Change>,
    new_ways: Vec<NewLine>,
}

/// The changes of one element. `tags`: in this order, set a tag (`Some`) or delete it (`None`).
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Change {
    remove: bool,
    keep: Option<(i64, i64)>,
    tags: Vec<(String, Option<String>)>,
}

impl Change {
    fn tags(&self, tags: &mut RawTags) {
        for (k, v) in &self.tags {
            match v {
                Some(v) => match tags.iter_mut().find(|t| &t.0 == k) {
                    Some(t) => t.1.clone_from(v),
                    None => tags.push((k.clone(), v.clone())),
                },
                None => tags.retain(|t| &t.0 != k),
            }
        }
    }
}

/// A new way: its points (rounded) and tags; `tile`: the tile (z, x, y) that owns it in quack by tiles.
#[derive(serde::Serialize, serde::Deserialize)]
struct NewLine {
    id: i64,
    tags: RawTags,
    pts: Vec<Pt>,
    /// the node ids of `pts`: an OSM node, or a new node `id * 1000 + position` (as `OsmFixes::apply_to_ways`)
    #[serde(default)]
    refs: Vec<i64>,
    closed: bool,
    tile: Option<(u8, u32, u32)>,
}

impl QuackFixes {
    /// The fixes of `fixes` for the `.osm.pbf` file that `open` opens. With new ways, one pass over the file finds the
    /// points of their OSM nodes and the tags of their `tags_from` ways. A new way with an OSM node that is not in the file
    /// is left out (the file of fixes is global), as in the build.
    pub fn new<R: Read + Send>(fixes: &crate::fixes::OsmFixes, open: impl Fn() -> Result<R, String>) -> Result<QuackFixes, String> {
        use crate::fixes::{Elem, NewNode, NEW_WAY_ID_MIN};
        let mut q = QuackFixes::default();
        for r in &fixes.ways {
            let c = q.ways.entry(r.osm_id).or_default();
            c.remove |= r.remove;
            c.keep = r.keep_nodes.or(c.keep);
            c.tags.extend(r.tags.iter().map(|(k, v)| (k.clone(), Some(v.clone()))));
        }
        for e in &fixes.tag_edits {
            let map = match e.elem {
                Elem::Node => &mut q.nodes,
                Elem::Way => &mut q.ways,
                Elem::Relation => &mut q.relations,
            };
            let c = map.entry(e.id).or_default();
            c.tags.extend(e.set.iter().map(|(k, v)| (k.clone(), Some(v.clone()))));
            c.tags.extend(e.delete.iter().map(|k| (k.clone(), None)));
        }
        // the network rules last: in the build they act after the tags
        for r in &fixes.ways {
            let num = |k: &str, v: Option<i32>| v.map(|v| (k.to_string(), Some(v.to_string())));
            let oneway = r.oneway.map(|b| ("oneway".to_string(), Some(if b { "yes" } else { "no" }.to_string())));
            let tags = [oneway, num("lanes", r.lanes), num("lanes:forward", r.lanes_forward), num("lanes:backward", r.lanes_backward), num("layer", r.layer)];
            q.ways.entry(r.osm_id).or_default().tags.extend(tags.into_iter().flatten());
        }
        if fixes.new_ways.is_empty() {
            return Ok(q);
        }
        // the points of the OSM nodes and the tags of the `tags_from` ways
        struct Find<'a> {
            nodes: &'a crate::util::FxHashSet<i64>,
            ways: &'a crate::util::FxHashSet<i64>,
            pts: Vec<(i64, Pt)>,
            tags: Vec<(i64, RawTags)>,
        }
        impl RawSink for Find<'_> {
            fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
                if self.nodes.contains(&id) {
                    self.pts.push((id, (round7(lon), round7(lat))));
                }
            }
            fn way<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = i64>) {
                if self.ways.contains(&id) {
                    self.tags.push((id, tags.map(|(k, v)| (k.to_string(), v.to_string())).collect()));
                }
            }
            fn relation<'a>(&mut self, _: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>) {}
        }
        let mut ids: (crate::util::FxHashSet<i64>, crate::util::FxHashSet<i64>) = Default::default();
        for n in &fixes.new_ways {
            ids.0.extend(n.nodes.iter().filter_map(|x| if let NewNode::Osm(id) = x { Some(*id) } else { None }));
            ids.1.extend(n.tags_from);
        }
        let (mut pts, mut from): (crate::util::FxHashMap<i64, Pt>, crate::util::FxHashMap<i64, RawTags>) = Default::default();
        crate::cutstream::each_block(
            &mut open()?,
            |b, buf| {
                let mut f = Find { nodes: &ids.0, ways: &ids.1, pts: Vec::new(), tags: Vec::new() };
                crate::pbfraw::decode_block(b, [!ids.0.is_empty(), !ids.1.is_empty(), false], buf, &mut f)?;
                Ok((f.pts, f.tags))
            },
            |(p, t), _| {
                pts.extend(p);
                from.extend(t);
                Ok(true)
            },
        )?;
        let mut seen = Vec::new();
        for n in &fixes.new_ways {
            if n.id < NEW_WAY_ID_MIN || seen.contains(&n.id) {
                return Err(format!("osm_overrides new_ways: id {} is below {NEW_WAY_ID_MIN} or used two times", n.id));
            }
            seen.push(n.id);
            let line = n.nodes.iter().map(|x| match x {
                NewNode::Osm(id) => pts.get(id).copied(),
                NewNode::At((lon, lat)) => Some((round7(*lon), round7(*lat))),
            });
            let Some(line) = line.collect::<Option<Vec<Pt>>>() else {
                continue;
            };
            // the own tags first: they have priority over the tags of `tags_from`
            let mut tags: RawTags = n.tags.clone();
            for (k, v) in n.tags_from.and_then(|id| from.get(&id)).into_iter().flatten() {
                if !tags.iter().any(|t| &t.0 == k) {
                    tags.push((k.clone(), v.clone()));
                }
            }
            if let Some(c) = q.ways.get(&n.id) {
                c.tags(&mut tags);
            }
            let closed = match (n.nodes.first(), n.nodes.last()) {
                (Some(NewNode::Osm(a)), Some(NewNode::Osm(b))) => a == b,
                _ => n.nodes.len() > 1 && line.first() == line.last(),
            };
            let refs = n.nodes.iter().enumerate().map(|(i, x)| match x {
                NewNode::Osm(id) => *id,
                NewNode::At(_) => n.id * 1000 + i as i64 + 1,
            });
            q.new_ways.push(NewLine { id: n.id, tags, pts: line, refs: refs.collect(), closed, tile: None });
        }
        Ok(q)
    }

    /// The fixes as JSON, for the workers of the browser and the DuckDB table function.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("the fixes as JSON")
    }

    pub fn from_json(text: &str) -> Result<QuackFixes, String> {
        serde_json::from_str(text).map_err(|e| format!("the OSM fixes of quack: {e}"))
    }

    /// quack by tiles: each new way goes to the tile of `tiles` with its first point, or else to the first tile (a tile
    /// with no OSM node has no run), so one tile makes it.
    pub fn place(&mut self, tiles: &[Tile]) {
        for l in &mut self.new_ways {
            let Some(&(lon, lat)) = l.pts.first() else { continue };
            l.tile = tiles.iter().find(|t| Tile::of(t.z, lon, lat) == **t).or(tiles.first()).map(|t| (t.z, t.x, t.y));
        }
    }

    /// Change the tags of node `id` (`cut --fixes`).
    pub(crate) fn node_tags(&self, id: i64, tags: &mut RawTags) {
        fix_tags(Some(&self.nodes), id, tags);
    }

    /// Change the tags of relation `id` (`cut --fixes`).
    pub(crate) fn relation_tags(&self, id: i64, tags: &mut RawTags) {
        fix_tags(Some(&self.relations), id, tags);
    }

    /// The new ways as (id, tags, node ids, points), sorted by id (`cut --fixes`).
    pub(crate) fn new_ways(&self) -> Vec<(i64, &RawTags, &[i64], &[Pt])> {
        let mut v: Vec<_> = self.new_ways.iter().map(|l| (l.id, &l.tags, &l.refs[..], &l.pts[..])).collect();
        v.sort_by_key(|w| w.0);
        v
    }

    /// Change the tags and the nodes of way `id`; false: the way is removed.
    pub(crate) fn way(&self, id: i64, tags: &mut RawTags, refs: &mut Vec<i64>) -> bool {
        let Some(c) = self.ways.get(&id) else { return true };
        if c.remove {
            return false;
        }
        // DuckDB refs[list_position(a) : list_position(b)], as `OsmFixes::apply_to_ways`
        if let Some((a, b)) = c.keep {
            if let (Some(pa), Some(pb)) = (refs.iter().position(|&n| n == a), refs.iter().position(|&n| n == b)) {
                if pa < pb {
                    refs.truncate(pb + 1);
                    refs.drain(..pa);
                }
            }
        }
        c.tags(tags);
        true
    }

    /// The rows of the new ways that `owner` owns (all with no owner).
    fn new_rows(&self, owner: Option<&Owner>, mode: Mode) -> Vec<Feature> {
        let mine = |l: &NewLine| {
            owner.is_none_or(|o| match l.tile {
                Some((z, x, y)) => (o.tile.z, o.tile.x, o.tile.y) == (z, x, y),
                None => l.pts.first().is_some_and(|p| Tile::of(o.tile.z, p.0, p.1) == o.tile),
            })
        };
        let row = |l: &NewLine| match mode {
            Mode::Gdal | Mode::Geofabrik => (l.pts.len() >= 2).then(|| gdal_way(l.id, &l.tags, &l.pts, l.closed && crate::gdal::is_area(&l.tags))).flatten(),
            Mode::Osm2pgsql => crate::osm2pgsql::way(l.id, &l.tags, &l.pts, l.closed),
            Mode::Quack => way_feature(l.id, &l.tags, &l.pts),
        };
        self.new_ways.iter().filter(|l| mine(l)).filter_map(row).collect()
    }
}

/// The tags of element `id` of `map` after its changes.
fn fix_tags(map: Option<&crate::util::FxHashMap<i64, Change>>, id: i64, tags: &mut RawTags) {
    if let Some(c) = map.and_then(|m| m.get(&id)) {
        c.tags(tags);
    }
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

/// For each polygon of `inner`: is it within `outer`, and is its border away from the border of `outer`? The answers of
/// `geo`'s `relate` that [`relation_geometry`] needs. `relate` of each inner ring against a long outer ring took 57.6 s for
/// relation 1239458 of Sweden (3 000 inner rings, an outer ring of 213 056 points); here one sweep over all edges finds the
/// polygons whose border meets the border of `outer`, and only these get the exact `relate`. Any other polygon is all
/// inside or all outside `outer`, so one vertex of each of its parts tells.
fn within_many(outer: &MultiPolygon, inner: &[&MultiPolygon]) -> Vec<(bool, bool)> {
    use geo::algorithm::coordinate_position::CoordinatePosition;
    let rings = |m: &MultiPolygon, poly: usize| -> Vec<Edge> {
        m.0.iter()
            .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
            .flat_map(|r| r.lines().map(move |line| Edge { line, poly }))
            .collect()
    };
    let mut edges = rings(outer, 0);
    for (k, m) in inner.iter().enumerate() {
        edges.extend(rings(m, k + 1));
    }
    let mut meets = vec![false; inner.len()];
    for (a, b, _) in Intersections::from_iter(edges.iter()) {
        match (a.poly, b.poly) {
            (0, k) | (k, 0) if k > 0 => meets[k - 1] = true,
            _ => {}
        }
    }
    inner
        .iter()
        .zip(meets)
        .map(|(m, meets)| {
            if meets {
                let r = m.relate(outer);
                let within = r.is_within();
                (within, !within || r.get(CoordPos::OnBoundary, CoordPos::OnBoundary) == Dimensions::Empty)
            } else {
                let inside = m.0.iter().all(|p| p.exterior().0.first().is_some_and(|c| outer.coordinate_position(c) == CoordPos::Inside));
                (inside, true)
            }
        })
        .collect()
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
            let candidates: Vec<&MultiPolygon> = inner
                .iter()
                .enumerate()
                .filter(|(i, ig)| in_box(*i) && !ig.0.is_empty() && !og.0.is_empty())
                .map(|(_, ig)| ig)
                .collect();
            let mut apart = true;
            let holes: Vec<&MultiPolygon> = candidates
                .iter()
                .zip(within_many(&og, &candidates))
                .filter_map(|(ig, (within, away))| {
                    apart &= !within || away;
                    within.then_some(*ig)
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
/// multipolygon or boundary with its member ways (quack --gdal: each relation with its member nodes and ways, and the
/// roles as they are).
#[derive(Default)]
struct WaysPart<'f> {
    gdal: bool,
    fixes: Option<&'f QuackFixes>,
    ways: Vec<(i64, RawTags, Vec<i64>)>,
    relations: Vec<(i64, RawTags, Vec<Member>)>,
}

/// A member of a relation: its id, its type and its role.
type Member = (i64, MemberType, String);

fn member_type(t: MemberType) -> u8 {
    match t {
        MemberType::Node => 0,
        MemberType::Way => 1,
        MemberType::Relation => 2,
    }
}

fn member_of(b: u8) -> MemberType {
    match b {
        0 => MemberType::Node,
        1 => MemberType::Way,
        _ => MemberType::Relation,
    }
}

impl RawSink for WaysPart<'_> {
    fn node<'a>(&mut self, _: i64, _: f64, _: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {}

    fn way<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        let mut refs: Vec<i64> = refs.collect();
        let mut tags: RawTags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        if self.fixes.is_some_and(|f| !f.way(id, &mut tags, &mut refs)) {
            return;
        }
        if refs.len() >= 2 {
            self.ways.push((id, tags, refs));
        }
    }

    fn relation<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        let mut tags: RawTags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        fix_tags(self.fixes.map(|f| &f.relations), id, &mut tags);
        if self.gdal {
            // GDAL: each relation, with its member nodes and ways
            let m: Vec<Member> = members.filter(|m| m.1 != MemberType::Relation).map(|m| (m.0, m.1, m.2.to_string())).collect();
            if !m.is_empty() {
                self.relations.push((id, tags, m));
            }
            return;
        }
        if !matches!(tag(&tags, "type"), Some("multipolygon" | "boundary")) {
            return;
        }
        let ways: Vec<Member> =
            // QuackOSM: COALESCE(ref_role, 'outer'), and ST_READOSM gives NULL for an empty role
            members
                .filter(|m| m.1 == MemberType::Way)
                .map(|m| (m.0, m.1, if m.2.is_empty() { "outer" } else { m.2 }.to_string()))
                .collect();
        if !ways.is_empty() {
            self.relations.push((id, tags, ways));
        }
    }
}

/// The nodes of one block (pass 2): each node for the join, and the features of the nodes with tags.
#[derive(Default)]
struct NodesPart<'f> {
    fixes: Option<&'f QuackFixes>,
    nodes: Vec<(i64, f64, f64)>,
    features: Vec<Feature>,
    /// quack by tiles and the tile tables: only the nodes in this tile are features
    tile: Option<Tile>,
    gdal: bool,
    osm2pgsql: bool,
}

impl RawSink for NodesPart<'_> {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        self.nodes.push((id, lon, lat));
        let mut tags: RawTags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        fix_tags(self.fixes.map(|f| &f.nodes), id, &mut tags);
        if self.osm2pgsql {
            if self.tile.is_none_or(|t| Tile::of(t.z, lon, lat) == t) {
                self.features.extend(crate::osm2pgsql::point(id, &tags, (lon, lat)));
            }
            return;
        }
        if self.gdal {
            if crate::gdal::is_point(&tags) && self.tile.is_none_or(|t| Tile::of(t.z, lon, lat) == t) {
                let geometry = crate::gdal::wkb_point((lon, lat));
                self.features.push(Feature::gdal(format!("node/{id}"), crate::gdal::POINTS, tags, geometry, (lon, lat)));
            }
            return;
        }
        let tags = clean(&tags);
        if !tags.is_empty() && self.tile.is_none_or(|t| Tile::of(t.z, lon, lat) == t) {
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

/// The header of a way in the temporary file of pass 1: its id, its number of nodes, its tags (before [`clean`]), and
/// whether its first node is its last node.
struct Header;

impl Codec<(i64, u32, RawTags, bool)> for Header {
    fn put(&self, r: &(i64, u32, RawTags, bool), out: &mut Vec<u8>) {
        out.extend_from_slice(&r.0.to_le_bytes());
        put_u32(out, r.1 as usize);
        put_tags(out, &r.2);
        out.push(r.3 as u8);
    }
    fn get(&self, b: &mut &[u8]) -> (i64, u32, RawTags, bool) {
        (get_i64(b), get_u32(b), get_tags(b), take(b, 1)[0] == 1)
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

/// A relation (pass 1): its id, its tags and its members, in their order.
struct RelHeader;

impl Codec<(i64, RawTags, Vec<Member>)> for RelHeader {
    fn put(&self, r: &(i64, RawTags, Vec<Member>), out: &mut Vec<u8>) {
        out.extend_from_slice(&r.0.to_le_bytes());
        put_tags(out, &r.1);
        put_u32(out, r.2.len());
        for (id, t, role) in &r.2 {
            out.extend_from_slice(&id.to_le_bytes());
            out.push(member_type(*t));
            put_str(out, role);
        }
    }
    fn get(&self, b: &mut &[u8]) -> (i64, RawTags, Vec<Member>) {
        let id = get_i64(b);
        let tags = get_tags(b);
        let n = get_u32(b);
        (id, tags, (0..n).map(|_| (get_i64(b), member_of(take(b, 1)[0]), get_str(b))).collect())
    }
}

/// A line of points on the grid with a key: the id of a member way, or the relation and the place of the member
/// (`relation << 32 | place`). The key comes first, so a sort of the bytes reads it with [`line_key`].
struct KeyedLine;

/// The flags of a [`KeyedLine`]: the tile owns the way (the tile tables, quack --gdal by tiles), the way is an area
/// (quack --gdal), the line is the point of a member node (quack --gdal).
const OWNED: u8 = 1;
const AREA: u8 = 2;
const NODE: u8 = 4;

/// A line of a member: its key, its flags, its points, and (quack --gdal) the tags that an area way gives to a
/// multipolygon with no tags of its own (`crate::gdal::way_tags`; else none).
type KLine = (i64, u8, Vec<Grid>, RawTags);

impl Codec<KLine> for KeyedLine {
    fn put(&self, r: &KLine, out: &mut Vec<u8>) {
        out.extend_from_slice(&r.0.to_le_bytes());
        out.push(r.1);
        put_u32(out, r.2.len());
        for g in &r.2 {
            out.extend_from_slice(&g[0].to_le_bytes());
            out.extend_from_slice(&g[1].to_le_bytes());
        }
        put_tags(out, &r.3);
    }
    fn get(&self, b: &mut &[u8]) -> KLine {
        let key = get_i64(b);
        let flags = take(b, 1)[0];
        let n = get_u32(b);
        let pts = (0..n)
            .map(|_| {
                let x = i32::from_le_bytes(take(b, 4).try_into().expect("4 bytes"));
                let y = i32::from_le_bytes(take(b, 4).try_into().expect("4 bytes"));
                [x, y]
            })
            .collect();
        (key, flags, pts, get_tags(b))
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

/// The number of `node/<id>`, `way/<id>` or `relation/<id>`.
fn feature_number(feature_id: &str) -> i64 {
    feature_id.split_once('/').and_then(|(_, n)| n.parse().ok()).expect("a feature id")
}

/// quack --gdal: the row of a way with tags and 2 or more points: the polygon of an area (when GDAL keeps a tag of it),
/// else a line.
fn gdal_way(id: i64, tags: &RawTags, pts: &[Pt], area: bool) -> Option<Feature> {
    use crate::gdal::*;
    if area {
        let tags = way_tags(tags);
        let g = wkb_multipolygon(&[vec![pts.to_vec()]]);
        return (!tags.is_empty()).then(|| Feature::gdal(format!("way/{id}"), MULTIPOLYGONS, tags, g, pts[0]));
    }
    Some(Feature::gdal(format!("way/{id}"), LINES, tags.clone(), wkb_line(pts), pts[0]))
}

/// quack --gdal: the rows of a relation from the lines of its members that the file has (in the order of the members).
///
/// - A multipolygon or boundary with all its member ways (not `subarea`): its outer closed member ways go to `used`; the
///   relation is a row when `mine` and it has a ring. `donate`: a relation with no tags of its own takes the tags of its
///   first outer member area (GDAL).
/// - Else the members (nodes only in `other_relations`; ways not `subarea`): one row, or `by_tiles` one row for each
///   member that the tile owns (`Feature::part`).
fn gdal_relation(
    id: i64,
    tags: RawTags,
    members: &[Member],
    got: Vec<(usize, u8, Vec<Pt>, RawTags)>,
    by_tiles: bool,
    donate: bool,
    mine: &dyn Fn(i64) -> bool,
    used: &std::sync::Mutex<Vec<i64>>,
) -> Vec<Feature> {
    use crate::gdal::*;
    let layer = relation_layer(&tags);
    let way = |place: usize| members[place].1 == MemberType::Way && members[place].2 != "subarea";
    if layer == MULTIPOLYGONS {
        let need = (0..members.len()).filter(|&p| way(p)).count();
        let have: Vec<&(usize, u8, Vec<Pt>, RawTags)> = got.iter().filter(|l| way(l.0)).collect();
        if have.len() != need {
            return Vec::new();
        }
        let outer = |l: &&&(usize, u8, Vec<Pt>, RawTags)| members[l.0].2 == "outer";
        used.lock().expect("the used ways").extend(
            have.iter().filter(outer).filter(|l| !l.2.is_empty() && l.2.first() == l.2.last()).map(|l| members[l.0].0),
        );
        if !mine(id) {
            return Vec::new();
        }
        let Some(polys) = multipolygon(have.iter().map(|l| l.2.clone()).collect()) else {
            return Vec::new();
        };
        let tags = if has_own_tags(&tags) || !donate {
            tags
        } else {
            // GDAL: the tags of the first outer member area, and the type
            let mut t = have.iter().filter(outer).map(|l| &l.3).find(|d| !d.is_empty()).cloned().unwrap_or_default();
            if let Some(v) = tags.iter().rev().find(|t| t.0 == "type") {
                t.push(v.clone());
            }
            t
        };
        let center = polys[0][0][0];
        // a tile alone (no `relations`): the tile of the first node of the first member way owns the relation
        let owner = have.first().is_some_and(|l| l.1 & OWNED != 0);
        return vec![Feature { owner, ..Feature::gdal(format!("relation/{id}"), MULTIPOLYGONS, tags, wkb_multipolygon(&polys), center) }];
    }
    let mut parts: Vec<(usize, Part)> = Vec::new();
    for (place, flags, pts, _) in got {
        if by_tiles && flags & OWNED == 0 {
            continue;
        }
        if members[place].1 == MemberType::Node {
            if layer == OTHER_RELATIONS {
                parts.push((place, Part::Point(pts[0])));
            }
        } else if way(place) {
            parts.push((place, if layer == OTHER_RELATIONS && flags & AREA != 0 { Part::Area(pts) } else { Part::Line(pts) }));
        }
    }
    let center = |p: &Part| match p {
        Part::Point(x) => *x,
        Part::Line(l) | Part::Area(l) => l[0],
    };
    if by_tiles {
        return parts
            .into_iter()
            .map(|(place, p)| Feature { part: Some(place as u32), ..Feature::gdal(format!("relation/{id}"), layer, tags.clone(), p.wkb(), center(&p)) })
            .collect();
    }
    if parts.is_empty() {
        return Vec::new();
    }
    let wkb: Vec<Vec<u8>> = parts.iter().map(|p| p.1.wkb()).collect();
    let g = wkb_collection(layer, &wkb.iter().map(|w| w.as_slice()).collect::<Vec<_>>());
    vec![Feature::gdal(format!("relation/{id}"), layer, tags, g, center(&parts[0].1))]
}

/// quack --osm2pgsql: the rows of a relation ([`crate::osm2pgsql::relation`]) from the lines of its member ways that the file
/// has (in the order of the members): the polygons and the line of a multipolygon or boundary when `mine` (the cut puts it
/// whole in each tile with a member); the line of a route, or `by_tiles` one part for each member way that the tile owns
/// (`Feature::part`).
fn osm2pgsql_relation(id: i64, tags: &RawTags, members: &[Member], got: Vec<(usize, u8, Vec<Pt>, RawTags)>, by_tiles: bool, mine: &dyn Fn(i64) -> bool) -> Vec<Feature> {
    let Some(rule) = crate::osm2pgsql::relation(tags) else { return Vec::new() };
    let ways: Vec<(usize, u8, Vec<Pt>)> = got.into_iter().filter(|l| members[l.0].1 == MemberType::Way).map(|l| (l.0, l.1, l.2)).collect();
    if rule.polygons {
        if !mine(id) {
            return Vec::new();
        }
        let owner = ways.first().is_some_and(|l| l.1 & OWNED != 0);
        let lines: Vec<Vec<Pt>> = ways.into_iter().map(|l| l.2).collect();
        return crate::osm2pgsql::relation_rows(id, &rule, &lines).into_iter().map(|f| Feature { owner, ..f }).collect();
    }
    if by_tiles {
        return ways
            .into_iter()
            .filter(|l| l.1 & OWNED != 0)
            .map(|(place, _, pts)| crate::osm2pgsql::relation_part(id, &rule, place as u32, &pts))
            .collect();
    }
    let lines: Vec<Vec<Pt>> = ways.into_iter().map(|l| l.2).collect();
    crate::osm2pgsql::relation_rows(id, &rule, &lines)
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
    fixes: Option<&QuackFixes>,
    emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    features_owned(open, budget, None, fixes, emit)
}

/// The rows that the file of one tile gives in quack by tiles (`crate::tilecut::cut_to_store`, docs/rust/quack.md):
/// the cut gives each tile its nodes, each way with a node in the tile (whole, with its nodes), and each multipolygon and
/// boundary with a member there (with all its members), so a row that a tile can make is the row of the whole file. Some
/// rows come from more than one tile; one tile owns each row:
///
/// - a node: the tile of its place;
/// - a way: the tile of the place of its first node (the cut and quack decode the same coordinates, so the tile that owns
///   the way has it);
/// - a relation: with `relations` (quack by tiles), the first tile that makes it (the ids that the tiles before made). With
///   no `relations` (the tile tables and the workers of the browser), the tile of the first node of its first member way.
///   The cut puts each multipolygon and boundary whole in each tile with a member, so that tile has it.
///
/// With `relations`, a tile gives only the rows that it owns. With no `relations`, a tile gives all the rows of its own
/// file, each with [`Feature::owner`], with no look at the other tiles: the nodes in the tile, the ways with a node in the
/// tile, and the relations that the file has whole (not the nodes and member ways that the cut added from other tiles).
pub struct Owner<'a> {
    pub tile: Tile,
    pub relations: Option<&'a std::sync::Mutex<crate::util::FxHashSet<i64>>>,
}

/// [`features`] with only the rows that `owner` owns (all rows with `None`).
pub fn features_owned<R: Read + Send>(
    open: impl Fn() -> Result<R, String>,
    budget: usize,
    owner: Option<&Owner>,
    fixes: Option<&QuackFixes>,
    emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    run(open, budget, owner, Mode::Quack, fixes, emit)
}

/// quack --gdal: the rows of GDAL's five layers ([`crate::gdal`], each row with its `layer`) from one read of the file, as
/// [`features`] reads it. `emit` gets the points, the lines and the polygons of the ways, the relations, then the polygons
/// of the ways that are outer members of a multipolygon (GDAL leaves out the ones of a complete multipolygon).
pub fn gdal_features<R: Read + Send>(
    open: impl Fn() -> Result<R, String>,
    budget: usize,
    mode: Mode,
    fixes: Option<&QuackFixes>,
    emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    run(open, budget, None, mode, fixes, emit)
}

/// [`gdal_features`] of the file of one tile alone (a worker of the browser for each tile): the rows that `owner` owns
/// have `Feature::owner`; with no `relations`, the tile of the first node of the first member way owns a multipolygon, and
/// the other relations come in parts (`Feature::part`, the members that the tile owns).
pub fn gdal_features_owned<R: Read + Send>(
    open: impl Fn() -> Result<R, String>,
    budget: usize,
    owner: &Owner,
    mode: Mode,
    fixes: Option<&QuackFixes>,
    emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    run(open, budget, Some(owner), mode, fixes, emit)
}

/// The engine of [`features_owned`] and [`gdal_features`]: `mode` chooses the rules of the layers.
fn run<R: Read + Send>(
    open: impl Fn() -> Result<R, String>,
    budget: usize,
    owner: Option<&Owner>,
    mode: Mode,
    fixes: Option<&QuackFixes>,
    mut emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    let gdal = mode != Mode::Quack;
    let o2p = mode == Mode::Osm2pgsql;
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
    // quack --gdal: the member nodes of the relations, and the ways that are outer members of a multipolygon or boundary
    let mut node_members = Writer::new(store).map_err(io)?;
    let mut member_nodes: Vec<i64> = Vec::new();
    let mut outer_ids: Vec<i64> = Vec::new();
    let (mut ways, mut rels) = (0u32, 0u32);
    let mut way = |id: i64, refs: &[i64], tags: RawTags, headers: &mut Writer, requests: &mut Buckets| -> Result<(), String> {
        for (k, &n) in refs.iter().enumerate() {
            requests.push(store, n, &Request, &(n, ways, k as u32)).map_err(io)?;
        }
        headers.push(&Header, &(id, refs.len() as u32, tags, refs.first() == refs.last())).map_err(io)?;
        ways = ways.checked_add(1).ok_or("quack: more than 4 294 967 295 ways")?;
        Ok(())
    };
    crate::cutstream::each_block(
        &mut open()?,
        |b, buf| {
            let mut part = WaysPart { gdal, fixes, ..Default::default() };
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
                let polygons = matches!(mode, Mode::Gdal | Mode::Geofabrik) && crate::gdal::relation_layer(&tags) == crate::gdal::MULTIPOLYGONS;
                for (place, (m, t, role)) in members.iter().enumerate() {
                    if *t == MemberType::Node {
                        node_members.push(&Request, &(*m, rels, place as u32)).map_err(io)?;
                        member_nodes.push(*m);
                        continue;
                    }
                    rel_members.push(&Request, &(*m, rels, place as u32)).map_err(io)?;
                    member_ids.push(*m);
                    if polygons && role == "outer" {
                        outer_ids.push(*m);
                    }
                }
                rel_headers.push(&RelHeader, &(id, tags, members)).map_err(io)?;
                rels = rels.checked_add(1).ok_or("quack: more than 4 294 967 295 relations")?;
            }
            Ok(true)
        },
    )?;
    for v in [&mut member_ids, &mut member_nodes, &mut outer_ids] {
        v.sort_unstable();
        v.dedup();
    }
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
    let mut node_lines = Writer::new(store).map_err(io)?;
    crate::cutstream::each_block(
        &mut open()?,
        |b, buf| {
            let mut part = NodesPart { tile: owner.map(|o| o.tile), gdal, osm2pgsql: o2p, fixes, ..Default::default() };
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
                if !member_nodes.is_empty() && member_nodes.binary_search(&id).is_ok() {
                    let here = owner.is_none_or(|o| Tile::of(o.tile.z, lon, lat) == o.tile);
                    let p = to_grid((round7(lon), round7(lat)));
                    node_lines.push(&KeyedLine, &(id, NODE | if here { OWNED } else { 0 }, vec![p], Vec::new())).map_err(io)?;
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
    // pass 3: the ways. A reader joins the headers with their points into batches; the features of each batch are made on
    // all threads and written, and the lines of the member ways go to a file.
    // (id, tags, points, owned, a node in the tile, an area of quack --gdal)
    type WayBatch = Vec<(i64, RawTags, Vec<Pt>, bool, bool, bool)>;
    let way_tile = owner.map(|o| o.tile);
    // the tile tables: a tile gives all its rows, each with its owner
    let mark = owner.is_some_and(|o| o.relations.is_none());
    let mut member_lines = Writer::new(store).map_err(io)?;
    let mut deferred = Writer::new(store).map_err(io)?;
    let headers = headers.finish().map_err(io)?;
    let read_ways = move |give: &mut dyn FnMut(WayBatch) -> Result<(), String>| -> Result<(), String> {
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
            let (id, refs, tags, closed) = h.map_err(io)?;
            let (mut pts, mut owned, mut touches) = (Vec::with_capacity(refs as usize), true, way_tile.is_none());
            let mut first_node = false;
            while let Some(p) = points.next_if(|p| p.as_ref().is_ok_and(|p| p.0 == number as u32)) {
                let p = p.map_err(io)?;
                if pts.is_empty() {
                    first_node = p.1 == 0;
                }
                if let Some(t) = way_tile {
                    let here = Tile::of(t.z, p.2, p.3) == t;
                    if pts.is_empty() {
                        owned = here;
                    }
                    touches |= here;
                }
                pts.push((round7(p.2), round7(p.3)));
            }
            if let Some(Err(_)) = points.peek() {
                return Err(io(points.next().expect("peeked").expect_err("an error")));
            }
            if o2p {
                // osm2pgsql keeps the nodes that the file has; `closed` (the ids) takes the place of the area
                if pts.len() >= 2 {
                    batch.push((id, tags, pts, owned, touches, closed));
                }
            } else if gdal {
                // GDAL keeps the nodes that the file has (2 or more), and closes an area again when its first node is not
                // in the file
                let area = closed && crate::gdal::is_area(&tags);
                if area && !first_node && !pts.is_empty() {
                    pts.push(pts[0]);
                }
                if pts.len() >= 2 {
                    batch.push((id, tags, pts, owned, touches, area));
                }
            } else if pts.len() == refs as usize {
                // a way with a node that is not in the file is not a feature, and not a member of a relation
                batch.push((id, tags, pts, owned, touches, false));
            }
            if batch.len() >= 50_000 {
                give(std::mem::take(&mut batch))?;
            }
        }
        give(batch)
    };
    {
        let mut use_ways = |batch: WayBatch| -> Result<(), String> {
            for (id, tags, pts, owned, _, area) in &batch {
                if member_ids.binary_search(id).is_ok() {
                    let flags = if *owned { OWNED } else { 0 } | if *area && !o2p { AREA } else { 0 };
                    let donor = if *area && outer_ids.binary_search(id).is_ok() { crate::gdal::way_tags(tags) } else { Vec::new() };
                    member_lines.push(&KeyedLine, &(*id, flags, pts.iter().map(|&p| to_grid(p)).collect(), donor)).map_err(io)?;
                }
            }
            // the member ways with no tags are not features
            let mut made: Vec<Feature> = batch
                .par_iter()
                // quack --gdal: only the ways that the tile owns, also in a tile alone (the polygons that wait lose the mark)
                .filter(|w| !w.1.is_empty() && if mark && !gdal { w.4 } else { w.3 })
                .filter_map(|(id, tags, pts, owned, _, area)| {
                    let f = match mode {
                        Mode::Osm2pgsql => crate::osm2pgsql::way(*id, tags, pts, *area),
                        Mode::Gdal | Mode::Geofabrik => gdal_way(*id, tags, pts, *area),
                        Mode::Quack => way_feature(*id, tags, pts),
                    };
                    f.map(|mut f| {
                        f.owner = *owned;
                        f
                    })
                })
                .collect();
            if gdal && !outer_ids.is_empty() {
                // the polygon of an outer member of a multipolygon waits for the multipolygons
                let (wait, now): (Vec<Feature>, Vec<Feature>) = made.into_iter().partition(|f| {
                    f.layer == crate::gdal::MULTIPOLYGONS && outer_ids.binary_search(&feature_number(&f.feature_id)).is_ok()
                });
                for f in wait {
                    deferred.push(&Numbered, &(feature_number(&f.feature_id) as u64, f)).map_err(io)?;
                }
                made = now;
            }
            counts.ways += made.len();
            emit(made)
        };
        // natively, a reader thread reads the batches while this thread makes the features. The browser has no such
        // thread (and a file in OPFS belongs to the thread that opened it), so there this thread does both.
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::scope(|scope| -> Result<(), String> {
            let (tx, rx) = std::sync::mpsc::sync_channel::<Result<WayBatch, String>>(4);
            scope.spawn(move || {
                let mut send = |b: WayBatch| tx.send(Ok(b)).map_err(|_| "quack: pass 3 stopped".to_string());
                if let Err(e) = read_ways(&mut send) {
                    let _ = tx.send(Err(e));
                }
            });
            for batch in rx {
                use_ways(batch?)?;
            }
            Ok(())
        })?;
        #[cfg(target_arch = "wasm32")]
        read_ways(&mut use_ways)?;
    }
    if let Some(f) = fixes {
        let made = f.new_rows(owner, mode);
        counts.ways += made.len();
        emit(made)?;
    }
    drop((member_ids, outer_ids));
    crate::util::step("quack 3: the ways");
    // pass 4: the relations. The members and the lines, each sorted by way, give the lines of each relation; sorted by
    // relation and place, they meet the relation headers. quack --gdal: the member nodes and their points too.
    let mut rel_lines = Writer::new(store).map_err(io)?;
    for (members, lines) in [(rel_members, member_lines), (node_members, node_lines)] {
        let members = sort_bytes(members.finish().map_err(io)?, crate::spill::join::node_id, budget, store).map_err(io)?;
        let lines = sort_bytes(lines.finish().map_err(io)?, line_key, budget, store).map_err(io)?;
        let mut lines = lines.read(&KeyedLine).map_err(io)?.peekable();
        for m in members.read(&Request).map_err(io)? {
            let (w, r, place) = m.map_err(io)?;
            while lines.next_if(|l| l.as_ref().is_ok_and(|l| l.0 < w)).is_some() {}
            match lines.peek() {
                Some(Ok(l)) if l.0 == w => {
                    rel_lines.push(&KeyedLine, &(((r as i64) << 32) | place as i64, l.1, l.2.clone(), l.3.clone())).map_err(io)?;
                }
                Some(Err(_)) => return Err(io(lines.next().expect("peeked").expect_err("an error"))),
                _ => {}
            }
        }
    }
    let rel_lines = sort_bytes(rel_lines.finish().map_err(io)?, line_key, budget, store).map_err(io)?;
    crate::util::step("quack 4a: the lines of the relations");
    // each relation with the lines of its members that the file has: (id, tags, members, (place, flags, points, tags))
    type Rel = (i64, RawTags, Vec<Member>, Vec<(usize, u8, Vec<Pt>, RawTags)>);
    type Rels<'a> = dyn Iterator<Item = Result<Rel, String>> + Send + 'a;
    let with_rels = move |use_rels: &mut dyn FnMut(&mut Rels) -> Result<(), String>| -> Result<(), String> {
        let mut lines = rel_lines.read(&KeyedLine).map_err(io)?.peekable();
        let headers = rel_headers.finish().map_err(io)?;
        let mut rels = headers.read(&RelHeader).map_err(io)?.enumerate().map(|(r, h)| -> Result<Rel, String> {
            let (id, tags, members) = h.map_err(io)?;
            let mut got = Vec::new();
            while let Some(l) = lines.next_if(|l| l.as_ref().is_ok_and(|l| (l.0 >> 32) as usize == r)) {
                let (key, flags, pts, donor) = l.map_err(io)?;
                got.push(((key & 0xffff_ffff) as usize, flags, pts.iter().map(from_grid).collect(), donor));
            }
            if let Some(Err(_)) = lines.peek() {
                return Err(io(lines.next().expect("peeked").expect_err("an error")));
            }
            Ok((id, tags, members, got))
        });
        use_rels(&mut rels)
    };
    // by tiles, the first tile with the relation whole makes it
    let mine = |id: i64| owner.and_then(|o| o.relations).is_none_or(|r| r.lock().expect("the relations").insert(id));
    // quack --gdal: the outer member ways of the complete multipolygons (GDAL does not give their polygons)
    let used = std::sync::Mutex::new(Vec::<i64>::new());
    let make = |(id, tags, members, got): Rel| -> Vec<Feature> {
        #[cfg(not(target_arch = "wasm32"))]
        let t = std::time::Instant::now();
        let made = if o2p {
            osm2pgsql_relation(id, &tags, &members, got, owner.is_some(), &mine)
        } else if gdal {
            gdal_relation(id, tags, &members, got, owner.is_some(), mode == Mode::Gdal, &mine, &used)
        } else {
            // a relation with a member way that is not a valid way is not a feature
            let owned = got.first().is_some_and(|l| l.1 & OWNED != 0);
            let m: Vec<(&str, Vec<Pt>)> = got.into_iter().map(|(place, _, pts, _)| (members[place].2.as_str(), pts)).collect();
            let complete = m.len() == members.len();
            let geometry = if complete && mine(id) { relation_geometry(&m) } else { None };
            let tags = clean(&tags);
            (geometry.is_some() && !tags.is_empty())
                .then(|| Feature { owner: owned, ..Feature::new(format!("relation/{id}"), tags, &geometry.unwrap()) })
                .into_iter()
                .collect()
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let secs = t.elapsed().as_secs_f64();
            if secs > 2.0 {
                eprintln!("quack: relation {id} ({} members) took {secs:.1} s", members.len());
            }
        }
        made
    };
    // natively, a reader thread streams the relations into all threads (a slow relation delays only itself), and this
    // thread writes each part of rows that the threads make
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::scope(|scope| -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<Feature>, String>>(REL_CHANNEL.load(std::sync::atomic::Ordering::Relaxed));
        let make = &make;
        scope.spawn(move || {
            let read = with_rels(&mut |rels| {
                rels.par_bridge().try_for_each_with(tx.clone(), |tx, rel| {
                    let made = rel.map(make);
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
            });
            if let Err(e) = read {
                let _ = tx.send(Err(e));
            }
        });
        let mut part = Vec::new();
        for f in rx {
            part.extend(f?);
            if part.len() >= 10_000 {
                counts.relations += part.len();
                emit(std::mem::take(&mut part))?;
            }
        }
        counts.relations += part.len();
        emit(part)
    })?;
    // the browser: the relations are read on this thread (the files in OPFS), then made on all threads, a batch of at
    // most REL_BATCH points at a time (all relations at once: Île-de-France stopped at 4 GiB)
    #[cfg(target_arch = "wasm32")]
    with_rels(&mut |rels| {
        const REL_BATCH: usize = 4 << 20;
        let (mut batch, mut points, mut more): (Vec<Rel>, usize, bool) = (Vec::new(), 0, true);
        while more {
            match rels.next() {
                Some(r) => {
                    let rel = r?;
                    points += rel.3.iter().map(|m| m.2.len()).sum::<usize>();
                    batch.push(rel);
                }
                None => more = false,
            }
            if points >= REL_BATCH || (!more && !batch.is_empty()) {
                let made: Vec<Feature> = std::mem::take(&mut batch).into_par_iter().flat_map_iter(make).collect();
                points = 0;
                counts.relations += made.len();
                emit(made)?;
            }
        }
        Ok(())
    })?;
    if gdal {
        // the polygons of the outer members that no complete multipolygon used
        let mut used = used.into_inner().expect("the used ways");
        used.sort_unstable();
        let mut part = Vec::new();
        for r in deferred.finish().map_err(io)?.read(&Numbered).map_err(io)? {
            let (way, mut f) = r.map_err(io)?;
            if mode == Mode::Geofabrik || used.binary_search(&(way as i64)).is_err() {
                f.layer = crate::gdal::MULTIPOLYGONS;
                part.push(f);
            }
        }
        counts.ways += part.len();
        emit(part)?;
    }
    crate::util::step("quack 4b: the relations");
    Ok(counts)
}

/// The rows of the relations that wait for the thread that writes them (pass 4). A test makes it small, so the channel is
/// full and the threads that make the relations wait (`duckosm-build/tests/quack_gdal_full.rs`).
pub static REL_CHANNEL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(4096);

/// The tiles that [`features_by_tiles`] and the tile tables make at the same time natively.
// ponytail: a fixed number, and a group waits for its slowest tile; a pool of tiles when the time matters more
pub const TILES_AT_ONCE: usize = 4;

/// The browser: a tile whose file has at most this many bytes keeps its temporary files in memory (a few times the size
/// of the file), not in OPFS.
#[cfg(target_arch = "wasm32")]
const MEMORY_TILE: u64 = 64 << 20;

/// The boundary of a cut of a whole file: the world, a little more, so each node is inside.
pub const WORLD: &str = r#"{"type":"Polygon","coordinates":[[[-181,-91],[181,-91],[181,91],[-181,91],[-181,-91]]]}"#;

/// The most nodes in a tile of [`features_by_tiles`] (a default for the callers).
pub const TILE_NODES: usize = 4_000_000;

/// The tiles of [`features_by_tiles`].
#[derive(Debug, Clone, Copy)]
pub enum TileSize {
    /// A quadtree: tiles of mixed zooms with at most this many nodes each (a tile of zoom 14 can have more).
    Nodes(usize),
    /// The tiles of one zoom (at most 14) that have a node.
    Zoom(u8),
}

/// The features table of [`features`], made tile by tile, so that the memory follows the size of a tile, not of the area
/// (docs/rust/quack.md, "By tiles"): the browser makes the table of a large area in its 4 GiB.
///
/// 1. A pass over the nodes counts the nodes of each tile of zoom 14. With [`TileSize::Nodes`], a tile with more than that
///    many nodes is cut into its four tiles of the next zoom (a quadtree), so the dense city has small tiles and the
///    country around it large ones. With [`TileSize::Zoom`], the tiles are the tiles of that zoom with a node.
/// 2. The tile cut (`crate::tilecut::cut_to_store`) writes the file of each tile to the temporary store, with the ways
///    whole, and each multipolygon and boundary with all its members in each tile with a member.
/// 3. Each tile gives the rows that it owns ([`Owner`]) to `emit`, so each row comes one time.
///
/// `open` opens the `.osm.pbf` file (sorted by type and id); it is read four times, and the cut reads some blocks again.
pub fn features_by_tiles<R: Read + std::io::Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    budget: usize,
    size: TileSize,
    fixes: Option<QuackFixes>,
    mut emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    features_of_tiles(open, budget, size, Mode::Quack, fixes, |_, part| emit(part))
}

/// [`gdal_features`] made tile by tile, as [`features_by_tiles`] makes the table of quack. A relation of
/// `multilinestrings` or `other_relations` comes in parts (`Feature::part`): each tile gives its members, and
/// `crate::gdal::Parts` joins them.
pub fn gdal_features_by_tiles<R: Read + std::io::Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    budget: usize,
    size: TileSize,
    mode: Mode,
    fixes: Option<QuackFixes>,
    mut emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    features_of_tiles(open, budget, size, mode, fixes, |_, part| emit(part))
}

/// [`features_by_tiles`], and `emit` gets the tile that made each part too (the tiles one after the other: a part of a
/// new tile means that the tile before is done).
fn features_of_tiles<R: Read + std::io::Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    budget: usize,
    size: TileSize,
    mode: Mode,
    mut fixes: Option<QuackFixes>,
    mut emit: impl FnMut(Tile, Vec<Feature>) -> Result<(), String>,
) -> Result<Counts, String> {
    crate::util::raise_open_files();
    let tiles = tiles_of_file(&open, size)?;
    if let Some(f) = fixes.as_mut() {
        f.place(&tiles);
    }
    let fixes = fixes.as_ref();
    crate::util::step(&format!(
        "quack by tiles 1: {} tiles of zoom {} to {}",
        tiles.len(),
        tiles.iter().map(|t| t.z).min().unwrap_or(0),
        tiles.iter().map(|t| t.z).max().unwrap_or(0)
    ));
    let world = crate::boundary::Boundary::parse(WORLD)?;
    let files = crate::tilecut::cut_to_store(&open, &world, &tiles, &["multipolygon", "boundary"])?;
    crate::util::step("quack by tiles 2: the tile cut");
    let relations = std::sync::Mutex::new(crate::util::FxHashSet::default());
    let mut counts = Counts::default();
    let todo: Vec<(usize, Tile, crate::cutstream::CutCounts)> =
        files.tiles.iter().enumerate().filter(|t| t.1 .1.nodes > 0).map(|(i, (t, c))| (i, *t, *c)).collect();
    let n = todo.len();
    let run = |&(i, tile, _): &(usize, Tile, crate::cutstream::CutCounts),
               give: &mut dyn FnMut(Vec<Feature>) -> Result<(), String>|
     -> Result<Counts, String> {
        let owner = Owner { tile, relations: Some(&relations) };
        let open_tile = || Ok(std::io::BufReader::with_capacity(1 << 20, files.reader(i)));
        run(open_tile, budget, Some(&owner), mode, fixes, give)
    };
    let mut done = |k: usize, (_, tile, c): &(usize, Tile, crate::cutstream::CutCounts), got: Counts| {
        counts.nodes += got.nodes;
        counts.ways += got.ways;
        counts.relations += got.relations;
        crate::util::step(&format!(
            "quack by tiles 3: tile {k} of {n} ({}, {} nodes, {} ways, {} relations in the file of the tile)",
            tile.name(),
            c.nodes,
            c.ways,
            c.relations
        ));
    };
    // natively, TILES_AT_ONCE tiles at the same time (one small tile does not use all threads: France used 4 of 12
    // cores), and their rows go to `emit` in the order of the tiles. Each tile has a thread of its own, not a thread of
    // the pool: a tile waits for rows that the pool makes, so tiles on the pool with 4 threads or fewer left no thread to
    // make them (the test stopped on the CI machine).
    // quack --gdal: the order of the rows does not matter, so each tile sends its parts at once, and a pool of TILES_AT_ONCE
    // threads takes the next tile. A group that waits for its last tile kept the rows of four tiles: France had 8.8 GB of
    // rows in memory at zoom 8.
    #[cfg(not(target_arch = "wasm32"))]
    if mode != Mode::Quack {
        enum Msg {
            Part(Tile, Vec<Feature>),
            Done(usize, Counts),
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Msg, String>>(TILES_AT_ONCE);
        std::thread::scope(|scope| -> Result<(), String> {
            for _ in 0..TILES_AT_ONCE {
                let (tx, run, next, todo) = (tx.clone(), &run, &next, &todo);
                scope.spawn(move || loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(t) = todo.get(k) else { break };
                    let stopped = || "quack: the rows of the tiles stopped".to_string();
                    let got = run(t, &mut |p| tx.send(Ok(Msg::Part(t.1, p))).map_err(|_| stopped()));
                    let sent = match got {
                        Ok(got) => tx.send(Ok(Msg::Done(k, got))),
                        Err(e) => tx.send(Err(e)),
                    };
                    if sent.is_err() {
                        break;
                    }
                });
            }
            drop(tx);
            let mut finished = 0;
            for m in rx {
                match m? {
                    Msg::Part(t, p) => emit(t, p)?,
                    Msg::Done(k, got) => {
                        finished += 1;
                        done(finished, &todo[k], got);
                    }
                }
            }
            Ok(())
        })?;
        return Ok(counts);
    }
    #[cfg(not(target_arch = "wasm32"))]
    for (g, group) in todo.chunks(TILES_AT_ONCE).enumerate() {
        let run = &run;
        let made: Vec<Result<(Vec<Vec<Feature>>, Counts), String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = group
                .iter()
                .map(|t| {
                    scope.spawn(move || {
                        let mut parts = Vec::new();
                        let got = run(t, &mut |p| {
                            parts.push(p);
                            Ok(())
                        })?;
                        Ok((parts, got))
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("a tile stopped (panic)".into()))).collect()
        });
        for (j, (t, m)) in group.iter().zip(made).enumerate() {
            let (parts, got) = m?;
            for p in parts {
                emit(t.1, p)?;
            }
            done(g * TILES_AT_ONCE + j + 1, t, got);
        }
    }
    // the browser: this thread reads the files of TILES_AT_ONCE small tiles from OPFS into memory, and the tiles run at
    // the same time on the threads of the pool, with their temporary files in memory (the store of a thread of the pool:
    // `spill::MemStore`); their rows go to `emit` in the order of the tiles. A large tile runs alone on this thread,
    // with its temporary files in OPFS. (One tile at a time with OPFS: about 11 s for each tile of Sweden on a Windows PC.)
    #[cfg(target_arch = "wasm32")]
    {
        let mut k = 0;
        while k < todo.len() {
            let small = |t: &(usize, Tile, crate::cutstream::CutCounts)| files.size(t.0) <= MEMORY_TILE;
            if !small(&todo[k]) {
                let got = run(&todo[k], &mut |p| emit(todo[k].1, p))?;
                k += 1;
                done(k, &todo[k - 1], got);
                continue;
            }
            let group: Vec<_> = todo[k..].iter().take(TILES_AT_ONCE).take_while(|t| small(t)).collect();
            let mut bytes = Vec::with_capacity(group.len());
            for t in &group {
                let mut b = Vec::with_capacity(files.size(t.0) as usize);
                files.reader(t.0).read_to_end(&mut b).map_err(|e| format!("quack: the file of a tile: {e}"))?;
                bytes.push(b);
            }
            let made: Vec<Result<(Vec<Vec<Feature>>, Counts), String>> = group
                .par_iter()
                .zip(&bytes)
                .map(|(&&(_, tile, _), b)| {
                    crate::util::quiet(|| {
                        let owner = Owner { tile, relations: Some(&relations) };
                        let mut parts = Vec::new();
                        let got = features_owned(|| Ok(std::io::Cursor::new(&b[..])), budget, Some(&owner), fixes, |p| {
                            parts.push(p);
                            Ok(())
                        })?;
                        Ok((parts, got))
                    })
                })
                .collect();
            drop(bytes);
            for (t, m) in group.into_iter().zip(made) {
                let (parts, got) = m?;
                for p in parts {
                    emit(t.1, p)?;
                }
                k += 1;
                done(k, t, got);
            }
        }
    }
    Ok(counts)
}

/// The tiles of [`features_by_tiles`] with a node: the quadtree of [`TileSize::Nodes`], or the tiles of one zoom.
pub fn tiles_of_file<R: Read + Send>(open: &impl Fn() -> Result<R, String>, size: TileSize) -> Result<Vec<Tile>, String> {
    const Z: u8 = 14;
    #[derive(Default)]
    struct Count(crate::util::FxHashMap<(u32, u32), u32>, bool);
    impl RawSink for Count {
        fn node<'a>(&mut self, _: i64, lat: f64, lon: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
            let t = Tile::of(Z, lon, lat);
            *self.0.entry((t.x, t.y)).or_default() += 1;
            self.1 = true;
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
    let mut all: crate::util::FxHashMap<(u32, u32), u32> = Default::default();
    let mut seen = false;
    crate::cutstream::each_block(
        &mut open()?,
        |b, buf| {
            let mut c = Count::default();
            crate::pbfraw::decode_block_tags(b, [true, false, false], [false; 3], buf, &mut c)?;
            Ok(c)
        },
        |c, _| {
            if !c.1 {
                // the nodes of a sorted file come first: a block with no nodes after them ends the pass
                return Ok(!seen);
            }
            seen = true;
            for (k, v) in c.0 {
                *all.entry(k).or_default() += v;
            }
            Ok(true)
        },
    )?;
    let at = |z: u8| -> crate::util::FxHashMap<(u32, u32), usize> {
        let mut m: crate::util::FxHashMap<(u32, u32), usize> = Default::default();
        for (&(x, y), &n) in &all {
            *m.entry((x >> (Z - z), y >> (Z - z))).or_default() += n as usize;
        }
        m
    };
    // a quadtree: a tile with more than `most` nodes (or of a zoom lower than the zoom) is split in four, down to zoom 14
    let leaf = |t: &Tile, n: usize| match size {
        TileSize::Nodes(most) => n <= most.max(1),
        TileSize::Zoom(z) => t.z >= z,
    };
    let maps: Vec<crate::util::FxHashMap<(u32, u32), usize>> = (0..=Z).map(at).collect();
    let mut tiles = Vec::new();
    let mut todo: Vec<Tile> = maps[0].keys().map(|&(x, y)| Tile { z: 0, x, y }).collect();
    while let Some(t) = todo.pop() {
        if t.z == Z || leaf(&t, maps[t.z as usize][&(t.x, t.y)]) {
            tiles.push(t);
            continue;
        }
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (x, y) = (2 * t.x + dx, 2 * t.y + dy);
            if maps[t.z as usize + 1].contains_key(&(x, y)) {
                todo.push(Tile { z: t.z + 1, x, y });
            }
        }
    }
    tiles.sort();
    Ok(tiles)
}

/// The geometry of a row from its WKB (the WKB of [`Geom::wkb`]: little endian, points, lines, polygons and
/// multipolygons).
pub fn read_wkb(b: &[u8]) -> Option<geo::Geometry> {
    fn u32_at(b: &[u8], at: &mut usize) -> Option<u32> {
        let v = u32::from_le_bytes(b.get(*at..*at + 4)?.try_into().ok()?);
        *at += 4;
        Some(v)
    }
    fn coord(b: &[u8], at: &mut usize) -> Option<geo::Coord> {
        let f = |i: usize| b.get(i..i + 8).map(|s| f64::from_le_bytes(s.try_into().unwrap()));
        let c = geo::coord! { x: f(*at)?, y: f(*at + 8)? };
        *at += 16;
        Some(c)
    }
    fn line(b: &[u8], at: &mut usize) -> Option<LineString> {
        let n = u32_at(b, at)?;
        (0..n).map(|_| coord(b, at)).collect::<Option<Vec<_>>>().map(LineString::new)
    }
    fn polygon(b: &[u8], at: &mut usize) -> Option<Polygon> {
        *at += 5;
        let mut rings = (0..u32_at(b, at)?).map(|_| line(b, at)).collect::<Option<Vec<_>>>()?;
        let exterior = if rings.is_empty() { LineString::new(Vec::new()) } else { rings.remove(0) };
        Some(Polygon::new(exterior, rings))
    }
    let mut at = 5;
    match u32::from_le_bytes(b.get(1..5)?.try_into().ok()?) {
        1 => coord(b, &mut at).map(|c| geo::Geometry::Point(c.into())),
        2 => line(b, &mut at).map(geo::Geometry::LineString),
        3 => polygon(b, &mut { 0 }).map(geo::Geometry::Polygon),
        6 => {
            let n = u32_at(b, &mut at)?;
            (0..n).map(|_| polygon(b, &mut at)).collect::<Option<Vec<_>>>().map(|p| geo::Geometry::MultiPolygon(MultiPolygon(p)))
        }
        _ => None,
    }
}

/// [`Feature::center`] of a row from its WKB (the same numbers: the middle of the box of its points). It reads the
/// points in place, with no geometry in memory (the merge of the sorted parts gets the center of each row).
pub fn wkb_center(b: &[u8]) -> Option<Pt> {
    struct Bx(f64, f64, f64, f64);
    fn u32_at(b: &[u8], at: &mut usize) -> Option<u32> {
        let v = u32::from_le_bytes(b.get(*at..*at + 4)?.try_into().ok()?);
        *at += 4;
        Some(v)
    }
    fn points(b: &[u8], at: &mut usize, n: u32, bx: &mut Bx) -> Option<()> {
        let end = *at + 16 * n as usize;
        for c in b.get(*at..end)?.chunks_exact(16) {
            let (x, y) = (f64::from_le_bytes(c[..8].try_into().unwrap()), f64::from_le_bytes(c[8..].try_into().unwrap()));
            *bx = Bx(bx.0.min(x), bx.1.min(y), bx.2.max(x), bx.3.max(y));
        }
        *at = end;
        Some(())
    }
    fn polygon(b: &[u8], at: &mut usize, bx: &mut Bx) -> Option<()> {
        *at += 5;
        for _ in 0..u32_at(b, at)? {
            let n = u32_at(b, at)?;
            points(b, at, n, bx)?;
        }
        Some(())
    }
    let mut bx = Bx(f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut at = 5;
    match u32::from_le_bytes(b.get(1..5)?.try_into().ok()?) {
        1 => points(b, &mut at, 1, &mut bx)?,
        2 => {
            let n = u32_at(b, &mut at)?;
            points(b, &mut at, n, &mut bx)?
        }
        3 => polygon(b, &mut 0, &mut bx)?,
        6 => {
            for _ in 0..u32_at(b, &mut at)? {
                polygon(b, &mut at, &mut bx)?;
            }
        }
        _ => return None,
    }
    Some(((bx.0 + bx.2) / 2.0, (bx.1 + bx.3) / 2.0))
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
pub fn ordered(x: f64) -> i64 {
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
        put_sorted_row(i, f.center, &f.feature_id, f.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())), &f.geometry, o);
    }
}

/// Write a row for [`StrSort::merged`] into `o` from its parts (no [`Feature`] in memory): its number in the merge, its
/// center, id, tags and WKB.
pub fn put_sorted_row<'a>(
    i: u64,
    center: Pt,
    id: &str,
    tags: impl ExactSizeIterator<Item = (&'a str, &'a str)>,
    geometry: &[u8],
    o: &mut Vec<u8>,
) {
    let s = |o: &mut Vec<u8>, b: &[u8]| {
        o.extend_from_slice(&(b.len() as u32).to_le_bytes());
        o.extend_from_slice(b);
    };
    o.extend_from_slice(&i.to_le_bytes());
    o.extend_from_slice(&center.0.to_le_bytes());
    o.extend_from_slice(&center.1.to_le_bytes());
    s(o, id.as_bytes());
    o.extend_from_slice(&(tags.len() as u32).to_le_bytes());
    for (k, v) in tags {
        s(o, k.as_bytes());
        s(o, v.as_bytes());
    }
    s(o, geometry);
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
        (i, Feature { feature_id, tags, geometry, center, owner: true, ..Default::default() })
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

/// The rows of the runs of [`StrSort`] in the order (x, y, row number): the runs merged, one block of each run in memory.
struct ByX {
    cursors: Vec<Cursor>,
    heap: std::collections::BinaryHeap<std::cmp::Reverse<((i64, i64, u64), usize)>>,
    tmp: Vec<u8>,
}

impl ByX {
    fn new(
        runs: Vec<Vec<(u64, usize, usize)>>,
        file: &mut dyn crate::spill::Blob,
    ) -> std::io::Result<ByX> {
        let mut x = ByX {
            cursors: Vec::with_capacity(runs.len()),
            heap: Default::default(),
            tmp: Vec::new(),
        };
        for run in runs {
            let mut c = Cursor {
                blocks: run.into(),
                buf: Vec::new(),
                pos: 0,
                row: (0, 0),
            };
            if c.advance(file, &mut x.tmp)? {
                x.heap.push(std::cmp::Reverse((
                    numbered_key(c.bytes()),
                    x.cursors.len(),
                )));
            }
            x.cursors.push(c);
        }
        Ok(x)
    }

    /// The bytes of the next row into `row`; false at the end.
    fn next(
        &mut self,
        file: &mut dyn crate::spill::Blob,
        row: &mut Vec<u8>,
    ) -> std::io::Result<bool> {
        let Some(std::cmp::Reverse((_, i))) = self.heap.pop() else {
            return Ok(false);
        };
        row.clear();
        row.extend_from_slice(self.cursors[i].bytes());
        if self.cursors[i].advance(file, &mut self.tmp)? {
            self.heap.push(std::cmp::Reverse((
                numbered_key(self.cursors[i].bytes()),
                i,
            )));
        }
        Ok(true)
    }
}

/// Sort `rows` by their key (the first three values) and write them, with their bytes in `bytes`, as a run of blocks at
/// `end` of `file` (compressed on all threads, 64 blocks at a time, so only 64 blocks wait in memory; the browser does not
/// compress them). Gives the blocks of the run: place, compressed length, length.
fn write_run(file: &mut dyn crate::spill::Blob, end: &mut u64, rows: &mut [Placed], bytes: &[u8]) -> std::io::Result<Vec<(u64, usize, usize)>> {
    use std::io::{SeekFrom, Write};
    rows.par_sort_unstable_by_key(|r| (r.0, r.1, r.2));
    file.seek(SeekFrom::Start(*end))?;
    let mut run = Vec::new();
    let mut w = std::io::BufWriter::with_capacity(1 << 20, file);
    for group in rows.chunks(64 * BLOCK_ROWS) {
        let blocks: Vec<(Vec<u8>, usize)> = group
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
        for (b, raw_len) in blocks {
            w.write_all(&b)?;
            run.push((*end, b.len(), raw_len));
            *end += b.len() as u64;
        }
    }
    w.flush()?;
    Ok(run)
}

/// The key of a row of [`Numbered`] in a strip: (y, up in the even strips, down in the odd strips; x; row number).
fn strip_key(b: &[u8], up: bool) -> (i64, i64, u64) {
    let (x, _, n) = numbered_key(b);
    let y = f64::from_le_bytes(b[16..24].try_into().unwrap());
    (ordered(if up { y } else { -y }), x, n)
}

/// A row in the bytes of [`StrSort`]: its sort key (two `ordered` values and the row number) and the place of its bytes.
type Placed = (i64, i64, u64, usize, usize);

/// QuackOSM's sort ([`str_order`]) with the rows on disk, for areas whose rows do not fit in memory (France: 99 million
/// rows). [`StrSort::push`] writes the rows as bytes into one buffer until it holds `budget` bytes, then sorts them by
/// (x, y, row number) and writes them as a run of blocks (compressed on all threads) at the end of one temporary file.
/// [`StrSort::finish`] merges the runs, so the rows come in the order of x, into the bytes of a strip; each time a strip
/// is full, it sorts the strip by y and gives its rows, a row group at a time. The order is the one of [`str_order`].
/// A strip with more than `budget` bytes goes to the end of the file in sorted pieces of `budget` bytes, and the pieces are
/// merged by y. The disk holds the rows at most two times, compressed. The memory is `budget` while the rows come, then
/// `budget` for the strip, one block of each run and of each piece, and one row group.
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

/// The sort of the rows of one tile of quack by tiles into the order (x, y, row number) of [`StrSort`] (a sorted part
/// for [`StrSort::merged`]). While the rows fit in the budget of [`StrSort`], they wait as they are and are sorted in
/// memory; more rows go to a [`StrSort`] (runs on disk).
pub struct PartSort {
    held: Vec<Feature>,
    bytes: usize,
    spill: Option<StrSort>,
    budget: usize,
}

impl Default for PartSort {
    fn default() -> Self {
        PartSort { held: Vec::new(), bytes: 0, spill: None, budget: sort_budget() }
    }
}

impl PartSort {
    pub fn push(&mut self, rows: Vec<Feature>) -> Result<(), String> {
        if let Some(s) = self.spill.as_mut() {
            return s.push(rows);
        }
        self.bytes += rows.iter().map(|f| 64 + f.feature_id.len() + f.geometry.len() + f.tags.iter().map(|(k, v)| 48 + k.len() + v.len()).sum::<usize>()).sum::<usize>();
        self.held.extend(rows);
        if self.bytes > self.budget {
            let mut s = StrSort::new(self.budget, ROW_GROUP).map_err(|e| format!("quack sort: {e}"))?;
            s.push(std::mem::take(&mut self.held))?;
            self.spill = Some(s);
        }
        Ok(())
    }

    /// Give the rows in the order (x, y, row number), in pieces of `group` rows.
    pub fn finish(mut self, group: usize, mut emit: impl FnMut(Vec<Feature>) -> Result<(), String>) -> Result<(), String> {
        if let Some(s) = self.spill {
            return s.sorted_by_x(group, emit);
        }
        // a stable sort: the order before the sort (the row number) breaks the ties
        self.held.par_sort_by_key(|f| (ordered(f.center.0), ordered(f.center.1)));
        let mut rows = self.held.into_iter();
        loop {
            let piece: Vec<Feature> = rows.by_ref().take(group).collect();
            if piece.is_empty() {
                return Ok(());
            }
            emit(piece)?;
        }
    }
}

/// The memory of the sort of [`StrSort`]: in the browser a quarter of [`BUDGET`], because the memory of WebAssembly never
/// shrinks (Kentucky sorted: 3.3 GiB with 512 MB, the limit is 4 GiB).
fn sort_budget() -> usize {
    if cfg!(target_arch = "wasm32") {
        BUDGET / 4
    } else {
        BUDGET
    }
}

impl Default for StrSort {
    fn default() -> Self {
        StrSort::new(sort_budget(), ROW_GROUP).expect("quack sort: a temporary file")
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
            // a run before the buffer passes the budget: so the buffer does not grow to the next size (twice the budget)
            if !self.rows.is_empty() && self.bytes.len() + b.len() > self.budget {
                self.flush().map_err(|e| format!("quack sort: {e}"))?;
            }
            let base = self.bytes.len();
            self.bytes.extend_from_slice(&b);
            self.rows.extend(placed.into_iter().map(|(x, y, n, start, len)| (x, y, n, base + start, len)));
        }
        Ok(())
    }

    /// Write the rows of the buffer as a new run, sorted.
    fn flush(&mut self) -> std::io::Result<()> {
        let run = write_run(&mut *self.file, &mut self.end, &mut self.rows, &self.bytes)?;
        self.runs.push(run);
        self.written += self.rows.len() as u64;
        self.rows.clear();
        self.bytes.clear();
        Ok(())
    }

    /// The rows of a strip from its sorted pieces in the file (`up`: the order of y), merged, a row group at a time.
    fn merge_strip(
        &mut self,
        pieces: Vec<Vec<(u64, usize, usize)>>,
        up: bool,
        out: &mut Vec<Feature>,
        emit: &mut impl FnMut(Vec<Feature>) -> Result<(), String>,
    ) -> std::io::Result<()> {
        use std::cmp::Reverse;
        let mut tmp = Vec::new();
        let mut cursors: Vec<Cursor> = Vec::with_capacity(pieces.len());
        let mut heap = std::collections::BinaryHeap::new();
        for run in pieces {
            let mut c = Cursor { blocks: run.into(), buf: Vec::new(), pos: 0, row: (0, 0) };
            if c.advance(&mut *self.file, &mut tmp)? {
                heap.push(Reverse((strip_key(c.bytes(), up), cursors.len())));
            }
            cursors.push(c);
        }
        // the bytes of the rows of a row group, read on all threads
        let (mut b, mut spans): (Vec<u8>, Vec<(usize, usize)>) = (Vec::new(), Vec::new());
        loop {
            let next = heap.pop();
            if let Some(Reverse((_, i))) = next {
                spans.push((b.len(), cursors[i].bytes().len()));
                b.extend_from_slice(cursors[i].bytes());
                if cursors[i].advance(&mut *self.file, &mut tmp)? {
                    heap.push(Reverse((strip_key(cursors[i].bytes(), up), i)));
                }
            }
            if out.len() + spans.len() == self.group || (next.is_none() && !spans.is_empty()) {
                out.extend(spans.par_iter().map(|&(at, len)| Numbered.get(&mut &b[at..at + len]).1).collect::<Vec<_>>());
                b.clear();
                spans.clear();
                if out.len() == self.group {
                    emit(std::mem::replace(out, Vec::with_capacity(self.group))).map_err(std::io::Error::other)?;
                }
            }
            if next.is_none() {
                return Ok(());
            }
        }
    }

    /// Give the rows in the order of the sort, in pieces of a row group.
    pub fn finish(
        mut self,
        emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
    ) -> Result<(), String> {
        let io = |e: std::io::Error| format!("quack sort: {e}");
        let n = (self.written + self.rows.len() as u64) as usize;
        if !self.rows.is_empty() {
            self.flush().map_err(io)?;
        }
        let mut src = ByX::new(std::mem::take(&mut self.runs), &mut *self.file).map_err(io)?;
        self.strips(n, |file, row| src.next(file, row), emit)
    }

    /// Give the rows in the order (x, y, row number), in pieces of `group` rows: the order of a sorted part of quack by
    /// tiles, which [`StrSort::merged`] merges. The rows of one run are sorted in memory.
    pub fn sorted_by_x(
        mut self,
        group: usize,
        mut emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
    ) -> Result<(), String> {
        let io = |e: std::io::Error| format!("quack sort: {e}");
        if self.runs.is_empty() {
            self.rows.par_sort_unstable_by_key(|r| (r.0, r.1, r.2));
            for rows in self.rows.chunks(group) {
                emit(
                    rows.par_iter()
                        .map(|r| Numbered.get(&mut &self.bytes[r.3..r.3 + r.4]).1)
                        .collect(),
                )?;
            }
            return Ok(());
        }
        if !self.rows.is_empty() {
            self.flush().map_err(io)?;
        }
        let mut src = ByX::new(std::mem::take(&mut self.runs), &mut *self.file).map_err(io)?;
        let mut row = Vec::new();
        // the bytes of `group` rows, then the rows read on all threads
        let (mut b, mut spans): (Vec<u8>, Vec<(usize, usize)>) = (Vec::new(), Vec::with_capacity(group));
        loop {
            let more = src.next(&mut *self.file, &mut row).map_err(io)?;
            if more {
                spans.push((b.len(), row.len()));
                b.extend_from_slice(&row);
            }
            if spans.len() == group || (!more && !spans.is_empty()) {
                emit(spans.par_iter().map(|&(at, len)| Numbered.get(&mut &b[at..at + len]).1).collect())?;
                b.clear();
                spans.clear();
            }
            if !more {
                break;
            }
        }
        Ok(())
    }

    /// QuackOSM's sort ([`str_order`]) of `n` rows that `next` writes ([`put_sorted_row`], with the number it gets) in
    /// the order (x, y, row number), for example the sorted parts of quack by tiles merged: only the strips, with no runs.
    /// `next` gives false at the end. The memory is the one of [`StrSort::finish`].
    pub fn merged(
        n: usize,
        next: impl FnMut(u64, &mut Vec<u8>) -> Result<bool, String>,
        emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
    ) -> Result<(), String> {
        StrSort::default().merged_rows(n, next, emit)
    }

    fn merged_rows(
        self,
        n: usize,
        mut next: impl FnMut(u64, &mut Vec<u8>) -> Result<bool, String>,
        emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
    ) -> Result<(), String> {
        // the number of the row in the merge: in the order of x and y, the same order as the row numbers of the parts
        let mut seq = 0u64;
        self.strips(
            n,
            |_, row| {
                row.clear();
                let more = next(seq, row).map_err(std::io::Error::other)?;
                seq += 1;
                Ok(more)
            },
            emit,
        )
    }

    /// The strips of [`str_order`] from `n` rows that `next` writes (as bytes of [`Numbered`]) in the order (x, y, row
    /// number): each strip is sorted by y and given, a row group at a time.
    fn strips(
        mut self,
        n: usize,
        mut next: impl FnMut(&mut dyn crate::spill::Blob, &mut Vec<u8>) -> std::io::Result<bool>,
        mut emit: impl FnMut(Vec<Feature>) -> Result<(), String>,
    ) -> Result<(), String> {
        let io = |e: std::io::Error| format!("quack sort: {e}");
        let group = self.group;
        let (_, strip_size) = strips(n.max(1), group);
        // the pieces of a strip go after the runs; the pieces of the next strip go over them, so the file holds the runs
        // and the pieces of one strip
        let base = self.end;
        // the buffer of the rows is the buffer of the strips now
        let (mut bytes, mut strip) = (
            std::mem::take(&mut self.bytes),
            std::mem::take(&mut self.rows),
        );
        bytes.clear();
        strip.clear();
        let mut row = Vec::new();
        let mut out: Vec<Feature> = Vec::with_capacity(group);
        let mut up = true;
        // a strip larger than the budget goes to the end of the file in sorted pieces, merged when the strip is full
        let (mut in_strip, mut pieces) = (0usize, Vec::new());
        loop {
            let more = next(&mut *self.file, &mut row).map_err(io)?;
            if more {
                // the strip: by y (up in the even strips, down in the odd strips), then x, then the row number
                let (x, _, n) = numbered_key(&row);
                let y = f64::from_le_bytes(row[16..24].try_into().unwrap());
                strip.push((
                    ordered(if up { y } else { -y }),
                    x,
                    n,
                    bytes.len(),
                    row.len(),
                ));
                bytes.extend_from_slice(&row);
                in_strip += 1;
            }
            let full = in_strip == strip_size || (!more && in_strip > 0);
            if bytes.len() > self.budget || (full && !pieces.is_empty()) {
                pieces.push(
                    write_run(&mut *self.file, &mut self.end, &mut strip, &bytes).map_err(io)?,
                );
                strip.clear();
                bytes.clear();
            }
            if full && !pieces.is_empty() {
                self.merge_strip(std::mem::take(&mut pieces), up, &mut out, &mut emit)
                    .map_err(io)?;
                self.end = base;
            } else if full {
                strip.par_sort_unstable_by_key(|r| (r.0, r.1, r.2));
                // the rows of the strip are read on all threads, a row group at a time
                let mut i = 0;
                while i < strip.len() {
                    let take = (group - out.len()).min(strip.len() - i);
                    let rows: Vec<Feature> = strip[i..i + take]
                        .par_iter()
                        .map(|r| Numbered.get(&mut &bytes[r.3..r.3 + r.4]).1)
                        .collect();
                    out.extend(rows);
                    i += take;
                    if out.len() == group {
                        emit(std::mem::replace(&mut out, Vec::with_capacity(group)))?;
                    }
                }
                strip.clear();
                bytes.clear();
            }
            if full {
                in_strip = 0;
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
            .map(|(i, &c)| Feature { feature_id: format!("node/{i}"), tags: vec![("k".into(), i.to_string())], geometry: vec![i as u8], center: c, owner: true, ..Default::default() })
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
    fn sorted_parts_merged_give_the_order_of_str_order() {
        // the flow of quack by tiles with the sort: each part sorted by x alone (some with runs on disk), the parts merged
        // by (x, y, part), then the strips. Repeated centers in several parts test the ties.
        let pts: Vec<Pt> = (0..5000u64)
            .map(|i| match i % 97 {
                0 => (f64::NAN, 1.0),
                _ => (
                    ((i * 7919) % 613) as f64 / 37.0 - 8.0,
                    ((i * 104729) % 211) as f64 / 13.0 % 5.0,
                ),
            })
            .collect();
        let feature = |i: usize| Feature {
            feature_id: format!("node/{i}"),
            tags: vec![],
            geometry: vec![1],
            center: pts[i],
            owner: true,
            ..Default::default()
        };
        let mut parts: Vec<std::collections::VecDeque<Feature>> = Vec::new();
        let mut at = 0;
        // the parts larger than 1000 rows pass the budget (in two pushes) and go to a StrSort with runs on disk
        for size in [700, 1, 2299, 2000] {
            let mut s = PartSort { budget: if size > 1000 { 10_000 } else { 1 << 20 }, ..PartSort::default() };
            s.push((at..at + size / 2).map(feature).collect()).unwrap();
            s.push((at + size / 2..at + size).map(feature).collect()).unwrap();
            assert_eq!(s.spill.is_some(), size > 1000);
            let mut part = std::collections::VecDeque::new();
            s.finish(64, |g| {
                part.extend(g);
                Ok(())
            })
            .unwrap();
            parts.push(part);
            at += size;
        }
        let key = |f: &Feature| (ordered(f.center.0), ordered(f.center.1));
        let mut heap: std::collections::BinaryHeap<_> = parts
            .iter()
            .enumerate()
            .map(|(i, p)| std::cmp::Reverse((key(&p[0]), i)))
            .collect();
        let mut got = Vec::new();
        StrSort::new(10_000, 100)
            .unwrap()
            .merged_rows(
                5000,
                |seq, row| {
                    let Some(std::cmp::Reverse((_, i))) = heap.pop() else {
                        return Ok(false);
                    };
                    let f = parts[i].pop_front().unwrap();
                    if let Some(g) = parts[i].front() {
                        heap.push(std::cmp::Reverse((key(g), i)));
                    }
                    Numbered::put_row(seq, &f, row);
                    Ok(true)
                },
                |g| {
                    got.extend(g.into_iter().map(|f| f.feature_id));
                    Ok(())
                },
            )
            .unwrap();
        let want: Vec<String> = str_order_by(&pts, 100)
            .into_iter()
            .map(|i| format!("node/{i}"))
            .collect();
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
