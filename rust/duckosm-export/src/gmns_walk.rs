//! `--walk-frame`: the footpath lanes moved off the road, the port of `_walk_frame`, `_walk_kerb` and `_walk_join` of
//! src/duckosm/gmns.py (docs/design/gmns_walking_frame.md). Lane geometry only: ids, lengths, lane numbers and movements stay.
//! The lines are checked by their properties, not by equality with Python (docs/design/all_in_rust.md).

use crate::gmns::{Ctx, DEFAULT_LANE_W, DEFAULT_WALK_W, NON_MOTOR};
use crate::shapely::{self as shp, P};
use crate::Result;
use geo::{BooleanOps, Buffer, MultiLineString};
use std::collections::HashMap;

const M: f64 = 111320.0;

/// A local metric frame at (x0, y0): to metres and back.
struct Frame {
    x0: f64,
    y0: f64,
    kx: f64,
}

impl Frame {
    fn new(x0: f64, y0: f64) -> Frame {
        let kx = y0.to_radians().cos();
        Frame { x0, y0, kx: if kx == 0.0 { 1.0 } else { kx } }
    }
    fn to_m(&self, l: &[P]) -> Vec<P> {
        l.iter().map(|(x, y)| ((x - self.x0) * self.kx * M, (y - self.y0) * M)).collect()
    }
    fn to_ll(&self, l: &[P]) -> Vec<P> {
        l.iter().map(|(x, y)| (x / (self.kx * M) + self.x0, y / M + self.y0)).collect()
    }
}

