//! Which lane goes to which, and the path through each junction, from SUMO netconvert: the port of src/duckosm/gmns_sumo.py
//! (`lanes_from_sumo`). The network goes to SUMO (`sumo::to_sumo_with`: GMNS's lanes, the legal turns, the bus-only edges),
//! netconvert builds it, and the export reads back each movement's lane ranges, `lane_connector`, `lane.geom_cut` and what it
//! cannot map (`lane_check`).

use crate::gmns::{window_angle, Ctx};
use crate::shapely::{self as shp, P};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

/// A turn:lanes part -> its place left to right among a link's directions.
fn rank(part: &str, drive_side: &str) -> Option<i64> {
    Some(match part {
        "sharp_left" => 1,
        "left" => 2,
        "slight_left" => 3,
        "through" | "merge_to_left" | "merge_to_right" => 4,
        "slight_right" => 5,
        "right" => 6,
        "sharp_right" => 7,
        "reverse" => {
            if drive_side == "right" {
                0
            } else {
                8
            }
        }
        _ => return None,
    })
}

/// The first name of a rank (4: through).
fn rank_name(r: i64) -> &'static str {
    match r {
        1 => "sharp_left",
        2 => "left",
        3 => "slight_left",
        4 => "through",
        5 => "slight_right",
        6 => "right",
        7 => "sharp_right",
        _ => "reverse",
    }
}

/// Python's `repr` of a text.
fn py_repr(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::from(q);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

pub(crate) type Check = (String, String, String);

/// `_arrow_fit`: which painted arrows fit their link, read by order as SUMO's OSM import reads them. Returns the direction
/// rank of each (link, exit) of the links whose arrows fit, the arrows of each lane, and the lane_check rows of the others.
#[allow(clippy::type_complexity)]
fn arrow_fit(c: &Ctx, sch: &str, drive_side: &str) -> Result<(HashMap<(i64, i64), i64>, Vec<(i64, BTreeMap<i64, Option<String>>)>, Vec<Check>)> {
    let sql = format!("SELECT link_id, lane_num, turn FROM {sch}.lane");
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let rows: Vec<(i64, i64, Option<String>)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
    let mut marked: Vec<(i64, BTreeMap<i64, Option<String>>)> = Vec::new();
    let mut at: HashMap<i64, usize> = HashMap::new();
    for (lk, n, t) in rows {
        let i = *at.entry(lk).or_insert_with(|| {
            marked.push((lk, BTreeMap::new()));
            marked.len() - 1
        });
        marked[i].1.insert(n, t);
    }
    marked.retain(|(_, ls)| ls.values().any(|t| t.as_deref().is_some_and(|v| !v.is_empty())));
    let ib_marked: HashSet<i64> = marked.iter().map(|m| m.0).collect();
    let sql = format!("SELECT DISTINCT ib_link_id, ob_link_id, type FROM {sch}.movement");
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let mut exits: HashMap<i64, BTreeSet<(i64, String)>> = HashMap::new();
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))).map_err(|e| e.to_string())? {
        let (ib, ob, typ) = r.map_err(|e| e.to_string())?;
        if ib_marked.contains(&ib) {
            exits.entry(ib).or_default().insert((ob, typ));
        }
    }
    let geom: HashMap<i64, String> = if marked.is_empty() {
        HashMap::new()
    } else {
        let sql = format!("SELECT link_id, geometry FROM {sch}.link");
        let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
        let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
        v
    };
    let reverse = rank("reverse", drive_side).unwrap();
    let (mut to_dir, mut unmatched) = (HashMap::new(), Vec::new());
    for (lk, ls) in &marked {
        let parts: Vec<Vec<String>> = ls.values().map(|t| t.as_deref().unwrap_or("through").split(';').map(|p| p.trim().to_string()).collect()).collect();
        let unknown: BTreeSet<&str> = parts.iter().flatten().map(|p| p.as_str()).filter(|p| rank(p, drive_side).is_none()).collect();
        let dirs: Vec<i64> = parts.iter().flatten().filter_map(|p| rank(p, drive_side)).collect::<BTreeSet<_>>().into_iter().collect();
        let ex: Vec<(i64, String)> = exits.get(lk).into_iter().flatten().filter(|(_, typ)| typ != "uturn" || dirs.contains(&reverse)).cloned().collect();
        let mut angle: HashMap<i64, Option<f64>> = HashMap::new();
        for (ob, typ) in &ex {
            if typ != "uturn" {
                angle.insert(*ob, window_angle(&geom[lk], &geom[ob]));
            }
        }
        let tags = ls.values().map(|t| t.clone().unwrap_or_default()).collect::<Vec<_>>().join("|");
        if !unknown.is_empty() {
            let u = unknown.iter().map(|x| py_repr(x)).collect::<Vec<_>>().join(", ");
            unmatched.push(("arrows not matched".into(), lk.to_string(), format!("arrows {}: unknown {u}", py_repr(&tags))));
        } else if angle.values().any(|a| a.is_none()) {
            unmatched.push(("arrows not matched".into(), lk.to_string(), format!("arrows {}: an exit shorter than half a metre has no angle", py_repr(&tags))));
        } else if ex.is_empty() {
            continue;
        } else if dirs.len() != ex.len() {
            unmatched.push(("arrows not matched".into(), lk.to_string(), format!("arrows {} name {} directions, the link has {} exits", py_repr(&tags), dirs.len(), ex.len())));
        } else {
            let u = if drive_side == "right" { f64::INFINITY } else { f64::NEG_INFINITY };
            let key = |o: &(i64, String)| -(if o.1 == "uturn" { u } else { angle[&o.0].unwrap() });
            let mut order = ex.clone();
            order.sort_by(|a, b| key(a).total_cmp(&key(b)).then(a.0.cmp(&b.0)));
            for ((ob, _), d) in order.iter().zip(&dirs) {
                to_dir.insert((*lk, *ob), *d);
            }
        }
    }
    Ok((to_dir, marked, unmatched))
}

