//! `duckosm.hdmap.to_commonroad` and `duckosm.hdmap.surfaces`: the lanelets of a CommonRoad map and the road surfaces, from the
//! lanes of a GMNS database. Python only makes the commonroad-io objects and the GeoDataFrames.

use crate::offset::{offset_curve, P};
use crate::Result;
use std::collections::HashMap;
use std::path::Path;

/// One lanelet: its id, its left, center and right borders (metres east and north of the origin, the same number of points),
/// its predecessors, successors, the lanelets on its left and right (same direction), its kind (`bus`, `bike` or `urban`),
/// the GMNS id (a `lane_id` or a `connector_id`) and the `edge_id`.
pub struct Lanelet {
    pub id: i64,
    pub left: Vec<P>,
    pub center: Vec<P>,
    pub right: Vec<P>,
    pub pred: Vec<i64>,
    pub succ: Vec<i64>,
    pub adj_left: Option<i64>,
    pub adj_right: Option<i64>,
    pub kind: String,
    pub gmns_id: String,
    pub edge_id: i64,
}

/// The lanelets, the origin `(lon, lat)` (the middle of the box of the lanes), and the numbers of lanes and connectors.
pub struct Map {
    pub lanelets: Vec<Lanelet>,
    pub origin: P,
    pub lanes: usize,
    pub connectors: usize,
}

struct Row {
    key: String,
    edge_id: i64,
    lane_num: Option<i64>,
    width: f64,
    uses: Option<String>,
    from_to: Option<(String, String)>,
    line: Vec<P>,
}

/// The `n` points at equal steps along `l`, the first and the last at the ends (shapely `line_interpolate_point` of
/// `linspace(0, 1, n)`, normalized).
fn resample(l: &[P], n: usize) -> Vec<P> {
    let len = crate::shapely::length(l);
    (0..n).map(|i| crate::shapely::interpolate(l, len * i as f64 / (n - 1) as f64)).collect()
}

/// `duckosm.hdmap.to_commonroad`: one lanelet for each lane (its line to the edge of the junction, `geom_cut`, and its width;
/// 3.25 m without a width) and one for each lane connector. A lane or connector whose left or right border is not one line
/// (too short, or too bent for its width) has no lanelet.
pub fn lanelets(gmns_db: &Path, mode: &str) -> Result<Map> {
    let con = crate::open(gmns_db)?;
    let g = format!("gmns_{mode}");
    let e = |x: duckdb::Error| x.to_string();
    let mut rows: Vec<Row> = Vec::new();
    let mut st = con
        .prepare(&format!(
            "SELECT lane_id::VARCHAR, link_id, lane_num, coalesce(width, 3.25)::DOUBLE, allowed_uses, ST_AsText(coalesce(geom_cut, geom))
             FROM {g}.lane WHERE coalesce(geom_cut, geom) IS NOT NULL ORDER BY lane_id"
        ))
        .map_err(e)?;
    let mut q = st.query([]).map_err(e)?;
    while let Some(r) = q.next().map_err(e)? {
        rows.push(Row { key: r.get(0).map_err(e)?, edge_id: r.get(1).map_err(e)?, lane_num: r.get(2).map_err(e)?, width: r.get(3).map_err(e)?,
                        uses: r.get(4).map_err(e)?, from_to: None, line: crate::shapely::wkt_points(&r.get::<_, String>(5).map_err(e)?) });
    }
    let lanes = rows.len();
    let has: i64 = con
        .query_row(&format!("SELECT count(*) FROM duckdb_tables() WHERE schema_name = '{g}' AND table_name = 'lane_connector'"), [], |r| r.get(0))
        .map_err(e)?;
    if has > 0 {
        let mut st = con
            .prepare(&format!(
                "SELECT connector_id::VARCHAR, from_lane_id::VARCHAR, to_lane_id::VARCHAR, coalesce(width, 3.25)::DOUBLE, ST_AsText(geom)
                 FROM {g}.lane_connector WHERE geom IS NOT NULL ORDER BY connector_id"
            ))
            .map_err(e)?;
        let mut q = st.query([]).map_err(e)?;
        while let Some(r) = q.next().map_err(e)? {
            let (from, to): (String, String) = (r.get(1).map_err(e)?, r.get(2).map_err(e)?);
            // a lane id is "<link_id>_<lane_num>": the connector goes back to the link of its first lane
            let edge_id = from.split('_').next().and_then(|x| x.parse().ok()).unwrap_or(0);
            rows.push(Row { key: r.get(0).map_err(e)?, edge_id, lane_num: None, width: r.get(3).map_err(e)?, uses: None,
                            from_to: Some((from, to)), line: crate::shapely::wkt_points(&r.get::<_, String>(4).map_err(e)?) });
        }
    }
    let connectors = rows.len() - lanes;
    let pts = rows.iter().flat_map(|r| r.line.iter());
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in pts {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let (lon0, lat0) = if rows.is_empty() { (0.0, 0.0) } else { ((x0 + x1) / 2.0, (y0 + y1) / 2.0) };
    let (kx, ky) = (111_320.0 * lat0.to_radians().cos(), 110_574.0);
    // the borders of each row, when both are one line
    let mut sides: HashMap<i64, (Vec<P>, Vec<P>)> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        let local: Vec<P> = r.line.iter().map(|&(x, y)| ((x - lon0) * kx, (y - lat0) * ky)).collect();
        let (Ok(l), Ok(rt)) = (offset_curve(&local, r.width / 2.0), offset_curve(&local, -r.width / 2.0)) else { continue };
        if let ([l], [rt]) = (l.as_slice(), rt.as_slice()) {
            if l.len() >= 2 && rt.len() >= 2 {
                let n = l.len().max(rt.len()).max(2);
                sides.insert(i as i64 + 1, (resample(l, n), resample(rt, n)));
            }
        }
    }
    let lane_id: HashMap<&str, i64> = rows[..lanes].iter().enumerate().map(|(i, r)| (r.key.as_str(), i as i64 + 1)).collect();
    let (mut succ, mut pred): (HashMap<i64, Vec<i64>>, HashMap<i64, Vec<i64>>) = Default::default();
    for (i, r) in rows.iter().enumerate().skip(lanes) {
        let id = i as i64 + 1;
        let (from, to) = r.from_to.as_ref().expect("a connector");
        if let (Some(&a), Some(&b)) = (lane_id.get(from.as_str()), lane_id.get(to.as_str())) {
            succ.entry(a).or_default().push(id);
            pred.entry(id).or_default().push(a);
            succ.entry(id).or_default().push(b);
            pred.entry(b).or_default().push(id);
        }
    }
    // lane 1 is the leftmost: lane n + 1 is on its right, in the same direction
    let num: HashMap<(i64, i64), i64> =
        rows[..lanes].iter().enumerate().filter_map(|(i, r)| r.lane_num.map(|n| ((r.edge_id, n), i as i64 + 1))).collect();
    let (mut left_of, mut right_of): (HashMap<i64, i64>, HashMap<i64, i64>) = Default::default();
    for (&(e, n), &i) in &num {
        if let Some(&j) = num.get(&(e, n + 1)) {
            right_of.insert(i, j);
            left_of.insert(j, i);
        }
    }
    let keep = |v: Option<&Vec<i64>>| v.into_iter().flatten().copied().filter(|x| sides.contains_key(x)).collect::<Vec<_>>();
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let id = i as i64 + 1;
        let Some((l, rt)) = sides.get(&id) else { continue };
        let kind = match r.uses.as_deref().and_then(|u| u.split(',').next()) {
            Some("bus") => "bus",
            Some("bike") => "bike",
            _ => "urban",
        };
        out.push(Lanelet {
            id,
            center: l.iter().zip(rt).map(|(a, b)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)).collect(),
            left: l.clone(),
            right: rt.clone(),
            pred: keep(pred.get(&id)),
            succ: keep(succ.get(&id)),
            adj_left: left_of.get(&id).copied().filter(|x| sides.contains_key(x)),
            adj_right: right_of.get(&id).copied().filter(|x| sides.contains_key(x)),
            kind: kind.into(),
            gmns_id: r.key.clone(),
            edge_id: r.edge_id,
        });
    }
    Ok(Map { lanelets: out, origin: (lon0, lat0), lanes, connectors })
}

