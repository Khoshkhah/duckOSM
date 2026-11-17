//! The MATSim network export (`network_v2`), the port of src/duckosm/matsim.py. docs/design/matsim_export.md.
//!
//! Each edge is one directed link (`edge_id` is the link id). The node coordinates are in a projected CRS (default: the
//! UTM zone of the data). More than one mode: one link for each `edge_id`, with the modes of all its rows.

use crate::{data_utm_crs, has_table, open, quoteattr, write_text, Result};
use duckdb::Connection;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

const MODES: [&str; 3] = ["driving", "cycling", "walking"];
const DTD: &str = "http://www.matsim.org/files/dtd/network_v2.dtd";
/// About 30 km/h: the free speed (m/s) when an edge has no speed and no cost.
const FALLBACK_MS: f64 = 8.33;

/// The capacity (vehicles/hour/lane) of a road class, as `_CAPACITY` of src/duckosm/gmns.py. Other classes: 800.
pub fn capacity(hw: &str) -> f64 {
    match hw {
        "motorway" => 2000.0,
        "trunk" => 1800.0,
        "primary" => 1600.0,
        "secondary" => 1400.0,
        "tertiary" => 1200.0,
        "unclassified" => 1000.0,
        "living_street" | "service" => 300.0,
        _ => 800.0,
    }
}

/// The road class without `_link` and without a second value (`primary_link;x` -> `primary`).
fn norm_hw(hw: Option<&str>) -> &str {
    let h = hw.unwrap_or("").split(';').next().unwrap_or("");
    h.strip_suffix("_link").unwrap_or(h)
}

fn matsim_mode(m: &str) -> &str {
    match m {
        "driving" => "car",
        "cycling" => "bike",
        "walking" => "walk",
        o => o,
    }
}

/// The MATSim modes of the duckOSM modes, in the order car, bike, walk.
fn modes_str(modes: &BTreeSet<&str>) -> String {
    let order = |m: &str| ["car", "bike", "walk"].iter().position(|x| *x == m).unwrap_or(99);
    let mut v: Vec<&str> = modes.iter().map(|m| matsim_mode(m)).collect();
    v.sort_by_key(|m| order(m));
    v.dedup();
    v.join(",")
}

/// `mode`: one mode, `all`, or a comma list. The modes that are asked for, in the order driving, cycling, walking.
fn resolve_modes(con: &Connection, mode: &str) -> Result<Vec<&'static str>> {
    let mut present = Vec::new();
    for m in MODES {
        if has_table(con, m, "edges")? {
            present.push(m);
        }
    }
    let req: Vec<String> = if mode == "all" {
        present.iter().map(|s| s.to_string()).collect()
    } else {
        mode.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    let missing: Vec<&String> = req.iter().filter(|m| !present.contains(&m.as_str())).collect();
    if !missing.is_empty() {
        return Err(format!("mode(s) {missing:?} not present in {present:?} — build them first"));
    }
    Ok(MODES.into_iter().filter(|m| req.iter().any(|r| r == m)).collect())
}

struct Link {
    id: i64,
    src: i64,
    tgt: i64,
    length: f64,
    freespeed: f64,
    capacity: f64,
    permlanes: i64,
    modes: String,
}

struct Row {
    mode: String,
    id: i64,
    src: i64,
    tgt: i64,
    hw: Option<String>,
    lanes: Option<i64>,
    spd: Option<f64>,
    length: Option<f64>,
    cost: Option<f64>,
}

fn pos(v: Option<f64>) -> Option<f64> {
    v.filter(|x| *x > 0.0)
}

