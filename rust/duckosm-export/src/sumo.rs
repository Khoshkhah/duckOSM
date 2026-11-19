//! The SUMO export, the port of `to_sumo` of src/duckosm/sumo.py for the command `sumo`: the plain XML (nodes, edges with
//! `id` = `edge_id`, the legal connections of `edge_graph`), the netconvert config (`.netccfg`), and netconvert.
//!
//! `to_sumo_with` also takes the options of the Python API that GMNS uses (`gmns_sumo`): the bus-only edges, each edge's lanes
//! (with their turn arrows, which netconvert's OSM import applies), attributes of an edge and more netconvert options. Not here:
//! the join of nodes when the options set `junctions.join`.

use crate::stream::{cast_col, each_chunk};
use crate::{has_table, open, quoteattr, Out, Result};
use duckdb::arrow::array::{Array, AsArray};
use duckdb::arrow::datatypes::Float64Type;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The default options of netconvert (`DEFAULT_NETCFG`): the ids and the topology of duckOSM stay as they are.
const DEFAULT_NETCFG: [(&str, &str); 5] = [
    ("proj.utm", "true"),
    ("geometry.remove", "false"),
    ("junctions.join", "false"),
    ("no-turnarounds.except-deadend", "true"),
    ("numerical-ids", "false"),
];

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
    spd: Option<f64>,
    wkt: Option<String>,
}

