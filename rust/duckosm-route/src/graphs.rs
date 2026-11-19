//! The two graphs of a mode, as rows for any graph library: the junction graph (duckOSM `to_networkx_nodes`: a node is a
//! junction with its longitude and latitude, an edge is a road of `<mode>.edges` with all its columns) and the turn graph
//! (duckOSM `to_networkx`: a node is an `edge_id`, an arc is a legal turn of `edge_graph` with its weight). Python's
//! `duckosm.to_rustworkx` reads these rows; with the cargo feature `petgraph`, [`to_petgraph`] makes petgraph graphs of them.

use crate::route::{open, routing_graph};
use crate::{err, Result};
use duckdb::Connection;
use serde_json::{Map, Value};
use std::path::Path;

/// A junction: `node_id`, longitude `x` and latitude `y`. A road end that `<mode>.nodes` does not have (a cut build) has no `x`
/// and `y`, as in networkx.
#[derive(Clone, Debug, PartialEq)]
pub struct Junction {
    pub node_id: i64,
    pub x: Option<f64>,
    pub y: Option<f64>,
}

/// A road of `<mode>.edges`: its two junctions and its columns (the geometry as WKT; not `source` and `target`).
#[derive(Clone, Debug, PartialEq)]
pub struct Road {
    pub source: i64,
    pub target: i64,
    pub edge_id: i64,
    pub columns: Map<String, Value>,
}

/// The junctions and the roads of `mode`, in the order of the tables. The roads' junctions that are not in `<mode>.nodes` come
/// after the others.
pub fn junction_rows(con: &Connection, mode: &str) -> Result<(Vec<Junction>, Vec<Road>)> {
    let mut junctions = Vec::new();
    let mut st = con
        .prepare(&format!("SELECT node_id, ST_X(geom), ST_Y(geom) FROM {mode}.nodes WHERE geom IS NOT NULL"))
        .map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        junctions.push(Junction { node_id: r.get(0).map_err(err)?, x: r.get(1).map_err(err)?, y: r.get(2).map_err(err)? });
    }
    let mut st = con
        .prepare(&format!(
            "SELECT to_json(e)::VARCHAR FROM (SELECT * EXCLUDE (geometry), ST_AsText(geometry) AS geometry FROM {mode}.edges) e"
        ))
        .map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    let mut roads = Vec::new();
    while let Some(r) = rows.next().map_err(err)? {
        let text: String = r.get(0).map_err(err)?;
        let Value::Object(mut columns) = serde_json::from_str(&text).map_err(|e| e.to_string())? else {
            return Err("a row of the edges is not an object".into());
        };
        let id = |c: &Map<String, Value>, k: &str| c.get(k).and_then(Value::as_i64).ok_or(format!("{mode}.edges: no {k}"));
        let (source, target, edge_id) = (id(&columns, "source")?, id(&columns, "target")?, id(&columns, "edge_id")?);
        columns.remove("source");
        columns.remove("target");
        roads.push(Road { source, target, edge_id, columns });
    }
    let known: std::collections::HashSet<i64> = junctions.iter().map(|j| j.node_id).collect();
    let mut extra: Vec<i64> = roads.iter().flat_map(|r| [r.source, r.target]).filter(|n| !known.contains(n)).collect();
    extra.dedup();
    let mut seen = std::collections::HashSet::new();
    junctions.extend(extra.into_iter().filter(|n| seen.insert(*n)).map(|node_id| Junction { node_id, x: None, y: None }));
    Ok((junctions, roads))
}

/// The turns of `mode` (`from_edge`, `to_edge`, weight): `weight` is `time` (seconds) or `length` (metres), as in
/// [`crate::route::routing_graph`].
pub fn turn_rows(con: &Connection, mode: &str, weight: &str) -> Result<Vec<(i64, i64, f64)>> {
    let g = routing_graph(con, mode, weight, &[])?;
    Ok(g.arcs().collect())
}

/// The two graphs of `mode` of the database `db` as petgraph graphs: the junction graph (a node is a [`Junction`], an edge
/// a [`Road`]; parallel roads stay) and the turn graph (a node is an `edge_id`, an arc a turn with its weight). Each graph comes
/// with the node index of each id.
#[cfg(feature = "petgraph")]
pub struct PetGraphs {
    pub junctions: petgraph::graph::DiGraph<Junction, Road>,
    pub junction_index: std::collections::HashMap<i64, petgraph::graph::NodeIndex>,
    pub turns: petgraph::graph::DiGraph<i64, f64>,
    pub turn_index: std::collections::HashMap<i64, petgraph::graph::NodeIndex>,
}

/// See [`PetGraphs`]. `weight`: `time` or `length`.
#[cfg(feature = "petgraph")]
pub fn to_petgraph(db: &Path, mode: &str, weight: &str) -> Result<PetGraphs> {
    use petgraph::graph::DiGraph;
    use std::collections::HashMap;
    let con = open(db)?;
    let (js, roads) = junction_rows(&con, mode)?;
    let mut junctions = DiGraph::with_capacity(js.len(), roads.len());
    let mut junction_index = HashMap::with_capacity(js.len());
    for j in js {
        let id = j.node_id;
        junction_index.insert(id, junctions.add_node(j));
    }
    for r in roads {
        let (a, b) = (junction_index[&r.source], junction_index[&r.target]);
        junctions.add_edge(a, b, r);
    }
    let arcs = turn_rows(&con, mode, weight)?;
    let mut turns = DiGraph::new();
    let mut turn_index = HashMap::new();
    for (from, to, w) in arcs {
        let a = *turn_index.entry(from).or_insert_with(|| turns.add_node(from));
        let b = *turn_index.entry(to).or_insert_with(|| turns.add_node(to));
        turns.add_edge(a, b, w);
    }
    Ok(PetGraphs { junctions, junction_index, turns, turn_index })
}

/// [`junction_rows`] and [`turn_rows`] of a database file.
pub fn rows_of(db: &Path, mode: &str, weight: &str) -> Result<((Vec<Junction>, Vec<Road>), Vec<(i64, i64, f64)>)> {
    let con = open(db)?;
    Ok((junction_rows(&con, mode)?, turn_rows(&con, mode, weight)?))
}
