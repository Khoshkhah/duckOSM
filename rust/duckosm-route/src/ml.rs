//! `duckosm.ml.graph_arrays`: the arrays of a graph of one mode for graph learning (PyTorch Geometric, DGL). Python only makes
//! the numpy arrays and the tensors.

use crate::{err, Result};
use duckdb::types::Value;
use duckdb::Connection;

/// The arrays of a graph: arcs (`src`, `dst`: node indices), the node features (`x`, row by row, with their names), the arc
/// features (`edge_attr`, with their names), the number of nodes, the `edge_id` (of each arc in the junction graph, of each
/// node in the turn graph) and the `node_id` of each node (junction graph only).
#[derive(Default)]
pub struct Arrays {
    pub src: Vec<i64>,
    pub dst: Vec<i64>,
    pub x: Vec<f32>,
    pub x_names: Vec<String>,
    pub edge_attr: Vec<f32>,
    pub edge_attr_names: Vec<String>,
    pub num_nodes: usize,
    pub edge_id: Vec<i64>,
    pub node_id: Vec<i64>,
}

/// The rows of `sql` as values, with the column names.
fn table(con: &Connection, sql: &str) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let mut st = con.prepare(sql).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    let names: Vec<String> = rows.as_ref().map(|s| s.column_names().into_iter().map(String::from).collect()).unwrap_or_default();
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(err)? {
        out.push((0..names.len()).map(|i| r.get::<_, Value>(i)).collect::<std::result::Result<Vec<_>, _>>().map_err(err)?);
    }
    Ok((names, out))
}

fn number(v: &Value) -> Option<f64> {
    Some(match v {
        Value::Boolean(b) => *b as i32 as f64,
        Value::TinyInt(i) => *i as f64,
        Value::SmallInt(i) => *i as f64,
        Value::Int(i) => *i as f64,
        Value::BigInt(i) => *i as f64,
        Value::HugeInt(i) => *i as f64,
        Value::UTinyInt(i) => *i as f64,
        Value::USmallInt(i) => *i as f64,
        Value::UInt(i) => *i as f64,
        Value::UBigInt(i) => *i as f64,
        Value::Float(f) => *f as f64,
        Value::Double(f) => *f,
        Value::Decimal(d) => d.to_string().parse().ok()?,
        _ => return None,
    })
}

/// The feature matrix of the columns `cols` of `rows` (pandas `get_dummies` with `prefix_sep="="`, then `fillna(0)`): a number
/// or a boolean is one column (NULL: 0); a text column becomes one 0/1 column for each value, `name=value`, after the other
/// columns, the values sorted. Gives the matrix (row by row) and the names.
fn matrix(names: &[String], rows: &[Vec<Value>], cols: &[usize]) -> (Vec<f32>, Vec<String>) {
    let text = |c: usize| rows.iter().any(|r| matches!(r[c], Value::Text(_)));
    let (num, txt): (Vec<usize>, Vec<usize>) = cols.iter().partition(|&&c| !text(c));
    let mut out_names: Vec<String> = num.iter().map(|&c| names[c].clone()).collect();
    let mut levels: Vec<(usize, Vec<String>)> = Vec::new();
    for &c in &txt {
        let mut vals: Vec<String> = rows.iter().filter_map(|r| if let Value::Text(t) = &r[c] { Some(t.clone()) } else { None }).collect();
        vals.sort();
        vals.dedup();
        out_names.extend(vals.iter().map(|v| format!("{}={v}", names[c])));
        levels.push((c, vals));
    }
    let mut m = Vec::with_capacity(rows.len() * out_names.len());
    for r in rows {
        m.extend(num.iter().map(|&c| number(&r[c]).unwrap_or(0.0) as f32));
        for (c, vals) in &levels {
            m.extend(vals.iter().map(|v| matches!(&r[*c], Value::Text(t) if t == v) as i32 as f32));
        }
    }
    (m, out_names)
}

/// The vertex features of the junction graph that duckOSM computes; any other name is a column of `<mode>.nodes`.
fn node_feature(name: &str, edges: &str) -> String {
    match name {
        "lon" => "ST_X(n.geom)".into(),
        "lat" => "ST_Y(n.geom)".into(),
        "in_degree" => format!("(SELECT count(*) FROM {edges} x WHERE x.target = n.node_id)"),
        "out_degree" => format!("(SELECT count(*) FROM {edges} x WHERE x.source = n.node_id)"),
        "degree" => format!("(SELECT count(*) FROM {edges} x WHERE x.source = n.node_id OR x.target = n.node_id)"),
        c => format!("n.\"{}\"", c.replace('"', "\"\"")),
    }
}

