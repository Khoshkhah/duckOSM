//! `export-graph` (duckOSM `routing.write_graph` with networkx's GraphML writer, the lxml one): the geographic node graph
//! (`to_networkx_nodes`, a MultiDiGraph keyed by `edge_id`) or the edge-based routing graph (`to_networkx`), as the same file.
//! Also the same graph as a Python pickle (`gpickle`, [`Doc::pickle`]): `pickle.load` gives the networkx graph of `write_graph`.

use crate::{err, Result};
use duckdb::types::Value;
use duckdb::Connection;
use std::collections::HashMap;

/// A value as Python has it after DuckDB: its GraphML type and its `str()`. `None`, `List` and `Wkb` (a shapely geometry) are
/// only in a pickle: GraphML drops None and writes a list as its `str()`.
#[derive(Clone, Debug)]
pub enum Py {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(String),
    None,
    List(Vec<Py>),
    Wkb(Vec<u8>),
}

impl Py {
    fn xml_type(&self) -> &'static str {
        match self {
            Py::Int(_) => "long",
            Py::Float(_) => "double",
            Py::Bool(_) => "boolean",
            _ => "string",
        }
    }
    fn text(&self) -> String {
        match self {
            Py::Int(i) => i.to_string(),
            Py::Float(f) => py_float(*f),
            Py::Bool(b) => if *b { "True".into() } else { "False".into() },
            Py::Str(s) => s.clone(),
            Py::None => "None".into(),
            Py::List(l) => format!("[{}]", l.iter().map(Py::repr).collect::<Vec<_>>().join(", ")),
            Py::Wkb(_) => "<geometry>".into(),
        }
    }
    /// Python's `repr` (a value in a list, `str(list)`).
    fn repr(&self) -> String {
        match self {
            Py::Str(s) => duckosm_build::place::py_repr(s),
            other => other.text(),
        }
    }
}

fn py_float(f: f64) -> String {
    if f.is_nan() {
        "nan".into()
    } else if f.is_infinite() {
        if f > 0.0 { "inf".into() } else { "-inf".into() }
    } else {
        duckosm_build::place::py_float(f)
    }
}

/// A DuckDB value as Python gets it.
pub fn py(v: Value) -> Py {
    match v {
        Value::Null => Py::None,
        Value::Boolean(b) => Py::Bool(b),
        Value::TinyInt(i) => Py::Int(i.into()),
        Value::SmallInt(i) => Py::Int(i.into()),
        Value::Int(i) => Py::Int(i.into()),
        Value::BigInt(i) => Py::Int(i.into()),
        Value::HugeInt(i) => Py::Int(i),
        Value::UTinyInt(i) => Py::Int(i.into()),
        Value::USmallInt(i) => Py::Int(i.into()),
        Value::UInt(i) => Py::Int(i.into()),
        Value::UBigInt(i) => Py::Int(i.into()),
        Value::Float(f) => Py::Float(f as f64),
        Value::Double(f) => Py::Float(f),
        Value::Text(s) => Py::Str(s),
        Value::List(l) | Value::Array(l) => Py::List(l.into_iter().map(py).collect()),
        Value::Blob(b) => Py::Wkb(b),
        other => Py::Str(format!("{other:?}")),
    }
}

/// lxml's escape of the text of an element.
fn esc_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// lxml's escape of the value of an attribute.
fn esc_attr(s: &str) -> String {
    esc_text(s).replace('"', "&quot;").replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;")
}

/// The data of a node or an edge, in networkx's order; a None value stays (GraphML drops it).
type Data = Vec<(String, Py)>;

/// The data that GraphML writes: no None.
fn graphml_data(d: &Data) -> impl Iterator<Item = &(String, Py)> {
    d.iter().filter(|x| !matches!(x.1, Py::None))
}

/// A graph to write: its data, its nodes (id, data) and its edges (source, target, key, data), in networkx's order.
#[derive(Default)]
pub struct Doc {
    pub graph: Data,
    pub nodes: Vec<(i64, Data)>,
    pub edges: Vec<(i64, i64, Option<i64>, Data)>,
}

