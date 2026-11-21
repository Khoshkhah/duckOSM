//! A build writes the comments of the tables and the columns into the `.duckdb` file (COMMENT ON). The text comes from
//! docs/reference/database.md, after the cleanup of the Markdown.

use duckosm_build::{build, NetworkMode as Mode, Params};
use std::path::PathBuf;

#[test]
fn the_columns_have_the_descriptions_of_the_schema_page() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-comments");
    let _ = std::fs::remove_dir_all(&out);
    let p = Params {
        modes: vec![Mode::Driving, Mode::Walking, Mode::Cycling],
        geoparquet: false,
        duckdb: true,
        boundary: Some(data.join("monaco.geojson")),
        features: true,
        built_at: Some(0),
        threads: Some(2),
        memory_limit: Some("1GB".into()),
        ..Params::default()
    };
    let sum = build(&data.join("monaco.osm.pbf"), &out, &p).unwrap();
    let db = sum.files.iter().find(|(k, _)| k == "duckdb").unwrap().1.clone();
    let con = duckdb::Connection::open(db).unwrap();
    let fixes: i64 = con.query_row("SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'main' AND table_name = 'fixes'", [], |r| r.get(0)).unwrap();
    assert_eq!(fixes, 0, "a build with no fixes file has no main.fixes");
    let mut stmt = con
        .prepare("SELECT column_name, comment FROM duckdb_columns() WHERE schema_name = 'driving' AND table_name = 'edges'")
        .unwrap();
    let got: Vec<(String, Option<String>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    let want = duckosm_build::after::comments(&["driving"]);
    let edge_id = got.iter().find(|c| c.0 == "edge_id").and_then(|c| c.1.clone()).unwrap_or_default();
    assert!(!edge_id.is_empty(), "driving.edges.edge_id has no comment");
    for (col, comment) in &got {
        let doc = want.iter().find(|w| w.0 == "driving" && w.1 == "edges" && w.2.as_deref() == Some(col.as_str())).map(|w| w.3.clone());
        assert_eq!(comment, &doc, "driving.edges.{col}");
    }
    let table: Option<String> = con
        .query_row("SELECT comment FROM duckdb_tables() WHERE schema_name = 'driving' AND table_name = 'edges'", [], |r| r.get(0))
        .unwrap();
    assert!(table.is_some_and(|t| t.starts_with("One row per direction of a road")));
}

/// A file with the name of a schema (`mm.duckdb`, multimodal on) makes `mm.transfers` ambiguous in DuckDB: the build
/// refuses it before it writes a file.
#[test]
fn a_database_with_the_name_of_a_schema_is_refused() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-mm-name");
    let _ = std::fs::remove_dir_all(&out);
    let p = Params { modes: vec![Mode::Driving, Mode::Walking], geoparquet: false, duckdb: true, name: "mm".into(), ..Params::default() };
    let p = Params { after: duckosm_build::after::After { multimodal: true, ..p.after.clone() }, ..p };
    let e = build(&data.join("monaco.osm.pbf"), &out, &p).unwrap_err();
    assert!(e.contains("can't be called mm.duckdb"), "{e}");
    assert!(!out.join("mm.duckdb").exists());
}
