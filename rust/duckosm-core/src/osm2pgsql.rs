//! duckosm quack --osm2pgsql (docs/concepts/osm2pgsql.md): the rules of osm2pgsql's `pgsql` output with its
//! `default.style` and `--hstore` (osm2pgsql 2.3.1: `output-pgsql.cpp`, `tagtransform-c.cpp`, `table.cpp`,
//! `geom-from-osm.cpp`, `geom-functions.cpp`), for the quack engine ([`crate::quack`]). The engine reads the file; these
//! rules decide what each element becomes. The tables:
//!
//! - `planet_osm_point`: a node with a tag that the style keeps;
//! - `planet_osm_line`: a way with a tag that the style keeps and 2 or more points, that is not a polygon; and the member
//!   ways of a relation `route` or `boundary` (or `multipolygon` with a tag `boundary`), joined (`line_merge`), with
//!   `osm_id` = minus the id of the relation;
//! - `planet_osm_polygon`: a closed way that is a polygon ([`filter_tags`]); and each polygon of a relation `multipolygon`
//!   or `boundary` (all member ways, roles not used), with `way_area`;
//! - `planet_osm_roads`: the rows of `planet_osm_line` that [`z_order`] marks (major roads, railways, administrative
//!   boundaries).
//!
//! A line is cut into parts of at most 100 km (EPSG:3857) or 1 degree (EPSG:4326). With EPSG:3857, a polygon that is not
//! valid is not a row (osm2pgsql's trigger `ST_IsValid`).
//!
//! The engine gives rows in EPSG:4326 (`Feature::layer`: [`POINT`], [`LINE`], [`POLYGON`]); [`finish`] makes the rows of
//! the four tables from them. A relation line is the member ways as one MULTILINESTRING (by tiles: one part for each
//! member, `Feature::part`), and [`finish`] joins them.

use crate::quack::{Feature, Pt};
use crate::raw::RawTags;

/// The tables. `Feature::layer` holds the number (1 to 4).
pub const POINT: u8 = 1;
pub const LINE: u8 = 2;
pub const POLYGON: u8 = 3;
pub const ROADS: u8 = 4;

/// The name of a table.
pub fn table_name(t: u8) -> &'static str {
    ["", "planet_osm_point", "planet_osm_line", "planet_osm_polygon", "planet_osm_roads"][t as usize]
}

/// The type of a column of the style: `text`, `int4` or `real`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Int,
    Real,
}

#[derive(Clone, Copy, PartialEq)]
enum Use {
    Column(Kind),
    NoColumn,
    Delete,
}

const N: u8 = 1;
const W: u8 = 2;
const NW: u8 = 3;
const T: Use = Use::Column(Kind::Text);
const D: Use = Use::Delete;

/// `default.style` of osm2pgsql, in its order: the key, node and/or way, the flag `polygon`, and the use.
#[rustfmt::skip]
const STYLE: &[(&str, u8, bool, Use)] = &[
    ("access", NW, false, T), ("addr:housename", NW, false, T), ("addr:housenumber", NW, false, T),
    ("addr:interpolation", NW, false, T), ("admin_level", NW, false, T), ("aerialway", NW, false, T),
    ("aeroway", NW, true, T), ("amenity", NW, true, T), ("area", NW, true, T), ("barrier", NW, false, T),
    ("bicycle", NW, false, T), ("brand", NW, false, T), ("bridge", NW, false, T), ("boundary", NW, false, T),
    ("building", NW, true, T), ("capital", N, false, T), ("construction", NW, false, T), ("covered", NW, false, T),
    ("culvert", NW, false, T), ("cutting", NW, false, T), ("denomination", NW, false, T), ("disused", NW, false, T),
    ("ele", N, false, T), ("embankment", NW, false, T), ("foot", NW, false, T), ("generator:source", NW, false, T),
    ("harbour", NW, true, T), ("highway", NW, false, T), ("historic", NW, true, T), ("horse", NW, false, T),
    ("intermittent", NW, false, T), ("junction", NW, false, T), ("landuse", NW, true, T), ("layer", NW, false, T),
    ("leisure", NW, true, T), ("lock", NW, false, T), ("man_made", NW, true, T), ("military", NW, true, T),
    ("motorcar", NW, false, T), ("name", NW, false, T), ("natural", NW, true, T), ("office", NW, true, T),
    ("oneway", NW, false, T), ("operator", NW, false, T), ("place", NW, true, T), ("population", NW, false, T),
    ("power", NW, true, T), ("power_source", NW, false, T), ("public_transport", NW, true, T), ("railway", NW, false, T),
    ("ref", NW, false, T), ("religion", NW, false, T), ("route", NW, false, T), ("service", NW, false, T),
    ("shop", NW, true, T), ("sport", NW, true, T), ("surface", NW, false, T), ("toll", NW, false, T),
    ("tourism", NW, true, T), ("tower:type", NW, false, T), ("tracktype", W, false, T), ("tunnel", NW, false, T),
    ("water", NW, true, T), ("waterway", NW, true, T), ("wetland", NW, true, T), ("width", NW, false, T),
    ("wood", NW, false, T), ("z_order", NW, false, Use::Column(Kind::Int)), ("way_area", W, false, Use::Column(Kind::Real)),
    ("abandoned:aeroway", W, true, Use::NoColumn), ("abandoned:amenity", W, true, Use::NoColumn),
    ("abandoned:building", W, true, Use::NoColumn), ("abandoned:landuse", W, true, Use::NoColumn),
    ("abandoned:power", W, true, Use::NoColumn), ("area:highway", W, true, Use::NoColumn),
    ("note", NW, false, D), ("note:*", NW, false, D), ("source", NW, false, D), ("source_ref", NW, false, D),
    ("source:*", NW, false, D), ("attribution", NW, false, D), ("comment", NW, false, D), ("fixme", NW, false, D),
    ("created_by", NW, false, D), ("odbl", NW, false, D), ("odbl:note", NW, false, D), ("SK53_bulk:load", NW, false, D),
    ("tiger:*", NW, false, D), ("NHD:*", NW, false, D), ("nhd:*", NW, false, D), ("gnis:*", NW, false, D),
    ("geobase:*", NW, false, D), ("accuracy:meters", NW, false, D), ("sub_sea:type", NW, false, D),
    ("waterway:type", NW, false, D), ("KSJ2:*", NW, false, D), ("yh:*", NW, false, D), ("osak:*", NW, false, D),
    ("kms:*", NW, false, D), ("ngbe:*", NW, false, D), ("naptan:*", NW, false, D), ("CLC:*", NW, false, D),
    ("3dshapes:ggmodelk", NW, false, D), ("AND_nosr_r", NW, false, D), ("import", NW, false, D), ("it:fvg:*", NW, false, D),
];