impl Doc {
    /// networkx `write_graphml` (lxml): the keys in reverse order of their making, then the graph data, the nodes, the edges.
    pub fn write(&self) -> String {
        let mut keys: Vec<(String, &'static str, String)> = Vec::new(); // (scope, type, name), in the order they are made
        let mut id: HashMap<(String, String), usize> = HashMap::new();
        let mut add = |scope: &str, name: &str, t: &'static str, keys: &mut Vec<(String, &'static str, String)>| {
            id.entry((scope.into(), name.into())).or_insert_with(|| {
                keys.push((scope.into(), t, name.into()));
                keys.len() - 1
            });
        };
        for (k, v) in graphml_data(&self.graph) {
            add("graph", k, v.xml_type(), &mut keys);
        }
        for (_, d) in &self.nodes {
            for (k, v) in graphml_data(d) {
                add("node", k, v.xml_type(), &mut keys);
            }
        }
        for (_, _, _, d) in &self.edges {
            for (k, v) in graphml_data(d) {
                add("edge", k, v.xml_type(), &mut keys);
            }
        }
        let key_of: HashMap<(&str, &str), usize> = keys.iter().enumerate().map(|(i, (s, _, n))| ((s.as_str(), n.as_str()), i)).collect();
        let mut out = String::from(
            "<?xml version='1.0' encoding='utf-8'?>\n<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd\">",
        );
        for (i, (scope, t, name)) in keys.iter().enumerate().rev() {
            out += &format!("<key id=\"d{i}\" for=\"{scope}\" attr.name=\"{}\" attr.type=\"{t}\"/>\n", esc_attr(name));
        }
        out += "<graph edgedefault=\"directed\">";
        for (k, v) in graphml_data(&self.graph) {
            out += &format!("<data key=\"d{}\">{}</data>\n", key_of[&("graph", k.as_str())], esc_text(&v.text()));
        }
        let element = |out: &mut String, open: String, tag: &str, scope: &str, d: &Data| {
            if graphml_data(d).next().is_none() {
                *out += &format!("{}/>\n", open);
            } else {
                *out += &format!("{open}>\n");
                for (k, v) in graphml_data(d) {
                    *out += &format!("  <data key=\"d{}\">{}</data>\n", key_of[&(scope, k.as_str())], esc_text(&v.text()));
                }
                *out += &format!("</{tag}>\n");
            }
        };
        for (n, d) in &self.nodes {
            element(&mut out, format!("<node id=\"{n}\""), "node", "node", d);
        }
        for (u, v, k, d) in &self.edges {
            let open = match k {
                Some(k) => format!("<edge source=\"{u}\" target=\"{v}\" id=\"{k}\""),
                None => format!("<edge source=\"{u}\" target=\"{v}\""),
            };
            element(&mut out, open, "edge", "edge", d);
        }
        out += "</graph></graphml>";
        out
    }

