//! The SUMO export of a GMNS db (`gmns_<mode>` schemas): docs/design/sim_from_gmns.md.
//!
//! GMNS already has the lanes of each link (`lane`) and the lane-to-lane connections (`lane_connector`, from the netconvert
//! run of `duckosm gmns`). This export writes them as SUMO plain XML, so netconvert builds those lanes and does not select
//! them again. `sumo::to_sumo_with` sends a GMNS db here.

use crate::stream::each_chunk;
use crate::sumo::{edge_xml, find_netconvert, rel, write_tll, Edge, Summary, DEFAULT_NETCFG};
use crate::{has_table, open, Out, Result};
use std::collections::HashMap;
use std::path::Path;

fn e(e: duckdb::Error) -> String {
    e.to_string()
}

/// The SUMO node type of a GMNS `ctrl_type` (the same three types as the duckOSM input).
fn node_type(ctrl: Option<&str>) -> Option<&'static str> {
    match ctrl? {
        "signal" => Some("traffic_light"),
        "stop_4_way" => Some("allway_stop"),
        "stop" | "stop_2_way" => Some("priority_stop"),
        _ => None,
    }
}

/// The lane use of `edge_xml` (`bus`, `bike`, `bus,bike`, or "" for general traffic) of a GMNS `allowed_uses`.
fn lane_use(allowed: &str) -> &'static str {
    let has = |u: &str| allowed.split(',').any(|x| x.trim() == u);
    match (has("bus"), has("bike"), has("auto")) {
        (_, _, true) => "",
        (true, true, false) => "bus,bike",
        (true, false, false) => "bus",
        (false, true, false) => "bike",
        _ => "",
    }
}

/// The SUMO vehicle classes of a GMNS `allowed_uses` that has no `auto` (a bus-only link or turn); None: no limit.
fn sumo_allow(allowed: Option<&str>) -> Option<String> {
    let a = allowed?;
    let v: Vec<&str> = a
        .split(',')
        .map(str::trim)
        .filter_map(|u| match u {
            "bus" => Some("bus"),
            "bike" => Some("bicycle"),
            "walk" => Some("pedestrian"),
            _ => None,
        })
        .collect();
    (!a.split(',').any(|u| u.trim() == "auto") && !v.is_empty()).then(|| v.join(" "))
}

