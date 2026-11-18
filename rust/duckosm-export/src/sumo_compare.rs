//! Compare the lanes that duckOSM gives SUMO with the lanes of SUMO's own OSM import (docs/design/gmns_lane_profile.md,
//! decision 2): the port of src/duckosm/sumo_compare.py. `lane_diff` builds both SUMO networks of one area and gives one row
//! for each OSM way and direction whose lanes differ, with the cause of the difference.

use crate::Result;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::process::Command;

/// (car, bus, bike) lanes.
pub type Lanes = (usize, usize, usize);

/// A way and direction whose lanes differ.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub way: String,
    pub dir: char,
    pub duckosm: Lanes,
    pub sumo_osm: Lanes,
    pub sources: Vec<String>,
    pub cause: &'static str,
}

/// The kind of a lane from its `allow` attribute.
fn kind(allow: &str) -> &'static str {
    let a: HashSet<&str> = allow.split_whitespace().collect();
    if a.len() == 1 && a.contains("bicycle") {
        "bike"
    } else if !a.is_empty() && !a.contains("passenger") && a.contains("bus") {
        "bus"
    } else if !a.is_empty() && !a.contains("passenger") {
        "other"
    } else {
        "car"
    }
}

fn attr(l: &str, k: &str) -> Option<String> {
    let key = format!(" {k}=\"");
    l.find(&key).map(|i| l[i + key.len()..].split('"').next().unwrap_or("").to_string())
}

/// `_lanes`: for each key (way, direction) of the edges of a .net.xml, the lanes of its longest kind of piece (one element on each
/// line, as netconvert writes). Of kinds as long, the first seen.
fn net_lanes(net: &Path, key: &dyn Fn(&str) -> Option<(String, char)>) -> Result<HashMap<(String, char), Lanes>> {
    let text = std::fs::read_to_string(net).map_err(|e| format!("{}: {e}", net.display()))?;
    let mut by: HashMap<(String, char), Vec<(Lanes, f64)>> = HashMap::new();
    let mut add = |k: Option<(String, char)>, lanes: &[(String, f64)]| {
        if let (Some(k), Some(first)) = (k, lanes.first()) {
            let ks: Vec<&str> = lanes.iter().map(|l| kind(&l.0)).collect();
            let n = |x: &str| ks.iter().filter(|k| **k == x).count();
            let c = (n("car"), n("bus"), n("bike"));
            let v = by.entry(k).or_default();
            match v.iter_mut().find(|e| e.0 == c) {
                Some(e) => e.1 += first.1,
                None => v.push((c, first.1)),
            }
        }
    };
    let (mut cur, mut lanes): (Option<Option<(String, char)>>, Vec<(String, f64)>) = (None, Vec::new());
    for l in text.lines().map(str::trim_start) {
        if l.starts_with("<edge ") {
            let internal = attr(l, "function").as_deref() == Some("internal");
            cur = if internal { None } else { Some(key(&attr(l, "id").unwrap_or_default())) };
            lanes.clear();
        } else if l.starts_with("<lane ") && cur.is_some() {
            lanes.push((attr(l, "allow").unwrap_or_default(), attr(l, "length").and_then(|v| v.parse().ok()).unwrap_or(0.0)));
        } else if l.starts_with("</edge>") {
            if let Some(k) = cur.take() {
                add(k, &lanes);
            }
        }
    }
    // Counter.most_common(1): the largest, the first inserted of equals
    Ok(by.into_iter().map(|(k, v)| (k, v.iter().fold(v[0], |b, x| if x.1 > b.1 { *x } else { b }).0)).collect())
}

fn query<T, F: FnMut(&duckdb::Row) -> duckdb::Result<T>>(con: &duckdb::Connection, sql: &str, f: F) -> Result<Vec<T>> {
    let mut st = con.prepare(sql).map_err(|e| format!("{e}\n{sql}"))?;
    let v = st.query_map([], f).map_err(|e| e.to_string())?.collect::<std::result::Result<Vec<T>, _>>();
    v.map_err(|e| e.to_string())
}

