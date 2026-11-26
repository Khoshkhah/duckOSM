//! duckOSM `routing.py`: the edge-based routing graph of a mode (`to_networkx`) and the shortest route between two edges
//! (`route`, `Router`).

use crate::graph::Graph;
use crate::{err, Result};
use duckdb::Connection;
use serde_json::{json, Value as J};

/// The `uses` of a mode's own turns (duckOSM `ROUTED_USES`).
pub fn routed_use(mode: &str) -> Result<&'static str> {
    match mode {
        "driving" => Ok("car"),
        "walking" => Ok("walk"),
        "cycling" => Ok("bike"),
        m => Err(format!("unknown mode {m}")),
    }
}

/// The labels of the restricted edges that `uses` may name (duckOSM `RESTRICTED_USES`).
pub const RESTRICTED_USES: [&str; 4] = ["bus", "private", "pedestrian", "delivery"];

/// duckOSM `check_uses`: the labels of a comma list, each once, in their order.
pub fn check_uses(uses: Option<&str>) -> Result<Vec<String>> {
    let Some(u) = uses else { return Ok(Vec::new()) };
    let items: Vec<String> = u.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    let bad: Vec<&String> = items.iter().filter(|x| !RESTRICTED_USES.contains(&x.as_str())).collect();
    if !bad.is_empty() {
        let b: Vec<String> = bad.iter().map(|x| format!("'{x}'")).collect();
        return Err(format!("unknown uses [{}]: choose from {}", b.join(", "), RESTRICTED_USES.join(", ")));
    }
    let mut out: Vec<String> = Vec::new();
    for i in items {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// duckOSM `routed_graph`: the rows of `<mode>.edge_graph` the mode routes on, as a FROM-able relation.
pub fn routed_graph(con: &Connection, mode: &str, uses: &[String]) -> Result<String> {
    let has: i64 = con
        .query_row(
            "SELECT count(*) FROM information_schema.columns WHERE table_schema = ? AND table_name = 'edge_graph' AND column_name = 'uses'",
            [mode],
            |r| r.get(0),
        )
        .map_err(err)?;
    if has == 0 {
        return Ok(format!("{mode}.edge_graph"));
    }
    let wanted: Vec<String> = std::iter::once(routed_use(mode)?.to_string()).chain(uses.iter().cloned()).map(|u| format!("'{u}'")).collect();
    Ok(format!("(SELECT * FROM {mode}.edge_graph WHERE uses IN ({}))", wanted.join(", ")))
}

/// duckOSM `_roads_relation`: `<mode>.edges`, or with `uses` also the rows of `private_edges` (with `access`).
pub fn roads_relation(mode: &str, uses: &[String]) -> String {
    if uses.is_empty() {
        return format!("{mode}.edges");
    }
    let cols = "edge_id, name, highway, length_m, maxspeed_kmh, cost_s, geometry";
    format!("(SELECT {cols}, NULL AS access FROM {mode}.edges UNION ALL SELECT {cols}, access FROM {mode}.private_edges)")
}

/// duckOSM `to_networkx(..., node_attrs=False)`: the routing graph; `weight` is `time` (the from-edge's `cost_s`, the
/// graph's `cost`) or `length` (its `length_m`).
pub fn routing_graph(con: &Connection, mode: &str, weight: &str, uses: &[String]) -> Result<Graph> {
    let graph = routed_graph(con, mode, uses)?;
    let roads = roads_relation(mode, uses);
    let sql = match weight {
        "time" => format!("SELECT from_edge, to_edge, cost::DOUBLE FROM {graph}"),
        // duckOSM's join has no fixed row order, so its ties change from run to run; here the rows keep the order of the graph
        "length" => format!(
            "SELECT g.from_edge, g.to_edge, e.length_m::DOUBLE FROM (SELECT *, row_number() OVER () AS rn FROM {graph}) g
             JOIN {roads} e ON e.edge_id = g.from_edge ORDER BY g.rn"
        ),
        _ => return Err("weight must be 'time' or 'length'".into()),
    };
    let mut st = con.prepare(&sql).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    let mut g = duckosm_router::graph::GraphBuilder::default();
    while let Some(r) = rows.next().map_err(err)? {
        let w: Option<f64> = r.get(2).map_err(err)?;
        g.add_arc(r.get(0).map_err(err)?, r.get(1).map_err(err)?, w.unwrap_or(f64::NAN));
    }
    Ok(g.finish())
}

/// Your own cost of each edge (tolls, CO2, danger): it replaces the time of the edge. `skip`: an edge with no own cost is
/// not used; else it costs its time (`cost_s`).
#[derive(Default, Clone)]
pub struct Own {
    pub costs: std::collections::HashMap<i64, f64>,
    pub skip: bool,
}

/// The values of a numeric column of `<mode>.edges` (not NULL) as own costs: `weight="<column>"`.
pub fn column_costs(con: &Connection, mode: &str, column: &str) -> Result<std::collections::HashMap<i64, f64>> {
    let numeric: Option<bool> = con
        .query_row(
            "SELECT data_type IN ('TINYINT','SMALLINT','INTEGER','BIGINT','HUGEINT','UTINYINT','USMALLINT','UINTEGER','UBIGINT',
                                  'FLOAT','DOUBLE','REAL') OR data_type LIKE 'DECIMAL%'
             FROM information_schema.columns WHERE table_schema = ? AND table_name = 'edges' AND column_name = ?",
            [mode, column],
            |r| r.get(0),
        )
        .ok();
    match numeric {
        None => return Err(format!("weight {column:?}: not 'time', 'length' or a column of {mode}.edges")),
        Some(false) => return Err(format!("weight {column:?}: the column of {mode}.edges is not numeric")),
        Some(true) => {}
    }
    let q = column.replace('"', "\"\"");
    let mut st = con.prepare(&format!("SELECT edge_id, \"{q}\"::DOUBLE FROM {mode}.edges WHERE \"{q}\" IS NOT NULL")).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    let mut out = std::collections::HashMap::new();
    while let Some(r) = rows.next().map_err(err)? {
        out.insert(r.get(0).map_err(err)?, r.get(1).map_err(err)?);
    }
    Ok(out)
}

/// The routing graph with own costs: an arc costs the own cost of its from-edge (an arc of `time` costs the time of its
/// from-edge). An edge with no own cost: its arcs keep their time, or with `skip` no arc goes into or out of the edge.
pub fn routing_graph_own(con: &Connection, mode: &str, uses: &[String], own: &Own) -> Result<Graph> {
    if let Some((e, c)) = own.costs.iter().find(|(_, c)| c.is_nan() || **c < 0.0) {
        return Err(format!("edge {e}: the cost {c} is not 0 or more; a route needs costs of 0 or more"));
    }
    let graph = routed_graph(con, mode, uses)?;
    let mut st = con.prepare(&format!("SELECT from_edge, to_edge, cost::DOUBLE FROM {graph}")).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    let mut g = duckosm_router::graph::GraphBuilder::default();
    while let Some(r) = rows.next().map_err(err)? {
        let (a, b): (i64, i64) = (r.get(0).map_err(err)?, r.get(1).map_err(err)?);
        let time: Option<f64> = r.get(2).map_err(err)?;
        if own.skip && !(own.costs.contains_key(&a) && own.costs.contains_key(&b)) {
            continue;
        }
        g.add_arc(a, b, own.costs.get(&a).copied().unwrap_or(time.unwrap_or(f64::NAN)));
    }
    Ok(g.finish())
}

/// Open a database read-only with the spatial extension.
pub fn open(db: &std::path::Path) -> Result<Connection> {
    let con = duckosm_tools::open_read_only(db)?;
    // the router holds its own graph: DuckDB only streams the rows to it (France: 68 million rows of the edge graph)
    con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false; SET memory_limit = '2GB';").map_err(err)?;
    Ok(con)
}

/// duckOSM `route`: `{edges, time_s, length_m, path}`, or None when the two edges are not connected.
pub fn route(con: &Connection, g: &Graph, from_edge: i64, to_edge: i64, mode: &str) -> Result<Option<J>> {
    let missing: Vec<String> = [from_edge, to_edge].iter().filter(|e| !g.contains(**e)).map(i64::to_string).collect();
    if !missing.is_empty() {
        return Err(format!("edge id(s) not in the {mode} routing graph: [{}]", missing.join(", ")));
    }
    let Some((_, path)) = g.bidirectional_dijkstra(from_edge, to_edge)? else { return Ok(None) };
    details(con, &path, mode).map(Some)
}

/// The dict of a route along `path` (edge ids): `{edges, time_s, length_m, path}`, `path` with the row of each edge.
pub fn details(con: &Connection, path: &[i64], mode: &str) -> Result<J> {
    let path = path.to_vec();
    let ids = path.iter().map(i64::to_string).collect::<Vec<_>>().join(", ");
    let mut st = con
        .prepare(&format!(
            "SELECT edge_id, name, highway, length_m::DOUBLE, cost_s::DOUBLE, ST_AsText(geometry) FROM {mode}.edges WHERE edge_id IN ({ids})"
        ))
        .map_err(err)?;
    let mut detail: std::collections::HashMap<i64, (Option<String>, Option<String>, Option<f64>, Option<f64>, Option<String>)> = Default::default();
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        detail.insert(r.get(0).map_err(err)?, (r.get(1).map_err(err)?, r.get(2).map_err(err)?, r.get(3).map_err(err)?, r.get(4).map_err(err)?, r.get(5).map_err(err)?));
    }
    let (mut time, mut length) = (0.0f64, 0.0f64);
    let mut rows_out = Vec::new();
    for e in &path {
        let (name, hw, len, cost, geom) = detail.get(e).cloned().ok_or(format!("edge {e} is not in {mode}.edges"))?;
        // Python sums from 0 (an int), skipping the NULLs
        if let Some(c) = cost {
            time += c;
        }
        if let Some(l) = len {
            length += l;
        }
        rows_out.push(json!({"edge_id": e, "name": name, "highway": hw, "length_m": len, "cost_s": cost, "geometry": geom}));
    }
    let any_cost = path.iter().any(|e| detail[e].3.is_some());
    let any_len = path.iter().any(|e| detail[e].2.is_some());
    Ok(json!({
        "edges": path,
        // sum() of nothing is the int 0 in Python
        "time_s": if any_cost { json!(time) } else { json!(0) },
        "length_m": if any_len { json!(length) } else { json!(0) },
        "path": rows_out,
    }))
}

