//! `export-graph` (duckOSM `routing.write_graph` with networkx's GraphML writer, the lxml one): the geographic node graph
//! (`to_networkx_nodes`, a MultiDiGraph keyed by `edge_id`) or the edge-based routing graph (`to_networkx`), as the same file.

use crate::{err, Result};
use duckdb::types::Value;
use duckdb::Connection;
use duckosm_export::stream::each_chunk;
use duckdb::arrow::array::{Array, ArrayRef, AsArray};
use duckdb::arrow::datatypes::{DataType, Float64Type, Int64Type};
use std::collections::HashMap;
use std::io::Write;

/// A value as Python has it after DuckDB: its GraphML type and its `str()`.
#[derive(Clone, Debug)]
pub enum Py {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl Py {
    fn xml_type(&self) -> &'static str {
        match self {
            Py::Int(_) => "long",
            Py::Float(_) => "double",
            Py::Bool(_) => "boolean",
            Py::Str(_) => "string",
        }
    }
    fn text(&self) -> String {
        match self {
            Py::Int(i) => i.to_string(),
            Py::Float(f) => py_float(*f),
            Py::Bool(b) => if *b { "True".into() } else { "False".into() },
            Py::Str(s) => s.clone(),
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

/// Python's `repr` of a value in a list (`str(list)`).
fn py_repr(v: &Value) -> String {
    match v {
        Value::Text(s) => duckosm_build::place::py_repr(s),
        Value::Null => "None".into(),
        other => match py(other.clone()) {
            Some(p) => p.text(),
            None => "None".into(),
        },
    }
}

/// A DuckDB value as Python gets it, None for NULL (GraphML drops it). A list is its `str()`.
pub fn py(v: Value) -> Option<Py> {
    Some(match v {
        Value::Null => return None,
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
        Value::List(l) | Value::Array(l) => Py::Str(format!("[{}]", l.iter().map(py_repr).collect::<Vec<_>>().join(", "))),
        other => Py::Str(format!("{other:?}")),
    })
}

/// lxml's escape of the text of an element.
fn esc_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// lxml's escape of the value of an attribute.
fn esc_attr(s: &str) -> String {
    esc_text(s).replace('"', "&quot;").replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;")
}

type Data = Vec<(String, Py)>;

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
        for (k, v) in &self.graph {
            add("graph", k, v.xml_type(), &mut keys);
        }
        for (_, d) in &self.nodes {
            for (k, v) in d {
                add("node", k, v.xml_type(), &mut keys);
            }
        }
        for (_, _, _, d) in &self.edges {
            for (k, v) in d {
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
        for (k, v) in &self.graph {
            out += &format!("<data key=\"d{}\">{}</data>\n", key_of[&("graph", k.as_str())], esc_text(&v.text()));
        }
        let element = |out: &mut String, open: String, tag: &str, scope: &str, d: &Data| {
            if d.is_empty() {
                *out += &format!("{}/>\n", open);
            } else {
                *out += &format!("{open}>\n");
                for (k, v) in d {
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
}

fn columns(con: &Connection, table: &str) -> Result<Vec<String>> {
    let mut st = con.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")).map_err(err)?;
    let v = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
    Ok(v)
}

/// Rows of `sql` as (column name, value) lists, NULLs left out.
fn rows(con: &Connection, sql: &str) -> Result<Vec<Vec<(String, Option<Py>)>>> {
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

fn int(v: &Option<Py>) -> Option<i64> {
    match v {
        Some(Py::Int(i)) => Some(*i as i64),
        _ => None,
    }
}

/// The GraphML type of a column as networkx gets it from DuckDB (a list is text).
fn xml_type(t: &DataType) -> &'static str {
    match t {
        t if t.is_integer() => "long",
        t if t.is_floating() => "double",
        DataType::Boolean => "boolean",
        _ => "string",
    }
}

/// The value of row `r` of a column of a [`Chunk`], as Python gets it after DuckDB (`py`); None for NULL.
fn py_at(col: &ArrayRef, r: usize) -> Option<Py> {
    if col.is_null(r) {
        return None;
    }
    Some(match col.data_type() {
        DataType::Boolean => Py::Bool(col.as_boolean().value(r)),
        DataType::Int64 => Py::Int(col.as_primitive::<Int64Type>().value(r).into()),
        DataType::Float64 => Py::Float(col.as_primitive::<Float64Type>().value(r)),
        DataType::Utf8 => Py::Str(col.as_string::<i32>().value(r).to_string()),
        DataType::List(_) => {
            let items = col.as_list::<i32>().value(r);
            let items = duckosm_export::stream::cast_col(&items).ok()?;
            let repr = |i: usize| match items.data_type() {
                _ if items.is_null(i) => "None".to_string(),
                DataType::Utf8 => duckosm_build::place::py_repr(items.as_string::<i32>().value(i)),
                _ => py_at(&items, i).map(|p| p.text()).unwrap_or_else(|| "None".into()),
            };
            Py::Str(format!("[{}]", (0..items.len()).map(repr).collect::<Vec<_>>().join(", ")))
        }
        other => Py::Str(format!("{other:?}")),
    })
}

/// The GraphML file as networkx's lxml writer makes it, written in parts: [`GraphWriter::start`] with all the keys, then
/// each node and each edge as it comes.
struct GraphWriter<'a, W: Write> {
    w: &'a mut W,
}

impl<W: Write> GraphWriter<'_, W> {
    fn put(&mut self, s: &str) -> Result<()> {
        self.w.write_all(s.as_bytes()).map_err(|e| e.to_string())
    }

    /// The keys (scope, type, name) in the order they are made, then the graph data. The keys go in reverse order.
    fn start(&mut self, keys: &[(String, &'static str, String)], graph: &Data, key_of: &HashMap<(String, String), usize>) -> Result<()> {
        self.put(
            "<?xml version='1.0' encoding='utf-8'?>\n<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd\">",
        )?;
        for (i, (scope, t, name)) in keys.iter().enumerate().rev() {
            self.put(&format!("<key id=\"d{i}\" for=\"{scope}\" attr.name=\"{}\" attr.type=\"{t}\"/>\n", esc_attr(name)))?;
        }
        self.put("<graph edgedefault=\"directed\">")?;
        for (k, v) in graph {
            self.put(&format!("<data key=\"d{}\">{}</data>\n", key_of[&("graph".to_string(), k.clone())], esc_text(&v.text())))?;
        }
        Ok(())
    }

    /// One node or edge: `open` is its start tag without the end, `d` its data as (key id, value).
    fn element(&mut self, open: &str, tag: &str, d: &[(usize, Py)]) -> Result<()> {
        if d.is_empty() {
            return self.put(&format!("{open}/>\n"));
        }
        let mut s = format!("{open}>\n");
        for (k, v) in d {
            s += &format!("  <data key=\"d{k}\">{}</data>\n", esc_text(&v.text()));
        }
        s += &format!("</{tag}>\n");
        self.put(&s)
    }

    fn end(&mut self) -> Result<()> {
        self.put("</graph></graphml>")
    }
}

/// The keys of `scope` in the order networkx makes them: a column gets its key at its first value that is not NULL, in
/// the order of the elements (`order`), then of the columns. `cols`: (name, GraphML type). Columns with no value get no key.
fn add_keys(
    con: &Connection,
    from: &str,
    order: &str,
    scope: &str,
    cols: &[(String, &'static str)],
    keys: &mut Vec<(String, &'static str, String)>,
    key_of: &mut HashMap<(String, String), usize>,
) -> Result<()> {
    if cols.is_empty() {
        return Ok(());
    }
    // the place of the first value (a list of numbers), sorted, then the column's index
    let sel: Vec<String> = cols.iter().map(|(c, _)| format!("min({order}) FILTER (WHERE \"{c}\" IS NOT NULL)")).collect();
    let mut firsts: Vec<(Vec<i64>, usize)> = con
        .query_row(&format!("SELECT {} FROM {from}", sel.join(", ")), [], |r| {
            let mut v = Vec::new();
            for i in 0..cols.len() {
                if let Value::List(l) = r.get::<_, Value>(i)? {
                    v.push((l.iter().map(|x| if let Value::BigInt(n) = x { *n } else { 0 }).collect(), i));
                }
            }
            Ok(v)
        })
        .map_err(err)?;
    firsts.sort();
    for (_, i) in firsts {
        let (name, t) = &cols[i];
        key_of.insert((scope.into(), name.clone()), keys.len());
        keys.push((scope.into(), t, name.clone()));
    }
    Ok(())
}

/// The column names and GraphML types of `sql` (the query runs with no rows).
fn column_types(con: &Connection, sql: &str) -> Result<Vec<(String, &'static str)>> {
    let mut st = con.prepare(&format!("SELECT * FROM ({sql}) LIMIT 0")).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    let st = rows.as_ref().ok_or("no statement")?;
    let names: Vec<String> = st.column_names().into_iter().map(String::from).collect();
    let types: Vec<&'static str> = (0..names.len()).map(|i| xml_type(&st.column_type(i))).collect();
    while rows.next().map_err(err)?.is_some() {}
    Ok(names.into_iter().zip(types).collect())
}

/// duckOSM `to_networkx_nodes` written as GraphML to `w`: junctions with `x`, `y` and the other node columns; one edge for
/// each row of `<mode>.edges`, keyed by `edge_id`, with all its columns (the geometry as WKT, or none). Gives the numbers
/// of nodes and edges.
///
/// The file is written while DuckDB reads the tables, in networkx's order: the nodes of `<mode>.nodes` (in the order of
/// the table), then the nodes that only an edge has (in the order of the edges); the edges of each node in that order,
/// grouped by target in the order of the first edge to it. `node_id` must be unique in `<mode>.nodes` (networkx would
/// join the data of two rows).
pub fn write_node_graph(con: &Connection, mode: &str, geometry: bool, w: &mut impl Write) -> Result<(usize, usize)> {
    let ncols: Vec<String> = columns(con, &format!("{mode}.nodes"))?.into_iter().filter(|c| c != "node_id" && c != "geom").collect();
    let mut nsel = vec!["ST_X(geom) AS x".to_string(), "ST_Y(geom) AS y".into()];
    nsel.extend(ncols.iter().map(|c| format!("\"{c}\"")));
    let ecols: Vec<String> = columns(con, &format!("{mode}.edges"))?.into_iter().filter(|c| geometry || c != "geometry").collect();
    let esel: Vec<String> = ecols
        .iter()
        .filter(|c| *c != "source" && *c != "target")
        .map(|c| if c == "geometry" { "ST_AsText(geometry) AS geometry".to_string() } else { format!("\"{c}\"") })
        .collect();
    let (dups, bad): (i64, i64) = con
        .query_row(
            &format!(
                "SELECT (SELECT count(*) - count(DISTINCT node_id) FROM {mode}.nodes WHERE geom IS NOT NULL), \
                 (SELECT count(*) FROM {mode}.edges WHERE edge_id IS NULL OR source IS NULL OR target IS NULL)"
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    if dups > 0 {
        return Err(format!("{mode}.nodes has {dups} rows with a node_id of another row"));
    }
    if bad > 0 {
        return Err(format!("{mode}.edges has {bad} rows with no edge_id, source or target"));
    }
    // the place of each node: its row in the nodes table, else 2^40 + its first place in the edges (2 * row, + 1 for a target)
    let with = format!(
        "WITH n AS (SELECT rowid AS np, node_id, {nsel} FROM {mode}.nodes WHERE geom IS NOT NULL), \
         e AS (SELECT rowid AS p, * FROM {mode}.edges), \
         extra AS (SELECT id AS node_id, 1099511627776 + min(o) AS np FROM (SELECT source AS id, 2 * p AS o FROM e \
           UNION ALL SELECT target, 2 * p + 1 FROM e) WHERE id NOT IN (SELECT node_id FROM n) GROUP BY id), \
         allnodes AS (SELECT * FROM n UNION ALL BY NAME SELECT * FROM extra), \
         o AS (SELECT e.*, a.np AS nps, min(p) OVER (PARTITION BY source, target) AS pp, \
           min(p) OVER (PARTITION BY source, target, edge_id) AS ep, \
           row_number() OVER (PARTITION BY source, target, edge_id ORDER BY p DESC) AS last \
           FROM e JOIN allnodes a ON e.source = a.node_id), \
         edges AS (SELECT * FROM o WHERE last = 1)",
        nsel = nsel.join(", ")
    );
    let mut nnames = vec!["x".to_string(), "y".into()];
    nnames.extend(ncols.iter().map(|c| format!("\"{c}\"")));
    let nodes_sql = format!("{with} SELECT node_id, {} FROM allnodes ORDER BY np", nnames.join(", "));
    let edges_sql = format!("{with} SELECT source, target, {} FROM edges ORDER BY nps, pp, ep", esel.join(", "));

    let graph: Data = vec![("crs".into(), Py::Str("EPSG:4326".into())), ("mode".into(), Py::Str(mode.into()))];
    let (mut keys, mut key_of) = (Vec::new(), HashMap::new());
    for (k, v) in &graph {
        key_of.insert(("graph".to_string(), k.clone()), keys.len());
        keys.push(("graph".to_string(), v.xml_type(), k.clone()));
    }
    let ntypes: Vec<(String, &'static str)> = column_types(con, &nodes_sql)?.into_iter().skip(1).collect();
    let etypes: Vec<(String, &'static str)> = column_types(con, &edges_sql)?.into_iter().skip(2).collect();
    add_keys(con, &format!("({with} SELECT * FROM allnodes)"), "[np]", "node", &ntypes, &mut keys, &mut key_of)?;
    add_keys(con, &format!("({with} SELECT nps, pp, ep, {} FROM edges)", esel.join(", ")), "[nps, pp, ep]", "edge", &etypes, &mut keys, &mut key_of)?;

    let mut g = GraphWriter { w };
    g.start(&keys, &graph, &key_of)?;
    let ids = |scope: &str, cols: &[(String, &'static str)]| -> Vec<usize> {
        cols.iter().map(|(c, _)| key_of.get(&(scope.to_string(), c.clone())).copied().unwrap_or(usize::MAX)).collect()
    };
    let (nk, ek) = (ids("node", &ntypes), ids("edge", &etypes));
    let mut d: Vec<(usize, Py)> = Vec::new();
    let mut n_nodes = 0;
    each_chunk(con, &nodes_sql, [], |c| {
        for r in 0..c.len {
            d.clear();
            d.extend(nk.iter().enumerate().filter_map(|(i, &k)| py_at(c.col(i + 1), r).map(|v| (k, v))));
            g.element(&format!("<node id=\"{}\"", c.i64(0, r).unwrap_or_default()), "node", &d)?;
        }
        n_nodes += c.len;
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    let mut n_edges = 0;
    let eid = etypes.iter().position(|(c, _)| c == "edge_id").ok_or("no edge_id column")?;
    each_chunk(con, &edges_sql, [], |c| {
        for r in 0..c.len {
            d.clear();
            d.extend(ek.iter().enumerate().filter_map(|(i, &k)| py_at(c.col(i + 2), r).map(|v| (k, v))));
            let (u, v, k) = (c.i64(0, r).unwrap_or_default(), c.i64(1, r).unwrap_or_default(), c.i64(eid + 2, r).unwrap_or_default());
            g.element(&format!("<edge source=\"{u}\" target=\"{v}\" id=\"{k}\""), "edge", &d)?;
        }
        n_edges += c.len;
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    g.end()?;
    Ok((n_nodes, n_edges))
}

/// duckOSM `to_networkx` (with the node data): each node an `edge_id` with `name`, `highway`, `length_m`, `maxspeed_kmh`,
/// `cost_s`, `geometry` (WKT) and with `uses` its `access`; each arc a legal turn with its `weight`.
pub fn edge_graph(con: &Connection, mode: &str, weight: &str, uses: &[String]) -> Result<Doc> {
    let g = crate::route::routing_graph(con, mode, weight, uses)?;
    let roads = crate::route::roads_relation(mode, uses);
    let with_access = !uses.is_empty();
    // the values stay None in their place until the file is written (networkx: dict.update, then _graphml_safe drops None)
    let mut data: Vec<Vec<(String, Option<Py>)>> = vec![Vec::new(); g.n_nodes()];
    let sql = format!(
        "SELECT edge_id, name, highway, length_m, maxspeed_kmh, cost_s, ST_AsText(geometry) AS geometry{} FROM {roads}",
        if with_access { ", access" } else { "" }
    );
    for r in rows(con, &sql)? {
        let Some(i) = int(&r[0].1).and_then(|e| g.index_of(e)) else { continue };
        let d = &mut data[i];
        let mut set = |k: &str, v: &Option<Py>| match d.iter_mut().find(|x| x.0 == k) {
            Some(x) => x.1 = v.clone(),
            None => d.push((k.to_string(), v.clone())),
        };
        for (k, v) in r.iter().skip(1).take(6) {
            set(k, v);
        }
        if with_access {
            if let Some(Py::Str(a)) = &r[7].1 {
                if !a.is_empty() {
                    set("access", &Some(Py::Str(a.clone())));
                }
            }
        }
    }
    let data: Vec<Data> = data.into_iter().map(|d| d.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect()).collect();
    let mut doc = Doc::default();
    doc.nodes = g.ids.iter().zip(data).map(|(&id, d)| (id, d)).collect();
    doc.edges = g.arcs().map(|(u, v, w)| (u, v, None, if w.is_nan() { Vec::new() } else { vec![("weight".to_string(), Py::Float(w))] })).collect();
    Ok(doc)
}
