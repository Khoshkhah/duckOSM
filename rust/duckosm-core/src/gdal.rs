//! duckosm quack --gdal: the rules of GDAL's OSM driver (GDAL 3.13, its default `osmconf.ini`), for the quack engine
//! ([`crate::quack`], docs/rust/quack.md, "GDAL layers"). The engine reads the file as quack does; these rules decide what
//! each element becomes, and the writers make GDAL's five layers from the rows.
//!
//! The rules are the ones of GDAL's `ogrosmdatasource.cpp` and `ogrosmlayer.cpp`:
//!
//! - `points`: a node with a tag that is not `created_by`, `converted_by`, `source`, `time`, `ele` or `attribution`;
//! - `lines`: a way with 2 or more nodes and a tag, that is not an area. Its line has the nodes that the file has (2 or
//!   more);
//! - `multipolygons`: a closed way (the first node is the last node) that is an area ([`is_area`]) and has a tag that GDAL
//!   keeps, except an outer member of a complete multipolygon or boundary; and a relation of the type `multipolygon` or
//!   `boundary` with all its member ways (not the role `subarea`): the closed member ways are rings, the other member ways
//!   are joined into rings ([`build_rings`]), and [`organize`] puts the holes in their polygons. A relation with no tag
//!   other than `type` and `created_by` takes the tags of its first outer member area;
//! - `multilinestrings`: a relation of the type `multilinestring` or `route`: its member ways;
//! - `other_relations`: each other relation: its member nodes as points and its member ways as lines (an area: a polygon).
//!
//! Each tag that is a column of the layer goes into its column; the other tags go into `other_tags` as GDAL writes them
//! (`"key"=>"value"`, with `\` before `"` and `\`), except the tags that the layer ignores.

use crate::quack::{Feature, Pt};
use crate::raw::RawTags;

/// The layers, in GDAL's order. `Feature::layer` holds the number (1 to 5); 0 is the table of quack.
pub const POINTS: u8 = 1;
pub const LINES: u8 = 2;
pub const MULTILINESTRINGS: u8 = 3;
pub const MULTIPOLYGONS: u8 = 4;
pub const OTHER_RELATIONS: u8 = 5;

/// The name of a layer.
pub fn layer_name(layer: u8) -> &'static str {
    ["", "points", "lines", "multilinestrings", "multipolygons", "other_relations"][layer as usize]
}

/// `closed_ways_are_polygons` of osmconf.ini: a key, or a key and a value.
const CLOSED_WAYS_ARE_POLYGONS: &[&str] = &[
    "aeroway", "amenity", "boundary", "building", "craft", "geological", "historic", "landuse", "leisure", "military",
    "natural", "office", "place", "shop", "sport", "tourism", "highway=platform", "public_transport=platform",
];

/// The keys that do not make a node a point (`unsignificant` of `[points]`).
const INSIGNIFICANT: &[&str] = &["created_by", "converted_by", "source", "time", "ele", "attribution"];

/// The tags that the layers do not write (`ignore` of each layer). A key with a `:` is ignored when the part up to the
/// `:` is in the list.
const IGNORE: &[&str] = &["created_by", "converted_by", "source", "time", "ele", "note", "todo", "openGeoDB:", "fixme", "FIXME"];
const IGNORE_AREA: &[&str] = &["area", "created_by", "converted_by", "source", "time", "ele", "note", "todo", "openGeoDB:", "fixme", "FIXME"];

/// The tags that GDAL does not keep with an area way (`m_ignoredKeys`), and the most tags it keeps.
const WAY_IGNORED: &[&str] = &["area", "created_by", "converted_by", "note", "todo", "fixme", "FIXME"];
const WAY_TAGS: usize = 255;

/// The columns of each layer after `osm_id` (and `osm_way_id`), before `z_order` and `other_tags` (`attributes`).
pub fn attributes(layer: u8) -> &'static [&'static str] {
    match layer {
        POINTS => &["name", "barrier", "highway", "ref", "address", "is_in", "place", "man_made"],
        LINES => &["name", "highway", "waterway", "aerialway", "barrier", "man_made", "railway"],
        MULTIPOLYGONS => &[
            "name", "type", "aeroway", "amenity", "admin_level", "barrier", "boundary", "building", "craft", "geological",
            "historic", "land_area", "landuse", "leisure", "man_made", "military", "natural", "office", "place", "shop",
            "sport", "tourism",
        ],
        _ => &["name", "type"],
    }
}

