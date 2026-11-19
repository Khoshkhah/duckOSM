//! The mesoscopic and microscopic networks of a GMNS database: the port of `to_meso` and `to_micro` of src/duckosm/gmns.py,
//! with the same SQL. `meso_<mode>`: a section link for each macro link and a connector for each movement. `micro_<mode>`:
//! each lane cut into cells, the lane changes, and a turn connector for each lane pair of a movement.

use crate::Result;
use duckdb::Connection;
use std::path::Path;

fn len_m(g: &str) -> String {
    format!("ST_Length_Spheroid(ST_FlipCoordinates({g}))")
}

fn bezier(x0: &str, y0: &str, cx0: &str, cy0: &str, cx1: &str, cy1: &str, x1: &str, y1: &str) -> String {
    let ts = (0..=10).map(|i| format!("{:.4}", i as f64 / 10.0)).collect::<Vec<_>>().join(", ");
    let bx = format!("pow(1-u,3)*{x0} + 3*pow(1-u,2)*u*{cx0} + 3*(1-u)*pow(u,2)*{cx1} + pow(u,3)*{x1}");
    let by = format!("pow(1-u,3)*{y0} + 3*pow(1-u,2)*u*{cy0} + 3*(1-u)*pow(u,2)*{cy1} + pow(u,3)*{y1}");
    format!("ST_MakeLine(list_transform([{ts}], lambda u: ST_Point({bx}, {by})))")
}

fn open(db: &Path) -> Result<Connection> {
    let con = Connection::open(db).map_err(|e| format!("{}: {e}", db.display()))?;
    con.execute_batch(&format!("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false; SET memory_limit = '{}';", crate::MEMORY_LIMIT))
        .map_err(|e| e.to_string())?;
    Ok(con)
}

fn exec(con: &Connection, sql: &str) -> Result<()> {
    con.execute_batch(sql).map_err(|e| format!("{e}\n{sql}"))
}

fn count(con: &Connection, sql: &str) -> Result<i64> {
    con.query_row(sql, [], |r| r.get(0)).map_err(|e| format!("{e}\n{sql}"))
}

/// `_meso_modes_present` and the check of the asked modes (default: driving).
fn modes_of(con: &Connection, modes: &[String]) -> Result<Vec<String>> {
    let sql = "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE schema_name LIKE 'gmns_%' AND table_name = 'link' ORDER BY schema_name";
    let mut st = con.prepare(sql).map_err(|e| e.to_string())?;
    let have: Vec<String> = st.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).map(|s| s.replacen("gmns_", "", 1)).collect();
    let chosen: Vec<String> = if modes.is_empty() { vec!["driving".into()] } else { modes.to_vec() };
    let unknown: Vec<&String> = chosen.iter().filter(|m| !have.contains(m)).collect();
    if !unknown.is_empty() {
        return Err(format!("mode(s) {unknown:?} have no gmns_<mode> schema (present: {have:?})"));
    }
    Ok(chosen)
}

/// The counts of a meso network: (mode, meso nodes, section links, connector links).
pub type MesoCounts = (String, i64, i64, i64);

