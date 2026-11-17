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

/// Shapefiles that gis-debug must fail: the same SQL as `bad_files` in tools/make_golden_exports.py.
fn bad_files(d: &Path) {
    std::fs::create_dir_all(d).unwrap();
    let con = duckosm_export::open(db()).unwrap();
    let d = d.display();
    con.execute_batch(&format!(
        "COPY (SELECT edge_id::DOUBLE AS edge_id, highway, geometry FROM driving.edges) TO '{d}/m_edges_driving.shp' \
         (FORMAT gdal, DRIVER 'ESRI Shapefile', SRS 'EPSG:4326');
         COPY (SELECT CASE WHEN edge_id % 7 = 0 THEN NULL ELSE edge_id END AS edge_id, highway, geometry FROM walking.edges) \
         TO '{d}/m_edges_walking.shp' (FORMAT gdal, DRIVER 'ESRI Shapefile', SRS 'EPSG:3857')"
    ))
    .unwrap();
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
    // gis-debug on these files (the GeoPackage of one file needs ogr2ogr, as in the golden), and on bad shapefiles
    assert!(d.join("monaco.gpkg").is_file(), "the gis-debug golden needs the GeoPackage of ogr2ogr (GDAL)");
    bad_files(&d.join("bad"));
    for (x, golden) in [("monaco.gpkg", "gpkg"), ("shp", "shp"), ("bad", "bad")] {
        let page = d.join(format!("{x}.html"));
        duckosm_export::gis_debug::write_debug(&d.join(x), Some(db()), &page, None).unwrap();
        same(&page, &format!("gis_debug_{golden}.html.gz"));
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

/// The lanelets of a Lanelet2 file: (tags, left boundary, right boundary), in the order of the file.
fn lanelets(text: &str) -> Vec<(Vec<String>, Vec<(f64, f64)>, Vec<(f64, f64)>)> {
    let attr = |l: &str, k: &str| -> String {
        let key = format!(" {k}=\"");
        l.find(&key).map(|i| l[i + key.len()..].split('"').next().unwrap().to_string()).unwrap_or_default()
    };
    let mut nodes = std::collections::HashMap::new();
    let mut ways: std::collections::HashMap<String, Vec<(f64, f64)>> = std::collections::HashMap::new();
    let mut out = Vec::new();
    let (mut way, mut rel): (Option<String>, Option<(Vec<String>, String, String)>) = (None, None);
    for l in text.lines().map(str::trim) {
        if l.starts_with("<node ") {
            nodes.insert(attr(l, "id"), (attr(l, "lon").parse::<f64>().unwrap(), attr(l, "lat").parse::<f64>().unwrap()));
        } else if l.starts_with("<way ") {
            way = Some(attr(l, "id"));
            ways.insert(attr(l, "id"), vec![]);
        } else if l.starts_with("<nd ") {
            ways.get_mut(way.as_ref().unwrap()).unwrap().push(nodes[&attr(l, "ref")]);
        } else if l.starts_with("<relation ") {
            rel = Some((vec![], String::new(), String::new()));
        } else if l.starts_with("<member ") {
            let r = rel.as_mut().unwrap();
            if attr(l, "role") == "left" { r.1 = attr(l, "ref") } else { r.2 = attr(l, "ref") }
        } else if l.starts_with("<tag ") && rel.is_some() {
            rel.as_mut().unwrap().0.push(format!("{}={}", attr(l, "k"), attr(l, "v")));
        } else if l.starts_with("</relation>") {
            let (t, a, b) = rel.take().unwrap();
            out.push((t, ways[&a].clone(), ways[&b].clone()));
        }
    }
    out
}

/// The Hausdorff distance in metres of two lon/lat lines (each point to the other line, both ways).
fn hausdorff_m(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    let k = a[0].1.to_radians().cos() * 111320.0;
    let m = |p: (f64, f64)| (p.0 * k, p.1 * 111320.0);
    let seg = |p: (f64, f64), s: (f64, f64), e: (f64, f64)| {
        let (p, s, e) = (m(p), m(s), m(e));
        let (dx, dy) = (e.0 - s.0, e.1 - s.1);
        let l2 = dx * dx + dy * dy;
        let t = if l2 == 0.0 { 0.0 } else { (((p.0 - s.0) * dx + (p.1 - s.1) * dy) / l2).clamp(0.0, 1.0) };
        (p.0 - s.0 - t * dx).hypot(p.1 - s.1 - t * dy)
    };
    let one = |x: &[(f64, f64)], y: &[(f64, f64)]| {
        x.iter().map(|p| y.windows(2).map(|w| seg(*p, w[0], w[1])).fold(f64::INFINITY, f64::min)).fold(0.0, f64::max)
    };
    one(a, b).max(one(b, a))
}

/// Lanelet2: the same lanelets, tags and boundaries as Python. A boundary point may move by 1 mm (the offset curve is
/// pure Rust, not GEOS) and by the rounding of the 8 decimals of the file (1.1 mm); the node and way ids are counters.
#[test]
fn lanelet2_equals_python() {
    let o = out("monaco.lanelet2.osm");
    duckosm_export::lanelet2::to_lanelet2(gmns(), &o, "driving", 0.01).unwrap();
    let (got, want) = (lanelets(&text(&o)), lanelets(&text(&data().join("exports/monaco.lanelet2.osm.gz"))));
    assert_eq!(got.len(), want.len());
    let mut bad = Vec::new();
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g.0, w.0);
        for (side, x, y) in [("left", &g.1, &w.1), ("right", &g.2, &w.2)] {
            let h = hausdorff_m(x, y);
            if h > 1e-3 + 1.2e-3 {
                bad.push(format!("{} {side}: {h:.4} m", g.0.iter().find(|t| t.starts_with("duckosm:edge_id")).unwrap()));
            }
        }
    }
    assert!(bad.is_empty(), "{} boundaries differ from Python:\n{}", bad.len(), bad.join("\n"));
}
