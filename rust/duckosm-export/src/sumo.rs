//! The SUMO export, the port of `to_sumo` of src/duckosm/sumo.py for the command `sumo`: the plain XML (nodes, edges with
//! `id` = `edge_id`, the legal connections of `edge_graph`), the netconvert config (`.netccfg`), and netconvert.
//!
//! `to_sumo_with` also takes the options of the Python API that GMNS uses (`gmns_sumo`): the bus-only edges, each edge's lanes
//! (with their turn arrows, which netconvert's OSM import applies), attributes of an edge and more netconvert options. With
//! `junctions.join` set, netconvert names the joins and this export merges those nodes itself ([`merge_joined`]).

use crate::stream::{cast_col, each_chunk};
use crate::{has_table, open, quoteattr, Out, Result};
use duckdb::arrow::array::{Array, AsArray};
use duckdb::arrow::datatypes::Float64Type;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The default options of netconvert (`DEFAULT_NETCFG`): the ids and the topology of duckOSM stay as they are.
const DEFAULT_NETCFG: [(&str, &str); 6] = [
    ("proj.utm", "true"),
    ("geometry.remove", "false"),
    ("junctions.join", "false"),
    ("no-turnarounds.except-deadend", "true"),
    ("numerical-ids", "false"),
    // OSM has no signal timings: a program that follows the traffic (SUMO's advice for OSM)
    ("tls.default-type", "actuated"),
];

use duckosm_core::chains::NODE_TYPES_SQL;

/// The bus stops (`.add.xml`), as `BUS_STOPS_SQL` of sumo.py (the same text).
const BUS_STOPS_SQL: &str = r#"WITH p AS (SELECT osm_id, lon, lat, tags['name'] AS name, tags['public_transport'] = 'stop_position' AS pos FROM raw.nodes
    WHERE tags['highway'] = 'bus_stop' OR tags['public_transport'] = 'platform' OR (tags['public_transport'] = 'stop_position' AND tags['bus'] = 'yes')),
s AS (SELECT * FROM p WHERE NOT pos OR NOT EXISTS (SELECT 1 FROM p q WHERE NOT q.pos
    AND 111320 * sqrt(pow(q.lat - p.lat, 2) + pow((q.lon - p.lon) * cos(radians(p.lat)), 2)) <= 30)),
e AS (SELECT edge_id, geometry, length_m FROM {mode}.edges WHERE edge_id IN (SELECT edge_id FROM bus.route_edges)),
c AS (SELECT s.osm_id, s.name, e.edge_id, e.length_m, s.lon, s.lat, e.geometry, ST_LineLocatePoint(e.geometry, ST_Point(s.lon, s.lat)) AS f
    FROM s JOIN e ON ST_DWithin(e.geometry, ST_Point(s.lon, s.lat), 0.0005)),
d AS (SELECT *, ST_LineInterpolatePoint(geometry, f) AS q, ST_LineInterpolatePoint(geometry, greatest(f - 0.01, 0)) AS a,
    ST_LineInterpolatePoint(geometry, least(f + 0.01, 1)) AS b FROM c),
k AS (SELECT osm_id, name, edge_id, f * length_m AS pos_m, length_m,
    111320 * sqrt(pow(ST_Y(q) - lat, 2) + pow((ST_X(q) - lon) * cos(radians(lat)), 2)) AS dist,
    (ST_X(b) - ST_X(a)) * (lat - ST_Y(a)) - (ST_Y(b) - ST_Y(a)) * (lon - ST_X(a)) AS side FROM d),
best AS (SELECT * FROM k WHERE dist <= 30 QUALIFY row_number() OVER (PARTITION BY osm_id ORDER BY side < 0 AND dist > 0.5 DESC, dist, edge_id) = 1)
SELECT b.osm_id, b.name, b.edge_id,
    (SELECT count(*) FROM {lanes} l WHERE l.edge_id = b.edge_id AND l.lane_num > (SELECT max(m.lane_num) FROM {lanes} m
        WHERE m.edge_id = b.edge_id AND m.use <> 'bike'))::BIGINT AS lane,
    round(greatest(b.pos_m - 7.5, 0), 2)::DOUBLE, round(least(b.pos_m + 7.5, b.length_m), 2)::DOUBLE,
    (SELECT string_agg(DISTINCT r.ref, ' ' ORDER BY r.ref) FROM bus.route_edges r WHERE r.edge_id = b.edge_id) AS lines
FROM best b ORDER BY b.osm_id"#;

/// The SUMO node type of the code of `NODE_TYPES_SQL`, strongest first.
const NODE_TYPES: [&str; 3] = ["traffic_light", "allway_stop", "priority_stop"];

/// The signal programs that netconvert built (the `<tlLogic>` elements of the .net.xml) as a .tll.xml, as `_write_tll`; the number
/// (none: no file).
fn write_tll(net: &Path, path: &Path) -> Result<usize> {
    let (mut out, mut n, mut inside) = (String::new(), 0, false);
    crate::each_line(net, |l| {
        let t = l.trim_start();
        if t.starts_with("<tlLogic ") {
            inside = true;
            n += 1;
        }
        if inside {
            out.push_str(l);
            out.push('\n');
        }
        if t.starts_with("</tlLogic>") {
            inside = false;
        }
        Ok(())
    })?;
    if n > 0 {
        std::fs::write(path, format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tlLogics>\n{out}</tlLogics>\n")).map_err(|x| x.to_string())?;
    }
    Ok(n)
}

fn priority(hw: Option<&str>) -> i32 {
    match hw.unwrap_or("") {
        "motorway" | "motorway_link" => 6,
        "trunk" | "trunk_link" => 5,
        "primary" | "primary_link" => 4,
        "secondary" | "secondary_link" => 3,
        "tertiary" | "tertiary_link" => 2,
        "service" => 0,
        _ => 1,
    }
}

/// The SUMO vehicle classes of an OSM vehicle list (`;` between the values), as `_classes`.
fn classes(osm: Option<&str>) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for v in osm.unwrap_or("").split(';') {
        let c = match v.trim() {
            "psv" => "bus coach taxi",
            "bus" => "bus coach",
            "taxi" => "taxi",
            "bicycle" => "bicycle",
            "hgv" => "truck trailer",
            "goods" => "delivery",
            "emergency" => "emergency",
            "motorcar" => "passenger",
            "motorcycle" => "motorcycle",
            "moped" => "moped",
            "hov" => "hov",
            "agricultural" => "agricultural",
            "motor_vehicle" => "passenger bus coach taxi truck trailer delivery motorcycle moped hov emergency evehicle",
            _ => "",
        };
        out.extend(c.split_whitespace());
    }
    out
}

/// A connection: from, to, allow (None: each vehicle), disallow.
pub type Conn = (i64, i64, Option<BTreeSet<&'static str>>, BTreeSet<&'static str>);
/// A row of `turn_permission`: from, to, allowed, vehicles, except_vehicles.
pub type Perm = (i64, i64, bool, Option<String>, Option<String>);

/// A connection as `_apply_permissions` keeps it: allow (None: each vehicle), disallow.
type ConnState = (Option<BTreeSet<&'static str>>, BTreeSet<&'static str>);

/// The turn permissions applied to the connections, as `_apply_permissions` (SUMO has no time: a ban with a condition is in
/// force). Only the turns with a permission row are in memory: the other turns of `edge_graph` go to the file as they come.
pub struct Permissions {
    /// a turn with a permission -> its connection at the end (None: no connection), and whether it stays at its place among
    /// the turns of `edge_graph`
    keyed: HashMap<(i64, i64), (Option<ConnState>, bool)>,
    /// the turns that a permission adds, in the order they are added (after the turns of `edge_graph`)
    added: Vec<(i64, i64)>,
}

impl Permissions {
    /// `perm` in its order; `in_graph`: the turns of `perm` that `edge_graph` has.
    pub fn new(perm: &[Perm], in_graph: &HashSet<(i64, i64)>) -> Permissions {
        let mut keyed: HashMap<(i64, i64), (Option<ConnState>, bool)> =
            in_graph.iter().map(|k| (*k, (Some((None, BTreeSet::new())), true))).collect();
        let mut added: Vec<(i64, i64)> = Vec::new();
        for (fe, te, allowed, veh, exc) in perm {
            let k = (*fe, *te);
            let e = keyed.entry(k).or_insert((None, false));
            if *allowed {
                // closed to all traffic, open to these
                if e.0.is_none() {
                    *e = (Some((Some(BTreeSet::new()), BTreeSet::new())), false);
                    added.push(k);
                }
                if let Some((Some(a), _)) = &mut e.0 {
                    a.extend(classes(veh.as_deref()));
                }
            } else if veh.is_none() && e.0.is_some() {
                // closed to all but `exc`
                let ex = classes(exc.as_deref());
                if exc.as_deref().is_some_and(|s| !s.is_empty()) && !ex.is_empty() {
                    e.0 = Some((Some(ex), BTreeSet::new()));
                } else {
                    *e = (None, false);
                    added.retain(|x| *x != k);
                }
            } else if let Some(c) = &mut e.0 {
                c.1.extend(classes(veh.as_deref()));
            }
        }
        Permissions { keyed, added }
    }

    fn kept(k: (i64, i64), c: &Option<ConnState>) -> Option<Conn> {
        let (a, d) = c.clone()?;
        (a.as_ref().is_none_or(|a| !a.is_empty())).then_some((k.0, k.1, a, d))
    }

    /// The connection of a turn of `edge_graph`, at its place.
    pub fn at(&self, fe: i64, te: i64) -> Option<Conn> {
        match self.keyed.get(&(fe, te)) {
            None => Some((fe, te, None, BTreeSet::new())),
            Some((c, true)) => Self::kept((fe, te), c),
            Some((_, false)) => None,
        }
    }

