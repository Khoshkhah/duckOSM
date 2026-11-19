//! The Shortbread 1.0 schema (https://shortbread-tiles.org/schema/1.0/, CC0): the vector tile layers of a feature of the
//! quack engine (`crate::quack`), with their attributes and their first zoom. `duckosm quack --pmtiles` uses it
//! (docs/exports/pmtiles.md).
//!
//! The rules follow the schema. Where the schema says nothing (the area test of a closed way, the first zoom of a water
//! polygon from its size, the z-order, the simplification), they follow the Tilemaker configuration of Geofabrik
//! (https://github.com/shortbread-tiles/shortbread-tilemaker, `process.lua` and `config.json`, version 1.0.1). The
//! differences from that configuration are in docs/exports/pmtiles.md, "Check against Tilemaker".

use crate::mvt::{mercator, ring_area, Shape, Value, P};
use crate::raw::RawTags;

/// A layer of the schema: its name, the simplification below zoom 14 (degrees at zoom 13; it doubles at each zoom
/// lower, as Tilemaker's `simplify_level` with `simplify_ratio` 2) and whether the features with equal attributes join
/// into one feature in a tile (Tilemaker's `combine_below`).
pub struct Layer {
    pub name: &'static str,
    pub simplify: f64,
    pub combine: bool,
}

const fn l(name: &'static str, simplify: f64, combine: bool) -> Layer {
    Layer { name, simplify, combine }
}

/// The layers, in the order of the schema. The ocean layer is not here: it comes from the water polygons of
/// osmdata.openstreetmap.de, not from the OSM file.
pub const LAYERS: [Layer; 25] = [
    l("water_polygons", 0.0001, false),
    l("water_polygons_labels", 0.0, false),
    l("water_lines", 0.0001, true),
    l("water_lines_labels", 0.0002, true),
    l("dam_lines", 0.0001, true),
    l("dam_polygons", 0.0001, false),
    l("pier_lines", 0.0002, false),
    l("pier_polygons", 0.0002, false),
    l("boundaries", 0.0001, true),
    l("boundary_labels", 0.0, false),
    l("place_labels", 0.0, false),
    l("land", 0.0001, false),
    l("sites", 0.0, false),
    l("buildings", 0.0, false),
    l("addresses", 0.0, false),
    l("streets", 0.0002, true),
    l("street_polygons", 0.0001, true),
    l("streets_polygons_labels", 0.0, false),
    l("street_labels", 0.0002, true),
    l("street_labels_points", 0.0, false),
    l("bridges", 0.0001, false),
    l("aerialways", 0.0003, false),
    l("ferries", 0.0, false),
    l("public_transport", 0.0, false),
    l("pois", 0.0, false),
];

const WATER_POLYGONS: u8 = 0;
const WATER_POLYGONS_LABELS: u8 = 1;
const WATER_LINES: u8 = 2;
const WATER_LINES_LABELS: u8 = 3;
const DAM_LINES: u8 = 4;
const DAM_POLYGONS: u8 = 5;
const PIER_LINES: u8 = 6;
const PIER_POLYGONS: u8 = 7;
const BOUNDARIES: u8 = 8;
const BOUNDARY_LABELS: u8 = 9;
const PLACE_LABELS: u8 = 10;
const LAND: u8 = 11;
const SITES: u8 = 12;
const BUILDINGS: u8 = 13;
const ADDRESSES: u8 = 14;
const STREETS: u8 = 15;
const STREET_POLYGONS: u8 = 16;
const STREETS_POLYGONS_LABELS: u8 = 17;
const STREET_LABELS: u8 = 18;
const STREET_LABELS_POINTS: u8 = 19;
const BRIDGES: u8 = 20;
const AERIALWAYS: u8 = 21;
const FERRIES: u8 = 22;
const PUBLIC_TRANSPORT: u8 = 23;
const POIS: u8 = 24;

/// The simplification of layer `layer` at zoom `z`, in world units (0 at zoom 14).
pub fn tolerance(layer: u8, z: u8) -> f64 {
    let s = LAYERS[layer as usize].simplify;
    if z >= 14 || s == 0.0 {
        return 0.0;
    }
    s / 360.0 * 2f64.powi(13 - z as i32)
}

/// A feature of a layer: its first zoom, the key of its order in the layer (smallest first), its attributes (each with
/// its first zoom) and its shape in world units.
#[derive(Debug, Clone)]
pub struct Out {
    pub layer: u8,
    pub minzoom: u8,
    pub sort: i64,
    pub attrs: Vec<(&'static str, Value, u8)>,
    pub shape: Shape,
}

/// The tags of a feature, with "" for a missing key.
struct Tags<'a>(&'a RawTags);

impl Tags<'_> {
    fn get(&self, k: &str) -> &str {
        self.0.iter().find(|(key, _)| key == k).map_or("", |(_, v)| v.as_str())
    }
    fn has(&self, k: &str) -> bool {
        !self.get(k).is_empty()
    }
}

/// The parts of one feature that the rules make.
struct Make<'a> {
    t: Tags<'a>,
    out: Vec<Out>,
}