fn id_list(ids: &[i64]) -> String {
    if ids.is_empty() { "NULL".into() } else { ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",") }
}

/// The cost of each edge of `ids` in the weight of a router: the cost of the last edge of a route, which the graph does not hold
/// (an arc costs its from-edge). `time` and `length`: `cost_s` and `length_m` (NULL: 0). Own costs: the own cost, else the
/// edge's `cost_s`, or NaN with `skip`.
pub fn edge_weights(con: &Connection, mode: &str, weight: &str, own: Option<&Own>, ids: &[i64]) -> Result<std::collections::HashMap<i64, f64>> {
    let col = match (own, weight) {
        (None, "length") => "coalesce(length_m, 0)::DOUBLE",
        _ => "coalesce(cost_s, 0)::DOUBLE",
    };
    let mut st = con.prepare(&format!("SELECT edge_id, {col} FROM {mode}.edges WHERE edge_id IN ({})", id_list(ids))).map_err(err)?;
    let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))).map_err(err)?;
    let mut out = std::collections::HashMap::new();
    for r in rows {
        let (e, t) = r.map_err(err)?;
        let w = match own {
            None => t,
            Some(o) => o.costs.get(&e).copied().unwrap_or(if o.skip { f64::NAN } else { t }),
        };
        out.insert(e, w);
    }
    Ok(out)
}

