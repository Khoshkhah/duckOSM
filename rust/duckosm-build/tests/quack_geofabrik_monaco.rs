//! `duckosm quack --geofabrik` on `tests/data/monaco_geofabrik.osm.pbf` (Geofabrik's Monaco of 2026-11-17) equals
//! Geofabrik's free GeoPackage of the same day (the golden file `tests/data/monaco_geofabrik.jsonl.gz`, made from
//! Geofabrik's file by `tools/geofabrik_golden.py`, not from our output): the same rows in each of the 20 layers, with the
//! same `osm_id`, `code`, `fclass` and column values, and the same geometries. The coordinates are integers of 1e-7
//! degrees; the rings of a polygon are compared with no regard to their first point, their direction and their order.
//! The GeoPackage of the whole file, of the tiles of zoom 12, and of each tile alone with the join (the browser), and
//! the zip file of shapefiles.
#![cfg(feature = "gpkg")]

use duckosm_build::quack_gdal::Mode;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, Read};
use std::path::Path;

/// The one known difference: relation 11384697 (a building) has two inner rings that share an edge. Geofabrik joins
/// them into one hole; duckOSM keeps the two rings (GDAL's rings). The area is the same (`ST_Equals` is true).
const SAME_AREA: &[(&str, &str)] = &[("gis_osm_buildings_a_free", "11384697")];

/// A ring with no closing point, counterclockwise, from its smallest point.
fn ring(pts: &[Value]) -> Value {
    let mut p: Vec<(i64, i64)> = pts.iter().map(|p| (p[0].as_i64().unwrap(), p[1].as_i64().unwrap())).collect();
    p.pop();
    let area: i128 = (0..p.len()).map(|i| {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        a.0 as i128 * b.1 as i128 - b.0 as i128 * a.1 as i128
    }).sum();
    if area < 0 {
        p.reverse();
    }
    let start = (0..p.len()).min_by_key(|&i| p[i]).unwrap_or(0);
    p.rotate_left(start);
    json!(p.iter().map(|x| [x.0, x.1]).collect::<Vec<_>>())
}

/// A geometry of the golden form: a point or a line as it is, a multipolygon as its sorted rings.
fn canon(g: &Value) -> Value {
    if g[0] != "MULTIPOLYGON" {
        return g.clone();
    }
    let mut rings: Vec<Value> = g[1].as_array().unwrap().iter().flat_map(|p| p[1].as_array().unwrap().iter().map(|r| ring(r[1].as_array().unwrap()))).collect();
    rings.sort_by_key(|r| r.to_string());
    json!(["MULTIPOLYGON", rings])
}

/// A WKB geometry (little endian) in the golden form.
fn from_wkb(b: &mut &[u8]) -> Value {
    let u32_ = |b: &mut &[u8]| {
        let v = u32::from_le_bytes(b[..4].try_into().unwrap());
        *b = &b[4..];
        v
    };
    let pt = |b: &mut &[u8]| {
        let x = f64::from_le_bytes(b[..8].try_into().unwrap());
        let y = f64::from_le_bytes(b[8..16].try_into().unwrap());
        *b = &b[16..];
        json!([(x * 1e7).round() as i64, (y * 1e7).round() as i64])
    };
    *b = &b[1..];
    match u32_(b) {
        1 => json!(["POINT", [pt(b)]]),
        2 => json!(["LINESTRING", (0..u32_(b)).map(|_| pt(b)).collect::<Vec<_>>()]),
        3 => json!(["POLYGON", (0..u32_(b)).map(|_| json!(["LINEARRING", (0..u32_(b)).map(|_| pt(b)).collect::<Vec<_>>()])).collect::<Vec<_>>()]),
        _ => json!(["MULTIPOLYGON", (0..u32_(b)).map(|_| from_wkb(b)).collect::<Vec<_>>()]),
    }
}

/// The rows by (table, osm_id, code): the columns and the geometry.
type Rows = BTreeMap<(String, String, i64), (Value, Value)>;

fn key(t: &str, cols: &Value) -> (String, String, i64) {
    (t.to_string(), cols["osm_id"].as_str().unwrap().to_string(), cols["code"].as_i64().unwrap())
}

