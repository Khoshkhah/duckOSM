//! The lane profile of Monaco (docs/lane_profile.md) against duckOSM's: the sample file cut to its boundary, three
//! modes, duckOSM's fixes file (`tests/data/osm_fixes.yaml`, a copy of its `osm_overrides/osm_overrides.yaml`).
//! `monaco_lane_profile.tsv` is `driving.lane_profile` of duckOSM's `monaco.duckdb`, exported on 2026-10-22.
//! Every row must be equal.

use duckosm_build::{build, NetworkMode, Params};
use std::path::PathBuf;

/// A row as one line: edge_id, lane_num, use, width_m, turn (empty for NULL), source, width_source.
fn line(f: [String; 7]) -> String {
    let w: f64 = f[3].parse().unwrap();
    format!(
        "{}\t{}\t{}\t{w}\t{}\t{}\t{}",
        f[0], f[1], f[2], f[4], f[5], f[6]
    )
}

#[test]
fn monaco_lane_profile_equals_duckosm() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-lanes");
    let p = Params {
        modes: vec![
            NetworkMode::Driving,
            NetworkMode::Walking,
            NetworkMode::Cycling,
        ],
        geoparquet: true,
        duckdb: true,
        name: "monaco".into(),
        boundary: Some(data.join("monaco.geojson")),
        osm_fixes: Some(data.join("osm_fixes.yaml")),
        built_at: Some(0),
        ..Params::default()
    };
    let sum = build(&data.join("monaco_sample.osm.pbf"), &out, &p).unwrap();
    assert_eq!(sum.lanes.as_deref(), Some("lane profile: 3176 lanes"));

    let con = duckdb::Connection::open(out.join("monaco.duckdb")).unwrap();
    let mut stmt = con
        .prepare(
            "SELECT edge_id::VARCHAR, lane_num::VARCHAR, use, width_m::VARCHAR, coalesce(turn, ''), source, width_source \
             FROM driving.lane_profile",
        )
        .unwrap();
    let mut got: Vec<String> = stmt
        .query_map([], |r| {
            Ok(line(std::array::from_fn(|i| {
                r.get::<_, String>(i).unwrap()
            })))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let tsv = std::fs::read_to_string(data.join("monaco_lane_profile.tsv")).unwrap();
    let mut want: Vec<String> = tsv
        .lines()
        .skip(1)
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            line(std::array::from_fn(|i| f[i].to_string()))
        })
        .collect();
    got.sort_unstable();
    want.sort_unstable();
    let only_rs: Vec<&String> = got.iter().filter(|r| !want.contains(r)).collect();
    let only_py: Vec<&String> = want.iter().filter(|r| !got.contains(r)).collect();
    assert!(
        only_rs.is_empty() && only_py.is_empty() && got == want,
        "{} rows only in duckosm-rust, {} only in duckOSM\nrust: {:#?}\nduckOSM: {:#?}",
        only_rs.len(),
        only_py.len(),
        &only_rs[..only_rs.len().min(20)],
        &only_py[..only_py.len().min(20)]
    );
}