/// `lane_diff`: the ways and directions whose lanes differ between duckOSM's SUMO network (`to_sumo` of `db`) and SUMO's own
/// import of `osm_file` (a `.osm`; a `.pbf` is converted with osmium), and the number of road way-directions compared.
pub fn lane_diff(db: &Path, osm_file: &Path, work_dir: &Path) -> Result<(Vec<Row>, usize)> {
    std::fs::create_dir_all(work_dir).map_err(|e| format!("{}: {e}", work_dir.display()))?;
    let (bin, home) = crate::sumo::find_netconvert()?;
    let mut osm = osm_file.to_path_buf();
    if osm_file.to_string_lossy().ends_with(".pbf") {
        osm = work_dir.join("area.osm");
        let st = Command::new("osmium").arg("cat").arg(osm_file).arg("-o").arg(&osm).arg("--overwrite").output();
        match st {
            Ok(o) if o.status.success() => {}
            Ok(o) => return Err(format!("osmium cat failed: {}", String::from_utf8_lossy(&o.stderr))),
            Err(e) => return Err(format!("osmium: {e} (lane_diff needs osmium for a .pbf)")),
        }
    }
    let own = work_dir.join("osm.net.xml");
    let mut c = Command::new(&bin);
    c.arg("--osm-files").arg(&osm).arg("-o").arg(&own);
    for a in [
        "--geometry.remove", "false", "--ramps.guess", "false", "--junctions.join", "false", "--osm.turn-lanes", "true", "--osm.lane-access", "true",
        "--osm.bike-access", "true", "--proj.utm", "true", "--no-warnings", "true",
    ] {
        c.arg(a);
    }
    if let (Some(h), None) = (&home, std::env::var_os("SUMO_HOME")) {
        c.env("SUMO_HOME", h);
    }
    let res = c.output().map_err(|e| format!("{}: {e}", bin.display()))?;
    if !res.status.success() {
        let err = String::from_utf8_lossy(&res.stderr);
        let tail: String = err.chars().rev().take(2000).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!("netconvert --osm-files failed (exit {}):\n{tail}", res.status.code().unwrap_or(-1)));
    }
    let ours = crate::sumo::to_sumo(db, work_dir, "driving", "duckosm", true, None, true)?.net.ok_or("to_sumo wrote no .net.xml")?;
    let cfg = duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly).map_err(|e| e.to_string())?;
    let con = duckdb::Connection::open_with_flags(db, cfg).map_err(|e| format!("{}: {e}", db.display()))?;
    let refs: HashMap<String, String> =
        query(&con, "SELECT edge_id::VARCHAR, edge_ref FROM driving.edges", |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?
            .into_iter()
            .filter_map(|(k, v)| v.map(|v| (k, v)))
            .collect();
    // the tags that name a cause: oneway:bus, oneway:psv, and whether a lanes tag is there
    let tags: HashMap<String, (Option<String>, Option<String>, bool)> = query(
        &con,
        "SELECT osm_id::VARCHAR, tags['oneway:bus'], tags['oneway:psv'],
                coalesce(map_contains(tags, 'lanes') OR map_contains(tags, 'lanes:forward') OR map_contains(tags, 'lanes:backward'), false) FROM raw.ways",
        |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))),
    )?
    .into_iter()
    .collect();
    let mut src: HashMap<(String, char), BTreeSet<String>> = HashMap::new();
    for (r, s) in query(&con, "SELECT e.edge_ref, p.source FROM driving.lane_profile p JOIN driving.edges e USING (edge_id)", |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let k = (r.split('#').next().unwrap_or("").to_string(), r.chars().last().unwrap_or(' '));
        src.entry(k).or_default().insert(s);
    }
    drop(con);
    let osm_key = |i: &str| {
        let w = i.trim_start_matches('-').split('#').next().unwrap_or("");
        (!w.is_empty() && w.chars().all(|c| c.is_ascii_digit())).then(|| (w.to_string(), if i.starts_with('-') { 'r' } else { 'f' }))
    };
    let duck_key = |i: &str| refs.get(i).filter(|r| r.contains('#')).map(|r| (r.split('#').next().unwrap_or("").to_string(), r.chars().last().unwrap()));
    let (d, o) = (net_lanes(&ours, &duck_key)?, net_lanes(&own, &osm_key)?);
    let mut keys: Vec<&(String, char)> = d.keys().filter(|k| o.get(*k).is_some_and(|x| x.0 + x.1 > 0)).collect();
    keys.sort();
    let empty = BTreeSet::new();
    let mut rows = Vec::new();
    for k in &keys {
        let (dk, ok) = (d[*k], o[*k]);
        if dk == ok {
            continue;
        }
        let s = src.get(*k).unwrap_or(&empty);
        let (bus, psv, has_lanes) = tags.get(&k.0).cloned().unwrap_or((None, None, false));
        let cause = if s.contains("override") {
            "override"
        } else if bus.as_deref() == Some("no") || psv.as_deref() == Some("no") {
            "contraflow bus lane" // counted in lanes (OSM); SUMO's import adds it on top
        } else if s.contains("inherited") {
            "inherited"
        } else if s.contains("turn:lanes") {
            "turn:lanes count" // more turn:lanes entries than lanes: we take the entries, SUMO the lanes tag
        } else if s.contains("default") && !has_lanes {
            "no lanes tag" // our default 1 lane, SUMO's type default
        } else if dk.2 != ok.2 && (dk.0, dk.1) == (ok.0, ok.1) {
            "bike lane"
        } else if dk.1 != ok.1 {
            "bus lane"
        } else {
            "other"
        };
        rows.push(Row { way: k.0.clone(), dir: k.1, duckosm: dk, sumo_osm: ok, sources: s.iter().cloned().collect(), cause });
    }
    Ok((rows, keys.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(kind(""), "car");
        assert_eq!(kind("bicycle"), "bike");
        assert_eq!(kind("bus taxi"), "bus");
        assert_eq!(kind("bus passenger"), "car");
        assert_eq!(kind("pedestrian"), "other");
    }
}