/// The merged line of the edges `ids` (ST_LineMerge; a MultiLineString where they do not join), as WKB.
pub fn line(con: &Connection, mode: &str, ids: &[i64]) -> Result<Vec<u8>> {
    con.query_row(
        &format!("SELECT ST_AsWKB(ST_LineMerge(ST_Collect(list(geometry)))) FROM {mode}.edges WHERE edge_id IN ({})", id_list(ids)),
        [],
        |r| r.get(0),
    )
    .map_err(err)
}

/// The area of the edges `ids`: the union of the edges, each with a buffer of `buffer_m` metres in the UTM zone of the area.
/// Gives the polygon (WKB, in lon/lat) and its area in km² (in the UTM zone), or None for no edges.
pub fn reach_area(con: &Connection, mode: &str, ids: &[i64], buffer_m: f64, utm: &str) -> Result<Option<(Vec<u8>, f64)>> {
    if ids.is_empty() {
        return Ok(None);
    }
    let sql = format!(
        "SELECT ST_AsWKB(ST_Transform(u, '{utm}', 'EPSG:4326', true)), ST_Area(u) / 1e6 FROM (
           SELECT ST_Union_Agg(ST_Buffer(ST_Transform(geometry, 'EPSG:4326', '{utm}', true), {buffer_m}, 4)) AS u
           FROM {mode}.edges WHERE edge_id IN ({}))",
        id_list(ids)
    );
    con.query_row(&sql, [], |r| Ok(Some((r.get(0)?, r.get(1)?)))).map_err(err)
}

