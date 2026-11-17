//! `source.type: duckdb` (clip.rs) equals duckOSM's Python clip (the reference, 2026-11-17). The parent: Monaco's sample PBF in
//! `data/sample/monaco.geojson`, driving and walking, no base-map layers, no levels, no bus routes (built here by this program;
//! its tables equal those of the Python build). The clip: the box [7.415, 43.728, 7.425, 43.738], `clip.predicate: intersects`.
//! The expected values are the counts and the MD5 of the keys of the Python clip of the same parent. It needs DuckDB's
//! spatial extension, as the build does.

use duckdb::Connection;
use std::path::PathBuf;

fn build(cfg: &str, dir: &std::path::Path) {
    std::fs::write(dir.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&dir.join("config.yaml"), false, true).unwrap();
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
}

#[test]
fn a_clip_of_a_database_equals_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-clip-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let opts = "options:\n  sea: false\n  levels: false\n  build_features: false\n  bus_routes: false\n";
    build(
        &format!(
            "name: parent\noutput_path: {o}\nsource: {{type: pbf, pbf_path: {s}/monaco.osm.pbf}}\nboundary: {{path: {s}/monaco.geojson}}\nmodes: [driving, walking]\n{opts}",
            o = out.display(),
            s = root.join("data/sample").display()
        ),
        &out,
    );
    build(
        &format!(
            "name: clipped\noutput_path: {o}\nsource: {{type: duckdb, source_db: {o}/parent.duckdb}}\nboundary: {{bbox: [7.415, 43.728, 7.425, 43.738]}}\nmodes: [driving, walking]\n{opts}",
            o = out.display()
        ),
        &out,
    );
    let con = Connection::open(out.join("clipped.duckdb")).unwrap();
    for (t, k, n, md5) in [
        ("driving.edges", "edge_id", 1018, "9194746df34f039093fedc5e25740542"),
        ("driving.nodes", "node_id", 691, "1d37e22e8dc3e3176414cd7f1e575187"),
        ("driving.edge_graph", "from_edge, to_edge", 2433, "17a94a97327174f92927bf1031951d25"),
        ("driving.turn_restrictions", "from_edge_id, to_edge_id", 17, "a2ddb7cb5fe32c65827c42870eeae8c8"),
        ("walking.edges", "edge_id", 4390, "49f12add579762fcea0ef0593ef30b9e"),
        ("walking.private_edges", "edge_id", 38, "9565cc270de4629f980dfa3f7c32d898"),
        ("walking.edge_graph", "from_edge, to_edge", 12896, "936093cfa77a94f15249ac408261d75b"),
    ] {
        let got: (i64, String) = con
            .query_row(&format!("SELECT count(*), md5(string_agg(concat_ws('|', {k}), ',' ORDER BY {k})) FROM {t}"), [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(got, (n, md5.to_string()), "{t}");
    }
    let meta: (f64, f64, i32, String) = con
        .query_row("SELECT center_lat, center_lon, initial_zoom, timezone FROM main.visualization_metadata", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!(meta, (43.733000000000004, 7.42, 14, "Europe/Monaco".to_string()));
    let _ = std::fs::remove_dir_all(&out);
}
