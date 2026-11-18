//! The bus routes of a build with no boundary (the whole file, `raw.*` from `read_osm`) equal duckOSM's Python build (the
//! reference, the Python engine, 2026-11-17): Monaco's whole sample PBF, the driving mode, the fixes of
//! `tests/data/osm_fixes.yaml`, no base-map layers, no levels. Each row: the number of rows of a table and the MD5 of its
//! rows as text. It needs DuckDB, as the build does.

use duckdb::Connection;
use std::path::PathBuf;

#[test]
fn the_bus_routes_of_a_whole_file_equal_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-bus-whole-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let cfg = format!(
        "name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}}}\nosm_overrides: {}\nmodes: [driving]\noptions:\n  sea: false\n  levels: false\n  build_features: false\n",
        out.display(),
        root.join("data/sample/monaco.osm.pbf").display(),
        root.join("tests/data/osm_fixes.yaml").display()
    );
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
    let con = Connection::open(out.join("monaco.duckdb")).unwrap();
    for (t, n, md5) in [("routes", 56, "67590372ec22fe36293ca6a623630e88"), ("route_edges", 5755, "f731962a870e699ca05ee322a65a5bab")] {
        let mut st = con.prepare(&format!("SELECT column_name FROM (DESCRIBE bus.{t})")).unwrap();
        let cols: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        let x = format!("concat_ws('|', {})", cols.iter().map(|c| format!("\"{c}\"::VARCHAR")).collect::<Vec<_>>().join(", "));
        let got: (i64, String) = con
            .query_row(&format!("SELECT count(*), md5(string_agg(x, ',' ORDER BY x)) FROM (SELECT {x} x FROM bus.{t})"), [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(got, (n, md5.to_string()), "bus.{t}");
    }
    let _ = std::fs::remove_dir_all(&out);
}
