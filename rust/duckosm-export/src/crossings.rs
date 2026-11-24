//! GMNS crossings: where a zebra is painted, on which lanes, on which stretch of each. The port of src/duckosm/crossings.py
//! (docs/design/gmns_crossings.md): `crossing` (one row for each crossing) and `lane_crossing` (one row for each lane it covers).
//! The lane surfaces are GEOS's flat-capped buffers (`offset::line_buffer`); the unions, intersections and clips are those of the
//! `geo` crate, so the stretches and widths agree with Python within a millimetre, not to the last bit.

use crate::gmns::Ctx;
use crate::shapely::{self as shp, P};
use crate::Result;
use geo::{Area, BooleanOps, Buffer, Centroid, Distance, Euclidean};
use geo::{Coord, LineString, MultiLineString, MultiPolygon, Polygon};
use std::collections::{HashMap, HashSet};

const ROAD_USES: &str = "('auto', 'bus', 'bus,bike')";
const DEFAULT_LANE_W: f64 = 3.25;
const ZEBRA_W_PER_ROAD_W: f64 = 0.4;
const ZEBRA_W_MIN: f64 = 2.5;
const ZEBRA_W_MAX: f64 = 4.0;
const MIN_ANGLE_SIN: f64 = 0.5;
const MAX_WIND: f64 = 1.3;
const LOOP_NODES: usize = 3;
const MAX_SAME_OVERLAP: f64 = 0.5;
const ROAD_REACH_M: f64 = 3.5;
const ROAD_ANGLE: f64 = 30.0;
const MIN_ALONG_CONNECTOR: f64 = 0.5;
const MAX_OVERLAP: f64 = 0.10;
const NODE_REACH_M: f64 = 25.0;

/// `_painted`: stripes painted, by the OSM tags.
fn painted(crossing: Option<&str>, markings: Option<&str>) -> bool {
    if crossing == Some("unmarked") || markings == Some("no") {
        return false;
    }
    markings.is_some() || matches!(crossing, Some("marked") | Some("zebra") | Some("uncontrolled"))
}

/// `_num`: a width tag between 0.5 and 20 m, else None.
fn num(v: Option<&str>) -> Option<f64> {
    let w: f64 = v?.replace(',', ".").trim().parse().ok()?;
    (0.5..=20.0).contains(&w).then_some(w)
}

fn level(ly: &Option<String>, br: &Option<String>, tu: &Option<String>) -> i64 {
    crate::gmns::level(ly, br, tu)
}

pub(crate) fn ls(l: &[P]) -> LineString<f64> {
    LineString(l.iter().map(|(x, y)| Coord { x: *x, y: *y }).collect())
}

pub(crate) fn mp(polys: &[crate::geos::Polygon]) -> MultiPolygon<f64> {
    let ring = |r: &Vec<P>| LineString(r.iter().map(|(x, y)| Coord { x: *x, y: *y }).collect());
    MultiPolygon(polys.iter().map(|p| Polygon::new(ring(&p.shell), p.holes.iter().map(ring).collect())).collect())
}

fn env_of(m: &MultiPolygon<f64>) -> (f64, f64, f64, f64) {
    m.0.iter().flat_map(|p| p.exterior().0.iter()).fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |e, c| {
        (e.0.min(c.x), e.1.min(c.y), e.2.max(c.x), e.3.max(c.y))
    })
}

fn meet(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    !(a.0 > b.2 || a.2 < b.0 || a.1 > b.3 || a.3 < b.1)
}

struct Lanes {
    id: Vec<String>,
    link: Vec<i64>,
    way: Vec<i64>,
    width: Vec<f64>,
    level: Vec<i64>,
    conn: Vec<bool>,
    geom: Vec<Vec<P>>,
    poly: Vec<MultiPolygon<f64>>,
    gpoly: Vec<Vec<crate::geos::Polygon>>,
    env: Vec<(f64, f64, f64, f64)>,
    tree: Vec<usize>,
    /// the position of each lane in `tree`, and the lanes of each grid cell (`index`)
    rank: Vec<usize>,
    grid: HashMap<(i64, i64), Vec<usize>>,
}

/// The side of a grid cell of `Lanes::grid`, in metres.
const CELL_M: f64 = 100.0;