/// `to_meso`: a `meso_<mode>` schema for each mode (default: driving), the sections inset `trim_m` metres from each end.
pub fn to_meso(gmns_db: &Path, modes: &[String], trim_m: f64) -> Result<Vec<MesoCounts>> {
    let con = open(gmns_db)?;
    let mut out = Vec::new();
    for mode in modes_of(&con, modes)? {
        let (g, ms) = (format!("gmns_{mode}"), format!("meso_{mode}"));
        exec(&con, &format!("DROP SCHEMA IF EXISTS {ms} CASCADE; CREATE SCHEMA {ms}"))?;
        // two meso nodes for each macro link, upstream and downstream
        exec(
            &con,
            &format!(
                "CREATE TABLE {ms}.meso_node AS
      WITH L AS (SELECT link_id, from_node_id, to_node_id, geom,
                        LEAST(0.35, {trim_m:?} / GREATEST(\"length\", 0.1)) AS tf FROM {g}.link)
      SELECT link_id::VARCHAR || 'u' AS node_id,
             ST_X(ST_LineInterpolatePoint(geom, tf)) AS x_coord,
             ST_Y(ST_LineInterpolatePoint(geom, tf)) AS y_coord,
             from_node_id AS macro_node_id, link_id AS macro_link_id,
             ST_LineInterpolatePoint(geom, tf) AS geom
      FROM L
      UNION ALL
      SELECT link_id::VARCHAR || 'd',
             ST_X(ST_LineInterpolatePoint(geom, 1 - tf)),
             ST_Y(ST_LineInterpolatePoint(geom, 1 - tf)),
             to_node_id, link_id, ST_LineInterpolatePoint(geom, 1 - tf)
      FROM L"
            ),
        )?;
        let sub = len_m("ST_LineSubstring(geom, tf, 1 - tf)");
        exec(
            &con,
            &format!(
                "CREATE TABLE {ms}.meso_link AS
      WITH L AS (SELECT link_id, geom, \"length\" AS len, lanes, free_speed, facility_type, allowed_uses,
                        LEAST(0.35, {trim_m:?} / GREATEST(\"length\", 0.1)) AS tf FROM {g}.link)
      SELECT 'M' || link_id::VARCHAR AS link_id,
             link_id::VARCHAR || 'u' AS from_node_id, link_id::VARCHAR || 'd' AS to_node_id,
             1 AS dir_flag, {sub}::DOUBLE AS length, lanes::INTEGER AS lanes,
             NULL::DOUBLE AS capacity, free_speed, facility_type, allowed_uses,
             ST_AsText(ST_LineSubstring(geom, tf, 1 - tf)) AS geometry,
             ST_LineSubstring(geom, tf, 1 - tf) AS geom,
             'normal' AS meso_type, link_id AS macro_link_id, NULL::VARCHAR AS movement_id,
             NULL::VARCHAR AS mvmt_txt_id, NULL::INTEGER AS start_ib_lane, NULL::INTEGER AS end_ib_lane,
             NULL::VARCHAR AS ctrl_type
      FROM L"
            ),
        )?;
        // the connectors: a smooth curve from the end of the inbound section to the start of the outbound one
        let k = "111320.0";
        let path = bezier("x0", "y0", "cx0", "cy0", "cx1", "cy1", "x1", "y1");
        let plen = len_m("path");
        exec(
            &con,
            &format!(
                "INSERT INTO {ms}.meso_link
      (link_id, from_node_id, to_node_id, dir_flag, length, lanes, capacity, free_speed,
       facility_type, allowed_uses, geometry, geom, meso_type, macro_link_id, movement_id,
       mvmt_txt_id, start_ib_lane, end_ib_lane, ctrl_type)
      WITH c1 AS (
        SELECT m.*, il.free_speed AS fs,
               ST_X(ST_EndPoint(ib.geom)) AS x0, ST_Y(ST_EndPoint(ib.geom)) AS y0,
               ST_X(ST_StartPoint(ob.geom)) AS x1, ST_Y(ST_StartPoint(ob.geom)) AS y1,
               ST_X(ST_PointN(ib.geom, (ST_NPoints(ib.geom) - 1)::INTEGER)) AS px,
               ST_Y(ST_PointN(ib.geom, (ST_NPoints(ib.geom) - 1)::INTEGER)) AS py,
               ST_X(ST_PointN(ob.geom, 2)) AS nx, ST_Y(ST_PointN(ob.geom, 2)) AS ny
        FROM {g}.movement m
        JOIN {ms}.meso_link ib ON ib.link_id = 'M' || m.ib_link_id::VARCHAR AND ib.meso_type = 'normal'
        JOIN {ms}.meso_link ob ON ob.link_id = 'M' || m.ob_link_id::VARCHAR AND ob.meso_type = 'normal'
        JOIN {g}.link il ON il.link_id = m.ib_link_id
      ), c2 AS (
        SELECT *, cos(radians(y0)) AS kx,
               (x0 - px) * cos(radians(y0)) AS dx0, (y0 - py) AS dy0,
               (nx - x1) * cos(radians(y1)) AS dx1, (ny - y1) AS dy1,
               sqrt(pow((x1 - x0) * cos(radians(y0)) * {k}, 2) + pow((y1 - y0) * {k}, 2)) AS chord
        FROM c1
      ), cp AS (
        SELECT *, x0 + dx0 / GREATEST(sqrt(dx0*dx0 + dy0*dy0), 1e-12) * 0.4 * chord / ({k} * kx) AS cx0,
                  y0 + dy0 / GREATEST(sqrt(dx0*dx0 + dy0*dy0), 1e-12) * 0.4 * chord / {k} AS cy0,
                  x1 - dx1 / GREATEST(sqrt(dx1*dx1 + dy1*dy1), 1e-12) * 0.4 * chord / ({k} * kx) AS cx1,
                  y1 - dy1 / GREATEST(sqrt(dx1*dx1 + dy1*dy1), 1e-12) * 0.4 * chord / {k} AS cy1
        FROM c2
      ), t AS (
        SELECT *, {path} AS path FROM cp
      )
      SELECT 'X' || ib_link_id::VARCHAR || '-' || ob_link_id::VARCHAR,
             ib_link_id::VARCHAR || 'd', ob_link_id::VARCHAR || 'u', 1,
             {plen}::DOUBLE,
             COALESCE(end_ib_lane - start_ib_lane + 1, 1)::INTEGER, NULL::DOUBLE, fs,
             'connector', allowed_uses,
             ST_AsText(path), path,
             'movement', NULL::BIGINT, mvmt_id,
             CASE type WHEN 'left' THEN 'L' WHEN 'right' THEN 'R' WHEN 'uturn' THEN 'U'
                       ELSE 'T' END,
             start_ib_lane, end_ib_lane, ctrl_type
      FROM t"
            ),
        )?;
        out.push((
            mode,
            count(&con, &format!("SELECT count(*) FROM {ms}.meso_node"))?,
            count(&con, &format!("SELECT count(*) FROM {ms}.meso_link WHERE meso_type='normal'"))?,
            count(&con, &format!("SELECT count(*) FROM {ms}.meso_link WHERE meso_type='movement'"))?,
        ));
    }
    Ok(out)
}

const MICRO_COLS: &str = "link_id, from_node_id, to_node_id, dir_flag, length, lanes, width, free_speed, \
facility_type, allowed_uses, geometry, geom, macro_link_id, meso_link_id, lane_no, \
cell_type, mvmt_txt_id, ctrl_type";

/// The counts of a micro network: (mode, micro nodes, cell links, lane-change links, turn links).
pub type MicroCounts = (String, i64, i64, i64, i64);

/// `to_micro`: a `micro_<mode>` schema for each mode (default: driving), each lane cut into cells of `cell_length_m`.
pub fn to_micro(gmns_db: &Path, modes: &[String], cell_length_m: f64) -> Result<Vec<MicroCounts>> {
    let con = open(gmns_db)?;
    let mut out = Vec::new();
    for mode in modes_of(&con, modes)? {
        let (g, mc) = (format!("gmns_{mode}"), format!("micro_{mode}"));
        let cl = format!("{cell_length_m:?}");
        // the cells along each lane's line where SUMO ends it (geom_cut, driving), else its line
        let has_cut = count(&con, &format!("SELECT count(*) FROM duckdb_columns() WHERE schema_name = '{g}' AND table_name = 'lane' AND column_name = 'geom_cut'"))? > 0;
        let lg = if has_cut { "COALESCE(geom_cut, geom)" } else { "geom" };
        exec(&con, &format!("DROP SCHEMA IF EXISTS {mc} CASCADE; CREATE SCHEMA {mc}"))?;
        exec(
            &con,
            &format!(
                "CREATE TABLE {mc}.micro_node AS
          WITH L AS (SELECT lane_id, link_id, lane_num, geom,
                            GREATEST(1, CEIL({} / {cl}))::BIGINT AS nc
                     FROM (SELECT lane_id, link_id, lane_num, ST_RemoveRepeatedPoints({lg}) AS geom
                           FROM {g}.lane WHERE {lg} IS NOT NULL) WHERE ST_NPoints(geom) >= 2)
          SELECT lane_id, k AS cell_k, lane_id || '@' || k AS node_id,
                 ST_X(ST_LineInterpolatePoint(geom, k::DOUBLE / nc)) AS x_coord,
                 ST_Y(ST_LineInterpolatePoint(geom, k::DOUBLE / nc)) AS y_coord,
                 link_id AS macro_link_id, lane_num AS lane_no,
                 ST_LineInterpolatePoint(geom, k::DOUBLE / nc) AS geom
          FROM L, range(nc + 1) AS s(k)",
                len_m("geom")
            ),
        )?;
        let sub = "ST_LineSubstring(geom, k::DOUBLE/nc, LEAST(1.0,(k+1.0)/nc))";
        exec(
            &con,
            &format!(
                "CREATE TABLE {mc}.micro_link AS
          WITH L AS (SELECT la.lane_id, la.link_id, la.lane_num, la.allowed_uses, la.width, la.geom,
                            lk.free_speed, lk.facility_type,
                            GREATEST(1, CEIL({} / {cl}))::BIGINT AS nc
                     FROM (SELECT * REPLACE (ST_RemoveRepeatedPoints({lg}) AS geom) FROM {g}.lane
                           WHERE {lg} IS NOT NULL) la JOIN {g}.link lk ON lk.link_id = la.link_id
                     WHERE ST_NPoints(la.geom) >= 2)
          SELECT 'C' || lane_id || '#' || k AS link_id, lane_id || '@' || k AS from_node_id,
                 lane_id || '@' || (k + 1) AS to_node_id, 1 AS dir_flag,
                 {}::DOUBLE AS length,
                 1 AS lanes, width, free_speed, facility_type, allowed_uses,
                 ST_AsText({sub}) AS geometry,
                 {sub} AS geom,
                 link_id AS macro_link_id, 'M' || link_id AS meso_link_id, lane_num AS lane_no,
                 'normal' AS cell_type, NULL::VARCHAR AS mvmt_txt_id, NULL::VARCHAR AS ctrl_type
          FROM L, range(nc) AS s(k)",
                len_m("la.geom"),
                len_m(sub)
            ),
        )?;
        // lane changes: adjacent lanes of a link, one cell forward (both directions)
        exec(
            &con,
            &format!(
                "INSERT INTO {mc}.micro_link ({MICRO_COLS})
          SELECT 'H' || a.node_id || '-' || b.node_id, a.node_id, b.node_id, 1,
                 {}, 1, NULL::DOUBLE, NULL::DOUBLE, 'lane_change',
                 NULL::VARCHAR, ST_AsText(ST_MakeLine(a.geom, b.geom)), ST_MakeLine(a.geom, b.geom),
                 a.macro_link_id, 'M' || a.macro_link_id, NULL::INTEGER, 'lane_change',
                 NULL::VARCHAR, NULL::VARCHAR
          FROM {mc}.micro_node a JOIN {mc}.micro_node b
            ON a.macro_link_id = b.macro_link_id AND abs(a.lane_no - b.lane_no) = 1
               AND b.cell_k = a.cell_k + 1",
                len_m("ST_MakeLine(a.geom, b.geom)")
            ),
        )?;
        // turns: inbound lane end -> outbound lane start, along the lane connector where there is one
        let has_lc = count(&con, &format!("SELECT count(*) FROM duckdb_tables() WHERE schema_name = '{g}' AND table_name = 'lane_connector'"))? > 0;
        let path = if has_lc { "COALESCE(lc.geom, ST_MakeLine(ie.pt, oe.pt))" } else { "ST_MakeLine(ie.pt, oe.pt)" };
        let lc_join = if has_lc {
            format!("LEFT JOIN {g}.lane_connector lc ON lc.mvmt_id = m.mvmt_id AND lc.from_lane_id = ie.lane_id AND lc.to_lane_id = oe.lane_id")
        } else {
            String::new()
        };
        exec(
            &con,
            &format!(
                "INSERT INTO {mc}.micro_link ({MICRO_COLS})
          WITH ie AS (SELECT macro_link_id AS lk, lane_no, arg_max(node_id, cell_k) AS node,
                             arg_max(geom, cell_k) AS pt, arg_max(lane_id, cell_k) AS lane_id
                      FROM {mc}.micro_node GROUP BY macro_link_id, lane_no),
               oe AS (SELECT macro_link_id AS lk, lane_no, arg_min(node_id, cell_k) AS node,
                             arg_min(geom, cell_k) AS pt, arg_min(lane_id, cell_k) AS lane_id
                      FROM {mc}.micro_node GROUP BY macro_link_id, lane_no),
               t AS (SELECT m.*, il.free_speed, ie.node AS inode, oe.node AS onode, {path} AS path
                     FROM {g}.movement m
                     JOIN ie ON ie.lk = m.ib_link_id
                       AND ie.lane_no BETWEEN COALESCE(m.start_ib_lane, 1) AND COALESCE(m.end_ib_lane, m.start_ib_lane, 1)
                     JOIN oe ON oe.lk = m.ob_link_id
                       AND oe.lane_no = COALESCE(m.start_ob_lane, 1) + ie.lane_no - COALESCE(m.start_ib_lane, 1)
                     JOIN {g}.link il ON il.link_id = m.ib_link_id
                     {lc_join})
          SELECT 'X' || inode || '-' || onode, inode, onode, 1, {}, 1, NULL::DOUBLE, free_speed,
                 'connector', allowed_uses, ST_AsText(path), path,
                 NULL::BIGINT, 'M' || ib_link_id, NULL::INTEGER, 'movement', mvmt_code, ctrl_type
          FROM t",
                len_m("path")
            ),
        )?;
        out.push((
            mode,
            count(&con, &format!("SELECT count(*) FROM {mc}.micro_node"))?,
            count(&con, &format!("SELECT count(*) FROM {mc}.micro_link WHERE cell_type='normal'"))?,
            count(&con, &format!("SELECT count(*) FROM {mc}.micro_link WHERE cell_type='lane_change'"))?,
            count(&con, &format!("SELECT count(*) FROM {mc}.micro_link WHERE cell_type='movement'"))?,
        ));
    }
    Ok(out)
}
