//! The MATSim network export (`network_v2`), the port of src/duckosm/matsim.py. docs/design/matsim_export.md.
//!
//! Each edge is one directed link (`edge_id` is the link id). The node coordinates are in a projected CRS (default: the
//! UTM zone of the data). More than one mode: one link for each `edge_id`, with the modes of all its rows.
//!
//! The input is a duckOSM db (`<mode>.edges`) or a GMNS db (`gmns_<mode>.link`, `link_id` = `edge_id`):
//! docs/design/sim_from_gmns.md. From GMNS, the bus-only links are in the network too (mode `bus`).

use crate::stream::each_chunk;
use crate::{data_utm_crs, has_table, open, quoteattr, Out, Result};
use duckdb::Connection;
use std::collections::BTreeSet;
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

/// The MATSim modes of the duckOSM modes, in the order car, bike, walk. A mode can be a comma list (the `allowed_uses` of a
/// GMNS bus-only link: `bus,bike`).
fn modes_str(modes: &BTreeSet<&str>) -> String {
    let order = |m: &str| ["car", "bike", "walk"].iter().position(|x| *x == m).unwrap_or(99);
    let mut v: Vec<&str> = modes.iter().flat_map(|m| m.split(',')).map(|m| matsim_mode(m.trim())).collect();
    v.sort_by_key(|m| order(m));
    v.dedup();
    v.join(",")
}

/// Is `con` a GMNS db: `gmns_<mode>.link` and no `<mode>.edges`?
pub fn is_gmns(con: &Connection) -> Result<bool> {
    let (mut edges, mut links) = (false, false);
    for m in MODES {
        edges |= has_table(con, m, "edges")?;
        links |= has_table(con, &format!("gmns_{m}"), "link")?;
    }
    Ok(links && !edges)
}

/// `mode`: one mode, `all`, or a comma list. The modes that are asked for, in the order driving, cycling, walking.
fn resolve_modes(con: &Connection, mode: &str, gmns: bool) -> Result<Vec<&'static str>> {
    let mut present = Vec::new();
    for m in MODES {
        let here = if gmns { has_table(con, &format!("gmns_{m}"), "link")? } else { has_table(con, m, "edges")? };
        if here {
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
    /// `--merge`: the edges of a chain of more than one edge, space-separated (the link's `edge_ids` attribute)
    edge_ids: Option<String>,
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
    /// GMNS `link.capacity` (vehicles/hour/lane); None: the class value of `hw`
    cap: Option<f64>,
    /// `--merge`: the edges of the chain (more than one), space-separated
    ids: Option<String>,
}

fn pos(v: Option<f64>) -> Option<f64> {
    v.filter(|x| *x > 0.0)
}

/// The link of one row (one mode).
fn single_link(r: &Row) -> Link {
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
        capacity: pos(r.cap).unwrap_or_else(|| capacity(norm_hw(r.hw.as_deref()))) * permlanes as f64,
        permlanes,
        modes: modes_str(&BTreeSet::from([r.mode.as_str()])),
        edge_ids: r.ids.clone(),
    }
}

/// The link of the rows of one `edge_id` (more than one mode). The car values come from the driving row.
fn multi_link(rs: &[Row]) -> Link {
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
        id: rs[0].id,
        src: rs[0].src,
        tgt: rs[0].tgt,
        length,
        freespeed,
        capacity: drow.and_then(|d| pos(d.cap)).unwrap_or_else(|| capacity(norm_hw(hw))) * permlanes as f64,
        permlanes,
        modes: modes_str(&modes),
        edge_ids: None,
    }
}

pub struct Summary {
    pub nodes: usize,
    pub links: usize,
    pub crs: String,
}

