//! duckosm quack --geofabrik: the layers of Geofabrik's free shapefiles and GeoPackages (`gis_osm_roads_free_1.shp`,
//! docs/concepts/geofabrik.md), for the quack engine ([`crate::quack`], [`crate::quack::Mode::Geofabrik`]). The engine
//! makes GDAL's rows ([`crate::gdal`]); [`rows`] gives the Geofabrik rows of each row: a node goes to the point layers, a
//! way to the line layers, and a closed way or a complete multipolygon or boundary to the area layers (`_a`). The tags
//! give the `fclass` and the `code` in each class of layers by the rules of `geofabrik_rules.txt`: one row for each key
//! of the rules that matches (a cafe with `shop=bakery` is two rows of `pois`).

use crate::gdal::{self, Schema, Value};
use crate::quack::{Feature, Pt};
use crate::raw::RawTags;
use std::sync::OnceLock;

/// The layers, in the order of Geofabrik's GeoPackage. `Feature::layer` is the place in this list plus 1.
const TABLES: [&str; 20] = [
    "gis_osm_traffic_free",
    "gis_osm_places_free",
    "gis_osm_natural_free",
    "gis_osm_pois_free",
    "gis_osm_transport_free",
    "gis_osm_pofw_free",
    "gis_osm_roads_free",
    "gis_osm_railways_free",
    "gis_osm_waterways_free",
    "gis_osm_pois_a_free",
    "gis_osm_buildings_a_free",
    "gis_osm_landuse_a_free",
    "gis_osm_traffic_a_free",
    "gis_osm_pofw_a_free",
    "gis_osm_water_a_free",
    "gis_osm_natural_a_free",
    "gis_osm_transport_a_free",
    "gis_osm_adminareas_a_free",
    "gis_osm_places_a_free",
    "gis_osm_protected_areas_a_free",
];

/// The classes of the rules: the name, the point or line layer (0: none) and the area layer (0: none).
const CLASSES: [(&str, u8, u8); 14] = [
    ("traffic", 1, 13),
    ("places", 2, 19),
    ("natural", 3, 16),
    ("pois", 4, 10),
    ("transport", 5, 17),
    ("pofw", 6, 14),
    ("roads", 7, 0),
    ("railways", 8, 0),
    ("waterways", 9, 0),
    ("buildings", 0, 11),
    ("landuse", 0, 12),
    ("water", 0, 15),
    ("adminareas", 0, 18),
    ("protected_areas", 0, 20),
];

/// Layers 1 to 6 are points, 7 to 9 lines, the others areas.
fn is_point(layer: u8) -> bool {
    layer <= 6
}

fn is_line(layer: u8) -> bool {
    (7..=9).contains(&layer)
}

/// The class of a layer.
fn class_of(layer: u8) -> &'static str {
    CLASSES.iter().find(|c| c.1 == layer || c.2 == layer).expect("a layer of a class").0
}

/// The layers of quack --geofabrik.
pub const SCHEMA: Schema = Schema { layers: 20, table, geometry, columns, values };

fn table(layer: u8) -> &'static str {
    TABLES[layer as usize - 1]
}

fn geometry(layer: u8) -> &'static str {
    if is_point(layer) {
        "POINT"
    } else if is_line(layer) {
        "LINESTRING"
    } else {
        "MULTIPOLYGON"
    }
}

/// The columns of a layer, as Geofabrik's files have them: name, integer, width.
fn columns(layer: u8) -> Vec<(&'static str, bool, u8)> {
    let mut c = vec![("osm_id", false, 12), ("code", true, 4), ("fclass", false, 28)];
    let name = ("name", false, 100);
    match class_of(layer) {
        "places" => c.extend([("population", true, 10), name]),
        "traffic" => c.extend([name, ("calming", false, 30)]),
        "roads" => c.extend([
            name,
            ("ref", false, 20),
            ("oneway", false, 1),
            ("maxspeed", true, 3),
            ("layer", true, 12),
            ("bridge", false, 1),
            ("tunnel", false, 1),
        ]),
        "railways" => c.extend([name, ("layer", true, 12), ("bridge", false, 1), ("tunnel", false, 1)]),
        "waterways" => c.extend([("width", true, 5), name]),
        "buildings" => c.extend([name, ("type", false, 20)]),
        "protected_areas" => {}
        _ => c.push(name),
    }
    c
}

/// The values of a row: [`rows`] keeps them in `Feature::tags` (column, value). An empty text is `''` and an integer
/// with no value 0, as in Geofabrik's GeoPackage.
fn values(f: &Feature) -> Vec<Option<Value>> {
    columns(f.layer)
        .iter()
        .map(|(c, int, _)| {
            let v = f.tags.iter().find(|t| t.0 == *c).map_or("", |t| t.1.as_str());
            Some(if *int { Value::Int(v.parse().unwrap_or(0)) } else { Value::Text(v.to_string()) })
        })
        .collect()
}