/// The columns of a table after `osm_id`, in osm2pgsql's order (the style's order), with their type. `tags` (hstore) and
/// `way` (the geometry) come after them.
pub fn columns(table: u8) -> Vec<(&'static str, Kind)> {
    let on = if table == POINT { N } else { W };
    STYLE
        .iter()
        .filter(|e| e.1 & on != 0)
        .filter_map(|e| match e.3 {
            Use::Column(k) => Some((e.0, k)),
            _ => None,
        })
        .collect()
}

/// `wild_match`: `*` is any text, `?` any one character.
fn wild(expr: &[u8], s: &[u8]) -> bool {
    match (expr.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => wild(&expr[1..], s) || (!s.is_empty() && wild(expr, &s[1..])),
        (Some(b'?'), Some(_)) => wild(&expr[1..], &s[1..]),
        (Some(a), Some(b)) if a == b => wild(&expr[1..], &s[1..]),
        _ => false,
    }
}

/// `check_key` with `--hstore`: None, the style deletes the tag; else the tag stays, with the flag `polygon` of its key.
fn check_key(k: &str, on: u8) -> Option<bool> {
    for &(name, types, polygon, u) in STYLE {
        if types & on == 0 {
            continue;
        }
        if u == Use::Delete {
            if wild(name.as_bytes(), k.as_bytes()) {
                return None;
            }
        } else if name == k {
            return Some(polygon);
        }
    }
    Some(false)
}

fn get<'a>(tags: &'a RawTags, k: &str) -> Option<&'a str> {
    tags.iter().find(|t| t.0 == k).map(|t| t.1.as_str())
}

/// `taglist_t::value_to_bool`.
fn value_to_bool(v: &str, default: bool) -> bool {
    if !default && matches!(v, "yes" | "true" | "1") {
        return true;
    }
    if default && matches!(v, "no" | "false" | "0") {
        return false;
    }
    default
}

#[derive(Clone, Copy, PartialEq)]
enum Obj {
    Node,
    Way,
    Relation,
}

/// `filter_tags`: the tags that the style keeps, and (a way) the way is a polygon. None: no tag stays (no row).
fn filter_tags(tags: &RawTags, obj: Obj) -> Option<(RawTags, bool)> {
    let on = if obj == Obj::Node { N } else { W };
    let (mut out, mut keep, mut flag, mut coast) = (RawTags::new(), false, false, false);
    for (k, v) in tags {
        if obj == Obj::Relation && k == "type" {
            out.push((k.clone(), v.clone()));
            continue;
        }
        // natural=coastline: the way is a polygon, and the tag goes
        if k == "natural" && v == "coastline" {
            coast = true;
            continue;
        }
        if let Some(p) = check_key(k, on) {
            keep = true;
            flag |= p;
            out.push((k.clone(), v.clone()));
        }
    }
    if !keep {
        return None;
    }
    let polygon = if obj != Obj::Way {
        false
    } else if coast {
        if get(&out, "area").is_none() {
            out.push(("area".into(), "yes".into()));
        }
        true
    } else {
        get(tags, "area").map_or(flag, |a| value_to_bool(a, flag))
    };
    Some((out, polygon))
}

/// C `strtol` (base 10) as an `int`.
fn strtol(s: &str) -> i32 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, d) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i64 = 0;
    for c in d.bytes().take_while(u8::is_ascii_digit) {
        n = n.saturating_mul(10).saturating_add((c - b'0') as i64);
    }
    (if neg { n.saturating_neg() } else { n }) as i32
}

/// `add_z_order`: the `z_order` of the tags, and the row goes to `planet_osm_roads` too.
pub fn z_order(tags: &RawTags) -> (i32, bool) {
    const LAYERS: &[(&str, i32, bool)] = &[
        ("proposed", 1, false), ("construction", 2, false), ("steps", 10, false), ("cycleway", 10, false),
        ("bridleway", 10, false), ("footway", 10, false), ("path", 10, false), ("track", 11, false),
        ("service", 15, false), ("tertiary_link", 24, false), ("secondary_link", 25, true), ("primary_link", 27, true),
        ("trunk_link", 28, true), ("motorway_link", 29, true), ("raceway", 30, false), ("pedestrian", 31, false),
        ("living_street", 32, false), ("road", 33, false), ("unclassified", 33, false), ("residential", 33, false),
        ("tertiary", 34, false), ("secondary", 36, true), ("primary", 37, true), ("trunk", 38, true),
        ("motorway", 39, true),
    ];
    let mut z = get(tags, "layer").map_or(0, strtol).wrapping_mul(100);
    let mut roads = false;
    if let Some(h) = get(tags, "highway") {
        if let Some(l) = LAYERS.iter().find(|l| l.0 == h) {
            z += l.1;
            roads = l.2;
        }
    }
    if get(tags, "railway").is_some_and(|r| !r.is_empty()) {
        z += 35;
        roads = true;
    }
    if get(tags, "boundary") == Some("administrative") {
        roads = true;
    }
    if get(tags, "bridge").is_some_and(|v| value_to_bool(v, false)) {
        z += 100;
    }
    if get(tags, "tunnel").is_some_and(|v| value_to_bool(v, false)) {
        z -= 100;
    }
    (z, roads)
}

fn row(feature_id: String, layer: u8, tags: RawTags, geometry: Vec<u8>, center: Pt) -> Feature {
    Feature { feature_id, tags, geometry, center, owner: true, layer, part: None }
}

