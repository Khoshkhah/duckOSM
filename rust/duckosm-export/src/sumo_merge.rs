//! SUMO `merge`: one SUMO edge for each chain of `driving.chains` (docs/reference/database.md#chains). The export writes
//! its plain XML files edge by edge; this step then joins the edges of each chain in those files. The id of a joined
//! edge is its `chain_id`, and its `<param key="edge_ids">` lists the duckOSM edges, so that `edge_map` maps it back.

use crate::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The chains of more than one edge.
pub struct Chains {
    /// edge -> (chain_id, position in the chain)
    of: HashMap<i64, (i64, usize)>,
    /// chain_id -> (edges, SUMO shape, length)
    chain: HashMap<i64, (Vec<i64>, Option<String>, f64)>,
    /// edge -> the length of the chain's edges before it
    offset: HashMap<i64, f64>,
}

impl Chains {
    pub fn read(con: &duckdb::Connection, mode: &str) -> Result<Chains> {
        if !crate::has_table(con, mode, "chains")? {
            return Err(format!(
                "merge: {mode}.chains is not in the file; build it again with this duckOSM version"
            ));
        }
        let e = |x: duckdb::Error| x.to_string();
        let mut st = con
            .prepare(&format!(
                "SELECT c.chain_id, list(m.edge_id ORDER BY m.i), list(coalesce(e.length_m, 0)::DOUBLE ORDER BY m.i), any_value(ST_AsText(c.geometry)) \
                 FROM {mode}.chains c, unnest(c.edge_ids) WITH ORDINALITY AS m(edge_id, i) JOIN {mode}.edges e ON e.edge_id = m.edge_id \
                 WHERE len(c.edge_ids) > 1 GROUP BY c.chain_id"
            ))
            .map_err(e)?;
        let rows = st
            .query_map([], |r| {
                let ids: Vec<i64> = list(r.get::<_, duckdb::types::Value>(1)?);
                let lens: Vec<f64> = match r.get::<_, duckdb::types::Value>(2)? {
                    duckdb::types::Value::List(l) => l
                        .into_iter()
                        .map(|v| {
                            if let duckdb::types::Value::Double(x) = v {
                                x
                            } else {
                                0.0
                            }
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                Ok((
                    r.get::<_, i64>(0)?,
                    ids,
                    lens,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(e)?;
        let mut c = Chains {
            of: HashMap::new(),
            chain: HashMap::new(),
            offset: HashMap::new(),
        };
        for row in rows {
            let (id, ids, lens, wkt) = row.map_err(e)?;
            let mut acc = 0.0;
            for (i, (eid, l)) in ids.iter().zip(&lens).enumerate() {
                c.of.insert(*eid, (id, i));
                c.offset.insert(*eid, acc);
                acc += l;
            }
            c.chain
                .insert(id, (ids, crate::sumo::shape_of(wkt.as_deref()), acc));
        }
        Ok(c)
    }

    /// The SUMO edge of the edge `id` at the end where a connection leaves it (`from`) or enters it (`!from`): its chain
    /// where that end is an end of the chain, None where the end is inside a chain.
    fn end(&self, id: &str, from: bool) -> Option<String> {
        let Ok(n) = id.parse::<i64>() else {
            return Some(id.to_string());
        };
        match self.of.get(&n) {
            None => Some(id.to_string()),
            Some((c, i)) => {
                let last = self.chain[c].0.len() - 1;
                ((from && *i == last) || (!from && *i == 0)).then(|| c.to_string())
            }
        }
    }

    /// The edges of the SUMO edge `id` read back from netconvert's network: (first, last).
    pub fn ends_of(&self, id: &str) -> (String, String) {
        match id.parse::<i64>().ok().and_then(|n| self.chain.get(&n)) {
            Some((ids, _, _)) => (ids[0].to_string(), ids[ids.len() - 1].to_string()),
            None => (id.to_string(), id.to_string()),
        }
    }

    pub fn len(&self) -> usize {
        self.chain.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }
}

fn list(v: duckdb::types::Value) -> Vec<i64> {
    match v {
        duckdb::types::Value::List(l) => l
            .into_iter()
            .filter_map(|v| {
                if let duckdb::types::Value::BigInt(i) = v {
                    Some(i)
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn attr<'a>(line: &'a str, k: &str) -> Option<&'a str> {
    let p = format!(" {k}=\"");
    let a = line.find(&p)? + p.len();
    Some(&line[a..a + line[a..].find('"')?])
}

fn set_attr(line: &str, k: &str, v: &str) -> String {
    let p = format!(" {k}=\"");
    match line.find(&p) {
        Some(a) => {
            let b = a + p.len() + line[a + p.len()..].find('"').unwrap_or(0);
            format!("{}{v}{}", &line[..a + p.len()], &line[b..])
        }
        None => line.to_string(),
    }
}

fn read(p: &Path) -> Result<String> {
    std::fs::read_to_string(p).map_err(|x| format!("{}: {x}", p.display()))
}

fn write(p: &Path, s: &str) -> Result<()> {
    std::fs::write(p, s).map_err(|x| format!("{}: {x}", p.display()))
}

/// Join the chains in the files that the export wrote: the edges (`edg`), the connections (`con`), the bus stops (`add`)
/// and the nodes (`nod`: a node that no edge uses now is gone). Gives the numbers of nodes and edges.
pub fn merge_files(
    c: &Chains,
    nod: &Path,
    edg: &Path,
    con: Option<&Path>,
    add: Option<&Path>,
) -> Result<(usize, usize)> {
    // the edges: the block of each edge (its <edge> line, its <lane> lines and </edge>)
    let text = read(edg)?;
    let mut out = String::new();
    let mut blocks: HashMap<i64, Vec<String>> = HashMap::new();
    let mut ends: HashMap<i64, (String, String)> = HashMap::new();
    let mut lines = text.lines().peekable();
    let mut order: Vec<Option<i64>> = Vec::new(); // None: a line that is not an edge
    let mut other: Vec<String> = Vec::new();
    while let Some(l) = lines.next() {
        if !l.trim_start().starts_with("<edge ") {
            other.push(l.to_string());
            order.push(None);
            continue;
        }
        let mut b = vec![l.to_string()];
        if !l.trim_end().ends_with("/>") {
            for m in lines.by_ref() {
                b.push(m.to_string());
                if m.trim_start().starts_with("</edge>") {
                    break;
                }
            }
        }
        let id: i64 = attr(l, "id")
            .and_then(|x| x.parse().ok())
            .unwrap_or_default();
        ends.insert(
            id,
            (
                attr(l, "from").unwrap_or_default().to_string(),
                attr(l, "to").unwrap_or_default().to_string(),
            ),
        );
        blocks.insert(id, b);
        order.push(Some(id));
    }
    let mut used: HashSet<String> = HashSet::new();
    let (mut n_edges, mut k) = (0, 0);
    let mut done: HashSet<i64> = HashSet::new();
    for o in &order {
        let Some(id) = o else {
            out.push_str(&other[k]);
            out.push('\n');
            k += 1;
            continue;
        };
        let b = match c.of.get(id) {
            None => blocks[id].clone(),
            Some((cid, _)) => {
                if !done.insert(*cid) {
                    continue;
                }
                let (ids, shape, len) = &c.chain[cid];
                let (first, last) = (ids[0], ids[ids.len() - 1]);
                // the last edge's lanes and values (the same on each edge of the chain), the ends and the line of the chain
                let Some(lb) = blocks.get(&last) else {
                    continue;
                };
                let mut head = set_attr(&lb[0], "id", &cid.to_string());
                head = set_attr(
                    &head,
                    "from",
                    &ends.get(&first).map(|x| x.0.clone()).unwrap_or_default(),
                );
                head = set_attr(&head, "length", &format!("{len:.2}"));
                if let Some(s) = shape {
                    head = set_attr(&head, "shape", s);
                }
                let param = format!(
                    "    <param key=\"edge_ids\" value=\"{}\"/>",
                    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(" ")
                );
                let mut b = Vec::new();
                if lb.len() == 1 {
                    b.push(
                        head.trim_end()
                            .trim_end_matches("/>")
                            .trim_end()
                            .to_string()
                            + ">",
                    );
                    b.push(param);
                    b.push("  </edge>".into());
                } else {
                    b.push(head);
                    b.extend(lb[1..lb.len() - 1].iter().cloned());
                    b.push(param);
                    b.push(lb[lb.len() - 1].clone());
                }
                b
            }
        };
        used.insert(attr(&b[0], "from").unwrap_or_default().to_string());
        used.insert(attr(&b[0], "to").unwrap_or_default().to_string());
        n_edges += 1;
        for l in b {
            out.push_str(&l);
            out.push('\n');
        }
    }
    write(edg, &out)?;
    // the nodes that an edge still uses
    let text = read(nod)?;
    let mut n_nodes = 0;
    let out: Vec<&str> = text
        .lines()
        .filter(|l| {
            if !l.trim_start().starts_with("<node ") {
                return true;
            }
            let keep = attr(l, "id").is_some_and(|i| used.contains(i));
            n_nodes += usize::from(keep);
            keep
        })
        .collect();
    write(nod, &(out.join("\n") + "\n"))?;
    // the connections: from the last edge of a chain, to the first one; a turn inside a chain is gone
    if let Some(p) = con.filter(|p| p.exists()) {
        let text = read(p)?;
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = String::new();
        for l in text.lines() {
            if l.trim_start().starts_with("<connection ") {
                let Some(f) = attr(l, "from").and_then(|f| c.end(f, true)) else {
                    continue;
                };
                let mut m = set_attr(l, "from", &f);
                if let Some(t) = attr(l, "to") {
                    let Some(t) = c.end(t, false) else { continue };
                    m = set_attr(&m, "to", &t);
                }
                if !seen.insert(m.clone()) {
                    continue;
                }
                out.push_str(&m);
            } else {
                out.push_str(l);
            }
            out.push('\n');
        }
        write(p, &out)?;
    }
    // the bus stops: on the lane of the chain, from the length of the edges before
    if let Some(p) = add.filter(|p| p.exists()) {
        let text = read(p)?;
        let mut out = String::new();
        for l in text.lines() {
            let mut m = l.to_string();
            if let Some((e, lane)) = attr(l, "lane").and_then(|x| x.rsplit_once('_')) {
                if let Some((cid, _)) = e.parse::<i64>().ok().and_then(|n| c.of.get(&n)) {
                    let off = c.offset[&e.parse::<i64>().unwrap_or_default()];
                    m = set_attr(&m, "lane", &format!("{cid}_{lane}"));
                    for k in ["startPos", "endPos"] {
                        if let Some(v) = attr(l, k).and_then(|v| v.parse::<f64>().ok()) {
                            m = set_attr(&m, k, &format!("{:.2}", v + off));
                        }
                    }
                }
            }
            out.push_str(&m);
            out.push('\n');
        }
        write(p, &out)?;
    }
    Ok((n_nodes, n_edges))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes() {
        let l = r#"  <edge id="1" from="2" to="3" length="4.00"/>"#;
        assert_eq!(attr(l, "to"), Some("3"));
        assert_eq!(
            set_attr(l, "from", "9"),
            r#"  <edge id="1" from="9" to="3" length="4.00"/>"#
        );
    }

    #[test]
    fn connection_ends() {
        let mut c = Chains {
            of: HashMap::new(),
            chain: HashMap::new(),
            offset: HashMap::new(),
        };
        c.chain.insert(100, (vec![1, 2, 3], None, 30.0));
        for (i, e) in [1, 2, 3].iter().enumerate() {
            c.of.insert(*e, (100, i));
        }
        assert_eq!(c.end("3", true).as_deref(), Some("100"));
        assert_eq!(c.end("2", true), None);
        assert_eq!(c.end("1", false).as_deref(), Some("100"));
        assert_eq!(c.end("3", false), None);
        assert_eq!(c.end("7", false).as_deref(), Some("7"));
        assert_eq!(c.ends_of("100"), ("1".into(), "3".into()));
    }
}