// ---- the rules

enum Cond {
    /// the tag has one of the values (`*`: any value)
    Is(&'static str, Vec<&'static str>),
    /// no tag, or another value
    Not(&'static str, Vec<&'static str>),
    Absent(&'static str),
}

struct Rule {
    class: usize,
    code: u16,
    fclass: &'static str,
    conds: Vec<Cond>,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let mut out = Vec::new();
        for line in include_str!("geofabrik_rules.txt").lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let mut w = line.split_whitespace();
            let class = w.next().expect("a class");
            let class = CLASSES.iter().position(|c| c.0 == class).unwrap_or_else(|| panic!("geofabrik_rules.txt: class {class}"));
            let code = w.next().and_then(|c| c.parse().ok()).expect("a code");
            let fclass = w.next().expect("an fclass");
            let conds = w
                .map(|c| {
                    if let Some(k) = c.strip_prefix('!') {
                        Cond::Absent(k)
                    } else if let Some((k, v)) = c.split_once("!=") {
                        Cond::Not(k, v.split('|').collect())
                    } else {
                        let (k, v) = c.split_once('=').unwrap_or_else(|| panic!("geofabrik_rules.txt: {c}"));
                        Cond::Is(k, v.split('|').collect())
                    }
                })
                .collect();
            out.push(Rule { class, code, fclass, conds });
        }
        out
    })
}

fn get<'a>(tags: &'a RawTags, k: &str) -> Option<&'a str> {
    tags.iter().find(|t| t.0 == k).map(|t| t.1.as_str())
}

/// The key of a rule: the key of its first condition.
fn key(r: &Rule) -> &'static str {
    match &r.conds[0] {
        Cond::Is(k, _) | Cond::Not(k, _) | Cond::Absent(k) => k,
    }
}

