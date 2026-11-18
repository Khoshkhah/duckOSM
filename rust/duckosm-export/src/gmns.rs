//! The GMNS export (`duckosm gmns`), the port of src/duckosm/gmns.py (docs/design/all_in_rust.md, group C).
//!
//! The export writes a new DuckDB file with one `gmns_<mode>` schema for each mode, with the source db attached as `s`. The
//! steps are those of `to_gmns` in the same order, with the same SQL; the Python loops are Rust loops over the same rows.
//! Ported so far: the fixed tables (`config`, `use_definition`, `use_group`), `node`, the lanes that a one-way road takes
//! from the road it continues, `link`, `geometry`, and the roads that a footpath runs along (`link_along`, `link.along_*`,
//! `link.parent_link_id`), the lanes and curbs, and the movements (without their lanes).

use crate::gmns_lanes as lanes;
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
        build_movement(&c, &sch, mode, uses)?;
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
    wkt: Option<String>,
    target: i64,
    name: Option<String>,
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

/// `_bezier_line_sql`: a SQL expression of a cubic Bézier line sampled at 11 points.
fn bezier_line_sql(x0: &str, y0: &str, cx0: &str, cy0: &str, cx1: &str, cy1: &str, x1: &str, y1: &str) -> String {
    let ts = (0..=10).map(|i| format!("{:.4}", i as f64 / 10.0)).collect::<Vec<_>>().join(", ");
    let bx = format!("pow(1-u,3)*{x0} + 3*pow(1-u,2)*u*{cx0} + 3*(1-u)*pow(u,2)*{cx1} + pow(u,3)*{x1}");
    let by = format!("pow(1-u,3)*{y0} + 3*pow(1-u,2)*u*{cy0} + 3*(1-u)*pow(u,2)*{cy1} + pow(u,3)*{y1}");
    format!("ST_MakeLine(list_transform([{ts}], lambda u: ST_Point({bx}, {by})))")
}

/// `_window_angle`: the turn from a link into the next one in degrees (left +), from the heading of the outbound link over its
/// first 15 m and of the inbound link over its last 15 m. None when a link is shorter than half a metre.
fn window_angle(inbound: &str, outbound: &str) -> Option<f64> {
    let local = |w: &str| -> Vec<P> {
        let pts = shp::wkt_points(w);
        let (x0, y0) = pts[0];
        let kx = y0.to_radians().cos() * 111320.0;
        pts.iter().map(|(x, y)| ((x - x0) * kx, (y - y0) * 111320.0)).collect()
    };
    let heading = |l: &[P], at: f64, to: f64| {
        let (p, q) = (shp::interpolate(l, at), shp::interpolate(l, to));
        (q.1 - p.1).atan2(q.0 - p.0).to_degrees()
    };
    let (li, lo) = (local(inbound), local(outbound));
    let (ni, no) = (shp::length(&li), shp::length(&lo));
    if ni < 0.5 || no < 0.5 {
        return None;
    }
    Some(lanes::pymod(heading(&lo, 0.0, 15.0f64.min(no)) - heading(&li, (ni - 15.0).max(0.0), ni) + 180.0, 360.0) - 180.0)
}

