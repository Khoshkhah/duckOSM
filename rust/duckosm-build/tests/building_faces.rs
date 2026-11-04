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