/// Export the mode `mode` of the GMNS db `db` to SUMO in `out_dir`. `config`: a .netccfg of the user (else the built-in
/// options). `netconvert`: also run netconvert (the .net.xml, and the .tll.xml of the signals).
///
/// `merge`: driving only, one SUMO edge for each chain of `gmns_driving.chains` (its id `chain_id`, its edges in
/// `<param key="edge_ids">`).
#[allow(clippy::too_many_arguments)]
pub fn to_sumo_gmns(db: &Path, out_dir: &Path, mode: &str, net_name: &str, connections: bool, config: Option<&Path>, netconvert: bool, merge: bool) -> Result<Summary> {
    let con = open(db)?;
    let sch = format!("gmns_{mode}");
    if !has_table(&con, &sch, "link")? {
        return Err(format!("{}: no {sch}.link — build the GMNS network of {mode} first (duckosm gmns -m {mode})", db.display()));
    }
    std::fs::create_dir_all(out_dir).map_err(|x| format!("{}: {x}", out_dir.display()))?;
    let path = |ext: &str| out_dir.join(format!("{net_name}.{ext}"));
    let (nod_path, edg_path, con_path, net_path) = (path("nod.xml"), path("edg.xml"), path("con.xml"), path("net.xml"));

    // the nodes, with the traffic lights and the stop signs of `ctrl_type` (driving only, as the duckOSM input)
    let (mut n_nodes, mut n_lights, mut n_stops) = (0, 0, 0);
    let mut f = Out::create(&nod_path, false)?;
    f.put("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<nodes>\n")?;
    each_chunk(&con, &format!("SELECT node_id, x_coord::DOUBLE, y_coord::DOUBLE, ctrl_type FROM {sch}.node WHERE x_coord IS NOT NULL AND y_coord IS NOT NULL ORDER BY rowid"), [], |c| {
        for i in 0..c.len {
            let t = if mode == "driving" { node_type(c.str(3, i)) } else { None };
            match t {
                Some("traffic_light") => n_lights += 1,
                Some(_) => n_stops += 1,
                None => {}
            }
            let a = t.map(|t| format!(" type=\"{t}\"")).unwrap_or_default();
            f.put(&format!(
                "  <node id=\"{}\" x=\"{:.7}\" y=\"{:.7}\"{a}/>\n",
                c.i64(0, i).unwrap_or_default(),
                c.f64(1, i).unwrap_or_default(),
                c.f64(2, i).unwrap_or_default()
            ))?;
        }
        n_nodes += c.len;
        Ok(())
    })?;
    f.put("</nodes>\n")?;
    f.finish()?;

    // the lanes of each link, left to right (by lane_num: a bike lane on the left is -1); in memory, a few per link
    // ponytail: all lanes in memory, a few hundred MB on a state; stream them with the links if a country needs it
    let mut lanes: HashMap<i64, Vec<(i64, String, f64)>> = HashMap::new();
    if has_table(&con, &sch, "lane")? {
        let mut st = con.prepare(&format!("SELECT link_id, lane_num, coalesce(allowed_uses, ''), coalesce(width, 3.25)::DOUBLE FROM {sch}.lane ORDER BY link_id, lane_num")).map_err(e)?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, f64>(3)?))).map_err(e)? {
            let (lk, n, u, w) = r.map_err(e)?;
            lanes.entry(lk).or_default().push((n, u, w));
        }
    }
    // SUMO's index of a lane: SUMO numbers the lanes from the right
    let index = |link: i64, lane_num: i64| -> Option<usize> {
        let ls = lanes.get(&link)?;
        ls.iter().position(|x| x.0 == lane_num).map(|p| ls.len() - 1 - p)
    };

    // the edges, in the order of the link table; `both`: the reverse link is there
    let mut n_edges = 0;
    let mut f = Out::create(&edg_path, false)?;
    f.put("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<edges>\n")?;
    let sql = format!(
        "SELECT l.link_id, l.from_node_id, l.to_node_id, l.facility_type, l.name, l.lanes::BIGINT, l.free_speed::DOUBLE, l.length::DOUBLE, \
         ST_AsText(l.geom), l.allowed_uses, \
         EXISTS (SELECT 1 FROM {sch}.link r WHERE r.from_node_id = l.to_node_id AND r.to_node_id = l.from_node_id) \
         FROM {sch}.link l WHERE l.from_node_id <> l.to_node_id ORDER BY l.rowid"
    );
    each_chunk(&con, &sql, [], |c| {
        for i in 0..c.len {
            let r = Edge {
                id: c.i64(0, i).unwrap_or_default(),
                src: c.i64(1, i).unwrap_or_default(),
                tgt: c.i64(2, i).unwrap_or_default(),
                hw: c.str(3, i).map(String::from),
                name: c.str(4, i).map(String::from),
                lanes: c.i64(5, i),
                spd: c.f64(6, i),
                length: c.f64(7, i),
                wkt: c.str(8, i).map(String::from),
            };
            let here: Option<Vec<(String, f64)>> = lanes.get(&r.id).map(|ls| ls.iter().map(|(_, u, w)| (lane_use(u).to_string(), *w)).collect());
            // a driving link that cars cannot use (bus-only): only its own vehicles
            let extra = if mode == "driving" { sumo_allow(c.str(9, i)).map(|a| vec![("allow".to_string(), a)]) } else { None };
            f.put(&edge_xml(&r, &r.src.to_string(), &r.tgt.to_string(), c.bool(10, i) == Some(true), here.as_deref(), extra.as_ref(), mode))?;
        }
        n_edges += c.len;
        Ok(())
    })?;
    f.put("</edges>\n")?;
    f.finish()?;

    // the connections: each lane pair of lane_connector; a movement with no lane pair, edge to edge; a link with no movement
    // out, `from` only (netconvert then makes no turn there)
    let mut n_conns = 0;
    let use_conns = connections && has_table(&con, &sch, "movement")?;
    if use_conns {
        let mut f = Out::create(&con_path, false)?;
        f.put("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<connections>\n")?;
        // walking: edge to edge only, as the duckOSM input (a footway has one lane, and netconvert puts the pedestrians on its
        // walking areas: it refuses a lane-to-lane connection there)
        let lc = mode != "walking" && has_table(&con, &sch, "lane_connector")?;
        let pairs = if lc {
            format!(
                "SELECT a.link_id AS ib, a.lane_num AS il, b.link_id AS ob, b.lane_num AS ol, m.allowed_uses AS uses, c.rowid AS k \
                 FROM {sch}.lane_connector c JOIN {sch}.lane a ON a.lane_id = c.from_lane_id JOIN {sch}.lane b ON b.lane_id = c.to_lane_id \
                 LEFT JOIN {sch}.movement m ON m.mvmt_id = c.mvmt_id"
            )
        } else {
            "SELECT NULL::BIGINT AS ib, NULL::BIGINT AS il, NULL::BIGINT AS ob, NULL::BIGINT AS ol, NULL::VARCHAR AS uses, NULL::BIGINT AS k WHERE false".to_string()
        };
        each_chunk(&con, &format!("SELECT ib, il, ob, ol, uses FROM ({pairs}) ORDER BY k"), [], |c| {
            for i in 0..c.len {
                let (ib, il, ob, ol) = (c.i64(0, i).unwrap_or_default(), c.i64(1, i).unwrap_or_default(), c.i64(2, i).unwrap_or_default(), c.i64(3, i).unwrap_or_default());
                let (Some(fl), Some(tl)) = (index(ib, il), index(ob, ol)) else { continue };
                // a bus-only or bike-only turn of the driving network (netconvert takes `allow` only lane to lane)
                let allow = if mode == "driving" { sumo_allow(c.str(4, i)).map(|a| format!(" allow=\"{a}\"")).unwrap_or_default() } else { String::new() };
                f.put(&format!("  <connection from=\"{ib}\" to=\"{ob}\" fromLane=\"{fl}\" toLane=\"{tl}\"{allow}/>\n"))?;
                n_conns += 1;
            }
            Ok(())
        })?;
        // the movements that have no lane pair (netconvert connected none of their lanes): netconvert selects the lanes. No
        // `allow` here: netconvert takes it only on a lane-to-lane connection
        let rest = format!(
            "SELECT m.ib_link_id, m.ob_link_id FROM {sch}.movement m \
             WHERE NOT EXISTS (SELECT 1 FROM ({pairs}) p WHERE p.ib = m.ib_link_id AND p.ob = m.ob_link_id) \
             GROUP BY ALL ORDER BY min(m.rowid)"
        );
        each_chunk(&con, &rest, [], |c| {
            for i in 0..c.len {
                f.put(&format!("  <connection from=\"{}\" to=\"{}\"/>\n", c.i64(0, i).unwrap_or_default(), c.i64(1, i).unwrap_or_default()))?;
                n_conns += 1;
            }
            Ok(())
        })?;
        let stuck = format!(
            "SELECT link_id FROM {sch}.link l WHERE l.from_node_id <> l.to_node_id \
             AND NOT EXISTS (SELECT 1 FROM {sch}.movement m WHERE m.ib_link_id = l.link_id) ORDER BY l.rowid"
        );
        each_chunk(&con, &stuck, [], |c| {
            for i in 0..c.len {
                f.put(&format!("  <connection from=\"{}\"/>\n", c.i64(0, i).unwrap_or_default()))?;
            }
            Ok(())
        })?;
        f.put("</connections>\n")?;
        f.finish()?;
    }
    eprintln!("SUMO[{mode}] from GMNS: {n_lights} traffic lights, {n_stops} stop signs, {n_conns} connections");
    // one edge for each chain: the same step as the duckOSM input, on the files just written
    if merge {
        let chains = crate::sumo_merge::Chains::read_gmns(&con)?;
        let before = n_edges;
        (n_nodes, n_edges) = crate::sumo_merge::merge_files(&chains, &nod_path, &edg_path, use_conns.then_some(con_path.as_path()), None)?;
        eprintln!("SUMO[{mode}] from GMNS: merge: {} chains join {before} edges into {n_edges}", chains.len());
    }

    // the netconvert options: the built-in ones, as the duckOSM input
    let mut opts: Vec<(String, String)> = DEFAULT_NETCFG.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    match mode {
        "walking" => opts.extend([("walkingareas".into(), "true".into()), ("default.lanewidth".into(), "2.0".into())]),
        "cycling" => opts.push(("default.lanewidth".into(), "1.5".into())),
        _ => {}
    }
    let write_cfg = |tll: Option<&Path>| -> Result<()> {
        let mut l = vec![
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
            "<configuration>".into(),
            format!("  <node-files value=\"{}\"/>", rel(&nod_path, out_dir)),
            format!("  <edge-files value=\"{}\"/>", rel(&edg_path, out_dir)),
        ];
        if use_conns {
            l.push(format!("  <connection-files value=\"{}\"/>", rel(&con_path, out_dir)));
        }
        if let Some(t) = tll {
            l.push(format!("  <tllogic-files value=\"{}\"/>", rel(t, out_dir)));
        }
        l.push(format!("  <output-file value=\"{}\"/>", rel(&net_path, out_dir)));
        for (k, v) in &opts {
            l.push(format!("  <{k} value=\"{v}\"/>"));
        }
        l.extend(["</configuration>".into(), String::new()]);
        std::fs::write(path("netccfg"), l.join("\n")).map_err(|x| x.to_string())
    };
    let mut cmd: Vec<String> = Vec::new();
    match config {
        Some(c) => {
            cmd.extend(["-c".into(), c.display().to_string(), "--node-files".into(), nod_path.display().to_string()]);
            cmd.extend(["--edge-files".into(), edg_path.display().to_string(), "--output-file".into(), net_path.display().to_string()]);
            if use_conns {
                cmd.extend(["--connection-files".into(), con_path.display().to_string()]);
            }
        }
        None => {
            write_cfg(None)?;
            cmd.extend(["-c".into(), path("netccfg").display().to_string()]);
        }
    }
    let mut s = Summary {
        nod: nod_path.clone(),
        edg: edg_path.clone(),
        con: use_conns.then(|| con_path.clone()),
        add: None,
        netccfg: Some(config.map_or_else(|| path("netccfg"), Path::to_path_buf)),
        net: None,
        tll: None,
        n_nodes,
        n_edges,
        n_connections: n_conns,
        n_traffic_lights: n_lights,
        n_stop_signs: n_stops,
        n_bus_stops: 0,
    };
    if !netconvert {
        return Ok(s);
    }
    let (bin, home) = find_netconvert()?;
    let name = bin.file_name().map_or_else(|| bin.display().to_string(), |n| n.to_string_lossy().into_owned());
    eprintln!("running: {name} {}", cmd.join(" "));
    let mut c = std::process::Command::new(&bin);
    c.args(&cmd);
    if let Some(h) = &home {
        if std::env::var_os("SUMO_HOME").is_none() {
            c.env("SUMO_HOME", h);
        }
    }
    let out = c.output().map_err(|x| format!("netconvert: {x}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("netconvert failed ({}):\n{}", out.status, &err[err.floor_char_boundary(err.len().saturating_sub(2000))..]));
    }
    let tll = path("tll.xml");
    if write_tll(&net_path, &tll)? > 0 {
        if config.is_none() {
            write_cfg(Some(&tll))?;
        }
        s.tll = Some(tll);
    }
    s.net = Some(net_path);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_use_and_allow() {
        assert_eq!(lane_use("auto"), "");
        assert_eq!(lane_use("bus,bike"), "bus,bike");
        assert_eq!(lane_use("bus"), "bus");
        assert_eq!(lane_use("bike"), "bike");
        assert_eq!(sumo_allow(Some("auto")), None);
        assert_eq!(sumo_allow(Some("bus,bike")).as_deref(), Some("bus bicycle"));
        assert_eq!(sumo_allow(None), None);
        assert_eq!(node_type(Some("stop_2_way")), Some("priority_stop"));
        assert_eq!(node_type(Some("yield")), None);
    }
}