impl Make<'_> {
    fn add(&mut self, layer: u8, minzoom: u8, shape: Shape) -> &mut Out {
        self.out.push(Out { layer, minzoom: minzoom.min(14), sort: 0, attrs: Vec::new(), shape });
        self.out.last_mut().unwrap()
    }
}

impl Out {
    fn s(&mut self, k: &'static str, v: &str) -> &mut Self {
        self.attrs.push((k, Value::Str(v.to_string()), 0));
        self
    }
    fn b(&mut self, k: &'static str, v: bool) -> &mut Self {
        self.attrs.push((k, Value::Bool(v), 0));
        self
    }
    fn i(&mut self, k: &'static str, v: i64) -> &mut Self {
        self.attrs.push((k, Value::Int(v), 0));
        self
    }
    fn f(&mut self, k: &'static str, v: f64) -> &mut Self {
        self.attrs.push((k, Value::Float(v), 0));
        self
    }
    /// The last attribute starts at zoom `z`.
    fn from(&mut self, z: u8) -> &mut Self {
        if let Some(a) = self.attrs.last_mut() {
            a.2 = z;
        }
        self
    }
    fn sort(&mut self, v: i64) -> &mut Self {
        self.sort = v;
        self
    }
    /// name, name_en and name_de: the tags name, name:en and name:de, when the feature has them.
    fn names(&mut self, t: &Tags) -> &mut Self {
        for (k, tag) in [("name", "name"), ("name_en", "name:en"), ("name_de", "name:de")] {
            if t.has(tag) {
                self.s(k, t.get(tag));
            }
        }
        self
    }
}

/// The geometry of a feature of the quack engine in world units.
enum Geom {
    Point(P),
    Line(Vec<P>),
    Polygons(Vec<Vec<Vec<P>>>),
}

fn geom(wkb: &[u8]) -> Option<Geom> {
    let ring = |l: &geo::LineString| l.0.iter().map(|c| mercator(c.x, c.y)).collect::<Vec<P>>();
    let poly = |p: &geo::Polygon| std::iter::once(p.exterior()).chain(p.interiors()).map(ring).collect::<Vec<_>>();
    Some(match crate::quack::read_wkb(wkb)? {
        geo::Geometry::Point(p) => Geom::Point(mercator(p.x(), p.y())),
        geo::Geometry::LineString(l) => Geom::Line(ring(&l)),
        geo::Geometry::Polygon(p) => Geom::Polygons(vec![poly(&p)]),
        geo::Geometry::MultiPolygon(m) => Geom::Polygons(m.0.iter().map(poly).collect()),
        _ => return None,
    })
}

/// The size of a world unit in meters of the Mercator projection (at the equator).
const WORLD_M: f64 = 40_075_016.685_578_5;
/// The circumference of Tilemaker's zoom formulas.
const CIRCUMFERENCE: f64 = 40_052_725.78;

/// Tilemaker's `zmin_for_area`: the first zoom at which an area (m², Mercator) has `px` square pixels.
fn zmin_for_area(px: f64, area: f64) -> i32 {
    (libm::log(px * CIRCUMFERENCE * CIRCUMFERENCE / (65536.0 * area)) / (2.0 * std::f64::consts::LN_2)).floor() as i32
}

/// Tilemaker's `zmin_for_length`: the first zoom at which a line of `length` m has `px` pixels.
fn zmin_for_length(px: f64, length: f64) -> i32 {
    (libm::log(CIRCUMFERENCE * px / (256.0 * length)) / std::f64::consts::LN_2).floor() as i32
}

/// The length of a line in meters on the sphere (as Tilemaker's `Length()`), from its world units.
fn length_m(l: &[P]) -> f64 {
    let ll = |p: &P| {
        let (lon, lat) = crate::mvt::lon_lat(*p);
        (lon.to_radians(), lat.to_radians())
    };
    l.windows(2)
        .map(|w| {
            let ((x1, y1), (x2, y2)) = (ll(&w[0]), ll(&w[1]));
            let (s1, s2) = (libm::sin((y2 - y1) / 2.0), libm::sin((x2 - x1) / 2.0));
            let a = s1 * s1 + libm::cos(y1) * libm::cos(y2) * s2 * s2;
            2.0 * 6_371_008.8 * libm::asin(a.sqrt())
        })
        .sum()
}

/// The area of polygons in square meters of the Mercator projection (the schema's `way_area`).
fn area_m2(ps: &[Vec<Vec<P>>]) -> f64 {
    ps.iter().map(|rings| ring_area(&rings[0]).abs() - rings[1..].iter().map(|r| ring_area(r).abs()).sum::<f64>()).sum::<f64>() * WORLD_M * WORLD_M
}

/// A label point of polygons: the inner point of the largest polygon (geo's `InteriorPoint`).
fn label_point(ps: &[Vec<Vec<P>>]) -> Option<P> {
    use geo::InteriorPoint;
    let largest = ps.iter().max_by(|a, b| ring_area(&a[0]).abs().total_cmp(&ring_area(&b[0]).abs()))?;
    let ls = |r: &Vec<P>| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
    let poly = geo::Polygon::new(ls(&largest[0]), largest[1..].iter().map(ls).collect());
    poly.interior_point().map(|p| [p.x(), p.y()])
}