    /// The connections that the permissions add, after the turns of `edge_graph`.
    pub fn added(&self) -> impl Iterator<Item = Conn> + '_ {
        self.added.iter().filter_map(|k| Self::kept(*k, &self.keyed[k].0))
    }

    /// The connections of the turns with a permission (where a turn keeps no connection, nothing).
    fn all_keyed(&self) -> impl Iterator<Item = Conn> + '_ {
        self.keyed.iter().filter_map(|(k, (c, _))| Self::kept(*k, c))
    }
}

/// The connections with the turn permissions, as `_apply_permissions`: [`Permissions`] on a list of turns.
pub fn apply_permissions(cs: &[(i64, i64)], perm: &[Perm]) -> Vec<Conn> {
    let keys: HashSet<(i64, i64)> = perm.iter().map(|p| (p.0, p.1)).collect();
    let p = Permissions::new(perm, &cs.iter().copied().filter(|k| keys.contains(k)).collect());
    let mut seen = HashSet::new();
    cs.iter().filter(|k| seen.insert(**k)).filter_map(|k| p.at(k.0, k.1)).chain(p.added()).collect()
}

/// A float as Python's `str(float)`: the shortest text that reads back the same, with `.0` or an exponent as Python writes it.
pub fn py_float(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return format!("{x:?}");
    }
    let sci = format!("{x:e}"); // for example 1.25e-5
    let (m, ex) = sci.split_once('e').unwrap();
    let ex: i32 = ex.parse().unwrap();
    if (-4..16).contains(&ex) {
        let s = format!("{x}");
        if s.contains('.') {
            s
        } else {
            s + ".0"
        }
    } else {
        format!("{m}e{}{:02}", if ex < 0 { '-' } else { '+' }, ex.abs())
    }
}

/// 'LINESTRING(lon lat, ...)' -> 'lon,lat lon,lat ...', as `_linestring_points`.
pub(crate) fn shape_of(wkt: Option<&str>) -> Option<String> {
    shape(wkt)
}

fn shape(wkt: Option<&str>) -> Option<String> {
    let w = wkt?;
    let (a, b) = (w.find('(')?, w.rfind(')')?);
    let pts: Vec<String> = w[a + 1..b]
        .split(',')
        .filter_map(|p| {
            let v: Vec<&str> = p.split_whitespace().collect();
            if v.len() < 2 {
                return None;
            }
            Some(format!("{},{}", py_float(v[0].parse().ok()?), py_float(v[1].parse().ok()?)))
        })
        .collect();
    (!pts.is_empty()).then(|| pts.join(" "))
}

#[derive(Clone)]
struct Edge {
    id: i64,
    src: i64,
    tgt: i64,
    hw: Option<String>,
    name: Option<String>,
    lanes: Option<i64>,
    spd: Option<f64>,
    length: Option<f64>,
    wkt: Option<String>,
}

/// The `<edge>` element of one edge, with its `<lane>`s: `from`, `to` (a joined node has a new name), `both` (the reverse edge
/// is there), `lanes` (use, width) left to right, `extra` (attributes that GMNS sets).
fn edge_xml(r: &Edge, from: &str, to: &str, both: bool, lanes: Option<&[(String, f64)]>, extra: Option<&Vec<(String, String)>>, mode: &str) -> String {
    let path_mode = mode == "walking" || mode == "cycling";
    let mut a = vec![format!("id=\"{}\"", r.id), format!("from=\"{from}\""), format!("to=\"{to}\""), format!("priority=\"{}\"", priority(r.hw.as_deref()))];
    if let Some(l) = lanes {
        a.push(format!("numLanes=\"{}\"", l.len()));
    } else if let Some(n) = r.lanes.filter(|n| *n > 0 && !path_mode) {
        a.push(format!("numLanes=\"{n}\""));
    }
    if let Some(s) = r.spd.filter(|s| *s > 0.0) {
        a.push(format!("speed=\"{:.3}\"", s / 3.6));
    }
    if let Some(l) = r.length.filter(|l| *l > 0.0) {
        a.push(format!("length=\"{l:.2}\""));
    }
    if let Some(h) = r.hw.as_deref().filter(|h| !h.is_empty()) {
        a.push(format!("type={}", quoteattr(h)));
    }
    match mode {
        "walking" => a.push("allow=\"pedestrian\"".into()),
        "cycling" => a.push("allow=\"bicycle\"".into()),
        _ => {}
    }
    // one-way: the line is the middle of its lanes; two-way: the middle of the whole road
    a.push(if both { "spreadType=\"roadCenter\"".into() } else { "spreadType=\"center\"".into() });
    if let Some(s) = shape(r.wkt.as_deref()) {
        a.push(format!("shape=\"{s}\""));
    }
    if let Some(n) = r.name.as_deref().filter(|n| !n.is_empty()) {
        a.push(format!("name={}", quoteattr(n)));
    }
    for (k, v) in extra.into_iter().flatten() {
        a.retain(|x| !x.starts_with(&format!("{k}=")));
        a.push(format!("{k}={}", quoteattr(v)));
    }
    let Some(l) = lanes else { return format!("  <edge {}/>\n", a.join(" ")) };
    // SUMO numbers the lanes from the right
    let mut t = format!("  <edge {}>\n", a.join(" "));
    for (i, (u, w)) in l.iter().rev().enumerate() {
        let allow = match u.as_str() {
            "bus" => " allow=\"bus\"",
            "bike" => " allow=\"bicycle\"",
            "bus,bike" => " allow=\"bus bicycle\"",
            _ => "",
        };
        t.push_str(&format!("    <lane index=\"{i}\"{allow} width=\"{w:.2}\"/>\n"));
    }
    t.push_str("  </edge>\n");
    t
}

pub struct Summary {
    pub nod: PathBuf,
    pub edg: PathBuf,
    pub con: Option<PathBuf>,
    /// the bus stops for SUMO (`--additional-files`), if there are some
    pub add: Option<PathBuf>,
    /// the netconvert config: the built-in one, or the user's
    pub netccfg: Option<PathBuf>,
    pub net: Option<PathBuf>,
    /// the signal programs that netconvert built, if it ran and there are signals
    pub tll: Option<PathBuf>,
    pub n_nodes: usize,
    pub n_edges: usize,
    pub n_connections: usize,
    pub n_traffic_lights: usize,
    pub n_stop_signs: usize,
    pub n_bus_stops: usize,
}

fn e(e: duckdb::Error) -> String {
    e.to_string()
}

/// netconvert: `SUMO_HOME/bin`, the PATH, or the eclipse-sumo package of the active Python environment (`VIRTUAL_ENV`).
/// Returns the program and the SUMO_HOME that it needs.
pub(crate) fn find_netconvert() -> Result<(PathBuf, Option<PathBuf>)> {
    if let Some(h) = std::env::var_os("SUMO_HOME").map(PathBuf::from) {
        if h.join("bin/netconvert").is_file() {
            return Ok((h.join("bin/netconvert"), None));
        }
    }
    if let Some(p) = std::env::var_os("PATH") {
        if let Some(p) = std::env::split_paths(&p).map(|d| d.join("netconvert")).find(|p| p.is_file()) {
            return Ok((p, None));
        }
    }
    if let Some(v) = std::env::var_os("VIRTUAL_ENV").map(PathBuf::from) {
        for lib in std::fs::read_dir(v.join("lib")).into_iter().flatten().flatten() {
            let home = lib.path().join("site-packages/sumo");
            if home.join("bin/netconvert").is_file() {
                return Ok((home.join("bin/netconvert"), Some(home)));
            }
        }
    }
    Err("netconvert not found. Install SUMO (system package) or `pip install duckosm[sumo]` (with its environment active), or set SUMO_HOME, or run with --no-netconvert to write only the plain XML".into())
}

fn rel(p: &Path, base: &Path) -> String {
    // the files are in the folder of the config: their names
    p.strip_prefix(base).unwrap_or(p).display().to_string()
}

/// The attributes `from`, `to`, `fromLane`, `toLane` of the `<connection>` elements of a .net.xml (one element on each line).
fn net_connections(net: &Path) -> Result<Vec<(String, String, String, String)>> {
    let attr = |l: &str, k: &str| -> String {
        let key = format!(" {k}=\"");
        l.find(&key).map(|i| l[i + key.len()..].split('"').next().unwrap_or("").to_string()).unwrap_or_default()
    };
    let mut out = Vec::new();
    crate::each_line(net, |l| {
        let l = l.trim_start();
        if l.starts_with("<connection ") {
            out.push((attr(l, "from"), attr(l, "to"), attr(l, "fromLane"), attr(l, "toLane")));
        }
        Ok(())
    })?;
    Ok(out)
}

/// The options of `to_sumo` that GMNS gives.
#[derive(Default)]
pub struct SumoOptions {
    /// Driving only: also the bus-only edges (`private_edges` with `access = 'bus'`) and the buses' turns.
    pub bus_edges: bool,
    /// Each edge's lanes left to right, (use, width, turn arrow), in place of `driving.lane_profile`.
    pub lanes: Option<HashMap<i64, Vec<(String, f64, Option<String>)>>>,
    /// `{edge_id: [(attribute, value)]}` set on those edges over what duckOSM writes.
    pub edge_attrs: HashMap<i64, Vec<(String, String)>>,
    /// More netconvert options over the built-in ones (a config dict of Python).
    pub config: Vec<(String, String)>,
    /// Driving only: one SUMO edge for each chain of `driving.chains` ([`crate::sumo_merge`]).
    pub merge: bool,
}

/// Export the mode `mode` of the database `db` to SUMO in `out_dir`, as `to_sumo` with the options of the command `sumo`.
/// `config`: a .netccfg of the user (else the built-in options).
pub fn to_sumo(db: &Path, out_dir: &Path, mode: &str, net_name: &str, connections: bool, config: Option<&Path>, netconvert: bool) -> Result<Summary> {
    to_sumo_with(db, out_dir, mode, net_name, connections, config, netconvert, &SumoOptions::default())
}

