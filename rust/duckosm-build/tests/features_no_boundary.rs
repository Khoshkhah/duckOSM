//! The base-map layers of a build with no boundary, and `features-check`, equal duckOSM's Python build (the reference, the
//! Python engine, 2026-11-17): Monaco's whole sample PBF (`data/sample/monaco.osm.pbf`), the driving mode, no levels, no bus
//! routes, no sea. Each row: the number of rows of a layer and the MD5 of its rows as text (the SQL of
//! tests/features_h3.rs). The report of `features-check` is compared as its lines in byte order (the Python order of groups
//! with equal keys is not fixed). It needs DuckDB's spatial extension, as the build does.

use duckdb::Connection;
use std::path::PathBuf;

#[test]
fn the_layers_without_a_boundary_equal_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-features-whole-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let cfg = format!(
        "name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}}}\nmodes: [driving]\noptions:\n  sea: false\n  levels: false\n  bus_routes: false\n",
        out.display(),
        root.join("data/sample/monaco.osm.pbf").display()
    );
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    assert!(c.skipped.is_empty() && c.params.features);
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();

    let db = out.join("monaco.duckdb");
    let con = Connection::open(&db).unwrap();
    con.execute_batch("LOAD spatial").unwrap();
    for (t, n, md5) in [
        ("buildings", 1625, "88cb71771479071957b66716e465da96"),
        ("land", 255, "a3b2b6b492bb93e3f11090344d1e7399"),
        ("paved", 90, "d35d37f03c63c62d17e9f24682a0bbd7"),
        ("place_labels", 10, "b04df83c12d905dca7ce941d09502e1a"),
        ("pois", 1706, "80cbaba92a4eef23fe32d37b1820381b"),
        ("public_transport", 119, "ed07ab164ee7117dcc25841d2d0dc797"),
        ("sites", 90, "b7d7087c6c2f331ecce1b7d15503a2a0"),
        ("streets", 3666, "cd397cb9d2850959d0f16456d7520425"),
        ("traffic", 663, "add017384318b746e711f2fe04d69ed1"),
        ("water_lines", 4, "3f2cb2c9aa9ff8b5ac3abc7765179fed"),
        ("water_polygons", 96, "95cacc03028ce45558dfc1eb0b894eac"),
    ] {
        let bearing = if t == "traffic" { ", round(bearing, 6)" } else { "" };
        let got: (i64, String) = con
            .query_row(
                &format!(
                    "SELECT count(*), md5(string_agg(x, ',' ORDER BY x)) FROM (SELECT concat_ws('|', osm_id, osm_type, kind, name, tags::VARCHAR, hex(ST_AsWKB(geom)){bearing}) x FROM features.{t})"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(got, (n, md5.to_string()), "features.{t}");
    }

    // features-check: the lines of duckOSM's report (--limit 1000), sorted in byte order
    drop(con);
    let groups = duckosm_build::features_check::missing_polygons(&db).unwrap();
    let mut lines: Vec<&str> = Vec::new();
    let text = duckosm_build::features_check::report(&groups, 1000);
    lines.extend(text.lines());
    lines.sort_unstable();
    let sorted = lines.join("\n") + "\n";
    let md5: String = Connection::open_in_memory().unwrap().query_row("SELECT md5(?)", [&sorted], |r| r.get(0)).unwrap();
    assert_eq!((lines.len(), md5.as_str()), (51, "8ccc1af14808b44fabc656c3d4f811fc"), "features-check:\n{text}");
    let _ = std::fs::remove_dir_all(&out);
}