pub struct Summary {
    pub nod: PathBuf,
    pub edg: PathBuf,
    pub con: Option<PathBuf>,
    pub net: Option<PathBuf>,
    pub n_nodes: usize,
    pub n_edges: usize,
    pub n_connections: usize,
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
                shape_id += 1;
                while node_ids.contains(&shape_id) {
                    shape_id += 1;
                }
                let (x, y) = pt.split_once(',').unwrap();
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
            if let Some(g) = got.iter_mut().find(|g| g.0 == fe) {
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
    std::fs::create_dir_all(out_dir).map_err(|x| format!("{}: {x}", out_dir.display()))?;
    let path = |ext: &str| out_dir.join(format!("{net_name}.{ext}"));
    let (nod_path, edg_path, con_path, net_path) = (path("nod.xml"), path("edg.xml"), path("con.xml"), path("net.xml"));

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
        if has_perm {
            let mut st = con
                .prepare(&format!("SELECT from_edge, to_edge, allowed, vehicles, except_vehicles FROM {mode}.turn_permission"))
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
    }
    if config.is_none() && o.config.iter().any(|(k, v)| k == "junctions.join" && v.eq_ignore_ascii_case("true")) {
        return Err("to_sumo: junctions.join (the join of nodes) is not ported to Rust".into());
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
        let mut nodes = Vec::new();
        each_chunk(&con, &nodes_sql, [], |c| {
            nodes.extend((0..c.len).map(|i| (c.i64(0, i).unwrap_or_default(), c.f64(1, i).unwrap_or_default(), c.f64(2, i).unwrap_or_default())));
            Ok(())
        })?;
        let mut rows = Vec::new();
        each_chunk(&con, &format!("SELECT * EXCLUDE (k) FROM ({rows_sql}) ORDER BY k"), [], |c| {
            rows.extend((0..c.len).map(|i| Edge {
                id: c.i64(0, i).unwrap_or_default(),
                src: c.i64(1, i).unwrap_or_default(),
                tgt: c.i64(2, i).unwrap_or_default(),
                hw: c.str(3, i).map(String::from),
                name: c.str(4, i).map(String::from),
                spd: c.f64(6, i),
                wkt: c.str(8, i).map(String::from),
            }));
            Ok(())
        })?;
        let mut sql = format!("SELECT edge_id, osm_id, is_reverse::BOOLEAN FROM {mode}.edges");
        if bus {
            sql.push_str(" UNION ALL SELECT edge_id, osm_id, is_reverse::BOOLEAN FROM driving.private_edges WHERE access = 'bus'");
        }
        let mut way_of: HashMap<i64, (i64, bool)> = HashMap::new();
        each_chunk(&con, &sql, [], |c| {
            way_of.extend((0..c.len).map(|i| (c.i64(0, i).unwrap_or_default(), (c.i64(1, i).unwrap_or_default(), c.bool(2, i).unwrap_or(false)))));
            Ok(())
        })?;
        let mut pairs = Vec::new();
        each_chunk(&con, &pairs_sql, [], |c| {
            pairs.extend((0..c.len).filter_map(|i| perms.at(c.i64(0, i).unwrap_or_default(), c.i64(1, i).unwrap_or_default()).map(|x| (x.0, x.1))));
            Ok(())
        })?;
        pairs.extend(perms.added().map(|x| (x.0, x.1)));
        lane_cs = arrow_connections(&way_of, &rows, profile, &arrowed, &pairs, &nodes, out_dir, net_name, &run_osm, &osm_opts)?;
        here_edges = rows.iter().filter(|r| r.src != r.tgt).map(|r| r.id).collect();
    } else if use_conns && !arrowed.is_empty() {
        eprintln!("SUMO connections[{mode}]: {} edges have turn arrows, but netconvert does not run: their connections are edge-level", arrowed.len());
    }
    let lane_edges: HashSet<i64> = lane_cs.iter().map(|x| x.0).collect();

    let path_mode = mode == "walking" || mode == "cycling";
    // the three files, each written while DuckDB reads the tables; gives the numbers of nodes, edges and connections
    let write = |lanes: Option<&HashMap<(String, String), Vec<(String, String)>>>| -> Result<(usize, usize, usize)> {
        let mut f = Out::create(&nod_path, false)?;
        f.put("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<nodes>\n")?;
        let mut n_nodes = 0;
        each_chunk(&con, &nodes_sql, [], |c| {
            for i in 0..c.len {
                let (n, x, y) = (c.i64(0, i).unwrap_or_default(), c.f64(1, i).unwrap_or_default(), c.f64(2, i).unwrap_or_default());
                f.put(&format!("  <node id=\"{n}\" x=\"{x:.7}\" y=\"{y:.7}\"/>\n"))?;
            }
            n_nodes += c.len;
            Ok(())
        })?;
        f.put("</nodes>\n")?;
        f.finish()?;
        let mut n_edges = 0;
        let mut f = Out::create(&edg_path, false)?;
        f.put("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<edges>\n")?;
        each_chunk(&con, &edges_sql, [], |c| {
            let (uses, widths) = (c.col(10).as_list::<i32>(), c.col(11).as_list::<i32>());
            for i in 0..c.len {
                let (id, src, tgt) = (c.i64(0, i).unwrap_or_default(), c.i64(1, i).unwrap_or_default(), c.i64(2, i).unwrap_or_default());
                let (hw, name, lanes, spd, len, wkt) = (c.str(3, i), c.str(4, i), c.i64(5, i), c.f64(6, i), c.f64(7, i), c.str(8, i));
                let mut a = vec![format!("id=\"{id}\""), format!("from=\"{src}\""), format!("to=\"{tgt}\""), format!("priority=\"{}\"", priority(hw))];
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
                if let Some(l) = &lanes_here {
                    a.push(format!("numLanes=\"{}\"", l.len()));
                } else if let Some(n) = lanes.filter(|n| *n > 0 && !path_mode) {
                    a.push(format!("numLanes=\"{n}\""));
                }
                if let Some(s) = spd.filter(|s| *s > 0.0) {
                    a.push(format!("speed=\"{:.3}\"", s / 3.6));
                }
                if let Some(l) = len.filter(|l| *l > 0.0) {
                    a.push(format!("length=\"{l:.2}\""));
                }
                if let Some(h) = hw.filter(|h| !h.is_empty()) {
                    a.push(format!("type={}", quoteattr(h)));
                }
                match mode {
                    "walking" => a.push("allow=\"pedestrian\"".into()),
                    "cycling" => a.push("allow=\"bicycle\"".into()),
                    _ => {}
                }
                // one-way: the line is the middle of its lanes; two-way: the middle of the whole road
                a.push(if c.bool(9, i) == Some(true) { "spreadType=\"roadCenter\"".into() } else { "spreadType=\"center\"".into() });
                if let Some(s) = shape(wkt) {
                    a.push(format!("shape=\"{s}\""));
                }
                if let Some(n) = name.filter(|n| !n.is_empty()) {
                    a.push(format!("name={}", quoteattr(n)));
                }
                for (k, v) in extra.into_iter().flatten() {
                    a.retain(|x| !x.starts_with(&format!("{k}=")));
                    a.push(format!("{k}={}", quoteattr(v)));
                }
                if let Some(l) = &lanes_here {
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
                    f.put(&t)?;
                } else {
                    f.put(&format!("  <edge {}/>\n", a.join(" ")))?;
                }
            }
            n_edges += c.len;
            Ok(())
        })?;
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
            // the turn arrows' lane connections, onto the edges still there
            let by_lane: HashSet<String> = lanes.map(|l| l.keys().map(|k| k.0.clone()).collect()).unwrap_or_default();
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
            each_chunk(&con, &stuck_sql, [], |c| {
                for i in 0..c.len {
                    f.put(&format!("  <connection from=\"{}\"/>\n", c.i64(0, i).unwrap_or_default()))?;
                }
                Ok(())
            })?;
            f.put("</connections>\n")?;
            f.finish()?;
        }
        Ok((n_nodes, n_edges, n_conns))
    };
    let (n_nodes, n_edges, n_connections) = write(None)?;
    let mut s = Summary {
        nod: nod_path.clone(),
        edg: edg_path.clone(),
        con: use_conns.then(|| con_path.clone()),
        net: None,
        n_nodes,
        n_edges,
        n_connections,
    };
    if !netconvert {
        return Ok(s);
    }

