//! The planner page routes with duckosm-planner on its data (`route_map::planner_data`); here, on Monaco (three modes, walk +
//! drive), its answers equal the program's (`duckosm-route`: route-points, route-multimodal-points, directions) for many pairs
//! of points, by time and by length, also where a point has no road near it. One difference is allowed: a tie. The database
//! gives the roads near a point in the order of its R-tree index (as in Python), the page in the order of the table, so when
//! two routes have the same cost each can take another one; then the time and the length must be equal.
//! The page in a browser: tests/test_route_map_rust.py.
#![cfg(feature = "duckdb")]

use duckosm_planner::Area;
use serde_json::Value as J;
use std::path::PathBuf;

/// The WKT of each road of a trip (duckosm-route's, not in the page's data) taken out.
fn no_wkt(mut v: J) -> J {
    if v.is_null() {
        return v;
    }
    for l in v["legs"].as_array_mut().into_iter().flatten() {
        for p in l["path"].as_array_mut().into_iter().flatten() {
            p.as_object_mut().map(|o| o.shift_remove("geometry"));
        }
    }
    v
}

#[test]
fn the_page_routes_as_the_program() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-edit-route-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let s = root.join("data/sample");
    let cfg = format!("name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}/monaco.osm.pbf}}\nboundary: {{path: {}/monaco.geojson}}\nmodes: [driving, walking, cycling]\noptions:\n  sea: false\n  levels: false\nmultimodal: {{enabled: true}}\n",
                      out.display(), s.display(), s.display());
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
    let db = out.join("monaco.duckdb");
    let con = duckosm_route::route::open(&db).unwrap();
    let data = duckosm_edit::route_map::planner_data(&con).unwrap();
    let area = Area::from_data(&serde_json::from_str(&data.to_string()).unwrap()).unwrap();
    let wd: Vec<String> = vec!["walking".into(), "driving".into()];
    let net = duckosm_route::points::network(&con, "mm", Some(&wd)).unwrap();

    // points near a point of a road (up to about 40 m off; some have no road of a mode within 50 m), from a fixed sequence
    let geom = data["geom"].as_array().unwrap();
    let mut seed: u64 = 7;
    let mut rnd = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (seed >> 11) as f64 / (1u64 << 53) as f64 };
    let mut near_road = || {
        let g = geom[(rnd() * geom.len() as f64) as usize].as_array().unwrap();
        let i = 2 * (rnd() * (g.len() / 2) as f64) as usize;
        (g[i].as_f64().unwrap() + (rnd() - 0.5) * 0.0008, g[i + 1].as_f64().unwrap() + (rnd() - 0.5) * 0.0006)
    };
    let (mut n, mut routes, mut ties) = (0, 0, 0);
    // equal, or a tie: the same time and length (to 1e-9)
    let mut same = |got: &Result<J, String>, want: &Result<J, String>, what: String| -> bool {
        if got == want {
            return true;
        }
        let (g, w) = (got.as_ref().expect(&what), want.as_ref().expect(&what));
        for k in ["time_s", "length_m"] {
            let (a, b) = (g[k].as_f64().expect(&what), w[k].as_f64().expect(&what));
            assert!((a - b).abs() < 1e-9, "{what}: {k} {a} != {b}");
        }
        ties += 1;
        false
    };
    for _ in 0..60 {
        let (a, b) = (near_road(), near_road());
        for mode in ["driving", "walking", "cycling"] {
            for weight in ["time", "length"] {
                let want = duckosm_route::points::route_points(&con, a, b, mode, weight, 50.0, 4.5);
                let got = area.route_points(a, b, mode, weight, 50.0, 4.5);
                let eq = same(&got, &want, format!("route_points {a:?} {b:?} {mode} {weight}"));
                if let (Ok(r), true) = (&want, eq) {
                    assert_eq!(area.directions(r, mode), duckosm_route::directions::directions(&con, r, mode), "directions {a:?} {b:?} {mode}");
                    routes += usize::from(!r.is_null());
                }
                n += 1;
            }
        }
        let want = duckosm_route::points::route_multimodal_points(&con, &net, a, b, 50.0, 4.5, "mm", Some(&wd), true).map(no_wkt);
        let got = area.route_multimodal_points(a, b, 50.0, 4.5, "time");
        let eq = same(&got, &want, format!("route_multimodal_points {a:?} {b:?}"));
        if let (Ok(r), true) = (&want, eq) {
            assert_eq!(area.directions(r, "walking"), duckosm_route::directions::directions(&con, r, "walking"), "directions {a:?} {b:?} walk + drive");
            routes += usize::from(!r.is_null());
        }
        n += 1;
    }
    assert!(routes > 100, "only {routes} routes of {n} questions");
    assert!(ties * 20 < routes, "{ties} ties of {routes} routes");
    eprintln!("{n} questions, {routes} routes, {ties} ties");

    // the page: the router in it, the data, the first choice
    let page = duckosm_edit::route_map::write_route_map(&db, &out.join("planner.html"), Some("walk+drive"), None, Some(false)).unwrap();
    let html = std::fs::read_to_string(page).unwrap();
    assert!(html.contains("rm_plan") && !html.contains("__RM__") && !html.contains("const WASM = \"__WASM__\""));
    assert!(html.contains(r#""first":["walking","driving"]"#));
    let _ = std::fs::remove_dir_all(&out);
}