/// The UTM zone of the edges of a mode (the middle of their box).
pub fn utm_of(con: &Connection, mode: &str) -> Result<String> {
    let (x, y): (f64, f64) = con
        .query_row(
            &format!("SELECT (min(ST_XMin(geometry)) + max(ST_XMax(geometry))) / 2, (min(ST_YMin(geometry)) + max(ST_YMax(geometry))) / 2 FROM {mode}.edges"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    Ok(duckosm_export::utm_epsg(x, y))
}

/// The `(lon, lat)` of a place name: the middle of the longest edge of that name in the mode, else the first hit of
/// OpenStreetMap's Nominatim in the box of the mode (this needs the internet).
pub fn geocode(con: &Connection, mode: &str, name: &str) -> Result<(f64, f64)> {
    let local: Option<(f64, f64)> = con
        .query_row(
            &format!(
                "SELECT ST_X(c), ST_Y(c) FROM (SELECT ST_LineInterpolatePoint(geometry, 0.5) AS c FROM {mode}.edges
                 WHERE lower(name) = lower(?) ORDER BY length_m DESC, edge_id LIMIT 1)"
            ),
            [name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| if matches!(e, duckdb::Error::QueryReturnedNoRows) { Ok(None) } else { Err(err(e)) })?;
    if let Some(p) = local {
        return Ok(p);
    }
    let (w, s, e, n): (f64, f64, f64, f64) = con
        .query_row(
            &format!("SELECT min(ST_XMin(geometry)), min(ST_YMin(geometry)), max(ST_XMax(geometry)), max(ST_YMax(geometry)) FROM {mode}.edges"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(err)?;
    let enc: String = name
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let url = format!("https://nominatim.openstreetmap.org/search?q={enc}&format=json&limit=1&viewbox={w},{n},{e},{s}&bounded=1");
    let ua = format!("duckOSM/{} (+https://github.com/Khoshkhah/duckOSM)", env!("CARGO_PKG_VERSION"));
    let mem = Connection::open_in_memory().map_err(err)?;
    let text: String = mem
        .execute_batch(&format!("INSTALL httpfs; LOAD httpfs; CREATE SECRET (TYPE http, EXTRA_HTTP_HEADERS MAP {{'User-Agent': '{ua}'}});"))
        .and_then(|_| mem.query_row(&format!("SELECT content FROM read_text('{url}')"), [], |r| r.get(0)))
        .map_err(|x| format!("Nominatim: {x}"))?;
    let hits: J = serde_json::from_str(&text).map_err(|x| format!("Nominatim: {x}"))?;
    let num = |v: &J| v.as_str().and_then(|t| t.parse::<f64>().ok());
    match hits.get(0) {
        Some(h) => Ok((num(&h["lon"]).ok_or("Nominatim: no lon")?, num(&h["lat"]).ok_or("Nominatim: no lat")?)),
        None => Err(format!("no place {name:?} in the network or in Nominatim inside the box of the network")),
    }
}