fn tunnel(t: &Tags) -> bool {
    matches!(t.get("tunnel"), "yes" | "building_passage") || t.get("covered") == "yes"
}

fn bridge(t: &Tags) -> bool {
    matches!(t.get("bridge"), "yes" | "viaduct" | "boardwalk" | "cantilever" | "covered" | "low_water_crossing" | "movable" | "trestle")
}

/// The OSM layer tag as a number from -7 to 7 (0 when missing).
fn layer_num(t: &Tags) -> i64 {
    t.get("layer").trim().parse::<f64>().map_or(0, |v| v.clamp(-7.0, 7.0) as i64)
}

/// Tilemaker's z-order of a street: the layer, a bridge or a tunnel, then the class of the road.
fn z_order(t: &Tags, rail: bool, ignore_bridge: bool) -> i64 {
    const STEP: i64 = 14;
    let mut z = 0;
    if !ignore_bridge && bridge(t) {
        z += STEP;
    } else if tunnel(t) {
        z -= STEP;
    }
    if !t.get("layer").is_empty() {
        z += layer_num(t) * STEP;
    }
    let (h, r) = (t.get("highway"), t.get("railway"));
    z + match () {
        _ if rail && r == "rail" && !t.has("service") => 13,
        _ if rail && r == "rail" => 12,
        _ if rail && matches!(r, "subway" | "light_rail" | "tram" | "funicular" | "monorail") => 11,
        _ => match h {
            "motorway" => 10,
            "trunk" => 9,
            "primary" => 8,
            "secondary" => 7,
            "tertiary" => 6,
            "unclassified" | "residential" | "motorway_link" | "trunk_link" | "primary_link" | "secondary_link" | "tertiary_link" | "busway"
            | "bus_guideway" => 5,
            "living_street" | "pedestrian" => 4,
            "service" => 3,
            "footway" | "bridleway" | "cycleway" | "path" | "track" => 2,
            "steps" => 1,
            _ => 0,
        },
    }
}

/// The order key of a number, largest first.
fn desc(v: f64) -> i64 {
    -crate::quack::ordered(v)
}

const POI_AMENITY: &[&str] = &[
    "arts_centre", "atm", "bank", "bar", "bench", "bicycle_rental", "biergarten", "cafe", "car_rental", "car_sharing", "car_wash",
    "cinema", "clinic", "college", "community_centre", "courthouse", "dentist", "doctors", "dog_park", "drinking_water", "embassy",
    "fast_food", "fire_station", "food_court", "fountain", "grave_yard", "hospital", "hunting_stand", "library", "marketplace",
    "nightclub", "nursing_home", "pharmacy", "place_of_worship", "playground", "police", "post_box", "post_office", "prison", "pub",
    "public_building", "recycling", "restaurant", "school", "shelter", "telephone", "theatre", "toilets", "townhall", "university",
    "vending_machine", "veterinary", "waste_basket",
];
const POI_EMERGENCY: &[&str] = &["defibrillator", "fire_hydrant", "phone"];
const POI_HIGHWAY: &[&str] = &["emergency_access_point"];
const POI_HISTORIC: &[&str] =
    &["archaeological_site", "battlefield", "castle", "fort", "memorial", "monument", "ruins", "wayside_cross", "wayside_shrine"];
const POI_LEISURE: &[&str] = &["golf_course", "ice_rink", "pitch", "sports_centre", "stadium", "swimming_pool", "water_park"];
const POI_MAN_MADE: &[&str] =
    &["lighthouse", "surveillance", "tower", "wastewater_plant", "water_well", "water_works", "watermill", "windmill"];
const POI_OFFICE: &[&str] = &["diplomatic"];
const POI_SHOP: &[&str] = &[
    "alcohol", "bakery", "beauty", "beverages", "bicycle", "books", "butcher", "car", "chemist", "clothes", "computer", "convenience",
    "department_store", "doityourself", "dry_cleaning", "florist", "furniture", "garden_centre", "general", "gift", "greengrocer",
    "hairdresser", "hardware", "jewelry", "kiosk", "laundry", "mall", "mobile_phone", "newsagent", "optician", "outdoor", "shoes",
    "sports", "stationery", "supermarket", "toys", "travel_agency", "video",
];
const POI_TOURISM: &[&str] = &[
    "artwork", "alpine_hut", "bed_and_breakfast", "camp_site", "caravan_site", "chalet", "guest_house", "hostel", "hotel",
    "information", "motel", "picnic_site", "theme_park", "viewpoint", "zoo",
];