/// All columns of a layer, in GDAL's order, with their type (true: Integer, false: String). The geometry is not here.
pub fn columns(layer: u8) -> Vec<(&'static str, bool)> {
    let mut c = vec![("osm_id", false)];
    if layer == MULTIPOLYGONS {
        c.push(("osm_way_id", false));
    }
    c.extend(attributes(layer).iter().map(|a| (*a, false)));
    if layer == LINES {
        c.push(("z_order", true));
    }
    c.push(("other_tags", false));
    c
}

/// A value of a column.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Int(i64),
}

/// `IsClosedWayTaggedAsPolygon`: `area=yes` or `area=no` decides; else a key or a key and a value of
/// [`CLOSED_WAYS_ARE_POLYGONS`].
pub fn is_area(tags: &RawTags) -> bool {
    let mut area = false;
    for (k, v) in tags {
        if k == "area" {
            if v == "yes" {
                return true;
            }
            if v == "no" {
                return false;
            }
        }
        if area {
            continue;
        }
        if CLOSED_WAYS_ARE_POLYGONS.iter().any(|p| p == k)
            || CLOSED_WAYS_ARE_POLYGONS.iter().any(|p| p.len() == k.len() + 1 + v.len() && p.starts_with(k.as_str()) && p.as_bytes()[k.len()] == b'=' && p.ends_with(v.as_str()))
        {
            area = true;
        }
    }
    area
}

/// A node with these tags is a point.
pub fn is_point(tags: &RawTags) -> bool {
    tags.iter().any(|(k, _)| !INSIGNIFICANT.contains(&k.as_str()))
}

/// The tags that GDAL keeps with an area way: the polygon of the way and the tags that a relation with no tags of its
/// own takes.
pub fn way_tags(tags: &RawTags) -> RawTags {
    tags.iter().filter(|(k, _)| !WAY_IGNORED.contains(&k.as_str())).take(WAY_TAGS).cloned().collect()
}

/// The relation has a tag other than `type` and `created_by`.
pub fn has_own_tags(tags: &RawTags) -> bool {
    tags.iter().any(|(k, _)| k != "type" && k != "created_by")
}

/// The layer of a relation.
pub fn relation_layer(tags: &RawTags) -> u8 {
    let mut layer = OTHER_RELATIONS;
    for (k, v) in tags {
        if k == "type" {
            layer = match v.as_str() {
                "multipolygon" | "boundary" => MULTIPOLYGONS,
                "multilinestring" | "route" => MULTILINESTRINGS,
                _ => OTHER_RELATIONS,
            };
        }
    }
    layer
}

fn other_tag(layer: u8, k: &str) -> bool {
    let ignore = if matches!(layer, POINTS | LINES) { IGNORE } else { IGNORE_AREA };
    if ignore.contains(&k) {
        return false;
    }
    match k.find(':') {
        Some(i) => !ignore.contains(&&k[..=i]),
        None => true,
    }
}

fn hstore(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

/// C `atoi`: the integer at the start of the text (0 when there is none).
fn atoi(s: &str) -> i64 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits.bytes().take_while(u8::is_ascii_digit).fold(0i64, |n, d| n.wrapping_mul(10).wrapping_add((d - b'0') as i64)) as i32 as i64;
    if neg {
        -n
    } else {
        n
    }
}