    /// `pickle.dump(G)` of `write_graph` (`gpickle`), protocol 3: a networkx `MultiDiGraph` (`multi`: the edges have keys) or
    /// `DiGraph`, with the attributes of networkx 3 (`graph`, `_node`, `_adj`, `_pred`, `_succ` = `_adj`, `__networkx_cache__`).
    /// `_pred[v][u]` is the same dict as `_adj[u][v]` (the memo of the pickle), as in networkx.
    pub fn pickle(&self, multi: bool) -> Vec<u8> {
        let mut w = Pickler { out: vec![0x80, 3], memo: 0 };
        w.out.push(b'c');
        w.out.extend_from_slice(if multi { b"networkx.classes.multidigraph\nMultiDiGraph\n" } else { b"networkx.classes.digraph\nDiGraph\n" });
        w.out.extend_from_slice(b")\x81}("); // cls.__new__(cls), then the dict of its attributes
        w.text("graph");
        w.dict(&self.graph);
        w.text("_node");
        w.out.extend_from_slice(b"}(");
        for (n, d) in &self.nodes {
            w.int(*n as i128);
            w.dict(d);
        }
        w.out.push(b'u');
        // the successors of each node, in the order of the edges; the memo id of each (u, v) dict
        let at: HashMap<i64, usize> = self.nodes.iter().enumerate().map(|(i, n)| (n.0, i)).collect();
        let mut succ: Vec<Vec<(i64, Vec<(Option<i64>, &Data)>)>> = vec![Vec::new(); self.nodes.len()];
        let mut pred: Vec<Vec<(i64, u32)>> = vec![Vec::new(); self.nodes.len()];
        for (u, v, k, d) in &self.edges {
            let list = &mut succ[at[u]];
            match list.iter_mut().find(|x| x.0 == *v) {
                Some(x) => x.1.push((*k, d)),
                None => list.push((*v, vec![(*k, d)])),
            }
        }
        w.text("_adj");
        w.out.push(b'}');
        let adj = w.put();
        w.out.push(b'(');
        for (i, list) in succ.iter().enumerate() {
            let u = self.nodes[i].0;
            w.int(u as i128);
            w.out.extend_from_slice(b"}(");
            for (v, es) in list {
                w.int(*v as i128);
                w.out.push(b'}');
                pred[at[v]].push((u, w.put()));
                if multi {
                    w.out.push(b'(');
                    for (k, d) in es {
                        w.int(k.unwrap_or_default() as i128);
                        w.dict(d);
                    }
                    w.out.push(b'u');
                } else {
                    w.items(es[0].1);
                }
            }
            w.out.push(b'u');
        }
        w.out.push(b'u');
        w.text("_pred");
        w.out.extend_from_slice(b"}(");
        for (i, list) in pred.iter().enumerate() {
            w.int(self.nodes[i].0 as i128);
            w.out.extend_from_slice(b"}(");
            for (u, id) in list {
                w.int(*u as i128);
                w.get(*id);
            }
            w.out.push(b'u');
        }
        w.out.push(b'u');
        w.text("_succ");
        w.get(adj);
        w.text("__networkx_cache__");
        w.out.extend_from_slice(b"}ub."); // the attributes, then BUILD and STOP
        w.out
    }
}

/// The opcodes of Python's pickle (protocol 3) that a graph needs.
struct Pickler {
    out: Vec<u8>,
    memo: u32,
}

impl Pickler {
    fn text(&mut self, s: &str) {
        self.out.push(b'X');
        self.out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        self.out.extend_from_slice(s.as_bytes());
    }
    fn int(&mut self, i: i128) {
        match i32::try_from(i) {
            Ok(i) => {
                self.out.push(b'J');
                self.out.extend_from_slice(&i.to_le_bytes());
            }
            Err(_) => {
                // LONG1: the bytes of the number, little-endian two's complement
                let b = i.to_le_bytes();
                let n = if i64::try_from(i).is_ok() { 8 } else { 16 };
                self.out.extend_from_slice(&[0x8a, n as u8]);
                self.out.extend_from_slice(&b[..n]);
            }
        }
    }
    fn value(&mut self, v: &Py) {
        match v {
            Py::Int(i) => self.int(*i),
            Py::Float(f) => {
                self.out.push(b'G');
                self.out.extend_from_slice(&f.to_be_bytes());
            }
            Py::Bool(b) => self.out.push(if *b { 0x88 } else { 0x89 }),
            Py::Str(s) => self.text(s),
            Py::None => self.out.push(b'N'),
            Py::List(l) => {
                self.out.push(b']');
                if !l.is_empty() {
                    self.out.push(b'(');
                    for x in l {
                        self.value(x);
                    }
                    self.out.push(b'e');
                }
            }
            Py::Wkb(b) => {
                // shapely's own pickle of a geometry: shapely.from_wkb(<WKB>)
                self.out.extend_from_slice(b"cshapely\nfrom_wkb\nB");
                self.out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                self.out.extend_from_slice(b);
                self.out.extend_from_slice(&[0x85, b'R']);
            }
        }
    }
    /// A new dict with `d`.
    fn dict(&mut self, d: &Data) {
        self.out.push(b'}');
        self.items(d);
    }
    /// The items of `d`, into the dict on the stack.
    fn items(&mut self, d: &Data) {
        if !d.is_empty() {
            self.out.push(b'(');
            for (k, v) in d {
                self.text(k);
                self.value(v);
            }
            self.out.push(b'u');
        }
    }
    /// Put the object on the stack in the memo: its id.
    fn put(&mut self) -> u32 {
        let id = self.memo;
        self.memo += 1;
        match u8::try_from(id) {
            Ok(b) => self.out.extend_from_slice(&[b'q', b]),
            Err(_) => {
                self.out.push(b'r');
                self.out.extend_from_slice(&id.to_le_bytes());
            }
        }
        id
    }
    fn get(&mut self, id: u32) {
        match u8::try_from(id) {
            Ok(b) => self.out.extend_from_slice(&[b'h', b]),
            Err(_) => {
                self.out.push(b'j');
                self.out.extend_from_slice(&id.to_le_bytes());
            }
        }
    }
}