/// The pois layer: a point with a tag of the schema's list (true), else nothing (false).
fn pois(m: &mut Make, at: P) -> bool {
    let keys: [(&'static str, &[&str]); 9] = [
        ("amenity", POI_AMENITY),
        ("shop", POI_SHOP),
        ("tourism", POI_TOURISM),
        ("man_made", POI_MAN_MADE),
        ("historic", POI_HISTORIC),
        ("leisure", POI_LEISURE),
        ("emergency", POI_EMERGENCY),
        ("highway", POI_HIGHWAY),
        ("office", POI_OFFICE),
    ];
    let found: Vec<(&'static str, String)> =
        keys.iter().filter(|(k, list)| list.contains(&m.t.get(k))).map(|(k, _)| (*k, m.t.get(k).to_string())).collect();
    if found.is_empty() {
        return false;
    }
    let t = Tags(m.t.0);
    let o = m.add(POIS, 14, Shape::Points(vec![at]));
    for (k, v) in &found {
        o.s(k, v);
    }
    let is = |k: &str, vs: &[&str]| vs.contains(&t.get(k));
    let opt = |o: &mut Out, k: &'static str| {
        if t.has(k) {
            o.s(k, t.get(k));
        }
    };
    if is("amenity", &["restaurant", "fast_food", "pub", "bar", "cafe"]) {
        opt(o, "cuisine");
    }
    if is("leisure", &["pitch", "sports_centre"]) {
        opt(o, "sport");
    }
    if is("amenity", &["vending_machine"]) {
        opt(o, "vending");
    }
    if is("tourism", &["information"]) {
        opt(o, "information");
    }
    if is("man_made", &["tower"]) {
        opt(o, "tower:type");
    }
    if is("amenity", &["place_of_worship"]) {
        opt(o, "religion");
        opt(o, "denomination");
    }
    if is("amenity", &["recycling"]) {
        for k in ["recycling:glass_bottles", "recycling:paper", "recycling:clothes", "recycling:scrap_metal"] {
            o.b(k, t.get(k) == "yes");
        }
    }
    if is("amenity", &["bank"]) {
        o.b("atm", t.get("atm") == "yes");
    }
    o.names(&t);
    for (k, tag) in [("housename", "addr:housename"), ("housenumber", "addr:housenumber")] {
        if t.has(tag) {
            o.s(k, t.get(tag));
        }
    }
    true
}

fn addresses(m: &mut Make, at: P) {
    let (name, number) = (m.t.get("addr:housename").to_string(), m.t.get("addr:housenumber").to_string());
    if name.is_empty() && number.is_empty() {
        return;
    }
    m.add(ADDRESSES, 14, Shape::Points(vec![at])).s("housename", &name).s("housenumber", &number);
}

fn public_transport(m: &mut Make, at: P) {
    let t = Tags(m.t.0);
    let (kind, z) = match (t.get("railway"), t.get("highway"), t.get("amenity"), t.get("aerialway"), t.get("aeroway")) {
        ("station" | "halt", ..) => (t.get("railway"), 13),
        ("tram_stop", ..) => ("tram_stop", 14),
        (_, "bus_stop", ..) => ("bus_stop", 14),
        (_, _, "bus_station", ..) => ("bus_station", 13),
        (_, _, "ferry_terminal", ..) => ("ferry_terminal", 12),
        (_, _, _, "station", _) => ("aerialway_station", 13),
        (.., "aerodrome") => ("aerodrome", 11),
        (.., "helipad") => ("helipad", 13),
        _ => return,
    };
    let o = m.add(PUBLIC_TRANSPORT, z, Shape::Points(vec![at]));
    o.s("kind", kind);
    if t.has("iata") {
        o.s("iata", t.get("iata"));
    }
    o.names(&t);
}

/// The place_labels layer of a node.
fn place(m: &mut Make, at: P) {
    let t = Tags(m.t.0);
    let p = t.get("place");
    let (mut z, default) = match p {
        "city" => (6, 100_000),
        "town" => (7, 5000),
        "village" => (10, 100),
        "hamlet" => (10, 50),
        "suburb" => (10, 1000),
        "quarter" => (10, 500),
        "neighbourhood" => (10, 100),
        "locality" | "island" => (10, 0),
        "isolated_dwelling" | "farm" => (10, 5),
        _ => return,
    };
    let mut kind = p;
    if matches!(p, "city" | "town" | "village" | "hamlet") {
        match t.get("capital") {
            "yes" => (z, kind) = (4, "capital"),
            "4" => (z, kind) = (4, "state_capital"),
            _ => {}
        }
    }
    let population = t.get("population").trim().parse::<i64>().unwrap_or(default);
    m.add(PLACE_LABELS, z, Shape::Points(vec![at])).s("kind", kind).names(&t).i("population", population).sort(-population);
}

/// The layers of a node.
fn node(m: &mut Make, at: P) {
    let t = Tags(m.t.0);
    if t.has("place") && t.has("name") {
        place(m, at);
    }
    if t.get("highway") == "motorway_junction" {
        m.add(STREET_LABELS_POINTS, 12, Shape::Points(vec![at])).s("kind", "motorway_junction").names(&t).s("ref", t.get("ref"));
    }
    public_transport(m, at);
    if pois(m, at) {
        return;
    }
    addresses(m, at);
}

/// The water_polygons, water_polygons_labels and dam_polygons layers of an area.
fn water_polygons(m: &mut Make, ps: &[Vec<Vec<P>>]) {
    let t = Tags(m.t.0);
    let (natural, waterway, landuse) = (t.get("natural"), t.get("waterway"), t.get("landuse"));
    let river = (natural == "water" && t.get("water") == "river") || waterway == "riverbank";
    let (kind, first) = if matches!(landuse, "reservoir" | "basin") {
        (landuse, 4)
    } else if natural == "glacier" || (natural == "water" && !river) {
        (natural, 4)
    } else if river {
        ("river", 4)
    } else if matches!(waterway, "dock" | "canal") {
        (waterway, 10)
    } else {
        ("", 0)
    };
    if !kind.is_empty() {
        let area = area_m2(ps);
        let z = zmin_for_area(1.0, area).clamp(first, 14) as u8;
        m.add(WATER_POLYGONS, z, Shape::Polygons(ps.to_vec())).s("kind", kind).f("way_area", area).sort(crate::quack::ordered(area));
        if t.has("name") {
            if let Some(at) = label_point(ps) {
                let z = zmin_for_area(100.0, area).clamp(first, 14) as u8;
                m.add(WATER_POLYGONS_LABELS, z, Shape::Points(vec![at])).s("kind", kind).f("way_area", area).names(&t).sort(desc(area));
            }
        }
    }
    if waterway == "dam" {
        m.add(DAM_POLYGONS, 12, Shape::Polygons(ps.to_vec())).s("kind", "dam");
    }
}

/// The water_lines, water_lines_labels and dam_lines layers of a line.
fn water_lines(m: &mut Make, l: &[P]) {
    let t = Tags(m.t.0);
    let kind = t.get("waterway");
    let (z, zl) = match kind {
        "river" | "canal" => {
            let z = zmin_for_length(0.25, length_m(l));
            (z.max(9), z.max(12))
        }
        "stream" | "ditch" => (14, 14),
        "dam" => {
            m.add(DAM_LINES, 12, Shape::Lines(vec![l.to_vec()])).s("kind", "dam");
            return;
        }
        _ => return,
    };
    let (tu, br, layer) = (tunnel(&t), bridge(&t), layer_num(&t));
    let line = Shape::Lines(vec![l.to_vec()]);
    if z <= 14 {
        m.add(WATER_LINES, z as u8, line.clone()).s("kind", kind).b("tunnel", tu).b("bridge", br).sort(layer);
    }
    if t.has("name") && zl <= 14 {
        m.add(WATER_LINES_LABELS, zl as u8, line).s("kind", kind).names(&t).b("tunnel", tu).b("bridge", br).sort(layer);
    }
}

fn land(m: &mut Make, ps: &[Vec<Vec<P>>]) {
    let t = Tags(m.t.0);
    let (landuse, natural, wetland, leisure) = (t.get("landuse"), t.get("natural"), t.get("wetland"), t.get("leisure"));
    let (kind, z) = if landuse == "forest" || natural == "wood" {
        ("forest", 7)
    } else if matches!(
        landuse,
        "residential" | "industrial" | "commercial" | "retail" | "railway" | "landfill" | "brownfield" | "greenfield" | "farmyard" | "farmland" | "garages"
    ) {
        (landuse, 10)
    } else if matches!(
        landuse,
        "grass" | "meadow" | "orchard" | "vineyard" | "allotments" | "village_green" | "recreation_ground" | "greenhouse_horticulture" | "plant_nursery" | "quarry"
    ) {
        (landuse, 11)
    } else if matches!(natural, "sand" | "beach") {
        (natural, 10)
    } else if matches!(natural, "heath" | "scrub" | "grassland" | "bare_rock" | "scree" | "shingle") {
        (natural, 11)
    } else if matches!(wetland, "swamp" | "bog" | "string_bog" | "wet_meadow" | "marsh") {
        (wetland, 11)
    } else if t.get("amenity") == "grave_yard" {
        ("grave_yard", 13)
    } else if matches!(leisure, "golf_course" | "park" | "garden" | "playground" | "miniature_golf") {
        (leisure, 11)
    } else if landuse == "cemetery" {
        ("cemetery", 13)
    } else {
        return;
    };
    m.add(LAND, z, Shape::Polygons(ps.to_vec())).s("kind", kind);
}

fn sites(m: &mut Make, ps: &[Vec<Vec<P>>]) {
    let t = Tags(m.t.0);
    let kind = match (t.get("amenity"), t.get("leisure"), t.get("landuse"), t.get("military")) {
        (a @ ("university" | "hospital" | "prison" | "parking" | "bicycle_parking" | "school" | "college"), ..) => a,
        (_, "sports_centre", ..) => "sports_centre",
        (_, _, "construction", _) => "construction",
        (.., "danger_area") => "danger_area",
        _ => return,
    };
    m.add(SITES, 14, Shape::Polygons(ps.to_vec())).s("kind", kind);
}

fn streets(m: &mut Make, l: &[P]) {
    let t = Tags(m.t.0);
    let (highway, railway, aeroway, service) = (t.get("highway"), t.get("railway"), t.get("aeroway"), t.get("service"));
    let (kind, z, rail) = match highway {
        "motorway" | "motorway_link" => ("motorway", 5, false),
        "trunk" | "trunk_link" => ("trunk", 6, false),
        "primary" | "primary_link" => ("primary", 8, false),
        "secondary" | "secondary_link" => ("secondary", 9, false),
        "tertiary" | "tertiary_link" => ("tertiary", 10, false),
        "unclassified" | "residential" | "bus_guideway" | "busway" => (highway, 12, false),
        "living_street" | "pedestrian" | "track" | "service" | "footway" | "steps" | "path" | "cycleway" => (highway, 13, false),
        "" => match (railway, aeroway) {
            ("rail" | "narrow_gauge", _) => (railway, if service.is_empty() { 8 } else { 10 }, true),
            ("light_rail" | "tram" | "subway" | "funicular" | "monorail", _) => (railway, 10, true),
            (_, "runway") => ("runway", 11, false),
            (_, "taxiway") => ("taxiway", 13, false),
            _ => return,
        },
        _ => return,
    };
    let link = matches!(highway, "motorway_link" | "trunk_link" | "primary_link" | "secondary_link" | "tertiary_link");
    let oneway = t.get("oneway");
    let o = m.add(STREETS, z, Shape::Lines(vec![l.to_vec()]));
    o.s("kind", kind).b("link", link).from(11).s("surface", t.get("surface")).from(11);
    o.b("tunnel", tunnel(&t)).from(11).b("bridge", bridge(&t)).from(11);
    o.b("oneway", !rail && matches!(oneway, "yes" | "1" | "true" | "-1")).from(14);
    o.b("oneway_reverse", !rail && oneway == "-1").from(14);
    if t.has("tracktype") {
        o.s("tracktype", t.get("tracktype")).from(11);
    }
    o.b("rail", rail);
    if !service.is_empty() {
        o.s("service", service).from(11);
    }
    o.s("bicycle", t.get("bicycle")).from(14).s("horse", t.get("horse")).from(14);
    o.sort(z_order(&t, rail, false));
}

fn street_labels(m: &mut Make, l: &[P]) {
    let t = Tags(m.t.0);
    let (highway, railway, aeroway) = (t.get("highway"), t.get("railway"), t.get("aeroway"));
    let (kind, z) = match highway {
        "motorway" => (highway, 10),
        "trunk" | "primary" => (highway, 12),
        "secondary" | "tertiary" | "motorway_link" | "trunk_link" | "primary_link" | "secondary_link" => (highway, 13),
        "tertiary_link" | "unclassified" | "residential" | "busway" | "bus_guideway" | "living_street" | "pedestrian" | "track"
        | "service" | "footway" | "steps" | "path" | "cycleway" => (highway, 14),
        "" => match (railway, aeroway) {
            ("rail" | "narrow_gauge" | "tram" | "light_rail" | "funicular" | "subway" | "monorail", _) => (railway, 10),
            (_, "runway") => ("runway", 11),
            (_, "taxiway") => ("taxiway", 13),
            _ => return,
        },
        _ => return,
    };
    let refs: Vec<&str> = t.get("ref").split(';').filter(|s| !s.is_empty()).collect();
    if !t.has("name") && refs.is_empty() {
        return;
    }
    let cols = refs.iter().map(|r| r.chars().count()).max().unwrap_or(0);
    let o = m.add(STREET_LABELS, z, Shape::Lines(vec![l.to_vec()]));
    o.s("kind", kind).b("tunnel", tunnel(&t)).s("ref", &refs.join("\n")).i("ref_rows", refs.len() as i64).i("ref_cols", cols as i64);
    o.names(&t).sort(z_order(&t, false, true));
}

fn street_polygons(m: &mut Make, ps: &[Vec<Vec<P>>]) {
    let t = Tags(m.t.0);
    let (kind, z) = match (t.get("highway"), t.get("area:aeroway")) {
        (h @ ("pedestrian" | "service"), _) => (h, 14),
        (_, "runway") => ("runway", 11),
        (_, "taxiway") => ("taxiway", 13),
        _ => return,
    };
    let o = m.add(STREET_POLYGONS, z, Shape::Polygons(ps.to_vec()));
    o.s("kind", kind).b("bridge", bridge(&t)).b("rail", false).s("service", t.get("service")).s("surface", t.get("surface"));
    o.b("tunnel", tunnel(&t)).sort(z_order(&t, false, false));
    if t.has("name") {
        if let Some(at) = label_point(ps) {
            m.add(STREETS_POLYGONS_LABELS, 14, Shape::Points(vec![at])).s("kind", kind).names(&t);
        }
    }
}

/// The boundary_labels layer of an administrative boundary of level 2 or 4 (a relation): a label point. Its lines are
/// in [`boundary_lines`].
fn boundary_labels(m: &mut Make, ps: &[Vec<Vec<P>>]) {
    let t = Tags(m.t.0);
    if t.get("boundary") != "administrative" {
        return;
    }
    let level = match t.get("admin_level") {
        "2" => 2,
        "4" => 4,
        _ => return,
    };
    let area = area_m2(ps);
    let km2 = area / 1e6;
    let z = match () {
        _ if level == 2 && km2 >= 2e6 => 2,
        _ if km2 >= 7e5 => 3,
        _ if km2 >= 1e5 => 4,
        _ => 5,
    };
    if let Some(at) = label_point(ps) {
        m.add(BOUNDARY_LABELS, z, Shape::Points(vec![at])).i("admin_level", level).f("way_area", area / 1e4).names(&t).sort(desc(area));
    }
}

/// The boundaries layer: each member way of a boundary relation (`type=boundary`) of `boundary=administrative` with
/// `admin_level` 2 to 4, or of `boundary=disputed` (with no admin_level or 2 to 4), as a line with the lowest admin
/// level of its relations. A way with a `type` tag is left out (Tilemaker). The member ways often have no tags, so the
/// quack engine does not give them: three reads of the file (`open`) find the relations, their ways and the nodes of the
/// ways. Only these ways and nodes stay in memory.
pub fn boundary_lines<Rd: std::io::Read>(open: impl Fn() -> Result<Rd, String>) -> Result<Vec<(i64, Out)>, String> {
    use crate::graph::MemberType;
    use crate::pbfraw::{decode_block_tags, read_block, Buffers};
    use crate::raw::RawSink;
    use crate::util::FxHashMap;
    type T<'a, 'b> = &'b mut dyn Iterator<Item = (&'a str, &'a str)>;
    #[derive(Default)]
    struct Pass {
        /// way -> (lowest admin level, a member of a disputed boundary)
        ways: FxHashMap<i64, (i64, bool)>,
        /// way -> (refs, maritime, disputed=yes)
        lines: FxHashMap<i64, (Vec<i64>, bool, bool)>,
        nodes: FxHashMap<i64, P>,
    }
    impl RawSink for Pass {
        fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, _: T<'a, '_>) {
            if let Some(p) = self.nodes.get_mut(&id) {
                *p = mercator(lon, lat);
            }
        }
        fn way<'a>(&mut self, id: i64, tags: T<'a, '_>, refs: &mut dyn Iterator<Item = i64>) {
            if !self.ways.contains_key(&id) {
                return;
            }
            let (mut maritime, mut disputed) = (false, false);
            for (k, v) in tags {
                match (k, v) {
                    ("type", _) => return,
                    ("maritime", "yes") | ("natural", "coastline") => maritime = true,
                    ("disputed", "yes") => disputed = true,
                    _ => {}
                }
            }
            self.lines.insert(id, (refs.collect(), maritime, disputed));
        }
        fn relation<'a>(&mut self, _: i64, tags: T<'a, '_>, members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>) {
            let (mut kind, mut boundary, mut level) = ("", "", None);
            for (k, v) in tags {
                match k {
                    "type" => kind = v,
                    "boundary" => boundary = v,
                    "admin_level" => level = Some(v.trim().parse::<i64>().unwrap_or(99)),
                    _ => {}
                }
            }
            let in_range = |l: Option<i64>| l.is_some_and(|l| (2..=4).contains(&l));
            let (level, disputed) = match boundary {
                _ if kind != "boundary" => return,
                "administrative" if in_range(level) => (level.unwrap(), false),
                "disputed" if level.is_none() || in_range(level) => (99, true),
                _ => return,
            };
            for (id, t, _) in members {
                if t == MemberType::Way {
                    let e = self.ways.entry(id).or_insert((99, false));
                    *e = (e.0.min(level), e.1 || disputed);
                }
            }
        }
    }
    let read = |pass: &mut Pass, keep: [bool; 3], tags: [bool; 3]| -> Result<(), String> {
        let mut r = open()?;
        let mut buf = Buffers::default();
        while let Some(b) = read_block(&mut r)? {
            decode_block_tags(&b, keep, tags, &mut buf, pass)?;
        }
        Ok(())
    };
    let mut pass = Pass::default();
    read(&mut pass, [false, false, true], [false, false, true])?;
    pass.ways.retain(|_, w| w.0 <= 4);
    if pass.ways.is_empty() {
        return Ok(Vec::new());
    }
    read(&mut pass, [false, true, false], [false, true, false])?;
    for (refs, ..) in pass.lines.values() {
        for &n in refs {
            pass.nodes.insert(n, [f64::NAN, f64::NAN]);
        }
    }
    read(&mut pass, [true, false, false], [false, false, false])?;
    let mut ids: Vec<&i64> = pass.lines.keys().collect();
    ids.sort();
    let mut out = Vec::new();
    for id in ids {
        let (refs, maritime, disputed) = &pass.lines[id];
        let (level, member_disputed) = pass.ways[id];
        let line: Vec<P> = refs.iter().filter_map(|n| pass.nodes.get(n)).filter(|p| !p[0].is_nan()).copied().collect();
        if line.len() < 2 {
            continue;
        }
        let mut o = Out { layer: BOUNDARIES, minzoom: if level == 2 { 0 } else { 7 }, sort: 0, attrs: Vec::new(), shape: Shape::Lines(vec![line]) };
        o.i("admin_level", level).b("maritime", *maritime).b("disputed", *disputed || member_disputed);
        out.push((*id, o));
    }
    Ok(out)
}