fn cells(e: (f64, f64, f64, f64)) -> impl Iterator<Item = (i64, i64)> {
    let c = |v: f64| (v / CELL_M).floor() as i64;
    let (x0, y0, x1, y1) = (c(e.0), c(e.1), c(e.2), c(e.3));
    (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
}

impl Lanes {
    /// shapely's `STRtree(poly).query(geom)`: the lanes whose envelope meets `e`, in the tree's order.
    /// A grid in place of the tree: a scan of all lanes for each query took hours on a large tile (Paris).
    fn query(&self, e: (f64, f64, f64, f64)) -> Vec<usize> {
        let mut v: Vec<usize> = cells(e).filter_map(|k| self.grid.get(&k)).flatten().copied().filter(|&j| meet(self.env[j], e)).collect();
        v.sort_unstable_by_key(|&j| self.rank[j]);
        v.dedup();
        v
    }
    fn index(&mut self) {
        self.rank = vec![0; self.tree.len()];
        for (r, &j) in self.tree.iter().enumerate() {
            self.rank[j] = r;
        }
        // an empty surface has an infinite envelope, which meets nothing
        for (j, e) in self.env.iter().enumerate().filter(|(_, e)| e.0.is_finite() && e.2.is_finite()) {
            for k in cells(*e) {
                self.grid.entry(k).or_default().push(j);
            }
        }
    }
    fn tangent(&self, j: usize, p: P) -> P {
        let g = &self.geom[j];
        // at a corner's bisector (to 1e-6 m) the corner: project jumped across it for a change of p in the last bits
        let s = shp::project_tol(g, p, 1e-6);
        let len = shp::length(g);
        let (a, b) = (shp::interpolate(g, (s - 0.5).max(0.0)), shp::interpolate(g, (s + 0.5).min(len)));
        let n = match (b.0 - a.0).hypot(b.1 - a.1) {
            0.0 => 1.0,
            n => n,
        };
        ((b.0 - a.0) / n, (b.1 - a.1) / n)
    }
}

/// The parts of `line` that lie across the road: inside the lanes' surface (within 0.1 m), straight enough, at 30 degrees or
/// more from the lane they lie on (`pieces`). Returns the parts and the lanes near the line.
fn pieces(l: &Lanes, line: &[P], only: Option<&HashSet<usize>>, lv: Option<i64>) -> (Vec<Vec<P>>, Vec<usize>) {
    let e = shp_env(line);
    let near: Vec<usize> = l
        .query((e.0 - 0.1, e.1 - 0.1, e.2 + 0.1, e.3 + 0.1))
        .into_iter()
        .filter(|j| only.is_none_or(|o| o.contains(j)) && lv.is_none_or(|v| l.level[*j] == v))
        .collect();
    if near.is_empty() {
        return (vec![], vec![]);
    }
    let mut u = l.poly[near[0]].clone();
    for &j in &near[1..] {
        u = u.union(&l.poly[j]);
    }
    let area = u.buffer_with_style(geo::algorithm::buffer::BufferStyle::new(0.1).line_join(geo::algorithm::buffer::LineJoin::Round(std::f64::consts::PI / 32.0)));
    let on: MultiLineString<f64> = area.clip(&MultiLineString(vec![ls(line)]), false);
    let parts = line_merge(&on);
    let mut out = Vec::new();
    for part in parts {
        let len = shp::length(&part);
        if len < 2.0 {
            continue;
        }
        let (a, b) = (part[0], *part.last().unwrap());
        let chord = (b.0 - a.0).hypot(b.1 - a.1);
        if chord < 1.0 || len > MAX_WIND * chord {
            continue;
        }
        let mid = shp::interpolate(&part, 0.5 * len);
        let lane = nearest(l, &near, mid);
        let (tx, ty) = l.tangent(lane, mid);
        if ((b.0 - a.0) * ty - (b.1 - a.1) * tx).abs() / chord < MIN_ANGLE_SIN {
            continue;
        }
        out.push(part);
    }
    (out, near)
}

/// shapely's `line_merge` of the pieces of a clip: pieces that meet end to start are one, in whatever order the clip gives them.
pub(crate) fn line_merge(on: &MultiLineString<f64>) -> Vec<Vec<P>> {
    let mut parts: Vec<Vec<P>> = on.0.iter().map(|g| g.0.iter().map(|c| (c.x, c.y)).collect::<Vec<P>>()).filter(|p| p.len() >= 2).collect();
    loop {
        let mut joined = false;
        'find: for i in 0..parts.len() {
            for j in 0..parts.len() {
                if i != j && shp::dist(*parts[i].last().unwrap(), parts[j][0]) < 1e-9 {
                    let tail = parts.remove(j);
                    let i = if j < i { i - 1 } else { i };
                    parts[i].extend_from_slice(&tail[1..]);
                    joined = true;
                    break 'find;
                }
            }
        }
        if !joined {
            break;
        }
    }
    parts
}

/// `_nearest`: the lane of `near` nearest to p; of those as near (within 1e-9 m), a lane before a connector, then the smallest
/// lane id.
fn nearest(l: &Lanes, near: &[usize], p: P) -> usize {
    let d: Vec<f64> = near.iter().map(|j| shp::point_line_distance(p, &l.geom[*j])).collect();
    let best = d.iter().copied().fold(f64::INFINITY, f64::min);
    *near.iter().zip(&d).filter(|(_, x)| **x <= best + 1e-9).map(|(j, _)| j).min_by(|a, b| l.conn[**a].cmp(&l.conn[**b]).then(l.id[**a].cmp(&l.id[**b]))).unwrap()
}