/// The lane connections that netconvert's OSM import builds from the edges in `arrowed` (their lanes have turn arrows), as
/// `_arrow_connections`: `{edge_id: [(fromLane, to_edge, toLane)]}` in SUMO's lane indices, the edges in the order of `arrowed`.
#[allow(clippy::too_many_arguments)]
fn arrow_connections(
    way_of: &HashMap<i64, (i64, bool)>,
    rows: &[Edge],
    profile: &HashMap<i64, Vec<(String, f64, Option<String>)>>,
    arrowed: &[i64],
    cs: &[(i64, i64)],
    nodes: &[(i64, f64, f64)],
    out_dir: &Path,
    net_name: &str,
    run: &dyn Fn(&[String]) -> Result<()>,
    opts: &[(String, String)],
) -> Result<Vec<(i64, Vec<(i64, i64, i64)>)>> {
    // by_ends: a dict of Python (a later row takes the value, the key keeps its first place)
    let mut keys: Vec<(i64, i64, i64)> = Vec::new();
    let mut by_ends: HashMap<(i64, i64, i64), &Edge> = HashMap::new();
    // a way with all its points at one place: netconvert's OSM import discards it ("only 1 node"), and then a connection
    // of it names an edge that netconvert does not know. It stays out of this file and gets no lane connections here.
    let one_place = |r: &Edge| shape(r.wkt.as_deref()).is_some_and(|s| s.split(' ').all(|p| Some(p) == s.split(' ').next()));
    for r in rows.iter().filter(|r| r.src != r.tgt && !one_place(r)) {
        let k = (r.src, r.tgt, way_of[&r.id].0);
        if by_ends.insert(k, r).is_none() {
            keys.push(k);
        }
    }
    let mut back: HashMap<i64, i64> = HashMap::new();
    for k in &keys {
        let r = by_ends[k];
        if let Some(p) = by_ends.get(&(r.tgt, r.src, way_of[&r.id].0)) {
            if way_of[&r.id].1 && !way_of[&p.id].1 {
                back.insert(p.id, r.id);
            }
        }
    }
    let reverse: HashSet<i64> = back.values().copied().collect();
    let lane_tags = |ls: &[(String, f64, Option<String>)], sfx: &str| -> Vec<(String, String)> {
        let mut t = vec![
            (format!("lanes{sfx}"), ls.len().to_string()),
            (format!("width:lanes{sfx}"), ls.iter().map(|x| format!("{:.2}", x.1)).collect::<Vec<_>>().join("|")),
        ];
        if ls.iter().any(|x| x.2.as_deref().is_some_and(|v| !v.is_empty())) {
            t.push((format!("turn:lanes{sfx}"), ls.iter().map(|x| x.2.clone().unwrap_or_default()).collect::<Vec<_>>().join("|")));
        }
        if ls.iter().any(|x| x.0 != "auto") {
            t.push((format!("access:lanes{sfx}"), ls.iter().map(|x| if x.0 == "auto" { "yes" } else { "no" }).collect::<Vec<_>>().join("|")));
            for (k, v) in [("bus", "bus"), ("bicycle", "bike")] {
                let vals: Vec<&str> = ls.iter().map(|x| if x.0.contains(v) { "designated" } else if x.0 == "auto" { "yes" } else { "no" }).collect();
                t.push((format!("{k}:lanes{sfx}"), vals.join("|")));
            }
        }
        t
    };
    let node_ids: HashSet<i64> = nodes.iter().map(|n| n.0).collect();
    // netconvert's OSM import joins the nodes at one position: a shape point at the place of a node (or of an earlier shape
    // point) would split the way, and a connection would then name an edge that netconvert does not know. Such a point
    // stays out: this pass reads back only the lane connections, not the shapes.
    let at = |x: f64, y: f64| ((x * 1e7).round() as i64, (y * 1e7).round() as i64);
    let mut taken: HashSet<(i64, i64)> = nodes.iter().map(|n| at(n.1, n.2)).collect();
    let mut sumo_id: Vec<(String, i64)> = Vec::new();
    let mut f = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<osm version=\"0.6\">\n");
    for (n, x, y) in nodes {
        f.push_str(&format!("  <node id=\"{n}\" lat=\"{y:.7}\" lon=\"{x:.7}\"/>\n"));
    }
    let mut ways = String::new();
    let mut shape_id: i64 = 1_000_000_000_000_000;
    for k in &keys {
        let r = by_ends[k];
        if reverse.contains(&r.id) {
            continue;
        }
        let mut refs = vec![r.src];
        let pts: Vec<String> = shape(r.wkt.as_deref()).map(|s| s.split(' ').map(String::from).collect()).unwrap_or_default();
        if pts.len() > 2 {
            for pt in &pts[1..pts.len() - 1] {
                let (x, y) = pt.split_once(',').unwrap();
                if !taken.insert(at(x.parse().unwrap_or(f64::NAN), y.parse().unwrap_or(f64::NAN))) {
                    continue;
                }
                shape_id += 1;
                while node_ids.contains(&shape_id) {
                    shape_id += 1;
                }
                f.push_str(&format!("  <node id=\"{shape_id}\" lat=\"{y}\" lon=\"{x}\"/>\n"));
                refs.push(shape_id);
            }
        }
        refs.push(r.tgt);
        let mut tags: Vec<(String, String)> = vec![("highway".into(), r.hw.as_deref().unwrap_or("road").split(';').next().unwrap_or("").to_string())];
        if let Some(n) = r.name.as_deref().filter(|n| !n.is_empty()) {
            tags.push(("name".into(), n.to_string()));
        }
        if let Some(sp) = r.spd.filter(|s| *s > 0.0) {
            tags.push(("maxspeed".into(), format!("{}", sp.round_ties_even() as i64)));
        }
        sumo_id.push((r.id.to_string(), r.id));
        let empty = Vec::new();
        if let Some(b) = back.get(&r.id) {
            let (pf, pb) = (profile.get(&r.id).unwrap_or(&empty), profile.get(b).unwrap_or(&empty));
            tags.extend(lane_tags(pf, ":forward"));
            tags.extend(lane_tags(pb, ":backward"));
            tags.push(("oneway".into(), "no".into()));
            tags.push(("lanes".into(), (pf.len() + pb.len()).to_string()));
            sumo_id.push((format!("-{}", r.id), *b));
        } else {
            tags.extend(lane_tags(profile.get(&r.id).unwrap_or(&empty), ""));
            tags.push(("oneway".into(), "yes".into()));
        }
        // a later value of the same key replaces an earlier one in its place (a dict of Python)
        let mut seen: Vec<(String, String)> = Vec::new();
        for (k, v) in tags {
            match seen.iter_mut().find(|x| x.0 == k) {
                Some(x) => x.1 = v,
                None => seen.push((k, v)),
            }
        }
        ways.push_str(&format!(
            "  <way id=\"{}\">{}{}</way>\n",
            r.id,
            refs.iter().map(|n| format!("<nd ref=\"{n}\"/>")).collect::<String>(),
            seen.iter().map(|(k, v)| format!("<tag k={} v={}/>", quoteattr(k), quoteattr(v))).collect::<String>()
        ));
    }
    f.push_str(&ways);
    f.push_str("</osm>\n");
    let path = |ext: &str| out_dir.join(format!("{net_name}.{ext}"));
    let osm_path = path("osm");
    std::fs::write(&osm_path, f).map_err(|x| x.to_string())?;
    let ours: HashMap<i64, String> = sumo_id.iter().map(|(s, e)| (*e, s.clone())).collect();
    let ours_order: Vec<i64> = {
        let mut v: Vec<i64> = Vec::new();
        for (_, e) in &sumo_id {
            if !v.contains(e) {
                v.push(*e);
            }
        }
        v
    };
    let mut hws: Vec<String> = rows.iter().map(|r| r.hw.as_deref().unwrap_or("road").split(';').next().unwrap_or("").to_string()).collect::<BTreeSet<_>>().into_iter().collect();
    hws.sort();
    let mut t = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<types>\n");
    for hw in &hws {
        t.push_str(&format!("  <type id=\"highway.{hw}\" priority=\"{}\" numLanes=\"1\" speed=\"13.89\" oneway=\"false\"/>\n", priority(Some(hw))));
    }
    t.push_str("</types>\n");
    let typ_path = path("osm.typ.xml");
    std::fs::write(&typ_path, t).map_err(|x| x.to_string())?;
    let has: HashSet<i64> = cs.iter().map(|c| c.0).collect();
    let mut c = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<connections>\n");
    for (a, b) in cs {
        if let (Some(x), Some(y)) = (ours.get(a), ours.get(b)) {
            c.push_str(&format!("  <connection from=\"{x}\" to=\"{y}\"/>\n"));
        }
    }
    for e in &ours_order {
        if !has.contains(e) {
            c.push_str(&format!("  <connection from=\"{}\"/>\n", ours[e]));
        }
    }
    c.push_str("</connections>\n");
    let con_path = path("osm.con.xml");
    std::fs::write(&con_path, c).map_err(|x| x.to_string())?;
    let net_path = path("osm.net.xml");
    let mut cmd: Vec<String> = [
        "--osm-files", &osm_path.display().to_string(), "--type-files", &typ_path.display().to_string(),
        "--connection-files", &con_path.display().to_string(), "--output-file", &net_path.display().to_string(),
        "--osm.turn-lanes", "true", "--osm.lane-access", "true",
        // two OSM ways on the same nodes are two duckOSM edges: without this, netconvert keeps one of them
        "--osm.skip-duplicates-check", "true",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for (k, v) in opts {
        cmd.push(format!("--{k}"));
        cmd.push(v.clone());
    }
    run(&cmd)?;
    let by_sumo: HashMap<&str, i64> = sumo_id.iter().map(|(s, e)| (s.as_str(), *e)).collect();
    let mut got: Vec<(i64, Vec<(i64, i64, i64)>)> = arrowed.iter().filter(|e| ours.contains_key(e)).map(|e| (*e, Vec::new())).collect();
    let got_at: HashMap<i64, usize> = got.iter().enumerate().map(|(i, g)| (g.0, i)).collect();
    let attr = |l: &str, k: &str| -> Option<String> {
        let key = format!(" {k}=\"");
        l.find(&key).map(|i| l[i + key.len()..].split('"').next().unwrap_or("").to_string())
    };
    let read = crate::each_line(&net_path, |l| {
        let l = l.trim_start();
        if l.starts_with("<edge ") && attr(l, "function").as_deref() != Some("internal") {
            let id = attr(l, "id").unwrap_or_default();
            if !by_sumo.contains_key(id.as_str()) {
                return Err(format!("netconvert's OSM import built edge '{id}', not one of ours (a way it split): cannot map it back"));
            }
        }
        if l.starts_with("<connection ") {
            let from = attr(l, "from").unwrap_or_default();
            if from.starts_with(':') {
                return Ok(());
            }
            let fe = by_sumo[from.as_str()];
            if let Some(g) = got_at.get(&fe).map(|&i| &mut got[i]) {
                let to = by_sumo[attr(l, "to").unwrap_or_default().as_str()];
                g.1.push((attr(l, "fromLane").unwrap_or_default().parse().unwrap_or(0), to, attr(l, "toLane").unwrap_or_default().parse().unwrap_or(0)));
            }
        }
        Ok(())
    });
    // the files of this pass are read: they go now, not with the folder (Kentucky: some GB)
    for p in [&osm_path, &typ_path, &con_path, &net_path] {
        let _ = std::fs::remove_file(p);
    }
    read?;
    eprintln!("SUMO connections: {} edges with turn arrows connected lane by lane by netconvert's OSM import", got.len());
    Ok(split_lone_lanes(got, profile))
}