/// The codes and the fclasses of the tags in a class: for each key of the rules, the first rule of that key that holds
/// (Geofabrik's files have a row for each: a cafe with `shop=bakery` is a `bakery` row and a `cafe` row).
fn classify(class: usize, tags: &RawTags) -> Vec<(u16, &'static str)> {
    let holds = |c: &Cond| match c {
        Cond::Is(k, vs) => get(tags, k).is_some_and(|v| vs.iter().any(|x| *x == "*" || *x == v)),
        Cond::Not(k, vs) => get(tags, k).is_none_or(|v| !vs.contains(&v)),
        Cond::Absent(k) => get(tags, k).is_none(),
    };
    let mut out: Vec<(u16, &'static str)> = Vec::new();
    let mut done: Vec<&str> = Vec::new();
    for r in rules().iter().filter(|r| r.class == class) {
        if !done.contains(&key(r)) && r.conds.iter().all(holds) {
            out.push((r.code, r.fclass));
            done.push(key(r));
        }
    }
    out
}

// ---- the rows

/// The text cut to at most `width` bytes, at the end of a character (Geofabrik cuts `type` to 20 bytes).
fn cut(s: &str, width: u8) -> &str {
    let mut n = s.len().min(width as usize);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

/// The row of `tags` in `layer` with the code and the fclass, its column values in `Feature::tags`.
fn row(layer: u8, id: &str, (code, fclass): (u16, &str), tags: &RawTags, geometry: Vec<u8>, center: Pt) -> Feature {
    let flag = |k: &str| if get(tags, k).is_some_and(|v| v != "no") { "T" } else { "F" };
    let int = |k: &str| get(tags, k).map_or(0, gdal::atoi).to_string();
    let mut t: RawTags = Vec::new();
    for (c, _, width) in columns(layer) {
        let v = match c {
            "osm_id" => id.to_string(),
            "code" => code.to_string(),
            "fclass" => fclass.to_string(),
            "oneway" => match get(tags, "oneway") {
                Some("yes" | "true" | "1") => "F",
                Some("-1" | "reverse") => "T",
                _ => "B",
            }
            .to_string(),
            "bridge" | "tunnel" => flag(c).to_string(),
            "maxspeed" | "layer" | "width" | "population" => int(c),
            "calming" => get(tags, "traffic_calming").unwrap_or("").to_string(),
            "type" => get(tags, "building").filter(|v| *v != "yes").unwrap_or("").to_string(),
            _ => get(tags, c).unwrap_or("").to_string(),
        };
        if !v.is_empty() {
            let v = if c == "osm_id" || width == 0 { v } else { cut(&v, width).to_string() };
            t.push((c.to_string(), v));
        }
    }
    Feature { feature_id: format!("{}/{id}", if is_point(layer) { "node" } else { "way" }), tags: t, geometry, center, owner: true, layer, part: None }
}

/// The rows of `tags` in the layers of one kind: `pick` gives the layer of a class (0: none).
fn rows_of(id: &str, tags: &RawTags, pick: impl Fn(&(&str, u8, u8)) -> u8, geometry: &[u8], center: Pt, out: &mut Vec<Feature>) {
    for (i, c) in CLASSES.iter().enumerate() {
        let layer = pick(c);
        if layer == 0 {
            continue;
        }
        for r in classify(i, tags) {
            out.push(row(layer, id, r, tags, geometry.to_vec(), center));
        }
    }
}

/// The points of a WKB line, or the rings of a WKB multipolygon (little endian, as `crate::gdal` writes them).
fn read_pts(b: &mut &[u8]) -> Vec<Pt> {
    let u32_ = |b: &mut &[u8]| {
        let v = u32::from_le_bytes(b[..4].try_into().expect("4 bytes"));
        *b = &b[4..];
        v
    };
    (0..u32_(b))
        .map(|_| {
            let x = f64::from_le_bytes(b[..8].try_into().expect("8 bytes"));
            let y = f64::from_le_bytes(b[8..16].try_into().expect("8 bytes"));
            *b = &b[16..];
            (x, y)
        })
        .collect()
}

fn rings(wkb: &[u8]) -> Vec<Vec<Pt>> {
    let mut b = &wkb[5..];
    let polys = u32::from_le_bytes(b[..4].try_into().expect("4 bytes"));
    b = &b[4..];
    let mut out = Vec::new();
    for _ in 0..polys {
        b = &b[5..];
        let n = u32::from_le_bytes(b[..4].try_into().expect("4 bytes"));
        b = &b[4..];
        for _ in 0..n {
            out.push(read_pts(&mut b));
        }
    }
    out
}

/// The Geofabrik rows of a row of GDAL's layers (the engine in [`crate::quack::Mode::Geofabrik`]), to `out`.
pub fn rows(f: &Feature, out: &mut Vec<Feature>) {
    let Some((kind, id)) = f.feature_id.split_once('/') else { return };
    if f.part.is_some() {
        return;
    }
    let point_or_line = |c: &(&str, u8, u8)| c.1;
    let area = |c: &(&str, u8, u8)| c.2;
    match f.layer {
        gdal::POINTS => {
            // the coordinates of OSM: 7 decimals (as the points of the ways)
            let xy = |at: usize| (f64::from_le_bytes(f.geometry[at..at + 8].try_into().expect("8 bytes")) * 1e7).round() / 1e7;
            let p = (xy(5), xy(13));
            rows_of(id, &f.tags, |c| if is_point(c.1) { c.1 } else { 0 }, &gdal::wkb_point(p), p, out)
        }
        gdal::LINES => {
            rows_of(id, &f.tags, |c| if is_line(c.1) { c.1 } else { 0 }, &f.geometry, f.center, out);
            // a closed way that GDAL draws as a line is an area too (waterway=riverbank, man_made=pier), except area=no
            let pts = read_pts(&mut &f.geometry[5..]);
            if pts.len() >= 4 && pts.first() == pts.last() && get(&f.tags, "area") != Some("no") {
                rows_of(id, &f.tags, area, &gdal::wkb_multipolygon(&[vec![pts]]), f.center, out);
            }
        }
        gdal::MULTIPOLYGONS => {
            let rs = rings(&f.geometry);
            if kind == "way" {
                // the ring of an area way is a line too (a closed highway with amenity=parking)
                let line = gdal::wkb_line(&rs[0]);
                rows_of(id, &f.tags, |c| if is_line(c.1) { point_or_line(c) } else { 0 }, &line, f.center, out);
            } else if rs.iter().any(|r| r.len() < 4 || r.first() != r.last()) {
                // a multipolygon whose rings do not close is not an area
                return;
            }
            rows_of(id, &f.tags, area, &f.geometry, f.center, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(t: &[(&str, &str)]) -> RawTags {
        t.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn the_first_rule_that_holds() {
        let pois = CLASSES.iter().position(|c| c.0 == "pois").unwrap();
        assert_eq!(classify(pois, &tags(&[("amenity", "recycling"), ("recycling:glass", "yes")])), vec![(2031, "recycling_glass")]);
        assert_eq!(classify(pois, &tags(&[("amenity", "recycling")])), vec![(2030, "recycling")]);
        assert_eq!(classify(pois, &tags(&[("amenity", "cafe"), ("shop", "bakery")])), vec![(2502, "bakery"), (2303, "cafe")]);
        let rail = CLASSES.iter().position(|c| c.0 == "railways").unwrap();
        assert_eq!(classify(rail, &tags(&[("railway", "rail"), ("traction", "rack")])), vec![(6109, "rack")]);
        assert_eq!(classify(rail, &tags(&[("railway", "rail")])), vec![(6101, "rail")]);
        assert_eq!(cut("bâtiment_de_bureaux_et", 20), "bâtiment_de_bureaux");
    }
}
