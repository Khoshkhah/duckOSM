//! Each Rust export of Monaco equals the output of the Python export (src/duckosm/), made before the port.
//!
//! The test builds Monaco (three modes, its boundary, the levels, the base-map layers so that raw.* stays in the file) into
//! `<target>/tmp/duckosm-exports/golden.duckdb`. The golden files in tests/data/exports/ are the output of the Python
//! exports on that database: tools/make_golden_exports.py. Run this test once, then the tool, to make them again.
//! The test needs DuckDB's spatial extension (the first `INSTALL` downloads it).

use duckosm_build::{build, LevelsMethod, NetworkMode, Params};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
}

/// The Monaco database, built once for all tests of this file.
fn db() -> &'static Path {
    static DB: OnceLock<PathBuf> = OnceLock::new();
    DB.get_or_init(|| {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-exports");
        let p = Params {
            modes: vec![NetworkMode::Driving, NetworkMode::Walking, NetworkMode::Cycling],
            geoparquet: false,
            duckdb: true,
            name: "golden".into(),
            boundary: Some(data().join("monaco.geojson")),
            levels: LevelsMethod::Tags,   // visualization.edge_levels: the GeoParquet edges files get its columns
            bus_routes: false,
            features: true,
            built_at: Some(0),
            ..Params::default()
        };
        build(&data().join("monaco_sample.osm.pbf"), &out, &p).unwrap();
        out.join("golden.duckdb")
    })
}

