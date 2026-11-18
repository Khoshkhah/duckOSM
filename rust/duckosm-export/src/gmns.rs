//! The GMNS export (`duckosm gmns`), the port of src/duckosm/gmns.py (docs/design/all_in_rust.md, group C).
//!
//! The export writes a new DuckDB file with one `gmns_<mode>` schema for each mode, with the source db attached as `s`. The
//! steps are those of `to_gmns` in the same order, with the same SQL; the Python loops are Rust loops over the same rows.
//! Ported so far: the fixed tables (`config`, `use_definition`, `use_group`), `node`, the lanes that a one-way road takes
//! from the road it continues, `link`, `geometry`, and the roads that a footpath runs along (`link_along`, `link.along_*`,
//! `link.parent_link_id`), and the lanes (without their geometry yet) and curbs.

use crate::shapely::{self as shp, P};
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
        for (col, typ) in [("along_link_id", "BIGINT"), ("along_mode", "VARCHAR"), ("along_gap_m", "DOUBLE"), ("along_kind", "VARCHAR")] {
            c.exec(&format!("ALTER TABLE {sch}.link ADD COLUMN {col} {typ}"))?;
        }
        c.exec(&format!(
            "CREATE TABLE {sch}.link_along(link_id BIGINT, along_link_id BIGINT, along_mode VARCHAR, covered_m DOUBLE, gap_m DOUBLE)"
        ))?;
        if mode != "driving" {
            sidewalk_parents(&c, &sch, has_raw, "right")?;
        }
        build_lane_curb(&c, &sch, mode, uses, has_raw, "right")?;
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

/// `crossings._level`: the OSM `layer` if it is a number other than 0, else 1 for a bridge, -1 for a tunnel, else 0.
fn level(layer: &Option<String>, bridge: &Option<String>, tunnel: &Option<String>) -> i64 {
    let ly = layer.as_deref().and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).map(|v| v.trunc() as i64).unwrap_or(0);
    if ly != 0 {
        return ly;
    }
    let yes = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.is_empty() && s != "no");
    if yes(bridge) {
        1
    } else if yes(tunnel) {
        -1
    } else {
        0
    }
}

/// `_default_lane_w`: the lane width of a road class.
fn default_lane_w(highway: Option<&str>) -> f64 {
    let h = highway.unwrap_or("").split(';').next().unwrap_or("");
    match h.strip_suffix("_link").unwrap_or(h) {
        "service" => 2.5,
        "residential" | "living_street" | "unclassified" => 3.0,
        _ => DEFAULT_LANE_W,
    }
}

const DEFAULT_LANE_W: f64 = 3.25;
const DEFAULT_WALK_W: f64 = 2.0;
const CROSSING_ALONG: f64 = 0.6;
const CROSSING_MIN_M: f64 = 10.0;
const ALONG_NEAR_M: f64 = 8.0;

struct Road {
    id: i64,
    line: Vec<P>,
    level: i64,
    env: (f64, f64, f64, f64),
}

fn envelope(l: &[P]) -> (f64, f64, f64, f64) {
    l.iter().fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |e, p| (e.0.min(p.0), e.1.min(p.1), e.2.max(p.0), e.3.max(p.1)))
}

/// The direction of the line at the distance t along it, radians.
fn heading(g: &[P], len: f64, t: f64) -> f64 {
    let (a, b) = (shp::interpolate(g, (t - 1.0).max(0.0)), shp::interpolate(g, (t + 1.0).min(len)));
    (b.1 - a.1).atan2(b.0 - a.0)
}

