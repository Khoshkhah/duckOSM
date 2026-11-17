//! The ASAM OpenDRIVE export (.xodr), the port of src/duckosm/opendrive.py. docs/design/opendrive_export.md.
//!
//! Phase 1 (the core db): one road for each directed edge, a reference line of straight pieces in a metric CRS and the
//! driving lanes on the right; no junctions. Phase 2 (`junctions`, a GMNS db): the links are roads, and at each junction
//! node each movement is a connecting road.

use crate::{data_utm_crs, has_table, open, quoteattr, write_text, Result};
use duckdb::Connection;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// The lane width (metres) when OSM has no width:lanes.
const LANE_W: &str = "3.25";

/// The proj4 text of the geoReference header, as `_geo_ref`. A CRS that is not EPSG:3006 or a WGS84 UTM zone: the text
/// of PROJ's `projinfo` (pyproj's `to_proj4` uses the same PROJ function), else `+init=<crs> +units=m +no_defs`.
fn geo_ref(crs: &str) -> String {
    if crs == "EPSG:3006" {
        return "+proj=tmerc +lat_0=0 +lon_0=15 +k=0.9996 +x_0=500000 +y_0=0 +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs".into();
    }
    let up = crs.to_uppercase();
    let code = up.strip_prefix("EPSG:").unwrap_or(&up);
    if let Ok(c) = code.parse::<i64>() {
        if (32601..=32760).contains(&c) && c % 100 <= 60 {
            let south = if c >= 32701 { " +south" } else { "" };
            return format!("+proj=utm +zone={}{south} +datum=WGS84 +units=m +no_defs", c % 100);
        }
    }
    // ponytail: projinfo can give a text that differs from pyproj's for an unusual CRS; the default (UTM) is exact
    if let Ok(o) = std::process::Command::new("projinfo").args(["-o", "PROJ", "-q", crs]).output() {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if o.status.success() && s.starts_with("+proj") {
            return s;
        }
    }
    format!("+init={} +units=m +no_defs", crs.to_lowercase())
}

fn coords(wkt: &str) -> Vec<(f64, f64)> {
    let (Some(a), Some(b)) = (wkt.find('('), wkt.rfind(')')) else { return vec![] };
    wkt[a + 1..b]
        .split(',')
        .filter_map(|p| {
            let mut it = p.split_whitespace().map(|v| v.parse::<f64>());
            match (it.next(), it.next()) {
                (Some(Ok(x)), Some(Ok(y))) => Some((x, y)),
                _ => None,
            }
        })
        .collect()
}

/// The straight pieces of a line: (s, x, y, heading, length), and the total length.
fn segments(pts: &[(f64, f64)]) -> (Vec<(f64, f64, f64, f64, f64)>, f64) {
    let mut segs = Vec::new();
    let mut s = 0.0;
    for w in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        let length = (x1 - x0).hypot(y1 - y0);
        if length <= 0.0 {
            continue;
        }
        segs.push((s, x0, y0, (y1 - y0).atan2(x1 - x0), length));
        s += length;
    }
    (segs, s)
}

fn planview(o: &mut Vec<String>, segs: &[(f64, f64, f64, f64, f64)]) {
    o.push("\t\t<planView>".into());
    for (s, x, y, hdg, length) in segs {
        o.push(format!(
            "\t\t\t<geometry s=\"{s:.4}\" x=\"{x:.4}\" y=\"{y:.4}\" hdg=\"{hdg:.6}\" length=\"{length:.4}\"><line/></geometry>"
        ));
    }
    o.push("\t\t</planView>".into());
}