/// Write the new lines `out` (lane id, lon/lat line) of schema `sch`.
fn set_geoms(c: &Ctx, sch: &str, out: &[(String, Vec<P>)]) -> Result<()> {
    if out.is_empty() {
        return Ok(());
    }
    c.exec("CREATE OR REPLACE TEMP TABLE _walk_geom(lane_id VARCHAR, wkt VARCHAR)")?;
    {
        let mut app = c.con.appender("_walk_geom").map_err(|e| e.to_string())?;
        for (id, l) in out {
            let pts: Vec<String> = l.iter().map(|(x, y)| format!("{x} {y}")).collect();
            app.append_row(duckdb::params![id, format!("LINESTRING ({})", pts.join(", "))]).map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
    }
    c.exec(&format!(
        "UPDATE {sch}.lane SET geom = ST_GeomFromText(w.wkt) FROM _walk_geom w WHERE {sch}.lane.lane_id::VARCHAR = w.lane_id; DROP TABLE _walk_geom"
    ))
}

fn has_driving_lanes(c: &Ctx) -> Result<bool> {
    Ok(c.count("SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'gmns_driving' AND table_name = 'lane'")? > 0)
}

/// `_walk_frame`: a mapped sidewalk (`along`: its link -> the road it runs along) on its road's kerb side becomes an offset curve
/// of the road's kerb-side lane, `clearance_m` outside its edge, where it runs alongside the road. The road's lanes in
/// `gmns_driving` are the frame where there are some.
pub(crate) fn walk_frame(c: &Ctx, sch: &str, drive_side: &str, along: &HashMap<i64, i64>, clearance_m: f64) -> Result<()> {
    let kids: Vec<(String, i64, i64, String, Option<f64>, String)> = {
        let sql = format!(
            "SELECT l.lane_id::VARCHAR, l.link_id, ST_AsText(l.geom), l.width, ST_AsText(k.geom) FROM {sch}.lane l
             JOIN {sch}.link k ON k.link_id = l.link_id WHERE l.geom IS NOT NULL AND l.allowed_uses = 'walk'"
        );
        let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
        let v: Vec<(String, i64, String, Option<f64>, String)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| e.to_string())?;
        v.into_iter().filter_map(|r| along.get(&r.1).map(|p| (r.0, r.1, *p, r.2, r.3, r.4))).collect()
    };
    if kids.is_empty() {
        return Ok(());
    }
    let kerb = if drive_side == "right" { -1.0 } else { 1.0 };
    // link -> (its lanes (num, line, width), its own line)
    type Road = (Vec<(i64, String, Option<f64>)>, String);
    let roads = |g: &str| -> Result<HashMap<i64, Road>> {
        let sql = format!("SELECT link_id, ST_AsText(geom) FROM {g}.link");
        let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
        let line: HashMap<i64, String> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
        let sql = format!("SELECT link_id, lane_num, ST_AsText(geom), width FROM {g}.lane WHERE geom IS NOT NULL");
        let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
        let mut lanes: HashMap<i64, Vec<(i64, String, Option<f64>)>> = HashMap::new();
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(|e| e.to_string())? {
            let (lk, num, w, wd) = r.map_err(|e| e.to_string())?;
            lanes.entry(lk).or_default().push((num, w, wd));
        }
        Ok(lanes.into_iter().filter_map(|(lk, v)| line.get(&lk).map(|l| (lk, (v, l.clone())))).collect())
    };
    let mut frame = roads(sch)?;
    if has_driving_lanes(c)? {
        frame.extend(roads("gmns_driving")?);
    }
    let (mut out, mut why): (Vec<(String, Vec<P>)>, Vec<(&str, i64)>) = (Vec::new(), Vec::new());
    let mut skip = |k: &'static str| match why.iter_mut().find(|w| w.0 == k) {
        Some(w) => w.1 += 1,
        None => why.push((k, 1)),
    };
    for (lane_id, _, parent, cw, ws, lw) in &kids {
        let Some((planes, pline)) = frame.get(parent) else {
            skip("no_road_lane");
            continue;
        };
        // the kerb-side lane: lane 1 is the leftmost, so the last lane on the right with right-hand traffic
        let cmp = |a: &&(i64, String, Option<f64>), b: &&(i64, String, Option<f64>)| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        let (_, pw, wp) = if drive_side == "right" { planes.iter().max_by(cmp) } else { planes.iter().min_by(cmp) }.unwrap();
        let wp = wp.filter(|w| *w != 0.0).unwrap_or(DEFAULT_LANE_W);
        let ws = ws.filter(|w| *w != 0.0).unwrap_or(DEFAULT_WALK_W);
        let (cg, pg, lg, rg) = (shp::wkt_points(cw), shp::wkt_points(pw), shp::wkt_points(lw), shp::wkt_points(pline));
        if cg.is_empty() || pg.is_empty() || lg.is_empty() || rg.is_empty() {
            skip("degenerate");
            continue;
        }
        let f = Frame::new(pg[0].0, pg[0].1);
        let (cl, pl, ln, rn) = (f.to_m(&cg), f.to_m(&pg), f.to_m(&lg), f.to_m(&rg));
        if shp::length(&cl) < 1.0 || shp::length(&pl) < 1.0 || shp::length(&ln) < 1.0 {
            skip("short");
            continue;
        }
        let mid = shp::interpolate(&ln, 0.5 * shp::length(&ln));
        let (t, rlen) = (shp::project(&rn, mid), shp::length(&rn));
        let (a, b, q) = (shp::interpolate(&rn, (t - 1.0).max(0.0)), shp::interpolate(&rn, (t + 1.0).min(rlen)), shp::interpolate(&rn, t));
        let side = (b.0 - a.0) * (mid.1 - q.1) - (b.1 - a.1) * (mid.0 - q.0);
        if side * kerb <= 0.0 {
            skip("far_side"); // the far side of the road link: left as it is
            continue;
        }
        let Ok(new) = crate::offset::offset_curve(&pl, kerb * (wp / 2.0 + clearance_m + ws / 2.0)) else {
            skip("offset_failed");
            continue;
        };
        if new.len() != 1 || new[0].len() < 2 || shp::point_line_distance(mid, &new[0]) > 12.0 {
            skip("offset_far");
            continue;
        }
        let new = &new[0];
        // move the footpath's own lane onto the frame where it runs alongside the road, vertex by vertex (it keeps its length,
        // its ends and its way on past the road): beyond the stretch the road covers the move fades out over 10 m
        let unit = |a: P, b: P| {
            let n = (b.0 - a.0).hypot(b.1 - a.1);
            let n = if n == 0.0 { 1.0 } else { n };
            ((b.0 - a.0) / n, (b.1 - a.1) / n)
        };
        let k = new.len();
        let (a0, b1) = (new[0], new[k - 1]);
        let (t0, t1) = (unit(a0, new[1]), unit(new[k - 2], b1));
        let moved: Vec<P> = shp::segmentize(&cl, 2.0)
            .into_iter()
            .map(|v| {
                let tp = shp::interpolate(new, shp::project(new, v));
                let over = 0f64.max(-((v.0 - a0.0) * t0.0 + (v.1 - a0.1) * t0.1)).max((v.0 - b1.0) * t1.0 + (v.1 - b1.1) * t1.1);
                let w = (1.0 - over / 10.0).max(0.0);
                (v.0 + w * (tp.0 - v.0), v.1 + w * (tp.1 - v.1))
            })
            .collect();
        if shp::length(&moved) < 0.5 {
            skip("cut_short");
            continue;
        }
        out.push((lane_id.clone(), f.to_ll(&moved)));
    }
    set_geoms(c, sch, &out)?;
    why.sort_by(|a, b| b.1.cmp(&a.1)); // stable: ties keep the order they were first seen, as Counter.most_common
    let left: Vec<String> = why.iter().map(|(k, n)| format!("{} {}", crate::cli::thousands(*n), k.replace('_', " "))).collect();
    eprintln!(
        "GMNS[{}]: {} footpath lanes placed from their road's frame; left as they were: {}",
        &sch[5..],
        crate::cli::thousands(out.len() as i64),
        if left.is_empty() { "none".to_string() } else { left.join(", ") }
    );
    Ok(())
}