/// `_split_lone_lanes`: a lane that is the only lane of its edge going into a road with more lanes feeds all of them.
fn split_lone_lanes(got: Vec<(i64, Vec<(i64, i64, i64)>)>, profile: &HashMap<i64, Vec<(String, f64, Option<String>)>>) -> Vec<(i64, Vec<(i64, i64, i64)>)> {
    got.into_iter()
        .map(|(e, ls)| {
            let mut by_to: Vec<(i64, Vec<(i64, i64)>)> = Vec::new();
            for &(fl, te, tl) in &ls {
                match by_to.iter_mut().find(|x| x.0 == te) {
                    Some(x) => {
                        if !x.1.contains(&(fl, tl)) {
                            x.1.push((fl, tl));
                        }
                    }
                    None => by_to.push((te, vec![(fl, tl)])),
                }
            }
            let mut keep = ls.clone();
            for (te, pairs) in by_to {
                let froms: HashSet<i64> = pairs.iter().map(|p| p.0).collect();
                let n_to = profile.get(&te).map(|v| v.len()).unwrap_or(0);
                if froms.len() == 1 && n_to > pairs.len() {
                    let fl = *froms.iter().next().unwrap();
                    keep.retain(|x| x.1 != te);
                    keep.extend((0..n_to as i64).map(|j| (fl, te, j)));
                }
            }
            (e, keep)
        })
        .collect()
}

