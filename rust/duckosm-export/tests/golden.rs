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
            tile_indexing: false, // the Python build has no tile columns
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

/// Lanelet2: the same lanelets and tags as Python, and each boundary within 1 mm of Python's (Hausdorff). The offset curve
/// is the pure Rust port of GEOS (`src/offset.rs`, `src/geos.rs`); on Monaco and Tartu the file has the same bytes.
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
            if h > 1e-3 {
                bad.push(format!("{} {side}: {h:.4} m", g.0.iter().find(|t| t.starts_with("duckosm:edge_id")).unwrap()));
            }
        }
    }
    assert!(bad.is_empty(), "{} boundaries differ from Python:\n{}", bad.len(), bad.join("\n"));
}

/// One line for each column of each table of the schemas `LIKE like`: its type, the rows and the MD5 of its values as text,
/// sorted. The same SQL as `columns` in tools/make_golden_exports.py.
fn gmns_columns(db: &Path, like: &str) -> Vec<String> {
    let con = duckdb::Connection::open(db).unwrap();
    con.execute_batch("INSTALL spatial; LOAD spatial").unwrap();
    let mut st = con
        .prepare(&format!(
            "SELECT table_schema || '.' || table_name, column_name, data_type FROM information_schema.columns \
             WHERE table_schema LIKE '{like}' ESCAPE '\\' ORDER BY 1, ordinal_position",
        ))
        .unwrap();
    let cols: Vec<(String, String, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(|r| r.unwrap()).collect();
    let mut lines = vec!["table\tcolumn\ttype\trows\tmd5".to_string()];
    for (t, c, typ) in cols {
        let v = if typ.starts_with("GEOMETRY") { format!("ST_AsText(\"{c}\")") } else { format!("\"{c}\"::VARCHAR") };
        let (n, h): (i64, String) = con
            .query_row(
                &format!(
                    "SELECT count(*), md5(coalesce(string_agg(coalesce('=' || {v}, '-'), chr(10) ORDER BY coalesce('=' || {v}, '-')), '')) FROM {t}"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        lines.push(format!("{t}\t{c}\t{typ}\t{n}\t{h}"));
    }
    lines
}

/// The computed geometry and the measures taken on it, which `gmns_lanes_by_properties` checks, not equality with Python (Kaveh,
/// 2026-11-18; docs/design/all_in_rust.md): the lane lines and the crossings, whose overlays are the `geo` crate's here and GEOS's
/// in Python. The micro cells are cut from the lane lines, so their points and lengths follow them. An entry is `table.column` (in
/// every schema), `schema.table` or `schema.table.column`.
const GMNS_BY_PROPERTIES: [&str; 16] = [
    "lane.geom",
    "lane.geom_cut",
    "lane_connector.geom",
    "crossing.width",
    "crossing.length",
    "crossing.geom",
    "lane_crossing.start_lr",
    "lane_crossing.end_lr",
    "lane_crossing.across_from",
    "lane_crossing.across_to",
    "micro_node.x_coord",
    "micro_node.y_coord",
    "micro_node.geom",
    "micro_link.length",
    "micro_link.geometry",
    "micro_link.geom",
];

/// The lane geometry: checked by its properties (`gmns_lanes_by_properties`), not by equality with Python (Kaveh, 2026-11-18;
/// docs/design/all_in_rust.md, "How the lane geometry is checked").
#[test]
fn gmns_lanes_by_properties() {
    let (a, b) = (out("gmns_a").join("monaco_gmns.duckdb"), out("gmns_b").join("monaco_gmns.duckdb"));
    duckosm_export::gmns::to_gmns(db(), &a, &[]).unwrap();
    duckosm_export::gmns::to_gmns(db(), &b, &[]).unwrap();
    let lanes = |p: &Path| -> Vec<String> {
        gmns_columns(p, "gmns\\_%").into_iter().filter(|l| l.contains(".lane\tgeom\t")).collect()
    };
    assert_eq!(lanes(&a), lanes(&b), "two runs give other lane geometry");
    let got = duckosm_export::gmns_lanes::lane_report(&a).unwrap().join("\n") + "\n";
    let path = data().join("exports/monaco_gmns_lanes.tsv");
    if std::env::var("GMNS_LANES_WRITE").is_ok() {
        std::fs::write(&path, &got).unwrap(); // the summary is made by the Rust export itself: GMNS_LANES_WRITE=1 cargo test ...
    }
    assert_eq!(got, text(&path), "the lane checks changed (GMNS_LANES_WRITE=1 writes them again after a wanted change)");
}

/// gmns-viz: the page of a db made from tests/data/exports/gmns_viewer/ (the columns that the viewer reads, from Python's GMNS
/// export with --meso --micro) has the MD5 and the size of Python's page of the same db. The same SQL as `viewer` in
/// tools/make_golden_exports.py.
#[test]
fn gmns_viewer_equals_python() {
    let p = out("viewer").join("monaco_gmns.duckdb");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&p);
    let con = duckdb::Connection::open(&p).unwrap();
    con.execute_batch("INSTALL spatial; LOAD spatial; CREATE SCHEMA gmns_driving; CREATE SCHEMA meso_driving; CREATE SCHEMA micro_driving").unwrap();
    for t in ["gmns_driving.lane", "gmns_driving.node", "gmns_driving.link", "gmns_driving.movement", "meso_driving.meso_link", "meso_driving.meso_node", "micro_driving.micro_link"] {
        let f = data().join(format!("exports/gmns_viewer/{t}.parquet"));
        con.execute_batch(&format!("CREATE TABLE {t} AS SELECT * FROM read_parquet('{}')", f.display())).unwrap();
    }
    drop(con);
    let html = out("viewer").join("v.html");
    duckosm_export::gmns_viewer::write_viewer(&p, &html, "driving").unwrap();
    let page = std::fs::read(&html).unwrap();
    let md5: String = duckdb::Connection::open_in_memory().unwrap().query_row("SELECT md5(?)", [String::from_utf8(page.clone()).unwrap()], |r| r.get(0)).unwrap();
    assert_eq!(format!("driving\t{md5}\t{}\n", page.len()), text(&data().join("exports/monaco_gmns_viewer.tsv")), "the viewer page differs from Python's");
}

/// GMNS `--walk-frame` (driving and walking): every column of gmns_walking but the lane lines equals Python; the lines are the
/// same on two runs, and the sidewalk lanes that still touch a driving lane are counted (`monaco_gmns_walk_frame.tsv`, made by
/// the Rust export, GMNS_LANES_WRITE=1).
#[test]
fn gmns_walk_frame() {
    let opts = duckosm_export::gmns::GmnsOptions { modes: vec!["driving".into(), "walking".into()], walk_frame: true, ..Default::default() };
    let (a, b) = (out("walk_frame_a").join("monaco_gmns.duckdb"), out("walk_frame_b").join("monaco_gmns.duckdb"));
    duckosm_export::gmns::to_gmns_with(db(), &a, &opts).unwrap();
    duckosm_export::gmns::to_gmns_with(db(), &b, &opts).unwrap();
    let got = gmns_columns(&a, "gmns\\_walking");
    assert_eq!(got, gmns_columns(&b, "gmns\\_walking"), "two runs differ");
    let bad = columns_differ(&got, &text(&data().join("exports/monaco_gmns_walk_frame_columns.tsv")));
    assert!(bad.is_empty(), "{} columns differ from the Python export:\n{}", bad.len(), bad.join("\n"));
    // the sidewalk lanes (and their points) that still lie on a driving lane, less 0.1 m from its edge
    let con = duckdb::Connection::open(&a).unwrap();
    con.execute_batch("LOAD spatial").unwrap();
    let (lanes, points): (i64, i64) = con
        .query_row(
            "WITH d AS (SELECT ST_Buffer(ST_Transform(geom, 'EPSG:4326', 'EPSG:32632', always_xy := true), COALESCE(width, 3.25) / 2 - 0.1) AS g
                        FROM gmns_driving.lane WHERE geom IS NOT NULL AND allowed_uses IN ('auto', 'bus', 'bus,bike')),
                  s AS (SELECT l.lane_id, unnest(ST_Dump(ST_Points(ST_Transform(l.geom, 'EPSG:4326', 'EPSG:32632', always_xy := true)))).geom AS p
                        FROM gmns_walking.lane l JOIN gmns_walking.link k USING (link_id) WHERE k.footway = 'sidewalk')
             SELECT count(DISTINCT lane_id), count(*) FROM s WHERE EXISTS (SELECT 1 FROM d WHERE ST_Intersects(d.g, s.p))",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let got = format!("sidewalk lanes on a driving lane\t{lanes}\nsidewalk points on a driving lane\t{points}\n");
    let path = data().join("exports/monaco_gmns_walk_frame.tsv");
    if std::env::var("GMNS_LANES_WRITE").is_ok() {
        std::fs::write(&path, &got).unwrap();
    }
    assert_eq!(got, text(&path), "the walk frame checks changed (GMNS_LANES_WRITE=1 writes them again after a wanted change)");
}

/// GMNS: each column of each table that the Rust export makes so far equals the Python export (the same type, rows and values).
#[test]
fn gmns_equals_python() {
    // its own folder: gmns() writes a file of the same name (the name is the dataset_name of the export)
    let o = out("gmns").join("monaco_gmns.duckdb");
    let csv = out("gmns").join("csv");
    let _ = std::fs::remove_dir_all(&csv);
    let opts = duckosm_export::gmns::GmnsOptions { combined: true, to_csv: Some(csv.clone()), ..Default::default() };
    duckosm_export::gmns::to_gmns_with(db(), &o, &opts).unwrap();
    // the CSVs: the header and the sorted rows of each file (the order of some tables is that of a parallel query)
    let mut files: Vec<PathBuf> = Vec::new();
    let mut todo = vec![csv.clone()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() {
                todo.push(e.path());
            } else if e.path().extension().is_some_and(|x| x == "csv") {
                files.push(e.path());
            }
        }
    }
    files.sort();
    let got_csv: String = files
        .iter()
        .map(|p| {
            let t = std::fs::read_to_string(p).unwrap();
            let mut lines = t.split('\n');
            let head = lines.next().unwrap().to_string();
            let mut rows: Vec<&str> = lines.filter(|r| !r.is_empty()).collect();
            rows.sort();
            let text = std::iter::once(head.as_str()).chain(rows).collect::<Vec<_>>().join("\n");
            let digest = duckdb::Connection::open_in_memory().unwrap().query_row("SELECT md5(?)", [text], |r| r.get::<_, String>(0)).unwrap();
            format!("{}\t{digest}\n", p.strip_prefix(&csv).unwrap().display())
        })
        .collect();
    assert_eq!(got_csv, text(&data().join("exports/monaco_gmns_csv.tsv")), "the GMNS CSVs differ from Python's");
    // gmns_check: the checks and the graph report
    let mut check: Vec<String> =
        duckosm_export::gmns_check::check_gmns(&o, &[]).unwrap().into_iter().map(|(s, n, bad, total)| format!("{s}\t{n}\t{bad}\t{total}")).collect();
    for g in duckosm_export::gmns_check::graph_report(&o, &[]).unwrap() {
        check.push(format!("graph\t{}\t{}\t{}\t{}\t{}\t{}", g.schema, g.nodes, g.links, g.biggest, g.parts, g.dead_ends));
    }
    assert_eq!(check.join("\n") + "\n", text(&data().join("exports/monaco_gmns_check.tsv")), "gmns_check differs from Python's");
    let mut bad = columns_differ(&gmns_columns(&o, "gmns\\_%"), &text(&data().join("exports/monaco_gmns_columns.tsv")));
    // meso and micro of driving and cycling
    let modes = ["driving".to_string(), "cycling".to_string()];
    duckosm_export::gmns_meso::to_meso(&o, &modes, 6.0).unwrap();
    duckosm_export::gmns_meso::to_micro(&o, &modes, 7.0).unwrap();
    let mut got = gmns_columns(&o, "meso\\_%");
    got.extend(gmns_columns(&o, "micro\\_%").into_iter().skip(1));
    bad.extend(columns_differ(&got, &text(&data().join("exports/monaco_gmns_meso_columns.tsv"))));
    // --gtfs: the stops of a feed as location rows
    let g = out("gmns").join("gtfs.duckdb");
    let opts = duckosm_export::gmns::GmnsOptions { gtfs: vec![data().join("exports/monaco_gtfs.zip")], ..Default::default() };
    duckosm_export::gmns::to_gmns_with(db(), &g, &opts).unwrap();
    let got: Vec<String> = gmns_columns(&g, "gmns\\_%").into_iter().filter(|l| l.starts_with("table\t") || l.contains(".location\t")).collect();
    bad.extend(columns_differ(&got, &text(&data().join("exports/monaco_gmns_gtfs_location.tsv"))));
    assert!(bad.is_empty(), "{} columns differ from the Python export:\n{}", bad.len(), bad.join("\n"));
}

/// The lines of `golden` (from `columns`) that `got` does not have, except the columns of `GMNS_BY_PROPERTIES` and the tables
/// that Rust does not make.
fn columns_differ(got: &[String], golden: &str) -> Vec<String> {
    let key = |l: &str| l.split('\t').take(2).collect::<Vec<_>>().join("\t");
    let tables: std::collections::HashSet<&str> = got.iter().skip(1).map(|l| l.split('\t').next().unwrap()).collect();
    let mine: std::collections::HashMap<String, &String> = got.iter().map(|l| (key(l), l)).collect();
    let mut bad = Vec::new();
    for want in golden.lines().skip(1) {
        let (t, c) = (want.split('\t').next().unwrap(), want.split('\t').nth(1).unwrap());
        let short = format!("{}.{c}", t.split('.').nth(1).unwrap());
        let pending = [short, t.to_string(), format!("{t}.{c}")].iter().any(|k| GMNS_BY_PROPERTIES.contains(&k.as_str()));
        if !tables.contains(t) || pending {
            continue;
        }
        match mine.get(&key(want)) {
            Some(g) if *g == want => {}
            Some(g) => bad.push(format!(" rust:   {g}\n python: {want}")),
            None => bad.push(format!(" missing: {want}")),
        }
    }
    bad
}

/// `junctions.join` (`duckosm sumo --join-junctions`; needs netconvert): the nodes that netconvert joins are merged, and the plain
/// XML of that equals Python's (`_merge_joined`, made with netconvert 1.27.0).
#[test]
fn sumo_join_equals_python() {
    let d = out("sumo_join");
    let o = duckosm_export::sumo::SumoOptions { config: vec![("junctions.join".into(), "true".into())], ..Default::default() };
    duckosm_export::sumo::to_sumo_with(db(), &d, "driving", "monaco", true, None, true, &o).unwrap();
    for f in ["nod", "edg", "con"] {
        same(&d.join(format!("monaco.{f}.xml")), &format!("sumo_join_driving.{f}.xml.gz"));
    }
}

/// sumo_compare: the lanes of duckOSM's SUMO network against SUMO's own OSM import of the same PBF, as Python's `lane_diff` gives
/// them (`monaco_sumo_lane_diff.tsv`; needs netconvert and osmium).
#[test]
fn sumo_lane_diff_equals_python() {
    let dir = out("lane_diff");
    let _ = std::fs::remove_dir_all(&dir);
    let (rows, n) = duckosm_export::sumo_compare::lane_diff(db(), &data().join("monaco_sample.osm.pbf"), &dir).unwrap();
    let t = |x: (usize, usize, usize)| format!("{},{},{}", x.0, x.1, x.2);
    let mut got = vec![format!("compared\t{n}")];
    got.extend(rows.iter().map(|r| format!("{}\t{}\t{}\t{}\t{}\t{}", r.way, r.dir, t(r.duckosm), t(r.sumo_osm), r.sources.join(","), r.cause)));
    assert_eq!(got.join("\n") + "\n", text(&data().join("exports/monaco_sumo_lane_diff.tsv")), "lane_diff differs from Python's");
}
