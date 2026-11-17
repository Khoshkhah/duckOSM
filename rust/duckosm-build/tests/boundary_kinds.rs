//! The kinds of boundary that are not a file, and `main.boundary_cells`, equal duckOSM's Python build (the reference, the
//! Python engine, 2026-11-17). The two builds: Monaco's sample PBF, the driving mode, no base-map layers, no levels, no bus
//! routes; (1) `boundary.place: Monaco`, (2) `boundary.path: data/sample/monaco.geojson` with `options.boundary_cells: true`
//! and `boundary_cell_resolutions: [7, 9, 10]`. It needs DuckDB's spatial extension, as the build does.

use duckdb::Connection;
use std::path::{Path, PathBuf};

fn build(name: &str, boundary: &str, options: &str) -> (PathBuf, Connection) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let sample = root.join("data/sample");
    let cfg = format!(
        "name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}/monaco.osm.pbf}}\nboundary:\n{}\nmodes: [driving]\n\
         options:\n  sea: false\n  levels: false\n  build_features: false\n  bus_routes: false\n{options}\n",
        out.display(),
        sample.display(),
        boundary.replace("SAMPLE", &sample.display().to_string())
    );
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
    let con = Connection::open(out.join("monaco.duckdb")).unwrap();
    con.execute_batch("LOAD spatial").unwrap();
    (out, con)
}

fn one<T: duckdb::types::FromSql>(con: &Connection, sql: &str) -> T {
    con.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn a_place_name_gives_the_border_of_the_pbf() {
    let (out, con) = build("place", "  place: Monaco", "");
    // the same border as GDAL's ogr2ogr gives duckOSM: the country (its level 2 relation, with the sea)
    let row: (String, i64, i64, f64, String, i64) = con
        .query_row("SELECT name, osm_id, admin_level, area_km2, source, ST_NPoints(geom) FROM main.boundary", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })
        .unwrap();
    assert_eq!(row, ("Monaco".into(), 1124039, 2, 79.8, "PBF monaco.osm.pbf".into(), 520));
    assert_eq!(one::<i64>(&con, "SELECT count(*) FROM driving.edges"), 2844);
    assert!(Path::new(&out.join("monaco.boundary.geojson")).exists());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_boundary_cells_are_the_cells_of_duckosm() {
    let (out, con) = build("cells", "  path: SAMPLE/monaco.geojson", "  boundary_cells: true\n  boundary_cell_resolutions: [7, 9, 10]");
    let mut st = con
        .prepare("SELECT resolution, count(*), md5(string_agg(h3_id, ',' ORDER BY h3_id)) FROM main.boundary_cells GROUP BY 1 ORDER BY 1")
        .unwrap();
    let rows: Vec<(i32, i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect();
    let want = [
        (7, 3, "45e174993a359d448f2b08cf1b5fbb1e"),
        (9, 40, "6d1bd2473d505e1d9a8125a7cd627f88"),
        (10, 204, "a050e159a9a2a07f3eb0cf3ffa255e1d"),
    ];
    assert_eq!(rows, want.map(|(r, n, m)| (r, n, m.to_string())).to_vec());
    let _ = std::fs::remove_dir_all(&out);
}