/// `to_sumo` with the options that GMNS gives (`SumoOptions`).
#[allow(clippy::too_many_arguments)]
pub fn to_sumo_with(db: &Path, out_dir: &Path, mode: &str, net_name: &str, connections: bool, config: Option<&Path>, netconvert: bool, o: &SumoOptions) -> Result<Summary> {
    let con = open(db)?;
    let chains = match o.merge {
        true if mode != "driving" => return Err(format!("merge: only the driving network has chains, not {mode}")),
        true => Some(crate::sumo_merge::Chains::read(&con, mode)?),
        false => None,
    };
    std::fs::create_dir_all(out_dir).map_err(|x| format!("{}: {x}", out_dir.display()))?;
    let path = |ext: &str| out_dir.join(format!("{net_name}.{ext}"));
    let (nod_path, edg_path, con_path, net_path) = (path("nod.xml"), path("edg.xml"), path("con.xml"), path("net.xml"));
    // the traffic lights and stop signs of OSM: node id -> (type, tl), as `_node_types`
    let mut types: HashMap<i64, (&'static str, Option<i64>)> = HashMap::new();
    if mode == "driving" && has_table(&con, "raw", "nodes")? && has_table(&con, mode, "way_nodes")? {
        let mut st = con.prepare(&NODE_TYPES_SQL.replace("{mode}", mode)).map_err(e)?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i32>(1)?, r.get::<_, Option<i64>>(2)?))).map_err(e)? {
            let (n, t, tl) = r.map_err(e)?;
            types.insert(n, (NODE_TYPES[(3 - t) as usize], tl));
        }
    }
    // (type, tl) of a node; a joined junction takes the strongest sign of its nodes
    let ntype = |id: &str| -> Option<(&'static str, Option<i64>)> {
        match id.strip_prefix("cluster_") {
            Some(m) => m
                .split('_')
                .filter_map(|n| n.parse::<i64>().ok().and_then(|n| types.get(&n).copied()))
                .min_by_key(|x| NODE_TYPES.iter().position(|t| *t == x.0)),
            None => id.parse::<i64>().ok().and_then(|n| types.get(&n).copied()),
        }
    };
    let node_xml = |n: &str, x: f64, y: f64| -> String {
        let mut a = String::new();
        if let Some((t, tl)) = ntype(n) {
            a.push_str(&format!(" type=\"{t}\""));
            if let Some(tl) = tl {
                a.push_str(&format!(" tl=\"{tl}\""));
            }
        }
        format!("  <node id=\"{n}\" x=\"{x:.7}\" y=\"{y:.7}\"{a}/>\n")
    };

    let bus = o.bus_edges && mode == "driving" && has_table(&con, "driving", "private_edges")?;
    let pin = 1099511627776i64; // 2^40: the rows of a second table come after the rows of the first
    // the nodes; then a bus-only edge's end that no driving edge reaches: its own line's end point (the starts, then the ends)
    let mut nodes_sql = format!("SELECT rowid AS k, node_id, ST_X(geom) AS x, ST_Y(geom) AS y FROM {mode}.nodes WHERE geom IS NOT NULL");
    if bus {
        nodes_sql = format!(
            "{nodes_sql} UNION ALL SELECT {pin} + min(o), id, arg_min(x, o), arg_min(y, o) FROM (\
             SELECT source AS id, ST_X(ST_StartPoint(geometry)) AS x, ST_Y(ST_StartPoint(geometry)) AS y, rowid AS o FROM driving.private_edges WHERE access = 'bus' \
             UNION ALL SELECT target, ST_X(ST_EndPoint(geometry)), ST_Y(ST_EndPoint(geometry)), {pin} + rowid FROM driving.private_edges WHERE access = 'bus') \
             WHERE id NOT IN (SELECT node_id FROM {mode}.nodes WHERE geom IS NOT NULL) GROUP BY id"
        );
    }
    let nodes_sql = format!("SELECT node_id, x, y FROM ({nodes_sql}) ORDER BY k");
    let cols = "edge_id, source, target, highway, name, lanes::BIGINT AS lanes, maxspeed_kmh::DOUBLE AS spd, length_m::DOUBLE AS len, ST_AsText(geometry) AS wkt";
    let bus_rows = if bus { format!(" UNION ALL SELECT {pin} + rowid, {cols} FROM driving.private_edges WHERE access = 'bus'") } else { String::new() };
    let rows_sql = format!("SELECT rowid AS k, {cols} FROM {mode}.edges{bus_rows}");
    // the lanes of driving.lane_profile, left to right: one <lane> each, with its vehicle classes and width (GMNS gives its own)
    let db_profile = o.lanes.is_none() && mode == "driving" && has_table(&con, "driving", "lane_profile")?;
    let profile_sql = if db_profile {
        "SELECT edge_id, list(coalesce(use, '') ORDER BY lane_num) AS uses, list(width_m::DOUBLE ORDER BY lane_num) AS widths FROM driving.lane_profile GROUP BY edge_id"
    } else {
        "SELECT NULL::BIGINT AS edge_id, NULL::VARCHAR[] AS uses, NULL::DOUBLE[] AS widths WHERE false"
    };
    // the edges in the order of the tables; `both`: the reverse edge is there (any row, also a loop)
    let edges_sql = format!(
        "WITH r AS ({rows_sql}), p AS ({profile_sql}) SELECT r.edge_id, r.source, r.target, r.highway, r.name, r.lanes, r.spd, r.len, r.wkt, \
         EXISTS (SELECT 1 FROM r r2 WHERE r2.source = r.target AND r2.target = r.source) AS both, p.uses, p.widths \
         FROM r LEFT JOIN p USING (edge_id) WHERE r.source <> r.target ORDER BY r.k"
    );
    let use_conns = connections && has_table(&con, mode, "edge_graph")?;
    if connections && !use_conns {
        eprintln!("{mode}.edge_graph absent — letting netconvert INFER connections (turn restrictions NOT enforced)");
    }
    // the turns of the mode's own edges (the column `uses`), as routed_graph; each turn once, at its first row
    let (mut pairs_sql, mut stuck_sql, mut perms) = (String::new(), String::new(), Permissions { keyed: HashMap::new(), added: Vec::new() });
    let mut prules: Vec<PathRule> = Vec::new();
    if use_conns {
        let has_uses: i64 = con
            .query_row(
                "SELECT count(*) FROM information_schema.columns WHERE table_schema = ? AND table_name = 'edge_graph' AND column_name = 'uses'",
                [mode],
                |r| r.get(0),
            )
            .map_err(e)?;
        let own = match mode {
            "driving" => "car",
            "walking" => "walk",
            "cycling" => "bike",
            _ => return Err(format!("unknown mode {mode}")),
        };
        let mut graph = if has_uses > 0 {
            format!("(SELECT rowid AS k, * FROM {mode}.edge_graph WHERE uses IN ('{own}'))")
        } else {
            format!("(SELECT rowid AS k, * FROM {mode}.edge_graph)")
        };
        if bus {
            // and the buses' turns, where a bus-only edge is in it
            let all = format!("SELECT edge_id FROM {mode}.edges UNION ALL SELECT edge_id FROM driving.private_edges WHERE access = 'bus'");
            graph = format!("(SELECT rowid AS k, * FROM {mode}.edge_graph WHERE from_edge IN ({all}) AND to_edge IN ({all}))");
        }
        let graph = format!("(SELECT k, from_edge, to_edge FROM {graph} QUALIFY row_number() OVER (PARTITION BY from_edge, to_edge ORDER BY k) = 1)");
        pairs_sql = format!("SELECT from_edge, to_edge FROM {graph} ORDER BY k");
        let has_perm = has_table(&con, mode, "turn_permission")?;
        let net = if bus {
            format!("SELECT edge_id FROM {mode}.edges UNION ALL SELECT edge_id FROM driving.private_edges WHERE access = 'bus'")
        } else {
            format!("SELECT edge_id FROM {mode}.edges")
        };
        if has_perm {
            let mut st = con
                // only the turns between edges of the network: a permission into an edge that SUMO does not have (a private
                // edge that is not for buses) would name an edge that netconvert does not know
                .prepare(&format!(
                    "SELECT from_edge, to_edge, allowed, vehicles, except_vehicles FROM {mode}.turn_permission \
                     WHERE from_edge IN ({net}) AND to_edge IN ({net})"
                ))
                .map_err(e)?;
            let perm: Vec<Perm> = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<bool>>(2)?.unwrap_or(false), r.get(3)?, r.get(4)?)))
                .map_err(e)?
                .collect::<std::result::Result<_, _>>()
                .map_err(e)?;
            let mut st = con
                .prepare(&format!(
                    "SELECT DISTINCT g.from_edge, g.to_edge FROM {graph} g JOIN {mode}.turn_permission t \
                     ON g.from_edge = t.from_edge AND g.to_edge = t.to_edge"
                ))
                .map_err(e)?;
            let in_graph: HashSet<(i64, i64)> =
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
            perms = Permissions::new(&perm, &in_graph);
        }
        // an edge with no legal successor gets a connection with only `from`: else netconvert makes its own there
        let mut froms = format!("SELECT from_edge FROM {graph} g");
        if has_perm {
            froms += &format!(
                " WHERE NOT EXISTS (SELECT 1 FROM {mode}.turn_permission t WHERE t.from_edge = g.from_edge AND t.to_edge = g.to_edge)"
            );
        }
        for c in perms.all_keyed() {
            froms += &format!(" UNION ALL SELECT {}", c.0);
        }
        stuck_sql = format!("SELECT edge_id FROM ({rows_sql}) WHERE source <> target AND edge_id NOT IN ({froms}) ORDER BY k");
        // the via-way restrictions (from edge, via edges, to edge): SUMO can give them only where the via edges go into a joined
        // junction, and the path becomes one connection
        if has_table(&con, mode, "turn_path_restrictions")? {
            let mut st = con
                .prepare(&format!(
                    "SELECT restriction_type, from_edge, via_edges, to_edge, except_vehicles, applies_to FROM {mode}.turn_path_restrictions"
                ))
                .map_err(e)?;
            prules = st
                .query_map([], |r| {
                    let via = match r.get::<_, duckdb::types::Value>(2)? {
                        duckdb::types::Value::List(l) => l.into_iter().filter_map(|v| if let duckdb::types::Value::BigInt(i) = v { Some(i) } else { None }).collect(),
                        _ => Vec::new(),
                    };
                    Ok(PathRule { rtype: r.get::<_, Option<String>>(0)?.unwrap_or_default(), from: r.get(1)?, via, to: r.get(3)?, except: r.get(4)?, veh: r.get(5)? })
                })
                .map_err(e)?
                .collect::<std::result::Result<_, _>>()
                .map_err(e)?;
        }
    }
    // the netconvert options: the built-in ones, then the caller's (a dict update of Python)
    let mut opts: Vec<(String, String)> = DEFAULT_NETCFG.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let set = |opts: &mut Vec<(String, String)>, k: &str, v: String| match opts.iter_mut().find(|x| x.0 == k) {
        Some(x) => x.1 = v,
        None => opts.push((k.to_string(), v)),
    };
    let mut osm_opts = opts.clone();
    for (k, v) in &o.config {
        set(&mut osm_opts, k, v.clone());
    }
    if mode == "walking" {
        set(&mut opts, "walkingareas", "true".into());
    }
    match mode {
        "walking" => set(&mut opts, "default.lanewidth", "2.0".into()),
        "cycling" => set(&mut opts, "default.lanewidth", "1.5".into()),
        _ => {}
    }
    for (k, v) in &o.config {
        set(&mut opts, k, v.clone());
    }
    // an arrowed edge -> its lane connections (SUMO's OSM import); only GMNS's lanes have turn arrows
    let empty = HashMap::new();
    let profile = o.lanes.as_ref().unwrap_or(&empty);
    let mut arrowed: Vec<i64> = profile.iter().filter(|(_, ls)| ls.iter().any(|x| x.2.as_deref().is_some_and(|v| !v.is_empty()))).map(|(k, _)| *k).collect();
    arrowed.sort();
    // the network in memory: only for netconvert's OSM import of the turn arrows (GMNS) and for a junction join
    let load_nodes = || -> Result<Vec<(i64, f64, f64)>> {
        let mut nodes = Vec::new();
        each_chunk(&con, &nodes_sql, [], |c| {
            nodes.extend((0..c.len).map(|i| (c.i64(0, i).unwrap_or_default(), c.f64(1, i).unwrap_or_default(), c.f64(2, i).unwrap_or_default())));
            Ok(())
        })?;
        Ok(nodes)
    };
    let load_rows = || -> Result<Vec<Edge>> {
        let mut rows = Vec::new();
        each_chunk(&con, &format!("SELECT * EXCLUDE (k) FROM ({rows_sql}) ORDER BY k"), [], |c| {
            rows.extend((0..c.len).map(|i| Edge {
                id: c.i64(0, i).unwrap_or_default(),
                src: c.i64(1, i).unwrap_or_default(),
                tgt: c.i64(2, i).unwrap_or_default(),
                hw: c.str(3, i).map(String::from),
                name: c.str(4, i).map(String::from),
                lanes: c.i64(5, i),
                spd: c.f64(6, i),
                length: c.f64(7, i),
                wkt: c.str(8, i).map(String::from),
            }));
            Ok(())
        })?;
        Ok(rows)
    };
    let load_conns = || -> Result<Vec<Conn>> {
        let mut cs = Vec::new();
        each_chunk(&con, &pairs_sql, [], |c| {
            cs.extend((0..c.len).filter_map(|i| perms.at(c.i64(0, i).unwrap_or_default(), c.i64(1, i).unwrap_or_default())));
            Ok(())
        })?;
        cs.extend(perms.added());
        Ok(cs)
    };
    let (mut lane_cs, mut here_edges): (Vec<(i64, Vec<(i64, i64, i64)>)>, HashSet<i64>) = (Vec::new(), HashSet::new());
    if use_conns && !arrowed.is_empty() && netconvert {
        // netconvert's OSM import reads the whole network: GMNS's lanes only
        let (bin, home) = find_netconvert()?;
        let run_osm = |cmd: &[String]| -> Result<()> {
            let mut c = std::process::Command::new(&bin);
            c.args(cmd);
            if let Some(h) = &home {
                if std::env::var_os("SUMO_HOME").is_none() {
                    c.env("SUMO_HOME", h);
                }
            }
            let out = c.output().map_err(|x| format!("netconvert: {x}"))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                return Err(format!("netconvert (OSM input, for the turn arrows) failed ({}):\n{}", out.status, &err[err.floor_char_boundary(err.len().saturating_sub(2000))..]));
            }
            Ok(())
        };
        let (nodes, rows) = (load_nodes()?, load_rows()?);
        let mut sql = format!("SELECT edge_id, osm_id, is_reverse::BOOLEAN FROM {mode}.edges");
        if bus {
            sql.push_str(" UNION ALL SELECT edge_id, osm_id, is_reverse::BOOLEAN FROM driving.private_edges WHERE access = 'bus'");
        }
        let mut way_of: HashMap<i64, (i64, bool)> = HashMap::new();
        each_chunk(&con, &sql, [], |c| {
            way_of.extend((0..c.len).map(|i| (c.i64(0, i).unwrap_or_default(), (c.i64(1, i).unwrap_or_default(), c.bool(2, i).unwrap_or(false)))));
            Ok(())
        })?;
        let pairs: Vec<(i64, i64)> = load_conns()?.into_iter().map(|x| (x.0, x.1)).collect();
        lane_cs = arrow_connections(&way_of, &rows, profile, &arrowed, &pairs, &nodes, out_dir, net_name, &run_osm, &osm_opts)?;
        here_edges = rows.iter().filter(|r| r.src != r.tgt).map(|r| r.id).collect();
    } else if use_conns && !arrowed.is_empty() {
        eprintln!("SUMO connections[{mode}]: {} edges have turn arrows, but netconvert does not run: their connections are edge-level", arrowed.len());
    }
    let lane_edges: HashSet<i64> = lane_cs.iter().map(|x| x.0).collect();

    // the three files, each written while DuckDB reads the tables (`joined`: None), or from the network of a junction join, which
    // is in memory; gives the numbers of nodes, edges and connections
    let write = |joined: Option<&Joined<'_>>, lanes: Option<&HashMap<(String, String), Vec<(String, String)>>>| -> Result<(usize, usize, usize, usize, usize)> {
        let mut f = Out::create(&nod_path, false)?;
        f.put("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<nodes>\n")?;
        let (mut n_nodes, mut n_lights, mut n_stops) = (0, 0, 0);
        let mut count = |n: &str| match ntype(n).map(|x| x.0) {
            Some("traffic_light") => n_lights += 1,
            Some(_) => n_stops += 1,
            None => {}
        };
        match joined {
            None => each_chunk(&con, &nodes_sql, [], |c| {
                for i in 0..c.len {
                    let (n, x, y) = (c.i64(0, i).unwrap_or_default().to_string(), c.f64(1, i).unwrap_or_default(), c.f64(2, i).unwrap_or_default());
                    if !types.is_empty() {
                        count(&n);
                    }
                    f.put(&node_xml(&n, x, y))?;
                }
                n_nodes += c.len;
                Ok(())
            })?,
            Some(j) => {
                for (n, x, y) in &j.m.nodes {
                    count(n);
                    f.put(&node_xml(n, *x, *y))?;
                }
                n_nodes = j.m.nodes.len();
            }
        }
        f.put("</nodes>\n")?;
        f.finish()?;
        let mut n_edges = 0;
        let mut f = Out::create(&edg_path, false)?;
        f.put("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<edges>\n")?;
        if let Some(j) = joined {
            let end = |n: i64| j.m.of.get(&n).cloned().unwrap_or_else(|| n.to_string());
            for r in &j.m.rows {
                let (from, to) = (end(r.src), end(r.tgt));
                if from == to {
                    continue; // SUMO refuses from == to
                }
                let extra = o.edge_attrs.get(&r.id);
                let here = if extra.is_some_and(|x| x.iter().any(|(k, _)| k == "numLanes")) { None } else { j.profile.get(&r.id).filter(|l| !l.is_empty()) };
                f.put(&edge_xml(r, &from, &to, j.both_ways.contains(&(r.tgt, r.src)), here.map(|v| v.as_slice()), extra, mode))?;
                n_edges += 1;
            }
        }
        if joined.is_none() {
          each_chunk(&con, &edges_sql, [], |c| {
            let (uses, widths) = (c.col(10).as_list::<i32>(), c.col(11).as_list::<i32>());
            for i in 0..c.len {
                let (id, src, tgt) = (c.i64(0, i).unwrap_or_default(), c.i64(1, i).unwrap_or_default(), c.i64(2, i).unwrap_or_default());
                let (hw, name, lanes, spd, len, wkt) = (c.str(3, i), c.str(4, i), c.i64(5, i), c.f64(6, i), c.f64(7, i), c.str(8, i));
                let r = Edge {
                    id,
                    src,
                    tgt,
                    hw: hw.map(String::from),
                    name: name.map(String::from),
                    lanes,
                    spd,
                    length: len,
                    wkt: wkt.map(String::from),
                };
                let extra = o.edge_attrs.get(&id);
                // the lanes (use, width), left to right
                let lanes_here: Option<Vec<(String, f64)>> = if extra.is_some_and(|x| x.iter().any(|(k, _)| k == "numLanes")) {
                    None
                } else if let Some(l) = &o.lanes {
                    l.get(&id).filter(|l| !l.is_empty()).map(|l| l.iter().map(|x| (x.0.clone(), x.1)).collect())
                } else if uses.is_valid(i) {
                    let (u, w) = (cast_col(&uses.value(i))?, cast_col(&widths.value(i))?);
                    let (u, w) = (u.as_string::<i32>(), w.as_primitive::<Float64Type>());
                    Some((0..u.len()).map(|j| (u.value(j).to_string(), w.value(j))).collect::<Vec<_>>()).filter(|l| !l.is_empty())
                } else {
                    None
                };
                f.put(&edge_xml(&r, &src.to_string(), &tgt.to_string(), c.bool(9, i) == Some(true), lanes_here.as_deref(), extra, mode))?;
            }
            n_edges += c.len;
            Ok(())
          })?;
        }
        f.put("</edges>\n")?;
        f.finish()?;
        let mut n_conns = 0;
        if use_conns {
            let mut f = Out::create(&con_path, false)?;
            f.put("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<connections>\n")?;
            let mut put = |f: &mut Out, (fe, te, allow, disallow): Conn| -> Result<()> {
                n_conns += 1;
                let by_lane = lanes.and_then(|l| l.get(&(fe.to_string(), te.to_string()))).filter(|v| !v.is_empty());
                if let Some(ls) = by_lane {
                    // a permission needs lanes, and then each move of its edge too
                    let mut p = String::new();
                    if let Some(a) = allow {
                        p.push_str(&format!(" allow=\"{}\"", a.iter().copied().collect::<Vec<_>>().join(" ")));
                    }
                    if !disallow.is_empty() {
                        p.push_str(&format!(" disallow=\"{}\"", disallow.iter().copied().collect::<Vec<_>>().join(" ")));
                    }
                    for (fl, tl) in ls {
                        f.put(&format!("  <connection from=\"{fe}\" to=\"{te}\" fromLane=\"{fl}\" toLane=\"{tl}\"{p}/>\n"))?;
                    }
                } else if !lane_edges.contains(&fe) {
                    f.put(&format!("  <connection from=\"{fe}\" to=\"{te}\"/>\n"))?;
                }
                Ok(())
            };
            match joined {
                None => {
                    each_chunk(&con, &pairs_sql, [], |c| {
                        for i in 0..c.len {
                            if let Some(x) = perms.at(c.i64(0, i).unwrap_or_default(), c.i64(1, i).unwrap_or_default()) {
                                put(&mut f, x)?;
                            }
                        }
                        Ok(())
                    })?;
                    for x in perms.added() {
                        put(&mut f, x)?;
                    }
                }
                Some(j) => {
                    for x in &j.m.cs {
                        put(&mut f, x.clone())?;
                    }
                }
            }
            // the turn arrows' lane connections, onto the edges still there
            let by_lane: HashSet<String> = lanes.map(|l| l.keys().map(|k| k.0.clone()).collect()).unwrap_or_default();
            let here_joined: Option<HashSet<i64>> = joined.map(|j| {
                let end = |n: i64| j.m.of.get(&n).cloned().unwrap_or_else(|| n.to_string());
                j.m.rows.iter().filter(|r| end(r.src) != end(r.tgt)).map(|r| r.id).collect()
            });
            let here_edges = here_joined.as_ref().unwrap_or(&here_edges);
            for (fe, ls) in &lane_cs {
                if !here_edges.contains(fe) || by_lane.contains(&fe.to_string()) {
                    continue;
                }
                let ls: Vec<&(i64, i64, i64)> = ls.iter().filter(|x| here_edges.contains(&x.1)).collect();
                for (fl, te, tl) in &ls {
                    f.put(&format!("  <connection from=\"{fe}\" to=\"{te}\" fromLane=\"{fl}\" toLane=\"{tl}\"/>\n"))?;
                }
                if ls.is_empty() {
                    f.put(&format!("  <connection from=\"{fe}\"/>\n"))?;
                }
            }
            match joined {
                None => each_chunk(&con, &stuck_sql, [], |c| {
                    for i in 0..c.len {
                        f.put(&format!("  <connection from=\"{}\"/>\n", c.i64(0, i).unwrap_or_default()))?;
                    }
                    Ok(())
                })?,
                Some(j) => {
                    for fe in &j.m.stuck {
                        f.put(&format!("  <connection from=\"{fe}\"/>\n"))?;
                    }
                }
            }
            f.put("</connections>\n")?;
            f.finish()?;
        }
        Ok((n_nodes, n_edges, n_conns, n_lights, n_stops))
    };
    let (mut n_nodes, mut n_edges, n_connections, n_traffic_lights, n_stop_signs) = write(None, None)?;
    // the bus stops, as `_write_bus_stops`
    let add_path = path("add.xml");
    let mut n_bus_stops = 0;
    if mode == "driving" && has_table(&con, "raw", "nodes")? && has_table(&con, "bus", "route_edges")? {
        let lanes = if has_table(&con, mode, "lane_profile")? {
            format!("{mode}.lane_profile")
        } else {
            "(SELECT NULL::BIGINT AS edge_id, NULL::INTEGER AS lane_num, NULL::VARCHAR AS use WHERE false)".to_string()
        };
        let mut st = con.prepare(&BUS_STOPS_SQL.replace("{mode}", mode).replace("{lanes}", &lanes)).map_err(e)?;
        let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<additional>\n");
        for r in st
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, f64>(4)?, r.get::<_, f64>(5)?, r.get::<_, Option<String>>(6)?))
            })
            .map_err(e)?
        {
            let (osm, name, eid, lane, a, b, lines) = r.map_err(e)?;
            x.push_str(&format!("  <busStop id=\"bus_stop_{osm}\" lane=\"{eid}_{lane}\" startPos=\"{a:.2}\" endPos=\"{b:.2}\" friendlyPos=\"true\""));
            if let Some(n) = name.filter(|n| !n.is_empty()) {
                x.push_str(&format!(" name={}", quoteattr(&n)));
            }
            if let Some(l) = lines.filter(|l| !l.is_empty()) {
                x.push_str(&format!(" lines={}", quoteattr(&l)));
            }
            x.push_str("/>\n");
            n_bus_stops += 1;
        }
        if n_bus_stops > 0 {
            x.push_str("</additional>\n");
            std::fs::write(&add_path, x).map_err(|x| x.to_string())?;
        }
    }
    eprintln!("SUMO[{mode}]: {n_traffic_lights} traffic lights, {n_stop_signs} stop signs, {n_bus_stops} bus stops");
    // the files with one edge for each chain (`merge`)
    let merge = |add: Option<&Path>| -> Result<(usize, usize)> {
        match &chains {
            Some(c) => crate::sumo_merge::merge_files(c, &nod_path, &edg_path, use_conns.then_some(con_path.as_path()), add),
            None => Ok((0, 0)),
        }
    };
    if let Some(c) = &chains {
        let before = n_edges;
        (n_nodes, n_edges) = merge((n_bus_stops > 0).then_some(add_path.as_path()))?;
        eprintln!("SUMO[{mode}]: merge: {} chains join {before} edges into {n_edges}", c.len());
    }
    if use_conns && !netconvert && perms.all_keyed().any(|c| c.2.is_some() || !c.3.is_empty()) {
        eprintln!("SUMO connections[{mode}]: some turns are open or closed to some vehicles only; netconvert takes that only lane by lane, which the export writes when it runs netconvert itself");
    }
    let mut s = Summary {
        nod: nod_path.clone(),
        edg: edg_path.clone(),
        con: use_conns.then(|| con_path.clone()),
        add: (n_bus_stops > 0).then_some(add_path),
        netccfg: Some(config.map_or_else(|| path("netccfg"), Path::to_path_buf)),
        net: None,
        tll: None,
        n_nodes,
        n_edges,
        n_connections,
        n_traffic_lights,
        n_stop_signs,
        n_bus_stops,
    };
    // the built-in netconvert config (.netccfg) with the options `opts`, and the signal programs `tll`
    let write_cfg = |opts: &[(String, String)], tll: Option<&Path>| -> Result<()> {
        let mut l = vec![
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
            "<configuration>".into(),
            format!("  <node-files value=\"{}\"/>", rel(&nod_path, out_dir)),
            format!("  <edge-files value=\"{}\"/>", rel(&edg_path, out_dir)),
        ];
        if use_conns {
            l.push(format!("  <connection-files value=\"{}\"/>", rel(&con_path, out_dir)));
        }
        if let Some(t) = tll {
            l.push(format!("  <tllogic-files value=\"{}\"/>", rel(t, out_dir)));
        }
        l.push(format!("  <output-file value=\"{}\"/>", rel(&net_path, out_dir)));
        for (k, v) in opts {
            l.push(format!("  <{k} value=\"{v}\"/>"));
        }
        l.extend(["</configuration>".into(), String::new()]);
        std::fs::write(path("netccfg"), l.join("\n")).map_err(|x| x.to_string())
    };
    let mut cmd: Vec<String> = Vec::new();
    match config {
        Some(c) => {
            cmd.extend(["-c".into(), c.display().to_string()]);
            cmd.extend(["--node-files".into(), nod_path.display().to_string(), "--edge-files".into(), edg_path.display().to_string()]);
            cmd.extend(["--output-file".into(), net_path.display().to_string()]);
            if use_conns {
                cmd.extend(["--connection-files".into(), con_path.display().to_string()]);
            }
        }
        None => {
            write_cfg(&opts, None)?;
            cmd.extend(["-c".into(), path("netccfg").display().to_string()]);
        }
    }
    if !netconvert {
        return Ok(s);
    }
    let (bin, home) = find_netconvert()?;
    let run = |extra: &[String]| -> Result<()> {
        // the name of the program only: its full path has the home folder of the user, and a notebook keeps this line in its outputs
        let name = bin.file_name().map_or_else(|| bin.display().to_string(), |n| n.to_string_lossy().into_owned());
        eprintln!("running: {name} {}", cmd.iter().chain(extra).cloned().collect::<Vec<_>>().join(" "));
        let mut c = std::process::Command::new(&bin);
        c.args(&cmd).args(extra);
        if let Some(h) = &home {
            if std::env::var_os("SUMO_HOME").is_none() {
                c.env("SUMO_HOME", h);
            }
        }
        let o = c.output().map_err(|x| format!("netconvert: {x}"))?;
        if !o.status.success() {
            let err = String::from_utf8_lossy(&o.stderr);
            return Err(format!("netconvert failed ({}):\n{}", o.status, &err[err.floor_char_boundary(err.len().saturating_sub(2000))..]));
        }
        Ok(())
    };
    // the merged network when netconvert joins nodes (in memory: a join is an option)
    let (j_nodes, j_rows, j_cs, j_stuck);
    let mut joined: Option<Joined> = None;
    let join = opts.iter().any(|(k, v)| k == "junctions.join" && v.eq_ignore_ascii_case("true"));
    if join && chains.is_some() {
        return Err("merge: not with junctions.join=true; give one of the two".into());
    }
    if use_conns && config.is_none() && join {
        // netconvert reads the connections before it joins nodes, and drops each connection to an edge that the join removes (an
        // edge between two joined nodes): a road whose only way on went through such an edge lost its turns. So netconvert names
        // the joins (pass 1), this export merges those nodes and writes each legal path through them as one connection, and
        // netconvert makes the network from that, with no more joins (pass 2).
        let joins_path = path("joins.xml");
        run(&["--junctions.join-output".into(), joins_path.display().to_string()])?;
        let groups = joined_groups(&joins_path)?;
        if !groups.is_empty() {
            (j_nodes, j_rows, j_cs) = (load_nodes()?, load_rows()?, load_conns()?);
            let mut st = Vec::new();
            each_chunk(&con, &stuck_sql, [], |c| {
                st.extend((0..c.len).map(|i| c.i64(0, i).unwrap_or_default()));
                Ok(())
            })?;
            j_stuck = st;
            let m = merge_joined(&j_nodes, &j_rows, &j_cs, &j_stuck, &groups, &prules);
            let both_ways: HashSet<(i64, i64)> = j_rows.iter().map(|r| (r.src, r.tgt)).collect();
            let mut profile: HashMap<i64, Vec<(String, f64)>> = o.lanes.iter().flatten().map(|(k, v)| (*k, v.iter().map(|x| (x.0.clone(), x.1)).collect())).collect();
            if db_profile {
                let mut st = con.prepare("SELECT edge_id, use, width_m::DOUBLE FROM driving.lane_profile ORDER BY edge_id, lane_num").map_err(e)?;
                for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, f64>(2)?))).map_err(e)? {
                    let (id, u, w) = r.map_err(e)?;
                    profile.entry(id).or_default().push((u, w));
                }
            }
            let j = Joined { m, both_ways, profile };
            write(Some(&j), None)?;
            let mut o2 = opts.clone();
            for x in o2.iter_mut().filter(|x| x.0 == "junctions.join") {
                x.1 = "false".into();
            }
            write_cfg(&o2, None)?;
            let n_rules: HashSet<(i64, &[i64])> = prules.iter().map(|p| (p.from, p.via.as_slice())).collect();
            eprintln!(
                "SUMO: {} joined junction(s), {} connections through them, {} of {} via-way restriction path(s) applied",
                groups.len(),
                j.m.cs.len(),
                j.m.n_paths,
                n_rules.len()
            );
            joined = Some(j);
        }
    }
    run(&[])?;
    // netconvert takes a permission only on a lane-to-lane connection, and then it works out no other move of that edge:
    // read the lanes that it chose for each move of those edges, and write them all again by lane
    let with_rule = |c: &Conn| c.2.is_some() || !c.3.is_empty();
    let froms: HashSet<String> = match &joined {
        Some(j) => j.m.cs.iter().filter(|c| with_rule(c)).map(|c| c.0.to_string()).collect(),
        None => perms.all_keyed().filter(with_rule).map(|c| c.0.to_string()).collect(),
    };
    if !froms.is_empty() {
        let mut lanes: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
        for (f, t, fl, tl) in net_connections(&net_path)? {
            // a joined edge: its connections leave its last edge and enter the first edge of the next one
            let (f, t) = match &chains {
                Some(c) => (c.ends_of(&f).1, c.ends_of(&t).0),
                None => (f, t),
            };
            if froms.contains(&f) && !f.starts_with(':') {
                lanes.entry((f, t)).or_default().push((fl, tl));
            }
        }
        write(joined.as_ref(), Some(&lanes))?;
        merge(None)?;
        run(&[])?;
    }
    let tll = path("tll.xml");
    if write_tll(&net_path, &tll)? > 0 {
        if config.is_none() {
            // the next `netconvert -c` keeps the programs that a user edits in it
            let mut o2 = opts.clone();
            if joined.is_some() {
                for x in o2.iter_mut().filter(|x| x.0 == "junctions.join") {
                    x.1 = "false".into();
                }
            }
            write_cfg(&o2, Some(&tll))?;
        }
        s.tll = Some(tll);
    }
    s.net = Some(net_path);
    Ok(s)
}

