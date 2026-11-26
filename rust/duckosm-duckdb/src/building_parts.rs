//! The parts of the buildings in 3D (docs/design/building_parts.md): the link of each part to its building, the heights in
//! metres of the buildings and the parts, the base of each part (`support`), and the walls of the parts
//! (`main.building_part_faces`). The build runs [`refresh`] after the features and the walls; `duckosm overture-buildings`
//! runs it again after it changes the tags.

use duckdb::Connection;
use std::path::Path;

/// The height of one level, in metres (the rule of duckOSM's pages: `heightOf` in web/fixes_editor/fixes_panel.js).
pub const LEVEL_M: f64 = 3.2;
/// The height of a building or a part that has no height and no levels, in metres.
pub const DEFAULT_M: f64 = 6.0;

/// The counts of [`refresh`].
#[derive(Debug, Default)]
pub struct Summary {
    pub parts: i64,
    /// The parts with no building.
    pub no_building: i64,
    pub stacked: i64,
    pub floating: i64,
    /// The rows of `main.building_part_faces` (None: no `main.building_faces`, so no walls of the parts).
    pub faces: Option<i64>,
}

fn has(con: &Connection, schema: &str, table: &str) -> Result<bool, String> {
    con.query_row(
        "SELECT count(*) > 0 FROM duckdb_tables() WHERE database_name = current_database() AND schema_name = ? AND table_name = ?",
        [schema, table],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

/// The SQL of the heights of one table (`features.buildings` or `features.building_parts`).
fn heights_sql(table: &str) -> String {
    format!(
        "ALTER TABLE {table} ADD COLUMN IF NOT EXISTS height_m DOUBLE;
         ALTER TABLE {table} ADD COLUMN IF NOT EXISTS min_height_m DOUBLE;
         ALTER TABLE {table} ADD COLUMN IF NOT EXISTS height_source VARCHAR;
         UPDATE {table} SET
             height_m = round(coalesce(_bp_metres(tags['height']), _bp_levels(tags['building:levels']), {DEFAULT_M}), 2),
             min_height_m = round(coalesce(_bp_metres(tags['min_height']), _bp_levels(tags['building:min_level']), 0), 2),
             height_source = CASE WHEN _bp_metres(tags['height']) IS NOT NULL THEN 'height'
                                  WHEN _bp_levels(tags['building:levels']) IS NOT NULL THEN 'levels' ELSE 'default' END;"
    )
}

/// Add the links, the heights, `support` and `main.building_part_faces` to the database `db` (each one again when it is
/// there). Nothing when the database has no `features.buildings`.
pub fn refresh(db: &Path) -> Result<Summary, String> {
    let con = Connection::open(db).map_err(|e| e.to_string())?;
    let e = |e: duckdb::Error| format!("building parts: {e}");
    if !has(&con, "features", "buildings")? {
        return Ok(Summary::default());
    }
    con.execute_batch(&format!(
        "LOAD spatial;
         -- a length in metres (also in feet), and a number of levels in metres
         CREATE OR REPLACE TEMP MACRO _bp_metres(v) AS
             TRY_CAST(regexp_extract(lower(trim(v)), '^(\\d+(\\.\\d+)?) *(m|meter|meters|metre|metres|ft|feet|'')?$', 1) AS DOUBLE)
             * CASE WHEN regexp_matches(lower(trim(v)), '^\\d+(\\.\\d+)? *(ft|feet|'')$') THEN 0.3048 ELSE 1 END;
         CREATE OR REPLACE TEMP MACRO _bp_levels(v) AS TRY_CAST(regexp_extract(trim(v), '^(\\d+(\\.\\d+)?)', 1) AS DOUBLE) * {LEVEL_M};
         {}",
        heights_sql("features.buildings")
    ))
    .map_err(e)?;
    con.execute_batch("ALTER TABLE features.buildings ADD COLUMN IF NOT EXISTS has_parts BOOLEAN; UPDATE features.buildings SET has_parts = false;")
        .map_err(e)?;
    if !has(&con, "features", "building_parts")? {
        return Ok(Summary::default());
    }
    con.execute_batch(&heights_sql("features.building_parts")).map_err(e)?;

    // 1. the link: an OSM relation type=building (outline and parts), else the smallest building that holds a point of the part
    let rel = if has(&con, "raw", "relations")? {
        "SELECT p.ref AS osm_id, p.typ AS osm_type, o.ref AS b_id, o.typ AS b_type
         FROM (SELECT osm_id AS rel, unnest(refs) AS ref, unnest(ref_roles) AS role, unnest(ref_types)::VARCHAR AS typ
               FROM raw.relations WHERE tags['type'] = 'building') p
         JOIN (SELECT osm_id AS rel, unnest(refs) AS ref, unnest(ref_roles) AS role, unnest(ref_types)::VARCHAR AS typ
               FROM raw.relations WHERE tags['type'] = 'building') o ON o.rel = p.rel AND o.role = 'outline'
         WHERE p.role = 'part'"
    } else {
        "SELECT NULL::BIGINT AS osm_id, NULL::VARCHAR AS osm_type, NULL::BIGINT AS b_id, NULL::VARCHAR AS b_type WHERE false"
    };
    con.execute_batch(&format!(
        "ALTER TABLE features.building_parts ADD COLUMN IF NOT EXISTS building_id BIGINT;
         ALTER TABLE features.building_parts ADD COLUMN IF NOT EXISTS building_type VARCHAR;
         ALTER TABLE features.building_parts ADD COLUMN IF NOT EXISTS support VARCHAR;
         CREATE OR REPLACE TEMP TABLE _bp_rel AS SELECT osm_id, osm_type, min(b_id) AS b_id, min(b_type) AS b_type FROM ({rel}) GROUP BY ALL;
         CREATE OR REPLACE TEMP TABLE _bp_in AS
             SELECT p.osm_id, p.osm_type, arg_min(b.osm_id, ST_Area(b.geom)) AS b_id, arg_min(b.osm_type, ST_Area(b.geom)) AS b_type
             FROM features.building_parts p JOIN features.buildings b ON ST_Contains(b.geom, ST_PointOnSurface(p.geom))
             GROUP BY p.osm_id, p.osm_type;
         UPDATE features.building_parts p SET building_id = NULL, building_type = NULL;
         UPDATE features.building_parts p SET building_id = l.b_id, building_type = l.b_type FROM _bp_in l
             WHERE l.osm_id = p.osm_id AND l.osm_type = p.osm_type;
         UPDATE features.building_parts p SET building_id = l.b_id, building_type = l.b_type FROM _bp_rel l
             WHERE l.osm_id = p.osm_id AND l.osm_type = p.osm_type;
         UPDATE features.buildings b SET has_parts = true FROM (SELECT DISTINCT building_id, building_type FROM features.building_parts) p
             WHERE p.building_id = b.osm_id AND p.building_type = b.osm_type;"
    ))
    .map_err(e)?;

    // 3. the base: on the ground, on other parts of the building (they cover 90 % of it and reach its base), or floating
    con.execute_batch(
        "CREATE OR REPLACE TEMP TABLE _bp_cover AS
             SELECT p.osm_id, p.osm_type,
                    ST_Area(ST_Intersection(any_value(p.geom), ST_Union_Agg(ST_MakeValid(q.geom)))) / nullif(ST_Area(any_value(p.geom)), 0) AS frac
             FROM features.building_parts p JOIN features.building_parts q
               ON q.building_id = p.building_id AND q.building_type = p.building_type AND NOT (q.osm_id = p.osm_id AND q.osm_type = p.osm_type)
              AND q.min_height_m < p.min_height_m AND q.height_m >= p.min_height_m - 0.5 AND ST_Intersects(p.geom, q.geom)
             WHERE p.min_height_m > 0
             GROUP BY p.osm_id, p.osm_type;
         UPDATE features.building_parts p SET support = CASE
             WHEN p.min_height_m <= 0 THEN 'ground'
             WHEN (SELECT frac FROM _bp_cover c WHERE c.osm_id = p.osm_id AND c.osm_type = p.osm_type) >= 0.9 THEN 'stacked'
             ELSE 'floating' END;",
    )
    .map_err(e)?;

    // 4. the walls of the parts: each segment of the outer rings, its open height range, and its place (outline or step)
    let faces = if has(&con, "main", "building_faces")? {
        let refs = if has(&con, "raw", "ways")? { "SELECT osm_id, refs FROM raw.ways" } else { "SELECT NULL::BIGINT AS osm_id, NULL::BIGINT[] AS refs WHERE false" };
        con.execute_batch(&format!(
            "CREATE OR REPLACE TEMP TABLE _bp_ring AS
                 SELECT p.osm_id, p.osm_type, p.building_id, p.building_type, p.min_height_m, p.height_m, coalesce(d.path[1], 1) AS poly,
                        ST_ExteriorRing(d.geom) AS ring, len(ST_Dump(p.geom)) AS n_polys
                 FROM features.building_parts p, LATERAL (SELECT unnest(ST_Dump(p.geom), recursive := true)) d;
             CREATE OR REPLACE TEMP TABLE _bp_seg AS
                 SELECT r.*, i - 1 AS face_index, ST_PointN(r.ring, i::INTEGER) AS a, ST_PointN(r.ring, (i + 1)::INTEGER) AS b
                 FROM _bp_ring r, LATERAL generate_series(1, ST_NPoints(r.ring) - 1) AS s(i);
             CREATE OR REPLACE TEMP TABLE _bp_seg2 AS
                 SELECT s.*, ST_MakeLine(a, b) AS geometry,
                        CASE WHEN s.osm_type = 'way' AND s.n_polys = 1 AND len(w.refs) = ST_NPoints(s.ring) THEN w.refs[s.face_index + 1] END AS from_node,
                        CASE WHEN s.osm_type = 'way' AND s.n_polys = 1 AND len(w.refs) = ST_NPoints(s.ring) THEN w.refs[s.face_index + 2] END AS to_node,
                        least(round(ST_X(a), 7)::VARCHAR || ',' || round(ST_Y(a), 7)::VARCHAR, round(ST_X(b), 7)::VARCHAR || ',' || round(ST_Y(b), 7)::VARCHAR)
                        || ';' ||
                        greatest(round(ST_X(a), 7)::VARCHAR || ',' || round(ST_Y(a), 7)::VARCHAR, round(ST_X(b), 7)::VARCHAR || ',' || round(ST_Y(b), 7)::VARCHAR) AS seg_key
                 FROM _bp_seg s LEFT JOIN ({refs}) w ON s.osm_type = 'way' AND w.osm_id = s.osm_id
                 WHERE NOT ST_Equals(a, b);
             -- the other parts of the same building on the same segment: each one closes the wall over its height range
             CREATE OR REPLACE TEMP TABLE _bp_nb AS
                 SELECT s.osm_id, s.osm_type, s.poly, s.face_index, o.min_height_m AS lo, o.height_m AS hi
                 FROM _bp_seg2 s JOIN _bp_seg2 o
                   ON o.seg_key = s.seg_key AND o.building_id = s.building_id AND o.building_type = s.building_type
                  AND NOT (o.osm_id = s.osm_id AND o.osm_type = s.osm_type);
             CREATE OR REPLACE TEMP TABLE _bp_cov AS
                 SELECT DISTINCT s.osm_id, s.osm_type, s.poly, s.face_index, s.min_height_m AS covered_to
                 FROM _bp_seg2 s JOIN _bp_nb USING (osm_id, osm_type, poly, face_index);
             -- the wall of the building at this place: within 0.5 m of the middle of the segment
             CREATE OR REPLACE TEMP TABLE _bp_out AS
                 SELECT s.osm_id, s.osm_type, s.poly, s.face_index, arg_min(f.face_type, ST_Distance(ST_Centroid(s.geometry), f.geometry)) AS face_type
                 FROM _bp_seg2 s JOIN main.building_faces f
                   ON f.building_id = s.building_id AND f.building_type = s.building_type
                  AND ST_DWithin(ST_Centroid(s.geometry), f.geometry, 0.5 / 111320.0)
                 GROUP BY ALL;"
        ))
        .map_err(e)?;
        // the closed range grows from the base of the wall through the neighbours that start in it (a stack of parts)
        for _ in 0..20 {
            let n = con
                .execute(
                    "UPDATE _bp_cov c SET covered_to = x.hi FROM (
                         SELECT c.osm_id, c.osm_type, c.poly, c.face_index, max(n.hi) AS hi FROM _bp_cov c JOIN _bp_nb n
                           ON n.osm_id = c.osm_id AND n.osm_type = c.osm_type AND n.poly = c.poly AND n.face_index = c.face_index
                          AND n.lo <= c.covered_to + 0.5 AND n.hi > c.covered_to + 0.001
                         GROUP BY ALL) x
                     WHERE x.osm_id = c.osm_id AND x.osm_type = c.osm_type AND x.poly = c.poly AND x.face_index = c.face_index",
                    [],
                )
                .map_err(e)?;
            if n == 0 {
                break;
            }
        }
        con.execute_batch(
            "CREATE OR REPLACE TABLE main.building_part_faces AS
                 SELECT s.osm_id AS part_id, s.osm_type AS part_type, s.building_id, s.building_type, s.poly AS polygon, s.face_index,
                        s.from_node, s.to_node, s.min_height_m AS z_bottom_m, s.height_m AS z_top_m,
                        CASE WHEN greatest(s.min_height_m, coalesce(c.covered_to, s.min_height_m)) < s.height_m - 0.01
                             THEN greatest(s.min_height_m, coalesce(c.covered_to, s.min_height_m)) END AS open_from_m,
                        CASE WHEN greatest(s.min_height_m, coalesce(c.covered_to, s.min_height_m)) < s.height_m - 0.01 THEN s.height_m END AS open_to_m,
                        CASE WHEN o.face_type IS NOT NULL THEN 'outline' ELSE 'step' END AS position,
                        coalesce(o.face_type, 'step') AS type,
                        ST_Length_Spheroid(ST_FlipCoordinates(s.geometry))::FLOAT AS length_m,
                        s.geometry
                 FROM _bp_seg2 s
                 LEFT JOIN _bp_cov c ON c.osm_id = s.osm_id AND c.osm_type = s.osm_type AND c.poly = s.poly AND c.face_index = s.face_index
                 LEFT JOIN _bp_out o ON o.osm_id = s.osm_id AND o.osm_type = s.osm_type AND o.poly = s.poly AND o.face_index = s.face_index
                 ORDER BY s.osm_type, s.osm_id, s.poly, s.face_index;
             COMMENT ON TABLE main.building_part_faces IS 'The walls of the building parts: one row for each segment of the outer ring of a part, with its heights and the range that is open to the air (docs/design/building_parts.md).';",
        )
        .map_err(e)?;
        Some(con.query_row("SELECT count(*) FROM main.building_part_faces", [], |r| r.get(0)).map_err(e)?)
    } else {
        None
    };
    let (parts, no_building, stacked, floating): (i64, i64, i64, i64) = con
        .query_row(
            "SELECT count(*), count(*) FILTER (WHERE building_id IS NULL), count(*) FILTER (WHERE support = 'stacked'),
                    count(*) FILTER (WHERE support = 'floating') FROM features.building_parts",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(e)?;
    con.execute_batch("CHECKPOINT").map_err(e)?;
    Ok(Summary { parts, no_building, stacked, floating, faces })
}
