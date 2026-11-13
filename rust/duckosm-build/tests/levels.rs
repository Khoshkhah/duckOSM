//! The drawing levels of Monaco (docs/levels.md): the sample file cut to its boundary, three modes. The expected
//! summaries were checked against roadstyle 0.24.2 with `tools/compare_levels.py` on 2026-10-22: the tags rule
//! equals roadstyle's on every row; the solve has the values of roadstyle's stages, and 13,288 of the 13,298 rows
//! equal to roadstyle's. With the crossing rule (roadstyle-levels f0a161b) the summary has five stages; 7,360 of the
//! 13,298 rows differ from the earlier values in at least one column.

use duckosm_build::{build, LevelsMethod, NetworkMode, Params};
use std::path::PathBuf;

#[test]
fn monaco_levels() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    for (method, want) in [
        (LevelsMethod::Tags, "levels (tags): 13298 edges, 6830 roads"),
        (
            LevelsMethod::Solve,
            "levels (solve): 13298 edges, 6830 roads, 0 stack pairs given up, 18 order wishes not kept \
             (best values of the stages: [0, 4, 43, 0, 6561080])",
        ),
    ] {
        let mut p = Params {
            modes: vec![NetworkMode::Driving, NetworkMode::Walking, NetworkMode::Cycling],
            geoparquet: true,
            duckdb: false,
            boundary: Some(data.join("monaco.geojson")),
            built_at: Some(0),
            ..Params::default()
        };
        p.levels = method;
        let out = tmp.join(format!("duckosm-levels-{}", method.name()));
        let sum = build(&data.join("monaco_sample.osm.pbf"), &out, &p).unwrap();
        assert_eq!(sum.levels.as_deref(), Some(want));
        assert!(out.join("edge_levels_visualization.parquet").exists());
        assert!(out.join("edge_levels_meta_visualization.parquet").exists());
    }
}
