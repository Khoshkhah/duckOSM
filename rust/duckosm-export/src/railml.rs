//! The railML 2.4 infrastructure export, the port of src/duckosm/railml.py. docs/design/railml_export.md.
//!
//! The `railway` ways of `raw.ways` are cut into tracks at the shared nodes and the switches. The ends of the tracks that
//! meet at a node are connected; the switches, signals and stations (OCPs) are added.

use crate::{has_table, open, quoteattr, write_text, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const RAIL: [&str; 6] = ["rail", "light_rail", "subway", "tram", "narrow_gauge", "funicular"];
const NS: &str = "https://www.railml.org/schemas/2013";

/// The metres from the start at each point, and the length (an equirectangular approximation), as `_mlen`.
fn mlen(pts: &[(f64, f64)]) -> (Vec<f64>, f64) {
    let mut cum = vec![0.0];
    for w in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        let kx = ((y0 + y1) / 2.0).to_radians().cos();
        let last = *cum.last().unwrap();
        cum.push(last + ((x1 - x0) * kx).hypot(y1 - y0) * 111320.0);
    }
    let total = *cum.last().unwrap();
    (cum, total)
}

struct Track {
    id: String,
    rw: String,
    nodes: Vec<i64>,
    cum: Vec<f64>,
    total: f64,
}

pub struct Summary {
    pub tracks: usize,
    pub switches: usize,
    pub signals: usize,
    pub ocps: usize,
}