/// `_sidewalk_parents`: for each footpath (a mapped sidewalk, or a footpath right beside a road), the road it runs along
/// (`link.along_*`, and `parent_link_id` of a sidewalk where the road is a link of this schema) and every road it runs along
/// for 4 m or more (`link_along`). The roads are those of `gmns_driving` when it is there, else the schema's own roads.
fn sidewalk_parents(c: &Ctx, sch: &str, has_raw: bool, drive_side: &str) -> Result<()> {
    const MAX_GAP_M: f64 = 30.0;
    const MAX_TURN_DEG: f64 = 30.0;
    const ADJACENT_M: f64 = 5.0;
    if !has_raw {
        return Ok(());
    }
    let near_classes = NON_MOTOR.iter().filter(|h| !["steps", "platform", "corridor"].contains(h)).map(|h| format!("'{h}'")).collect::<Vec<_>>().join(", ");
    type Sw = (i64, String, Option<String>, Option<String>, Option<String>, String, Option<bool>, i64, i64);
    let sql = format!(
        "SELECT k.link_id, ST_AsText(k.geom), k.layer::VARCHAR, k.bridge::VARCHAR, k.tunnel::VARCHAR,
           CASE WHEN w.tags['footway'] = 'sidewalk' THEN 'sidewalk' ELSE 'adjacent' END, w.tags['footway'] = 'crossing', k.from_node_id, k.to_node_id
         FROM {sch}.link k JOIN s.raw.ways w ON w.osm_id = abs(k.osm_id)
         WHERE w.tags['footway'] = 'sidewalk' OR (COALESCE(w.tags['footway'], '') NOT IN ('link', 'sidewalk')
           AND regexp_replace(split_part(k.facility_type, ';', 1), '_link$', '') IN ({near_classes}))"
    );
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let sw: Vec<Sw> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| e.to_string())?;
    drop(st);
    if sw.is_empty() {
        return Ok(());
    }
    let nonroad = NON_MOTOR.iter().map(|h| format!("'{h}'")).collect::<Vec<_>>().join(", ");
    let has_link = |s: &str| c.count(&format!("SELECT count(*) FROM duckdb_tables() WHERE schema_name = '{s}' AND table_name = 'link'")).map(|n| n > 0);
    let have_drive = sch != "gmns_driving" && has_link("gmns_driving")?;
    let road_schema = if have_drive { "gmns_driving" } else { sch };
    let sql = format!(
        "SELECT link_id, ST_AsText(geom), layer::VARCHAR, bridge::VARCHAR, tunnel::VARCHAR FROM {road_schema}.link WHERE
            regexp_replace(split_part(facility_type, ';', 1), '_link$', '') NOT IN ({nonroad})"
    );
    type RoadRow = (i64, String, Option<String>, Option<String>, Option<String>);
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let rows: Vec<RoadRow> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| e.to_string())?;
    drop(st);
    if rows.is_empty() {
        return Ok(());
    }
    let own: HashSet<i64> = if have_drive {
        let mine: HashSet<i64> = c.strings(&format!("SELECT link_id::VARCHAR FROM {sch}.link"))?.iter().map(|s| s.parse().unwrap()).collect();
        rows.iter().map(|r| r.0).filter(|id| mine.contains(id)).collect()
    } else {
        rows.iter().map(|r| r.0).collect()
    };
    let first = shp::wkt_points(&sw[0].1)[0];
    let scale = (first.1.to_radians().cos() * 111320.0, 111320.0);
    let to_m = |w: &str| -> Vec<P> { shp::wkt_points(w).into_iter().map(|(x, y)| (x * scale.0, y * scale.1)).collect() };
    let roads: Vec<Road> = rows
        .iter()
        .map(|r| {
            let line = to_m(&r.1);
            let env = envelope(&line);
            Road { id: r.0, line, level: level(&r.2, &r.3, &r.4), env }
        })
        .collect();
    let tree = shp::strtree_order(&roads.iter().map(|r| r.env).collect::<Vec<_>>());
    // a road's edge is far from its centre line: half its lanes (a twin pair: one direction's lanes)
    let mut half: HashMap<i64, f64> = HashMap::new();
    for g in [sch, "gmns_driving"] {
        if !has_link(g)? {
            continue;
        }
        let sql = format!("SELECT link_id, COALESCE(lanes, 1)::BIGINT, facility_type, edge_ref FROM {g}.link");
        let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
        let links: Vec<(i64, i64, Option<String>, Option<String>)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let stem = |e: &str| e[..e.char_indices().last().map(|x| x.0).unwrap_or(0)].to_string();
        let mut pair: HashMap<String, usize> = HashMap::new();
        for (_, _, _, e) in &links {
            if let Some(e) = e.as_deref().filter(|e| !e.is_empty()) {
                *pair.entry(stem(e)).or_default() += 1;
            }
        }
        for (k, n, ft, e) in &links {
            let twin = e.as_deref().is_some_and(|e| !e.is_empty() && pair[&stem(e)] > 1);
            half.entry(*k).or_insert(*n as f64 * default_lane_w(ft.as_deref()) * if twin { 1.0 } else { 0.5 });
        }
    }
    let kerb = if drive_side == "right" { -1.0 } else { 1.0 };
    let mode_of = |rid: i64| if own.contains(&rid) { sch.trim_start_matches("gmns_").to_string() } else { "driving".to_string() };
    let mut parents: Vec<(i64, i64, f64, String)> = Vec::new(); // (road, link, distance, kind)
    let mut along_rows: Vec<(i64, i64, String, f64, f64)> = Vec::new();
    let mut match_one = |link_id: i64, g: &[P], lv: i64, kind: &str, crossing: bool, allowed: Option<&HashSet<i64>>| -> HashSet<i64> {
        let glen = shp::length(g);
        let mid = shp::interpolate(g, 0.5 * glen);
        let mut ds: Vec<f64> = (0..glen as i64).step_by(2).map(|d| d as f64).collect();
        ds.push(glen);
        let samples: Vec<P> = ds.iter().map(|d| shp::interpolate(g, *d)).collect();
        let genv = envelope(g);
        let mut best: Option<((i64, f64, i32), i64, f64, f64)> = None;
        let mut route: Vec<(i64, f64, f64)> = Vec::new();
        for r in tree.iter().map(|&i| &roads[i]) {
            let e = r.env;
            let env_gap = (e.0 - genv.2).max(genv.0 - e.2).max(0.0).hypot((e.1 - genv.3).max(genv.1 - e.3).max(0.0));
            if env_gap > MAX_GAP_M {
                continue;
            }
            let gd = shp::line_distance(g, &r.line);
            if gd > MAX_GAP_M || r.level != lv || allowed.is_some_and(|a| !a.contains(&r.id)) {
                continue;
            }
            let rl = &r.line;
            let rlen = shp::length(rl);
            let hw = half.get(&r.id).copied().unwrap_or(0.0);
            let mut cov = 0i64;
            for sp in &samples {
                let t = shp::project(rl, *sp);
                if shp::dist(*sp, shp::interpolate(rl, t)) - hw > ALONG_NEAR_M {
                    continue;
                }
                let turn = (((heading(rl, rlen, t) - heading(g, glen, shp::project(g, *sp)) + std::f64::consts::PI / 2.0)
                    .rem_euclid(std::f64::consts::PI))
                    - std::f64::consts::PI / 2.0)
                    .abs();
                cov += (turn.to_degrees() <= MAX_TURN_DEG) as i64;
            }
            if cov == 0 || (crossing && (cov as f64) < CROSSING_ALONG * samples.len() as f64) {
                continue;
            }
            let t = shp::project(rl, mid);
            let p = shp::interpolate(rl, t);
            let (a, b) = (shp::interpolate(rl, (t - 1.0).max(0.0)), shp::interpolate(rl, (t + 1.0).min(rlen)));
            let side = (b.0 - a.0) * (mid.1 - p.1) - (b.1 - a.1) * (mid.0 - p.0);
            let key = (-cov, ((gd - hw).max(0.0) * 2.0).round_ties_even() / 2.0, if side * kerb > 0.0 { 0 } else { 1 });
            if cov >= 2 && (kind == "sidewalk" || gd - hw - DEFAULT_WALK_W / 2.0 <= ADJACENT_M) {
                route.push((r.id, cov as f64 * 2.0, gd));
            }
            let less = |k: &(i64, f64, i32), b: &(i64, f64, i32)| k.partial_cmp(b) == Some(std::cmp::Ordering::Less);
            if best.as_ref().is_none_or(|b| less(&key, &b.0)) {
                best = Some((key, r.id, gd, gd - hw - DEFAULT_WALK_W / 2.0));
            }
        }
        let Some(best) = best.filter(|b| kind == "sidewalk" || b.3 <= ADJACENT_M) else {
            return HashSet::new();
        };
        parents.push((best.1, link_id, best.2, kind.to_string()));
        for (rid, cov_m, d) in &route {
            along_rows.push((link_id, *rid, mode_of(*rid), *cov_m, shp::py_round(*d, 2)));
        }
        route.iter().map(|r| r.0).chain([best.1]).collect()
    };
    let mut at_node: HashMap<i64, HashSet<i64>> = HashMap::new();
    let mut deferred = Vec::new();
    for (link_id, w, layer, bridge, tunnel, kind, crossing, n0, n1) in &sw {
        let g = to_m(w);
        if shp::length(&g) < 1.0 {
            continue;
        }
        let lv = level(layer, bridge, tunnel);
        let crossing = crossing.unwrap_or(false);
        if crossing && shp::length(&g) < CROSSING_MIN_M {
            deferred.push((*link_id, g, lv, kind.clone(), *n0, *n1));
            continue;
        }
        let got = match_one(*link_id, &g, lv, kind, crossing, None);
        at_node.entry(*n0).or_default().extend(got.iter().copied());
        at_node.entry(*n1).or_default().extend(got);
    }
    for (link_id, g, lv, kind, n0, n1) in &deferred {
        let allowed: HashSet<i64> = at_node.get(n0).into_iter().chain(at_node.get(n1)).flatten().copied().collect();
        if !allowed.is_empty() {
            match_one(*link_id, g, *lv, kind, true, Some(&allowed));
        }
    }
    c.exec("CREATE OR REPLACE TEMP TABLE _parents(road BIGINT, link BIGINT, mode VARCHAR, gap DOUBLE, kind VARCHAR)")?;
    {
        let mut app = c.con.appender("_parents").map_err(|e| e.to_string())?;
        for (r, k, d, kd) in &parents {
            app.append_row(duckdb::params![r, k, mode_of(*r), shp::py_round(*d, 2), kd]).map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
    }
    let mut app = c.con.appender_to_db("link_along", sch).map_err(|e| e.to_string())?;
    for (k, r, m, cov, gap) in &along_rows {
        app.append_row(duckdb::params![k, r, m, cov, gap]).map_err(|e| e.to_string())?;
    }
    app.flush().map_err(|e| e.to_string())?;
    drop(app);
    let own_list = if own.is_empty() { "NULL".to_string() } else { own.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",") };
    c.exec(&format!(
        "UPDATE {sch}.link SET parent_link_id = p.road FROM _parents p WHERE p.link = {sch}.link.link_id AND p.kind = 'sidewalk' AND p.road IN ({own_list});
         UPDATE {sch}.link SET along_link_id = p.road, along_mode = p.mode, along_gap_m = p.gap, along_kind = p.kind FROM _parents p WHERE p.link = {sch}.link.link_id;
         DROP TABLE _parents"
    ))
}