/// A lane polygon of `surfaces`: `(lane_id, edge_id, lane_num, width, allowed_uses, WKB)`.
pub type LaneSurface = (String, i64, Option<i64>, f64, Option<String>, Vec<u8>);

/// `duckosm.hdmap.surfaces`: one polygon for each lane with a cut line (its width, flat ends), and one for each junction node
/// (the union of its lane connectors with their widths): `(node_id, connectors, WKB)`. DuckDB computes them in the UTM zone of
/// the area (the middle of the nodes).
pub fn surfaces(gmns_db: &Path, mode: &str) -> Result<(Vec<LaneSurface>, Vec<(i64, i64, Vec<u8>)>)> {
    let con = crate::open(gmns_db)?;
    let g = format!("gmns_{mode}");
    let e = |x: duckdb::Error| x.to_string();
    let (lon, lat): (f64, f64) = con.query_row(&format!("SELECT avg(x_coord), avg(y_coord) FROM {g}.node"), [], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?;
    let utm = crate::utm_epsg(lon, lat);
    let surface = |line: &str, width: &str| {
        format!(
            "ST_Transform(ST_Buffer(ST_Transform({line}, 'EPSG:4326', '{utm}', always_xy := true), {width} / 2, 8, 'CAP_FLAT', 'JOIN_ROUND', 5.0), \
             '{utm}', 'EPSG:4326', always_xy := true)"
        )
    };
    let mut st = con
        .prepare(&format!(
            "SELECT lane_id::VARCHAR, link_id, lane_num, coalesce(width, 3.25)::DOUBLE, allowed_uses, ST_AsWKB({})::BLOB
             FROM {g}.lane WHERE geom_cut IS NOT NULL ORDER BY lane_id",
            surface("geom_cut", "coalesce(width, 3.25)")
        ))
        .map_err(e)?;
    let lanes = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .map_err(e)?
        .collect::<std::result::Result<Vec<LaneSurface>, _>>()
        .map_err(e)?;
    let n: i64 = con
        .query_row(
            &format!("SELECT count(*) FROM duckdb_tables() WHERE schema_name = '{g}' AND table_name IN ('lane_connector', 'movement')"),
            [],
            |r| r.get(0),
        )
        .map_err(e)?;
    let mut junctions = Vec::new();
    if n == 2 {
        let mut st = con
            .prepare(&format!(
                "WITH c AS (SELECT m.node_id, {} AS g FROM {g}.lane_connector c JOIN {g}.movement m USING (mvmt_id) WHERE c.geom IS NOT NULL)
                 SELECT node_id, count(*), ST_AsWKB(ST_Union_Agg(g))::BLOB FROM c GROUP BY node_id ORDER BY node_id",
                surface("c.geom", "coalesce(c.width, 3.25)")
            ))
            .map_err(e)?;
        junctions = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(e)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(e)?;
    }
    Ok((lanes, junctions))
}
