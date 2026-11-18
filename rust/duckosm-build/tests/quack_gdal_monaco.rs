//! `duckosm quack --gdal` on `tests/data/monaco.osm.pbf` equals `ogr2ogr -f GPKG out.gpkg monaco.osm.pbf` of GDAL
//! 3.13.3 (the golden file `tests/data/monaco_gdal.jsonl.gz`, made from that GeoPackage before the code): the same rows
//! in each layer, with the same ids and column values, and the same geometries. The coordinates are integers of 1e-7
//! degrees; the rings of a multipolygon are compared with no regard to their first point, their direction and their
//! order. The whole file, the file by tiles of zoom 12, and each tile alone with the join (the browser).
#![cfg(feature = "gpkg")]

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::BufRead;

/// A geometry as the golden file writes it: [type, coordinates] or [type, parts].
fn normalize(g: &Value) -> Value {
    let name = g[0].as_str().expect("a type");
    if name == "MULTIPOLYGON" {
        let mut polys: Vec<Value> = g[1]
            .as_array()
            .expect("polygons")
            .iter()
            .map(|p| {
                let rings = p[1].as_array().expect("rings");
                let mut holes: Vec<Value> = rings[1..].iter().map(|r| ring(r, false)).collect();
                holes.sort_by_key(|h| h.to_string());
                let mut all = vec![ring(&rings[0], true)];
                all.extend(holes);
                Value::Array(all)
            })
            .collect();
        polys.sort_by_key(|p| p.to_string());
        return json!([name, polys]);
    }
    g.clone()
}

/// A ring with no closing point, from its smallest point, counterclockwise (an outer ring) or clockwise (a hole).
fn ring(r: &Value, outer: bool) -> Value {
    let mut pts: Vec<(i64, i64)> =
        r[1].as_array().expect("points").iter().map(|p| (p[0].as_i64().expect("x"), p[1].as_i64().expect("y"))).collect();
    pts.pop();
    let area: i128 = (0..pts.len()).map(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        a.0 as i128 * b.1 as i128 - b.0 as i128 * a.1 as i128
    }).sum();
    if (area > 0) != outer {
        pts.reverse();
    }
    let start = (0..pts.len()).min_by_key(|&i| pts[i]).unwrap_or(0);
    pts.rotate_left(start);
    json!(pts.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>())
}

/// A WKB geometry (little endian) in the form of the golden file.
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
        t => {
            let name = match t {
                5 => "MULTILINESTRING",
                6 => "MULTIPOLYGON",
                _ => "GEOMETRYCOLLECTION",
            };
            json!([name, (0..u32_(b)).map(|_| from_wkb(b)).collect::<Vec<_>>()])
        }
    }
}

type Rows = BTreeMap<(String, String, String), (Value, Value)>;

fn golden() -> Rows {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco_gdal.jsonl.gz");
    let r = std::io::BufReader::new(flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap()));
    let mut rows = Rows::new();
    for line in r.lines().skip(1) {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let cols = v[1].clone();
        let id = |c: &str| cols.get(c).and_then(|x| x.as_str()).unwrap_or("").to_string();
        rows.insert((v[0].as_str().unwrap().to_string(), id("osm_id"), id("osm_way_id")), (cols, normalize(&v[2])));
    }
    rows
}

fn ours(gpkg: &std::path::Path) -> Rows {
    let con = rusqlite::Connection::open(gpkg).unwrap();
    let mut rows = Rows::new();
    for layer in ["points", "lines", "multilinestrings", "multipolygons", "other_relations"] {
        let mut st = con.prepare(&format!("SELECT * FROM \"{layer}\"")).unwrap();
        let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let mut q = st.query([]).unwrap();
        while let Some(r) = q.next().unwrap() {
            let mut cols = serde_json::Map::new();
            let mut geom = Value::Null;
            for (i, n) in names.iter().enumerate() {
                match r.get_ref(i).unwrap() {
                    rusqlite::types::ValueRef::Text(t) if n != "fid" => {
                        cols.insert(n.clone(), json!(std::str::from_utf8(t).unwrap()));
                    }
                    rusqlite::types::ValueRef::Integer(x) if n != "fid" => {
                        cols.insert(n.clone(), json!(x));
                    }
                    rusqlite::types::ValueRef::Blob(b) if n == "geom" => {
                        // the GeoPackage header: 8 bytes, and 32 of the envelope when its flags say so
                        let skip = 8 + if (b[3] >> 1) & 7 == 1 { 32 } else { 0 };
                        geom = from_wkb(&mut &b[skip..]);
                    }
                    _ => {}
                }
            }
            let cols = Value::Object(cols);
            let id = |c: &str| cols.get(c).and_then(|x| x.as_str()).unwrap_or("").to_string();
            rows.insert((layer.to_string(), id("osm_id"), id("osm_way_id")), (cols.clone(), normalize(&geom)));
        }
    }
    rows
}

/// How the GeoPackage is made: the whole file, by tiles, or each tile alone and then the join (the browser).
enum Mode {
    Whole,
    Tiles,
    TilesAlone,
}

/// The browser's way: the tile cut, each tile of zoom 12 alone (`quack_gdal_tile_to`), then the join.
fn tiles_alone(pbf: &std::path::Path, gpkg: &std::path::Path) {
    use duckosm_core::quack;
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(pbf).map_err(|e| e.to_string())?));
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
        duckosm_build::quack_gdal::quack_gdal_tile_to(open_tile, *tile, &mut out, None).unwrap();
        parts.push(std::io::Cursor::new(out));
    }
    assert!(parts.len() > 1, "more than one tile");
    duckosm_build::quack_gdal::quack_gdal_join_to::<_, Vec<u8>>(parts, Some(gpkg), None).unwrap();
}