/// `_fork_letters`: the turn letter of the `mvmt_code` of every `diverge` movement: the straightest branch keeps T, a branch
/// 8 degrees or more to its left is L, to its right R.
fn fork_letters(c: &Ctx, sch: &str, mv: &str) -> Result<()> {
    let sql = format!(
        "SELECT m.mvmt_id, m.node_id, m.mvmt_code, il.geometry, ol.geometry FROM {mv} m
        JOIN {sch}.link il ON il.link_id = m.ib_link_id JOIN {sch}.link ol ON ol.link_id = m.ob_link_id
        WHERE m.type = 'diverge' AND m.mvmt_code IS NOT NULL"
    );
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let rows: Vec<(String, i64, String, String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut by_node: Vec<(i64, Vec<(String, String)>)> = Vec::new();
    let mut ang: HashMap<String, f64> = HashMap::new();
    for (mid, node, code, gi, go) in rows {
        if let Some(a) = window_angle(&gi, &go) {
            ang.insert(mid.clone(), a);
            match by_node.iter_mut().find(|x| x.0 == node) {
                Some(x) => x.1.push((mid, code)),
                None => by_node.push((node, vec![(mid, code)])),
            }
        }
    }
    c.exec("CREATE OR REPLACE TEMP TABLE _fork_codes(mvmt_id VARCHAR, code VARCHAR)")?;
    let mut app = c.con.appender("_fork_codes").map_err(|e| e.to_string())?;
    for (_, ms) in &by_node {
        // the first of the least |angle|, as Python's min()
        let mut straight = ang[&ms[0].0];
        for (mid, _) in ms {
            if ang[mid].abs() < straight.abs() {
                straight = ang[mid];
            }
        }
        for (mid, code) in ms {
            let a = ang[mid];
            let letter = if (a - straight).abs() < 8.0 { "T" } else if a > straight { "L" } else { "R" };
            let head: String = code.chars().take(2).collect();
            app.append_row(duckdb::params![mid, format!("{head}{letter}")]).map_err(|e| e.to_string())?;
        }
    }
    app.flush().map_err(|e| e.to_string())?;
    drop(app);
    c.exec(&format!("UPDATE {mv} SET mvmt_code = f.code FROM _fork_codes f WHERE f.mvmt_id = {mv}.mvmt_id; DROP TABLE _fork_codes"))
}

/// `_fork_types`: the movement types `diverge` (at a fork) and `merge`, and the fork letters.
fn fork_types(c: &Ctx, sch: &str, mv: &str) -> Result<()> {
    c.exec(&format!(
        "UPDATE {mv} m SET type = 'diverge' WHERE type <> 'uturn' AND m.node_id IN (
        SELECT f.node_id FROM {mv} f
        WHERE (SELECT count(*) FROM {sch}.link k WHERE k.to_node_id = f.node_id) = 1 AND f.type <> 'uturn'
        GROUP BY f.node_id HAVING count(*) >= 2 AND max(abs(f._ang)) < 45.0)"
    ))?;
    fork_letters(c, sch, mv)?;
    c.exec(&format!(
        "UPDATE {mv} m SET type = 'merge' WHERE type <> 'uturn' AND m.ob_link_id IN (
        SELECT f.ob_link_id FROM {mv} f
        WHERE (SELECT count(*) FROM {sch}.link k WHERE k.from_node_id = f.node_id) = 1 AND f.type <> 'uturn'
        GROUP BY f.ob_link_id HAVING count(*) >= 2 AND max(abs(f._ang)) < 45.0)"
    ))
}