/// The values of the columns of a row ([`columns`] of its layer), as `SetFieldsFromTags` sets them.
pub fn values(f: &Feature) -> Vec<Option<Value>> {
    let cols = columns(f.layer);
    let mut out: Vec<Option<Value>> = vec![None; cols.len()];
    let (kind, id) = f.feature_id.split_once('/').expect("a feature id");
    // a way of the multipolygons has its id in osm_way_id
    let id_col = if kind == "way" && f.layer == MULTIPOLYGONS { 1 } else { 0 };
    out[id_col] = Some(Value::Text(id.to_string()));
    let mut other = String::new();
    for (k, v) in &f.tags {
        if let Some(i) = cols.iter().position(|c| c.0 == k) {
            if i != 0 {
                out[i] = Some(if cols[i].1 { Value::Int(atoi(v)) } else { Value::Text(v.clone()) });
                continue;
            }
        }
        if other_tag(f.layer, k) {
            if !other.is_empty() {
                other.push(',');
            }
            hstore(k, &mut other);
            other.push_str("=>");
            hstore(v, &mut other);
        }
    }
    if !other.is_empty() {
        out[cols.len() - 1] = Some(Value::Text(other));
    }
    if f.layer == LINES {
        // the z_order of osmconf.ini (GDAL's code of it): highway and railway from their columns, the others from the tags
        let text = |c: &str| match cols.iter().position(|x| x.0 == c).and_then(|i| out[i].as_ref()) {
            Some(Value::Text(s)) => Some(s.as_str()),
            _ => None,
        };
        let first = |k: &str| f.tags.iter().find(|t| t.0 == k).map(|t| t.1.as_str());
        let mut z = match text("highway") {
            Some("minor" | "road" | "unclassified" | "residential") => 3,
            Some("tertiary_link" | "tertiary") => 4,
            Some("secondary_link" | "secondary") => 6,
            Some("primary_link" | "primary") => 7,
            Some("trunk_link" | "trunk") => 8,
            Some("motorway_link" | "motorway") => 9,
            _ => 0,
        };
        let yes = |v: Option<&str>| matches!(v, Some("yes" | "true" | "1"));
        if yes(first("bridge")) {
            z += 10;
        }
        if yes(first("tunnel")) {
            z -= 10;
        }
        if text("railway").is_some() {
            z += 5;
        }
        if let Some(l) = first("layer") {
            z += 10 * atoi(l);
        }
        let i = cols.iter().position(|c| c.0 == "z_order").expect("z_order");
        out[i] = Some(Value::Int(z as i32 as i64));
    }
    out
}

// ---- geometries: WKB, little endian

fn wkb_head(out: &mut Vec<u8>, kind: u32) {
    out.push(1);
    out.extend_from_slice(&kind.to_le_bytes());
}

fn wkb_pts(out: &mut Vec<u8>, pts: &[Pt]) {
    out.extend_from_slice(&(pts.len() as u32).to_le_bytes());
    for p in pts {
        out.extend_from_slice(&p.0.to_le_bytes());
        out.extend_from_slice(&p.1.to_le_bytes());
    }
}

pub fn wkb_point(p: Pt) -> Vec<u8> {
    let mut o = Vec::with_capacity(21);
    wkb_head(&mut o, 1);
    o.extend_from_slice(&p.0.to_le_bytes());
    o.extend_from_slice(&p.1.to_le_bytes());
    o
}

pub fn wkb_line(pts: &[Pt]) -> Vec<u8> {
    let mut o = Vec::with_capacity(9 + 16 * pts.len());
    wkb_head(&mut o, 2);
    wkb_pts(&mut o, pts);
    o
}

fn wkb_polygon(out: &mut Vec<u8>, rings: &[Vec<Pt>]) {
    wkb_head(out, 3);
    out.extend_from_slice(&(rings.len() as u32).to_le_bytes());
    rings.iter().for_each(|r| wkb_pts(out, r));
}

pub fn wkb_multipolygon(polys: &[Vec<Vec<Pt>>]) -> Vec<u8> {
    let mut o = Vec::new();
    wkb_head(&mut o, 6);
    o.extend_from_slice(&(polys.len() as u32).to_le_bytes());
    polys.iter().for_each(|p| wkb_polygon(&mut o, p));
    o
}

/// A part of a relation of `multilinestrings` or `other_relations`: a member node, a member way, or a member area.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Point(Pt),
    Line(Vec<Pt>),
    Area(Vec<Pt>),
}

impl Part {
    pub fn wkb(&self) -> Vec<u8> {
        match self {
            Part::Point(p) => wkb_point(*p),
            Part::Line(l) => wkb_line(l),
            Part::Area(r) => {
                let mut o = Vec::new();
                wkb_polygon(&mut o, std::slice::from_ref(r));
                o
            }
        }
    }
}

/// The geometry of a relation of `multilinestrings` (a MULTILINESTRING) or `other_relations` (a GEOMETRYCOLLECTION) from the
/// WKB of its parts, in the order of the members.
pub fn wkb_collection(layer: u8, parts: &[&[u8]]) -> Vec<u8> {
    let mut o = Vec::new();
    wkb_head(&mut o, if layer == MULTILINESTRINGS { 5 } else { 7 });
    o.extend_from_slice(&(parts.len() as u32).to_le_bytes());
    parts.iter().for_each(|p| o.extend_from_slice(p));
    o
}

