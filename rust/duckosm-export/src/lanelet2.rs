//! The Lanelet2 export (OSM XML), the port of src/duckosm/lanelet2.py. docs/design/lanelet2_export.md.
//!
//! Each lane of the GMNS db (centerline and width) is one lanelet: two boundary ways (the centerline offset by half the
//! width to each side, `offset::offset_curve` in place of shapely) and a `type=lanelet` relation. The points of the
//! boundaries are merged on a grid of `snap_m`, so that lanelets share their nodes.

use crate::offset::{offset_curve, P};
use crate::{escape, has_table, open, write_text, Result};
use std::collections::HashMap;
use std::path::Path;

fn subtype(use_: Option<&str>) -> &'static str {
    match use_.unwrap_or("") {
        "bus" | "bus,bike" => "bus_lane",
        "bike" => "bicycle_lane",
        "walk" => "walkway",
        _ => "road",
    }
}

/// A float as Python's `f"{x:g}"`: 6 significant digits, no trailing zeros.
pub fn py_g(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    let e = format!("{x:.5e}");
    let (m, ex) = e.split_once('e').unwrap();
    let ex: i32 = ex.parse().unwrap();
    let trim = |s: &str| if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() };
    if (-4..6).contains(&ex) {
        trim(&format!("{x:.*}", (5 - ex) as usize))
    } else {
        format!("{}e{}{:02}", trim(m), if ex < 0 { '-' } else { '+' }, ex.abs())
    }
}

/// The points of a 'LINESTRING (x y, ...)' text.
fn coords(wkt: &str) -> Vec<P> {
    if !wkt.starts_with("LINESTRING") {
        return vec![];
    }
    let (Some(a), Some(b)) = (wkt.find('('), wkt.rfind(')')) else { return vec![] };
    wkt[a + 1..b]
        .split(',')
        .filter_map(|p| {
            let mut it = p.split_whitespace().map(|v| v.parse::<f64>().ok());
            Some((it.next()??, it.next()??))
        })
        .collect()
}

/// The lon/lat line offset sideways by `off_m` metres (left: positive), as `_offset_wkt` of src/duckosm/gmns.py: in a
/// local metre frame at the first point, the longest part. The line unchanged when there is no offset. An error where the
/// Rust offset curve does not follow GEOS (`offset::offset_curve`).
pub fn offset_lonlat(line: &[P], off_m: f64) -> Result<Vec<P>> {
    if off_m.abs() < 1e-6 || line.len() < 2 {
        return Ok(line.to_vec());
    }
    let (x0, y0) = line[0];
    let kx = match y0.to_radians().cos() {
        0.0 => 1.0,
        k => k,
    };
    let m = 111320.0;
    let local: Vec<P> = line.iter().map(|(x, y)| ((x - x0) * kx * m, (y - y0) * m)).collect();
    let parts = offset_curve(&local, off_m).map_err(|e| format!("the offset curve of {line:?} by {off_m} m: {e}"))?;
    let len = |p: &Vec<P>| p.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum::<f64>();
    // the first of the longest parts, as Python's max()
    let mut best: Option<&Vec<P>> = None;
    for p in &parts {
        if best.is_none_or(|b| len(p) > len(b)) {
            best = Some(p);
        }
    }
    Ok(match best {
        Some(b) if b.len() >= 2 => b.iter().map(|(x, y)| (x / (kx * m) + x0, y / m + y0)).collect(),
        _ => line.to_vec(),
    })
}

pub struct Summary {
    pub lanelets: usize,
    pub ways: usize,
    pub nodes: usize,
}

