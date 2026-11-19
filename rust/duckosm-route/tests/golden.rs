//! The routes and the GraphML files equal duckOSM's Python output (2026-11-17) on the database of this config: Monaco's
//! sample PBF in its boundary, three modes, no sea, no levels, no tile columns (the Python build has none). tests/data/golden_route/monaco_routes.json has 40 routes for
//! each of driving and walking, by time and by length (`Router(con, mode, weight).route(a, b)`: the edge ids, `time_s`,
//! `length_m`; null when there is no route). The GraphML files of `duckosm export-graph` (node graph, edge graph by time)
//! are too large for the repository: the test has their MD5.

use md5::{Digest, Md5};
use std::path::PathBuf;

#[test]
fn routes_and_graphml_equal_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-route-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let s = root.join("data/sample");
    let cfg = format!(
        "name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}/monaco.osm.pbf}}\nboundary: {{path: {}/monaco.geojson}}\nmodes: [driving, walking, cycling]\noptions:\n  sea: false\n  levels: false\n  tile_indexing: false\n",
        out.display(),
        s.display(),
        s.display()
    );
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
    let con = duckosm_route::route::open(&out.join("monaco.duckdb")).unwrap();

    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("tests/data/golden_route/monaco_routes.json")).unwrap()).unwrap();
    let mut graphs = std::collections::HashMap::new();
    for r in golden.as_array().unwrap() {
        let (mode, weight) = (r["mode"].as_str().unwrap(), r["weight"].as_str().unwrap());
        let g = graphs
            .entry((mode.to_string(), weight.to_string()))
            .or_insert_with(|| duckosm_route::route::routing_graph(&con, mode, weight, &[]).unwrap());
        let (a, b) = (r["from"].as_i64().unwrap(), r["to"].as_i64().unwrap());
        let got = duckosm_route::route::route(&con, g, a, b, mode).unwrap();
        match got {
            None => assert!(r["edges"].is_null(), "{mode} {weight} {a} {b}: no route, duckOSM has one"),
            Some(got) => {
                assert_eq!(got["edges"], r["edges"], "{mode} {weight} {a} {b}");
                assert_eq!(got["time_s"].as_f64(), r["time_s"].as_f64(), "{mode} {weight} {a} {b}");
                assert_eq!(got["length_m"].as_f64(), r["length_m"].as_f64(), "{mode} {weight} {a} {b}");
            }
        }
    }

    let md5 = |s: &str| format!("{:x}", Md5::digest(s.as_bytes()));
    let mut node = Vec::new();
    duckosm_route::graphml::write_node_graph(&con, "driving", true, &mut node).unwrap();
    assert_eq!(md5(std::str::from_utf8(&node).unwrap()), "77fefab017a85c5cbc85320f422920a8", "the node graph");
    let edge = duckosm_route::graphml::edge_graph(&con, "driving", "time", &[]).unwrap().write();
    assert_eq!(md5(&edge), "19781276d6fde55c062f42306b6912dd", "the edge graph");
    let _ = std::fs::remove_dir_all(&out);
}
