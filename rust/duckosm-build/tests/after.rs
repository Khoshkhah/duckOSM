//! The steps after the tables equal those of duckOSM's Python build (the reference): the multimodal transfers with their
//! costs, the indexes, the build checks and their section of the report. The build is tests/data/golden_monaco_after.yaml
//! (Monaco, three modes, no OSM fixes, the Python engine); this test runs the same config with `engine` removed.
//! It needs DuckDB's spatial extension (the R-tree indexes), as the build does.

use duckdb::Connection;
use std::path::PathBuf;

#[test]
fn the_steps_after_the_tables_equal_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-after-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let golden = std::fs::read_to_string(root.join("tests/data/golden_monaco_after.yaml")).unwrap();
    let cfg = golden
        .replace("output_path: /tmp/golden_after", &format!("output_path: {}", out.display()))
        .replace("data/sample/", &format!("{}/data/sample/", root.display()))
        .replace("  engine: python\n", "");
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    assert!(c.ignored.iter().any(|s| s.contains("min_component_edges")));
    let mut p = c.params;
    p.geoparquet = false;
    std::env::set_current_dir(&out).unwrap(); // the report goes to ./reports, as in duckOSM
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();

    let con = Connection::open(out.join("monaco.duckdb")).unwrap();
    let (n, md5): (i64, String) = con
        .query_row(
            "SELECT count(*), md5(string_agg(concat_ws('|', node_id, from_mode, to_mode, cost_s, kind), ',' ORDER BY node_id, from_mode, to_mode)) FROM mm.transfers",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((n, md5.as_str()), (12130, "b2d975bbac013eaba712db659d033323"), "mm.transfers");
    let indexes: i64 = con.query_row("SELECT count(*) FROM duckdb_indexes()", [], |r| r.get(0)).unwrap();
    assert_eq!(indexes, 24);

    let report = std::fs::read_dir(out.join("reports")).unwrap().map(|e| e.unwrap().path()).find(|p| p.extension().is_some_and(|e| e == "md")).unwrap();
    let text = std::fs::read_to_string(report).unwrap();
    let section = &text[text.find("## Validation").expect("no validation section")..];
    assert_eq!(section, std::fs::read_to_string(root.join("tests/data/monaco_validation.md")).unwrap());
    let _ = std::fs::remove_dir_all(&out);
}