const DEFAULT_BIKE_W: f64 = 1.5;
const DEFAULT_BIKE_LANE_W: f64 = 1.5;

type Tags = HashMap<String, String>;

/// A tag value as Python's `tags.get(k)` where an empty text counts as missing in an `or`.
fn tag<'a>(t: &'a Tags, k: &str) -> Option<&'a str> {
    t.get(k).map(|s| s.as_str())
}

/// Python's `a or b` for two optional texts.
fn or<'a>(a: Option<&'a str>, b: Option<&'a str>) -> Option<&'a str> {
    match a {
        Some(x) if !x.is_empty() => Some(x),
        _ => b,
    }
}

/// `_split`: a `|` list, empty for a missing or empty text.
fn split(s: Option<&str>) -> Vec<String> {
    match s {
        Some(x) if !x.is_empty() => x.split('|').map(String::from).collect(),
        _ => vec![],
    }
}

/// `_num`: a number, or None.
fn num(s: Option<&str>) -> Option<f64> {
    s.and_then(|x| x.trim().parse::<f64>().ok())
}

/// Python's `x or y` for numbers (0 and None are false).
fn nor(a: Option<f64>, b: f64) -> f64 {
    match a {
        Some(v) if v != 0.0 => v,
        _ => b,
    }
}

fn tags_of(v: duckdb::types::Value) -> Tags {
    use duckdb::types::Value;
    let text = |v: &Value| match v {
        Value::Text(s) => Some(s.clone()),
        _ => None,
    };
    match v {
        Value::Map(m) => m.iter().filter_map(|(k, v)| Some((text(k)?, text(v)?))).collect(),
        _ => Tags::new(),
    }
}