/// The row of a node, or None (`node_add`).
pub fn point(id: i64, tags: &RawTags, p: Pt) -> Option<Feature> {
    let (tags, _) = filter_tags(tags, Obj::Node)?;
    Some(row(format!("node/{id}"), POINT, tags, crate::gdal::wkb_point(p), p))
}

/// The row of a way with its points (`pts`, one for each node of the way), or None (`way_add`). `closed`: the first node
/// is the last node.
pub fn way(id: i64, tags: &RawTags, pts: &[Pt], closed: bool) -> Option<Feature> {
    let (mut tags, polygon) = filter_tags(tags, Obj::Way)?;
    if pts.len() < 2 {
        return None;
    }
    tags.push(("z_order".into(), z_order(&tags).0.to_string()));
    if polygon && closed {
        // a closed way of fewer than 4 nodes is never a polygon; else the first outer ring that the assembler makes
        if pts.len() < 4 {
            return None;
        }
        let ring = assemble(std::slice::from_ref(&pts.to_vec()))?.into_iter().next()?.into_iter().next()?;
        return Some(row(format!("way/{id}"), POLYGON, tags, wkb_rings(3, &[ring]), pts[0]));
    }
    Some(row(format!("way/{id}"), LINE, tags, crate::gdal::wkb_line(pts), pts[0]))
}

/// What a relation makes (`relation_add` and `filter_rel_member_tags`): its tags, a line of the member ways, polygons.
pub struct Relation {
    pub tags: RawTags,
    pub lines: bool,
    pub polygons: bool,
}

/// The rule of a relation, or None (no row): a relation `route`, `multipolygon` or `boundary` with a tag that the style
/// keeps.
pub fn relation(tags: &RawTags) -> Option<Relation> {
    let kind = get(tags, "type").filter(|t| matches!(*t, "route" | "multipolygon" | "boundary"))?;
    let (pre, _) = filter_tags(tags, Obj::Relation)?;
    let route = kind == "route";
    let mut out = RawTags::new();
    let add = |out: &mut RawTags, k: &str, v: &str| {
        if get(out, k).is_none() {
            out.push((k.to_string(), v.to_string()));
        }
    };
    for (k, v) in &pre {
        if route && k == "name" {
            add(&mut out, "route_name", v);
        }
        if k != "type" {
            add(&mut out, k, v);
        }
    }
    if out.is_empty() {
        return None;
    }
    if route {
        let mut network = None;
        if let Some(n) = get(&pre, "network") {
            let state = match get(&pre, "state") {
                Some("alternate") => "alternate",
                Some("connection") => "connection",
                _ => "yes",
            };
            if matches!(n, "lcn" | "rcn" | "ncn" | "lwn" | "rwn" | "nwn") {
                add(&mut out, n, state);
                network = Some(n);
            }
        }
        let color = match get(&pre, "preferred_color") {
            Some(c @ ("0" | "1" | "2" | "3" | "4")) => c,
            _ => "0",
        };
        add(&mut out, "route_pref_color", color);
        if let (Some(r), Some(n)) = (get(&pre, "ref"), network) {
            add(&mut out, &format!("{n}_ref"), r);
        }
    }
    let boundary = kind == "boundary" || (kind == "multipolygon" && get(&out, "boundary").is_some());
    let multipolygon = kind == "multipolygon" && !boundary;
    out.push(("z_order".into(), z_order(&out).0.to_string()));
    Some(Relation { tags: out, lines: !multipolygon, polygons: boundary || multipolygon })
}

/// The rows of a relation (`rule`) from the lines of its member ways that the file has, in the order of the members:
/// the line of the members (a MULTILINESTRING that [`finish`] joins) and the polygons.
pub fn relation_rows(id: i64, rule: &Relation, lines: &[Vec<Pt>]) -> Vec<Feature> {
    let mut out = Vec::new();
    let Some(center) = lines.iter().find_map(|l| l.first().copied()) else {
        return out;
    };
    if rule.lines {
        out.push(relation_line(id, rule, lines.to_vec()));
    }
    if rule.polygons {
        for p in assemble(lines).unwrap_or_default() {
            out.push(row(format!("relation/{id}"), POLYGON, rule.tags.clone(), wkb_rings(3, &p), center));
        }
    }
    out
}

/// The line of the member ways of a relation: a MULTILINESTRING that [`finish`] joins.
fn relation_line(id: i64, rule: &Relation, lines: Vec<Vec<Pt>>) -> Feature {
    let center = lines.iter().find_map(|l| l.first().copied()).unwrap_or_default();
    row(format!("relation/{id}"), LINE, rule.tags.clone(), wkb_rings(5, &lines), center)
}

/// By tiles: one member way of the line of a relation, a LINESTRING at its place `place` in the members
/// (`gdal::Parts` joins the parts into one row).
pub fn relation_part(id: i64, rule: &Relation, place: u32, pts: &[Pt]) -> Feature {
    Feature { part: Some(place), ..row(format!("relation/{id}"), LINE, rule.tags.clone(), crate::gdal::wkb_line(pts), pts[0]) }
}

// ---- the rows of the tables

/// The `osm_id` of a row: the id of the node or the way, minus the id of a relation.
pub fn osm_id(f: &Feature) -> i64 {
    let (kind, id) = f.feature_id.split_once('/').expect("a feature id");
    let id: i64 = id.parse().expect("an id");
    if kind == "relation" {
        -id
    } else {
        id
    }
}

/// A coordinate as osmium has it (an integer of 1e-7 degrees).
fn fixed(v: f64) -> f64 {
    (v * 1e7).round() / 1e7
}

