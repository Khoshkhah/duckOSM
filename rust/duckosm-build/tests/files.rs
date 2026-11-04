//! A build with the format duckdb only: the summary lists the `raw.*` files, which stay in the output folder, and
//! the temporary folder of the GeoParquet files is removed, also after an error.

use duckosm_build::{build, Params};
use std::path::PathBuf;

/// The temporary folders of this process.
fn temp_folders() -> usize {
    let prefix = format!("duckosm-{}-", std::process::id());
    std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(&prefix)
        })
        .count()
}

#[test]
fn duckdb_only_with_raw() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-duckdb-raw");
    let p = Params {
        geoparquet: false,
        duckdb: true,
        raw: true,
        boundary: Some(data.join("monaco.geojson")),
        built_at: Some(0),
        ..Params::default()
    };
    let sum = build(&data.join("monaco_sample.osm.pbf"), &out, &p).unwrap();
    let kinds: Vec<&str> = sum.files.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kinds, ["raw_nodes", "raw_ways", "raw_relations", "duckdb"]);
    assert!(sum.files.iter().all(|(_, f)| f.exists()));
    assert_eq!(temp_folders(), 0);

    assert!(build(&data.join("no_such_file.osm.pbf"), &out, &p).is_err());
    assert_eq!(temp_folders(), 0);
}
