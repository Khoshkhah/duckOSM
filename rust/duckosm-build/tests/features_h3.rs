//! The H3 columns and the base-map layers of the native build equal those of duckOSM's Python build (the reference).
//!
//! Monaco, three modes, its boundary, duckOSM's defaults. The golden file `tests/data/monaco_features_h3_summary.tsv` comes
//! from duckOSM 0.7.0 (DuckDB 1.5.3) with the Python engine: tools/make_golden_features.py, the build is
//! tests/data/golden_monaco_features.yaml. Each row has the number of rows of a table and the MD5 of its rows as text, so
//! this test fails when one value of one row differs. The SQL is the same as in the tool: if you change one, change the other.
//! The test needs DuckDB's spatial extension (the first `INSTALL` downloads it), as the build does.

use duckdb::Connection;
use duckosm_build::{build, LevelsMethod, NetworkMode, Params};
use std::path::PathBuf;

const MODES: [&str; 3] = ["driving", "walking", "cycling"];
const LAYERS: [&str; 11] = [
    "buildings",
    "land",
    "paved",
    "place_labels",
    "pois",
    "public_transport",
    "sites",
    "streets",
    "traffic",
    "water_lines",
    "water_polygons",
];

/// (name, a SELECT of one text for each row), as in tools/make_golden_features.py.
fn tables() -> Vec<(String, String)> {
    let mut v = Vec::new();
    for t in LAYERS {
        let bearing = if t == "traffic" { ", round(bearing, 6)" } else { "" };
        v.push((
            format!("features.{t}"),
            format!("SELECT concat_ws('|', osm_id, osm_type, kind, name, tags::VARCHAR, hex(ST_AsWKB(geom)){bearing}) FROM features.{t}"),
        ));
    }
    for (t, sql) in [
        ("nodes", "SELECT concat_ws('|', osm_id, lat, lon, tags::VARCHAR) FROM raw.nodes"),
        ("ways", "SELECT concat_ws('|', osm_id, tags::VARCHAR, refs::VARCHAR) FROM raw.ways"),
        (
            "relations",
            "SELECT concat_ws('|', osm_id, tags::VARCHAR, refs::VARCHAR, ref_roles::VARCHAR, ref_types::VARCHAR) FROM raw.relations",
        ),
    ] {
        v.push((format!("raw.{t}"), sql.to_string()));
    }
    for m in MODES {
        v.push((
            format!("{m}.nodes.h3"),
            format!("SELECT concat_ws('|', node_id, h3_cell) FROM {m}.nodes"),
        ));
        v.push((
            format!("{m}.edges.h3"),
            format!("SELECT concat_ws('|', edge_id, from_cell, to_cell, lca_res) FROM {m}.edges"),
        ));
    }
    v
}

#[test]
fn monaco_features_and_h3_equal_the_python_build() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-features-h3");
    let p = Params {
        modes: vec![NetworkMode::Driving, NetworkMode::Walking, NetworkMode::Cycling],
        geoparquet: false,
        duckdb: true,
        name: "golden".into(),
        boundary: Some(data.join("monaco.geojson")),
        levels: LevelsMethod::None,
        bus_routes: false,
        h3_resolution: Some(8),
        features: true,
        built_at: Some(0),
        ..Params::default()
    };
    let sum = build(&data.join("monaco_sample.osm.pbf"), &out, &p).unwrap();
    // raw.* is in the .duckdb file, not a file of the output folder (without p.raw)
    let kinds: Vec<&str> = sum.files.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kinds, ["duckdb"]);
    assert!(!out.join("raw_nodes.parquet").exists());

    let con = Connection::open(out.join("golden.duckdb")).unwrap();
    con.execute_batch("INSTALL spatial; LOAD spatial").unwrap();
    let golden = std::fs::read_to_string(data.join("monaco_features_h3_summary.tsv")).unwrap();
    let want: Vec<(&str, &str, &str)> = golden
        .lines()
        .skip(1)
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c[0], c[1], c[2])
        })
        .collect();
    let tables = tables();
    assert_eq!(want.len(), tables.len());
    let mut bad = Vec::new();
    for ((name, select), (gname, rows, md5)) in tables.iter().zip(&want) {
        assert_eq!(name, gname);
        let (n, h): (i64, String) = con
            .query_row(
                &format!("SELECT count(*), md5(string_agg(s, chr(10) ORDER BY s)) FROM ({select}) x(s)"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        if n.to_string() != *rows || h != *md5 {
            bad.push(format!("{name}: {n} rows, {h}; golden {rows} rows, {md5}"));
        }
    }
    assert!(bad.is_empty(), "differs from the Python build:\n{}", bad.join("\n"));
}