/// EPSG:3857 as osmium's `lonlat_to_mercator` (the fast form below 78 degrees), with the latitude cut at 89.99.
pub fn mercator(p: Pt) -> Pt {
    const R: f64 = 6378137.0;
    let (lon, lat) = (fixed(p.0), fixed(p.1).clamp(-89.99, 89.99));
    let y = if !(-78.0..=78.0).contains(&lat) {
        R * (std::f64::consts::PI / 4.0 + lat.to_radians() / 2.0).tan().ln()
    } else {
        // Horner's rule with fused multiply-adds, as clang compiles osmium's code (`-ffp-contract=on`)
        let horner = |c: &[f64]| c.iter().fold(0.0f64, |acc, &k| acc.mul_add(lat, k));
        #[rustfmt::skip]
        let num = horner(&[-3.1112583378460085319e-23, 2.0465852743943268009e-19, 6.4905282018672673884e-18,
            -1.9685447939983315591e-14, -2.2022588158115104182e-13, 5.1617537365509453239e-10, 2.5380136069803016519e-9,
            -5.1448323697228488745e-6, -9.4888671473357768301e-6, 1.7453292518154191887e-2]) * lat;
        #[rustfmt::skip]
        let den = horner(&[-1.9741136066814230637e-22, -1.258514031244679556e-20, 4.8141483273572351796e-17,
            8.6876090870176172185e-16, -2.3298743439377541768e-12, -1.9300094785736130185e-11, 4.3251609106864178231e-8,
            1.7301944508516974048e-7, -3.4554675198786337842e-4, -5.4367203601085991108e-4, 1.0]);
        R * num / den
    };
    (R * lon.to_radians(), y)
}

