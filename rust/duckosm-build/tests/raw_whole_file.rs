//! A build of a whole file (no boundary) with the format duckdb has duckOSM's `raw.*`, made with `read_osm`
//! (docs/design/streaming_load.md). Its own test file: the test of `files.rs` counts the temporary folders of its process.

use duckosm_build::{build, Params};
use std::path::PathBuf;

#[test]
fn whole_file_build_has_raw() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-whole-file-raw");
    let p = Params {
        geoparquet: false,
        duckdb: true,
        built_at: Some(0),
        threads: Some(2),
        memory_limit: Some("1GB".into()),
        ..Params::default()
    };
    let sum = build(&data.join("monaco.osm.pbf"), &out, &p).unwrap();
    // the rows of ST_READOSM on Monaco (duckosm-duckdb/tests/read_osm.rs compares the rows themselves)
    assert_eq!(sum.raw, Some((34291, 5650, 336)));
    let db = sum.files.iter().find(|(k, _)| k == "duckdb").unwrap().1.clone();
    let con = duckdb::Connection::open(db).unwrap();
    for (t, n) in [("nodes", 34291i64), ("ways", 5650), ("relations", 336)] {
        let got: i64 = con
            .query_row(&format!("SELECT count(*) FROM raw.{t}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(got, n, "raw.{t}");
    }
}