/// GEOS `TopologyPreservingSimplifier` on one line, as far as the lane middles need it: Douglas-Peucker (no section is kept
/// back to keep the topology: a lane middle is a gentle line).
fn simplify(pts: &[P], tol: f64) -> Vec<P> {
    fn section(pts: &[P], i: usize, j: usize, tol: f64, keep: &mut [bool]) {
        if i + 1 >= j {
            return;
        }
        let (mut max, mut at) = (-1.0, i);
        for k in i + 1..j {
            let d = shp::point_to_segment(pts[k], pts[i], pts[j]);
            if d > max {
                max = d;
                at = k;
            }
        }
        if max <= tol {
            for x in keep.iter_mut().take(j).skip(i + 1) {
                *x = false;
            }
        } else {
            section(pts, i, at, tol, keep);
            section(pts, at, j, tol, keep);
        }
    }
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![true; pts.len()];
    section(pts, 0, pts.len() - 1, tol, &mut keep);
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// `_lane_middles`: the middle line between each link's leftmost and rightmost lane, as the shape that SUMO gets (centred).
fn lane_middles(c: &Ctx, sch: &str) -> Result<HashMap<i64, Vec<(String, String)>>> {
    let sql = format!("SELECT link_id, ST_AsText(geom) FROM {sch}.lane WHERE geom IS NOT NULL ORDER BY link_id, lane_num");
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let mut by_link: Vec<(i64, Vec<Vec<P>>)> = Vec::new();
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).map_err(|e| e.to_string())? {
        let (lk, w) = r.map_err(|e| e.to_string())?;
        match by_link.last_mut() {
            Some(x) if x.0 == lk => x.1.push(shp::wkt_points(&w)),
            _ => by_link.push((lk, vec![shp::wkt_points(&w)])),
        }
    }
    let mut out = HashMap::new();
    for (lk, ls) in by_link {
        let (a, b) = (&ls[0], &ls[ls.len() - 1]);
        let (la, lb) = (shp::length(a), shp::length(b));
        if la == 0.0 || lb == 0.0 {
            continue;
        }
        let n = (a.len().max(b.len()).max(2) * 2).min(400);
        let step = 1.0 / (n - 1) as f64;
        let mid: Vec<P> = (0..n)
            .map(|i| {
                let f = if i == n - 1 { 1.0 } else { i as f64 * step };
                let (p, q) = (shp::interpolate(a, f * la), shp::interpolate(b, f * lb));
                ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0)
            })
            .collect();
        let mid = simplify(&mid, 1e-7);
        let shape = mid.iter().map(|(x, y)| format!("{x:.8},{y:.8}")).collect::<Vec<_>>().join(" ");
        out.insert(lk, vec![("shape".to_string(), shape), ("spreadType".to_string(), "center".to_string())]);
    }
    Ok(out)
}

