//! `duckosm-rust gmns-map` on Monaco: the GMNS file (duckosm-export, with netconvert on the PATH), the level area (roadstyle's make_area
//! on the roads of card 8), and the lane page of lanestyle's Rust engine. lanestyle's own golden check compares the page's items with
//! the Python lanestyle 0.3.1; here the command makes the page, with the lanes in it, and says how to make a missing level area.
#![cfg(feature = "duckdb")]

use duckosm_edit::{gmns_map, native::NativeDb, page, Db};
use std::path::PathBuf;

#[test]
fn the_lane_page_of_monaco() {
    if std::process::Command::new("netconvert").arg("--version").output().is_err() {
        eprintln!("no netconvert on the PATH: the GMNS lanes need it; skipped");
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-edit-gmns-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let s = root.join("data/sample");
    let cfg = format!("name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}/monaco.osm.pbf}}\nboundary: {{path: {}/monaco.geojson}}\nmodes: [driving, walking, cycling]\noptions:\n  sea: false\n",
                      out.display(), s.display(), s.display());
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
    let db = out.join("monaco.duckdb");
    let gmns = out.join("monaco_gmns.duckdb");
    duckosm_export::gmns::to_gmns(&db, &gmns, &["driving".to_string()]).unwrap();
    let html = out.join("lanes.html");
    // no level area: the error says how to make it
    let e = gmns_map::write_map(&gmns, &html, &db, None).unwrap_err();
    assert!(e.contains("duckosm levels"), "{e}");
    // the level area of the build's roads (duckosm levels: roadstyle's make_area)
    let mut ndb = NativeDb::new(&db);
    ndb.open().unwrap();
    let roads = roadstyle::read::read_roads(page::level_edges(&mut ndb).unwrap(), "geometry").unwrap();
    ndb.close();
    roadstyle::level_area::make_area(&roads, &gmns_map::default_area(&db), &roadstyle::level_area::MakeOptions::default()).unwrap();
    let t = std::time::Instant::now();
    let p = gmns_map::write_map(&gmns, &html, &db, None).unwrap();
    eprintln!("gmns-map: {:.1} s", t.elapsed().as_secs_f64());
    let page = std::fs::read_to_string(p).unwrap();
    assert!(page.contains("maplibregl") && page.contains("roads-simple") && page.contains("<title>Lanes"), "not the lane page");
    let _ = std::fs::remove_dir_all(&out);
}
