//! `info` and `way` give the output of duckOSM's Python commands. The golden files in tests/data/golden_tools/ are the output
//! of `duckosm info` / `duckosm way` (Python, 2026-11-17) on the database of this config: Monaco's sample PBF in its boundary,
//! three modes, no sea, no levels, no tile columns (the Python build has none). `info` leaves out the build time and the version (its first line, and two keys of the JSON).

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

/// `boundary` writes the file of duckOSM's `duckosm boundary` (Python, 2026-11-17), from the borders of Monaco's sample PBF,
/// offline: by name ("Monaco": the country, with the other matches) and by a name with other case and no hyphen.
#[test]
fn boundary_equals_duckosm() {
    use duckosm_build::place::{find_boundary, write_boundary, Wanted};
    let pbf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/sample/monaco.osm.pbf");
    let out = std::env::temp_dir().join(format!("duckosm-tools-boundary-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    for (name, file) in [("Monaco", "boundary_monaco.geojson"), ("monte carlo", "boundary_monte_carlo.geojson")] {
        let (geom, info) = find_boundary(Wanted::Name(name), Some(&pbf), true).unwrap();
        let path = out.join(file);
        write_boundary(geom, info.clone(), &path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), golden(file), "{name}");
        if name == "Monaco" {
            // the other matches of the name, best first
            let others: Vec<i64> = info["others"].as_array().unwrap().iter().map(|o| o["osm_id"].as_i64().unwrap()).collect();
            assert_eq!(others.first(), Some(&2220322), "{others:?}");
        }
    }
    let e = find_boundary(Wanted::Name("Nowhere"), Some(&pbf), true).unwrap_err();
    assert_eq!(e, "no boundary found for 'Nowhere' in monaco.osm.pbf. Try a more specific name (e.g. 'Södermalm, Stockholm') or --osm-id.");
    let _ = std::fs::remove_dir_all(&out);
}