fn lanes(o: &mut Vec<String>, n: i64) {
    o.push("\t\t<lanes>".into());
    o.push("\t\t\t<laneSection s=\"0\">".into());
    o.push("\t\t\t\t<center><lane id=\"0\" type=\"none\" level=\"false\"/></center>".into());
    o.push("\t\t\t\t<right>".into());
    for i in 1..=n {
        o.push(format!("\t\t\t\t\t<lane id=\"-{i}\" type=\"driving\" level=\"false\">"));
        o.push(format!("\t\t\t\t\t\t<width sOffset=\"0\" a=\"{LANE_W}\" b=\"0\" c=\"0\" d=\"0\"/>"));
        o.push("\t\t\t\t\t</lane>".into());
    }
    o.push("\t\t\t\t</right>".into());
    o.push("\t\t\t</laneSection>".into());
    o.push("\t\t</lanes>".into());
}

/// The file: the header (the extent of all points), then `body`.
fn document(name: &str, xs: &[f64], ys: &[f64], crs: &str, body: Vec<String>) -> Result<String> {
    if xs.is_empty() {
        return Err("no road with a geometry of two or more points".into());
    }
    let mx = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mn = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let mut o = vec![
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
        "<OpenDRIVE>".into(),
        format!(
            "\t<header revMajor=\"1\" revMinor=\"7\" name={} version=\"1.00\" north=\"{:.2}\" south=\"{:.2}\" east=\"{:.2}\" west=\"{:.2}\">",
            quoteattr(name),
            mx(ys),
            mn(ys),
            mx(xs),
            mn(xs)
        ),
        format!("\t\t<geoReference><![CDATA[{}]]></geoReference>", geo_ref(crs)),
        "\t</header>".into(),
    ];
    o.extend(body);
    o.push("</OpenDRIVE>\n".into());
    Ok(o.join("\n"))
}

fn tx(col: &str) -> String {
    format!("ST_AsText(ST_Transform({col}, 'EPSG:4326', ?, always_xy := true))")
}

fn e(e: duckdb::Error) -> String {
    e.to_string()
}

pub struct Summary {
    pub roads: usize,
    pub connecting_roads: usize,
    pub junctions: usize,
    pub crs: String,
}

