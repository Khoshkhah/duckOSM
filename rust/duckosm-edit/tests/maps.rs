//! `duckosm viz` (maps.rs) on Monaco's sample PBF in its boundary, three modes, with the stored levels: the pages of
//! tests/test_maps.py (the names, the order, the size), the map of one mode with only its roads, and the error of a mode
//! that is not in the file. The pages in a browser: tests/test_maps.py.
#![cfg(feature = "duckdb")]

use duckosm_edit::maps::{render_maps, Viz};
use duckosm_edit::native::NativeDb;
use std::path::PathBuf;

#[test]
fn viz_pages_of_monaco() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-edit-maps-{}", std::process::id()));
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

    let pages = render_maps(&mut NativeDb::new(&db), &db, &Viz::new(out.join("reports"))).unwrap();
    let names: Vec<String> = pages.iter().map(|p| p.file_name().unwrap().to_string_lossy().into()).collect();
    assert_eq!(names, ["monaco_map.html", "monaco_cycling_network.html", "monaco_driving_network.html", "monaco_walking_network.html"]);
    let size = |p: &PathBuf| std::fs::metadata(p).unwrap().len();
    assert!(pages.iter().all(|p| size(p) > 100_000));
    // the map of one mode has only that mode's roads: the driving map is the smallest
    assert!(size(&pages[2]) < size(&pages[0]) && size(&pages[2]) < size(&pages[3]));
    let one = Viz { modes: Some(vec!["driving".into()]), all_modes: false, ..Viz::new(out.join("one")) };
    let pages = render_maps(&mut NativeDb::new(&db), &db, &one).unwrap();
    assert_eq!(pages.len(), 1);
    assert!(pages[0].ends_with("monaco_driving_network.html"));
    let bad = Viz { modes: Some(vec!["flying".into()]), ..Viz::new(out.join("bad")) };
    assert!(render_maps(&mut NativeDb::new(&db), &db, &bad).unwrap_err().contains("not in"));
    let _ = std::fs::remove_dir_all(&out);
}

/// The ground rule (docs/concepts/ground-reference.md, section 3): a page of an area with a ground draws that ground (roadstyle's page option
/// `ground`, with the terrain file of main.elevation_grid), not the AWS Terrain Tiles. An area with no ground gets no option.
#[test]
fn pages_draw_the_area_ground() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-edit-ground-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let s = root.join("data/sample");
    let tif = root.join("rust/duckosm-build/tests/data/monaco-copernicus-glo30.tif");
    for (name, dem) in [("bare", String::new()),
                        ("dem", format!("elevation: {{dem: {}, product: DSM, vertical_datum: EGM2008, license: test}}\n", tif.display()))] {
        let cfg = format!("name: {name}\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}/monaco.osm.pbf}}\nboundary: {{path: {}/monaco.geojson}}\nmodes: [driving]\noptions:\n  sea: false\n{dem}",
                          out.display(), s.display(), s.display());
        std::fs::write(out.join(format!("{name}.yaml")), cfg).unwrap();
        let c = duckosm_build::config::read(&out.join(format!("{name}.yaml")), false, true).unwrap();
        let mut p = c.params;
        p.geoparquet = false;
        duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
        let db = out.join(format!("{name}.duckdb"));
        let mut d = NativeDb::new(&db);
        duckosm_edit::Db::open(&mut d).unwrap();
        let html = duckosm_edit::page::make(&mut d, name).unwrap();
        let has = html.contains("data:application/vnd.pmtiles;base64,");
        assert_eq!(has, name == "dem", "{name}: the page has the area's terrain file");
        if has {
            assert!(html.contains("\"product\":\"DSM\"") && html.contains("\"vertical_datum\":\"EGM2008\""), "the facts of main.elevation_metadata");
        }
    }
    let _ = std::fs::remove_dir_all(&out);
}
