//! The SUMO export, the port of `to_sumo` of src/duckosm/sumo.py for the command `sumo`: the plain XML (nodes, edges with
//! `id` = `edge_id`, the legal connections of `edge_graph`), the netconvert config (`.netccfg`), and netconvert.
//!
//! Not here (the Python API of GMNS, `gmns_sumo`, group C): the join of nodes when a config dict sets `junctions.join`,
//! the turn arrows, `bus_edges`, `edge_attrs` and `lanes`. The command does not reach them.

use crate::{has_table, open, quoteattr, Result};
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

/// The connections with the turn permissions, as `_apply_permissions` (SUMO has no time: a ban with a condition is in force).
pub fn apply_permissions(cs: &[(i64, i64)], perm: &[Perm]) -> Vec<Conn> {
    let mut order: Vec<(i64, i64)> = Vec::new();
    let mut conn: HashMap<(i64, i64), (Option<BTreeSet<&'static str>>, BTreeSet<&'static str>)> = HashMap::new();
    for k in cs {
        if conn.insert(*k, (None, BTreeSet::new())).is_none() {
            order.push(*k);
        }
    }
    for (fe, te, allowed, veh, exc) in perm {
        let k = (*fe, *te);
        if *allowed {
            // closed to all traffic, open to these
            if !conn.contains_key(&k) {
                conn.insert(k, (Some(BTreeSet::new()), BTreeSet::new()));
                order.push(k);
            }
            if let Some(a) = &mut conn.get_mut(&k).unwrap().0 {
                a.extend(classes(veh.as_deref()));
            }
        } else if veh.is_none() && conn.contains_key(&k) {
            // closed to all but `exc`
            let ex = classes(exc.as_deref());
            if exc.as_deref().is_some_and(|s| !s.is_empty()) && !ex.is_empty() {
                conn.insert(k, (Some(ex), BTreeSet::new()));
            } else {
                conn.remove(&k);
                order.retain(|x| *x != k);
            }
        } else if let Some(c) = conn.get_mut(&k) {
            c.1.extend(classes(veh.as_deref()));
        }
    }
    order
        .into_iter()
        .filter_map(|k| {
            let (a, d) = conn.remove(&k)?;
            (a.as_ref().is_none_or(|a| !a.is_empty())).then_some((k.0, k.1, a, d))
        })
        .collect()
}