fn rows(con: &Connection, modes: &[&str]) -> Result<Vec<Row>> {
    let sql = modes
        .iter()
        .map(|m| {
            format!(
                "SELECT '{m}' AS mode, edge_id, source, target, highway, lanes::BIGINT, maxspeed_kmh::DOUBLE, length_m::DOUBLE, \
                 cost_s::DOUBLE FROM {m}.edges WHERE source IS NOT NULL AND target IS NOT NULL"
            )
        })
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    let mut st = con.prepare(&format!("SELECT * FROM ({sql})")).map_err(|e| e.to_string())?;
    let it = st
        .query_map([], |r| {
            Ok(Row {
                mode: r.get(0)?,
                id: r.get(1)?,
                src: r.get(2)?,
                tgt: r.get(3)?,
                hw: r.get(4)?,
                lanes: r.get(5)?,
                spd: r.get(6)?,
                length: r.get(7)?,
                cost: r.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
    it.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())
}

fn single_links(rows: Vec<Row>) -> Vec<Link> {
    rows.into_iter()
        .map(|r| {
            let permlanes = r.lanes.unwrap_or(1).max(1);
            let length = pos(r.length).unwrap_or(1.0);
            let freespeed = match (pos(r.spd), pos(r.cost)) {
                (Some(s), _) => s / 3.6,
                (None, Some(c)) => length / c,
                _ => FALLBACK_MS,
            };
            Link {
                id: r.id,
                src: r.src,
                tgt: r.tgt,
                length,
                freespeed,
                capacity: capacity(norm_hw(r.hw.as_deref())) * permlanes as f64,
                permlanes,
                modes: modes_str(&BTreeSet::from([r.mode.as_str()])),
            }
        })
        .collect()
}

/// One link for each `edge_id` (in the order of its first row). The car values come from the driving row.
fn multi_links(rows: Vec<Row>) -> Vec<Link> {
    let mut order: Vec<i64> = Vec::new();
    let mut groups: HashMap<i64, Vec<Row>> = HashMap::new();
    for r in rows {
        groups.entry(r.id).or_insert_with(|| {
            order.push(r.id);
            Vec::new()
        });
        groups.get_mut(&r.id).unwrap().push(r);
    }
    order
        .into_iter()
        .map(|id| {
            let rs = &groups[&id];
            let modes: BTreeSet<&str> = rs.iter().map(|r| r.mode.as_str()).collect();
            let length = rs.iter().find_map(|r| pos(r.length)).unwrap_or(1.0);
            let drow = rs.iter().find(|r| r.mode == "driving");
            let hw = match drow {
                Some(d) => d.hw.as_deref(),
                None => rs.iter().find_map(|r| r.hw.as_deref().filter(|h| !h.is_empty())),
            };
            let permlanes = drow.and_then(|d| d.lanes).filter(|l| *l != 0).unwrap_or(1).max(1);
            let freespeed = match drow.and_then(|d| pos(d.spd)) {
                Some(s) => s / 3.6,
                None => match rs.iter().filter_map(|r| pos(r.cost)).reduce(f64::min) {
                    Some(c) => length / c,
                    None => FALLBACK_MS,
                },
            };
            Link {
                id,
                src: rs[0].src,
                tgt: rs[0].tgt,
                length,
                freespeed,
                capacity: capacity(norm_hw(hw)) * permlanes as f64,
                permlanes,
                modes: modes_str(&modes),
            }
        })
        .collect()
}

pub struct Summary {
    pub nodes: usize,
    pub links: usize,
    pub crs: String,
}

/// Write a MATSim `network.xml` (gzip when `gz`) from the database `db`, as `to_matsim` of src/duckosm/matsim.py.
pub fn to_matsim(db: &Path, out: &Path, mode: &str, crs: Option<&str>, gz: bool) -> Result<Summary> {
    let con = open(db)?;
    let modes = resolve_modes(&con, mode)?;
    let crs = match crs {
        Some(c) => c.to_string(),
        None => data_utm_crs(&con, &format!("{}.nodes", modes[0]), "geom")?,
    };
    let union = modes.iter().map(|m| format!("SELECT node_id, geom FROM {m}.nodes")).collect::<Vec<_>>().join(" UNION ");
    let mut xy: HashMap<i64, (f64, f64)> = HashMap::new();
    {
        let mut st = con
            .prepare(&format!(
                "SELECT node_id, ST_X(t), ST_Y(t) FROM (SELECT node_id, ST_Transform(geom, 'EPSG:4326', ?, always_xy := true) t FROM ({union}))"
            ))
            .map_err(|e| e.to_string())?;
        let mut q = st.query([&crs]).map_err(|e| e.to_string())?;
        while let Some(r) = q.next().map_err(|e| e.to_string())? {
            let (x, y): (Option<f64>, Option<f64>) = (r.get(1).map_err(|e| e.to_string())?, r.get(2).map_err(|e| e.to_string())?);
            if let (Some(x), Some(y)) = (x, y) {
                xy.insert(r.get(0).map_err(|e| e.to_string())?, (x, y));
            }
        }
    }
    // nodes.ele: only when `duckosm elevation` added it
    let mut ele: HashMap<i64, f64> = HashMap::new();
    let have_ele: i64 = con
        .query_row(
            "SELECT count(*) FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'nodes' AND column_name = 'ele'",
            [modes[0]],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if have_ele > 0 {
        let sql = modes
            .iter()
            .map(|m| format!("SELECT node_id, ele::DOUBLE FROM {m}.nodes WHERE ele IS NOT NULL"))
            .collect::<Vec<_>>()
            .join(" UNION ");
        let mut st = con.prepare(&sql).map_err(|e| e.to_string())?;
        let it = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
        for p in it {
            let (k, v) = p.map_err(|e| e.to_string())?;
            ele.insert(k, v);
        }
    }
    let rows = rows(&con, &modes)?;
    drop(con);
    let raw = if modes.len() == 1 { single_links(rows) } else { multi_links(rows) };
    let links: Vec<Link> = raw.into_iter().filter(|l| xy.contains_key(&l.src) && xy.contains_key(&l.tgt)).collect();
    let used: BTreeSet<i64> = links.iter().flat_map(|l| [l.src, l.tgt]).collect();

    let mut o = String::with_capacity(200 * (links.len() + used.len()));
    o.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    o.push_str(&format!("<!DOCTYPE network SYSTEM \"{DTD}\">\n<network>\n\t<attributes>\n"));
    o.push_str(&format!(
        "\t\t<attribute name=\"coordinateReferenceSystem\" class=\"java.lang.String\">{crs}</attribute>\n\t</attributes>\n\t<nodes>\n"
    ));
    for n in &used {
        let (x, y) = xy[n];
        let z = ele.get(n).map(|z| format!(" z=\"{z:.2}\"")).unwrap_or_default();
        o.push_str(&format!("\t\t<node id=\"{n}\" x=\"{x:.2}\" y=\"{y:.2}\"{z}/>\n"));
    }
    o.push_str("\t</nodes>\n\t<links capperiod=\"01:00:00\" effectivecellsize=\"7.5\" effectivelanewidth=\"3.75\">\n");
    for l in &links {
        o.push_str(&format!(
            "\t\t<link id=\"{}\" from=\"{}\" to=\"{}\" length=\"{:.2}\" freespeed=\"{:.4}\" capacity=\"{:.1}\" permlanes=\"{}\" modes={}/>\n",
            l.id,
            l.src,
            l.tgt,
            l.length,
            l.freespeed,
            l.capacity,
            l.permlanes,
            quoteattr(&l.modes)
        ));
    }
    o.push_str("\t</links>\n</network>\n");
    write_text(out, &o, gz)?;
    Ok(Summary { nodes: used.len(), links: links.len(), crs })
}