fn phase1(con: &Connection, mode: &str, crs: &str, name: &str) -> Result<(String, Summary)> {
    let have_z: i64 = con
        .query_row(
            "SELECT count(*) FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'edges' AND column_name = 'z_from'",
            [mode],
            |r| r.get(0),
        )
        .map_err(e)?;
    let zsel = if have_z > 0 { ", z_from::DOUBLE, z_to::DOUBLE" } else { ", NULL::DOUBLE, NULL::DOUBLE" };
    let mut st = con
        .prepare(&format!(
            "SELECT edge_id, COALESCE(lanes, 1)::BIGINT, name, {}{zsel} FROM {mode}.edges WHERE geometry IS NOT NULL",
            tx("geometry")
        ))
        .map_err(e)?;
    type Row = (i64, i64, Option<String>, Option<String>, Option<f64>, Option<f64>);
    let rows: Vec<Row> = st
        .query_map([crs], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .map_err(e)?
        .collect::<std::result::Result<_, _>>()
        .map_err(e)?;
    let (mut xs, mut ys, mut o) = (Vec::new(), Vec::new(), Vec::new());
    let mut roads = 0;
    for (eid, n, nm, wkt, z0, z1) in rows {
        let pts = coords(wkt.as_deref().unwrap_or(""));
        if pts.len() < 2 {
            continue;
        }
        let (segs, total) = segments(&pts);
        if segs.is_empty() {
            continue;
        }
        xs.extend(pts.iter().map(|p| p.0));
        ys.extend(pts.iter().map(|p| p.1));
        o.push(format!("\t<road name={} length=\"{total:.4}\" id=\"{eid}\" junction=\"-1\">", quoteattr(nm.as_deref().unwrap_or(""))));
        planview(&mut o, &segs);
        if let (Some(z0), Some(z1)) = (z0, z1) {
            // a straight ramp from z_from to z_to: after planView, before lanes (the order of the schema)
            let b = if total > 0.0 { (z1 - z0) / total } else { 0.0 };
            o.push("\t\t<elevationProfile>".into());
            o.push(format!("\t\t\t<elevation s=\"0\" a=\"{z0:.4}\" b=\"{b:.6}\" c=\"0\" d=\"0\"/>"));
            o.push("\t\t</elevationProfile>".into());
        }
        lanes(&mut o, n.max(1));
        o.push("\t</road>".into());
        roads += 1;
    }
    let doc = document(name, &xs, &ys, crs, o)?;
    Ok((doc, Summary { roads, connecting_roads: 0, junctions: 0, crs: crs.into() }))
}

fn junctions(con: &Connection, g: &str, crs: &str, name: &str) -> Result<(String, Summary)> {
    let mut st = con
        .prepare(&format!(
            "SELECT link_id, from_node_id, to_node_id, COALESCE(lanes, 1)::BIGINT, name, {} FROM {g}.link WHERE geom IS NOT NULL",
            tx("geom")
        ))
        .map_err(e)?;
    type Link = (i64, i64, i64, i64, Option<String>, Option<String>);
    let links: Vec<Link> = st
        .query_map([crs], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .map_err(e)?
        .collect::<std::result::Result<_, _>>()
        .map_err(e)?;
    let mut st = con
        .prepare(&format!(
            "SELECT node_id, ib_link_id, ob_link_id, {} FROM {g}.movement WHERE geometry IS NOT NULL AND ib_link_id IS NOT NULL \
             AND ob_link_id IS NOT NULL",
            tx("ST_GeomFromText(geometry)")
        ))
        .map_err(e)?;
    let movements: Vec<(i64, i64, i64, Option<String>)> = st
        .query_map([crs], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(e)?
        .collect::<std::result::Result<_, _>>()
        .map_err(e)?;

    // a junction: a node where an inbound link has two or more outbound choices, or where three or more inbound links meet
    let mut ib_obs: HashMap<i64, HashMap<i64, BTreeSet<i64>>> = HashMap::new();
    for (node, ib, ob, _) in &movements {
        ib_obs.entry(*node).or_default().entry(*ib).or_default().insert(*ob);
    }
    let jnodes: BTreeSet<i64> =
        ib_obs.iter().filter(|(_, ibs)| ibs.len() >= 3 || ibs.values().any(|o| o.len() >= 2)).map(|(n, _)| *n).collect();
    // at a node that is not a junction: road to road (the first movement wins)
    let (mut succ, mut pred) = (HashMap::new(), HashMap::new());
    for (node, ib, ob, _) in &movements {
        if !jnodes.contains(node) {
            succ.entry(*ib).or_insert(*ob);
            pred.entry(*ob).or_insert(*ib);
        }
    }

    let (mut xs, mut ys, mut road_o, mut conn_o) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut roads = 0;
    for (lid, fnode, tnode, n, nm, wkt) in &links {
        let pts = coords(wkt.as_deref().unwrap_or(""));
        if pts.len() < 2 {
            continue;
        }
        let (segs, total) = segments(&pts);
        if segs.is_empty() {
            continue;
        }
        xs.extend(pts.iter().map(|p| p.0));
        ys.extend(pts.iter().map(|p| p.1));
        let mut link = Vec::new();
        if jnodes.contains(fnode) {
            link.push(format!("\t\t\t<predecessor elementType=\"junction\" elementId=\"{fnode}\"/>"));
        } else if let Some(p) = pred.get(lid) {
            link.push(format!("\t\t\t<predecessor elementType=\"road\" elementId=\"{p}\" contactPoint=\"end\"/>"));
        }
        if jnodes.contains(tnode) {
            link.push(format!("\t\t\t<successor elementType=\"junction\" elementId=\"{tnode}\"/>"));
        } else if let Some(s) = succ.get(lid) {
            link.push(format!("\t\t\t<successor elementType=\"road\" elementId=\"{s}\" contactPoint=\"start\"/>"));
        }
        road_o.push(format!("\t<road name={} length=\"{total:.4}\" id=\"{lid}\" junction=\"-1\">", quoteattr(nm.as_deref().unwrap_or(""))));
        if !link.is_empty() {
            road_o.push("\t\t<link>".into());
            road_o.extend(link);
            road_o.push("\t\t</link>".into());
        }
        planview(&mut road_o, &segs);
        lanes(&mut road_o, (*n).max(1));
        road_o.push("\t</road>".into());
        roads += 1;
    }
    // a connecting road for each movement at a junction; the junctions in the order of their first connecting road
    let mut ci = 0;
    let mut order: Vec<i64> = Vec::new();
    let mut conns: HashMap<i64, Vec<(String, i64)>> = HashMap::new();
    for (node, ib, ob, wkt) in &movements {
        let Some(wkt) = wkt.as_deref().filter(|w| !w.is_empty() && jnodes.contains(node)) else { continue };
        let pts = coords(wkt);
        if pts.len() < 2 {
            continue;
        }
        let (segs, total) = segments(&pts);
        if segs.is_empty() {
            continue;
        }
        let cid = format!("c{ci}");
        ci += 1;
        xs.extend(pts.iter().map(|p| p.0));
        ys.extend(pts.iter().map(|p| p.1));
        conn_o.push(format!("\t<road name=\"\" length=\"{total:.4}\" id=\"{cid}\" junction=\"{node}\">"));
        conn_o.push("\t\t<link>".into());
        conn_o.push(format!("\t\t\t<predecessor elementType=\"road\" elementId=\"{ib}\" contactPoint=\"end\"/>"));
        conn_o.push(format!("\t\t\t<successor elementType=\"road\" elementId=\"{ob}\" contactPoint=\"start\"/>"));
        conn_o.push("\t\t</link>".into());
        planview(&mut conn_o, &segs);
        lanes(&mut conn_o, 1);
        conn_o.push("\t</road>".into());
        conns.entry(*node).or_insert_with(|| {
            order.push(*node);
            Vec::new()
        });
        conns.get_mut(node).unwrap().push((cid, *ib));
    }
    let mut junc_o = Vec::new();
    for node in &order {
        junc_o.push(format!("\t<junction id=\"{node}\" name=\"\">"));
        for (k, (cid, ib)) in conns[node].iter().enumerate() {
            junc_o.push(format!("\t\t<connection id=\"{k}\" incomingRoad=\"{ib}\" connectingRoad=\"{cid}\" contactPoint=\"start\">"));
            junc_o.push("\t\t\t<laneLink from=\"-1\" to=\"-1\"/>".into());
            junc_o.push("\t\t</connection>".into());
        }
        junc_o.push("\t</junction>".into());
    }
    road_o.extend(conn_o);
    road_o.extend(junc_o);
    let doc = document(name, &xs, &ys, crs, road_o)?;
    Ok((doc, Summary { roads, connecting_roads: ci, junctions: order.len(), crs: crs.into() }))
}

/// Write an OpenDRIVE file from the database `db`, as `to_opendrive`. `crs`: None = the UTM zone of the data.
/// `with_junctions`: `db` is a GMNS db (`gmns_<mode>.link` and `.movement`).
pub fn to_opendrive(db: &Path, out: &Path, mode: &str, crs: Option<&str>, with_junctions: bool) -> Result<Summary> {
    let con = open(db)?;
    let name = db.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let (xml, s) = if with_junctions {
        let g = format!("gmns_{mode}");
        if !has_table(&con, &g, "movement")? {
            return Err(format!("--junctions needs a GMNS db (no '{g}.movement' in {}) — build with duckosm gmns", db.display()));
        }
        let crs = match crs {
            Some(c) => c.to_string(),
            None => data_utm_crs(&con, &format!("{g}.link"), "geom")?,
        };
        junctions(&con, &g, &crs, name)?
    } else {
        if !has_table(&con, mode, "edges")? {
            return Err(format!("no '{mode}.edges' in {} — is this a built duckOSM db with mode '{mode}'?", db.display()));
        }
        let crs = match crs {
            Some(c) => c.to_string(),
            None => data_utm_crs(&con, &format!("{mode}.edges"), "geometry")?,
        };
        phase1(&con, mode, &crs, name)?
    };
    write_text(out, &xml, false)?;
    Ok(s)
}