// ---- the parts of the relations of quack --gdal by tiles

/// The rows of `multilinestrings` and `other_relations` that come in parts (`Feature::part`, quack --gdal by tiles): the
/// parts wait in a temporary file; [`Parts::rows`] joins the parts of each relation, in the order of its members, into
/// one row.
pub struct Parts {
    store: Box<dyn crate::spill::Store>,
    w: crate::spill::Writer,
}

/// A part in the temporary file: the relation, the place of the member, the layer, the tags and the WKB of the member.
struct PartCodec;

impl crate::spill::Codec<(i64, u32, u8, RawTags, Vec<u8>)> for PartCodec {
    fn put(&self, r: &(i64, u32, u8, RawTags, Vec<u8>), o: &mut Vec<u8>) {
        o.extend_from_slice(&r.0.to_le_bytes());
        o.extend_from_slice(&r.1.to_le_bytes());
        o.push(r.2);
        let s = |o: &mut Vec<u8>, b: &[u8]| {
            o.extend_from_slice(&(b.len() as u32).to_le_bytes());
            o.extend_from_slice(b);
        };
        o.extend_from_slice(&(r.3.len() as u32).to_le_bytes());
        for (k, v) in &r.3 {
            s(o, k.as_bytes());
            s(o, v.as_bytes());
        }
        s(o, &r.4);
    }
    fn get(&self, b: &mut &[u8]) -> (i64, u32, u8, RawTags, Vec<u8>) {
        fn take<'a>(b: &mut &'a [u8], n: usize) -> &'a [u8] {
            let (x, rest) = b.split_at(n);
            *b = rest;
            x
        }
        let u32_ = |b: &mut &[u8]| u32::from_le_bytes(take(b, 4).try_into().expect("4 bytes"));
        let bytes = |b: &mut &[u8]| {
            let n = u32_(b) as usize;
            take(b, n).to_vec()
        };
        let text = |b: &mut &[u8]| String::from_utf8_lossy(&bytes(b)).into_owned();
        let id = i64::from_le_bytes(take(b, 8).try_into().expect("8 bytes"));
        let place = u32_(b);
        let layer = take(b, 1)[0];
        let tags = (0..u32_(b)).map(|_| (text(b), text(b))).collect();
        (id, place, layer, tags, bytes(b))
    }
}

/// A row of quack --gdal as bytes, for the files of the tiles of the browser ([`get_row`] reads it): the layer, the place
/// of a part (u32::MAX: a whole row), the feature id, the tags and the WKB.
pub fn put_row(f: &Feature, o: &mut Vec<u8>) {
    o.push(f.layer);
    o.extend_from_slice(&f.part.unwrap_or(u32::MAX).to_le_bytes());
    let s = |o: &mut Vec<u8>, b: &[u8]| {
        o.extend_from_slice(&(b.len() as u32).to_le_bytes());
        o.extend_from_slice(b);
    };
    s(o, f.feature_id.as_bytes());
    o.extend_from_slice(&(f.tags.len() as u32).to_le_bytes());
    for (k, v) in &f.tags {
        s(o, k.as_bytes());
        s(o, v.as_bytes());
    }
    s(o, &f.geometry);
}

/// The row of [`put_row`].
pub fn get_row(b: &mut &[u8]) -> Feature {
    fn take<'a>(b: &mut &'a [u8], n: usize) -> &'a [u8] {
        let (x, rest) = b.split_at(n);
        *b = rest;
        x
    }
    let u32_ = |b: &mut &[u8]| u32::from_le_bytes(take(b, 4).try_into().expect("4 bytes"));
    let bytes = |b: &mut &[u8]| {
        let n = u32_(b) as usize;
        take(b, n).to_vec()
    };
    let text = |b: &mut &[u8]| String::from_utf8_lossy(&bytes(b)).into_owned();
    let layer = take(b, 1)[0];
    let part = Some(u32_(b)).filter(|p| *p != u32::MAX);
    let feature_id = text(b);
    let tags = (0..u32_(b)).map(|_| (text(b), text(b))).collect();
    let geometry = bytes(b);
    let center = crate::quack::wkb_center(&geometry).unwrap_or_default();
    Feature { feature_id, tags, geometry, center, owner: true, layer, part }
}

