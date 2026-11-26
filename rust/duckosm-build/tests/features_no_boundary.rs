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
        ("building_parts", 61, "b716a78a39dadf25570784402ca44c7f"),
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
    assert_eq!((lines.len(), md5.as_str()), (47, "e06f067b74f645d2d6715b2fcf57373e"), "features-check:\n{text}");
    let _ = std::fs::remove_dir_all(&out);
}

/// The same build with the walking mode (its sidewalk rule) and the fixes of `tests/data/osm_fixes_raw.yaml` (a rule of each
/// kind that changes `raw.*`): duckOSM applies both to `raw.*` before the layers. The expected values come from the Python build
/// with the same file.
#[test]
fn the_fixes_change_raw_as_in_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-features-fixes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let cfg = format!(
        "name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}}}\nosm_overrides: {}\nmodes: [driving, walking]\noptions:\n  sea: false\n  levels: false\n  bus_routes: false\n",
        out.display(),
        root.join("data/sample/monaco.osm.pbf").display(),
        root.join("tests/data/osm_fixes_raw.yaml").display()
    );
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
    let con = Connection::open(out.join("monaco.duckdb")).unwrap();
    con.execute_batch("LOAD spatial").unwrap();
    let layer = "concat_ws('|', osm_id, osm_type, kind, name, tags::VARCHAR, hex(ST_AsWKB(geom)))";
    for (t, sql, n, md5) in [
        ("raw.nodes", "concat_ws('|', osm_id, lat, lon, tags::VARCHAR)", 41710, "129f6e8522b1a56a32eac7b95eebf013"),
        ("raw.ways", "concat_ws('|', osm_id, tags::VARCHAR, refs::VARCHAR)", 6249, "928333fc50d13e1675be2bfd65cc00b4"),
        ("features.buildings", layer, 1623, "3ac5cbaebf236732557711a606f07769"),
        ("features.streets", layer, 3667, "f09924d037759c3462d79b8b9329f119"),
    ] {
        let got: (i64, String) = con
            .query_row(&format!("SELECT count(*), md5(string_agg(x, ',' ORDER BY x)) FROM (SELECT {sql} x FROM {t})"), [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(got, (n, md5.to_string()), "{t}");
    }
    let _ = std::fs::remove_dir_all(&out);
}