/// `_walk_kerb`: a mapped sidewalk that still lies on a road is pushed out to the kerb: each vertex within a driving lane that runs
/// along the sidewalk moves to that road's edge plus half the sidewalk's width (+ `clearance_m`).
pub(crate) fn walk_kerb(c: &Ctx, sch: &str, clearance_m: f64) -> Result<()> {
    if !has_driving_lanes(c)? {
        return Ok(());
    }
    let sql = format!(
        "SELECT l.lane_id::VARCHAR, ST_AsText(l.geom), l.width FROM {sch}.lane l JOIN {sch}.link k ON k.link_id = l.link_id
         WHERE k.footway = 'sidewalk' AND l.geom IS NOT NULL AND l.allowed_uses = 'walk'"
    );
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let kids: Vec<(String, String, Option<f64>)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT ST_AsText(geom), COALESCE(width, {DEFAULT_LANE_W:?}) FROM gmns_driving.lane
         WHERE geom IS NOT NULL AND allowed_uses IN ('auto', 'bus', 'bus,bike')"
    );
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let roads: Vec<(String, f64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
    if kids.is_empty() || roads.is_empty() {
        return Ok(());
    }
    let p0 = shp::wkt_points(&roads[0].0)[0];
    let f = Frame::new(p0.0, p0.1);
    let lanes: Vec<Vec<P>> = roads.iter().map(|r| f.to_m(&shp::wkt_points(&r.0))).collect();
    let mut polys = Vec::with_capacity(lanes.len());
    for (ln, (_, wd)) in lanes.iter().zip(&roads) {
        polys.push(crate::crossings::mp(&crate::offset::line_buffer(ln, wd / 2.0, true)?));
    }
    let envs: Vec<(f64, f64, f64, f64)> = polys
        .iter()
        .map(|m| m.0.iter().flat_map(|p| p.exterior().0.iter()).fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |e, c| (e.0.min(c.x), e.1.min(c.y), e.2.max(c.x), e.3.max(c.y))))
        .collect();
    let order = shp::strtree_order(&envs);
    let mut out = Vec::new();
    for (lane_id, wkt, wd) in &kids {
        let half = wd.unwrap_or(DEFAULT_WALK_W) / 2.0 + clearance_m;
        let pts = shp::segmentize(&f.to_m(&shp::wkt_points(wkt)), 2.0);
        if pts.len() < 2 {
            continue;
        }
        let tang: Vec<P> = shp::gradient(&pts)
            .into_iter()
            .map(|(x, y)| {
                let n = x.hypot(y).max(1e-9);
                (x / n, y / n)
            })
            .collect();
        let (mut moved, mut changed) = (pts.clone(), false);
        for (i, &v) in pts.iter().enumerate() {
            let near: Vec<usize> = order
                .iter()
                .copied()
                .filter(|&j| {
                    let e = envs[j];
                    if e.0 > v.0 + 25.0 || e.2 < v.0 - 25.0 || e.1 > v.1 + 25.0 || e.3 < v.1 - 25.0 {
                        return false;
                    }
                    let lane = &lanes[j];
                    let s0 = shp::project(lane, v);
                    let (a, b) = (shp::interpolate(lane, (s0 - 0.5).max(0.0)), shp::interpolate(lane, (s0 + 0.5).min(shp::length(lane))));
                    let n = (b.0 - a.0).hypot(b.1 - a.1);
                    let n = if n == 0.0 { 1.0 } else { n };
                    (((b.0 - a.0) * tang[i].0 + (b.1 - a.1) * tang[i].1) / n).abs() >= 0.7 // a lane that runs along the sidewalk
                })
                .collect();
            if near.is_empty() {
                continue;
            }
            let (nx, ny) = (-tang[i].1, tang[i].0); // across the sidewalk: across the road
            let mut u = polys[near[0]].clone();
            for &j in &near[1..] {
                u = u.union(&polys[j]);
            }
            let road = u.buffer(0.05); // lanes that touch are one road
            let across = vec![(v.0 - nx * 25.0, v.1 - ny * 25.0), (v.0 + nx * 25.0, v.1 + ny * 25.0)];
            let cut = road.clip(&MultiLineString(vec![crate::crossings::ls(&across)]), false);
            // the road's stretches along the line, as (from, to) distances from v, less the 0.05 m the road was grown by
            let ts: Vec<(f64, f64)> = crate::crossings::line_merge(&cut)
                .iter()
                .map(|g| {
                    let u: Vec<f64> = g.iter().map(|(x, y)| (x - v.0) * nx + (y - v.1) * ny).collect();
                    (u.iter().copied().fold(f64::INFINITY, f64::min) + 0.05, u.iter().copied().fold(f64::NEG_INFINITY, f64::max) - 0.05)
                })
                .collect();
            let here: Vec<(f64, f64)> = ts.into_iter().filter(|t| t.0 - half <= 0.0 && 0.0 <= t.1 + half).collect();
            let key = |t: &(f64, f64)| if t.0 <= 0.0 && 0.0 <= t.1 { 0.0 } else { t.0.abs().min(t.1.abs()) };
            let Some(&(t0, t1)) = here.iter().reduce(|a, b| if key(b) < key(a) { b } else { a }) else {
                continue;
            };
            let shift = if t0 <= 0.0 && 0.0 <= t1 {
                if t1 <= -t0 { t1 + half } else { t0 - half } // inside the road: out through the nearer kerb
            } else if t0 > 0.0 {
                t0 - half
            } else {
                t1 + half
            };
            if shift.abs() > 1e-6 {
                moved[i] = (v.0 + nx * shift, v.1 + ny * shift);
                changed = true;
            }
        }
        if changed {
            for _ in 0..2 {
                // a light smoothing: no ragged edge where the push changes
                let old = moved.clone();
                for k in 1..old.len() - 1 {
                    moved[k] = ((old[k - 1].0 + 2.0 * old[k].0 + old[k + 1].0) / 4.0, (old[k - 1].1 + 2.0 * old[k].1 + old[k + 1].1) / 4.0);
                }
            }
            out.push((lane_id.clone(), f.to_ll(&moved)));
        }
    }
    set_geoms(c, sch, &out)?;
    eprintln!("GMNS[{}]: {} sidewalk lanes pushed out of a road, to its kerb", &sch[5..], crate::cli::thousands(out.len() as i64));
    Ok(())
}

