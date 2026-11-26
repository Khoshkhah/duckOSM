//! `building_parts::refresh` on a small database (docs/design/building_parts.md): the link, the heights in metres, `support`
//! and the open height range of the walls of the parts.

use duckdb::Connection;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> String {
    format!("ST_GeomFromText('POLYGON(({x0} {y0}, {x1} {y0}, {x1} {y1}, {x0} {y1}, {x0} {y0}))')")
}

#[test]
fn links_heights_support_and_open_walls() {
    let db = std::env::temp_dir().join(format!("duckosm-building-parts-{}.duckdb", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let con = Connection::open(&db).unwrap();
    // building 1: a left part of 4 levels, a right part of 8 levels, and a part of levels 4 to 6 on the left part;
    // building 2: one part from level 1 to 2 with nothing under it (a passage)
    con.execute_batch(&format!(
        "INSTALL spatial; LOAD spatial; CREATE SCHEMA features;
         CREATE TABLE features.buildings AS SELECT * FROM (VALUES
             (1, 'way', MAP {{'building': 'yes', 'height': '30 m'}}, {b1}),
             (2, 'way', MAP {{'building': 'yes'}}, {b2})) t(osm_id, osm_type, tags, geom);
         CREATE TABLE features.building_parts AS SELECT * FROM (VALUES
             (10, 'way', MAP {{'building:part': 'yes', 'building:levels': '4'}}, {left}),
             (11, 'way', MAP {{'building:part': 'yes', 'building:levels': '8'}}, {right}),
             (12, 'way', MAP {{'building:part': 'yes', 'building:levels': '6', 'building:min_level': '4'}}, {left}),
             (13, 'way', MAP {{'building:part': 'yes', 'height': '20 ft', 'min_height': '3'}}, {b2})) t(osm_id, osm_type, tags, geom);
         CREATE TABLE main.building_faces AS
             SELECT 1::BIGINT AS building_id, 'way' AS building_type, 'front' AS face_type, ST_GeomFromText('LINESTRING(0 0, 0.001 0)') AS geometry;",
        b1 = rect(0.0, 0.0, 0.001, 0.001),
        b2 = rect(0.002, 0.0, 0.003, 0.001),
        left = rect(0.0, 0.0, 0.0005, 0.001),
        right = rect(0.0005, 0.0, 0.001, 0.001),
    ))
    .unwrap();
    drop(con);

    let s = duckosm_duckdb::building_parts::refresh(&db).unwrap();
    assert_eq!((s.parts, s.no_building, s.stacked, s.floating, s.faces), (4, 0, 1, 1, Some(16)));

    let con = Connection::open(&db).unwrap();
    con.execute_batch("LOAD spatial").unwrap();
    let q = |sql: &str| -> Vec<String> {
        let mut st = con.prepare(sql).unwrap();
        st.query_map([], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(
        q("SELECT concat_ws('|', osm_id, building_id, height_m, min_height_m, height_source, support) FROM features.building_parts ORDER BY osm_id"),
        ["10|1|12.8|0.0|levels|ground", "11|1|25.6|0.0|levels|ground", "12|1|19.2|12.8|levels|stacked", "13|2|6.1|3.0|height|floating"]
    );
    assert_eq!(q("SELECT concat_ws('|', osm_id, height_m, height_source, has_parts) FROM features.buildings ORDER BY osm_id"), ["1|30.0|height|true", "2|6.0|default|true"]);
    // the wall between the left and the right part: parts 10 and 12 close it up to 19.2 m, so part 11 is open from 19.2 m
    // to 25.6 m; the walls of parts 10 and 12 there are inside part 11
    let mid = "ST_Equals(geometry, ST_GeomFromText('LINESTRING(0.0005 0, 0.0005 0.001)')) OR ST_Equals(geometry, ST_GeomFromText('LINESTRING(0.0005 0.001, 0.0005 0)'))";
    assert_eq!(
        q(&format!("SELECT concat_ws('|', part_id, position, coalesce(open_from_m::VARCHAR, '-'), coalesce(open_to_m::VARCHAR, '-')) FROM main.building_part_faces WHERE {mid} ORDER BY part_id")),
        ["10|step|-|-", "11|step|19.2|25.6", "12|step|-|-"]
    );
    // the bottom walls lie on the front wall of building 1
    assert_eq!(
        q("SELECT DISTINCT concat_ws('|', part_id, position, type) FROM main.building_part_faces WHERE ST_Y(ST_StartPoint(geometry)) = 0 AND ST_Y(ST_EndPoint(geometry)) = 0 AND building_id = 1 ORDER BY 1"),
        ["10|outline|front", "11|outline|front", "12|outline|front"]
    );
    // a second run gives the same rows
    drop(con);
    let again = duckosm_duckdb::building_parts::refresh(&db).unwrap();
    assert_eq!((again.parts, again.stacked, again.floating, again.faces), (4, 1, 1, Some(16)));
    let _ = std::fs::remove_file(&db);
}