/// Write a Lanelet2 .osm from `gmns_<mode>.lane` (and `link` for the speed), as `to_lanelet2`.
pub fn to_lanelet2(gmns_db: &Path, out: &Path, mode: &str, snap_m: f64) -> Result<Summary> {
    let con = open(gmns_db)?;
    let g = format!("gmns_{mode}");
    if !has_table(&con, &g, "lane")? {
        return Err(format!("no '{g}.lane' in {} — build a GMNS db first (duckosm gmns)", gmns_db.display()));
    }
    let e = |e: duckdb::Error| e.to_string();
    let mut st = con
        .prepare(&format!(
            "SELECT la.link_id, la.allowed_uses, COALESCE(la.width, 3.25)::DOUBLE, ST_AsText(la.geom), lk.free_speed::DOUBLE \
             FROM {g}.lane la LEFT JOIN {g}.link lk ON lk.link_id = la.link_id WHERE la.geom IS NOT NULL"
        ))
        .map_err(e)?;
    type Row = (i64, Option<String>, f64, String, Option<f64>);
    let lanes: Vec<Row> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .map_err(e)?
        .collect::<std::result::Result<_, _>>()
        .map_err(e)?;
    drop(st);
    drop(con);

    let snap = snap_m / 111320.0;
    let mut counter = 0i64;
    let mut node_id: HashMap<(i64, i64), i64> = HashMap::new();
    let mut nodes: Vec<(i64, f64, f64)> = Vec::new();
    let mut ways: Vec<(i64, Vec<i64>)> = Vec::new();
    let mut rels: Vec<(i64, i64, i64, Vec<(&str, String)>)> = Vec::new();
    let mut way = |pts: &[P], counter: &mut i64, ways: &mut Vec<(i64, Vec<i64>)>| -> Option<i64> {
        let mut ns: Vec<i64> = Vec::new();
        for &(x, y) in pts {
            let key = ((x / snap).round_ties_even() as i64, (y / snap).round_ties_even() as i64);
            let nid = *node_id.entry(key).or_insert_with(|| {
                *counter += 1;
                nodes.push((*counter, x, y));
                *counter
            });
            if ns.last() != Some(&nid) {
                ns.push(nid); // snapped duplicates in a row: one node
            }
        }
        if ns.len() < 2 {
            return None;
        }
        *counter += 1;
        ways.push((*counter, ns));
        Some(*counter)
    };
    for (link_id, use_, width, wkt, free_speed) in lanes {
        let line = coords(&wkt);
        let (left, right) = (offset_lonlat(&line, width / 2.0)?, offset_lonlat(&line, -width / 2.0)?);
        if left.len() < 2 || right.len() < 2 {
            continue;
        }
        let Some(lw) = way(&left, &mut counter, &mut ways) else { continue };
        let Some(rw) = way(&right, &mut counter, &mut ways) else { continue };
        let mut tags = vec![
            ("type", "lanelet".to_string()),
            ("subtype", subtype(use_.as_deref()).to_string()),
            ("location", "urban".into()),
            ("one_way", "yes".into()),
            ("duckosm:edge_id", link_id.to_string()),
        ];
        if let Some(s) = free_speed.filter(|s| *s > 0.0) {
            tags.push(("speed_limit", py_g(s)));
        }
        counter += 1;
        rels.push((counter, lw, rw, tags));
    }

    let mut o = vec!["<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(), "<osm version=\"0.6\" generator=\"duckosm\">".into()];
    for (nid, lon, lat) in &nodes {
        o.push(format!("  <node id=\"{nid}\" visible=\"true\" version=\"1\" lat=\"{lat:.8}\" lon=\"{lon:.8}\"/>"));
    }
    for (wid, ns) in &ways {
        o.push(format!("  <way id=\"{wid}\" visible=\"true\" version=\"1\">"));
        o.extend(ns.iter().map(|n| format!("    <nd ref=\"{n}\"/>")));
        o.push("    <tag k=\"type\" v=\"line_thin\"/>".into());
        o.push("  </way>".into());
    }
    for (rid, lw, rw, tags) in &rels {
        o.push(format!("  <relation id=\"{rid}\" visible=\"true\" version=\"1\">"));
        o.push(format!("    <member type=\"way\" ref=\"{lw}\" role=\"left\"/>"));
        o.push(format!("    <member type=\"way\" ref=\"{rw}\" role=\"right\"/>"));
        o.extend(tags.iter().map(|(k, v)| format!("    <tag k=\"{}\" v=\"{}\"/>", escape(k), escape(v))));
        o.push("  </relation>".into());
    }
    o.push("</osm>\n".into());
    write_text(out, &o.join("\n"), false)?;
    Ok(Summary { lanelets: rels.len(), ways: ways.len(), nodes: nodes.len() })
}

#[cfg(test)]
mod tests {
    #[test]
    fn g_as_python() {
        for (x, s) in [(13.890000343322754, "13.89"), (50.0, "50"), (1234567.0, "1.23457e+06"), (0.00001234, "1.234e-05"), (8.333333, "8.33333")] {
            assert_eq!(super::py_g(x), s);
        }
    }
}
