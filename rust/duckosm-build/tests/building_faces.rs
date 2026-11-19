//! `main.building_faces` on Monaco (docs/osm-editing.md, M4): the numbers of the method, the same file on two runs, and a rule of
//! `extra.building_faces` that changes the type of one wall and keeps the computed type.

use duckosm_build::{build, Params};
use duckosm_core::filter::Mode;
use std::path::{Path, PathBuf};

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
}

fn run(name: &str, fixes: Option<PathBuf>) -> (Option<(usize, usize)>, PathBuf) {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let p = Params {
        modes: vec![Mode::Driving, Mode::Walking],
        geoparquet: true,
        duckdb: false,
        raw: true,
        building_faces: true,
        osm_fixes: fixes,
        boundary: Some(data().join("monaco.geojson")),
        built_at: Some(0),
        ..Params::default()
    };
    let sum = build(&data().join("monaco_sample.osm.pbf"), &out, &p).unwrap();
    (sum.building_faces, out.join("building_faces_main.parquet"))
}

fn wall(file: &Path, building: i64, a: i64, b: i64) -> (String, String, bool) {
    let c = duckdb::Connection::open_in_memory().unwrap();
    c.query_row(
        &format!(
            "SELECT face_type, computed_type, edited FROM read_parquet('{}') WHERE building_id = {building} AND from_node = {a} AND to_node = {b}",
            file.display()
        ),
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

#[test]
fn monaco_faces() {
    // the numbers change when a threshold of building_faces.rs changes: that is on purpose
    let (n, one) = run("faces-1", None);
    assert_eq!(
        n,
        Some((10909, 1262)),
        "walls and buildings (all 1,231 building ways and 31 multipolygon buildings of the cut have walls)"
    );
    let (_, two) = run("faces-2", None);
    assert_eq!(
        std::fs::read(&one).unwrap(),
        std::fs::read(&two).unwrap(),
        "the same bytes on two runs"
    );
    assert_eq!(
        wall(&one, 48807846, 619263371, 619263365),
        ("front".into(), "front".into(), false)
    );

    // a rule: the type of this wall is `side`, with the nodes in the other order
    let yaml = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("faces-rule.yaml");
    std::fs::write(&yaml, "extra:\n  building_faces:\n    - {building: 48807846, from_node: 619263365, to_node: 619263371, type: side}\n").unwrap();
    let (n, edited) = run("faces-3", Some(yaml));
    assert_eq!(n, Some((10909, 1262)), "a rule changes no row count");
    assert_eq!(
        wall(&edited, 48807846, 619263371, 619263365),
        ("side".into(), "front".into(), true)
    );
}

#[test]
fn faces_in_the_duckdb_file() {
    // the .duckdb output alone (no GeoParquet): the walls are main.building_faces in the file, as each other table
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("faces-db");
    let p = Params {
        modes: vec![Mode::Driving, Mode::Walking],
        geoparquet: false,
        duckdb: true,
        building_faces: true,
        boundary: Some(data().join("monaco.geojson")),
        name: "monaco".into(),
        built_at: Some(0),
        ..Params::default()
    };
    let sum = build(&data().join("monaco_sample.osm.pbf"), &out, &p).unwrap();
    assert_eq!(sum.building_faces, Some((10909, 1262)));
    let c = duckdb::Connection::open(out.join("monaco.duckdb")).unwrap();
    let n: i64 = c.query_row("SELECT count(*) FROM main.building_faces", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 10909, "every wall is in main.building_faces of the .duckdb file");
}

/// The SQL of duckOSM's Python build (`rust_engine.BUILDING_ENTRANCES_SQL`), on `raw.*`: the old table.
const OLD_ENTRANCES_SQL: &str = "
CREATE TABLE old_entrances AS
WITH e AS (
  SELECT osm_id, tags, lon, lat FROM raw.nodes WHERE map_extract(tags, 'entrance')[1] IS NOT NULL OR map_extract(tags, 'door')[1] IS NOT NULL
), j AS (
  SELECT node_id, min(building_id) AS building_id FROM (
    SELECT w.osm_id AS building_id, unnest(w.refs) AS node_id FROM raw.ways w WHERE map_extract(w.tags, 'building')[1] IS NOT NULL)
  WHERE node_id IN (SELECT osm_id FROM e) GROUP BY node_id)
SELECT e.osm_id, 'node' AS osm_type, coalesce(map_extract(e.tags, 'entrance')[1], 'door') AS kind, map_extract(e.tags, 'door')[1] AS door,
       map_extract(e.tags, 'access')[1] AS access, map_extract(e.tags, 'addr:housenumber')[1] AS housenumber, map_extract(e.tags, 'addr:street')[1] AS street,
       map_extract(e.tags, 'level')[1] AS level, map_extract(e.tags, 'wheelchair')[1] AS wheelchair, map_extract(e.tags, 'name')[1] AS name,
       j.building_id, e.tags, ST_Point(e.lon, e.lat) AS geom
FROM e LEFT JOIN j ON j.node_id = e.osm_id";

/// The count, the doors on a building, and an md5 of all rows of a table of entrances.
fn entrances_of(c: &duckdb::Connection, table: &str) -> (i64, i64, String) {
    c.query_row(
        &format!(
            "SELECT count(*), count(building_id), md5(string_agg(x, ',' ORDER BY x)) FROM (SELECT building_id, concat_ws('|', osm_id, osm_type, kind, \
             door, access, housenumber, street, level, wheelchair, name, building_id, tags::VARCHAR, hex(ST_AsWKB(geom))) x FROM {table})"
        ),
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

#[test]
fn entrances_equal_the_old_sql() {
    // main.building_entrances comes from the nodes that the walls step reads: it needs no raw.*. With --raw, the old SQL on raw.*
    // gives the same rows; without --raw, the table is the same.
    let make = |name: &str, raw: bool| {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
        let p = Params {
            modes: vec![Mode::Driving, Mode::Walking],
            geoparquet: false,
            duckdb: true,
            raw,
            building_faces: true,
            boundary: Some(data().join("monaco.geojson")),
            name: "monaco".into(),
            built_at: Some(0),
            ..Params::default()
        };
        let sum = build(&data().join("monaco_sample.osm.pbf"), &out, &p).unwrap();
        assert_eq!(sum.building_entrances, Some(110));
        let c = duckdb::Connection::open(out.join("monaco.duckdb")).unwrap();
        c.execute_batch("INSTALL spatial; LOAD spatial;").unwrap();
        c
    };
    let with_raw = make("entrances-raw", true);
    with_raw.execute_batch(OLD_ENTRANCES_SQL).unwrap();
    let old = entrances_of(&with_raw, "old_entrances");
    assert_eq!((old.0, old.1), (110, 83), "the old table: 110 doors, 83 on a building outline (docs/rust/tables.md)");
    assert_eq!(entrances_of(&with_raw, "main.building_entrances"), old, "the rows of the old SQL");
    let no_raw = make("entrances-no-raw", false);
    let has_raw: i64 = no_raw
        .query_row("SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'raw'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(has_raw, 0, "a build without --raw has no raw.*");
    assert_eq!(entrances_of(&no_raw, "main.building_entrances"), old, "the same rows without raw.*");
}