fn golden() -> Rows {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco_geofabrik.jsonl.gz");
    let r = std::io::BufReader::new(flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap()));
    let mut rows = Rows::new();
    for line in r.lines().skip(1) {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let t = v[0].as_str().unwrap();
        assert!(rows.insert(key(t, &v[1]), (v[1].clone(), canon(&v[2]))).is_none(), "one row for each key");
    }
    rows
}

fn from_gpkg(gpkg: &Path) -> Rows {
    let con = rusqlite::Connection::open(gpkg).unwrap();
    let tables: Vec<String> =
        con.prepare("SELECT table_name FROM gpkg_contents").unwrap().query_map([], |r| r.get(0)).unwrap().map(|t| t.unwrap()).collect();
    assert_eq!(tables.len(), 20, "the layers");
    let mut rows = Rows::new();
    for t in tables {
        let mut st = con.prepare(&format!("SELECT * FROM \"{t}\"")).unwrap();
        let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let mut q = st.query([]).unwrap();
        while let Some(r) = q.next().unwrap() {
            let mut cols = serde_json::Map::new();
            let mut geom = Value::Null;
            for (i, n) in names.iter().enumerate() {
                match r.get_ref(i).unwrap() {
                    rusqlite::types::ValueRef::Text(s) => drop(cols.insert(n.clone(), json!(std::str::from_utf8(s).unwrap()))),
                    rusqlite::types::ValueRef::Integer(x) if n != "fid" => drop(cols.insert(n.clone(), json!(x))),
                    rusqlite::types::ValueRef::Blob(b) => {
                        let skip = 8 + if (b[3] >> 1) & 7 == 1 { 32 } else { 0 };
                        geom = from_wkb(&mut &b[skip..]);
                    }
                    _ => {}
                }
            }
            let cols = Value::Object(cols);
            rows.insert(key(&t, &cols), (cols, canon(&geom)));
        }
    }
    rows
}

/// The rows of the zip file of shapefiles, in the golden form (an empty text is "").
fn from_shp_zip(path: &Path) -> Rows {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut file = |name: &str| {
        let mut b = Vec::new();
        zip.by_name(name).unwrap_or_else(|_| panic!("{name} in the zip file")).read_to_end(&mut b).unwrap();
        std::io::Cursor::new(b)
    };
    let mut rows = Rows::new();
    let schema = duckosm_build::quack_gdal::schema(Mode::Geofabrik);
    for layer in 1..=schema.layers {
        let t = (schema.table)(layer);
        assert_eq!(std::str::from_utf8(file(&format!("{t}_1.cpg")).get_ref()).unwrap(), "UTF-8\n");
        let shapes = shapefile::ShapeReader::new(file(&format!("{t}_1.shp"))).unwrap();
        let dbf = dbase::Reader::new(file(&format!("{t}_1.dbf"))).unwrap();
        let mut reader = shapefile::Reader::new(shapes, dbf);
        for r in reader.iter_shapes_and_records() {
            let (shape, record) = r.unwrap();
            let mut cols = serde_json::Map::new();
            for (c, int, _) in (schema.columns)(layer) {
                let v = match record.get(c) {
                    Some(dbase::FieldValue::Character(s)) => json!(s.clone().unwrap_or_default()),
                    Some(dbase::FieldValue::Numeric(n)) => json!(n.map_or(0, |n| n as i64)),
                    other => panic!("{t}.{c}: {other:?}"),
                };
                assert_eq!(v.is_i64(), int, "{t}.{c}: the type");
                cols.insert(c.to_string(), v);
            }
            let pt = |p: &shapefile::Point| json!([(p.x * 1e7).round() as i64, (p.y * 1e7).round() as i64]);
            let geom = match shape {
                shapefile::Shape::Point(p) => json!(["POINT", [pt(&p)]]),
                shapefile::Shape::Polyline(l) => json!(["LINESTRING", l.parts()[0].iter().map(pt).collect::<Vec<_>>()]),
                shapefile::Shape::Polygon(a) => {
                    let rings: Vec<Value> = a.rings().iter().map(|r| json!(["LINEARRING", r.points().iter().map(pt).collect::<Vec<_>>()])).collect();
                    json!(["MULTIPOLYGON", [["POLYGON", rings]]])
                }
                other => panic!("{t}: {other}"),
            };
            let cols = Value::Object(cols);
            rows.insert(key(t, &cols), (cols, canon(&geom)));
        }
    }
    rows
}