fn check(mode: Mode) {
    let pbf = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let name = match mode {
        Mode::Whole => "whole",
        Mode::Tiles => "tiles",
        Mode::TilesAlone => "alone",
    };
    let dir = std::env::temp_dir().join(format!("quack_gdal_monaco_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gpkg = dir.join("monaco_gdal.gpkg");
    match mode {
        Mode::Whole => drop(duckosm_build::quack_gdal::quack_gdal(&pbf, Some(&gpkg), None, None, None).unwrap()),
        Mode::Tiles => drop(duckosm_build::quack_gdal::quack_gdal(&pbf, Some(&gpkg), None, Some(duckosm_build::TileSize::Zoom(12)), None).unwrap()),
        Mode::TilesAlone => tiles_alone(&pbf, &gpkg),
    }
    let (want, got) = (golden(), ours(&gpkg));
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(want.len(), got.len(), "the number of rows");
    let mut bad = 0;
    for (k, w) in &want {
        match got.get(k) {
            None => {
                bad += 1;
                eprintln!("missing: {k:?}");
            }
            Some(g) if g != w => {
                bad += 1;
                if g.0 != w.0 {
                    eprintln!("columns of {k:?}: {} != {}", g.0, w.0);
                } else {
                    eprintln!("geometry of {k:?}");
                }
            }
            _ => {}
        }
    }
    assert_eq!(bad, 0, "rows that differ from ogr2ogr");
}

#[test]
fn monaco_equals_ogr2ogr() {
    check(Mode::Whole);
}

#[test]
fn monaco_by_tiles_equals_ogr2ogr() {
    check(Mode::Tiles);
}

/// Each tile alone and the join (the quack page of duckOSM Studio).
#[test]
fn monaco_tiles_alone_equal_ogr2ogr() {
    check(Mode::TilesAlone);
}

/// quack --gdal with the OSM fixes (`tests/data/osm_fixes_quack.yaml`): the whole file, by tiles, and each tile alone
/// with the join (the browser: the fixes as JSON, placed on the tiles) give the same rows, with the fixes in them.
#[test]
fn monaco_fixes() {
    use duckosm_core::quack::{self, QuackFixes};
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let pbf = data.join("monaco.osm.pbf");
    let osm = duckosm_build::config::read_osm_fixes(&data.join("osm_fixes_quack.yaml")).unwrap();
    let dir = std::env::temp_dir().join(format!("quack_gdal_fixes_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let make = |name: &str, tiles: Option<duckosm_build::TileSize>| {
        let gpkg = dir.join(format!("{name}.gpkg"));
        duckosm_build::quack_gdal::quack_gdal(&pbf, Some(&gpkg), None, tiles, Some(&osm)).unwrap();
        ours(&gpkg)
    };
    let whole = make("whole", None);
    let row = |layer: &str, id: &str, way: &str| whole.get(&(layer.to_string(), id.to_string(), way.to_string())).map(|r| &r.0);
    let col = |layer: &str, id: &str, way: &str, c: &str| row(layer, id, way).and_then(|r| r.get(c)).and_then(|v| v.as_str()).map(String::from);
    assert!(row("multipolygons", "", "23715051").is_none(), "removed");
    let colle = col("lines", "4227277", "", "other_tags").expect("the other tags of a line");
    for t in [r#""surface"=>"cobblestone""#, r#""sidewalk"=>"both""#] {
        assert!(colle.contains(t), "{t} in {colle}");
    }
    let rule = col("lines", "503462470", "", "other_tags").expect("the other tags of a line");
    for t in [r#""oneway"=>"yes""#, r#""lanes"=>"3""#] {
        assert!(rule.contains(t), "{t} in {rule}");
    }
    assert_eq!(col("lines", "9000000000001", "", "name").as_deref(), Some("Test"));
    assert_eq!(col("lines", "9000000000002", "", "highway").as_deref(), Some("service"));
    assert_eq!(col("multipolygons", "393226", "", "name").as_deref(), Some("Test relation"));
    assert!(row("points", "21911886", "").expect("a point").to_string().contains("marked"));
    assert!(whole != golden(), "the fixes change the rows");
    assert!(make("tiles", Some(duckosm_build::TileSize::Zoom(12))) == whole, "by tiles");
    // each tile alone, as the browser's workers
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(&pbf).map_err(|e| e.to_string())?));
    let tiles = quack::tiles_of_file(&open, quack::TileSize::Zoom(14)).unwrap();
    let mut placed = QuackFixes::new(&osm, open).unwrap();
    placed.place(&tiles);
    let json = placed.to_json();
    let world = duckosm_core::boundary::Boundary::parse(quack::WORLD).unwrap();
    let files = duckosm_core::tilecut::cut_to_store(&open, &world, &tiles, &["multipolygon", "boundary"]).unwrap();
    let mut parts = Vec::new();
    for (i, (tile, c)) in files.tiles.iter().enumerate() {
        if c.nodes == 0 {
            continue;
        }
        let mut out = Vec::new();
        let open_tile = || Ok(std::io::BufReader::new(files.reader(i)));
        let worker = QuackFixes::from_json(&json).unwrap();
        duckosm_build::quack_gdal::quack_gdal_tile_to(open_tile, *tile, &mut out, Some(&worker)).unwrap();
        parts.push(std::io::Cursor::new(out));
    }
    let gpkg = dir.join("alone.gpkg");
    duckosm_build::quack_gdal::quack_gdal_join_to::<_, Vec<u8>>(parts, Some(&gpkg), None).unwrap();
    let alone = ours(&gpkg);
    std::fs::remove_dir_all(&dir).ok();
    assert!(alone == whole, "each tile alone");
}
