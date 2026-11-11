//! `read_osm` gives the same `raw.*` as `ST_READOSM` of DuckDB's spatial extension (docs/design/streaming_load.md):
//! the same column types and the same rows, on Monaco. The test installs the spatial extension (a download the first
//! time), as the test of the base-map layers does.

use duckdb::Connection;
use std::path::PathBuf;

const TABLES: [&str; 3] = ["nodes", "ways", "relations"];

#[test]
fn read_osm_equals_st_readosm() {
    let pbf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let path = pbf.display().to_string().replace('\'', "''");
    let con = Connection::open_in_memory().unwrap();
    con.execute_batch("INSTALL spatial; LOAD spatial; CREATE SCHEMA st;").unwrap();
    let o = format!("ST_READOSM('{path}')");
    con.execute_batch(&format!(
        "CREATE TABLE st.nodes AS SELECT id AS osm_id, lat, lon, tags FROM {o} WHERE kind = 'node' AND lat IS NOT NULL AND lon IS NOT NULL;
         CREATE TABLE st.ways AS SELECT id AS osm_id, tags, refs FROM {o} WHERE kind = 'way' AND len(refs) >= 2;
         CREATE TABLE st.relations AS SELECT id AS osm_id, tags, refs, ref_roles, ref_types FROM {o} WHERE kind = 'relation';"
    ))
    .unwrap();
    duckosm_duckdb::read_osm::register(&con).unwrap();
    con.execute_batch(&duckosm_duckdb::read_osm::raw_sql(&path)).unwrap();
    let rows = |sql: &str| -> i64 { con.query_row(sql, [], |r| r.get(0)).unwrap() };
    let types = |schema: &str, t: &str| -> Vec<(String, String)> {
        let mut q = con
            .prepare(&format!(
                "SELECT column_name, data_type FROM information_schema.columns
                 WHERE table_schema = '{schema}' AND table_name = '{t}' ORDER BY ordinal_position"
            ))
            .unwrap();
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for t in TABLES {
        assert_eq!(types("raw", t), types("st", t), "the columns of raw.{t}");
        let n = rows(&format!("SELECT count(*) FROM st.{t}"));
        assert!(n > 0, "st.{t} is empty");
        assert_eq!(rows(&format!("SELECT count(*) FROM raw.{t}")), n, "the rows of raw.{t}");
        for (a, b) in [("raw", "st"), ("st", "raw")] {
            let diff = rows(&format!("SELECT count(*) FROM (SELECT * FROM {a}.{t} EXCEPT ALL SELECT * FROM {b}.{t})"));
            assert_eq!(diff, 0, "{diff} rows of {a}.{t} are not in {b}.{t}");
        }
    }
}

/// A file that does not exist, and a kind that does not exist, give errors, not a panic.
#[test]
fn read_osm_errors() {
    let con = Connection::open_in_memory().unwrap();
    duckosm_duckdb::read_osm::register(&con).unwrap();
    assert!(con.execute_batch("SELECT * FROM read_osm('/no/such/file.osm.pbf', 'node')").is_err());
    let pbf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let e = con
        .execute_batch(&format!("SELECT * FROM read_osm('{}', 'area')", pbf.display()))
        .unwrap_err()
        .to_string();
    assert!(e.contains("node, way or relation"), "{e}");
}
