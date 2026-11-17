//! The MATSim turn lanes (`lanes.xml`, laneDefinitions_v2.0) and signals (`signalSystems`, `signalGroups`,
//! `signalControl` v2.0) from a GMNS db, the port of src/duckosm/matsim_lanes.py. docs/design/matsim_export.md.

use crate::{has_table, open, write_text, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

const NS: &str = "http://www.matsim.org/files/dtd";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

fn root(tag: &str, xsd: &str) -> String {
    format!("<{tag} xmlns=\"{NS}\" xmlns:xsi=\"{XSI}\" xsi:schemaLocation=\"{NS} {NS}/{xsd}\">")
}

/// A movement into a link: (node, outbound link, start_ib_lane, end_ib_lane, mvmt_code).
type Mv = (i64, i64, Option<i64>, Option<i64>, Option<String>);

/// The phase of an approach from its movement codes: NB/SB -> NS, EB/WB -> EW.
fn orient(ms: &[Mv]) -> &'static str {
    let first = |m: &Mv| m.4.as_deref().and_then(|c| c.chars().next());
    let ns = ms.iter().filter(|m| matches!(first(m), Some('N' | 'S'))).count();
    let ew = ms.iter().filter(|m| matches!(first(m), Some('E' | 'W'))).count();
    if ns >= ew {
        "NS"
    } else {
        "EW"
    }
}

/// The lane groups (represented lanes, outbound links): from turn:lanes when a movement has them, else one group.
fn lane_groups(ms: &[Mv], n_lanes: i64) -> Vec<(i64, Vec<i64>)> {
    if ms.iter().any(|m| m.2.is_some()) {
        let mut groups: BTreeMap<(i64, i64), BTreeSet<i64>> = BTreeMap::new();
        for (_, ob, sib, eib, _) in ms {
            let nz = |v: Option<i64>| v.filter(|x| *x != 0);
            let lo = nz(*sib).unwrap_or(1);
            let hi = nz(*eib).or(nz(*sib)).unwrap_or(1);
            groups.entry((lo, hi)).or_default().insert(*ob);
        }
        return groups.into_iter().map(|((lo, hi), obs)| ((hi - lo + 1).max(1), obs.into_iter().collect())).collect();
    }
    vec![(n_lanes, ms.iter().map(|m| m.1).collect::<BTreeSet<_>>().into_iter().collect())]
}

pub struct Summary {
    pub lanes_links: usize,
    pub signal_systems: Option<usize>,
}

