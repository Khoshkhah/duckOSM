//! `route_multimodal`, `route_points` and `route_multimodal_points` equal duckOSM's Python output (2026-11-17) on the
//! database of this config: Monaco's sample PBF in its boundary, three modes, no sea, no levels, no base-map layers, with
//! `multimodal.enabled`. tests/data/golden_route/monaco_multimodal_points.json has 8 answers of each (the geometries taken out):
//! random node pairs, and random points near nodes, in driving and walking, by time and by length; and
//! tests/data/golden_route/monaco_directions.json the steps of `directions` for each of these routes.

use serde_json::Value as J;
use std::path::PathBuf;

fn strip(v: &J) -> J {
    match v {
        J::Object(o) => J::Object(o.iter().filter(|(k, _)| k.as_str() != "geometry").map(|(k, x)| (k.clone(), strip(x))).collect()),
        J::Array(a) => J::Array(a.iter().map(strip).collect()),
        x => x.clone(),
    }
}

fn answer(r: duckosm_route::Result<J>) -> J {
    match r {
        Ok(v) => strip(&v),
        Err(e) => serde_json::json!({"error": e}),
    }
}

#[test]
fn multimodal_and_points_equal_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-route-mm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let s = root.join("data/sample");
    let cfg = format!(
        "name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}/monaco.osm.pbf}}\nboundary: {{path: {}/monaco.geojson}}\nmodes: [driving, walking, cycling]\noptions:\n  sea: false\n  levels: false\n  tile_indexing: false\n  build_features: false\nmultimodal: {{enabled: true}}\n",
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
    let net = duckosm_route::points::network(&con, "mm", None).unwrap();
    let g: J = serde_json::from_str(&std::fs::read_to_string(root.join("tests/data/golden_route/monaco_multimodal_points.json")).unwrap()).unwrap();

    // the routes in the order of the steps of the golden file: route_points, route_multimodal_points, route_multimodal
    let mut routes: Vec<(String, J)> = Vec::new();
    let found = |v: &J| !v.is_null() && v.get("error").is_none();
    let mut mm_routes = Vec::new();
    for q in g["mm"].as_array().unwrap() {
        let (a, b) = (q[0][0].as_i64().unwrap(), q[0][1].as_i64().unwrap());
        let full = duckosm_route::points::route_multimodal(&con, &net, Some(a), Some(b), "mm", "walking", "walking", true, None);
        if let Ok(v) = &full {
            if found(v) {
                mm_routes.push(("walking".to_string(), v.clone()));
            }
        }
        assert_eq!(answer(full), q[1], "route_multimodal {a} {b}");
    }
    let pts: Vec<Vec<f64>> = g["points"].as_array().unwrap().iter().map(|p| p.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()).collect();
    for q in g["pts"].as_array().unwrap() {
        let (mode, w) = (q[0].as_str().unwrap(), q[1].as_str().unwrap());
        for (p, want) in pts.iter().zip(q[2].as_array().unwrap()) {
            let full = duckosm_route::points::route_points(&con, (p[0], p[1]), (p[2], p[3]), mode, w, 50.0, 4.5);
            if let Ok(v) = &full {
                if found(v) {
                    routes.push((mode.to_string(), v.clone()));
                }
            }
            assert_eq!(&answer(full), want, "route_points {mode} {w} {p:?}");
        }
    }
    for (p, want) in pts.iter().zip(g["mmpts"].as_array().unwrap()) {
        let full = duckosm_route::points::route_multimodal_points(&con, &net, (p[0], p[1]), (p[2], p[3]), 50.0, 4.5, "mm", None, true);
        if let Ok(v) = &full {
            if found(v) {
                routes.push(("walking".to_string(), v.clone()));
            }
        }
        assert_eq!(&answer(full), want, "route_multimodal_points {p:?}");
    }
    routes.extend(mm_routes);
    let steps: J = serde_json::from_str(&std::fs::read_to_string(root.join("tests/data/golden_route/monaco_directions.json")).unwrap()).unwrap();
    let steps = steps.as_array().unwrap();
    assert_eq!(routes.len(), steps.len());
    for (i, ((mode, r), want)) in routes.iter().zip(steps).enumerate() {
        assert_eq!(mode, want[0].as_str().unwrap());
        assert_eq!(duckosm_route::directions::directions(&con, r, mode).unwrap(), want[1], "directions of route {i}");
    }
    let _ = std::fs::remove_dir_all(&out);
}