/// `admin` and `extract` give the tables of duckOSM's `duckosm admin` and `duckosm extract` (Python, 2026-11-17): admin on the
/// database of `info_and_way_equal_duckosm` with Monaco's sample PBF, then `extract --name "monte carlo"` from it. Each row: the
/// number of rows of a table and the MD5 of its rows as text (geometries as WKB).
#[test]
fn admin_and_extract_equal_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-tools-extract-{}", std::process::id()));
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
    let db = out.join("monaco.duckdb");
    duckosm_tools::admin::admin(&s.join("monaco.osm.pbf"), &db, None).unwrap();
    let rows = |db: &std::path::Path, t: &str| -> (i64, Option<String>) {
        let con = duckdb::Connection::open(db).unwrap();
        con.execute_batch("LOAD spatial").unwrap();
        let mut st = con.prepare(&format!("SELECT column_name, column_type FROM (DESCRIBE {t})")).unwrap();
        let cols: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        let x = cols
            .iter()
            .map(|(c, ty)| if ty.starts_with("GEOMETRY") { format!("hex(ST_AsWKB(\"{c}\"))") } else { format!("\"{c}\"::VARCHAR") })
            .collect::<Vec<_>>()
            .join(", ");
        con.query_row(&format!("SELECT count(*), md5(string_agg(x, ',' ORDER BY x)) FROM (SELECT concat_ws('|', {x}) x FROM {t})"), [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap()
    };
    let con = duckdb::Connection::open(&db).unwrap();
    con.execute_batch("LOAD spatial").unwrap();
    let admin: (i64, String) = con
        .query_row(
            "SELECT count(*), md5(string_agg(x, ',' ORDER BY x)) FROM (SELECT concat_ws('|', osm_id, name, name_en, admin_level, parent_osm_id, hex(ST_AsWKB(geometry))) x FROM main.admin_boundaries)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(admin, (11, "7295fecdb15e821e753b7647c0249fba".to_string()));
    drop(con);

    let part = out.join("part.duckdb");
    duckosm_tools::extract::extract(&db, &part, duckosm_tools::extract::Area::Name("monte carlo")).unwrap();
    for (t, n, md5) in [
        ("driving.edges", 671, "9a75a6265ec956886941744670a31486"),
        ("driving.nodes", 416, "2825d84df6b8a74bb07d5129d5a572fd"),
        ("driving.edge_graph", 1485, "b818e64e360472cc20a7026a4ef6b2ae"),
        ("driving.private_edges", 111, "eedc476d857bf42912e5fa2086870f04"),
        ("driving.turn_restrictions", 9, "ef0a0bb2692d44970a23cc950f4b9acb"),
        ("driving.ways", 208, "c8b2c86706c3798d830b105f0afc1a61"),
        ("driving.way_nodes", 1648, "e9c02e1e78703b08f0c14f7c3f704e0a"),
        ("walking.edges", 2610, "9c0b0946a8da25c47171374b608cc975"),
        ("walking.edge_graph", 7830, "1ea201277fd201cc395de953410c7d0c"),
        ("cycling.edges", 2336, "1839042a1dca413fb631bf07eef65702"),
        ("cycling.nodes", 982, "31f72e03393ba5dbb4f23b00d6eb5897"),
        ("main.boundary", 1, "81413ffaf1d6551c1f21b329820b5ffe"),
        ("main.admin_boundaries", 1, "bac5003260a48d438b700a28b45de311"),
    ] {
        assert_eq!(rows(&part, t), (n, Some(md5.to_string())), "{t}");
    }
    let con = duckdb::Connection::open(&part).unwrap();
    let meta: (f64, f64, i32, String) = con
        .query_row("SELECT center_lat, center_lon, initial_zoom, timezone FROM main.visualization_metadata", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!(meta, (43.74031361981264, 7.426073129259986, 15, "Europe/Monaco".to_string()));
    let _ = std::fs::remove_dir_all(&out);
}

/// The table summary of `elevation_equals_duckosm`: the same SQL as `summary` in tools/make_golden_elevation.py.
fn elevation_summary(db: &std::path::Path) -> String {
    let con = duckosm_tools::open_read_only(db).unwrap();
    let mut out = String::new();
    for m in ["cycling", "driving", "walking"] {
        for (t, key) in [("nodes", "node_id"), ("edges", "edge_id")] {
            let mut st = con
                .prepare(
                    "SELECT column_name FROM duckdb_columns() WHERE schema_name = ? AND table_name = ? \
                     AND (column_name LIKE 'ele%' OR column_name LIKE 'z\\_%' ESCAPE '\\') ORDER BY column_index",
                )
                .unwrap();
            let cols: Vec<String> = st.query_map([m, t], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
            let sel = std::iter::once(key.to_string()).chain(cols.iter().cloned()).collect::<Vec<_>>().join(", ");
            let (n, h): (i64, String) = con
                .query_row(
                    &format!("SELECT count(*), md5(string_agg(t::VARCHAR, chr(10) ORDER BY t::VARCHAR)) FROM (SELECT {sel} FROM {m}.{t}) t"),
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            out += &format!("{m}.{t}\t{}\t{n}\t{h}\n", cols.join(","));
        }
    }
    let mut st = con
        .prepare("SELECT t::VARCHAR FROM (SELECT * EXCLUDE (uri, sampled_at, duckosm_version) FROM main.elevation_metadata) t ORDER BY ele_column")
        .unwrap();
    for r in st.query_map([], |r| r.get::<_, String>(0)).unwrap() {
        out += &(r.unwrap() + "\n");
    }
    out
}

/// `elevation` writes the heights of duckOSM's `duckosm elevation` (Python, 2026-11-17, tests/data/golden_tools/monaco_elevation.tsv):
/// the Monaco crop of Copernicus GLO-30 as the reference ground in `ele`, then a smaller crop (a part of Monaco: the other nodes get
/// the fill value) in `ele_dsm` of two modes. The database stays in <target>/tmp/duckosm-tools-elevation/ for
/// tools/make_golden_elevation.py.
#[test]
fn elevation_equals_duckosm() {
    use duckosm_tools::elevation::{to_elevation, Options};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-tools-elevation");
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
    let db = out.join("elevation.duckdb");
    std::fs::copy(out.join("monaco.duckdb"), &db).unwrap();

    let o = |dem: &str| Options {
        dem: Some(root.join(dem).to_string_lossy().into_owned()),
        source: "auto".into(),
        modes: Vec::new(),
        nodata_fill: 0.0,
        suffix: String::new(),
        product: None,
        vertical_datum: None,
        license: None,
        reference: true,
    };
    let whole = Options {
        product: Some("DTM".into()),
        vertical_datum: Some("EGM2008".into()),
        license: Some("Copernicus open (attribution)".into()),
        ..o("rust/duckosm-build/tests/data/monaco-copernicus-glo30.tif")
    };
    // the ground rule: a DEM in `ele` needs its product, datum and license
    assert!(to_elevation(&db, &Options { license: None, ..o("rust/duckosm-build/tests/data/monaco-copernicus-glo30.tif") }).unwrap_err().contains("reference ground"));
    let r = to_elevation(&db, &whole).unwrap();
    assert_eq!((r.n_nodes, r.n_nodata, r.column.as_str()), (10986, 0, "ele"));
    let crop = Options { suffix: "dsm".into(), nodata_fill: -5.5, modes: vec!["walking".into(), "driving".into()], ..o("tests/data/golden_tools/monaco_dem_crop.tif") };
    let r = to_elevation(&db, &crop).unwrap();
    assert_eq!((r.n_nodes, r.n_nodata, r.column.as_str()), (6348, 1194, "ele_dsm"));
    assert_eq!(elevation_summary(&db), golden("monaco_elevation.tsv"));
    let e = to_elevation(&db, &Options { modes: vec!["boat".into()], ..whole }).unwrap_err();
    assert_eq!(e, "mode(s) ['boat'] not found (present: ['cycling', 'driving', 'walking'])");
}