/// The Shortbread features of a feature of the quack engine (`feature_id` "node/1", "way/2" or "relation/3", its tags
/// and its WKB in longitude and latitude).
pub fn features(feature_id: &str, tags: &RawTags, wkb: &[u8]) -> Vec<Out> {
    let mut m = Make { t: Tags(tags), out: Vec::new() };
    let Some(g) = geom(wkb) else { return m.out };
    let relation = feature_id.starts_with("relation/");
    let t = Tags(tags);
    match g {
        Geom::Point(p) => node(&mut m, p),
        g => {
            // Tilemaker's area test: a closed way is an area unless area=no; a multipolygon or a boundary is an area. The
            // quack engine (as QuackOSM) has no tag `area`: a way with `highway` is a polygon only with area=yes.
            let way_polygon = !relation && matches!(g, Geom::Polygons(_));
            let explicit = t.get("area") == "yes"
                || (way_polygon && t.has("highway"))
                || relation
                || matches!(t.get("type"), "multipolygon" | "boundary")
                || matches!(t.get("area:aeroway"), "runway" | "taxiway");
            let (line, polys): (Option<Vec<P>>, Option<Vec<Vec<Vec<P>>>>) = match g {
                Geom::Line(l) => {
                    let closed = l.len() >= 4 && l.first() == l.last();
                    let poly = (closed && (explicit || t.get("area") != "no")).then(|| vec![vec![l.clone()]]);
                    (Some(l), poly)
                }
                Geom::Polygons(ps) => {
                    let line = (!relation && ps.len() == 1).then(|| ps[0][0].clone());
                    (line, Some(ps))
                }
                Geom::Point(_) => unreachable!(),
            };
            let area = polys.is_some();
            if let Some(ps) = &polys {
                if t.has("waterway") || t.has("natural") || t.has("landuse") {
                    water_polygons(&mut m, ps);
                }
                if matches!(t.get("man_made"), "pier" | "breakwater" | "groyne") {
                    let kind = t.get("man_made");
                    m.add(PIER_POLYGONS, 12, Shape::Polygons(ps.clone())).s("kind", kind);
                }
                if t.get("man_made") == "bridge" {
                    m.add(BRIDGES, 12, Shape::Polygons(ps.clone())).s("kind", "bridge");
                }
                land(&mut m, ps);
                sites(&mut m, ps);
                if relation {
                    boundary_labels(&mut m, ps);
                }
                if t.has("building") && t.get("building") != "no" {
                    m.add(BUILDINGS, 14, Shape::Polygons(ps.clone())).i("dummy", 1);
                }
            }
            if let Some(l) = &line {
                if !area && t.has("waterway") {
                    water_lines(&mut m, l);
                }
                if !area && matches!(t.get("man_made"), "pier" | "breakwater" | "groyne") {
                    let kind = t.get("man_made");
                    m.add(PIER_LINES, 12, Shape::Lines(vec![l.clone()])).s("kind", kind);
                }
                if !explicit && (t.has("highway") || t.has("railway") || t.has("aeroway")) {
                    streets(&mut m, l);
                    street_labels(&mut m, l);
                }
                if matches!(
                    t.get("aerialway"),
                    "cable_car" | "gondola" | "goods" | "chair_lift" | "drag_lift" | "t-bar" | "j-bar" | "platter" | "rope_tow"
                ) {
                    let kind = t.get("aerialway");
                    m.add(AERIALWAYS, 12, Shape::Lines(vec![l.clone()])).s("kind", kind);
                }
                if t.get("route") == "ferry" {
                    let z = if t.get("motor_vehicle") == "no" { 12 } else { 10 };
                    m.add(FERRIES, z, Shape::Lines(vec![l.clone()])).s("kind", "ferry").names(&t);
                }
            }
            if let Some(ps) = &polys {
                if explicit && (t.has("highway") || t.has("area:aeroway")) {
                    street_polygons(&mut m, ps);
                }
                if let Some(at) = label_point(ps) {
                    public_transport(&mut m, at);
                    if !pois(&mut m, at) {
                        addresses(&mut m, at);
                    }
                }
            }
        }
    }
    m.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quack::Geom;

    fn tags(t: &[(&str, &str)]) -> RawTags {
        t.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_street_and_a_poi() {
        let line = Geom::Line(vec![(7.42, 43.73), (7.43, 43.74)]).wkb();
        let out = features("way/1", &tags(&[("highway", "residential"), ("name", "Rue"), ("oneway", "-1")]), &line);
        let layers: Vec<&str> = out.iter().map(|o| LAYERS[o.layer as usize].name).collect();
        assert_eq!(layers, ["streets", "street_labels"]);
        assert_eq!(out[0].minzoom, 12);
        assert!(out[0].attrs.contains(&("oneway_reverse", Value::Bool(true), 14)));
        let pt = Geom::Point((7.42, 43.73)).wkb();
        let out = features("node/1", &tags(&[("amenity", "cafe"), ("cuisine", "coffee"), ("addr:housenumber", "3")]), &pt);
        assert_eq!(out.len(), 1, "a poi is not also an address");
        assert!(out[0].attrs.contains(&("cuisine", Value::Str("coffee".into()), 0)));
    }
}