fn out(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-exports-out");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// The text of a file; gunzipped when its name ends with .gz (the gzip header has a time and a file name).
fn text(p: &Path) -> String {
    let b = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    if p.extension().is_some_and(|e| e == "gz") {
        let mut s = String::new();
        flate2::read::GzDecoder::new(&b[..]).read_to_string(&mut s).unwrap();
        s
    } else {
        String::from_utf8(b).unwrap()
    }
}

/// The two files are equal: else the first line that differs.
fn same(got: &Path, golden: &str) {
    let (a, b) = (text(got), text(&data().join("exports").join(golden)));
    if a != b {
        let (n, (x, y)) = a.lines().zip(b.lines()).enumerate().find(|(_, (x, y))| x != y).unwrap_or((0, ("", "")));
        panic!("{golden} differs from the Python export (lines {} and {}), line {}:\n rust:   {x}\n python: {y}", a.lines().count(), b.lines().count(), n + 1);
    }
}

#[test]
fn matsim_equals_python() {
    let o = out("monaco_network.xml.gz");
    duckosm_export::matsim::to_matsim(db(), &o, "driving", None, true).unwrap();
    same(&o, "monaco_network.xml.gz");
    let o = out("monaco_all_network.xml");
    duckosm_export::matsim::to_matsim(db(), &o, "all", Some("EPSG:3035"), false).unwrap();
    same(&o, "monaco_all_network.xml.gz");
}

#[test]
fn railml_equals_python() {
    let o = out("monaco.railml.xml");
    duckosm_export::railml::to_railml(db(), &o).unwrap();
    same(&o, "monaco.railml.xml");
}

/// One line of a layer: its columns, its number of rows and the MD5 of its rows as text. `src`: a FROM item. The same SQL
/// as `summary` in tools/make_golden_exports.py: if you change one, change the other.
fn summary(src: &str) -> String {
    let con = duckdb::Connection::open_in_memory().unwrap();
    con.execute_batch("INSTALL spatial; LOAD spatial").unwrap();
    let cols: String = con
        .query_row(&format!("SELECT string_agg(column_name || ':' || column_type, ',') FROM (DESCRIBE SELECT * FROM {src})"), [], |r| r.get(0))
        .unwrap();
    let (n, h): (i64, String) = con
        .query_row(
            &format!("SELECT count(*), md5(coalesce(string_agg(t::VARCHAR, chr(10) ORDER BY t::VARCHAR), '')) FROM {src} t"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    format!("{cols}\t{n}\t{h}")
}

#[test]
fn gis_equals_python() {
    use duckosm_export::gis::to_gis;
    let golden = std::fs::read_to_string(data().join("exports/monaco_gis_summary.tsv")).unwrap();
    let mut got = vec!["export\tlayer\tcolumns\trows\tmd5".to_string()];
    let d = out("gis");
    let _ = std::fs::remove_dir_all(&d);
    let gp = d.join("gp");
    let s = to_gis(db(), &gp, &[], "geoparquet", true, None, "EPSG:4326").unwrap();
    for (lyr, _) in &s.layers {
        got.push(format!("geoparquet\t{lyr}\t{}", summary(&format!("read_parquet('{}/{lyr}.parquet')", gp.display()))));
    }
    same(&gp.join("layers.json"), "monaco_geoparquet_layers.json");
    // without ogr2ogr (the CI), each layer is its own file: the same rows
    let s = to_gis(db(), &d.join("monaco.gpkg"), &[], "gpkg", true, Some("monaco"), "EPSG:4326").unwrap();
    for (lyr, _) in &s.layers {
        let src = match &s.path {
            Some(p) => format!("ST_Read('{}', layer = '{lyr}')", p.display()),
            None => format!("ST_Read('{}/monaco_{lyr}.gpkg')", d.display()),
        };
        got.push(format!("gpkg\t{lyr}\t{}", summary(&src)));
    }
    let shp = d.join("shp");
    let s = to_gis(db(), &shp, &["driving".into()], "shp", true, Some("monaco"), "EPSG:4326").unwrap();
    for (lyr, _) in &s.layers {
        got.push(format!("shp\t{lyr}\t{}", summary(&format!("ST_Read('{}/monaco_{lyr}.shp')", shp.display()))));
        let prj = std::fs::read_to_string(shp.join(format!("monaco_{lyr}.prj"))).unwrap();
        got.push(format!("shp\t{lyr}.prj\t{}\t\t", prj.trim()));
    }
    let got = got.join("\n") + "\n";
    let bad: Vec<String> = got.lines().zip(golden.lines()).filter(|(a, b)| a != b).map(|(a, b)| format!(" rust:   {a}\n python: {b}")).collect();
    assert!(bad.is_empty() && got.lines().count() == golden.lines().count(), "differs from the Python export:\n{}", bad.join("\n"));
}

/// A GMNS db from the Parquet files of tests/data/exports/gmns_driving/ (the columns that the exports read), made once. The
/// same SQL as `gmns_db` in tools/make_golden_exports.py.
fn gmns() -> &'static Path {
    static DB: OnceLock<PathBuf> = OnceLock::new();
    DB.get_or_init(|| {
        let p = out("monaco_gmns.duckdb");
        let _ = std::fs::remove_file(&p);
        let con = duckdb::Connection::open(&p).unwrap();
        con.execute_batch("INSTALL spatial; LOAD spatial; CREATE SCHEMA gmns_driving").unwrap();
        for t in ["link", "movement", "node", "lane"] {
            let f = data().join(format!("exports/gmns_driving/{t}.parquet"));
            con.execute_batch(&format!("CREATE TABLE gmns_driving.{t} AS SELECT * FROM read_parquet('{}')", f.display())).unwrap();
        }
        p
    })
}

#[test]
fn opendrive_equals_python() {
    use duckosm_export::opendrive::to_opendrive;
    let o = out("monaco.xodr");
    to_opendrive(db(), &o, "driving", None, false).unwrap();
    same(&o, "monaco.xodr.gz");
    let o = out("monaco_junctions.xodr");
    to_opendrive(gmns(), &o, "driving", None, true).unwrap();
    same(&o, "monaco_junctions.xodr.gz");
}

#[test]
fn matsim_lanes_equals_python() {
    let d = out("matsim_lanes");
    let s = duckosm_export::matsim_lanes::to_matsim_lanes(gmns(), &d, "driving", true, 90).unwrap();
    assert!(s.lanes_links > 0 && s.signal_systems.unwrap() > 0);
    for f in ["lanes", "signalSystems", "signalGroups", "signalControl"] {
        same(&d.join(format!("{f}.xml")), &format!("matsim_lanes_{f}.xml.gz"));
    }
}

#[test]
fn sumo_equals_python() {
    for m in ["driving", "walking"] {
        let d = out(&format!("sumo_{m}"));
        let s = duckosm_export::sumo::to_sumo(db(), &d, m, "monaco", true, None, false).unwrap();
        assert!(s.net.is_none() && s.con.is_some());
        for f in ["nod", "edg", "con"] {
            same(&d.join(format!("monaco.{f}.xml")), &format!("sumo_{m}.{f}.xml.gz"));
        }
    }
}