/// `_build_movement`: one row for each legal turn of `edge_graph` (the mode's own rows and the bus rows), its type from the
/// bearing change, its code and a Bézier line through the junction. The lanes come later.
fn build_movement(c: &Ctx, sch: &str, mode: &str, uses: &str) -> Result<()> {
    if !c.exists(mode, "edge_graph")? {
        return c.exec(&format!(
            "CREATE TABLE {sch}.movement(
          mvmt_id VARCHAR, node_id BIGINT, name VARCHAR, ib_link_id BIGINT, start_ib_lane INT,
          end_ib_lane INT, ob_link_id BIGINT, start_ob_lane INT, end_ob_lane INT, type VARCHAR,
          penalty DOUBLE, capacity DOUBLE, ctrl_type VARCHAR, mvmt_code VARCHAR,
          allowed_uses VARCHAR, geometry VARCHAR)"
        ));
    }
    let routed = match mode {
        "driving" => "car",
        "walking" => "walk",
        "cycling" => "bike",
        m => m,
    };
    let has_uses = c.src_cols(mode, "edge_graph")?.contains("uses");
    let own = format!("'{routed}'");
    let (o_uses, i_uses) = if has_uses { ("o.uses".to_string(), "i.uses".to_string()) } else { (own.clone(), own.clone()) };
    let gate = if has_uses { format!("WHERE uses IN ({own}, 'bus')") } else { String::new() };
    let perm = if c.exists(mode, "turn_permission")? {
        format!(
            "UNION ALL SELECT from_edge, to_edge, string_agg(DISTINCT u, ',' ORDER BY u), true FROM (
                  SELECT from_edge, to_edge, CASE WHEN v IN ('psv', 'bus') THEN 'bus' WHEN v = 'bicycle' THEN 'bike' END AS u
                  FROM (SELECT from_edge, to_edge, trim(unnest(string_split(vehicles, ';'))) AS v FROM s.{mode}.turn_permission WHERE allowed))
                WHERE u IS NOT NULL GROUP BY 1, 2"
        )
    } else {
        String::new()
    };
    let eg_uses = if has_uses { "uses".to_string() } else { own.clone() };
    let bez = bezier_line_sql("t.x0", "t.y0", "t.cx0", "t.cy0", "t.cx1", "t.cy1", "t.x1", "t.y1");
    c.exec(&format!(
        "CREATE TEMP TABLE _mv_all AS
      WITH mv AS (
        SELECT eg.from_edge AS ib, eg.to_edge AS ob, fe.target AS node_id, eg.uses = {own} AS _own,
               CASE WHEN eg._perm THEN eg.uses WHEN eg.uses = {own} THEN '{uses}'
                    WHEN (NOT fe._bus OR fe._bike) AND (NOT te._bus OR te._bike) THEN 'bus,bike' ELSE eg.uses END AS allowed_uses,
               fe.geometry AS ibg, te.geometry AS obg,
               ST_PointN(fe.geometry, ST_NPoints(fe.geometry)::INTEGER)       AS pe,
               ST_PointN(fe.geometry, ST_NPoints(fe.geometry)::INTEGER - 1)   AS pp,
               ST_PointN(te.geometry, 1) AS qs, ST_PointN(te.geometry, 2) AS qn
        FROM (SELECT from_edge, to_edge, {eg_uses} AS uses, false AS _perm FROM s.{mode}.edge_graph {gate} {perm}) eg
        JOIN _edges fe ON fe.edge_id = eg.from_edge
        JOIN _edges te ON te.edge_id = eg.to_edge
        WHERE NOT (te.osm_id = fe.osm_id AND te.source = fe.target AND te.target = fe.source
                   AND EXISTS (SELECT 1 FROM (SELECT * FROM s.{mode}.edge_graph {gate}) o
                               WHERE o.from_edge = eg.from_edge AND o.to_edge <> eg.to_edge
                                 AND (eg.uses <> {own} OR {o_uses} = {own}))
                   AND EXISTS (SELECT 1 FROM (SELECT * FROM s.{mode}.edge_graph {gate}) i
                               WHERE i.to_edge = eg.to_edge AND i.from_edge <> eg.from_edge
                                 AND (eg.uses <> {own} OR {i_uses} = {own})))
      ), b AS (
        SELECT ib, ob, node_id, _own, allowed_uses, ibg, obg,
          atan2(ST_Y(pe) - ST_Y(pp), (ST_X(pe) - ST_X(pp)) * cos(radians(ST_Y(pe)))) AS in_b,
          atan2(ST_Y(qn) - ST_Y(qs), (ST_X(qn) - ST_X(qs)) * cos(radians(ST_Y(qs)))) AS out_b
        FROM mv
      ), t AS (
        SELECT ib, ob, node_id, _own, allowed_uses, ibg, obg, ang,
          CASE WHEN abs(ang) >= 150 THEN 'uturn' WHEN abs(ang) < 30 THEN 'thru'
               WHEN ang >= 30 THEN 'left' ELSE 'right' END AS type,
          ((90 - degrees(in_b)) - floor((90 - degrees(in_b)) / 360) * 360) AS hdg
        FROM (SELECT *, ((degrees(out_b - in_b) + 180) - floor((degrees(out_b - in_b) + 180) / 360) * 360) - 180 AS ang FROM b)
      ), pts AS (
        SELECT t.*,
          ST_X(ST_LineInterpolatePoint(ibg, 0.94)) AS x0, ST_Y(ST_LineInterpolatePoint(ibg, 0.94)) AS y0,
          ST_X(ST_LineInterpolatePoint(ibg, 0.78)) AS xb, ST_Y(ST_LineInterpolatePoint(ibg, 0.78)) AS yb,
          ST_X(ST_LineInterpolatePoint(obg, 0.06)) AS x1, ST_Y(ST_LineInterpolatePoint(obg, 0.06)) AS y1,
          ST_X(ST_LineInterpolatePoint(obg, 0.22)) AS xa, ST_Y(ST_LineInterpolatePoint(obg, 0.22)) AS ya
        FROM t
      ), cp AS (
        SELECT pts.*,
          x0 + (x0 - xb) / GREATEST(sqrt(pow(x0-xb,2)+pow(y0-yb,2)), 1e-9)
               * (0.3 * sqrt(pow(x1-x0,2)+pow(y1-y0,2))) AS cx0,
          y0 + (y0 - yb) / GREATEST(sqrt(pow(x0-xb,2)+pow(y0-yb,2)), 1e-9)
               * (0.3 * sqrt(pow(x1-x0,2)+pow(y1-y0,2))) AS cy0,
          x1 - (xa - x1) / GREATEST(sqrt(pow(xa-x1,2)+pow(ya-y1,2)), 1e-9)
               * (0.3 * sqrt(pow(x1-x0,2)+pow(y1-y0,2))) AS cx1,
          y1 - (ya - y1) / GREATEST(sqrt(pow(xa-x1,2)+pow(ya-y1,2)), 1e-9)
               * (0.3 * sqrt(pow(x1-x0,2)+pow(y1-y0,2))) AS cy1
        FROM pts
      )
      SELECT t.ib::VARCHAR || '-' || t.ob::VARCHAR AS mvmt_id, t.node_id, NULL::VARCHAR AS name,
             t.ib AS ib_link_id, NULL::INT AS start_ib_lane, NULL::INT AS end_ib_lane,
             t.ob AS ob_link_id, NULL::INT AS start_ob_lane, NULL::INT AS end_ob_lane, t.type,
             NULL::DOUBLE AS penalty, NULL::DOUBLE AS capacity,
             CASE WHEN sig.osm_id IS NOT NULL THEN 'signal' END AS ctrl_type,
             CASE WHEN t.type = 'uturn' THEN NULL
                  ELSE (CASE WHEN t.hdg < 45 OR t.hdg >= 315 THEN 'NB' WHEN t.hdg < 135 THEN 'EB'
                             WHEN t.hdg < 225 THEN 'SB' ELSE 'WB' END)
                       || (CASE t.type WHEN 'left' THEN 'L' WHEN 'right' THEN 'R' ELSE 'T' END)
             END AS mvmt_code,
             t.allowed_uses,
             ST_AsText({bez}) AS geometry,
             t.ang AS _ang, t._own
      FROM cp t
      LEFT JOIN _sig sig ON sig.osm_id = t.node_id"
    ))?;
    c.exec(&format!("CREATE TABLE {sch}.movement AS SELECT * EXCLUDE (_own) FROM _mv_all WHERE _own"))?;
    c.exec("CREATE TEMP TABLE _mv_bus AS SELECT * EXCLUDE (_own) FROM _mv_all WHERE node_id IN (SELECT node_id FROM _mv_all WHERE NOT _own)")?;
    c.exec("CREATE TEMP TABLE _bus_mv_ids AS SELECT mvmt_id FROM _mv_all WHERE NOT _own")?;
    c.exec("DROP TABLE _mv_all")?;
    let n_bus = c.count("SELECT count(*) FROM _bus_mv_ids")?;
    fork_types(c, sch, &format!("{sch}.movement"))?;
    if n_bus > 0 {
        fork_types(c, sch, "_mv_bus")?;
        eprintln!("GMNS[{mode}]: {n_bus} movements into or out of bus-only links");
    }
    c.exec(&format!("ALTER TABLE {sch}.movement DROP COLUMN IF EXISTS _ang"))?;
    c.exec(&format!("INSERT INTO {sch}.movement SELECT * EXCLUDE (_ang) FROM _mv_bus WHERE mvmt_id IN (SELECT mvmt_id FROM _bus_mv_ids)"))?;
    c.exec("DROP TABLE _mv_bus; DROP TABLE _bus_mv_ids")
}