/// One edge of the GMNS network, as `_build_lane_curb` reads it.
struct LaneRow {
    edge_id: i64,
    is_rev: bool,
    lanes: Option<i64>,
    length_m: Option<f64>,
    source: i64,
    oneway: bool,
    tags: Tags,
    osm_id: i64,
    bus: bool,
    bike: Option<bool>,
    way_end: bool,
}

/// The rules of the lanes of one edge from its tags (`pick`, `bike_extra`, `bike_left`, `is_foot`, `lane_widths`, `_rule_lanes`).
struct Rules<'a> {
    mode: &'a str,
    drive_side: &'a str,
}

impl Rules<'_> {
    /// OSM: an unsuffixed `*:lanes` is the way's forward direction; `:backward` is the reverse edge.
    fn pick<'t>(&self, t: &'t Tags, base: &str, is_rev: bool) -> Option<&'t str> {
        if is_rev {
            tag(t, &format!("{base}:backward"))
        } else {
            or(tag(t, &format!("{base}:forward")), tag(t, base))
        }
    }
    /// An on-road bike lane on the right of this edge's direction: (drawn width, tagged width).
    fn bike_extra(&self, t: &Tags, is_rev: bool) -> Option<(f64, Option<f64>)> {
        if self.mode != "driving" || self.drive_side != "right" || split(self.pick(t, "bicycle:lanes", is_rev)).iter().any(|x| x == "designated") {
            return None;
        }
        for key in [if is_rev { "cycleway:left" } else { "cycleway:right" }, "cycleway:both", "cycleway"] {
            if tag(t, key) == Some("lane") {
                let w = num(tag(t, &format!("{key}:width"))).filter(|v| *v != 0.0).or(num(tag(t, "cycleway:width")).filter(|v| *v != 0.0));
                return Some((w.unwrap_or(DEFAULT_BIKE_LANE_W), w));
            }
        }
        None
    }
    /// An on-road bike lane on the left of a one-way road's travel: GMNS lane -1.
    fn bike_left(&self, t: &Tags, is_rev: bool, oneway: bool) -> Option<(f64, Option<f64>)> {
        if self.mode != "driving" || self.drive_side != "right" || !oneway || is_rev || tag(t, "oneway:bicycle") == Some("no") {
            return None;
        }
        for key in ["cycleway:left", "cycleway:both"] {
            if tag(t, key) == Some("lane") && !matches!(tag(t, &format!("{key}:oneway")), Some("-1") | Some("no")) {
                let w = num(tag(t, &format!("{key}:width"))).filter(|v| *v != 0.0).or(num(tag(t, "cycleway:width")).filter(|v| *v != 0.0));
                return Some((w.unwrap_or(DEFAULT_BIKE_LANE_W), w));
            }
        }
        None
    }
    fn is_foot(&self, t: &Tags) -> bool {
        self.mode == "walking" && tag(t, "highway").is_some_and(|h| NON_MOTOR.contains(&h))
    }
    /// (turns, widths, the drawn width of each lane).
    fn lane_widths(&self, t: &Tags, lanes: Option<i64>, is_rev: bool, bus: bool, oneway: bool) -> (Vec<String>, Vec<String>, Vec<f64>) {
        let mut turns = split(self.pick(t, "turn:lanes", is_rev));
        let widths = split(self.pick(t, "width:lanes", is_rev));
        if bus {
            let w = nor(widths.first().and_then(|w| num(Some(w))), DEFAULT_LANE_W);
            turns.truncate(1);
            return (turns, widths, vec![w]);
        }
        if self.mode == "walking" {
            turns.clear();
        }
        let n = if self.mode == "walking" { 1 } else { (turns.len() as i64).max(match lanes { Some(l) if l != 0 => l, _ => 1 }).max(1) as usize };
        let (extra, left) = (self.bike_extra(t, is_rev), self.bike_left(t, is_rev, oneway));
        let dw = if self.is_foot(t) { DEFAULT_WALK_W } else { default_lane_w(tag(t, "highway")) };
        let mut w: Vec<f64> = left.iter().map(|l| l.0).collect();
        w.extend((0..n).map(|i| nor(widths.get(i).and_then(|x| num(Some(x))), dw)));
        w.extend(extra.iter().map(|e| e.0));
        (turns, widths, w)
    }
    /// `_rule_lanes`: [(lane_num, use, width, turn)] left to right.
    #[allow(clippy::too_many_arguments)]
    fn rule_lanes(&self, r: &LaneRow, one: bool, bus_of: &HashMap<i64, &str>, uses: &str) -> Vec<(i64, String, Option<f64>, Option<String>)> {
        let t = &r.tags;
        let bus = bus_of.get(&r.edge_id);
        let (turns, widths, w_each) = self.lane_widths(t, r.lanes, r.is_rev, bus.is_some(), one);
        let [bikes, psvs, buses] = ["bicycle:lanes", "psv:lanes", "bus:lanes"].map(|k| split(self.pick(t, k, r.is_rev)));
        let n = w_each.len();
        let extra = if bus.is_some() { None } else { self.bike_extra(t, r.is_rev) };
        let left = if bus.is_some() { None } else { self.bike_left(t, r.is_rev, one) };
        let k0 = left.is_some() as i64;
        // Python's list[m] with a negative m counts from the end
        let at = |v: &Vec<String>, m: i64| -> Option<String> {
            let i = if m < 0 { v.len() as i64 + m } else { m };
            (m < v.len() as i64 && i >= 0).then(|| v[i as usize].clone())
        };
        let yes = |x: Option<String>| x.is_some_and(|v| v == "designated" || v == "yes");
        let mut out = Vec::new();
        for i in 0..n {
            let m = i as i64 - k0;
            let lane_num = if left.is_some() && i == 0 { -1 } else { m + 1 };
            let u = if let Some(b) = bus {
                b.to_string()
            } else if (extra.is_some() && i == n - 1) || (left.is_some() && i == 0) || yes(at(&bikes, m)) {
                "bike".to_string()
            } else if yes(at(&psvs, m)) || yes(at(&buses, m)) {
                "bus".to_string()
            } else {
                uses.to_string()
            };
            let mut width = if extra.is_some() && i == n - 1 {
                extra.unwrap().1
            } else if left.is_some() && i == 0 {
                left.unwrap().1
            } else {
                at(&widths, m).and_then(|w| num(Some(&w)))
            };
            if width.is_none() && u == "bike" {
                width = Some(DEFAULT_BIKE_W);
            } else if width.is_none() && u != "walk" {
                width = Some(w_each[i]);
            }
            let turn = if r.way_end && m >= 0 && (m as usize) < turns.len() && !["", "none"].contains(&turns[m as usize].as_str()) {
                Some(turns[m as usize].clone())
            } else {
                None
            };
            out.push((lane_num, u, width, turn));
        }
        out
    }
}

