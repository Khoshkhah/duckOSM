//! `duckosm-rust quack --gdal` on `tests/data/monaco.osm.pbf` equals `ogr2ogr -f GPKG out.gpkg monaco.osm.pbf` of GDAL
//! 3.13.3 (the golden file `tests/data/monaco_gdal.jsonl.gz`, made from that GeoPackage before the code): the same rows
//! in each layer, with the same ids and column values, and the same geometries. The coordinates are integers of 1e-7
//! degrees; the rings of a multipolygon are compared with no regard to their first point, their direction and their
//! order. The whole file and the file by tiles of zoom 12.
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

fn check(tiles: Option<duckosm_build::TileSize>) {
    let pbf = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let dir = std::env::temp_dir().join(format!("quack_gdal_monaco_{}_{}", std::process::id(), tiles.is_some()));
    std::fs::create_dir_all(&dir).unwrap();
    let gpkg = dir.join("monaco_gdal.gpkg");
    duckosm_build::quack_gdal::quack_gdal(&pbf, Some(&gpkg), None, tiles).unwrap();
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
    check(None);
}

#[test]
fn monaco_by_tiles_equals_ogr2ogr() {
    check(Some(duckosm_build::TileSize::Zoom(12)));
}