fn columns(con: &Connection, table: &str) -> Result<Vec<String>> {
    let mut st = con.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")).map_err(err)?;
    let v = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
    Ok(v)
}

/// Rows of `sql` as (column name, value) lists.
fn rows(con: &Connection, sql: &str) -> Result<Vec<Data>> {
    let mut st = con.prepare(sql).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    let names: Vec<String> = rows.as_ref().map(|s| s.column_names().into_iter().map(String::from).collect()).unwrap_or_default();
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(err)? {
        let mut v = Vec::new();
        for (i, n) in names.iter().enumerate() {
            v.push((n.clone(), py(r.get::<_, Value>(i).map_err(err)?)));
        }
        out.push(v);
    }
    Ok(out)
}

fn int(v: &Py) -> Option<i64> {
    match v {
        Py::Int(i) => Some(*i as i64),
        _ => None,
    }
}

/// duckOSM `to_networkx_nodes`: junctions with `x`, `y` and the other node columns; one edge for each row of `<mode>.edges`,
/// keyed by `edge_id`, with all its columns. `geometry`: `wkt` (a text), `shapely` (a shapely object, only in a pickle) or `none`.
pub fn node_graph(con: &Connection, mode: &str, geometry: &str) -> Result<Doc> {
    let mut doc = Doc { graph: vec![("crs".into(), Py::Str("EPSG:4326".into())), ("mode".into(), Py::Str(mode.into()))], ..Default::default() };
    let ncols: Vec<String> = columns(con, &format!("{mode}.nodes"))?.into_iter().filter(|c| c != "node_id" && c != "geom").collect();
    let mut sel = vec!["node_id".to_string(), "ST_X(geom) AS x".into(), "ST_Y(geom) AS y".into()];
    sel.extend(ncols.iter().map(|c| format!("\"{c}\"")));
    let mut index: HashMap<i64, usize> = HashMap::new();
    for r in rows(con, &format!("SELECT {} FROM {mode}.nodes WHERE geom IS NOT NULL", sel.join(", ")))? {
        let id = int(&r[0].1).ok_or("a node with no node_id")?;
        let data: Data = r.into_iter().skip(1).collect();
        match index.get(&id) {
            Some(&i) => {
                // networkx add_node again: the data is updated
                for (k, v) in data {
                    match doc.nodes[i].1.iter_mut().find(|x| x.0 == k) {
                        Some(x) => x.1 = v,
                        None => doc.nodes[i].1.push((k, v)),
                    }
                }
            }
            None => {
                index.insert(id, doc.nodes.len());
                doc.nodes.push((id, data));
            }
        }
    }
    let ecols = columns(con, &format!("{mode}.edges"))?;
    let esel: Vec<String> = ecols
        .iter()
        .filter(|c| geometry != "none" || c.as_str() != "geometry")
        .map(|c| match (c.as_str(), geometry) {
            ("geometry", "shapely") => "ST_AsWKB(geometry)::BLOB AS geometry".to_string(),
            ("geometry", _) => "ST_AsText(geometry) AS geometry".to_string(),
            _ => format!("\"{c}\""),
        })
        .collect();
    // the edges of each (source, target) pair, the pairs in the order of the first edge from source to target
    let mut succ: Vec<Vec<(i64, Vec<(i64, Data)>)>> = vec![Vec::new(); doc.nodes.len()];
    for r in rows(con, &format!("SELECT {} FROM {mode}.edges", esel.join(", ")))? {
        let get = |n: &str| r.iter().find(|x| x.0 == n).and_then(|x| int(&x.1));
        let (eid, s, t) = (get("edge_id").ok_or("an edge with no edge_id")?, get("source").ok_or("no source")?, get("target").ok_or("no target")?);
        for n in [s, t] {
            if !index.contains_key(&n) {
                index.insert(n, doc.nodes.len());
                doc.nodes.push((n, Vec::new()));
                succ.push(Vec::new());
            }
        }
        let data: Data = r.into_iter().filter(|(k, _)| k != "source" && k != "target").collect();
        let list = &mut succ[index[&s]];
        match list.iter_mut().find(|x| x.0 == t) {
            Some(x) => match x.1.iter_mut().find(|e| e.0 == eid) {
                Some(e) => e.1 = data,
                None => x.1.push((eid, data)),
            },
            None => list.push((t, vec![(eid, data)])),
        }
    }
    for (i, list) in succ.into_iter().enumerate() {
        let u = doc.nodes[i].0;
        for (v, es) in list {
            for (k, d) in es {
                doc.edges.push((u, v, Some(k), d));
            }
        }
    }
    Ok(doc)
}