/// Write `lanes.xml` (and the three signal files when `signals`) into `out_dir`, as `to_matsim_lanes`.
pub fn to_matsim_lanes(gmns_db: &Path, out_dir: &Path, mode: &str, signals: bool, cycle_s: i64) -> Result<Summary> {
    let con = open(gmns_db)?;
    let g = format!("gmns_{mode}");
    if !has_table(&con, &g, "movement")? {
        return Err(format!("no '{g}.movement' in {} — build a GMNS db first (duckosm gmns)", gmns_db.display()));
    }
    let e = |e: duckdb::Error| e.to_string();
    let mut links: HashMap<i64, (i64, f64, f64)> = HashMap::new();
    let mut st = con
        .prepare(&format!("SELECT link_id, lanes::BIGINT, capacity::DOUBLE, length::DOUBLE FROM {g}.link"))
        .map_err(e)?;
    for r in st
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<f64>>(2)?, r.get::<_, Option<f64>>(3)?)))
        .map_err(e)?
    {
        let (id, lanes, cap, len) = r.map_err(e)?;
        let nz = |v: Option<f64>, d: f64| v.filter(|x| *x != 0.0).unwrap_or(d);
        links.insert(id, (lanes.filter(|l| *l != 0).unwrap_or(1).max(1), nz(cap, 800.0), nz(len, 50.0)));
    }
    // the movements of each inbound link, the links in the order of their first movement
    let mut order: Vec<i64> = Vec::new();
    let mut by_ib: HashMap<i64, Vec<Mv>> = HashMap::new();
    let mut st = con
        .prepare(&format!(
            "SELECT node_id, ib_link_id, ob_link_id, start_ib_lane::BIGINT, end_ib_lane::BIGINT, mvmt_code FROM {g}.movement \
             WHERE ib_link_id IS NOT NULL AND ob_link_id IS NOT NULL"
        ))
        .map_err(e)?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(1)?, (r.get(0)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))).map_err(e)? {
        let (ib, m): (i64, Mv) = r.map_err(e)?;
        by_ib.entry(ib).or_insert_with(|| {
            order.push(ib);
            Vec::new()
        });
        by_ib.get_mut(&ib).unwrap().push(m);
    }
    let mut st = con.prepare(&format!("SELECT node_id FROM {g}.node WHERE ctrl_type='signal'")).map_err(e)?;
    let sig_nodes: HashSet<i64> = st.query_map([], |r| r.get(0)).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
    drop(st);
    drop(con);
    std::fs::create_dir_all(out_dir).map_err(|x| format!("{}: {x}", out_dir.display()))?;

    let mut o = vec!["<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(), root("laneDefinitions", "laneDefinitions_v2.0.xsd")];
    for ib in &order {
        let ms = &by_ib[ib];
        let (n_lanes, per_cap, length) = links.get(ib).copied().unwrap_or((1, 800.0, 50.0));
        o.push(format!("\t<lanesToLinkAssignment linkIdRef=\"{ib}\">"));
        for (k, (rep, obs)) in lane_groups(ms, n_lanes).into_iter().enumerate() {
            o.push(format!("\t\t<lane id=\"{ib}.{}\">", k + 1));
            o.push("\t\t\t<leadsTo>".into());
            o.extend(obs.iter().map(|ob| format!("\t\t\t\t<toLink refId=\"{ob}\"/>")));
            o.push("\t\t\t</leadsTo>".into());
            o.push(format!("\t\t\t<representedLanes number=\"{rep}\"/>"));
            o.push(format!("\t\t\t<capacity vehiclesPerHour=\"{:.1}\"/>", per_cap * rep as f64));
            o.push(format!("\t\t\t<startsAt meterFromLinkEnd=\"{:.1}\"/>", length.min(45.0)));
            o.push("\t\t\t<alignment>0</alignment>".into());
            o.push("\t\t</lane>".into());
        }
        o.push("\t</lanesToLinkAssignment>".into());
    }
    o.push("</laneDefinitions>\n".into());
    write_text(&out_dir.join("lanes.xml"), &o.join("\n"), false)?;
    let mut s = Summary { lanes_links: order.len(), signal_systems: None };
    if !signals {
        return Ok(s);
    }

    // the approaches of each signal node: (inbound link, its outbound links, NS or EW)
    let mut node_order: Vec<i64> = Vec::new();
    let mut node_appr: HashMap<i64, Vec<(i64, Vec<i64>, &str)>> = HashMap::new();
    for ib in &order {
        let ms = &by_ib[ib];
        let node = ms[0].0;
        if !sig_nodes.contains(&node) {
            continue;
        }
        let obs: Vec<i64> = ms.iter().map(|m| m.1).collect::<BTreeSet<_>>().into_iter().collect();
        node_appr.entry(node).or_insert_with(|| {
            node_order.push(node);
            Vec::new()
        });
        node_appr.get_mut(&node).unwrap().push((*ib, obs, orient(ms)));
    }
    let head = |t: &str, x: &str| vec!["<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(), root(t, x)];
    let (mut ss, mut sg, mut sc) =
        (head("signalSystems", "signalSystems_v2.0.xsd"), head("signalGroups", "signalGroups_v2.0.xsd"), head("signalControl", "signalControl_v2.0.xsd"));
    let (half, allred) = (cycle_s.div_euclid(2), 3);
    for node in &node_order {
        ss.push(format!("\t<signalSystem id=\"{node}\">"));
        ss.push("\t\t<signals>".into());
        sg.push(format!("\t<signalSystem refId=\"{node}\">"));
        sc.push(format!("\t<signalSystem refId=\"{node}\">"));
        sc.push("\t\t<signalSystemController>".into());
        sc.push("\t\t\t<controllerIdentifier>DefaultPlanbasedSignalSystemController</controllerIdentifier>".into());
        sc.push("\t\t\t<signalPlan id=\"1\">".into());
        sc.push(format!("\t\t\t\t<cycleTime sec=\"{cycle_s}\"/>"));
        sc.push("\t\t\t\t<offset sec=\"0\"/>".into());
        for (ib, obs, orient) in &node_appr[node] {
            let sid = format!("{node}.{ib}");
            ss.push(format!("\t\t\t<signal id=\"{sid}\" linkIdRef=\"{ib}\">"));
            ss.push("\t\t\t\t<turningMoveRestrictions>".into());
            ss.extend(obs.iter().map(|ob| format!("\t\t\t\t\t<toLink refId=\"{ob}\"/>")));
            ss.push("\t\t\t\t</turningMoveRestrictions>".into());
            ss.push("\t\t\t</signal>".into());
            sg.push(format!("\t\t<signalGroup id=\"{sid}\">"));
            sg.push(format!("\t\t\t<signal refId=\"{sid}\"/>"));
            sg.push("\t\t</signalGroup>".into());
            let (on, drop) = if *orient == "NS" { (0, half - allred) } else { (half, cycle_s - allred) };
            sc.push(format!("\t\t\t\t<signalGroupSettings refId=\"{sid}\">"));
            sc.push(format!("\t\t\t\t\t<onset sec=\"{on}\"/>"));
            sc.push(format!("\t\t\t\t\t<dropping sec=\"{drop}\"/>"));
            sc.push("\t\t\t\t</signalGroupSettings>".into());
        }
        ss.push("\t\t</signals>".into());
        ss.push("\t</signalSystem>".into());
        sg.push("\t</signalSystem>".into());
        sc.push("\t\t\t</signalPlan>".into());
        sc.push("\t\t</signalSystemController>".into());
        sc.push("\t</signalSystem>".into());
    }
    ss.push("</signalSystems>\n".into());
    sg.push("</signalGroups>\n".into());
    sc.push("</signalControl>\n".into());
    for (name, lines) in [("signalSystems", ss), ("signalGroups", sg), ("signalControl", sc)] {
        write_text(&out_dir.join(format!("{name}.xml")), &lines.join("\n"), false)?;
    }
    s.signal_systems = Some(node_order.len());
    Ok(s)
}