/// `fill_point_list`: no two equal points one after the other.
fn dedupe(pts: &[Pt]) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(pts.len());
    for &p in pts {
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

/// `split_linestring` of `segmentize`: the line in parts of at most `max` long.
fn split_line(line: &[Pt], max: f64, out: &mut Vec<Vec<Pt>>) {
    let dist_of = |a: Pt, b: Pt| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let mut dist = 0.0;
    let mut prev = line[0];
    let mut cur = vec![prev];
    for &p in &line[1..] {
        let delta = dist_of(prev, p);
        if dist + delta > max {
            let splits = ((dist + delta) / max).floor() as usize;
            let mut ip = p;
            for j in 0..splits {
                let frac = ((j + 1) as f64 * max - dist) / delta;
                ip = (frac * (p.0 - prev.0) + prev.0, frac * (p.1 - prev.1) + prev.1);
                if frac != 0.0 {
                    cur.push(ip);
                }
                out.push(std::mem::replace(&mut cur, vec![ip]));
            }
            if p == ip {
                dist = 0.0;
                prev = p;
                continue;
            }
            dist = dist_of(p, ip);
        } else {
            dist += delta;
        }
        cur.push(p);
        prev = p;
    }
    if cur.len() > 1 {
        out.push(cur);
    }
}

/// `geom::line_merge` of osm2pgsql: the lines joined at their ends, where exactly two ends meet.
pub fn line_merge(lines: &[Vec<Pt>]) -> Vec<Vec<Pt>> {
    const NO: usize = usize::MAX;
    // (point, line, front)
    let mut ends: Vec<(Pt, usize, bool)> = Vec::with_capacity(2 * lines.len());
    for (i, l) in lines.iter().enumerate() {
        ends.push((l[0], i, true));
        ends.push((*l.last().expect("a line"), i, false));
    }
    ends.sort_by(|a, b| (a.0 .0, a.0 .1, a.1, a.2).partial_cmp(&(b.0 .0, b.0 .1, b.1, b.2)).expect("no NaN"));
    // (left, right, done)
    let mut conn: Vec<(usize, usize, bool)> = vec![(NO, NO, false); lines.len()];
    let mut i = 0;
    while i + 1 < ends.len() {
        if ends[i].0 != ends[i + 1].0 {
            i += 1;
            continue;
        }
        let (a, b) = (ends[i], ends[i + 1]);
        if a.2 {
            conn[a.1].0 = b.1;
        } else {
            conn[a.1].1 = b.1;
        }
        if b.2 {
            conn[b.1].0 = a.1;
        } else {
            conn[b.1].1 = a.1;
        }
        i += 2;
    }
    let add = |out: &mut Vec<Pt>, pts: &mut dyn Iterator<Item = Pt>| {
        if !out.is_empty() {
            pts.next();
        }
        out.extend(pts);
    };
    let mut out = Vec::new();
    let mut done = 0;
    for i in 0..lines.len() {
        if conn[i].2 || (conn[i].0 != NO && conn[i].1 != NO) {
            continue;
        }
        let (mut line, mut prev, mut cur) = (Vec::new(), NO, i);
        while cur != NO {
            let c = conn[cur];
            let l = &lines[cur];
            let forward = c.0 == prev;
            prev = cur;
            if forward {
                add(&mut line, &mut l.iter().copied());
                cur = c.1;
            } else {
                add(&mut line, &mut l.iter().rev().copied());
                cur = c.0;
            }
            conn[prev].2 = true;
            done += 1;
        }
        out.push(line);
    }
    if done < lines.len() {
        // rings with no end
        for i in 0..lines.len() {
            if conn[i].2 {
                continue;
            }
            let (mut line, mut prev, mut cur) = (Vec::new(), conn[i].0, i);
            loop {
                let c = conn[cur];
                let l = &lines[cur];
                let forward = c.0 == prev && (conn[c.0].2 || lines[c.0].last() == l.first());
                prev = cur;
                if forward {
                    add(&mut line, &mut l.iter().copied());
                    cur = c.1;
                } else {
                    add(&mut line, &mut l.iter().rev().copied());
                    cur = c.0;
                }
                conn[prev].2 = true;
                if cur == i {
                    break;
                }
            }
            out.push(line);
        }
    }
    out
}

/// The rows of the tables from a row of the engine (EPSG:4326), with `merc` in EPSG:3857: a point; a line, cut into parts
/// (a relation: its member ways joined first), each also in `planet_osm_roads` when [`z_order`] says so; a polygon with
/// `way_area` (not valid in EPSG:3857: no row).
pub fn finish(f: Feature, merc: bool, out: &mut Vec<Feature>) {
    let proj = |p: Pt| if merc { mercator(p) } else { (fixed(p.0), fixed(p.1)) };
    let rings = read_rings(&f.geometry);
    match f.layer {
        POINT => out.push(Feature { geometry: crate::gdal::wkb_point(proj(rings[0][0])), ..f }),
        LINE => {
            let mut lines: Vec<Vec<Pt>> = rings.iter().map(|l| dedupe(&l.iter().map(|&p| (fixed(p.0), fixed(p.1))).collect::<Vec<_>>())).filter(|l| l.len() > 1).collect();
            if lines.len() > 1 {
                lines = line_merge(&lines);
            }
            let roads = z_order(&f.tags).1;
            let mut parts = Vec::new();
            for l in &lines {
                let l: Vec<Pt> = l.iter().map(|&p| proj(p)).collect();
                split_line(&l, if merc { 100_000.0 } else { 1.0 }, &mut parts);
            }
            for p in parts {
                let r = Feature { geometry: crate::gdal::wkb_line(&p), center: p[0], ..f.clone() };
                if roads {
                    out.push(Feature { layer: ROADS, ..r.clone() });
                }
                out.push(r);
            }
        }
        _ => {
            let rings: Vec<Vec<Pt>> = rings.iter().map(|r| r.iter().map(|&p| proj(p)).collect()).collect();
            if merc && !valid(&rings) {
                return;
            }
            let area = (ring_area(&rings[0]).abs() - rings[1..].iter().map(|r| ring_area(r).abs()).sum::<f64>()).abs();
            // osm2pgsql writes the area as text with "{:g}" (6 digits), and PostgreSQL keeps a real
            let area: f64 = format!("{area:.5e}").parse().expect("a number");
            let mut tags = f.tags.clone();
            match tags.iter_mut().find(|t| t.0 == "way_area") {
                Some(t) => t.1 = area.to_string(),
                None => tags.push(("way_area".into(), area.to_string())),
            }
            out.push(Feature { tags, geometry: wkb_rings(3, &rings), ..f });
        }
    }
}

/// The polygon of `rings` is valid: the rules of the geo crate's `Validation` for a polygon (osm2pgsql's `ST_IsValid`
/// check), with the test of each ring for a self-intersection by a sweep ([`self_intersects`]) in place of geo's test of
/// each pair of segments, which took most of the time of a run.
pub fn valid(rings: &[Vec<Pt>]) -> bool {
    use geo::coordinate_position::CoordPos;
    use geo::dimensions::Dimensions;
    use geo::{PreparedGeometry, Relate, RemoveRepeatedPoints};
    let ls = |r: &Vec<Pt>| geo::LineString::from(r.iter().map(|p| geo::coord! { x: p.0, y: p.1 }).collect::<Vec<_>>());
    let poly = geo::Polygon::new(ls(&rings[0]), rings[1..].iter().map(ls).collect());
    if poly.exterior().0.is_empty() {
        return true;
    }
    for r in std::iter::once(poly.exterior()).chain(poly.interiors()) {
        if r.0.is_empty() {
            continue;
        }
        if r.remove_repeated_points().0.len() < 4 || self_intersects(r) || r.0.iter().any(|c| !c.x.is_finite() || !c.y.is_finite()) {
            return false;
        }
    }
    let holes = poly.interiors();
    if holes.iter().all(|r| r.0.is_empty()) {
        return true;
    }
    let outer = geo::Polygon::new(poly.exterior().clone(), vec![]);
    let outer = PreparedGeometry::from(&outer);
    for (k, h1) in holes.iter().enumerate() {
        if h1.0.is_empty() {
            continue;
        }
        let p1 = geo::Polygon::new(h1.clone(), vec![]);
        let p1 = PreparedGeometry::from(&p1);
        let m = outer.relate(&p1);
        if !m.is_contains() || m.get(CoordPos::OnBoundary, CoordPos::OnBoundary) == Dimensions::OneDimensional {
            return false;
        }
        for h2 in &holes[k + 1..] {
            if h2.0.is_empty() {
                continue;
            }
            let m = p1.relate(&geo::Polygon::new(h2.clone(), vec![]));
            if m.get(CoordPos::Inside, CoordPos::Inside) == Dimensions::TwoDimensional
                || m.get(CoordPos::OnBoundary, CoordPos::OnBoundary) == Dimensions::OneDimensional
            {
                return false;
            }
        }
    }
    true
}

/// Two segments of the ring meet, other than the end of one at the start of the other: the rule of geo's
/// `linestring_has_self_intersection`. A sweep by x: the segments sorted by their smallest x, and each compared only with
/// the segments before it whose boxes overlap its box.
fn self_intersects(r: &geo::LineString) -> bool {
    use geo::Intersects;
    let segs: Vec<geo::Line> = r.lines().collect();
    let x0 = |l: &geo::Line| l.start.x.min(l.end.x);
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|&a, &b| x0(&segs[a]).total_cmp(&x0(&segs[b])));
    let mut open: Vec<usize> = Vec::new();
    for &i in &order {
        let a = segs[i];
        let (ax, ay0, ay1) = (x0(&a), a.start.y.min(a.end.y), a.start.y.max(a.end.y));
        open.retain(|&j| segs[j].start.x.max(segs[j].end.x) >= ax);
        for &j in &open {
            let b = segs[j];
            if b.start.y.max(b.end.y) < ay0 || b.start.y.min(b.end.y) > ay1 {
                continue;
            }
            if a.intersects(&b) && a.start != b.end && a.end != b.start {
                return true;
            }
        }
        open.push(i);
    }
    false
}

/// The area of a ring (signed), as Boost's `surveyor`: the sum of (x1 - x2)(y1 + y2), which loses less with large
/// coordinates than x1 y2 - x2 y1; each step a fused multiply-add, as clang compiles it.
fn ring_area(r: &[Pt]) -> f64 {
    r.windows(2).fold(0.0, |s, w| (w[0].0 - w[1].0).mul_add(w[0].1 + w[1].1, s)) / 2.0
}