impl Parts {
    pub fn new() -> Result<Parts, String> {
        let store = crate::spill::temp_store();
        let w = crate::spill::Writer::new(&*store).map_err(|e| format!("quack --gdal: a temporary file: {e}"))?;
        Ok(Parts { store, w })
    }

    /// Keep a part.
    pub fn push(&mut self, f: &Feature) -> Result<(), String> {
        let id = f.feature_id.split_once('/').and_then(|(_, n)| n.parse::<i64>().ok()).ok_or("quack --gdal: a part with no relation id")?;
        let row = (id, f.part.unwrap_or(0), f.layer, f.tags.clone(), f.geometry.clone());
        self.w.push(&PartCodec, &row).map_err(|e| format!("quack --gdal: a temporary file: {e}"))
    }

    /// The rows of the relations, one relation at a time, to `give`.
    pub fn rows(self, budget: usize, mut give: impl FnMut(Feature) -> Result<(), String>) -> Result<(), String> {
        let io = |e: std::io::Error| format!("quack --gdal: a temporary file: {e}");
        let key = |b: &[u8]| (i64::from_le_bytes(b[..8].try_into().expect("8 bytes")), u32::from_le_bytes(b[8..12].try_into().expect("4 bytes")));
        let rows = crate::spill::sort_bytes(self.w.finish().map_err(io)?, key, budget, &*self.store).map_err(io)?;
        let mut group: Vec<(i64, u32, u8, RawTags, Vec<u8>)> = Vec::new();
        let mut flush = |group: &mut Vec<(i64, u32, u8, RawTags, Vec<u8>)>| -> Result<(), String> {
            if group.is_empty() {
                return Ok(());
            }
            let first = &group[0];
            let g = wkb_collection(first.2, &group.iter().map(|p| p.4.as_slice()).collect::<Vec<_>>());
            let center = crate::quack::wkb_center(&first.4).unwrap_or_default();
            let f = Feature { feature_id: format!("relation/{}", first.0), tags: first.3.clone(), geometry: g, center, owner: true, layer: first.2, part: None };
            group.clear();
            give(f)
        };
        for r in rows.read(&PartCodec).map_err(io)? {
            let r = r.map_err(io)?;
            if group.last().is_some_and(|g| g.0 != r.0) {
                flush(&mut group)?;
            }
            // a member that two tiles give (none: each tile gives the members that it owns) comes one time
            if !group.last().is_some_and(|g| g.1 == r.1) {
                group.push(r);
            }
        }
        flush(&mut group)
    }
}

// ---- the polygons of a multipolygon relation

/// `OGRBuildPolygonFromEdges` with no tolerance: each ring starts with the first line that is left and takes the first
/// line whose end meets its loose end, until it closes or no line meets it. A ring that does not close is in the result too.
pub fn build_rings(lines: Vec<Vec<Pt>>) -> Vec<Vec<Pt>> {
    let mut left: std::collections::VecDeque<Vec<Pt>> = lines.into_iter().filter(|l| l.len() >= 2).collect();
    let mut rings = Vec::new();
    while let Some(first) = left.pop_front() {
        let mut ring = first;
        let mut work = true;
        while ring.first() != ring.last() && !left.is_empty() && work {
            work = false;
            let end = *ring.last().expect("a point");
            let mut best: Option<(usize, bool)> = None;
            for (i, l) in left.iter().enumerate() {
                if l[0] == end {
                    best = Some((i, false));
                }
                if l[l.len() - 1] == end {
                    best = Some((i, true));
                }
                if best.is_some() {
                    break;
                }
            }
            if let Some((i, reverse)) = best {
                let l = left.remove(i).expect("a line");
                let it: Box<dyn Iterator<Item = &Pt>> = if reverse { Box::new(l.iter().rev()) } else { Box::new(l.iter()) };
                // the first point of the line is the loose end of the ring
                ring.extend(it.skip(1));
                work = true;
            }
        }
        rings.push(ring);
    }
    rings
}

/// `get_LinearArea` of GDAL.
fn area(r: &[Pt]) -> f64 {
    let n = r.len();
    if n < 2 {
        return 0.0;
    }
    let mut s = r[0].0 * (r[1].1 - r[n - 1].1);
    for i in 1..n - 1 {
        s += r[i].0 * (r[i + 1].1 - r[i - 1].1);
    }
    s += r[n - 1].0 * (r[0].1 - r[n - 2].1);
    0.5 * s.abs()
}