/// The groups of nodes that netconvert joins into one junction (its `--junctions.join-output`: `<join nodes="1 2 3"/>`).
fn joined_groups(path: &Path) -> Result<Vec<Vec<String>>> {
    let text = std::fs::read_to_string(path).map_err(|x| format!("{}: {x}", path.display()))?;
    Ok(text
        .split("<join ")
        .skip(1)
        .filter_map(|j| j.split("nodes=\"").nth(1)?.split('"').next())
        .map(|n| n.split_whitespace().map(String::from).collect())
        .collect())
}

/// A row of `turn_path_restrictions`: a path from an edge over the via edges to an edge.
struct PathRule {
    rtype: String,
    from: i64,
    via: Vec<i64>,
    to: i64,
    except: Option<String>,
    veh: Option<String>,
}

/// A junction join to write: the merged network, the (source, target) pairs of all edges, and each edge's lanes (use, width).
struct Joined<'a> {
    m: Merged<'a>,
    both_ways: HashSet<(i64, i64)>,
    profile: HashMap<i64, Vec<(String, f64)>>,
}

/// The network with the joined nodes merged ([`merge_joined`]).
struct Merged<'a> {
    nodes: Vec<(String, f64, f64)>,
    rows: Vec<&'a Edge>,
    of: HashMap<i64, String>,
    cs: Vec<Conn>,
    stuck: Vec<i64>,
    n_paths: usize,
}