/// The WKB of a polygon (3) or a MULTILINESTRING (5) from its rings or lines.
fn wkb_rings(kind: u32, rings: &[Vec<Pt>]) -> Vec<u8> {
    let mut o = vec![1u8];
    o.extend_from_slice(&kind.to_le_bytes());
    o.extend_from_slice(&(rings.len() as u32).to_le_bytes());
    for r in rings {
        if kind == 5 {
            o.push(1);
            o.extend_from_slice(&2u32.to_le_bytes());
        }
        o.extend_from_slice(&(r.len() as u32).to_le_bytes());
        r.iter().for_each(|p| {
            o.extend_from_slice(&p.0.to_le_bytes());
            o.extend_from_slice(&p.1.to_le_bytes());
        });
    }
    o
}

/// The points of a WKB of this module: a point, a line, a polygon (its rings) or a MULTILINESTRING (its lines).
fn read_rings(b: &[u8]) -> Vec<Vec<Pt>> {
    let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes")) as usize;
    let f64_at = |at: usize| f64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"));
    let pts = |at: &mut usize| {
        let n = u32_at(*at);
        let v: Vec<Pt> = (0..n).map(|i| (f64_at(*at + 4 + 16 * i), f64_at(*at + 12 + 16 * i))).collect();
        *at += 4 + 16 * n;
        v
    };
    match u32_at(1) {
        1 => vec![vec![(f64_at(5), f64_at(13))]],
        2 => vec![pts(&mut 5)],
        kind => {
            let mut at = 9;
            (0..u32_at(5))
                .map(|_| {
                    // a line of a MULTILINESTRING or GEOMETRYCOLLECTION (the joined parts) has a header of its own
                    at += if kind == 3 { 0 } else { 5 };
                    pts(&mut at)
                })
                .collect()
        }
    }
}

/// A value of a column.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Text(String),
    Int(i32),
    Real(f32),
}

/// `sscanf("%18lld")`: the integer at the start, and the text after it.
fn scan_int(s: &str) -> Option<(i64, &str)> {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let sign = usize::from(t.starts_with(['-', '+']));
    let digits = t[sign..].bytes().take(18).take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let end = sign + digits;
    Some((t[..end].parse().ok()?, &t[end..]))
}

/// `sscanf("%lf")`: the number at the start, and the text after it.
fn scan_real(s: &str) -> Option<(f64, &str)> {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let end = (1..=t.len()).rev().filter(|&i| t.is_char_boundary(i)).find(|&i| t[..i].parse::<f64>().is_ok() && !t[..i].ends_with(['e', 'E']))?;
    Some((t[..end].parse().ok()?, &t[end..]))
}

/// `table_t::escape_type`: the value of a tag in a column of the type `kind` (None: NULL).
fn cell(v: &str, kind: Kind) -> Option<Cell> {
    match kind {
        Kind::Text => Some(Cell::Text(v.to_string())),
        Kind::Int => {
            let (from, rest) = scan_int(v)?;
            let to = rest.strip_prefix('-').and_then(scan_int).map(|t| t.0);
            let n = match to {
                Some(to) => from / 2 + to / 2 + (from % 2 + to % 2) / 2,
                None => from,
            };
            i32::try_from(n).ok().map(Cell::Int)
        }
        Kind::Real => {
            let v = v.replace(',', ".");
            let feet = if v.len() > 1 && v.ends_with("ft") { 0.3048 } else { 1.0 };
            let (from, rest) = scan_real(&v)?;
            let n = match rest.strip_prefix('-').and_then(scan_real) {
                Some((to, _)) => (from * feet + to * feet) / 2.0,
                None => from * feet,
            };
            Some(Cell::Real(n as f32))
        }
    }
}

/// The values of the columns ([`columns`] of the table of the row) and the `tags` (hstore: the tags with no column,
/// not `z_order`), as `table_t::write_row` writes them.
pub fn values(f: &Feature) -> (Vec<Option<Cell>>, RawTags) {
    let cols = columns(if f.layer == POINT { POINT } else { LINE });
    let mut used = vec![false; f.tags.len()];
    let vals = cols
        .iter()
        .map(|(c, kind)| {
            let i = f.tags.iter().position(|t| t.0 == *c)?;
            used[i] = true;
            cell(&f.tags[i].1, *kind)
        })
        .collect();
    let tags = f.tags.iter().zip(used).filter(|(t, u)| !u && t.0 != "z_order").map(|(t, _)| t.clone()).collect();
    (vals, tags)
}

// ---- the polygons: osmium's assembler (`BasicAssembler`, as osm2pgsql's `area_assembler_t` calls it)

type IPt = (i64, i64);

fn ipt(p: Pt) -> IPt {
    ((p.0 * 1e7).round() as i64, (p.1 * 1e7).round() as i64)
}

fn fpt(p: IPt) -> Pt {
    (p.0 as f64 / 1e7, p.1 as f64 / 1e7)
}

fn cross(o: IPt, a: IPt, b: IPt) -> i128 {
    (a.0 - o.0) as i128 * (b.1 - o.1) as i128 - (a.1 - o.1) as i128 * (b.0 - o.0) as i128
}

fn on_segment(a: IPt, b: IPt, p: IPt) -> bool {
    a.0.min(b.0) <= p.0 && p.0 <= a.0.max(b.0) && a.1.min(b.1) <= p.1 && p.1 <= a.1.max(b.1)
}

/// Two segments with no common end meet (`calculate_intersection` of osmium: also a touch or an overlap).
fn meet(s: (IPt, IPt), t: (IPt, IPt)) -> bool {
    if s.0 == t.0 || s.0 == t.1 || s.1 == t.0 || s.1 == t.1 {
        return false;
    }
    let (d1, d2) = (cross(t.0, t.1, s.0), cross(t.0, t.1, s.1));
    let (d3, d4) = (cross(s.0, s.1, t.0), cross(s.0, s.1, t.1));
    if ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0)) {
        return true;
    }
    (d1 == 0 && on_segment(t.0, t.1, s.0))
        || (d2 == 0 && on_segment(t.0, t.1, s.1))
        || (d3 == 0 && on_segment(s.0, s.1, t.0))
        || (d4 == 0 && on_segment(s.0, s.1, t.1))
}

