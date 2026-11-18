//! The value checks of a GMNS database, the port of src/duckosm/gmns_check.py (docs/design/gmns_spec_conformance.md): what
//! the values say about each other (a link starts and ends on its nodes, lane numbers run 1..n, a movement turns where its
//! inbound link ends, ...), and how connected each network is.

use crate::Result;
use duckdb::Connection;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const TOL_DEG: f64 = 1e-6;
const LEN_TOL: f64 = 0.01;

/// (name, bad-count SQL, total-count SQL) of the checks of schema `s` (`_checks`).
fn checks(s: &str) -> Vec<(&'static str, String, String)> {
    let (l, n, k, m) = (format!("{s}.link"), format!("{s}.node"), format!("{s}.lane"), format!("{s}.movement"));
    let cnt = format!("(SELECT link_id, count(*) n, min(lane_num) lo, max(lane_num) hi FROM {k} GROUP BY link_id)");
    let all = |t: &str| format!("SELECT count(*) FROM {t}");
    vec![
        ("link starts at its from_node", format!("SELECT count(*) FROM {l} l JOIN {n} n ON n.node_id = l.from_node_id
            WHERE abs(ST_X(ST_StartPoint(l.geom)) - n.x_coord) > {TOL_DEG}
               OR abs(ST_Y(ST_StartPoint(l.geom)) - n.y_coord) > {TOL_DEG}"), all(&l)),
        ("link ends at its to_node", format!("SELECT count(*) FROM {l} l JOIN {n} n ON n.node_id = l.to_node_id
            WHERE abs(ST_X(ST_EndPoint(l.geom)) - n.x_coord) > {TOL_DEG}
               OR abs(ST_Y(ST_EndPoint(l.geom)) - n.y_coord) > {TOL_DEG}"), all(&l)),
        ("link from_node / to_node exist", format!("SELECT count(*) FROM {l} WHERE from_node_id NOT IN
            (SELECT node_id FROM {n}) OR to_node_id NOT IN (SELECT node_id FROM {n})"), all(&l)),
        ("link is not a loop", format!("SELECT count(*) FROM {l} WHERE from_node_id = to_node_id"), all(&l)),
        ("link.length is its geometry's length", format!("SELECT count(*) FROM {l} WHERE abs(length /
            NULLIF(ST_Length_Spheroid(ST_FlipCoordinates(geom)), 0) - 1) > {LEN_TOL}"), all(&l)),
        ("node has a link", format!("SELECT count(*) FROM {n} WHERE node_id NOT IN
            (SELECT from_node_id FROM {l} UNION SELECT to_node_id FROM {l})"), all(&n)),
        ("link has lane rows", format!("SELECT count(*) FROM {l} WHERE link_id NOT IN (SELECT link_id FROM {k})"), all(&l)),
        ("lane_num is 1..n", format!("SELECT count(*) FROM {cnt} WHERE lo <> 1 OR hi <> n"), format!("SELECT count(*) FROM {cnt}")),
        ("link.lanes is not more than the lane rows", format!("SELECT count(*) FROM {l} l JOIN {cnt} c USING (link_id)
            WHERE l.lanes > c.n"), format!("SELECT count(*) FROM {l} WHERE lanes IS NOT NULL")),
        ("location lies on its link", format!("SELECT count(*) FROM {s}.location x LEFT JOIN {l} l USING (link_id)
            WHERE l.link_id IS NULL OR x.lr < 0 OR x.lr > l.length + 0.01"), format!("SELECT count(*) FROM {s}.location")),
        ("movement turns where its inbound link ends", format!("SELECT count(*) FROM {m} m JOIN {l} l ON l.link_id = m.ib_link_id
            WHERE l.to_node_id <> m.node_id"), all(&m)),
        ("movement turns where its outbound link starts", format!("SELECT count(*) FROM {m} m JOIN {l} l ON l.link_id = m.ob_link_id
            WHERE l.from_node_id <> m.node_id"), all(&m)),
        ("movement ib and ob links differ", format!("SELECT count(*) FROM {m} WHERE ib_link_id = ob_link_id"), all(&m)),
        ("movement inbound lanes exist", format!("SELECT count(*) FROM {m} m JOIN {cnt} c ON c.link_id = m.ib_link_id
            WHERE m.start_ib_lane IS NOT NULL AND (m.start_ib_lane < 1 OR m.end_ib_lane > c.n
                                                   OR m.start_ib_lane > m.end_ib_lane)"), all(&m)),
        ("movement outbound lanes exist", format!("SELECT count(*) FROM {m} m JOIN {cnt} c ON c.link_id = m.ob_link_id
            WHERE m.start_ob_lane IS NOT NULL AND (m.start_ob_lane < 1 OR m.end_ob_lane > c.n
                                                   OR m.start_ob_lane > m.end_ob_lane)"), all(&m)),
        ("movement lane ranges are as long on both sides", format!("SELECT count(*) FROM {m}
            WHERE start_ib_lane IS NOT NULL AND start_ob_lane IS NOT NULL
              AND end_ib_lane - start_ib_lane <> end_ob_lane - start_ob_lane"), all(&m)),
        ("movement has both lane ranges or neither", format!("SELECT count(*) FROM {m}
            WHERE (start_ib_lane IS NULL) <> (end_ib_lane IS NULL) OR (start_ob_lane IS NULL) <> (end_ob_lane IS NULL)"), all(&m)),
    ]
}

fn open(path: &Path) -> Result<Connection> {
    let cfg = duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly).map_err(|e| e.to_string())?;
    let con = Connection::open_with_flags(path, cfg).map_err(|e| format!("{}: {e}", path.display()))?;
    con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;").map_err(|e| e.to_string())?;
    Ok(con)
}

fn strings(con: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut st = con.prepare(sql).map_err(|e| format!("{e}\n{sql}"))?;
    let v = st.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<std::result::Result<Vec<String>, _>>();
    v.map_err(|e| e.to_string())
}

fn count(con: &Connection, sql: &str) -> Result<i64> {
    con.query_row(sql, [], |r| r.get(0)).map_err(|e| format!("{e}\n{sql}"))
}

/// `check_gmns`: the checks of every `gmns_<mode>` schema (or only `modes`): (schema, check, bad, total); bad 0 passes.
pub fn check_gmns(path: &Path, modes: &[String]) -> Result<Vec<(String, &'static str, i64, i64)>> {
    let con = open(path)?;
    let schemas: Vec<String> = strings(
        &con,
        "SELECT DISTINCT table_schema FROM information_schema.tables WHERE table_name = 'lane' AND table_schema LIKE 'gmns\\_%' ESCAPE '\\' ORDER BY 1",
    )?
    .into_iter()
    .filter(|s| modes.is_empty() || modes.iter().any(|m| s[5..] == *m))
    .collect();
    let have: HashSet<String> = strings(&con, "SELECT table_schema || '.' || table_name FROM information_schema.tables")?.into_iter().collect();
    let mut out = Vec::new();
    for s in &schemas {
        for (name, bad, total) in checks(s) {
            if have.contains(&format!("{s}.location")) || !bad.contains(".location") {
                out.push((s.clone(), name, count(&con, &bad)?, count(&con, &total)?));
            }
        }
    }
    Ok(out)
}

/// A line of `graph_report`.
pub struct Graph {
    pub schema: String,
    pub nodes: i64,
    pub links: usize,
    pub biggest: usize,
    pub share: f64,
    pub parts: usize,
    pub dead_ends: usize,
}

/// `graph_report`: how connected each `gmns_<mode>` network is: its largest strongly connected part, the parts in all, the
/// links whose end node has no way on.
pub fn graph_report(path: &Path, modes: &[String]) -> Result<Vec<Graph>> {
    let con = open(path)?;
    let schemas = strings(
        &con,
        "SELECT DISTINCT table_schema FROM information_schema.tables WHERE table_name = 'link' AND table_schema LIKE 'gmns\\_%' ESCAPE '\\' AND table_schema <> 'gmns_all' ORDER BY 1",
    )?;
    let mut out = Vec::new();
    for s in schemas {
        if !modes.is_empty() && !modes.iter().any(|m| s[5..] == *m) {
            continue;
        }
        let sql = format!("SELECT from_node_id, to_node_id FROM {s}.link");
        let mut st = con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
        let edges: Vec<(i64, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
        let nodes = count(&con, &format!("SELECT count(*) FROM {s}.node"))?;
        let sizes = strong_components(&edges);
        let outgoing: HashSet<i64> = edges.iter().map(|e| e.0).collect();
        let dead = edges.iter().filter(|e| !outgoing.contains(&e.1)).count();
        let big = sizes.iter().copied().max().unwrap_or(0);
        out.push(Graph { schema: s, nodes, links: edges.len(), biggest: big, share: if nodes > 0 { big as f64 / nodes as f64 } else { 0.0 }, parts: sizes.len(), dead_ends: dead });
    }
    Ok(out)
}

/// The sizes of the strongly connected components of a directed graph (Kosaraju, without recursion).
fn strong_components(edges: &[(i64, i64)]) -> Vec<usize> {
    let mut fwd: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut back: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut nodes: Vec<i64> = Vec::new();
    let mut seen_node = HashSet::new();
    for &(a, b) in edges {
        fwd.entry(a).or_default().push(b);
        back.entry(b).or_default().push(a);
        for n in [a, b] {
            if seen_node.insert(n) {
                nodes.push(n);
            }
        }
    }
    let empty = Vec::new();
    let (mut order, mut seen) = (Vec::new(), HashSet::new());
    for &root in &nodes {
        if !seen.insert(root) {
            continue;
        }
        let mut stack: Vec<(i64, usize)> = vec![(root, 0)];
        while let Some((v, i)) = stack.pop() {
            let next = fwd.get(&v).unwrap_or(&empty);
            if let Some(&w) = next[i..].iter().find(|w| !seen.contains(*w)) {
                let at = i + next[i..].iter().position(|x| *x == w).unwrap();
                stack.push((v, at + 1));
                seen.insert(w);
                stack.push((w, 0));
            } else {
                order.push(v);
            }
        }
    }
    let (mut sizes, mut done) = (Vec::new(), HashSet::new());
    for &root in order.iter().rev() {
        if !done.insert(root) {
            continue;
        }
        let (mut stack, mut size) = (vec![root], 0);
        while let Some(v) = stack.pop() {
            size += 1;
            for &w in back.get(&v).unwrap_or(&empty) {
                if done.insert(w) {
                    stack.push(w);
                }
            }
        }
        sizes.push(size);
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components() {
        // 1 <-> 2 -> 3 <-> 4, 5 alone after 4
        let mut s = strong_components(&[(1, 2), (2, 1), (2, 3), (3, 4), (4, 3), (4, 5)]);
        s.sort();
        assert_eq!(s, vec![1, 2, 2]);
    }
}