/// A float as Python's `str(float)`: the shortest text that reads back the same, with `.0` or an exponent as Python writes it.
fn py_float(x: f64) -> String {
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
fn find_netconvert() -> Result<(PathBuf, Option<PathBuf>)> {
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
    let text = std::fs::read_to_string(net).map_err(|x| format!("{}: {x}", net.display()))?;
    let attr = |l: &str, k: &str| -> String {
        let key = format!(" {k}=\"");
        l.find(&key).map(|i| l[i + key.len()..].split('"').next().unwrap_or("").to_string()).unwrap_or_default()
    };
    Ok(text
        .lines()
        .map(str::trim_start)
        .filter(|l| l.starts_with("<connection "))
        .map(|l| (attr(l, "from"), attr(l, "to"), attr(l, "fromLane"), attr(l, "toLane")))
        .collect())
}

/// Export the mode `mode` of the database `db` to SUMO in `out_dir`, as `to_sumo` with the options of the command `sumo`.
/// `config`: a .netccfg of the user (else the built-in options).
pub fn to_sumo(db: &Path, out_dir: &Path, mode: &str, net_name: &str, connections: bool, config: Option<&Path>, netconvert: bool) -> Result<Summary> {
    let con = open(db)?;
    std::fs::create_dir_all(out_dir).map_err(|x| format!("{}: {x}", out_dir.display()))?;
    let path = |ext: &str| out_dir.join(format!("{net_name}.{ext}"));
    let (nod_path, edg_path, con_path, net_path) = (path("nod.xml"), path("edg.xml"), path("con.xml"), path("net.xml"));

    let mut st = con.prepare(&format!("SELECT node_id, ST_X(geom), ST_Y(geom) FROM {mode}.nodes WHERE geom IS NOT NULL")).map_err(e)?;
    let nodes: Vec<(i64, f64, f64)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
    let mut st = con
        .prepare(&format!(
            "SELECT edge_id, source, target, highway, name, lanes::BIGINT, maxspeed_kmh::DOUBLE, length_m::DOUBLE, ST_AsText(geometry) \
             FROM {mode}.edges"
        ))
        .map_err(e)?;
    let rows: Vec<Edge> = st
        .query_map([], |r| {
            Ok(Edge {
                id: r.get(0)?,
                src: r.get(1)?,
                tgt: r.get(2)?,
                hw: r.get(3)?,
                name: r.get(4)?,
                lanes: r.get(5)?,
                spd: r.get(6)?,
                length: r.get(7)?,
                wkt: r.get(8)?,
            })
        })
        .map_err(e)?
        .collect::<std::result::Result<_, _>>()
        .map_err(e)?;
    let both_ways: HashSet<(i64, i64)> = rows.iter().map(|r| (r.src, r.tgt)).collect();
    // the lanes of driving.lane_profile, left to right: one <lane> each, with its vehicle classes and width
    let mut profile: HashMap<i64, Vec<(String, f64)>> = HashMap::new();
    if mode == "driving" && has_table(&con, "driving", "lane_profile")? {
        let mut st = con.prepare("SELECT edge_id, use, width_m::DOUBLE FROM driving.lane_profile ORDER BY edge_id, lane_num").map_err(e)?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, f64>(2)?))).map_err(e)? {
            let (id, u, w) = r.map_err(e)?;
            profile.entry(id).or_default().push((u, w));
        }
    }
    let use_conns = connections && has_table(&con, mode, "edge_graph")?;
    if connections && !use_conns {
        eprintln!("{mode}.edge_graph absent — letting netconvert INFER connections (turn restrictions NOT enforced)");
    }
    let (mut cs, mut stuck) = (Vec::new(), Vec::new());
    if use_conns {
        // the turns of the mode's own edges (the column `uses`), as routed_graph
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
        let graph = if has_uses > 0 { format!("(SELECT * FROM {mode}.edge_graph WHERE uses IN ('{own}'))") } else { format!("{mode}.edge_graph") };
        let mut st = con.prepare(&format!("SELECT from_edge, to_edge FROM {graph}")).map_err(e)?;
        let pairs: Vec<(i64, i64)> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
        let perm: Vec<Perm> = if has_table(&con, mode, "turn_permission")? {
            let mut st = con
                .prepare(&format!("SELECT from_edge, to_edge, allowed, vehicles, except_vehicles FROM {mode}.turn_permission"))
                .map_err(e)?;
            let v = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<bool>>(2)?.unwrap_or(false), r.get(3)?, r.get(4)?)))
                .map_err(e)?
                .collect::<std::result::Result<_, _>>()
                .map_err(e)?;
            v
        } else {
            vec![]
        };
        cs = apply_permissions(&pairs, &perm);
        // an edge with no legal successor gets a connection with only `from`: else netconvert makes its own there
        let has: HashSet<i64> = cs.iter().map(|c| c.0).collect();
        stuck = rows.iter().filter(|r| r.src != r.tgt && !has.contains(&r.id)).map(|r| r.id).collect();
    }
    drop(st);
    drop(con);

    let path_mode = mode == "walking" || mode == "cycling";
    let write = |lanes: Option<&HashMap<(String, String), Vec<(String, String)>>>| -> Result<usize> {
        let mut f = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<nodes>\n");
        for (n, x, y) in &nodes {
            f.push_str(&format!("  <node id=\"{n}\" x=\"{x:.7}\" y=\"{y:.7}\"/>\n"));
        }
        f.push_str("</nodes>\n");
        std::fs::write(&nod_path, f).map_err(|x| x.to_string())?;
        let mut n_edges = 0;
        let mut f = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<edges>\n");
        for r in &rows {
            if r.src == r.tgt {
                continue; // SUMO refuses from == to
            }
            let mut a = vec![format!("id=\"{}\"", r.id), format!("from=\"{}\"", r.src), format!("to=\"{}\"", r.tgt), format!("priority=\"{}\"", priority(r.hw.as_deref()))];
            let here = profile.get(&r.id).filter(|l| !l.is_empty());
            if let Some(l) = here {
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
            a.push(if both_ways.contains(&(r.tgt, r.src)) { "spreadType=\"roadCenter\"".into() } else { "spreadType=\"center\"".into() });
            if let Some(s) = shape(r.wkt.as_deref()) {
                a.push(format!("shape=\"{s}\""));
            }
            if let Some(n) = r.name.as_deref().filter(|n| !n.is_empty()) {
                a.push(format!("name={}", quoteattr(n)));
            }
            if let Some(l) = here {
                // SUMO numbers the lanes from the right
                f.push_str(&format!("  <edge {}>\n", a.join(" ")));
                for (i, (u, w)) in l.iter().rev().enumerate() {
                    let allow = match u.as_str() {
                        "bus" => " allow=\"bus\"",
                        "bike" => " allow=\"bicycle\"",
                        "bus,bike" => " allow=\"bus bicycle\"",
                        _ => "",
                    };
                    f.push_str(&format!("    <lane index=\"{i}\"{allow} width=\"{w:.2}\"/>\n"));
                }
                f.push_str("  </edge>\n");
            } else {
                f.push_str(&format!("  <edge {}/>\n", a.join(" ")));
            }
            n_edges += 1;
        }
        f.push_str("</edges>\n");
        std::fs::write(&edg_path, f).map_err(|x| x.to_string())?;
        if use_conns {
            let mut f = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<connections>\n");
            for (fe, te, allow, disallow) in &cs {
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
                        f.push_str(&format!("  <connection from=\"{fe}\" to=\"{te}\" fromLane=\"{fl}\" toLane=\"{tl}\"{p}/>\n"));
                    }
                } else {
                    f.push_str(&format!("  <connection from=\"{fe}\" to=\"{te}\"/>\n"));
                }
            }
            for fe in &stuck {
                f.push_str(&format!("  <connection from=\"{fe}\"/>\n"));
            }
            f.push_str("</connections>\n");
            std::fs::write(&con_path, f).map_err(|x| x.to_string())?;
        }
        Ok(n_edges)
    };
    let n_edges = write(None)?;
    let mut s = Summary {
        nod: nod_path.clone(),
        edg: edg_path.clone(),
        con: use_conns.then(|| con_path.clone()),
        net: None,
        n_nodes: nodes.len(),
        n_edges,
        n_connections: cs.len(),
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
            let mut opts: Vec<(&str, String)> = DEFAULT_NETCFG.iter().map(|(k, v)| (*k, v.to_string())).collect();
            if mode == "walking" {
                opts.push(("walkingareas", "true".into()));
            }
            match mode {
                "walking" => opts.push(("default.lanewidth", "2.0".into())),
                "cycling" => opts.push(("default.lanewidth", "1.5".into())),
                _ => {}
            }
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
    let froms: HashSet<String> = cs.iter().filter(|c| c.2.is_some() || !c.3.is_empty()).map(|c| c.0.to_string()).collect();
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