    let (bin, home) = find_netconvert()?;
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
            let cfg = path("netccfg");
            let mut l = vec![
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
                "<configuration>".into(),
                format!("  <node-files value=\"{}\"/>", rel(&nod_path, out_dir)),
                format!("  <edge-files value=\"{}\"/>", rel(&edg_path, out_dir)),
            ];
            if use_conns {
                l.push(format!("  <connection-files value=\"{}\"/>", rel(&con_path, out_dir)));
            }
            l.push(format!("  <output-file value=\"{}\"/>", rel(&net_path, out_dir)));
            for (k, v) in &opts {
                l.push(format!("  <{k} value=\"{v}\"/>"));
            }
            l.extend(["</configuration>".into(), String::new()]);
            std::fs::write(&cfg, l.join("\n")).map_err(|x| x.to_string())?;
            cmd.extend(["-c".into(), cfg.display().to_string()]);
        }
    }
    let run = || -> Result<()> {
        eprintln!("running: {} {}", bin.display(), cmd.join(" "));
        let mut c = std::process::Command::new(&bin);
        c.args(&cmd);
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
    run()?;
    // netconvert takes a permission only on a lane-to-lane connection, and then it works out no other move of that edge:
    // read the lanes that it chose for each move of those edges, and write them all again by lane
    let froms: HashSet<String> = perms.all_keyed().filter(|c| c.2.is_some() || !c.3.is_empty()).map(|c| c.0.to_string()).collect();
    if !froms.is_empty() {
        let mut lanes: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
        for (f, t, fl, tl) in net_connections(&net_path)? {
            if froms.contains(&f) && !f.starts_with(':') {
                lanes.entry((f, t)).or_default().push((fl, tl));
            }
        }
        write(Some(&lanes))?;
        run()?;
    }
    s.net = Some(net_path);
    Ok(s)
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
