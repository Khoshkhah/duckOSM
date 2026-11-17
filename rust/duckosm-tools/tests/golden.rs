//! `info` and `way` give the output of duckOSM's Python commands. The golden files in tests/data/golden_tools/ are the output
//! of `duckosm info` / `duckosm way` (Python, 2026-11-17) on the database of this config: Monaco's sample PBF in its boundary,
//! three modes, no sea, no levels. `info` leaves out the build time and the version (its first line, and two keys of the JSON).

use std::path::PathBuf;

fn golden(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/golden_tools").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn info_and_way_equal_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-tools-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let s = root.join("data/sample");
    let cfg = format!(
        "name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}/monaco.osm.pbf}}\nboundary: {{path: {}/monaco.geojson}}\nmodes: [driving, walking, cycling]\noptions:\n  sea: false\n  levels: false\n",
        out.display(),
        s.display(),
        s.display()
    );
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
    let db = out.join("monaco.duckdb");

    let d = duckosm_tools::info::db_info(&db).unwrap();
    let text = duckosm_tools::info::text(&db, &d);
    assert_eq!(text.split_once('\n').unwrap().1, golden("monaco_info.txt"));
    let mut d = d;
    d.as_object_mut().unwrap().shift_remove("built_at");
    d.as_object_mut().unwrap().shift_remove("duckosm_version");
    assert_eq!(duckosm_tools::json::dumps(&d) + "\n", golden("monaco_info.json"));

    for (id, json, file) in [
        (4227277, false, "monaco_way_4227277.txt"),
        (4227277, true, "monaco_way_4227277.json"),
        (-1525349445, false, "monaco_way_connector.txt"),
    ] {
        let mut w = Vec::new();
        duckosm_tools::way::way(&db, id, &[], false, None, json, &mut w).unwrap();
        assert_eq!(String::from_utf8(w).unwrap(), golden(file), "{file}");
    }
    let _ = std::fs::remove_dir_all(&out);
}