/// The geometry of the lanes, as `_build_lane_curb` places them: the runs of pieces, the paired carriageways, the tapers at a
/// joint and the carriageways that leave a two-way road in OSM's V.
struct LaneGeo {
    side_sign: f64,
    info: HashMap<i64, lanes::Info>,
    w_of: HashMap<i64, Vec<f64>>,
    opp_total: HashMap<i64, f64>,
    runs: Vec<Vec<i64>>,
    wh: HashMap<i64, (usize, usize)>,
    merged: HashMap<usize, String>,
    gaps: HashMap<i64, f64>,
    run_profiles: HashMap<usize, Vec<(f64, f64)>>,
    run_place: Vec<Option<f64>>,
    taper: HashMap<i64, [f64; 4]>,
    vfix: HashMap<i64, (i64, bool)>,
    run_geoms: HashMap<i64, Vec<Vec<P>>>,
    wkt_of: HashMap<i64, Option<String>>,
}

/// The points of a `LINESTRING (...)` text, as Python's `_w.loads(wkt).coords`.
fn pts_of(w: &str) -> Vec<P> {
    shp::wkt_points(w)
}

impl LaneGeo {
    fn new(c: &Ctx, rows: &[LaneRow], prof: &HashMap<i64, Vec<(i64, String, Option<f64>, Option<String>)>>, rules: &Rules, contra: &HashSet<i64>, bus_of: &HashMap<i64, &str>) -> Result<LaneGeo> {
        let side_sign = if rules.drive_side == "right" { -1.0 } else { 1.0 };
        let order: Vec<i64> = rows.iter().map(|r| r.edge_id).collect();
        let mut info: HashMap<i64, lanes::Info> = HashMap::new();
        let (mut w_of, mut place_of) = (HashMap::new(), HashMap::new());
        let tags_of: HashMap<i64, &Tags> = rows.iter().map(|r| (r.edge_id, &r.tags)).collect();
        for r in rows {
            let oneway = r.oneway && !contra.contains(&r.osm_id);
            let w_each: Vec<f64> = if !prof.is_empty() {
                prof[&r.edge_id].iter().map(|x| x.2.unwrap_or(0.0)).collect()
            } else {
                rules.lane_widths(&r.tags, r.lanes, r.is_rev, bus_of.contains_key(&r.edge_id), oneway).2
            };
            let foot = rules.is_foot(&r.tags);
            let place = if oneway || foot || side_sign < 0.0 { lanes::placement(rules.pick(&r.tags, "placement", r.is_rev), &w_each) } else { None };
            place_of.insert(r.edge_id, place);
            let (h0, h1) = r.wkt.as_deref().map(|w| lanes::headings(&pts_of(w))).unwrap_or((0.0, 0.0));
            info.insert(
                r.edge_id,
                lanes::Info {
                    osm_id: r.osm_id,
                    is_rev: r.is_rev,
                    source: r.source,
                    target: r.target,
                    oneway: oneway || foot,
                    widths: w_each.clone(),
                    name: r.name.clone(),
                    roundabout: tag(&r.tags, "junction") == Some("roundabout"),
                    h0,
                    h1,
                    opp: -1.0,
                },
            );
            w_of.insert(r.edge_id, w_each);
        }
        // a two-way road's line is the middle of both directions: the other direction's total width
        let mut by_ends: HashMap<(i64, i64, i64), i64> = HashMap::new();
        for e in &order {
            let v = &info[e];
            by_ends.insert((v.osm_id, v.source, v.target), *e);
        }
        let mut opp_total = HashMap::new();
        for e in &order {
            let v = &info[e];
            if let Some(o) = by_ends.get(&(v.osm_id, v.target, v.source)) {
                opp_total.insert(*e, w_of[o].iter().sum::<f64>());
            }
        }
        for e in &order {
            let t = shp::py_round(opp_total.get(e).copied().unwrap_or(-1.0), 2);
            info.get_mut(e).unwrap().opp = t;
        }
        let (runs, wh, closed) = lanes::chain_runs(&order, &info);
        let wkt_of: HashMap<i64, Option<String>> = rows.iter().map(|r| (r.edge_id, r.wkt.clone())).collect();
        let mut merged: HashMap<usize, String> = HashMap::new();
        'runs: for (i, run) in runs.iter().enumerate() {
            let mut cs: Vec<String> = Vec::new();
            for e in run {
                let Some(w) = wkt_of[e].as_deref().filter(|w| !w.is_empty()) else { continue 'runs };
                let (Some(a), Some(b)) = (w.find('('), w.rfind(')')) else { continue 'runs };
                let c: Vec<String> = w[a + 1..b].split(',').map(|p| p.trim().to_string()).collect();
                if !cs.is_empty() && c[0] == *cs.last().unwrap() {
                    cs.extend_from_slice(&c[1..]);
                } else {
                    cs.extend(c);
                }
            }
            merged.insert(i, format!("LINESTRING ({})", cs.join(", ")));
        }
        // the paired carriageways, one level at a time
        let mut run_profiles = HashMap::new();
        let mut run_gaps: HashMap<usize, f64> = HashMap::new();
        let cand: Vec<(usize, Vec<P>, f64)> = runs
            .iter()
            .enumerate()
            .filter(|(i, run)| merged.contains_key(i) && info[&run[0]].oneway && !rules.is_foot(tags_of[&run[0]]))
            .map(|(i, run)| (i, pts_of(&merged[&i]), w_of[&run[0]].iter().sum::<f64>() / 2.0))
            .collect();
        let lv_of: HashMap<usize, i64> = cand
            .iter()
            .map(|(i, _, _)| {
                let t = tags_of[&runs[*i][0]];
                let g = |k: &str| t.get(k).cloned();
                (*i, level(&g("layer"), &g("bridge"), &g("tunnel")))
            })
            .collect();
        let mut lvs: Vec<i64> = lv_of.values().copied().collect::<HashSet<_>>().into_iter().collect();
        lvs.sort();
        for lv in lvs {
            let group: Vec<(usize, Vec<P>, f64)> = cand.iter().filter(|x| lv_of[&x.0] == lv).cloned().collect();
            run_gaps.extend(lanes::paired_gaps(&group, side_sign, &mut run_profiles));
        }
        let mut gaps = HashMap::new();
        for (i, run) in runs.iter().enumerate() {
            if let Some(g) = run_gaps.get(&i) {
                for e in run {
                    gaps.insert(*e, *g);
                }
            }
        }
        if !gaps.is_empty() {
            eprintln!("GMNS[{}]: {} one-way edges placed with their opposite carriageway", rules.mode, gaps.len());
        }
        let run_place: Vec<Option<f64>> = runs.iter().map(|run| run.iter().find_map(|e| place_of[e])).collect();
        let mut g = LaneGeo {
            side_sign,
            info,
            w_of,
            opp_total,
            runs,
            wh,
            merged,
            gaps,
            run_profiles,
            run_place,
            taper: HashMap::new(),
            vfix: HashMap::new(),
            run_geoms: HashMap::new(),
            wkt_of,
        };
        // a two-way road's centring changes over a taper at a plain joint
        let len_of: HashMap<i64, f64> = rows.iter().map(|r| (r.edge_id, r.length_m.unwrap_or(0.0))).collect();
        let (mut at_src, mut at_tgt): (HashMap<i64, Vec<i64>>, HashMap<i64, Vec<i64>>) = (HashMap::new(), HashMap::new());
        let mut roads_at: HashMap<i64, HashSet<i64>> = HashMap::new();
        for e in &order {
            let v = &g.info[e];
            at_src.entry(v.source).or_default().push(*e);
            at_tgt.entry(v.target).or_default().push(*e);
            roads_at.entry(v.source).or_default().insert(v.osm_id);
            roads_at.entry(v.target).or_default().insert(v.osm_id);
        }
        for e in &order {
            let Some(ce) = g.centring(*e) else { continue };
            let v = g.info[e].clone();
            let on: Vec<i64> = at_src
                .get(&v.target)
                .map(|fs| {
                    fs.iter()
                        .copied()
                        .filter(|f| *f != *e && g.info[f].target != v.source && g.centring(*f).is_some() && (lanes::pymod(g.info[f].h0 - v.h1 + 180.0, 360.0) - 180.0).abs() < 30.0)
                        .collect()
                })
                .unwrap_or_default();
            if on.len() != 1 || roads_at[&v.target] != HashSet::from([v.osm_id, g.info[&on[0]].osm_id]) {
                continue;
            }
            let f = on[0];
            let ds = g.centring(f).unwrap() - ce;
            if ds.abs() < 0.05 {
                continue;
            }
            let half_l = ds.abs() * lanes::LANE_TAPER_PER_M / 2.0;
            let te = g.taper.entry(*e).or_insert([0.0; 4]);
            te[2] = ds / 2.0;
            te[3] = half_l.min(len_of[e] / 2.0);
            let tf = g.taper.entry(f).or_insert([0.0; 4]);
            tf[0] = -ds / 2.0;
            tf[1] = half_l.min(len_of[&f] / 2.0);
        }
        // a carriageway leaves a two-way road in OSM's V: its lanes go straight on from the two-way road's lanes
        for e in &order {
            if g.centring(*e).is_none() {
                continue;
            }
            let v = g.info[e].clone();
            for (at_end, node) in [(true, v.target), (false, v.source)] {
                if roads_at[&node].len() != 3 {
                    continue;
                }
                let empty = Vec::new();
                let cands = if at_end { at_src.get(&node).unwrap_or(&empty) } else { at_tgt.get(&node).unwrap_or(&empty) };
                let on: Vec<i64> = cands
                    .iter()
                    .copied()
                    .filter(|f| {
                        let fi = &g.info[f];
                        let d = if at_end { fi.h0 - v.h1 } else { v.h0 - fi.h1 };
                        fi.oneway && fi.osm_id != v.osm_id && g.w_of[f].len() == g.w_of[e].len() && lanes::pymod((d + 180.0).abs(), 360.0) - 180.0 < 30.0
                    })
                    .collect();
                if on.len() != 1 {
                    continue;
                }
                let third: HashSet<i64> = roads_at[&node].iter().copied().filter(|x| *x != v.osm_id && *x != g.info[&on[0]].osm_id).collect();
                let (bl, al) = if at_end { (at_tgt.get(&node), at_src.get(&node)) } else { (at_src.get(&node), at_tgt.get(&node)) };
                let back: Vec<i64> = bl.unwrap_or(&empty).iter().copied().filter(|x| third.contains(&g.info[x].osm_id)).collect();
                let ahead = al.unwrap_or(&empty).iter().any(|x| third.contains(&g.info[x].osm_id));
                if back.len() != 1 || ahead || !g.info[&back[0]].oneway {
                    continue;
                }
                g.vfix.insert(on[0], (*e, at_end));
            }
        }
        // the lanes of each run of two or more pieces, from the run's one offset curve
        for (i, run) in g.runs.clone().iter().enumerate() {
            if run.len() > 1 && g.merged.contains_key(&i) {
                let pieces: Vec<Vec<P>> = run.iter().map(|e| pts_of(g.wkt_of[e].as_deref().unwrap_or(""))).collect();
                if let Some(per_piece) = lanes::run_lane_wkts(&pieces, &g.offsets(run[0]), closed.contains(&i))? {
                    for (e, l) in run.iter().zip(per_piece) {
                        g.run_geoms.insert(*e, l);
                    }
                }
            }
        }
        c.exec("CREATE OR REPLACE TEMP TABLE _lane_runs(link_id BIGINT, run_id BIGINT, seq BIGINT, n BIGINT, closed BOOLEAN)")?;
        let mut app = c.con.appender("_lane_runs").map_err(|e| e.to_string())?;
        for (e, (i, k)) in &g.wh {
            app.append_row(duckdb::params![e, *i as i64, *k as i64, g.runs[*i].len() as i64, closed.contains(i)]).map_err(|e| e.to_string())?;
        }
        app.flush().map_err(|e| e.to_string())?;
        Ok(g)
    }

    fn centring(&self, e: i64) -> Option<f64> {
        if self.info[&e].oneway || self.gaps.contains_key(&e) || self.run_place[self.wh[&e].0].is_some() {
            return None;
        }
        let total: f64 = self.w_of[&e].iter().sum();
        Some(-self.side_sign * (total - self.opp_total.get(&e).copied().unwrap_or(total)) / 2.0)
    }

    /// `offsets`: each lane's offset from the road line (left +).
    fn offsets(&self, e: i64) -> Vec<f64> {
        let w_each = &self.w_of[&e];
        let oneway = self.info[&e].oneway;
        let total: f64 = w_each.iter().sum();
        let half = total / 2.0;
        let gap = self.gaps.get(&e).copied();
        let place = self.run_place[self.wh[&e].0];
        let centre = -self.side_sign * (total - self.opp_total.get(&e).copied().unwrap_or(total)) / 2.0;
        let mut run = 0.0;
        let mut out = Vec::new();
        for &w in w_each {
            let d = if self.side_sign < 0.0 { run + w / 2.0 } else { total - run - w / 2.0 };
            out.push(if let Some(p) = place {
                p - (run + w / 2.0)
            } else if oneway && gap.is_some() {
                self.side_sign * (d - gap.unwrap() / 2.0)
            } else if oneway {
                half - (run + w / 2.0)
            } else {
                self.side_sign * d + centre
            });
            run += w;
        }
        out
    }

    /// The geometry of lane `i` of the edge `r`, as WKT.
    fn lane(&self, r: &LaneRow, i: usize) -> Result<Option<String>> {
        let e = r.edge_id;
        let Some(w) = r.wkt.as_deref() else { return Ok(None) };
        let line = pts_of(w);
        let geom: Vec<P> = match self.run_geoms.get(&e) {
            Some(g) => g[i].clone(),
            None => crate::lanelet2::offset_lonlat(&line, self.offsets(e)[i])?,
        };
        Ok(Some(match self.finish(r, i, geom.clone())? {
            Some(g) => g,
            None => lanes::wkt(&geom),
        }))
    }

    /// The moves after the offset: along a varying gap, over a taper, and straight on through OSM's V. None: no move.
    fn finish(&self, r: &LaneRow, i: usize, mut geom: Vec<P>) -> Result<Option<String>> {
        let e = r.edge_id;
        let ri = self.wh[&e].0;
        let mut moved = false;
        if let (Some(g0), Some(prof)) = (self.gaps.get(&e), self.run_profiles.get(&ri)) {
            if self.run_place[ri].is_none() {
                if let Some(g) = lanes::vary(&geom, &pts_of(&self.merged[&ri]), prof, *g0, self.side_sign) {
                    geom = g;
                    moved = true;
                }
            }
        }
        if let Some(t) = self.taper.get(&e) {
            if let Some(g) = lanes::taper(&geom, t[0], t[1], t[2], t[3]) {
                geom = g;
                moved = true;
            }
        }
        if let Some(&(two, leaves)) = self.vfix.get(&e) {
            let le = crate::lanelet2::offset_lonlat(&pts_of(self.wkt_of[&two].as_deref().unwrap_or("")), self.offsets(two)[i])?;
            let lf = pts_of(self.wkt_of[&e].as_deref().unwrap_or(""));
            let n = le.len();
            let (a, b) = if leaves { (le[n - 2], le[n - 1]) } else { (le[1], le[0]) };
            let kx = lanes::kx_of(b.1);
            let (tx, ty) = ((b.0 - a.0) * kx, b.1 - a.1);
            let nn = match tx.hypot(ty) {
                0.0 => 1.0,
                x => x,
            };
            let fc: Vec<P> = if leaves { lf.clone() } else { lf.iter().rev().copied().collect() };
            let seg = ((fc[1].0 - fc[0].0) * kx * 111320.0).hypot((fc[1].1 - fc[0].1) * 111320.0);
            if let Some(g) = lanes::straight(&geom, b, (tx / nn, ty / nn), seg.max(5.0).min(40.0), leaves) {
                geom = g;
                moved = true;
            }
        }
        Ok(moved.then(|| lanes::wkt(&geom)))
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
             {tagcol} AS tags, e.osm_id, e._bus, e._bike, ({endcol})::BOOLEAN AS way_end, ST_AsText(e.geometry), e.target, e.name
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
                    wkt: r.get(11)?,
                    target: r.get(12)?,
                    name: r.get(13)?,
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
    let geo = LaneGeo::new(c, &rows, &prof, &rules, &contra, &bus_of)?;
    c.exec("CREATE OR REPLACE TEMP TABLE _lane_rows(lane_id VARCHAR, link_id BIGINT, lane_num BIGINT, allowed_uses VARCHAR, width DOUBLE, turn VARCHAR, geom_wkt VARCHAR)")?;
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
            for (i, (n, u, w, t)) in these.into_iter().enumerate() {
                let g = geo.lane(r, i)?;
                lanes.append_row(duckdb::params![format!("{}_{n}", r.edge_id), r.edge_id, n, u, w, t, g]).map_err(|e| e.to_string())?;
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
           NULL::VARCHAR AS l_barrier, width, turn, ST_GeomFromText(geom_wkt) AS geom FROM _lane_rows;
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