/// `OGRLinearRing::isPointInRing`.
fn in_ring(r: &[Pt], p: Pt) -> bool {
    if r.len() < 4 {
        return false;
    }
    let mut crossings = 0;
    let (mut px, mut py) = (r[0].0 - p.0, r[0].1 - p.1);
    for q in &r[1..] {
        let (x1, y1) = (q.0 - p.0, q.1 - p.1);
        let (x2, y2) = (px, py);
        if (y1 > 0.0 && y2 <= 0.0) || (y2 > 0.0 && y1 <= 0.0) {
            if 0.0 < (x1 * y2 - x2 * y1) / (y2 - y1) {
                crossings += 1;
            }
        }
        px = x1;
        py = y1;
    }
    crossings % 2 == 1
}

/// `OGRLinearRing::isPointOnRingBoundary`.
fn on_ring(r: &[Pt], p: Pt) -> bool {
    if r.len() < 4 {
        return false;
    }
    let (mut px, mut py) = (p.0 - r[0].0, p.1 - r[0].1);
    for i in 1..r.len() {
        let (dx1, dy1) = (p.0 - r[i].0, p.1 - r[i].1);
        let (dx2, dy2) = (px, py);
        if dx1 * dy2 - dx2 * dy1 == 0.0 && !(dx1 == dx2 && dy1 == dy2) {
            let (sx, sy) = (r[i].0 - r[i - 1].0, r[i].1 - r[i - 1].1);
            let cross = dx2 * sx + dy2 * sy;
            if cross >= 0.0 && cross <= sx * sx + sy * sy {
                return true;
            }
        }
        px = dx1;
        py = dy1;
    }
    false
}

#[derive(Clone, Copy)]
struct Env {
    min: Pt,
    max: Pt,
}

impl Env {
    fn of(r: &[Pt]) -> Env {
        r.iter().fold(Env { min: (f64::MAX, f64::MAX), max: (f64::MIN, f64::MIN) }, |e, p| Env {
            min: (e.min.0.min(p.0), e.min.1.min(p.1)),
            max: (e.max.0.max(p.0), e.max.1.max(p.1)),
        })
    }
    fn contains(&self, o: &Env) -> bool {
        self.min.0 <= o.min.0 && self.min.1 <= o.min.1 && self.max.0 >= o.max.0 && self.max.1 >= o.max.1
    }
}

/// `this` is inside `other` (the fast test of `organizePolygons`): the first point of `this` is in `other`, or, when it
/// is on the boundary of `other`, the next point of `this` that is not on the boundary (then the middle of a segment).
fn inside(this: &[Pt], other: &[Pt]) -> bool {
    let start = this[0];
    if !on_ring(other, start) {
        return in_ring(other, start);
    }
    let n = this.len();
    let mut prev = start;
    let mut k = 1;
    while k < n {
        let p = this[k];
        k += 1;
        if p == prev {
            continue;
        }
        if on_ring(other, p) {
        } else {
            return in_ring(other, p);
        }
        prev = p;
    }
    if n > 2 {
        let mut prev = this[0];
        for &p in &this[1..] {
            if p == prev {
                continue;
            }
            let mid = ((p.0 + prev.0) / 2.0, (p.1 + prev.1) / 2.0);
            if on_ring(other, mid) {
            } else {
                return in_ring(other, mid);
            }
            prev = p;
        }
    }
    false
}