/// `graph`: `nodes` (a node is a junction, an arc an edge with the `features` columns) or `turns` (a node is an edge with the
/// `features` columns, an arc a legal turn of `edge_graph` with its `cost`).
pub fn graph_arrays(con: &Connection, mode: &str, graph: &str, features: &[String], node_features: &[String]) -> Result<Arrays> {
    if mode.is_empty() || !mode.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{mode:?} is not a mode name"));
    }
    let q = |c: &String| format!("\"{}\"", c.replace('"', "\"\""));
    let cols: String = features.iter().map(|c| format!(", e.{}", q(c))).collect();
    let e = format!("\"{mode}\".edges");
    let int = |v: &Value| number(v).unwrap_or(0.0) as i64;
    let mut a = Arrays::default();
    if graph == "nodes" {
        let sel: Vec<String> = node_features.iter().map(|c| format!("{} AS {}", node_feature(c, &e), q(c))).collect();
        let (nn, nodes) = table(con, &format!("SELECT n.node_id, {} FROM \"{mode}\".nodes n ORDER BY n.node_id", sel.join(", ")))?;
        let (an, arcs) = table(
            con,
            &format!(
                "WITH n AS (SELECT node_id, row_number() OVER (ORDER BY node_id) - 1 AS i FROM \"{mode}\".nodes)
                 SELECT s.i, t.i, e.edge_id{cols} FROM {e} e JOIN n s ON s.node_id = e.source JOIN n t ON t.node_id = e.target ORDER BY e.edge_id"
            ),
        )?;
        a.num_nodes = nodes.len();
        a.node_id = nodes.iter().map(|r| int(&r[0])).collect();
        (a.x, a.x_names) = matrix(&nn, &nodes, &(1..nn.len()).collect::<Vec<_>>());
        a.src = arcs.iter().map(|r| int(&r[0])).collect();
        a.dst = arcs.iter().map(|r| int(&r[1])).collect();
        a.edge_id = arcs.iter().map(|r| int(&r[2])).collect();
        (a.edge_attr, a.edge_attr_names) = matrix(&an, &arcs, &(3..an.len()).collect::<Vec<_>>());
    } else {
        let (nn, nodes) = table(con, &format!("SELECT e.edge_id{cols} FROM {e} e ORDER BY e.edge_id"))?;
        let uses = crate::route::routed_use(mode).map(str::to_string).unwrap_or_else(|_| mode.to_string());
        let (an, arcs) = table(
            con,
            &format!(
                "WITH n AS (SELECT edge_id, row_number() OVER (ORDER BY edge_id) - 1 AS i FROM {e})
                 SELECT a.i, b.i, g.cost FROM \"{mode}\".edge_graph g JOIN n a ON a.edge_id = g.from_edge JOIN n b ON b.edge_id = g.to_edge
                 WHERE g.uses IS NULL OR g.uses = '{uses}' ORDER BY a.i, b.i"
            ),
        )?;
        a.num_nodes = nodes.len();
        a.edge_id = nodes.iter().map(|r| int(&r[0])).collect();
        (a.x, a.x_names) = matrix(&nn, &nodes, &(1..nn.len()).collect::<Vec<_>>());
        a.src = arcs.iter().map(|r| int(&r[0])).collect();
        a.dst = arcs.iter().map(|r| int(&r[1])).collect();
        (a.edge_attr, a.edge_attr_names) = matrix(&an, &arcs, &[2]);
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_columns_become_one_column_for_each_value_after_the_numbers() {
        let names = vec!["highway".to_string(), "length_m".to_string(), "oneway".to_string()];
        let t = |s: &str| Value::Text(s.into());
        let rows = vec![
            vec![t("primary"), Value::Double(10.0), Value::Boolean(true)],
            vec![t("footway"), Value::Null, Value::Boolean(false)],
            vec![Value::Null, Value::Double(2.5), Value::Null],
        ];
        let (m, n) = matrix(&names, &rows, &[0, 1, 2]);
        assert_eq!(n, ["length_m", "oneway", "highway=footway", "highway=primary"]);
        assert_eq!(m, [10.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 2.5, 0.0, 0.0, 0.0]);
    }
}