type Classes = Option<BTreeSet<&'static str>>;

/// The vehicles that both steps let through (None: each vehicle).
fn both(a: &Classes, b: &Classes) -> Classes {
    match (a, b) {
        (None, b) => b.clone(),
        (a, None) => a.clone(),
        (Some(a), Some(b)) => Some(a.intersection(b).copied().collect()),
    }
}

/// `_merge_joined`: merge each group of nodes into one junction node `cluster_<ids>` (netconvert's own name), at their mean
/// position. An edge between two nodes of one group is gone (netconvert drops it too); each legal path from an edge into a group
/// through such edges to an edge out of it (the successors in `cs`, no node two times) becomes one connection, open to the
/// vehicles that each step lets through and closed to those that a step keeps out. A via-way restriction (`prules`) on a path that
/// the group took in is applied to that connection: banned (no_*: onto its to edge; only_*: onto each other one) for all but the
/// exempt, or for its one vehicle class. `n_paths`: how many restriction paths applied.
fn merge_joined<'a>(nodes: &[(i64, f64, f64)], rows: &'a [Edge], cs: &[Conn], stuck: &[i64], groups: &[Vec<String>], prules: &[PathRule]) -> Merged<'a> {
    let of: HashMap<i64, String> = groups
        .iter()
        .flat_map(|g| {
            let name = format!("cluster_{}", g.join("_"));
            g.iter().filter_map(move |n| n.parse::<i64>().ok().map(|n| (n, name.clone())))
        })
        .collect();
    let pos: HashMap<i64, (f64, f64)> = nodes.iter().map(|(n, x, y)| (*n, (*x, *y))).collect();
    let mut merged: Vec<(String, f64, f64)> = nodes.iter().filter(|n| !of.contains_key(&n.0)).map(|(n, x, y)| (n.to_string(), *x, *y)).collect();
    for g in groups {
        let ps: Vec<(f64, f64)> = g.iter().filter_map(|n| n.parse::<i64>().ok().and_then(|n| pos.get(&n).copied())).collect();
        if !ps.is_empty() {
            let k = ps.len() as f64;
            merged.push((format!("cluster_{}", g.join("_")), ps.iter().map(|p| p.0).sum::<f64>() / k, ps.iter().map(|p| p.1).sum::<f64>() / k));
        }
    }
    let j = |n: i64| of.get(&n).cloned().unwrap_or_else(|| n.to_string());
    let inside: HashSet<i64> = rows.iter().filter(|r| of.contains_key(&r.src) && j(r.src) == j(r.tgt)).map(|r| r.id).collect();
    let rows2: Vec<&Edge> = rows.iter().filter(|r| !inside.contains(&r.id)).collect();
    let ends: HashMap<i64, (i64, i64)> = rows.iter().map(|r| (r.id, (r.src, r.tgt))).collect();
    let mut succ: HashMap<i64, Vec<&Conn>> = HashMap::new();
    for c in cs {
        succ.entry(c.0).or_default().push(c);
    }
    let mut by_path: HashMap<(i64, &[i64]), Vec<&PathRule>> = HashMap::new();
    for p in prules {
        by_path.entry((p.from, p.via.as_slice())).or_default().push(p);
    }
    let mut applied: HashSet<(i64, Vec<i64>)> = HashSet::new();
    // one move found along two paths: open to the vehicles that either path lets through
    let mut cs2: std::collections::BTreeMap<(i64, i64), (Classes, BTreeSet<&'static str>)> = std::collections::BTreeMap::new();
    let mut keep = |fe: i64, te: i64, a: Classes, d: BTreeSet<&'static str>| {
        let v = match cs2.get(&(fe, te)) {
            Some((a0, d0)) => (
                match (a0, &a) {
                    (Some(x), Some(y)) => Some(x.union(y).copied().collect()),
                    _ => None,
                },
                d0.intersection(&d).copied().collect(),
            ),
            None => (a, d),
        };
        cs2.insert((fe, te), v);
    };
    for (fe, te, allow, disallow) in cs {
        if inside.contains(fe) {
            continue;
        }
        if !inside.contains(te) {
            keep(*fe, *te, allow.clone(), disallow.clone());
            continue;
        }
        // through the edges that the group took in, never back to a node passed
        let mut todo: Vec<(i64, HashSet<i64>, Classes, BTreeSet<&'static str>, Vec<i64>)> =
            vec![(*te, HashSet::from([ends[fe].1, ends[te].1]), allow.clone(), disallow.clone(), vec![*te])];
        while let Some((e, seen, a, d, inner)) = todo.pop() {
            for (_, nxt, a2, d2) in succ.get(&e).into_iter().flatten() {
                let (nxt_end, a3, d3) = (ends[nxt].1, both(&a, a2), d.union(d2).copied().collect::<BTreeSet<_>>());
                if inside.contains(nxt) {
                    if !seen.contains(&nxt_end) {
                        let mut s2 = seen.clone();
                        s2.insert(nxt_end);
                        let mut i2 = inner.clone();
                        i2.push(*nxt);
                        todo.push((*nxt, s2, a3, d3, i2));
                    }
                } else if !seen.contains(&nxt_end) && nxt_end != ends[fe].0 {
                    // a via-way restriction on this path: (allow, disallow), or None when it is banned to all
                    let mut ad = Some((a3, d3));
                    for p in by_path.get(&(*fe, inner.as_slice())).into_iter().flatten() {
                        let Some((a, d)) = ad.as_mut() else { break };
                        if (p.rtype.starts_with("no_") && p.to == *nxt) || (p.rtype.starts_with("only_") && p.to != *nxt) {
                            applied.insert((*fe, inner.clone()));
                            let exc = classes(p.except.as_deref());
                            if p.veh.as_deref().is_some_and(|v| !v.is_empty()) {
                                d.extend(classes(p.veh.as_deref()));
                            } else if p.except.as_deref().is_some_and(|v| !v.is_empty()) && !exc.is_empty() {
                                *a = both(a, &Some(exc));
                            } else {
                                ad = None;
                            }
                        }
                    }
                    if let Some((a, d)) = ad {
                        keep(*fe, *nxt, a, d);
                    }
                }
            }
        }
    }
    let cs2: Vec<Conn> = cs2.into_iter().filter(|(_, (a, _))| a.as_ref().is_none_or(|a| !a.is_empty())).map(|((f, t), (a, d))| (f, t, a, d)).collect();
    let has: HashSet<i64> = cs2.iter().map(|c| c.0).collect();
    let had: HashSet<i64> = cs.iter().map(|c| c.0).collect();
    let mut stuck2: Vec<i64> = stuck.iter().filter(|f| !inside.contains(f)).copied().collect();
    stuck2.extend(rows2.iter().filter(|r| !has.contains(&r.id) && j(r.src) != j(r.tgt) && had.contains(&r.id)).map(|r| r.id));
    Merged { nodes: merged, rows: rows2, of, cs: cs2, stuck: stuck2, n_paths: applied.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_float_as_python() {
        for (x, s) in [(7.0, "7.0"), (7.4123, "7.4123"), (43.73, "43.73"), (1e-5, "1e-05"), (1.5e-7, "1.5e-07"), (1e16, "1e+16"), (-0.5, "-0.5"), (0.0001, "0.0001")] {
            assert_eq!(py_float(x), s);
        }
    }

    /// `merge_joined` as Python's `_merge_joined` on a small junction mapped as two nodes (2, 3): the edge 11 between them goes,
    /// the straight move from 10 over 11 is banned, the left one is banned except for buses.
    #[test]
    fn merge_joined_as_python() {
        let nodes = [(1, 0.0, 0.0), (2, 1.0, 0.0), (3, 1.0, 0.2), (4, 2.0, 0.0), (5, 1.0, 1.0)];
        let edge = |id, src, tgt| Edge { id, src, tgt, hw: None, name: None, lanes: None, spd: None, length: None, wkt: None };
        let rows = [edge(10, 1, 2), edge(11, 2, 3), edge(12, 3, 4), edge(13, 3, 5), edge(14, 4, 1)];
        let cs: Vec<Conn> = [(10, 11), (11, 12), (11, 13), (12, 14)].iter().map(|&(f, t)| (f, t, None, BTreeSet::new())).collect();
        let rule = |rtype: &str, to, except: Option<&str>| PathRule { rtype: rtype.into(), from: 10, via: vec![11], to, except: except.map(String::from), veh: None };
        let prules = [rule("no_left_turn", 13, Some("bus")), rule("no_straight_on", 12, None)];
        let m = merge_joined(&nodes, &rows, &cs, &[13], &[vec!["2".into(), "3".into()]], &prules);
        let names: Vec<(String, f64, f64)> = ["1", "4", "5"].iter().map(|n| n.to_string()).zip([(0.0, 0.0), (2.0, 0.0), (1.0, 1.0)]).map(|(n, (x, y))| (n, x, y)).collect();
        assert_eq!(m.nodes[..3], names[..]);
        assert_eq!(m.nodes[3].0, "cluster_2_3");
        assert!((m.nodes[3].1 - 1.0).abs() < 1e-12 && (m.nodes[3].2 - 0.1).abs() < 1e-12);
        assert_eq!(m.rows.iter().map(|r| r.id).collect::<Vec<_>>(), [10, 12, 13, 14]);
        assert_eq!(m.cs, vec![(10, 13, Some(BTreeSet::from(["bus", "coach"])), BTreeSet::new()), (12, 14, None, BTreeSet::new())]);
        assert_eq!((m.stuck, m.n_paths), (vec![13], 1));
    }

    /// The same cases as Python's `_apply_permissions` gives (checked with duckosm.sumo, 2026-11-17).
    #[test]
    fn permissions_as_python() {
        let s = |v: &[&'static str]| v.iter().copied().collect::<BTreeSet<_>>();
        let cs = [(1, 2), (1, 3), (2, 3), (3, 4)];
        let perm: Vec<Perm> = vec![
            (1, 2, false, Some("hgv".into()), None),    // closed to trucks
            (1, 3, false, None, Some("psv".into())),    // closed to all but buses
            (2, 3, false, None, None),                  // closed to all
            (5, 6, true, Some("bus;taxi".into()), None), // open to buses and taxis only
            (3, 4, true, Some("bicycle".into()), None), // an open turn stays open to all
        ];
        let got = apply_permissions(&cs, &perm);
        assert_eq!(
            got,
            vec![
                (1, 2, None, s(&["trailer", "truck"])),
                (1, 3, Some(s(&["bus", "coach", "taxi"])), s(&[])),
                (3, 4, None, s(&[])),
                (5, 6, Some(s(&["bus", "coach", "taxi"])), s(&[])),
            ]
        );
    }
}