/// The parts of the line inside the polygons (GEOS's intersection of a line and a polygon): the line is cut where it crosses a
/// ring (GEOS's intersection points), and a piece stays when its middle is inside a shell and not inside a hole. Returns the
/// pieces' points.
fn clip(line: &[P], polys: &[crate::geos::Polygon]) -> Vec<Vec<P>> {
    // a piece on a ring (within 1e-6 m: GEOS's overlay takes a line along the boundary, such as a lane's flat end, as inside)
    let inside = |p: P| {
        polys.iter().any(|pg| {
            let on = |r: &Vec<P>| r.windows(2).any(|e| shp::point_to_segment(p, e[0], e[1]) <= 1e-6);
            on(&pg.shell) || pg.holes.iter().any(on) || (crate::geos::in_or_on_ring(p, &pg.shell) && !pg.holes.iter().any(|h| crate::geos::in_or_on_ring(p, h)))
        })
    };
    let mut out: Vec<Vec<P>> = Vec::new();
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len2 = (b.0 - a.0).powi(2) + (b.1 - a.1).powi(2);
        if len2 == 0.0 {
            continue;
        }
        let mut cuts: Vec<(f64, P)> = vec![(0.0, a), (1.0, b)];
        for pg in polys {
            for r in std::iter::once(&pg.shell).chain(pg.holes.iter()) {
                for e in r.windows(2) {
                    for x in crate::geos::intersection_points(a, b, e[0], e[1]) {
                        let t = ((x.0 - a.0) * (b.0 - a.0) + (x.1 - a.1) * (b.1 - a.1)) / len2;
                        cuts.push((t.clamp(0.0, 1.0), x));
                    }
                }
            }
        }
        cuts.sort_by(|x, y| x.0.total_cmp(&y.0));
        cuts.dedup_by(|x, y| x.1 == y.1);
        for k in cuts.windows(2) {
            let (p, q) = (k[0].1, k[1].1);
            if p == q || !inside(((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0)) {
                continue;
            }
            match out.last_mut() {
                Some(last) if *last.last().unwrap() == p => last.push(q),
                _ => out.push(vec![p, q]),
            }
        }
    }
    out
}

/// The flat-capped surface of a lane (metres), as `offset::line_buffer`. None: the buffer failed (it counts in `failed`).
fn surface(g: &[P], half_w: f64, failed: &mut usize) -> Option<Vec<crate::geos::Polygon>> {
    let r = crate::offset::line_buffer(g, half_w, true).ok();
    *failed += r.is_none() as usize;
    r
}

fn shp_env(l: &[P]) -> (f64, f64, f64, f64) {
    crate::gmns_lanes::envelope(l)
}