fn ids(s: &str) -> Vec<i64> {
    s.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

/// Write a railML 2.4 infrastructure file from the `raw` schema of the database `db`, as `to_railml`.
pub fn to_railml(db: &Path, out: &Path) -> Result<Summary> {
    let con = open(db)?;
    if !has_table(&con, "raw", "ways")? {
        return Err(format!("no 'raw.ways' in {} — need a built duckOSM db (with the raw OSM schema)", db.display()));
    }
    let types = RAIL.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(",");
    let e = |e: duckdb::Error| e.to_string();
    let mut st = con
        .prepare(&format!(
            "SELECT osm_id, tags['railway'], array_to_string(refs, ',') FROM raw.ways WHERE tags['railway'] IN ({types}) AND refs IS NOT NULL"
        ))
        .map_err(e)?;
    let ways: Vec<(i64, String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(e)?
        .collect::<std::result::Result<_, _>>()
        .map_err(e)?;
    let mut st = con
        .prepare(&format!(
            "SELECT osm_id, lon, lat, tags['railway'] FROM raw.nodes WHERE osm_id IN \
             (SELECT unnest(refs) FROM raw.ways WHERE tags['railway'] IN ({types}))"
        ))
        .map_err(e)?;
    let mut nodes: HashMap<i64, (f64, f64, Option<String>)> = HashMap::new();
    for r in st.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))).map_err(e)? {
        let (k, v) = r.map_err(e)?;
        nodes.insert(k, v);
    }
    let mut st = con
        .prepare("SELECT osm_id, lon, lat, tags['name'] FROM raw.nodes WHERE tags['railway'] IN ('station','halt')")
        .map_err(e)?;
    let ocps: Vec<(i64, f64, f64, Option<String>)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(e)?
        .collect::<std::result::Result<_, _>>()
        .map_err(e)?;
    drop(st);
    drop(con);

    let tag = |n: i64| nodes.get(&n).and_then(|t| t.2.as_deref());
    // cut each way at the shared nodes and the switches
    let mut occ: HashMap<i64, usize> = HashMap::new();
    for (_, _, refs) in &ways {
        for r in ids(refs) {
            *occ.entry(r).or_default() += 1;
        }
    }
    let is_split = |n: i64| occ.get(&n).copied().unwrap_or(0) >= 2 || tag(n) == Some("switch");
    let mut tracks = Vec::new();
    for (oid, rw, refs) in &ways {
        let refs: Vec<i64> = ids(refs).into_iter().filter(|r| nodes.contains_key(r)).collect();
        if refs.len() < 2 {
            continue;
        }
        let mut segs = Vec::new();
        let mut cur = vec![refs[0]];
        for &r in &refs[1..] {
            cur.push(r);
            if is_split(r) && r != *refs.last().unwrap() {
                segs.push(std::mem::replace(&mut cur, vec![r]));
            }
        }
        segs.push(cur);
        for s in segs {
            if s.len() < 2 {
                continue;
            }
            let pts: Vec<(f64, f64)> = s.iter().map(|n| (nodes[n].0, nodes[n].1)).collect();
            let (cum, total) = mlen(&pts);
            if total <= 0.0 {
                continue;
            }
            tracks.push(Track { id: format!("tr_{oid}_{}_{}", s[0], s[s.len() - 1]), rw: rw.clone(), nodes: s, cum, total });
        }
    }

    // the topology: the ends of the tracks at each node (in the order of the first end), then the connections
    let mut node_order: Vec<i64> = Vec::new();
    let mut node_ends: HashMap<i64, Vec<(usize, bool)>> = HashMap::new(); // (track, true = begin)
    for (i, t) in tracks.iter().enumerate() {
        for (n, begin) in [(t.nodes[0], true), (*t.nodes.last().unwrap(), false)] {
            node_ends
                .entry(n)
                .or_insert_with(|| {
                    node_order.push(n);
                    Vec::new()
                })
                .push((i, begin));
        }
    }
    let mut end_xml: HashMap<(usize, bool), String> = HashMap::new();
    let mut switch_nodes: HashSet<i64> = HashSet::new();
    let mut cn = 0;
    for node in &node_order {
        let ends = &node_ends[node];
        let rw = tag(*node);
        if ends.len() == 1 {
            end_xml.insert(
                ends[0],
                if rw == Some("buffer_stop") { format!("<bufferStop id=\"bs_{node}\"/>") } else { format!("<openEnd id=\"oe_{node}\"/>") },
            );
            continue;
        }
        if ends.len() >= 3 || rw == Some("switch") {
            switch_nodes.insert(*node);
        }
        let mut i = 0;
        while i + 1 < ends.len() {
            end_xml.insert(ends[i], format!("<connection id=\"cn{cn}\" ref=\"cn{}\"/>", cn + 1));
            end_xml.insert(ends[i + 1], format!("<connection id=\"cn{}\" ref=\"cn{cn}\"/>", cn + 1));
            cn += 2;
            i += 2;
        }
        if i < ends.len() {
            end_xml.insert(ends[i], format!("<openEnd id=\"oe_{node}_{}\"/>", ends[i].0));
        }
    }

    // a switch goes to its first track, a signal to each track that has it
    let mut seen_switch: HashSet<i64> = HashSet::new();
    let mut switch_of: HashMap<usize, Vec<(i64, f64)>> = HashMap::new();
    let mut signal_of: HashMap<usize, Vec<(i64, f64)>> = HashMap::new();
    let mut n_signals = 0;
    for (i, t) in tracks.iter().enumerate() {
        for (k, &n) in t.nodes.iter().enumerate() {
            if tag(n) == Some("signal") {
                signal_of.entry(i).or_default().push((n, t.cum[k]));
                n_signals += 1;
            }
            if switch_nodes.contains(&n) && seen_switch.insert(n) {
                switch_of.entry(i).or_default().push((n, t.cum[k]));
            }
        }
    }

    let mut o = vec![
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
        format!("<railml xmlns=\"{NS}\" version=\"2.4\">"),
        "\t<infrastructure id=\"is_1\">".into(),
        "\t\t<operationControlPoints>".into(),
    ];
    for (nid, lon, lat, name) in &ocps {
        let name = name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("ocp_{nid}"));
        o.push(format!("\t\t\t<ocp id=\"ocp_{nid}\" name={}>", quoteattr(&name)));
        o.push(format!("\t\t\t\t<geoCoord coord=\"{lon:.6} {lat:.6}\" epsgCode=\"4326\"/>"));
        o.push("\t\t\t</ocp>".into());
    }
    o.push("\t\t</operationControlPoints>".into());
    o.push("\t\t<tracks>".into());
    for (i, t) in tracks.iter().enumerate() {
        let kind = if ["rail", "subway", "light_rail"].contains(&t.rw.as_str()) { "mainTrack" } else { "secondaryTrack" };
        let begin = end_xml.get(&(i, true)).cloned().unwrap_or_else(|| format!("<openEnd id=\"oeb_{i}\"/>"));
        let end = end_xml.get(&(i, false)).cloned().unwrap_or_else(|| format!("<openEnd id=\"oee_{i}\"/>"));
        o.push(format!("\t\t\t<track id=\"{}\" type=\"{kind}\">", t.id));
        o.push("\t\t\t\t<trackTopology>".into());
        o.push(format!("\t\t\t\t\t<trackBegin id=\"tb_{i}\" pos=\"0.0\">{begin}</trackBegin>"));
        o.push(format!("\t\t\t\t\t<trackEnd id=\"te_{i}\" pos=\"{:.3}\">{end}</trackEnd>", t.total));
        o.push("\t\t\t\t</trackTopology>".into());
        if let Some(sw) = switch_of.get(&i) {
            o.push("\t\t\t\t<trackElements>".into());
            o.push("\t\t\t\t\t<switches>".into());
            for (node, pos) in sw {
                o.push(format!("\t\t\t\t\t\t<switch id=\"sw_{node}\" pos=\"{pos:.3}\" name=\"sw_{node}\"/>"));
            }
            o.push("\t\t\t\t\t</switches>".into());
            o.push("\t\t\t\t</trackElements>".into());
        }
        if let Some(sg) = signal_of.get(&i) {
            o.push("\t\t\t\t<ocsElements>".into());
            o.push("\t\t\t\t\t<signals>".into());
            for (node, pos) in sg {
                o.push(format!("\t\t\t\t\t\t<signal id=\"sig_{node}\" pos=\"{pos:.3}\" dir=\"up\"/>"));
            }
            o.push("\t\t\t\t\t</signals>".into());
            o.push("\t\t\t\t</ocsElements>".into());
        }
        o.push("\t\t\t</track>".into());
    }
    o.push("\t\t</tracks>".into());
    o.push("\t</infrastructure>".into());
    o.push("</railml>\n".into());
    write_text(out, &o.join("\n"), false)?;
    Ok(Summary { tracks: tracks.len(), switches: seen_switch.len(), signals: n_signals, ocps: ocps.len() })
}