/// The point is inside the ring (not on it).
fn in_ring(r: &[IPt], p: IPt) -> bool {
    let mut inside = false;
    for w in r.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a.1 > p.1) != (b.1 > p.1) {
            let c = cross(a, b, p);
            if (c > 0) == (b.1 > a.1) {
                inside = !inside;
            }
        }
    }
    inside
}

/// The polygons (each an outer ring and its holes) that the lines make, as osmium's assembler makes them: the segments of
/// all lines, two equal segments take each other away, segments that meet (not at a common end) or a ring that does not
/// close give None; the rings are cut where they touch themselves, and a ring inside an odd number of rings is a hole of
/// the smallest ring that holds it.
pub fn assemble(lines: &[Vec<Pt>]) -> Option<Vec<Vec<Vec<Pt>>>> {
    let mut segs: Vec<(IPt, IPt)> = Vec::new();
    for l in lines {
        for w in l.windows(2) {
            let (a, b) = (ipt(w[0]), ipt(w[1]));
            if a != b {
                segs.push(if a < b { (a, b) } else { (b, a) });
            }
        }
    }
    segs.sort_unstable();
    let mut kept = Vec::with_capacity(segs.len());
    let mut i = 0;
    while i < segs.len() {
        if i + 1 < segs.len() && segs[i] == segs[i + 1] {
            i += 2;
        } else {
            kept.push(segs[i]);
            i += 1;
        }
    }
    let segs = kept;
    if segs.is_empty() {
        return None;
    }
    // the segments are sorted by their first x, the smallest x of each
    for i in 0..segs.len() {
        for t in &segs[i + 1..] {
            if t.0 .0 > segs[i].1 .0 {
                break;
            }
            if meet(segs[i], *t) {
                return None;
            }
        }
    }
    let mut at: crate::util::FxHashMap<IPt, Vec<usize>> = Default::default();
    for (i, s) in segs.iter().enumerate() {
        at.entry(s.0).or_default().push(i);
        at.entry(s.1).or_default().push(i);
    }
    if at.values().any(|v| v.len() % 2 == 1) {
        return None;
    }
    // the rings: walk the unused segments; where the walk comes back to a point of its path, that loop is a ring
    let mut used = vec![false; segs.len()];
    let mut rings: Vec<Vec<IPt>> = Vec::new();
    for s in 0..segs.len() {
        if used[s] {
            continue;
        }
        used[s] = true;
        let mut path = vec![segs[s].0, segs[s].1];
        while path.len() > 1 {
            let cur = *path.last().expect("a point");
            let next = at[&cur].iter().copied().find(|&j| !used[j])?;
            used[next] = true;
            let p = if segs[next].0 == cur { segs[next].1 } else { segs[next].0 };
            match path.iter().position(|&q| q == p) {
                Some(k) => {
                    let mut ring = path.split_off(k);
                    ring.push(p);
                    path.push(p);
                    rings.push(ring);
                }
                None => path.push(p),
            }
        }
    }
    // the nesting: from the largest ring to the smallest
    let area = |r: &[IPt]| r.windows(2).map(|w| w[0].0 as i128 * w[1].1 as i128 - w[1].0 as i128 * w[0].1 as i128).sum::<i128>().abs();
    rings.sort_by_cached_key(|r| std::cmp::Reverse(area(r)));
    let bbox = |r: &[IPt]| r.iter().fold((i64::MAX, i64::MAX, i64::MIN, i64::MIN), |b, p| (b.0.min(p.0), b.1.min(p.1), b.2.max(p.0), b.3.max(p.1)));
    let boxes: Vec<_> = rings.iter().map(|r| bbox(r)).collect();
    let mut parent: Vec<Option<usize>> = vec![None; rings.len()];
    let mut depth = vec![0usize; rings.len()];
    for i in 0..rings.len() {
        for j in (0..i).rev() {
            let (a, b) = (boxes[i], boxes[j]);
            if a.0 < b.0 || a.1 < b.1 || a.2 > b.2 || a.3 > b.3 {
                continue;
            }
            // a point of ring i that is not on ring j (the rings touch only at points), else the middle of a segment
            let p = match rings[i].iter().find(|p| !rings[j].contains(p)) {
                Some(&p) => p,
                None => continue,
            };
            if in_ring(&rings[j], p) {
                parent[i] = Some(j);
                depth[i] = depth[j] + 1;
                break;
            }
        }
    }
    // as osmium: an outer ring counterclockwise, a hole clockwise (each ring starts at its smallest point)
    for (r, d) in rings.iter_mut().zip(&depth) {
        let signed: i128 = r.windows(2).map(|w| w[0].0 as i128 * w[1].1 as i128 - w[1].0 as i128 * w[0].1 as i128).sum();
        if (signed > 0) != (d % 2 == 0) {
            r.reverse();
        }
    }
    let polys: Vec<Vec<Vec<Pt>>> = (0..rings.len())
        .filter(|&i| depth[i] % 2 == 0)
        .map(|i| {
            let mut p = vec![rings[i].iter().map(|&q| fpt(q)).collect()];
            // the holes in the order that osmium makes the rings: by their first (smallest) point
            let mut holes: Vec<usize> = (0..rings.len()).filter(|&k| parent[k] == Some(i) && depth[k] % 2 == 1).collect();
            holes.sort_by_key(|&k| rings[k][0]);
            p.extend(holes.into_iter().map(|k| rings[k].iter().map(|&q| fpt(q)).collect()));
            p
        })
        .collect();
    Some(polys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(kv: &[(&str, &str)]) -> RawTags {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn style_columns_and_tags() {
        assert_eq!((columns(POINT).len(), columns(LINE).len()), (67, 67));
        assert!(columns(LINE).iter().any(|c| c.0 == "way_area" && c.1 == Kind::Real));
        assert!(!columns(POINT).iter().any(|c| c.0 == "tracktype"));
        // a node with only deleted tags is no row; `ele` stays
        assert!(point(1, &t(&[("source", "x"), ("note:en", "y")]), (0.0, 0.0)).is_none());
        assert!(point(1, &t(&[("ele", "5")]), (0.0, 0.0)).is_some());
        // building makes a polygon, area=no stops it, natural=coastline makes one and goes
        assert!(filter_tags(&t(&[("building", "yes")]), Obj::Way).unwrap().1);
        assert!(!filter_tags(&t(&[("building", "yes"), ("area", "no")]), Obj::Way).unwrap().1);
        let (tags, p) = filter_tags(&t(&[("natural", "coastline"), ("name", "x")]), Obj::Way).unwrap();
        assert!(p && tags == t(&[("name", "x"), ("area", "yes")]));
    }

    #[test]
    fn z_order_and_roads() {
        assert_eq!(z_order(&t(&[("highway", "primary"), ("bridge", "yes"), ("layer", "1")])), (237, true));
        assert_eq!(z_order(&t(&[("highway", "residential"), ("tunnel", "yes")])), (-67, false));
        assert_eq!(z_order(&t(&[("railway", "rail")])), (35, true));
        assert_eq!(z_order(&t(&[("boundary", "administrative")])), (0, true));
    }

    #[test]
    fn route_tags() {
        let r = relation(&t(&[("type", "route"), ("name", "A"), ("network", "lcn"), ("ref", "7")])).unwrap();
        assert!(r.lines && !r.polygons);
        assert_eq!(get(&r.tags, "route_name"), Some("A"));
        assert_eq!(get(&r.tags, "lcn"), Some("yes"));
        assert_eq!(get(&r.tags, "lcn_ref"), Some("7"));
        assert_eq!(get(&r.tags, "route_pref_color"), Some("0"));
        assert!(relation(&t(&[("type", "site"), ("name", "A")])).is_none());
        let b = relation(&t(&[("type", "multipolygon"), ("boundary", "x")])).unwrap();
        assert!(b.lines && b.polygons);
    }

    #[test]
    fn merge_and_split() {
        let m = line_merge(&[vec![(1.0, 0.0), (2.0, 0.0)], vec![(0.0, 0.0), (1.0, 0.0)]]);
        assert_eq!(m, vec![vec![(2.0, 0.0), (1.0, 0.0), (0.0, 0.0)]]);
        let mut parts = Vec::new();
        split_line(&[(0.0, 0.0), (250.0, 0.0)], 100.0, &mut parts);
        assert_eq!(parts, vec![vec![(0.0, 0.0), (100.0, 0.0)], vec![(100.0, 0.0), (200.0, 0.0)], vec![(200.0, 0.0), (250.0, 0.0)]]);
    }

    #[test]
    fn assembler() {
        let sq = |x: f64, y: f64, s: f64| vec![(x, y), (x + s, y), (x + s, y + s), (x, y + s), (x, y)];
        // a hole, a polygon in the hole
        let p = assemble(&[sq(0.0, 0.0, 10.0), sq(2.0, 2.0, 6.0), sq(4.0, 4.0, 1.0)]).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p.iter().map(|p| p.len()).sum::<usize>(), 3);
        // two halves of a ring join; an open line gives None; a bow tie gives None
        assert_eq!(assemble(&[vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)], vec![(1.0, 1.0), (0.0, 1.0), (0.0, 0.0)]]).unwrap().len(), 1);
        assert!(assemble(&[vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]]).is_none());
        assert!(assemble(&[vec![(0.0, 0.0), (1.0, 1.0), (1.0, 0.0), (0.0, 1.0), (0.0, 0.0)]]).is_none());
    }

    /// `valid` equals geo's `is_valid` on rings that are valid and not valid in many ways.
    #[test]
    fn valid_equals_geo() {
        use geo::Validation;
        let sq = |x: f64, y: f64, d: f64| vec![(x, y), (x + d, y), (x + d, y + d), (x, y + d), (x, y)];
        let cases: Vec<Vec<Vec<Pt>>> = vec![
            vec![sq(0.0, 0.0, 10.0)],
            vec![vec![(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0), (0.0, 0.0)]], // bow tie
            vec![vec![(0.0, 0.0), (10.0, 0.0), (5.0, 0.0), (5.0, 5.0), (0.0, 0.0)]],    // spike
            vec![vec![(0.0, 0.0), (10.0, 0.0), (0.0, 0.0), (0.0, 0.0)]],                // too few points
            vec![vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (2.0, 0.0), (0.0, 4.0), (0.0, 0.0)]], // touches itself
            vec![sq(0.0, 0.0, 10.0), sq(2.0, 2.0, 2.0)],
            vec![sq(0.0, 0.0, 10.0), sq(0.0, 2.0, 2.0)],  // hole on the outer ring along a line
            vec![sq(0.0, 0.0, 10.0), sq(8.0, 8.0, 4.0)],  // hole not inside
            vec![sq(0.0, 0.0, 10.0), sq(2.0, 2.0, 3.0), sq(3.0, 3.0, 3.0)], // holes overlap
            vec![sq(0.0, 0.0, 10.0), sq(2.0, 2.0, 2.0), sq(4.0, 2.0, 2.0)], // holes share a side
            vec![sq(0.0, 0.0, 10.0), sq(2.0, 2.0, 2.0), sq(4.0, 4.0, 2.0)], // holes touch at a point
            vec![vec![(0.0, 0.0), (5.0, 0.0), (5.0, 0.0), (5.0, 5.0), (0.0, 0.0)]],     // a repeated point
        ];
        // and many rings of random points, most of them not valid
        let mut seed = 7u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) % 8) as f64
        };
        let mut all = cases;
        for n in 0..400 {
            let mut r: Vec<Pt> = (0..4 + n % 9).map(|_| (rnd(), rnd())).collect();
            r.push(r[0]);
            all.push(vec![r]);
        }
        for rings in &all {
            let ls = |r: &Vec<Pt>| geo::LineString::from(r.iter().map(|p| geo::coord! { x: p.0, y: p.1 }).collect::<Vec<_>>());
            let want = geo::Polygon::new(ls(&rings[0]), rings[1..].iter().map(ls).collect()).is_valid();
            assert_eq!(valid(rings), want, "{rings:?}");
        }
    }

}