/// `build_crossings`: `crossing` and `lane_crossing` of a driving schema (empty without the raw OSM tags).
pub(crate) fn build_crossings(c: &Ctx, sch: &str, has_raw: bool) -> Result<()> {
    c.exec(&format!(
        "CREATE TABLE {sch}.crossing(crossing_id VARCHAR, source VARCHAR, crossing_type VARCHAR, markings VARCHAR,
           painted BOOLEAN, width DOUBLE, length DOUBLE, osm_id BIGINT, geom GEOMETRY);
         CREATE TABLE {sch}.lane_crossing(crossing_id VARCHAR, lane_id VARCHAR, link_id BIGINT, start_lr DOUBLE,
           end_lr DOUBLE, across_from DOUBLE, across_to DOUBLE, to_left BOOLEAN)"
    ))?;
    if !has_raw {
        return Ok(());
    }
    type LaneRow = (String, i64, f64, String, i64, Option<String>, Option<String>, Option<String>, bool);
    // the lanes and the lane connectors stay in DuckDB (`_xl`, numbered `k` in the order of the query): only their envelopes and
    // the lanes near a crossing come into memory (docs/design/gmns_streaming.md, step 3)
    c.exec(&format!(
        "CREATE OR REPLACE TEMP TABLE _xl AS SELECT row_number() OVER () - 1 AS k, * FROM (
           SELECT l.lane_id, l.link_id, COALESCE(l.width, {DEFAULT_LANE_W})::DOUBLE AS width, ST_AsText(l.geom) AS wkt, k.osm_id,
                  k.layer::VARCHAR AS layer, k.bridge::VARCHAR AS bridge, k.tunnel::VARCHAR AS tunnel, false AS conn
           FROM {sch}.lane l JOIN {sch}.link k ON k.link_id = l.link_id
           WHERE l.geom IS NOT NULL AND l.allowed_uses IN {ROAD_USES})"
    ))?;
    if c.count(&format!("SELECT count(*) FROM duckdb_tables() WHERE schema_name = '{sch}' AND table_name = 'lane_connector'"))? > 0 {
        c.exec(&format!(
            "INSERT INTO _xl SELECT (SELECT count(*) FROM _xl) + row_number() OVER () - 1, * FROM (
               SELECT c.connector_id, l.link_id, COALESCE(c.width, {DEFAULT_LANE_W})::DOUBLE, ST_AsText(c.geom), k.osm_id,
                      k.layer::VARCHAR, k.bridge::VARCHAR, k.tunnel::VARCHAR, true
               FROM {sch}.lane_connector c JOIN {sch}.lane l ON l.lane_id = c.from_lane_id
               JOIN {sch}.link k ON k.link_id = l.link_id
               WHERE c.geom IS NOT NULL AND l.allowed_uses IN {ROAD_USES})"
        ))?;
    }
    let n_lanes = c.count("SELECT count(*) FROM _xl")?;
    let lane_rows = |sql: &str| -> Result<Vec<(i64, LaneRow)>> {
        let mut st = c.con.prepare(sql).map_err(|e| format!("{e}\n{sql}"))?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?))))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        v
    };
    type WayRow = (i64, Option<String>, Option<String>, Option<String>, Vec<i64>, Option<String>, Option<String>, Option<String>);
    let sql = "SELECT osm_id, tags['crossing'], tags['crossing:markings'], tags['width'], refs, tags['layer'], tags['bridge'], tags['tunnel']
               FROM s.raw.ways WHERE tags['footway'] = 'crossing'";
    let mut st = c.con.prepare(sql).map_err(|e| format!("{e}\n{sql}"))?;
    let ways: Vec<WayRow> = st
        .query_map([], |r| {
            let refs: duckdb::types::Value = r.get(4)?;
            let refs = match refs {
                duckdb::types::Value::List(v) => v.into_iter().filter_map(|x| if let duckdb::types::Value::BigInt(i) = x { Some(i) } else { None }).collect(),
                _ => vec![],
            };
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, refs, r.get(5)?, r.get(6)?, r.get(7)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| e.to_string())?;
    drop(st);
    let sql = "SELECT osm_id, lon, lat, tags['crossing'], tags['crossing:markings'] FROM s.raw.nodes WHERE tags['highway'] = 'crossing'";
    let mut st = c.con.prepare(sql).map_err(|e| format!("{e}\n{sql}"))?;
    let nodes: Vec<(i64, f64, f64, Option<String>, Option<String>)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
    drop(st);
    let sql = format!("SELECT osm_id, refs FROM s.raw.ways WHERE osm_id IN (SELECT DISTINCT osm_id FROM {sch}.link)");
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let mut road_ways: HashMap<i64, HashSet<i64>> = HashMap::new();
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, duckdb::types::Value>(1)?))).map_err(|e| e.to_string())? {
        let (osm, refs) = r.map_err(|e| e.to_string())?;
        if let duckdb::types::Value::List(v) = refs {
            for x in v {
                if let duckdb::types::Value::BigInt(n) = x {
                    road_ways.entry(n).or_default().insert(osm);
                }
            }
        }
    }
    drop(st);
    // a crossing way with several zebra nodes on roads is a loop round a junction: no crossing line
    let zebra_nodes: HashSet<i64> = nodes.iter().map(|n| n.0).collect();
    let ways: Vec<WayRow> = ways.into_iter().filter(|w| w.4.iter().filter(|r| zebra_nodes.contains(r) && road_ways.contains_key(r)).count() < LOOP_NODES).collect();
    if n_lanes == 0 || (ways.is_empty() && nodes.is_empty()) {
        c.exec("DROP TABLE _xl")?;
        return Ok(());
    }
    let (x0, y0) = shp::wkt_points(&c.strings("SELECT wkt FROM _xl WHERE k = 0")?[0])[0];
    let kx = crate::gmns_lanes::kx_of(y0);
    const M: f64 = 111320.0;
    let to_m = |lon: f64, lat: f64| ((lon - x0) * kx * M, (lat - y0) * M);
    let to_ll = |x: f64, y: f64| (x / (kx * M) + x0, y / M + y0);
    let ref_ids: Vec<i64> = ways.iter().flat_map(|w| w.4.iter().copied()).collect::<HashSet<_>>().into_iter().collect();
    let mut node_xy: HashMap<i64, P> = HashMap::new();
    if !ref_ids.is_empty() {
        c.exec("CREATE OR REPLACE TEMP TABLE _refs(osm_id BIGINT)")?;
        let mut app = c.con.appender("_refs").map_err(|e| e.to_string())?;
        for r in &ref_ids {
            app.append_row([r]).map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
        drop(app);
        let sql = "SELECT n.osm_id, n.lon, n.lat FROM s.raw.nodes n JOIN _refs r USING (osm_id)";
        let mut st = c.con.prepare(sql).map_err(|e| e.to_string())?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?))).map_err(|e| e.to_string())? {
            let (o, lon, lat) = r.map_err(|e| e.to_string())?;
            node_xy.insert(o, to_m(lon, lat));
        }
        drop(st);
        c.exec("DROP TABLE _refs")?;
    }
    // every lane needs the envelope of its surface, because the order of the lane tree (as shapely's STRtree) depends on all
    // envelopes. Only the lanes near a crossing keep their surface: the surfaces of all lanes of the area took most of the
    // memory of this step. The surfaces are made on all cores (one core: Kentucky 105 s).
    const HOT_M: f64 = 50.0;
    let mut hot: HashSet<(i64, i64)> = HashSet::new();
    let mut heat = |e: (f64, f64, f64, f64)| hot.extend(cells((e.0 - HOT_M, e.1 - HOT_M, e.2 + HOT_M, e.3 + HOT_M)));
    for w in &ways {
        let pts: Vec<P> = w.4.iter().filter_map(|r| node_xy.get(r).copied()).collect();
        if pts.len() >= 2 {
            heat(shp_env(&pts));
        }
    }
    for n in &nodes {
        let p = to_m(n.1, n.2);
        heat((p.0, p.1, p.0, p.1));
    }
    use rayon::prelude::*;
    // 1. the envelope of every lane's surface, in chunks of lanes on all cores: the order of the lane tree (as shapely's
    // STRtree) depends on all of them. A lane is kept when its surface is near a crossing.
    const CHUNK: i64 = 500_000;
    let mut unbuffered = 0;
    let (mut env_all, mut near_k): (Vec<(f64, f64, f64, f64)>, Vec<(usize, i64)>) = (Vec::new(), Vec::new());
    let mut lo = 0;
    while lo < n_lanes {
        let rows = lane_rows(&format!("SELECT k, lane_id, link_id, width, wkt, osm_id, layer, bridge, tunnel, conn FROM _xl WHERE k >= {lo} AND k < {} ORDER BY k", lo + CHUNK))?;
        // each lane: None (too short), Some(None) (the buffer failed), or its envelope and whether it is near a crossing
        let made: Vec<Option<Option<((f64, f64, f64, f64), bool)>>> = rows
            .par_iter()
            .map(|(_, (_, _, w, wkt, ..))| {
                let g: Vec<P> = shp::wkt_points(wkt).into_iter().map(|(x, y)| to_m(x, y)).collect();
                if shp::length(&g) < 0.5 {
                    return None;
                }
                let Some(gp) = surface(&g, w / 2.0, &mut 0) else { return Some(None) };
                let (e, h) = (shp_env(&g), w / 2.0);
                Some(Some((env_of(&mp(&gp)), cells((e.0 - h, e.1 - h, e.2 + h, e.3 + h)).any(|q| hot.contains(&q)))))
            })
            .collect();
        for ((k, _), m) in rows.iter().zip(made) {
            match m {
                Some(Some((env, near))) => {
                    if near {
                        near_k.push((env_all.len(), *k));
                    }
                    env_all.push(env);
                }
                Some(None) => unbuffered += 1,
                None => {}
            }
        }
        lo += CHUNK;
    }
    // 2. the tree order of all lanes; the lanes near a crossing keep it among themselves
    let rank_all: Vec<usize> = {
        let mut r = vec![0; env_all.len()];
        for (q, &j) in shp::strtree_order(&env_all).iter().enumerate() {
            r[j] = q;
        }
        r
    };
    drop(env_all);
    // 3. the lanes near a crossing, in the order of `k`
    c.exec("CREATE OR REPLACE TEMP TABLE _xh(k BIGINT)")?;
    {
        let mut app = c.con.appender("_xh").map_err(|e| e.to_string())?;
        for (_, k) in &near_k {
            app.append_row([k]).map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
    }
    let rows = lane_rows("SELECT k, lane_id, link_id, width, wkt, osm_id, layer, bridge, tunnel, conn FROM _xl SEMI JOIN _xh USING (k) ORDER BY k")?;
    c.exec("DROP TABLE _xh; DROP TABLE _xl")?;
    let made: Vec<(Vec<P>, (f64, f64, f64, f64), MultiPolygon<f64>, Vec<crate::geos::Polygon>)> = rows
        .par_iter()
        .map(|(_, (_, _, w, wkt, ..))| {
            let g: Vec<P> = shp::wkt_points(wkt).into_iter().map(|(x, y)| to_m(x, y)).collect();
            let gp = surface(&g, w / 2.0, &mut 0).unwrap_or_default();
            let poly = mp(&gp);
            (g, env_of(&poly), poly, gp)
        })
        .collect();
    let mut l = Lanes { id: vec![], link: vec![], way: vec![], width: vec![], level: vec![], conn: vec![], geom: vec![], poly: vec![], gpoly: vec![], env: vec![], tree: vec![], rank: vec![], grid: HashMap::new() };
    for ((_, (lid, lk, w, _, osm, ly, br, tu, is_conn)), (g, env, poly, gp)) in rows.into_iter().zip(made) {
        l.gpoly.push(gp);
        l.env.push(env);
        l.id.push(lid);
        l.link.push(lk);
        l.way.push(osm);
        l.level.push(level(&ly, &br, &tu));
        l.conn.push(is_conn);
        l.width.push(w);
        l.geom.push(g);
        l.poly.push(poly);
    }
    if unbuffered > 0 {
        eprintln!("GMNS[crossings]: {unbuffered} lane(s) without a surface (the buffer failed also with less precision): no crossing on them");
    }
    // the lanes near a crossing in the order of the tree of all lanes
    let mut tree: Vec<usize> = (0..near_k.len()).collect();
    tree.sort_unstable_by_key(|&j| rank_all[near_k[j].0]);
    l.tree = tree;
    l.index();
    // the lanes of each link: a scan of all lanes for each crossing took hours on a large tile (Paris)
    let mut lanes_of_link: HashMap<i64, Vec<usize>> = HashMap::new();
    for (j, k) in l.link.iter().enumerate() {
        lanes_of_link.entry(*k).or_default().push(j);
    }
    // every crossing: (source, osm_id, type, markings, width, the line, the lanes it may use, level)
    struct Cand {
        src: &'static str,
        osm: i64,
        ct: Option<String>,
        mk: Option<String>,
        width: Option<f64>,
        line: Vec<P>,
        only: Option<HashSet<usize>>,
        level: Option<i64>,
    }
    let mut cands: Vec<Cand> = Vec::new();
    for (osm, ct, mk, wd, refs, ly, br, tu) in &ways {
        let pts: Vec<P> = refs.iter().filter_map(|r| node_xy.get(r).copied()).collect();
        if pts.len() >= 2 {
            cands.push(Cand { src: "w", osm: *osm, ct: ct.clone(), mk: mk.clone(), width: num(wd.as_deref()), line: pts, only: None, level: Some(level(ly, br, tu)) });
        }
    }
    let mut lanes_of_way: HashMap<i64, Vec<usize>> = HashMap::new();
    for (j, w) in l.way.iter().enumerate() {
        lanes_of_way.entry(*w).or_default().push(j);
    }
    let on_crossing_way: HashSet<i64> = ways.iter().flat_map(|w| w.4.iter().copied()).collect();
    for (osm, lon, lat, ct, mk) in &nodes {
        if on_crossing_way.contains(osm) || road_ways.get(osm).map(|s| s.len()) != Some(1) {
            continue;
        }
        let way = *road_ways[osm].iter().next().unwrap();
        let only: HashSet<usize> = lanes_of_way.get(&way).into_iter().flatten().copied().collect();
        let p = to_m(*lon, *lat);
        let mut near: Vec<usize> = only.iter().copied().filter(|j| shp::point_line_distance(p, &l.geom[*j]) < 12.0).collect();
        near.sort();
        if near.is_empty() {
            continue;
        }
        let lane = nearest(&l, &near, p);
        let (tx, ty) = l.tangent(lane, p);
        let line = vec![(p.0 - ty * NODE_REACH_M, p.1 + tx * NODE_REACH_M), (p.0 + ty * NODE_REACH_M, p.1 - tx * NODE_REACH_M)];
        cands.push(Cand { src: "n", osm: *osm, ct: ct.clone(), mk: mk.clone(), width: None, line, only: Some(only), level: None });
    }
    // (length, candidate, parts, near)
    let mut found: Vec<(f64, usize, Vec<Vec<P>>, Vec<usize>)> = Vec::new();
    for (ci, cd) in cands.iter().enumerate() {
        let (mut parts, near) = pieces(&l, &cd.line, None, cd.level);
        if cd.src == "n" && !parts.is_empty() {
            // the part through the node itself
            let m = shp::interpolate(&cd.line, 0.5 * shp::length(&cd.line));
            let mut best = 0;
            for k in 1..parts.len() {
                if shp::point_line_distance(m, &parts[k]) < shp::point_line_distance(m, &parts[best]) {
                    best = k;
                }
            }
            parts = vec![parts.swap_remove(best)];
        }
        if !parts.is_empty() {
            found.push((parts.iter().map(|q| shp::length(q)).sum(), ci, parts, near));
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0)); // the long crossings first (a stable sort, as Python's)
    let mut crossing_rows: Vec<(String, &str, Option<String>, Option<String>, bool, f64, f64, i64, String)> = Vec::new();
    let mut lane_rows: Vec<(String, String, i64, f64, f64, f64, f64, bool)> = Vec::new();
    // the crossings placed so far, with their boxes: one union of all of them took hours on a large tile (Paris)
    let mut placed: Vec<((f64, f64, f64, f64), MultiPolygon<f64>)> = Vec::new();
    let mut seen: HashMap<(&str, i64), usize> = HashMap::new();
    for (_, ci, parts, near) in &found {
        let cd = &cands[*ci];
        // 1. the lanes the crossing goes through, and where along each piece
        let mut cov: Vec<(usize, usize, f64, f64)> = Vec::new(); // (lane, piece index, s0, s1)
        for (pi, part) in parts.iter().enumerate() {
            for &j in near {
                let inter = clip(part, &l.gpoly[j]);
                let ilen: f64 = inter.iter().map(|g| shp::length(g)).sum();
                if inter.is_empty() || ilen < 0.3 {
                    continue;
                }
                let ss: Vec<f64> = inter.iter().flatten().map(|c| shp::project(part, *c)).collect();
                let (lo, hi) = (ss.iter().copied().fold(f64::INFINITY, f64::min), ss.iter().copied().fold(f64::NEG_INFINITY, f64::max));
                if hi - lo >= 0.3 {
                    cov.push((j, pi, lo, hi));
                }
            }
        }
        if cov.is_empty() {
            continue;
        }
        if cd.src == "n" && !cov.iter().any(|x| cd.only.as_ref().unwrap().contains(&x.0)) {
            continue;
        }
        // the lanes most nearly perpendicular to the crossing line, within 0.15 in sine of the best of each piece
        let sin_of = |&(j, pi, s0, s1): &(usize, usize, f64, f64)| {
            let part = &parts[pi];
            let (a, b) = (shp::interpolate(part, s0), shp::interpolate(part, s1));
            let n = match (b.0 - a.0).hypot(b.1 - a.1) {
                0.0 => 1.0,
                n => n,
            };
            let (tx, ty) = l.tangent(j, shp::interpolate(part, (s0 + s1) / 2.0));
            (((b.0 - a.0) * ty - (b.1 - a.1) * tx) / n).abs()
        };
        let sins: Vec<f64> = cov.iter().map(sin_of).collect();
        let mut best: HashMap<usize, f64> = HashMap::new();
        for (it, sn) in cov.iter().zip(&sins) {
            let e = best.entry(it.1).or_insert(0.0);
            *e = e.max(*sn);
        }
        let cov: Vec<(usize, usize, f64, f64)> = cov.iter().zip(&sins).filter(|(it, sn)| **sn >= best[&it.1] - 0.15).map(|(it, _)| *it).collect();
        // 2. the roads it crosses: the covered lanes grouped by direction
        let mut roads: Vec<(f64, Vec<(usize, usize, f64, f64, P)>)> = Vec::new();
        for &(j, pi, s0, s1) in &cov {
            let t = l.tangent(j, shp::interpolate(&parts[pi], (s0 + s1) / 2.0));
            let ang = crate::gmns_lanes::pymod(t.1.atan2(t.0).to_degrees(), 180.0);
            match roads.iter_mut().find(|r| (crate::gmns_lanes::pymod(ang - r.0 + 90.0, 180.0) - 90.0).abs() < ROAD_ANGLE) {
                Some(r) => r.1.push((j, pi, s0, s1, t)),
                None => roads.push((ang, vec![(j, pi, s0, s1, t)])),
            }
        }
        if cd.src == "n" {
            let only = cd.only.as_ref().unwrap();
            let mine: Vec<_> = roads.iter().filter(|r| r.1.iter().any(|x| only.contains(&x.0))).cloned().collect();
            if !mine.is_empty() {
                roads = mine;
            }
        }
        let mut fresh: Vec<MultiPolygon<f64>> = Vec::new();
        let links_of: Vec<HashSet<i64>> = roads.iter().map(|r| r.1.iter().filter(|x| !l.conn[x.0]).map(|x| l.link[x.0]).collect()).collect();
        for (k_road, (_, road)) in roads.iter().enumerate() {
            let others: HashSet<i64> = links_of.iter().enumerate().filter(|(k2, _)| *k2 != k_road).flat_map(|(_, s)| s.iter().copied()).filter(|x| !links_of[k_road].contains(x)).collect();
            let own: Vec<P> = road.iter().filter(|x| !l.conn[x.0]).map(|x| x.4).collect();
            let axes: Vec<P> = if own.is_empty() { road.iter().map(|x| x.4).collect() } else { own };
            let mut ends: Vec<P> = Vec::new();
            for (pi, part) in parts.iter().enumerate() {
                let ss: Vec<(f64, f64)> = road.iter().filter(|x| x.1 == pi).map(|x| (x.2, x.3)).collect();
                if ss.is_empty() {
                    continue;
                }
                ends.push(shp::interpolate(part, ss.iter().map(|x| x.0).fold(f64::INFINITY, f64::min)));
                ends.push(shp::interpolate(part, ss.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max)));
            }
            let (ux0, uy0) = axes[0];
            let sx: f64 = axes.iter().map(|(ax, ay)| if ax * ux0 + ay * uy0 >= 0.0 { *ax } else { -ax }).sum();
            let sy: f64 = axes.iter().map(|(ax, ay)| if ax * ux0 + ay * uy0 >= 0.0 { *ay } else { -ay }).sum();
            let nrm = match sx.hypot(sy) {
                0.0 => 1.0,
                n => n,
            };
            let (ux, uy) = (sx / nrm, sy / nrm);
            let (vx, vy) = (-uy, ux);
            let vs: Vec<f64> = ends.iter().map(|p| p.0 * vx + p.1 * vy).collect();
            let (mut v_lo, mut v_hi) = (vs.iter().copied().fold(f64::INFINITY, f64::min), vs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
            let u_c = ends.iter().map(|p| p.0 * ux + p.1 * uy).sum::<f64>() / ends.len() as f64;
            let mid = (ends.iter().map(|p| p.0).sum::<f64>() / ends.len() as f64, ends.iter().map(|p| p.1).sum::<f64>() / ends.len() as f64);
            let crossed: HashSet<i64> = road.iter().map(|x| l.link[x.0]).collect();
            let part_lines: Vec<LineString<f64>> = parts.iter().map(|q| ls(q)).collect();
            for &j2 in crossed.iter().filter_map(|k| lanes_of_link.get(k)).flatten() {
                if !l.conn[j2] && cd.level.is_none_or(|v| l.level[j2] == v) {
                    let dmin = part_lines.iter().map(|q| Euclidean.distance(&l.poly[j2], q)).fold(f64::INFINITY, f64::min);
                    if dmin < ROAD_REACH_M {
                        let pt = shp::interpolate(&l.geom[j2], shp::project(&l.geom[j2], mid));
                        let vc = pt.0 * vx + pt.1 * vy;
                        v_lo = v_lo.min(vc - l.width[j2] / 2.0);
                        v_hi = v_hi.max(vc + l.width[j2] / 2.0);
                    }
                }
            }
            if v_hi - v_lo < 1.0 {
                continue;
            }
            let width = cd.width.unwrap_or_else(|| (ZEBRA_W_PER_ROAD_W * (v_hi - v_lo)).max(ZEBRA_W_MIN).min(ZEBRA_W_MAX));
            let corner = |su: f64, vv: f64| (ux * (u_c + su * width / 2.0) + vx * vv, uy * (u_c + su * width / 2.0) + vy * vv);
            let corners = [corner(1.0, v_lo), corner(1.0, v_hi), corner(-1.0, v_hi), corner(-1.0, v_lo)];
            let mut ring: Vec<P> = corners.to_vec();
            ring.push(corners[0]);
            let rect = MultiPolygon(vec![Polygon::new(ls(&ring), vec![])]);
            let rect_area = rect.unsigned_area();
            let re = env_of(&rect);
            let mut near = placed.iter().filter(|(e, _)| meet(*e, re)).map(|(_, p)| p);
            if let Some(first) = near.next() {
                let pl = near.fold(first.clone(), |u, p| u.union(p));
                if rect.intersection(&pl).unsigned_area() > MAX_OVERLAP * rect_area {
                    continue;
                }
            }
            if fresh.iter().any(|f| rect.intersection(f).unsigned_area() > MAX_SAME_OVERLAP * rect_area.min(f.unsigned_area())) {
                continue;
            }
            // 3. the rectangle on each lane it overlaps
            let mut rows = Vec::new();
            for j in l.query(env_of(&rect)).into_iter().filter(|j| cd.level.is_none_or(|v| l.level[*j] == v)) {
                if others.contains(&l.link[j]) {
                    continue;
                }
                let ip = rect.intersection(&l.poly[j]);
                if ip.0.is_empty() || ip.unsigned_area() < 0.2 {
                    continue;
                }
                let pts: Vec<P> = ip.0.iter().flat_map(|p| p.exterior().0.iter()).map(|c| (c.x, c.y)).collect();
                if pts.is_empty() {
                    continue;
                }
                let lrs: Vec<f64> = pts.iter().map(|p| shp::project(&l.geom[j], *p)).collect();
                let (a, b) = (lrs.iter().copied().fold(f64::INFINITY, f64::min), lrs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
                let vv: Vec<f64> = pts.iter().map(|(x, y)| x * vx + y * vy).collect();
                let (ov_lo, ov_hi) = (vv.iter().copied().fold(f64::INFINITY, f64::min).max(v_lo), vv.iter().copied().fold(f64::NEG_INFINITY, f64::max).min(v_hi));
                if b - a < 0.3 || ov_hi - ov_lo < 0.1 {
                    continue;
                }
                let Some(cen) = ip.centroid() else { continue };
                let (tx, ty) = l.tangent(j, (cen.x(), cen.y()));
                if (tx * ux + ty * uy).abs() < (if l.conn[j] { MIN_ALONG_CONNECTOR } else { ROAD_ANGLE.to_radians().cos() }) {
                    continue;
                }
                let nv = -ty * vx + tx * vy;
                rows.push((l.id[j].clone(), l.link[j], a, b, ov_lo - v_lo, ov_hi - v_lo, nv > 0.0));
            }
            if rows.is_empty() {
                continue;
            }
            let n = seen.get(&(cd.src, cd.osm)).copied().unwrap_or(0);
            seen.insert((cd.src, cd.osm), n + 1);
            let cid = if n > 0 { format!("{}{}#{}", cd.src, cd.osm, n + 1) } else { format!("{}{}", cd.src, cd.osm) };
            fresh.push(rect);
            let wkt_ring: Vec<String> = ring.iter().map(|(x, y)| {
                let (lon, lat) = to_ll(*x, *y);
                format!("{lon:?} {lat:?}")
            }).collect();
            crossing_rows.push((
                cid.clone(),
                if cd.src == "w" { "way" } else { "node" },
                cd.ct.clone(),
                cd.mk.clone(),
                painted(cd.ct.as_deref(), cd.mk.as_deref()),
                width,
                v_hi - v_lo,
                cd.osm,
                format!("POLYGON (({}))", wkt_ring.join(", ")),
            ));
            for (lid, lk, a, b, f, t, left) in rows {
                lane_rows.push((cid.clone(), lid, lk, a, b, f, t, left));
            }
        }
        for rect in fresh {
            placed.push((env_of(&rect), rect));
        }
    }
    if !crossing_rows.is_empty() {
        c.exec("CREATE OR REPLACE TEMP TABLE _cr(crossing_id VARCHAR, source VARCHAR, crossing_type VARCHAR, markings VARCHAR, painted BOOLEAN, width DOUBLE, length DOUBLE, osm_id BIGINT, wkt VARCHAR)")?;
        let mut app = c.con.appender("_cr").map_err(|e| e.to_string())?;
        for r in &crossing_rows {
            app.append_row(duckdb::params![r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8]).map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
        drop(app);
        let mut app = c.con.appender_to_db("lane_crossing", sch).map_err(|e| e.to_string())?;
        for r in &lane_rows {
            app.append_row(duckdb::params![r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7]).map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
        drop(app);
        c.exec(&format!(
            "INSERT INTO {sch}.crossing SELECT crossing_id, source, crossing_type, markings, painted, width, length, osm_id, ST_GeomFromText(wkt) FROM _cr; DROP TABLE _cr"
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_query_is_the_scan() {
        let mut l = Lanes { id: vec![], link: vec![], way: vec![], width: vec![], level: vec![], conn: vec![], geom: vec![], poly: vec![], gpoly: vec![], env: vec![], tree: vec![], rank: vec![], grid: HashMap::new() };
        let mut s = 7u64;
        let mut r = || { s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (s >> 33) as f64 / (1u64 << 31) as f64 };
        for _ in 0..500 {
            let (x, y, w, h) = (r() * 2000.0 - 1000.0, r() * 2000.0 - 1000.0, r() * 300.0, r() * 30.0);
            l.env.push((x, y, x + w, y + h));
        }
        l.env.push((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY));
        l.tree = shp::strtree_order(&l.env);
        l.index();
        for _ in 0..200 {
            let (x, y) = (r() * 2200.0 - 1100.0, r() * 2200.0 - 1100.0);
            let e = (x, y, x + r() * 150.0, y + r() * 150.0);
            let scan: Vec<usize> = l.tree.iter().copied().filter(|&j| meet(l.env[j], e)).collect();
            assert_eq!(l.query(e), scan);
        }
    }
}