/// Write a MATSim `network.xml` (gzip when `gz`) from the database `db`, as `to_matsim` of src/duckosm/matsim.py.
///
/// The file is written while DuckDB reads the tables: no list of all nodes or all links is in memory. The links are in
/// the order of the rows of the edge tables (the modes in the order driving, cycling, walking), each with both its
/// nodes; more than one mode: one link for each `edge_id`, at the place of its first row, from the rows of all modes.
/// The nodes are the nodes of the links, in the order of their ids, in the CRS `crs` (default: the UTM zone of the data).
/// `merge`: driving only, one link for each chain (`driving.chains`; GMNS: `gmns_driving.chains`), its id the `chain_id`
/// and its edges in the attribute `edge_ids` (docs/design/sim_from_gmns.md, "Chains").
pub fn to_matsim(db: &Path, out: &Path, mode: &str, crs: Option<&str>, gz: bool, merge: bool) -> Result<Summary> {
    let con = open(db)?;
    let gmns = is_gmns(&con)?;
    let modes = resolve_modes(&con, mode, gmns)?;
    if merge {
        if modes != ["driving"] {
            return Err("merge: only the driving network has chains; give -m driving".into());
        }
        let (sch, t) = if gmns { ("gmns_driving", "chains") } else { ("driving", "chains") };
        if !has_table(&con, sch, t)? {
            return Err(format!("merge: {sch}.{t} is not in {}; make it again with this duckOSM version", db.display()));
        }
    }
    // the tables of a mode: duckOSM's, or GMNS's
    let nodes_of = |m: &str| if gmns { format!("gmns_{m}.node") } else { format!("{m}.nodes") };
    let crs = match crs {
        Some(c) => c.to_string(),
        None => data_utm_crs(&con, &nodes_of(modes[0]), "geom")?,
    };
    let union = |cols: &str, filter: &str| modes.iter().map(|m| format!("SELECT {cols} FROM {}{filter}", nodes_of(m))).collect::<Vec<_>>().join(" UNION ");
    // the nodes with coordinates in `crs`; a node of more than one mode has one place
    let xy = format!(
        "SELECT node_id, first(x) AS x, first(y) AS y FROM (SELECT node_id, ST_X(t) AS x, ST_Y(t) AS y FROM \
         (SELECT node_id, ST_Transform(geom, 'EPSG:4326', $1, always_xy := true) t FROM ({}))) \
         WHERE x IS NOT NULL AND y IS NOT NULL GROUP BY node_id",
        union("node_id, geom", "")
    );
    // the rows of the edge tables; `k` is the place of the row (the mode, then the row of the table)
    let rows = modes
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if merge && gmns {
                // a chain takes the values of its last link (the same on each link of the chain)
                "SELECT c.rowid AS k, CASE WHEN l.allowed_uses IS NULL OR l.allowed_uses LIKE '%auto%' THEN 'driving' ELSE l.allowed_uses END AS mode, \
                 c.chain_id AS edge_id, c.from_node_id AS source, c.to_node_id AS target, l.facility_type AS highway, l.lanes::BIGINT AS lanes, \
                 l.free_speed::DOUBLE AS spd, c.length::DOUBLE AS len, NULL::DOUBLE AS cost, l.capacity::DOUBLE AS cap, \
                 CASE WHEN len(c.link_ids) > 1 THEN array_to_string(c.link_ids, ' ') END AS ids \
                 FROM gmns_driving.chains c JOIN gmns_driving.link l ON l.link_id = c.link_ids[len(c.link_ids)]"
                    .to_string()
            } else if merge {
                format!(
                    "SELECT {i} * 1099511627776 + rowid AS k, '{m}' AS mode, chain_id AS edge_id, source, target, highway, lanes::BIGINT AS lanes, \
                     maxspeed_kmh::DOUBLE AS spd, length_m::DOUBLE AS len, cost_s::DOUBLE AS cost, NULL::DOUBLE AS cap, \
                     CASE WHEN len(edge_ids) > 1 THEN array_to_string(edge_ids, ' ') END AS ids FROM {m}.chains \
                     WHERE source IS NOT NULL AND target IS NOT NULL"
                )
            } else if gmns {
                // a driving link that cars cannot use (a bus-only link): its own uses (`bus`, `bus,bike`) are its modes
                let mode = if *m == "driving" {
                    "CASE WHEN allowed_uses IS NULL OR allowed_uses LIKE '%auto%' THEN 'driving' ELSE allowed_uses END".to_string()
                } else {
                    format!("'{m}'")
                };
                format!(
                    "SELECT {i} * 1099511627776 + rowid AS k, {mode} AS mode, link_id AS edge_id, from_node_id AS source, to_node_id AS target, \
                     facility_type AS highway, lanes::BIGINT AS lanes, free_speed::DOUBLE AS spd, length::DOUBLE AS len, NULL::DOUBLE AS cost, \
                     capacity::DOUBLE AS cap, NULL::VARCHAR AS ids FROM gmns_{m}.link WHERE from_node_id IS NOT NULL AND to_node_id IS NOT NULL"
                )
            } else {
                format!(
                    "SELECT {i} * 1099511627776 + rowid AS k, '{m}' AS mode, edge_id, source, target, highway, lanes::BIGINT AS lanes, \
                     maxspeed_kmh::DOUBLE AS spd, length_m::DOUBLE AS len, cost_s::DOUBLE AS cost, NULL::DOUBLE AS cap, NULL::VARCHAR AS ids FROM {m}.edges \
                     WHERE source IS NOT NULL AND target IS NOT NULL"
                )
            }
        })
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    let has = |col: &str| format!("{col} IN (SELECT node_id FROM xy)");
    // the links: their rows in the order of the file, and the nodes they use
    let (links, ends) = if modes.len() == 1 {
        (
            format!("SELECT k AS fk, * FROM r WHERE {} AND {} ORDER BY k", has("source"), has("target")),
            format!("SELECT source, target FROM r WHERE {} AND {}", has("source"), has("target")),
        )
    } else {
        // one link for each edge_id: its first row gives its place and its nodes
        let first = format!(
            "SELECT edge_id, min(k) AS fk, arg_min(source, k) AS source, arg_min(target, k) AS target FROM r GROUP BY edge_id \
             HAVING {} AND {}",
            has("arg_min(source, k)"),
            has("arg_min(target, k)")
        );
        (format!("SELECT f.fk, r.* FROM r JOIN ({first}) f USING (edge_id) ORDER BY f.fk, r.k"), format!("SELECT source, target FROM ({first})"))
    };
    let (ele_schema, ele_table, ele_col) = if gmns { (format!("gmns_{}", modes[0]), "node", "z_coord") } else { (modes[0].to_string(), "nodes", "ele") };
    let have_ele: i64 = con
        .query_row(
            "SELECT count(*) FROM duckdb_columns() WHERE schema_name = ? AND table_name = ? AND column_name = ?",
            [ele_schema.as_str(), ele_table, ele_col],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    // nodes.ele (GMNS: node.z_coord): only when `duckosm elevation` added it
    let ele = if have_ele > 0 {
        format!(
            "SELECT node_id, first(ele) AS ele FROM ({}) GROUP BY node_id",
            union(&format!("node_id, {ele_col}::DOUBLE AS ele"), &format!(" WHERE {ele_col} IS NOT NULL"))
        )
    } else {
        "SELECT NULL::BIGINT AS node_id, NULL::DOUBLE AS ele WHERE false".into()
    };
    let with = format!("WITH xy AS MATERIALIZED ({xy}), r AS ({rows})");

    let mut o = Out::create(out, gz)?;
    o.put("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")?;
    o.put(&format!("<!DOCTYPE network SYSTEM \"{DTD}\">\n<network>\n\t<attributes>\n"))?;
    o.put(&format!(
        "\t\t<attribute name=\"coordinateReferenceSystem\" class=\"java.lang.String\">{crs}</attribute>\n\t</attributes>\n\t<nodes>\n"
    ))?;
    let mut nodes = 0;
    each_chunk(
        &con,
        &format!(
            "{with}, used AS (SELECT source AS n FROM ({ends}) UNION SELECT target FROM ({ends})) \
             SELECT xy.node_id, xy.x, xy.y, e.ele FROM xy JOIN used ON xy.node_id = used.n LEFT JOIN ({ele}) e \
             ON e.node_id = xy.node_id ORDER BY xy.node_id"
        ),
        [&crs],
        |c| {
            for i in 0..c.len {
                let (n, x, y) = (c.i64(0, i).unwrap_or_default(), c.f64(1, i).unwrap_or_default(), c.f64(2, i).unwrap_or_default());
                let z = c.f64(3, i).map(|z| format!(" z=\"{z:.2}\"")).unwrap_or_default();
                o.put(&format!("\t\t<node id=\"{n}\" x=\"{x:.2}\" y=\"{y:.2}\"{z}/>\n"))?;
            }
            nodes += c.len;
            Ok(())
        },
    )?;
    o.put("\t</nodes>\n\t<links capperiod=\"01:00:00\" effectivecellsize=\"7.5\" effectivelanewidth=\"3.75\">\n")?;
    let mut n_links = 0;
    let mut put_link = |o: &mut Out, l: Link| {
        n_links += 1;
        let head = format!(
            "\t\t<link id=\"{}\" from=\"{}\" to=\"{}\" length=\"{:.2}\" freespeed=\"{:.4}\" capacity=\"{:.1}\" permlanes=\"{}\" modes={}",
            l.id,
            l.src,
            l.tgt,
            l.length,
            l.freespeed,
            l.capacity,
            l.permlanes,
            quoteattr(&l.modes)
        );
        match &l.edge_ids {
            // a merged link: its edges, so that edge_map maps the results back to each edge
            Some(ids) => o.put(&format!(
                "{head}>\n\t\t\t<attributes>\n\t\t\t\t<attribute name=\"edge_ids\" class=\"java.lang.String\">{ids}</attribute>\n\t\t\t</attributes>\n\t\t</link>\n"
            )),
            None => o.put(&format!("{head}/>\n")),
        }
    };
    // the rows of one link (more than one mode): they come one after the other
    let (mut group, mut group_at): (Vec<Row>, i64) = (Vec::new(), i64::MIN);
    each_chunk(&con, &format!("{with} {links}"), [&crs], |c| {
        for i in 0..c.len {
            let fk = c.i64(0, i).unwrap_or_default();
            let r = Row {
                mode: c.str(2, i).unwrap_or_default().to_string(),
                id: c.i64(3, i).unwrap_or_default(),
                src: c.i64(4, i).unwrap_or_default(),
                tgt: c.i64(5, i).unwrap_or_default(),
                hw: c.str(6, i).map(String::from),
                lanes: c.i64(7, i),
                spd: c.f64(8, i),
                length: c.f64(9, i),
                cost: c.f64(10, i),
                cap: c.f64(11, i),
                ids: c.str(12, i).map(String::from),
            };
            if modes.len() == 1 {
                put_link(&mut o, single_link(&r))?;
                continue;
            }
            if fk != group_at && !group.is_empty() {
                put_link(&mut o, multi_link(&group))?;
                group.clear();
            }
            group_at = fk;
            group.push(r);
        }
        Ok(())
    })?;
    if !group.is_empty() {
        put_link(&mut o, multi_link(&group))?;
    }
    o.put("\t</links>\n</network>\n")?;
    o.finish()?;
    Ok(Summary { nodes, links: n_links, crs })
}