/// `_build_lane_curb`: the lanes of each link (from `driving.lane_profile` in driving, else from the tags) and `curb_seg` from the
/// parking tags. The lane geometry is not ported yet: `geom` is NULL.
fn build_lane_curb(c: &Ctx, sch: &str, mode: &str, uses: &str, has_raw: bool, drive_side: &str) -> Result<()> {
    let raw_join = if has_raw { "LEFT JOIN s.raw.ways w ON w.osm_id = abs(e.osm_id)" } else { "" };
    let tagcol = if has_raw { "w.tags" } else { "NULL::MAP(VARCHAR, VARCHAR)" };
    let endcol = if has_raw { "e.target IS NOT DISTINCT FROM (CASE WHEN e.is_reverse THEN w.refs[1] ELSE w.refs[-1] END)" } else { "true" };
    let oneway_sel = if c.src_cols(mode, "edges")?.contains("oneway") { "e.oneway" } else { "false" };
    let mut rows: Vec<LaneRow> = Vec::new();
    for src in ["_edges_car", "_edges_bus"] {
        let sql = format!(
            "SELECT e.edge_id, e.is_reverse::BOOLEAN, COALESCE(li.lanes, e.lanes)::BIGINT, e.length_m::DOUBLE, e.source, ({oneway_sel})::BOOLEAN AS oneway,
             {tagcol} AS tags, e.osm_id, e._bus, e._bike, ({endcol})::BOOLEAN AS way_end
             FROM {src} e {raw_join} LEFT JOIN _lanes_inf li ON li.edge_id = e.edge_id"
        );
        let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
        let got = st
            .query_map([], |r| {
                Ok(LaneRow {
                    edge_id: r.get(0)?,
                    is_rev: r.get::<_, Option<bool>>(1)?.unwrap_or(false),
                    lanes: r.get(2)?,
                    length_m: r.get(3)?,
                    source: r.get(4)?,
                    oneway: r.get::<_, Option<bool>>(5)?.unwrap_or(false),
                    tags: tags_of(r.get(6)?),
                    osm_id: r.get(7)?,
                    bus: r.get::<_, Option<bool>>(8)?.unwrap_or(false),
                    bike: r.get(9)?,
                    way_end: r.get::<_, Option<bool>>(10)?.unwrap_or(false),
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows.extend(got);
    }
    // driving: every edge's lanes from driving.lane_profile, decided once in the build
    type Lane = (i64, String, Option<f64>, Option<String>);
    let mut prof: HashMap<i64, Vec<Lane>> = HashMap::new();
    if mode == "driving" && c.exists("driving", "lane_profile")? {
        let sql = "SELECT edge_id, lane_num::BIGINT, use, width_m::DOUBLE, turn FROM s.driving.lane_profile ORDER BY edge_id, lane_num";
        let mut st = c.con.prepare(sql).map_err(|e| format!("{e}\n{sql}"))?;
        let it = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))).map_err(|e| e.to_string())?;
        for x in it {
            let (e, l) = x.map_err(|e| e.to_string())?;
            prof.entry(e).or_default().push(l);
        }
        let missing: Vec<i64> = rows.iter().map(|r| r.edge_id).filter(|e| !prof.contains_key(e)).collect();
        if !missing.is_empty() {
            return Err(format!(
                "driving.lane_profile has no lanes for {} edge(s), e.g. {:?}: build the db again",
                missing.len(),
                &missing[..missing.len().min(5)]
            ));
        }
    }
    let bus_of: HashMap<i64, &str> = rows.iter().filter(|r| r.bus).map(|r| (r.edge_id, if r.bike == Some(true) { "bus,bike" } else { "bus" })).collect();
    // a one-way street with a bus lane the other way is two-way on the road
    let bus_ways: HashSet<i64> = rows.iter().filter(|r| r.bus && r.oneway).map(|r| r.osm_id).collect();
    let car_ways: HashSet<i64> = rows.iter().filter(|r| !r.bus).map(|r| r.osm_id).collect();
    let contra: HashSet<i64> = bus_ways.intersection(&car_ways).copied().collect();
    let rules = Rules { mode, drive_side };
    c.exec("CREATE OR REPLACE TEMP TABLE _lane_rows(lane_id VARCHAR, link_id BIGINT, lane_num BIGINT, allowed_uses VARCHAR, width DOUBLE, turn VARCHAR)")?;
    c.exec("CREATE OR REPLACE TEMP TABLE _curb_rows(curb_seg_id VARCHAR, link_id BIGINT, ref_node_id BIGINT, start_lr DOUBLE, end_lr DOUBLE, regulation VARCHAR)")?;
    {
        let mut lanes = c.con.appender("_lane_rows").map_err(|e| e.to_string())?;
        let mut curbs = c.con.appender("_curb_rows").map_err(|e| e.to_string())?;
        for r in &rows {
            let one = r.oneway && !contra.contains(&r.osm_id);
            let these: Vec<Lane> = match prof.get(&r.edge_id) {
                Some(p) if !prof.is_empty() => p.clone(),
                _ => rules.rule_lanes(r, one, &bus_of, uses),
            };
            for (n, u, w, t) in these {
                lanes.append_row(duckdb::params![format!("{}_{n}", r.edge_id), r.edge_id, n, u, w, t]).map_err(|e| e.to_string())?;
            }
            for side in ["left", "right", "both"] {
                let val = or(tag(&r.tags, &format!("parking:{side}")), tag(&r.tags, &format!("parking:lane:{side}")));
                if let Some(v) = val.filter(|v| !v.is_empty() && !["no", "separate", "none"].contains(v)) {
                    curbs.append_row(duckdb::params![format!("{}_{side}", r.edge_id), r.edge_id, r.source, 0.0, r.length_m, v]).map_err(|e| e.to_string())?;
                }
            }
        }
        lanes.flush().map_err(|e| e.to_string())?;
        curbs.flush().map_err(|e| e.to_string())?;
    }
    c.exec(&format!(
        "CREATE TABLE {sch}.lane AS SELECT lane_id, link_id, lane_num, allowed_uses, NULL::VARCHAR AS r_barrier,
           NULL::VARCHAR AS l_barrier, width, turn, NULL::GEOMETRY AS geom FROM _lane_rows;
         CREATE TABLE {sch}.curb_seg AS SELECT curb_seg_id, link_id, ref_node_id, start_lr, end_lr, regulation, NULL::INTEGER AS width
           FROM _curb_rows;
         DROP TABLE _lane_rows; DROP TABLE _curb_rows"
    ))
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