/// The parts of a .net.xml that the export reads (`_read_net`).
struct Net {
    off: (f64, f64),
    proj: String,
    lanes: HashMap<String, (Vec<P>, f64)>,
    conns: Vec<(String, i64, String, i64, Option<String>)>,
    nxt: HashMap<(String, String), String>,
}

fn read_net(path: &Path) -> Result<Net> {
    let attr = |l: &str, k: &str| -> Option<String> {
        let key = format!(" {k}=\"");
        l.find(&key).map(|i| l[i + key.len()..].split('"').next().unwrap_or("").to_string())
    };
    let mut net = Net { off: (0.0, 0.0), proj: String::new(), lanes: HashMap::new(), conns: Vec::new(), nxt: HashMap::new() };
    crate::each_line(path, |l| {
        let l = l.trim_start();
        if l.starts_with("<location ") {
            let o = attr(l, "netOffset").unwrap_or_default();
            let (x, y) = o.split_once(',').unwrap_or(("0", "0"));
            net.off = (x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
            net.proj = attr(l, "projParameter").unwrap_or_default().replace("&quot;", "\"");
        } else if l.starts_with("<lane ") {
            let pts: Vec<P> = attr(l, "shape")
                .unwrap_or_default()
                .split_whitespace()
                .filter_map(|p| {
                    let mut it = p.split(',').map(|v| v.parse::<f64>());
                    Some((it.next()?.ok()?, it.next()?.ok()?))
                })
                .collect();
            let w = attr(l, "width").and_then(|w| w.parse().ok()).unwrap_or(3.2);
            net.lanes.insert(attr(l, "id").unwrap_or_default(), (pts, w));
        } else if l.starts_with("<connection ") {
            let f = attr(l, "from").unwrap_or_default();
            if f.starts_with(':') {
                if let Some(v) = attr(l, "via").filter(|v| !v.is_empty()) {
                    net.nxt.insert((f, attr(l, "fromLane").unwrap_or_default()), v);
                }
            } else {
                net.conns.push((
                    f,
                    attr(l, "fromLane").and_then(|v| v.parse().ok()).unwrap_or(0),
                    attr(l, "to").unwrap_or_default(),
                    attr(l, "toLane").and_then(|v| v.parse().ok()).unwrap_or(0),
                    attr(l, "via").filter(|v| !v.is_empty()),
                ));
            }
        }
        Ok(())
    })?;
    if net.proj.is_empty() {
        return Err(format!("{}: no <location> (no projection to read the shapes back)", path.display()));
    }
    Ok(net)
}

/// `_path`: the shape through the junction: the internal lane `via` and the internal lanes it goes on into.
fn path_of(via: Option<&str>, net: &Net) -> Vec<P> {
    let (mut pts, mut seen): (Vec<P>, HashSet<String>) = (Vec::new(), HashSet::new());
    let mut via = via.map(String::from);
    while let Some(v) = via.filter(|v| !seen.contains(v)) {
        seen.insert(v.clone());
        let shape = net.lanes.get(&v).map(|x| x.0.clone()).unwrap_or_default();
        if !pts.is_empty() && !shape.is_empty() && shape[0] == *pts.last().unwrap() {
            pts.extend_from_slice(&shape[1..]);
        } else {
            pts.extend(shape);
        }
        let (edge, k) = v.rsplit_once('_').unwrap_or((&v, ""));
        via = net.nxt.get(&(edge.to_string(), k.to_string())).cloned();
    }
    pts
}

/// `_runs`: lane pairs -> runs that go on side by side.
pub(crate) fn runs(pairs: &[(i64, i64)]) -> Vec<Vec<(i64, i64)>> {
    let mut out: Vec<Vec<(i64, i64)>> = Vec::new();
    for p in pairs.iter().copied().collect::<BTreeSet<_>>() {
        match out.last_mut() {
            Some(r) if *r.last().unwrap() == (p.0 - 1, p.1 - 1) => r.push(p),
            _ => out.push(vec![p]),
        }
    }
    out
}

/// The movement rows back into `{sch}.movement`: (old id, new id, lanes (start in, end in, start out, end out), bike row,
/// kept as it was). A movement becomes one row for each of its runs of lanes; a row of no connection keeps its lanes. Shared by
/// netconvert's lanes and the lane rules (gmns_lane_rules).
#[allow(clippy::type_complexity)]
pub(crate) fn apply_rows(c: &Ctx, sch: &str, rows: &[(String, String, Option<(i64, i64, i64, i64)>, bool, bool)]) -> Result<()> {
    c.exec("CREATE OR REPLACE TEMP TABLE _mv_new(old VARCHAR, new VARCHAR, keep BOOLEAN, sil INT, eil INT, sol INT, eol INT, bike BOOLEAN, k BIGINT)")?;
    {
        let mut app = c.con.appender("_mv_new").map_err(|e| e.to_string())?;
        for (k, (old, new, l, bike, keep)) in rows.iter().enumerate() {
            app.append_row(duckdb::params![old, new, keep, l.map(|x| x.0 as i32), l.map(|x| x.1 as i32), l.map(|x| x.2 as i32), l.map(|x| x.3 as i32), bike, k as i64])
                .map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
    }
    c.exec(&format!(
        "CREATE OR REPLACE TEMP TABLE _mv_old AS SELECT * FROM {sch}.movement;
         DELETE FROM {sch}.movement;
         INSERT INTO {sch}.movement SELECT m.* REPLACE (
             n.new AS mvmt_id,
             CASE WHEN n.keep THEN m.start_ib_lane ELSE n.sil END AS start_ib_lane,
             CASE WHEN n.keep THEN m.end_ib_lane ELSE n.eil END AS end_ib_lane,
             CASE WHEN n.keep THEN m.start_ob_lane ELSE n.sol END AS start_ob_lane,
             CASE WHEN n.keep THEN m.end_ob_lane ELSE n.eol END AS end_ob_lane,
             CASE WHEN n.bike THEN 'bike' ELSE m.allowed_uses END AS allowed_uses)
           FROM _mv_new n JOIN _mv_old m ON m.mvmt_id = n.old ORDER BY n.k;
         DROP TABLE _mv_new; DROP TABLE _mv_old"
    ))
}

/// `lanes_from_sumo`: see the module comment. `source`: the duckOSM db. `geometry`: also `lane.geom_cut` and `lane_connector`.
pub(crate) fn lanes_from_sumo(c: &Ctx, source: &Path, sch: &str, mode: &str, geometry: bool, drive_side: &str) -> Result<()> {
    let (to_dir, marked, unmatched) = if mode == "driving" { arrow_fit(c, sch, drive_side)? } else { (HashMap::new(), Vec::new(), Vec::new()) };
    let fits: HashSet<i64> = to_dir.keys().map(|k| k.0).collect();
    let sql = format!("SELECT link_id, allowed_uses, COALESCE(width, 3.25)::DOUBLE, turn FROM {sch}.lane ORDER BY link_id, lane_num");
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let mut ours_lanes: HashMap<i64, Vec<(String, f64, Option<String>)>> = HashMap::new();
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?, r.get::<_, Option<String>>(3)?))).map_err(|e| e.to_string())? {
        let (lk, u, w, t) = r.map_err(|e| e.to_string())?;
        let u = if mode == "cycling" { "bike".to_string() } else { u };
        ours_lanes.entry(lk).or_default().push((u, w, if fits.contains(&lk) { t } else { None }));
    }
    let edge_attrs = if geometry { lane_middles(c, sch)? } else { HashMap::new() };
    // a folder of its own: two exports can run at the same time in one process
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("duckosm-gmns-sumo-{}-{k}-{mode}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    // the folder goes on each way out, also on an error (Kentucky: 1.3 GB)
    struct Gone<'a>(&'a Path);
    impl Drop for Gone<'_> {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0);
        }
    }
    let _gone = Gone(&tmp);
    let opts = crate::sumo::SumoOptions {
        bus_edges: mode == "driving",
        lanes: Some(ours_lanes),
        edge_attrs,
        config: if drive_side == "left" { vec![("lefthand".into(), "true".into())] } else { vec![] },
        merge: false,
    };
    let s = crate::sumo::to_sumo_with(source, &tmp, mode, "gmns", true, None, true, &opts)?;
    let net = read_net(s.net.as_ref().ok_or("netconvert built no net")?)?;
    let mut check: Vec<Check> = Vec::new();
    c.exec(&format!("CREATE OR REPLACE TABLE {sch}.lane_check(kind VARCHAR, id VARCHAR, detail VARCHAR)"))?;
    // our lanes of each link, left to right, and SUMO's (index from the right)
    let sql = format!("SELECT link_id, lane_num, allowed_uses, lane_id, width FROM {sch}.lane ORDER BY link_id, lane_num");
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let mut ours: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    let (mut use_of, mut lid_of, mut width_of) = (BTreeMap::new(), HashMap::new(), HashMap::new());
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<f64>>(4)?))).map_err(|e| e.to_string())? {
        let (lk, n, u, lid, w) = r.map_err(|e| e.to_string())?;
        ours.entry(lk).or_default().push(n);
        use_of.insert((lk, n), u);
        lid_of.insert((lk, n), lid.clone());
        width_of.insert(lid, w);
    }
    let mut n_sumo: HashMap<i64, usize> = HashMap::new();
    for id in net.lanes.keys().filter(|id| !id.starts_with(':')) {
        if let Some(e) = id.rsplit_once('_').and_then(|x| x.0.parse::<i64>().ok()) {
            *n_sumo.entry(e).or_default() += 1;
        }
    }
    let bad: BTreeSet<i64> = ours.iter().filter(|(lk, ls)| n_sumo.get(lk) != Some(&ls.len())).map(|(lk, _)| *lk).collect();
    for lk in &bad {
        check.push(("lane count".into(), lk.to_string(), format!("duckOSM {} lanes, SUMO {}", ours[lk].len(), n_sumo.get(lk).copied().unwrap_or(0))));
    }
    let num = |edge: i64, idx: i64| -> i64 {
        let ls = &ours[&edge];
        ls[ls.len() - 1 - idx as usize]
    };
    let sumo_lane = |edge: i64, n: i64| -> String {
        let ls = &ours[&edge];
        format!("{edge}_{}", ls.len() - 1 - ls.iter().position(|x| *x == n).unwrap())
    };
    let mut by_pair: HashMap<(i64, i64), Vec<(i64, i64, Option<String>)>> = HashMap::new();
    for (f, fl, t, tl, via) in &net.conns {
        let (Ok(f), Ok(t)) = (f.parse::<i64>(), t.parse::<i64>()) else { continue };
        if bad.contains(&f) || bad.contains(&t) || !ours.contains_key(&f) || !ours.contains_key(&t) {
            continue;
        }
        by_pair.entry((f, t)).or_default().push((num(f, *fl), num(t, *tl), via.clone()));
    }
    // the movements, each made one row for each run of lanes
    let sql = format!("SELECT mvmt_id, ib_link_id, ob_link_id, type FROM {sch}.movement");
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let mv: Vec<(String, i64, i64, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
    // (old id, new id, lanes, bike row, kept as it was: no connection)
    type Row = (String, String, Option<(i64, i64, i64, i64)>, bool, bool);
    let mut rows: Vec<Row> = Vec::new();
    let mut connectors: Vec<(String, String, String, String, f64, Vec<P>)> = Vec::new();
    let mut row_ends: Vec<(i64, i64, Option<(i64, i64, i64, i64)>, bool, String)> = Vec::new(); // (ib, ob, lanes, bike, id)
    for (mid, ib, ob, typ) in &mv {
        let empty = Vec::new();
        let got = by_pair.get(&(*ib, *ob)).unwrap_or(&empty);
        if got.is_empty() {
            rows.push((mid.clone(), mid.clone(), None, false, true));
            row_ends.push((*ib, *ob, None, false, mid.clone()));
            check.push(("no lane connection".into(), mid.clone(), format!("{typ} {ib} -> {ob}: netconvert built none")));
            continue;
        }
        let is_bike = |a: i64, b: i64| mode == "driving" && (use_of.get(&(*ib, a)).map(|u| u == "bike").unwrap_or(false) || use_of.get(&(*ob, b)).map(|u| u == "bike").unwrap_or(false));
        let bike: Vec<(i64, i64)> = got.iter().filter(|x| is_bike(x.0, x.1)).map(|x| (x.0, x.1)).collect();
        let motor: Vec<(i64, i64)> = got.iter().map(|x| (x.0, x.1)).filter(|p| !bike.contains(p)).collect();
        let mut mid_of: HashMap<(i64, i64), String> = HashMap::new();
        for (k, run) in runs(&motor).iter().enumerate() {
            let m = if k == 0 { mid.clone() } else { format!("{mid}-{}", k + 1) };
            let lanes = (run[0].0, run[run.len() - 1].0, run[0].1, run[run.len() - 1].1);
            rows.push((mid.clone(), m.clone(), Some(lanes), false, false));
            row_ends.push((*ib, *ob, Some(lanes), false, m.clone()));
            for p in run {
                mid_of.insert(*p, m.clone());
            }
        }
        if motor.is_empty() {
            rows.push((mid.clone(), mid.clone(), None, false, false));
            row_ends.push((*ib, *ob, None, false, mid.clone()));
        }
        for &(a, b) in &bike {
            let m = format!("{mid}-b{a}");
            rows.push((mid.clone(), m.clone(), Some((a, a, b, b)), true, false));
            row_ends.push((*ib, *ob, Some((a, a, b, b)), true, m.clone()));
            mid_of.insert((a, b), m);
        }
        if geometry {
            for (a, b, via) in got {
                let (la, lb) = (lid_of[&(*ib, *a)].clone(), lid_of[&(*ob, *b)].clone());
                let mut shape = path_of(via.as_deref(), &net);
                let ea = net.lanes.get(&sumo_lane(*ib, *a)).map(|x| x.0.clone()).unwrap_or_default();
                let eb = net.lanes.get(&sumo_lane(*ob, *b)).map(|x| x.0.clone()).unwrap_or_default();
                if shape.len() < 2 {
                    shape = if !ea.is_empty() && !eb.is_empty() { vec![*ea.last().unwrap(), eb[0]] } else { vec![] };
                } else if !ea.is_empty() && !eb.is_empty() {
                    let inner = shape[1..shape.len() - 1].to_vec();
                    shape = std::iter::once(*ea.last().unwrap()).chain(inner).chain(std::iter::once(eb[0])).collect();
                }
                if shape.len() >= 2 {
                    let w = width_of.get(&la).copied().flatten().unwrap_or(3.25).min(width_of.get(&lb).copied().flatten().unwrap_or(3.25));
                    connectors.push((format!("{la}>{lb}"), mid_of[&(*a, *b)].clone(), la, lb, w, shape));
                }
            }
        }
    }
    // the rows go back into movement: a row of no connection keeps its lanes (-1 marks it)
    apply_rows(c, sch, &rows)?;
    // every lane of a link with movements has a way out and a way in (driving: the car lanes)
    let (mut outs, mut ins): (HashMap<i64, HashSet<i64>>, HashMap<i64, HashSet<i64>>) = (HashMap::new(), HashMap::new());
    for (ib, ob, lanes, _, _) in &row_ends {
        if let Some((a0, a1, b0, b1)) = lanes {
            outs.entry(*ib).or_default().extend(*a0..=*a1);
            ins.entry(*ob).or_default().extend(*b0..=*b1);
        }
    }
    let has_out: HashSet<i64> = row_ends.iter().map(|r| r.0).collect();
    let has_in: HashSet<i64> = row_ends.iter().map(|r| r.1).collect();
    check.extend(unmatched);
    let marked_of: HashMap<i64, &BTreeMap<i64, Option<String>>> = marked.iter().map(|(lk, ls)| (*lk, ls)).collect();
    let ranks = |lk: i64, n: i64| -> Option<BTreeSet<i64>> {
        let t = marked_of.get(&lk)?.get(&n)?.as_deref().filter(|t| !t.is_empty())?;
        Some(t.split(';').filter_map(|p| rank(p.trim(), drive_side)).collect())
    };
    for (ib, ob, lanes, bike, id) in &row_ends {
        let Some(d) = to_dir.get(&(*ib, *ob)) else { continue };
        let Some((a0, a1, _, _)) = lanes else { continue };
        if *bike {
            continue;
        }
        for n in *a0..=*a1 {
            if let Some(rs) = ranks(*ib, n) {
                if !rs.contains(d) {
                    let t = marked_of[ib][&n].clone().unwrap_or_default();
                    check.push(("against its arrow".into(), lid_of[&(*ib, n)].clone(), format!("{id}: lane {n} marked {} goes to the exit of {}", py_repr(&t), py_repr(rank_name(*d)))));
                }
            }
        }
    }
    for ((lk, n), u) in &use_of {
        if (mode == "driving" && u == "bike") || bad.contains(lk) {
            continue;
        }
        if has_out.contains(lk) && !outs.get(lk).is_some_and(|s| s.contains(n)) {
            check.push(("no way out".into(), lid_of[&(*lk, *n)].clone(), "no movement leaves this lane".into()));
        }
        if has_in.contains(lk) && !ins.get(lk).is_some_and(|s| s.contains(n)) {
            check.push(("no way in".into(), lid_of[&(*lk, *n)].clone(), "no movement enters this lane".into()));
        }
    }
    if geometry {
        // SUMO's lane line (to the junction) as geom_cut, and the paths through the junctions; the shapes back to lon/lat
        let mut cut: Vec<(String, Vec<P>)> = Vec::new();
        for (lk, nums) in &ours {
            if bad.contains(lk) {
                continue;
            }
            for n in nums {
                if let Some((shape, _)) = net.lanes.get(&sumo_lane(*lk, *n)).filter(|x| x.0.len() >= 2) {
                    cut.push((lid_of[&(*lk, *n)].clone(), shape.clone()));
                }
            }
        }
        c.exec("CREATE OR REPLACE TEMP TABLE _xy(id BIGINT, k BIGINT, x DOUBLE, y DOUBLE)")?;
        {
            let mut app = c.con.appender("_xy").map_err(|e| e.to_string())?;
            for (i, sh) in cut.iter().map(|x| &x.1).chain(connectors.iter().map(|x| &x.5)).enumerate() {
                for (k, p) in sh.iter().enumerate() {
                    app.append_row(duckdb::params![i as i64, k as i64, p.0 - net.off.0, p.1 - net.off.1]).map_err(|e| e.to_string())?;
                }
            }
            app.flush().map_err(|e| e.to_string())?;
        }
        let proj = net.proj.replace('\'', "''");
        // the points of each shape in lon/lat, joined in their order; the sort spills to disk, a string_agg of all shapes
        // did not (Kentucky: out of memory at 12.7 GB)
        let mut ll: Vec<String> = vec![String::new(); cut.len() + connectors.len()];
        let sql = format!(
            "SELECT id, ST_X(g), ST_Y(g) FROM (SELECT id, k, ST_Transform(ST_Point(x, y), '{proj}', 'EPSG:4326', always_xy := true) AS g FROM _xy) \
             ORDER BY id, k"
        );
        crate::stream::each_chunk(c.con, &sql, [], |ch| {
            for i in 0..ch.len {
                if let (Some(id), Some(x), Some(y)) = (ch.i64(0, i), ch.f64(1, i), ch.f64(2, i)) {
                    let s = &mut ll[id as usize];
                    if !s.is_empty() {
                        s.push_str(", ");
                    }
                    s.push_str(&format!("{x:.8} {y:.8}"));
                }
            }
            Ok(())
        })
        .map_err(|e| format!("{e}\n{sql}"))?;
        c.exec(&format!(
            "ALTER TABLE {sch}.lane ADD COLUMN IF NOT EXISTS geom_cut GEOMETRY;
             CREATE OR REPLACE TABLE {sch}.lane_connector(connector_id VARCHAR, mvmt_id VARCHAR, from_lane_id VARCHAR, to_lane_id VARCHAR, width DOUBLE, geom GEOMETRY);
             CREATE OR REPLACE TEMP TABLE _cut(lane_id VARCHAR, wkt VARCHAR);
             DROP TABLE _xy"
        ))?;
        {
            let mut app = c.con.appender("_cut").map_err(|e| e.to_string())?;
            for (i, (lid, _)) in cut.iter().enumerate() {
                app.append_row(duckdb::params![lid, format!("LINESTRING ({})", ll[i])]).map_err(|e| e.to_string())?;
            }
            app.flush().map_err(|e| e.to_string())?;
        }
        c.exec("CREATE OR REPLACE TEMP TABLE _con(connector_id VARCHAR, mvmt_id VARCHAR, from_lane_id VARCHAR, to_lane_id VARCHAR, width DOUBLE, wkt VARCHAR)")?;
        {
            let mut app = c.con.appender("_con").map_err(|e| e.to_string())?;
            for (i, (id, m, a, b, w, _)) in connectors.iter().enumerate() {
                app.append_row(duckdb::params![id, m, a, b, w, format!("LINESTRING ({})", ll[cut.len() + i])]).map_err(|e| e.to_string())?;
            }
            app.flush().map_err(|e| e.to_string())?;
        }
        c.exec(&format!(
            "UPDATE {sch}.lane l SET geom_cut = ST_GeomFromText(c.wkt) FROM _cut c WHERE c.lane_id = l.lane_id;
             INSERT INTO {sch}.lane_connector SELECT connector_id, mvmt_id, from_lane_id, to_lane_id, width, ST_GeomFromText(wkt) FROM _con;
             DROP TABLE _cut; DROP TABLE _con"
        ))?;
    }
    if !check.is_empty() {
        let mut app = c.con.appender_to_db("lane_check", sch).map_err(|e| e.to_string())?;
        for (k, i, d) in &check {
            app.append_row(duckdb::params![k, i, d]).map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
    }
    eprintln!(
        "GMNS[{}] lanes from SUMO: {} movements -> {} rows, {} connectors, {} lane-count mismatches, {} checks",
        mode,
        mv.len(),
        rows.len(),
        connectors.len(),
        bad.len(),
        check.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lane pairs that go on side by side are one movement row; a lane feeding two lanes ahead is two rows (ported from
    /// tests/test_gmns.py).
    #[test]
    fn lane_pairs_become_runs_side_by_side() {
        assert_eq!(runs(&[(1, 1), (2, 2)]), vec![vec![(1, 1), (2, 2)]]);
        assert_eq!(runs(&[(2, 1), (2, 2)]), vec![vec![(2, 1)], vec![(2, 2)]]);
        assert_eq!(runs(&[(1, 1), (1, 2), (2, 3)]), vec![vec![(1, 1)], vec![(1, 2), (2, 3)]]);
    }
}