fn compare(what: &str, got: Rows) {
    let want = golden();
    let mut bad = 0;
    for (k, w) in &want {
        match got.get(k) {
            None => {
                bad += 1;
                eprintln!("{what}: missing {k:?}");
            }
            Some(g) if g.0 != w.0 => {
                bad += 1;
                eprintln!("{what}: columns of {k:?}: {} != {}", g.0, w.0);
            }
            Some(g) if g.1 != w.1 && !SAME_AREA.contains(&(k.0.as_str(), k.1.as_str())) => {
                bad += 1;
                eprintln!("{what}: geometry of {k:?}");
            }
            _ => {}
        }
    }
    for k in got.keys().filter(|k| !want.contains_key(*k)) {
        bad += 1;
        eprintln!("{what}: extra {k:?}");
    }
    assert_eq!(bad, 0, "{what}: rows that differ from Geofabrik's file");
}

fn pbf() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco_geofabrik.osm.pbf")
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("quack_geofabrik_monaco_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn monaco_equals_geofabrik() {
    let d = dir("whole");
    let (gpkg, shp) = (d.join("monaco.gpkg"), d.join("monaco.shp.zip"));
    let n = duckosm_build::quack_gdal::quack_layers(&pbf(), Mode::Geofabrik, Some(&gpkg), None, Some(&shp), None, None).unwrap();
    assert_eq!(n.rows.iter().sum::<usize>(), 8732);
    compare("GeoPackage", from_gpkg(&gpkg));
    compare("shapefiles", from_shp_zip(&shp));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn monaco_by_tiles_equals_geofabrik() {
    let d = dir("tiles");
    let gpkg = d.join("monaco.gpkg");
    duckosm_build::quack_gdal::quack_layers(&pbf(), Mode::Geofabrik, Some(&gpkg), None, None, Some(duckosm_build::TileSize::Zoom(12)), None).unwrap();
    compare("tiles of zoom 12", from_gpkg(&gpkg));
    std::fs::remove_dir_all(&d).ok();
}

/// The browser's way: the tile cut, each tile of zoom 12 alone (`quack_gdal_tile_to`), then the join.
#[test]
fn monaco_tiles_alone_equals_geofabrik() {
    use duckosm_core::quack;
    let pbf = pbf();
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(&pbf).map_err(|e| e.to_string())?));
    let tiles = quack::tiles_of_file(&open, quack::TileSize::Zoom(12)).unwrap();
    let world = duckosm_core::boundary::Boundary::parse(quack::WORLD).unwrap();
    let files = duckosm_core::tilecut::cut_to_store(&open, &world, &tiles, &["multipolygon", "boundary"]).unwrap();
    let mut parts = Vec::new();
    for (i, (tile, c)) in files.tiles.iter().enumerate() {
        if c.nodes == 0 {
            continue;
        }
        let mut out = Vec::new();
        let open_tile = || Ok(std::io::BufReader::new(files.reader(i)));
        duckosm_build::quack_gdal::quack_gdal_tile_to(open_tile, *tile, Mode::Geofabrik, &mut out, None).unwrap();
        parts.push(std::io::Cursor::new(out));
    }
    assert!(parts.len() > 1, "more than one tile");
    let d = dir("alone");
    let gpkg = d.join("monaco.gpkg");
    let zip = std::fs::File::create(d.join("monaco.shp.zip")).unwrap();
    duckosm_build::quack_gdal::quack_gdal_join_to::<_, std::fs::File>(parts, Mode::Geofabrik, Some(&gpkg), None, Some(zip)).unwrap();
    compare("tiles alone", from_gpkg(&gpkg));
    compare("tiles alone, shapefiles", from_shp_zip(&d.join("monaco.shp.zip")));
    std::fs::remove_dir_all(&d).ok();
}
