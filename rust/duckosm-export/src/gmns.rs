//! The GMNS export (`duckosm gmns`), the port of src/duckosm/gmns.py (docs/design/all_in_rust.md, group C).
//!
//! The export writes a new DuckDB file with one `gmns_<mode>` schema for each mode, with the source db attached as `s`. The
//! steps are those of `to_gmns` in the same order, with the same SQL; the Python loops are Rust loops over the same rows.
//! Ported so far: the fixed tables (`config`, `use_definition`, `use_group`), `node`, the lanes that a one-way road takes
//! from the road it continues, `link` and `geometry`.

use crate::Result;
use duckdb::Connection;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// duckOSM mode schema -> GMNS use.
fn mode_use(mode: &str) -> &str {
    match mode {
        "driving" => "auto",
        "walking" => "walk",
        "cycling" => "bike",
        m => m,
    }
}

/// (persons_per_vehicle, pce) of a use, as SQL text (Python's float text).
fn use_def(u: &str) -> (&'static str, &'static str) {
    match u {
        "walk" => ("1.0", "0.0"),
        "bike" => ("1.0", "0.2"),
        "bus" => ("25.0", "2.0"),
        _ => ("1.0", "1.0"),
    }
}

/// Saturation capacity (pce/hr/lane) by facility type.
const CAPACITY: [(&str, i32); 10] = [
    ("motorway", 2000),
    ("trunk", 1800),
    ("primary", 1600),
    ("secondary", 1400),
    ("tertiary", 1200),
    ("unclassified", 1000),
    ("residential", 800),
    ("living_street", 300),
    ("service", 300),
    ("road", 800),
];

/// Roads that cars cannot use (duckOSM's `road_filter.NON_MOTOR`).
const NON_MOTOR: [&str; 8] = ["footway", "path", "cycleway", "steps", "pedestrian", "bridleway", "corridor", "platform"];

fn capacity_case(hw: &str) -> String {
    let whens: Vec<String> = CAPACITY.iter().map(|(k, v)| format!("WHEN '{k}' THEN {v}")).collect();
    format!("CASE {hw} {} ELSE 800 END", whens.join(" "))
}

/// SQL: the length in metres of a lon/lat geometry, on the ellipsoid.
fn len_m(g: &str) -> String {
    format!("ST_Length_Spheroid(ST_FlipCoordinates({g}))")
}

const BIKE_FACILITY: &str = "CASE WHEN {v} IS NULL THEN NULL ELSE CASE {v} WHEN 'lane' THEN 'unseparated bike lane' WHEN 'track' THEN 'separated bike lane' \
WHEN 'share_busway' THEN 'shared lane' WHEN 'shared_lane' THEN 'shared lane' \
WHEN 'opposite' THEN 'counter-flow bike lane' WHEN 'opposite_lane' THEN 'counter-flow bike lane' \
WHEN 'shoulder' THEN 'paved shoulder' WHEN 'no' THEN 'none' WHEN 'none' THEN 'none' \
ELSE 'other' END END";
const PED_FACILITY: &str = "CASE WHEN {v} IS NULL THEN NULL ELSE CASE {v} WHEN 'both' THEN 'sidewalk' WHEN 'left' THEN 'sidewalk' WHEN 'right' THEN 'sidewalk' \
WHEN 'yes' THEN 'sidewalk' WHEN 'separate' THEN 'offstreet_path' WHEN 'no' THEN 'none' \
WHEN 'none' THEN 'none' ELSE 'unknown' END END";

struct Ctx<'a> {
    con: &'a Connection,
}

impl Ctx<'_> {
    fn exec(&self, sql: &str) -> Result<()> {
        self.con.execute_batch(sql).map_err(|e| format!("{e}\n{sql}"))
    }
    fn count(&self, sql: &str) -> Result<i64> {
        self.con.query_row(sql, [], |r| r.get(0)).map_err(|e| format!("{e}\n{sql}"))
    }
    fn strings(&self, sql: &str) -> Result<Vec<String>> {
        let mut st = self.con.prepare(sql).map_err(|e| format!("{e}\n{sql}"))?;
        let v = st.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>();
        v.map_err(|e| e.to_string())
    }
    fn exists(&self, schema: &str, table: &str) -> Result<bool> {
        Ok(self.count(&format!(
            "SELECT count(*) FROM duckdb_tables() WHERE database_name = 's' AND schema_name = '{schema}' AND table_name = '{table}'"
        ))? > 0)
    }
    /// The columns of the source db's `<mode>.<table>`.
    fn src_cols(&self, mode: &str, table: &str) -> Result<HashSet<String>> {
        Ok(self
            .strings(&format!(
                "SELECT column_name FROM duckdb_columns() WHERE database_name = 's' AND schema_name = '{mode}' AND table_name = '{table}'"
            ))?
            .into_iter()
            .collect())
    }
}