/// `OGRGeometryFactory::organizePolygons` (METHOD=DEFAULT, the fast test) on rings: the polygons, each an outer ring and
/// its holes, in the order of the rings. A ring is a hole of the smallest larger ring that holds it, when that ring is not
/// a hole; else it is a polygon. With a ring of fewer than 4 points, each ring is a polygon.
pub fn organize(rings: Vec<Vec<Pt>>) -> Vec<Vec<Vec<Pt>>> {
    let n = rings.len();
    if n <= 1 {
        return rings.into_iter().map(|r| vec![r]).collect();
    }
    let mixed = rings.iter().any(|r| r.len() < 4);
    let areas: Vec<f64> = rings.iter().map(|r| area(r)).collect();
    let envs: Vec<Env> = rings.iter().map(|r| Env::of(r)).collect();
    let mut top = vec![true; n];
    let mut outer: Vec<Option<usize>> = vec![None; n];
    if !mixed {
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|a, b| areas[*b].partial_cmp(&areas[*a]).unwrap_or(std::cmp::Ordering::Equal));
        top = vec![false; n];
        top[order[0]] = true;
        for &i in &order[1..] {
            let e = envs[i];
            let mut cands: Vec<usize> = (0..n)
                .filter(|&j| !(envs[j].min.0 > e.max.0 || envs[j].max.0 < e.min.0 || envs[j].min.1 > e.max.1 || envs[j].max.1 < e.min.1))
                .collect();
            cands.sort_by(|a, b| areas[*a].partial_cmp(&areas[*b]).unwrap_or(std::cmp::Ordering::Equal));
            let mut found = false;
            for j in cands {
                if areas[j] < areas[i] || j == i {
                    continue;
                }
                if envs[j].contains(&e) && inside(&rings[i], &rings[j]) {
                    if top[j] {
                        top[i] = false;
                        outer[i] = Some(j);
                    } else {
                        top[i] = true;
                    }
                    found = true;
                    break;
                }
            }
            if !found {
                top[i] = true;
            }
        }
    }
    let mut polys: Vec<Option<Vec<Vec<Pt>>>> = rings.into_iter().map(|r| Some(vec![r])).collect();
    for i in 0..n {
        if let Some(j) = outer[i] {
            let hole = polys[i].take().expect("a ring").pop().expect("a ring");
            polys[j].as_mut().expect("an outer ring").push(hole);
        }
    }
    polys.into_iter().zip(top).filter(|(_, t)| *t).filter_map(|(p, _)| p).collect()
}

/// The polygons of a multipolygon relation from its member ways (not the role `subarea`), in the order of the members
/// (`BuildMultiPolygon`): each closed way is a ring; the other ways are joined into rings, and each closed ring of 4 or more
/// points of the result is a ring too. None: no ring.
pub fn multipolygon(members: Vec<Vec<Pt>>) -> Option<Vec<Vec<Vec<Pt>>>> {
    let (mut rings, mut lines) = (Vec::new(), Vec::new());
    for m in members {
        if !m.is_empty() && m.first() == m.last() {
            rings.push(m);
        } else {
            lines.push(m);
        }
    }
    if !lines.is_empty() {
        // GDAL organizes the joined rings first, then takes the rings of each polygon from the last one to the first
        for poly in organize(build_rings(lines)) {
            for r in poly.into_iter().rev() {
                if r.len() >= 4 && r.first() == r.last() {
                    rings.push(r);
                }
            }
        }
    }
    (!rings.is_empty()).then(|| organize(rings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(x: f64, y: f64, s: f64) -> Vec<Pt> {
        vec![(x, y), (x + s, y), (x + s, y + s), (x, y + s), (x, y)]
    }

    #[test]
    fn areas_holes_and_rings() {
        let t = |kv: &[(&str, &str)]| kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<RawTags>();
        assert!(is_area(&t(&[("building", "yes")])));
        assert!(is_area(&t(&[("highway", "platform")])));
        assert!(!is_area(&t(&[("highway", "footway")])));
        assert!(!is_area(&t(&[("building", "yes"), ("area", "no")])));
        assert!(is_area(&t(&[("highway", "footway"), ("area", "yes")])));
        // a hole in a polygon, and a polygon apart
        let p = organize(vec![sq(1.0, 1.0, 1.0), sq(0.0, 0.0, 4.0), sq(10.0, 10.0, 1.0)]);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], vec![sq(0.0, 0.0, 4.0), sq(1.0, 1.0, 1.0)]);
        // two halves join into one ring
        let r = build_rings(vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)], vec![(0.0, 0.0), (0.0, 1.0), (1.0, 1.0)]]);
        assert_eq!(r, vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)]]);
        let f = Feature { feature_id: "way/7".into(), tags: t(&[("highway", "primary"), ("bridge", "yes"), ("note", "x"), ("a\"b", "c")]), layer: LINES, ..Default::default() };
        let v = values(&f);
        assert_eq!(v[0], Some(Value::Text("7".into())));
        assert_eq!(v[8], Some(Value::Int(17)));
        assert_eq!(v[9], Some(Value::Text(r#""bridge"=>"yes","a\"b"=>"c""#.into())));
        assert_eq!(atoi(" -2;3"), -2);
    }
}