/// `_walk_join`: every footway lane that ends at a node where a moved sidewalk ends, ends at the mean of the moved sidewalk ends.
pub(crate) fn walk_join(c: &Ctx, sch: &str) -> Result<()> {
    let nonroad = NON_MOTOR.iter().map(|h| format!("'{h}'")).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT l.lane_id::VARCHAR, k.from_node_id, k.to_node_id, k.footway, ST_AsText(l.geom)
         FROM {sch}.lane l JOIN {sch}.link k ON k.link_id = l.link_id
         WHERE l.geom IS NOT NULL AND l.allowed_uses = 'walk'
           AND regexp_replace(split_part(k.facility_type, ';', 1), '_link$', '') IN ({nonroad})"
    );
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let rows: Vec<(String, i64, i64, Option<String>, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let sql = format!("SELECT node_id, x_coord, y_coord FROM {sch}.node");
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let node_rows: Vec<(i64, f64, f64)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
    let nodes: HashMap<i64, P> = node_rows.iter().map(|r| (r.0, (r.1, r.2))).collect();
    if rows.is_empty() || nodes.is_empty() {
        return Ok(());
    }
    let f = Frame::new(node_rows[0].1, node_rows[0].2);
    let mut lanes: HashMap<String, Vec<P>> = HashMap::new();
    let (mut ends, mut node_order): (HashMap<i64, Vec<(String, bool, bool)>>, Vec<i64>) = (HashMap::new(), Vec::new());
    for (lane_id, fnode, tnode, fw, wkt) in rows {
        let pts = f.to_m(&shp::wkt_points(&wkt));
        if pts.len() < 2 || !nodes.contains_key(&fnode) || !nodes.contains_key(&tnode) {
            continue;
        }
        lanes.insert(lane_id.clone(), pts);
        let side = fw.as_deref() == Some("sidewalk");
        for (nd, last) in [(fnode, false), (tnode, true)] {
            if !ends.contains_key(&nd) {
                node_order.push(nd);
            }
            ends.entry(nd).or_default().push((lane_id.clone(), last, side));
        }
    }
    let (mut changed, mut seen): (Vec<String>, std::collections::HashSet<String>) = (Vec::new(), Default::default());
    for nd in node_order {
        let lst = &ends[&nd];
        if lst.len() < 2 {
            continue;
        }
        let home = f.to_m(&[nodes[&nd]])[0];
        let end = |lanes: &HashMap<String, Vec<P>>, ln: &str, last: bool| {
            let l = &lanes[ln];
            if last { l[l.len() - 1] } else { l[0] }
        };
        let moved: Vec<P> = lst.iter().filter(|(ln, last, side)| *side && shp::dist(end(&lanes, ln, *last), home) > 0.05).map(|(ln, last, _)| end(&lanes, ln, *last)).collect();
        if moved.is_empty() {
            continue;
        }
        let n = moved.len() as f64;
        let target = (moved.iter().map(|p| p.0).sum::<f64>() / n, moved.iter().map(|p| p.1).sum::<f64>() / n);
        for (ln, last, _) in lst {
            if shp::dist(end(&lanes, ln, *last), target) > 0.02 {
                let l = lanes.get_mut(ln).unwrap();
                let k = if *last { l.len() - 1 } else { 0 };
                l[k] = target;
                if seen.insert(ln.clone()) {
                    changed.push(ln.clone());
                }
            }
        }
    }
    let out: Vec<(String, Vec<P>)> = changed.into_iter().map(|ln| (ln.clone(), f.to_ll(&lanes[&ln]))).collect();
    set_geoms(c, sch, &out)?;
    eprintln!("GMNS[{}]: {} footway lanes joined at the nodes their sidewalks were moved from", &sch[5..], crate::cli::thousands(out.len() as i64));
    Ok(())
}