/// The counts of the main tables of one mode.
pub struct ModeSummary {
    pub mode: String,
    pub counts: Vec<(&'static str, i64)>,
}

/// Write the GMNS db `out` from the built duckOSM db `source`, as `to_gmns` with its defaults (every mode when `modes` is
/// empty). Returns the counts of each mode.
pub fn to_gmns(source: &Path, out: &Path, modes: &[String]) -> Result<Vec<ModeSummary>> {
    crate::open(source)?; // fail fast if the source is unreadable
    if out.exists() {
        std::fs::remove_file(out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let con = Connection::open(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let c = Ctx { con: &con };
    c.exec("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;")?;
    c.exec(&format!("ATTACH '{}' AS s (READ_ONLY)", source.display().to_string().replace('\'', "''")))?;
    let present = c.strings(
        "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE database_name = 's' AND table_name = 'edges' \
         AND schema_name NOT IN ('information_schema', 'pg_catalog', 'main', 'raw') ORDER BY schema_name",
    )?;
    let chosen: Vec<String> = if modes.is_empty() { present.clone() } else { modes.to_vec() };
    if chosen.is_empty() {
        return Err("no mode schemas with an 'edges' table in the source db".into());
    }
    let unknown: Vec<&String> = chosen.iter().filter(|m| !present.contains(m)).collect();
    if !unknown.is_empty() {
        return Err(format!("mode(s) {unknown:?} not found (present: {present:?})"));
    }
    let has_raw = c.exists("raw", "ways")?;
    if has_raw {
        c.exec("CREATE TEMP TABLE _sig AS SELECT DISTINCT osm_id FROM s.raw.nodes WHERE tags['highway'] = 'traffic_signals'")?;
    } else {
        c.exec("CREATE TEMP TABLE _sig(osm_id BIGINT)")?;
        eprintln!("GMNS: no 'raw' schema — lane/signal/curb detail limited (no OSM tags)");
    }
    let name = out.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut summary = Vec::new();
    for mode in &chosen {
        let sch = format!("gmns_{mode}");
        let uses = mode_use(mode);
        c.exec(&format!("CREATE SCHEMA {sch}"))?;
        build_fixed(&c, &sch, &name, mode, uses)?;
        edge_view(&c, mode, has_raw)?;
        build_node(&c, &sch, mode)?;
        infer_lanes(&c, mode, has_raw)?;
        build_link(&c, &sch, mode, uses, has_raw)?;
        build_geometry(&c, &sch)?;
        let mut counts = Vec::new();
        for t in ["node", "link", "geometry"] {
            counts.push((t, c.count(&format!("SELECT count(*) FROM {sch}.{t}"))?));
        }
        summary.push(ModeSummary { mode: mode.clone(), counts });
    }
    c.exec("DETACH s")?;
    Ok(summary)
}

/// config + use_definition + use_group: the small fixed tables.
fn build_fixed(c: &Ctx, sch: &str, name: &str, mode: &str, uses: &str) -> Result<()> {
    c.exec(&format!(
        "CREATE TABLE {sch}.config AS SELECT * FROM (VALUES
      ('{name}_{mode}', 'meter', 'meter', 'kmh', 'EPSG:4326', 'wkt', NULL::VARCHAR, 0.97::DOUBLE, 'integer')
    ) t(dataset_name, short_length, long_length, speed, crs, geometry_field_format,
        currency, version_number, id_type)"
    ))?;
    // every use a lane can carry must be defined: a driving network's lanes are auto, bus or bike
    let mut defined = vec![uses];
    if mode == "driving" {
        defined.extend(["bus", "bike"]);
    }
    let rows: Vec<String> = defined.iter().map(|u| format!("('{u}', {}, {})", use_def(u).0, use_def(u).1)).collect();
    c.exec(&format!(
        "CREATE TABLE {sch}.use_definition AS SELECT use, persons_per_vehicle, pce,
      NULL::VARCHAR AS special_conditions, NULL::VARCHAR AS description
      FROM (VALUES {}) t(use, persons_per_vehicle, pce)",
        rows.join(", ")
    ))?;
    c.exec(&format!(
        "CREATE TABLE {sch}.use_group AS SELECT '{mode}' AS use_group, '{uses}' AS uses, NULL::VARCHAR AS description"
    ))
}

/// `_edges` (a temp view): the links of the mode, `_edges_car` (the mode's edges) then `_edges_bus` (in driving, the bus-only
/// edges of `private_edges`; else none), as `_edge_view`.
fn edge_view(c: &Ctx, mode: &str, has_raw: bool) -> Result<()> {
    c.exec(&format!("CREATE OR REPLACE TEMP VIEW _edges_car AS SELECT *, false AS _bus, NULL::BOOLEAN AS _bike FROM s.{mode}.edges"))?;
    c.exec("CREATE OR REPLACE TEMP VIEW _edges AS SELECT * FROM _edges_car")?;
    c.exec("CREATE OR REPLACE TEMP VIEW _edges_bus AS SELECT * FROM _edges_car WHERE false")?;
    if mode != "driving" || !c.exists("driving", "private_edges")? {
        return Ok(());
    }
    let mut bike = Vec::new();
    if has_raw {
        bike.push("w.tags['oneway:bicycle'] = 'no' OR 'share_busway' IN (w.tags['cycleway'], w.tags['cycleway:left'], w.tags['cycleway:right'])");
    }
    if c.exists("cycling", "edges")? {
        bike.push("p.edge_id IN (SELECT edge_id FROM s.cycling.edges)");
    }
    let raw_join = if has_raw { "LEFT JOIN s.raw.ways w ON w.osm_id = abs(p.osm_id)" } else { "" };
    let b = if bike.is_empty() { "false".to_string() } else { bike.join(" OR ") };
    c.exec(&format!(
        "CREATE OR REPLACE TEMP VIEW _edges_bus AS
      SELECT p.*, true AS _bus, COALESCE({b}, false) AS _bike
      FROM s.driving.private_edges p {raw_join} WHERE p.access = 'bus'"
    ))?;
    c.exec("CREATE OR REPLACE TEMP VIEW _edges AS SELECT * FROM _edges_car UNION ALL BY NAME SELECT * FROM _edges_bus")
}

fn build_node(c: &Ctx, sch: &str, mode: &str) -> Result<()> {
    let z = if c.src_cols(mode, "nodes")?.contains("ele") { "n.ele::DOUBLE" } else { "NULL::DOUBLE" };
    c.exec(&format!(
        "CREATE TABLE {sch}.node AS SELECT
      n.node_id, NULL::VARCHAR AS name, ST_X(n.geom) AS x_coord, ST_Y(n.geom) AS y_coord,
      {z} AS z_coord, NULL::VARCHAR AS node_type,
      CASE WHEN sig.osm_id IS NOT NULL THEN 'signal' END AS ctrl_type,
      1::BIGINT AS zone_id, NULL::BIGINT AS parent_node_id, n.geom
    FROM s.{mode}.nodes n LEFT JOIN _sig sig ON sig.osm_id = n.node_id
    WHERE n.geom IS NOT NULL"
    ))?;
    // a bus-only road's own nodes are not in the mode's nodes: its edges' ends, no height
    c.exec(&format!(
        "INSERT INTO {sch}.node BY NAME
      SELECT node_id, any_value(ST_X(p)) AS x_coord, any_value(ST_Y(p)) AS y_coord, 1::BIGINT AS zone_id, any_value(p) AS geom,
             CASE WHEN bool_or(sig.osm_id IS NOT NULL) THEN 'signal' END AS ctrl_type
      FROM (SELECT source AS node_id, ST_StartPoint(geometry) AS p FROM _edges_bus
            UNION ALL SELECT target, ST_EndPoint(geometry) FROM _edges_bus) e
      LEFT JOIN _sig sig ON sig.osm_id = e.node_id
      WHERE node_id NOT IN (SELECT node_id FROM {sch}.node) GROUP BY node_id"
    ))
}

struct LaneEdge {
    id: i64,
    source: i64,
    target: i64,
    cls: Option<String>,
    lanes: Option<i64>,
    tagged: bool,
    oneway: bool,
}

/// Lane counts for one-way roads OSM gives none for, as `_inherit_lanes`: an untagged one-way edge takes the lanes of the road
/// it plainly continues (the node between them joins only the two, both one-way, of one class) when that has more; else of the
/// one it plainly leads into. Up to `passes` passes over the edges in their order. Returns the edges that change, in the order
/// of their first change.
fn inherit_lanes(mut edges: Vec<LaneEdge>, passes: usize) -> Vec<(i64, i64)> {
    let index: HashMap<i64, usize> = edges.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
    let mut at: HashMap<i64, HashSet<i64>> = HashMap::new();
    for e in &edges {
        at.entry(e.source).or_default().insert(e.id);
        at.entry(e.target).or_default().insert(e.id);
    }
    let mut changed: Vec<(i64, i64)> = Vec::new();
    for _ in 0..passes {
        let mut step = false;
        for i in 0..edges.len() {
            if edges[i].tagged || !edges[i].oneway {
                continue;
            }
            for (node, end_is_target) in [(edges[i].source, true), (edges[i].target, false)] {
                let others: Vec<i64> = at[&node].iter().copied().filter(|x| *x != edges[i].id).collect();
                if others.len() != 1 {
                    continue;
                }
                let o = &edges[index[&others[0]]];
                let o_end = if end_is_target { o.target } else { o.source };
                if o.oneway && o.cls == edges[i].cls && o_end == node && py_or1(o.lanes) > py_or1(edges[i].lanes) {
                    let l = o.lanes;
                    edges[i].lanes = l;
                    let id = edges[i].id;
                    let v = l.unwrap_or(0);
                    match changed.iter_mut().find(|c| c.0 == id) {
                        Some(c) => c.1 = v,
                        None => changed.push((id, v)),
                    }
                    step = true;
                    break;
                }
            }
        }
        if !step {
            break;
        }
    }
    changed
}

/// Python's `x or 1` for a lane count.
fn py_or1(x: Option<i64>) -> i64 {
    match x {
        Some(v) if v != 0 => v,
        _ => 1,
    }
}

/// `_lanes_inf(edge_id, lanes)` (a temp table): `inherit_lanes` over the mode's edges, as `_infer_lanes`.
fn infer_lanes(c: &Ctx, mode: &str, has_raw: bool) -> Result<()> {
    c.exec("CREATE OR REPLACE TEMP TABLE _lanes_inf(edge_id BIGINT, lanes INTEGER)")?;
    if !has_raw || mode != "driving" || !c.src_cols(mode, "edges")?.contains("oneway") {
        return Ok(());
    }
    let sql = format!(
        "SELECT e.edge_id, e.source, e.target, regexp_replace(split_part(e.highway, ';', 1), '_link$', ''), e.lanes::BIGINT,
        (w.tags['lanes'] IS NOT NULL OR w.tags['turn:lanes'] IS NOT NULL OR w.tags['width:lanes'] IS NOT NULL OR w.tags['lanes:forward'] IS NOT NULL),
        e.oneway::BOOLEAN FROM s.{mode}.edges e LEFT JOIN s.raw.ways w ON w.osm_id = abs(e.osm_id)"
    );
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let edges: Vec<LaneEdge> = st
        .query_map([], |r| {
            Ok(LaneEdge {
                id: r.get(0)?,
                source: r.get(1)?,
                target: r.get(2)?,
                cls: r.get(3)?,
                lanes: r.get(4)?,
                tagged: r.get::<_, Option<bool>>(5)?.unwrap_or(false),
                oneway: r.get::<_, Option<bool>>(6)?.unwrap_or(false),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let got = inherit_lanes(edges, 6);
    if !got.is_empty() {
        let mut app = c.con.appender("_lanes_inf").map_err(|e| e.to_string())?;
        for (id, l) in &got {
            app.append_row(duckdb::params![id, *l as i32]).map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
        eprintln!("GMNS[{mode}]: {} one-way edges without a lanes tag take the lane count of the road they continue", got.len());
    }
    Ok(())
}

fn build_link(c: &Ctx, sch: &str, mode: &str, uses: &str, has_raw: bool) -> Result<()> {
    let raw_join = if has_raw { "LEFT JOIN s.raw.ways w ON w.osm_id = abs(e.osm_id)" } else { "" };
    let bike = if has_raw { BIKE_FACILITY.replace("{v}", "w.tags['cycleway']") } else { "NULL::VARCHAR".into() };
    let ped = if has_raw { PED_FACILITY.replace("{v}", "w.tags['sidewalk']") } else { "NULL::VARCHAR".into() };
    let cols = c.src_cols(mode, "edges")?;
    let lvl: Vec<String> =
        ["bridge", "tunnel", "layer"].iter().map(|k| if cols.contains(*k) { format!("e.{k}") } else { format!("NULL::VARCHAR AS {k}") }).collect();
    let edge_ref = if cols.contains("edge_ref") { "e.edge_ref AS edge_ref" } else { "NULL::VARCHAR AS edge_ref" };
    let footway = if has_raw { "w.tags['footway'] AS footway" } else { "NULL::VARCHAR AS footway" };
    let crossing = if has_raw {
        "CASE WHEN w.tags['footway'] = 'crossing' THEN w.tags['crossing'] END AS crossing, \
         CASE WHEN w.tags['footway'] = 'crossing' THEN w.tags['crossing:markings'] END AS crossing_markings"
    } else {
        "NULL::VARCHAR AS crossing, NULL::VARCHAR AS crossing_markings"
    };
    let glen = len_m("e.geometry");
    let structure: Vec<String> =
        ["tunnel", "bridge"].iter().filter(|k| cols.contains(**k)).map(|k| format!("COALESCE(e.{k}, 'no') NOT IN ('no', '')")).collect();
    let structure = if structure.is_empty() { "false".to_string() } else { structure.join(" OR ") };
    let grade = if !(cols.contains("z_from") && cols.contains("z_to")) {
        "NULL::DOUBLE".to_string()
    } else {
        format!(
            "CASE WHEN NOT ({structure}) AND abs(100.0 * (e.z_to - e.z_from) / NULLIF({glen}, 0)) <= 100 THEN 100.0 * (e.z_to - e.z_from) / {glen} END"
        )
    };
    let hw = "regexp_replace(split_part(e.highway, ';', 1), '_link$', '')";
    let motor = if mode == "driving" {
        "true".to_string()
    } else {
        format!("{hw} NOT IN ({})", NON_MOTOR.iter().map(|h| format!("'{h}'")).collect::<Vec<_>>().join(", "))
    };
    let cap = capacity_case(hw);
    let sel = |from: &str| {
        format!(
            "SELECT
      e.edge_id AS link_id, e.name AS name, e.source AS from_node_id, e.target AS to_node_id,
      true AS directed, e.edge_id AS geometry_id, ST_AsText(e.geometry) AS geometry,
      NULL::BIGINT AS parent_link_id, 1 AS dir_flag,
      {glen} AS length,
      {grade}::DOUBLE AS grade,
      e.highway AS facility_type,
      CASE WHEN {motor} THEN {cap}::DOUBLE END AS capacity,
      e.maxspeed_kmh AS free_speed, CASE WHEN e._bus THEN 1 WHEN {motor} THEN COALESCE(li.lanes, e.lanes) END AS lanes,
      {bike} AS bike_facility, {ped} AS ped_facility, NULL::VARCHAR AS parking,
      CASE WHEN e._bike THEN 'bus,bike' WHEN e._bus THEN 'bus' ELSE '{uses}' END AS allowed_uses,
      NULL::DOUBLE AS toll, NULL::VARCHAR AS jurisdiction, NULL::DOUBLE AS row_width, abs(e.osm_id) AS osm_id, {edge_ref}, {footway}, {crossing}, {lvl}, e.geometry AS geom
    FROM {from} e {raw_join} LEFT JOIN _lanes_inf li ON li.edge_id = e.edge_id",
            lvl = lvl.join(", ")
        )
    };
    c.exec(&format!("CREATE TABLE {sch}.link AS {}", sel("_edges_car")))?;
    c.exec(&format!("INSERT INTO {sch}.link BY NAME {}", sel("_edges_bus")))
}

fn build_geometry(c: &Ctx, sch: &str) -> Result<()> {
    c.exec(&format!("CREATE TABLE {sch}.geometry AS SELECT edge_id AS geometry_id, ST_AsText(geometry) AS geometry, geometry AS geom FROM _edges_car"))?;
    c.exec(&format!("INSERT INTO {sch}.geometry SELECT edge_id, ST_AsText(geometry), geometry FROM _edges_bus"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: i64, s: i64, t: i64, lanes: i64, tagged: bool) -> LaneEdge {
        LaneEdge { id, source: s, target: t, cls: Some("primary".into()), lanes: Some(lanes), tagged, oneway: true }
    }

    /// A tagged 2-lane one-way road, then two untagged pieces: both take 2 lanes, piece by piece.
    #[test]
    fn lanes_inherit_piece_by_piece() {
        let got = inherit_lanes(vec![e(3, 3, 4, 1, false), e(2, 2, 3, 1, false), e(1, 1, 2, 2, true)], 6);
        assert_eq!(got, vec![(2, 2), (3, 2)]);
    }
}