/// duckOSM `to_networkx` (with the node data): each node an `edge_id` with `name`, `highway`, `length_m`, `maxspeed_kmh`,
/// `cost_s`, `geometry` (WKT) and with `uses` its `access`; each arc a legal turn with its `weight`.
pub fn edge_graph(con: &Connection, mode: &str, weight: &str, uses: &[String]) -> Result<Doc> {
    let g = crate::route::routing_graph(con, mode, weight, uses)?;
    let roads = crate::route::roads_relation(mode, uses);
    let with_access = !uses.is_empty();
    // the values stay None in their place until the file is written (networkx: dict.update, then _graphml_safe drops None)
    let mut data: Vec<Data> = vec![Vec::new(); g.n_nodes()];
    let sql = format!(
        "SELECT edge_id, name, highway, length_m, maxspeed_kmh, cost_s, ST_AsText(geometry) AS geometry{} FROM {roads}",
        if with_access { ", access" } else { "" }
    );
    for r in rows(con, &sql)? {
        let Some(i) = int(&r[0].1).and_then(|e| g.index_of(e)) else { continue };
        let d = &mut data[i];
        let mut set = |k: &str, v: &Py| match d.iter_mut().find(|x| x.0 == k) {
            Some(x) => x.1 = v.clone(),
            None => d.push((k.to_string(), v.clone())),
        };
        for (k, v) in r.iter().skip(1).take(6) {
            set(k, v);
        }
        if with_access {
            if let Py::Str(a) = &r[7].1 {
                if !a.is_empty() {
                    set("access", &Py::Str(a.clone()));
                }
            }
        }
    }
    let mut doc = Doc::default();
    doc.nodes = g.ids.iter().zip(data).map(|(&id, d)| (id, d)).collect();
    doc.edges = g.arcs().map(|(u, v, w)| (u, v, None, vec![("weight".to_string(), if w.is_nan() { Py::None } else { Py::Float(w) })])).collect();
    Ok(doc)
}
