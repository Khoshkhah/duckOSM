//! The kinds of boundary of a config that are not a file (duckOSM `Config.materialize_boundary`, `area.py`): a box
//! (`boundary.bbox`), an H3 cell (`boundary.h3_cell`) or a place name (`boundary.place`), each written as
//! `<name>.boundary.geojson` next to the database; the grown boundary of `boundary.buffer_m`; and `main.boundary_cells`
//! (`processors/boundary_cells.py`). The place, the buffer and the cells use DuckDB's spatial extension, with the SQL of duckOSM.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// A `[west, south, east, north]` box (degrees) as a GeoJSON Polygon (duckOSM `area.bbox_geojson`).
pub fn bbox_geojson(b: &[f64]) -> Result<Value, String> {
    let [w, s, e, n] = b[..] else {
        return Err(format!("boundary.bbox must be [west, south, east, north], got {b:?}"));
    };
    if !(-180.0 <= w && w < e && e <= 180.0 && -90.0 <= s && s < n && n <= 90.0) {
        return Err(format!("boundary.bbox must be [west, south, east, north] in degrees, got {b:?}"));
    }
    Ok(json!({"type": "Polygon", "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]}))
}

/// The outline of the H3 cell `cell` (hex text) as a GeoJSON Polygon (duckOSM `area.h3_geojson`).
pub fn h3_geojson(cell: &str) -> Result<Value, String> {
    let c: h3o::CellIndex = cell.parse().map_err(|_| format!("not an H3 cell: {cell:?}"))?;
    let mut ring: Vec<[f64; 2]> = c.boundary().iter().map(|p| [p.lng(), p.lat()]).collect();
    ring.push(ring[0]);
    Ok(json!({"type": "Polygon", "coordinates": [ring]}))
}

/// Write a boundary as a FeatureCollection of one feature, `props` its properties without `others` (duckOSM
/// `area.write_boundary`), as Python's `json.dumps` writes it.
pub fn write_boundary(geometry: Value, mut props: Value, out: &Path) -> Result<(), String> {
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    props.as_object_mut().map(|o| o.shift_remove("others"));
    let doc = json!({"type": "FeatureCollection", "features": [{"type": "Feature", "properties": props, "geometry": geometry}]});
    std::fs::write(out, py_dumps(&doc)).map_err(|e| format!("{}: {e}", out.display()))
}

/// Python's `json.dumps(v)`: `", "` and `": "` between the items, and each non-ASCII letter as `\uXXXX`.
pub fn py_dumps(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("[{}]", a.iter().map(py_dumps).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter().map(|(k, v)| format!("{}: {}", py_dumps(&Value::String(k.clone())), py_dumps(v))).collect::<Vec<_>>().join(", ")
        ),
        Value::String(s) => {
            let mut e = String::new();
            for c in serde_json::to_string(s).expect("JSON of a text").chars() {
                if c.is_ascii() {
                    e.push(c);
                } else {
                    let mut b = [0u16; 2];
                    for u in c.encode_utf16(&mut b) {
                        e += &format!("\\u{u:04x}");
                    }
                }
            }
            e
        }
        Value::Number(n) => match n.as_f64() {
            Some(f) if !n.is_i64() && !n.is_u64() => py_float(f),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// Python's `repr` of a float.
pub fn py_float(f: f64) -> String {
    if f.is_finite() && f != 0.0 && (f.abs() >= 1e16 || f.abs() < 1e-4) {
        // Python: 1e+16, 1e-05
        let s = format!("{f:e}");
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let e: i32 = e.parse().unwrap_or(0);
        format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    } else {
        format!("{f:?}")
    }
}

/// Python's `repr` of a text.
pub fn py_repr(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::from(q);
    for c in s.chars() {
        match c {
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out += &format!("\\x{:02x}", c as u32),
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

#[cfg(feature = "duckdb")]
mod sql {
    use super::*;
    use duckosm_duckdb::duckdb::{params, Connection};

    fn err(e: duckosm_duckdb::duckdb::Error) -> String {
        e.to_string()
    }

    fn spatial() -> Result<Connection, String> {
        let con = Connection::open_in_memory().map_err(err)?;
        con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;").map_err(err)?;
        Ok(con)
    }

    fn quote(p: &Path) -> String {
        p.display().to_string().replace('\'', "''")
    }

    /// GDAL's `osmconf.ini` (GDAL 3.4): the OSM driver of GDAL needs a config file, and the DuckDB build has none.
    const OSMCONF: &str = include_str!("osmconf.ini");

    /// The `ST_Read` of the multipolygons of a PBF with `boundary=administrative`, as `ogr2ogr ... multipolygons -where
    /// "boundary='administrative'"` reads them (duckOSM `admin.extract_boundaries`), with GDAL's columns. `conf` is a file
    /// with [`OSMCONF`] (see [`write_osmconf`]).
    pub fn admin_multipolygons(pbf: &Path, conf: &Path) -> String {
        format!(
            "SELECT * FROM ST_Read('{}', layer = 'multipolygons', open_options = ['CONFIG_FILE={}']) WHERE boundary = 'administrative'",
            quote(pbf),
            quote(conf)
        )
    }

    /// The columns of duckOSM's `admin_boundaries` (`admin.load_boundaries`) from `source`, a SELECT of GDAL's multipolygons.
    pub fn admin_select(source: &str) -> String {
        format!(
            "SELECT TRY_CAST(COALESCE(osm_id, osm_way_id) AS BIGINT) AS osm_id, name,
                    NULLIF(regexp_extract(other_tags, '\"name:en\"=>\"([^\"]*)\"', 1), '') AS name_en,
                    TRY_CAST(admin_level AS INTEGER) AS admin_level, geom::GEOMETRY AS geometry
             FROM ({source})"
        )
    }

    /// Write GDAL's `osmconf.ini` to a temporary file; the caller removes it.
    pub fn write_osmconf() -> Result<PathBuf, String> {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let conf = std::env::temp_dir().join(format!("duckosm-osmconf-{}-{n}.ini", std::process::id()));
        std::fs::write(&conf, OSMCONF).map_err(|e| e.to_string())?;
        Ok(conf)
    }

    /// What to look for: a name (case, accents and hyphens ignored, `name:en` too) or one OSM relation id.
    #[derive(Clone, Copy, Debug)]
    pub enum Wanted<'a> {
        Name(&'a str),
        OsmId(i64),
    }

    /// The admin boundaries for `want` in the borders of `pbf` (OSM relations with `boundary=administrative`, read by GDAL
    /// as `ogr2ogr ... multipolygons` does), best first (duckOSM `area._from_pbf` and `admin_match_sql`): an exact name
    /// first, then the largest area; at most 6.
    fn from_pbf(pbf: &Path, want: Wanted) -> Result<Vec<(Value, Value)>, String> {
        let conf = write_osmconf()?;
        let con = spatial()?;
        let read = con.execute_batch(&format!("CREATE TEMP TABLE admin_boundaries AS {}", admin_select(&admin_multipolygons(pbf, &conf))));
        let _ = std::fs::remove_file(&conf);
        read.map_err(|e| format!("the borders of {}: {e}", pbf.display()))?;
        let pick = "SELECT ST_AsGeoJSON(geom), osm_id, name, admin_level, ST_Area_Spheroid(ST_FlipCoordinates(geom)) / 1e6 FROM ";
        let (sql, arg) = match want {
            Wanted::OsmId(id) => (
                format!("{pick}(SELECT geometry AS geom, osm_id, name, admin_level FROM admin_boundaries WHERE osm_id = {id})"),
                None,
            ),
            Wanted::Name(n) => {
                let norm = |x: &str| format!("replace(strip_accents(lower({x})), '-', ' ')");
                let (q, nm, en) = (norm("$1"), norm("name"), norm("coalesce(name_en, '')"));
                (
                    format!(
                        "{pick}(SELECT geometry AS geom, osm_id, name, admin_level FROM admin_boundaries
                           WHERE {nm} LIKE '%' || {q} || '%' OR {en} LIKE '%' || {q} || '%'
                           ORDER BY ({nm} = {q} OR {en} = {q}) DESC, ST_Area(geometry) DESC LIMIT 6)"
                    ),
                    Some(n),
                )
            }
        };
        let mut st = con.prepare(&sql).map_err(err)?;
        let map = |r: &duckosm_duckdb::duckdb::Row| -> duckosm_duckdb::duckdb::Result<(String, Option<i64>, Option<String>, Option<i64>, f64)> {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        };
        let rows: Vec<_> = match arg {
            Some(a) => st.query_map(params![a], map).map_err(err)?.collect::<Result<_, _>>().map_err(err)?,
            None => st.query_map([], map).map_err(err)?.collect::<Result<_, _>>().map_err(err)?,
        };
        let file = pbf.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        rows.into_iter()
            .map(|(geom, osm_id, name, level, area)| {
                let info = json!({"name": name, "osm_id": osm_id, "admin_level": level, "area_km2": round2(area), "source": format!("PBF {file}")});
                Ok((serde_json::from_str(&geom).map_err(|e| e.to_string())?, info))
            })
            .collect()
    }

    /// Python's `round(x, 2)`.
    fn round2(x: f64) -> f64 {
        let r = (x * 100.0).round() / 100.0;
        // round half to even, as Python: the two neighbours of a tie
        let lo = (x * 100.0).floor() / 100.0;
        if ((x - lo) * 100.0 - 0.5).abs() < 1e-9 && ((lo * 100.0).round() as i64) % 2 == 0 { lo } else { r }
    }

    /// The polygons that OpenStreetMap's Nominatim finds for `want` (duckOSM `area._from_nominatim`), best first. Empty when
    /// it finds none or cannot be reached.
    fn from_nominatim(want: Wanted) -> Result<Vec<(Value, Value)>, String> {
        let con = spatial()?;
        // Nominatim asks each program to name itself
        let ua = format!("duckOSM/{} (+https://github.com/Khoshkhah/duckOSM)", env!("CARGO_PKG_VERSION"));
        let enc = |x: &str| -> String {
            x.bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
                    b' ' => "+".to_string(),
                    _ => format!("%{b:02X}"),
                })
                .collect()
        };
        let url = match want {
            Wanted::OsmId(id) => format!("https://nominatim.openstreetmap.org/lookup?osm_ids=R{id}&format=geojson&polygon_geojson=1"),
            Wanted::Name(n) => format!("https://nominatim.openstreetmap.org/search?q={}&format=geojson&polygon_geojson=1&limit=5", enc(n)),
        };
        let text: Result<String, _> = con
            .execute_batch(&format!("INSTALL httpfs; LOAD httpfs; CREATE SECRET (TYPE http, EXTRA_HTTP_HEADERS MAP {{'User-Agent': '{ua}'}});"))
            .and_then(|_| con.query_row(&format!("SELECT content FROM read_text('{url}')"), [], |r| r.get(0)));
        let text = match text {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  Nominatim not reachable: {e}");
                return Ok(Vec::new());
            }
        };
        let doc: Value = serde_json::from_str(&text).map_err(|e| format!("Nominatim: {e}"))?;
        let mut found = Vec::new();
        for f in doc["features"].as_array().into_iter().flatten() {
            if matches!(f["geometry"]["type"].as_str(), Some("Polygon" | "MultiPolygon")) {
                let geom = f["geometry"].to_string();
                let area: f64 = con
                    .query_row("SELECT ST_Area_Spheroid(ST_FlipCoordinates(ST_GeomFromGeoJSON(?))) / 1e6", params![geom], |r| r.get(0))
                    .map_err(err)?;
                let p = &f["properties"];
                let name = if p["display_name"].is_null() { p["name"].clone() } else { p["display_name"].clone() };
                let info = json!({"name": name, "osm_id": p["osm_id"].clone(), "admin_level": null, "area_km2": round2(area), "source": "Nominatim"});
                found.push((f["geometry"].clone(), info));
            }
        }
        Ok(found)
    }

    /// The boundary for `want` (duckOSM `area.find_boundary`): the borders of `pbf` first, then Nominatim unless `offline`.
    /// The info has `name`, `osm_id`, `admin_level`, `area_km2`, `source` and `others` (the other matches, best first).
    pub fn find_boundary(want: Wanted, pbf: Option<&Path>, offline: bool) -> Result<(Value, Value), String> {
        let label = match want {
            Wanted::Name(n) => n.to_string(),
            Wanted::OsmId(id) => format!("OSM relation {id}"),
        };
        let with_others = |mut hits: Vec<(Value, Value)>, source: Option<&str>| -> (Value, Value) {
            let (geom, mut info) = hits.remove(0);
            let others: Vec<Value> = hits
                .into_iter()
                .map(|(_, mut i)| {
                    i.as_object_mut().map(|o| o.shift_remove("source"));
                    i
                })
                .collect();
            if let Some(s) = source {
                info["source"] = json!(s);
            }
            info["others"] = Value::Array(others);
            (geom, info)
        };
        if let Some(pbf) = pbf {
            let hits = from_pbf(pbf, want)?;
            if !hits.is_empty() {
                return Ok(with_others(hits, None));
            }
            let file = pbf.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            eprintln!("  no border matching {} in {file}", py_repr(&label));
        }
        if !offline {
            let hits = from_nominatim(want)?;
            if !hits.is_empty() {
                return Ok(with_others(hits, None));
            }
        }
        let place = match pbf {
            Some(p) => {
                let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                format!("in {file}{}", if offline { "" } else { " or on Nominatim" })
            }
            None => "on Nominatim".into(),
        };
        Err(format!(
            "no boundary found for {} {place}. Try a more specific name (e.g. 'Södermalm, Stockholm') or --osm-id.",
            py_repr(&label)
        ))
    }

    /// The boundary of the place `name` for a build (duckOSM `Config.materialize_boundary`): the borders of `pbf` first (a
    /// clip has none), then Nominatim. The info without `others`.
    pub fn find(name: &str, pbf: Option<&Path>) -> Result<(Value, Value), String> {
        let (geom, mut info) = find_boundary(Wanted::Name(name), pbf, false)?;
        info.as_object_mut().map(|o| o.shift_remove("others"));
        Ok((geom, info))
    }

    /// The boundary grown by `m` metres in the UTM zone of its centroid, as `<stem>.buffer<m>m.geojson` next to it (duckOSM
    /// `importer._effective_boundary_path`).
    pub fn buffered(path: &Path, m: f64) -> Result<PathBuf, String> {
        let path = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
        let con = spatial()?;
        let (lon, lat): (f64, f64) = con
            .query_row(&format!("SELECT ST_X(ST_Centroid(geom)), ST_Y(ST_Centroid(geom)) FROM ST_Read('{}') LIMIT 1", quote(&path)), [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(err)?;
        // duckOSM utils.utm_epsg
        let srid = if lat >= 0.0 { 32600 } else { 32700 } + ((lon + 180.0) / 6.0).floor().rem_euclid(60.0) as i64 + 1;
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let out = path.with_file_name(format!("{stem}.buffer{}m.geojson", m as i64));
        let _ = std::fs::remove_file(&out);
        con.execute_batch(&format!(
            "COPY (SELECT ST_Transform(ST_Buffer(ST_Transform(geom, 'EPSG:4326', 'EPSG:{srid}', always_xy := true), {m}),
                                      'EPSG:{srid}', 'EPSG:4326', always_xy := true) AS geom
                   FROM ST_Read('{}')) TO '{}' (FORMAT gdal, DRIVER 'GeoJSON')",
            quote(&path),
            quote(&out)
        ))
        .map_err(|e| format!("boundary.buffer_m: {e}"))?;
        eprintln!("  Boundary buffered +{m} m (UTM {srid}) -> {}", out.display());
        Ok(out)
    }

    /// The average edge of a hexagon in km at each resolution, as the H3 C library (and so duckOSM) has it: rounded, so not
    /// `h3o::Resolution::edge_length_km`.
    const EDGE_KM: [f64; 16] = [
        1281.256011, 483.0568391, 182.5129565, 68.97922179, 26.07175968, 9.85409099, 3.724532667, 1.406475763, 0.53141401,
        0.200786148, 0.075863783, 0.028663897, 0.010830188, 0.00409201, 0.0015461, 0.000584169,
    ];

    /// `main.boundary_cells` (duckOSM `BoundaryCellsBuilder`): at each resolution, the H3 cells whose centre is in the
    /// boundary grown by about one cell edge, and whose outline touches the boundary.
    pub fn boundary_cells(db: &Path, boundary: &Path, resolutions: &[u8]) -> Result<(), String> {
        let con = Connection::open(db).map_err(err)?;
        con.execute_batch(
            "INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;
             CREATE TABLE IF NOT EXISTS main.boundary_cells (h3_id VARCHAR PRIMARY KEY, resolution INTEGER, geometry GEOMETRY);",
        )
        .map_err(err)?;
        let src = format!("(SELECT ST_Union_Agg(geom) AS g FROM ST_Read('{}'))", quote(&boundary.canonicalize().map_err(|e| e.to_string())?));
        for &r in resolutions {
            let res = h3o::Resolution::try_from(r).map_err(|e| e.to_string())?;
            let buf = EDGE_KM[r as usize] / 111.0;
            // the parts of the boundary, each grown (shapely's buffer: 16 segments for each quarter circle)
            let mut st = con
                .prepare(&format!("SELECT ST_AsGeoJSON(ST_Buffer(p.geom, {buf}, 16)) FROM (SELECT unnest(ST_Dump(g)) AS p FROM {src})"))
                .map_err(err)?;
            let parts: Vec<String> = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<Result<_, _>>().map_err(err)?;
            let mut tiler = h3o::geom::TilerBuilder::new(res).containment_mode(h3o::geom::ContainmentMode::ContainsCentroid).build();
            for part in parts {
                let v: Value = serde_json::from_str(&part).map_err(|e| e.to_string())?;
                let ring = |r: &Value| -> geo::LineString {
                    r.as_array().into_iter().flatten().filter_map(|c| Some(geo::coord! {x: c[0].as_f64()?, y: c[1].as_f64()?})).collect()
                };
                let rings: Vec<geo::LineString> = v["coordinates"].as_array().into_iter().flatten().map(ring).collect();
                let Some((outer, holes)) = rings.split_first() else { continue };
                tiler.add(geo::Polygon::new(outer.clone(), holes.to_vec())).map_err(|e| format!("boundary cells: {e}"))?;
            }
            con.execute_batch(&format!(
                "DELETE FROM main.boundary_cells WHERE resolution = {r}; CREATE OR REPLACE TEMP TABLE _bc_stage (h3_id VARCHAR, resolution INTEGER, wkt VARCHAR);"
            ))
            .map_err(err)?;
            {
                let mut app = con.appender("_bc_stage").map_err(err)?;
                for c in tiler.into_coverage() {
                    let mut ring: Vec<String> = c.boundary().iter().map(|p| format!("{:?} {:?}", p.lng(), p.lat())).collect();
                    ring.push(ring[0].clone());
                    app.append_row(params![c.to_string(), r as i32, format!("POLYGON (({}))", ring.join(", "))]).map_err(err)?;
                }
            }
            let n = con
                .execute(
                    &format!(
                        "INSERT INTO main.boundary_cells SELECT h3_id, resolution, ST_GeomFromText(wkt) FROM _bc_stage, {src} b
                         WHERE ST_Intersects(ST_GeomFromText(wkt), b.g)"
                    ),
                    [],
                )
                .map_err(err)?;
            con.execute_batch("DROP TABLE _bc_stage").map_err(err)?;
            eprintln!("  Boundary cells res {r}: {n}");
        }
        con.execute_batch("CHECKPOINT").map_err(err)
    }
}

#[cfg(feature = "duckdb")]
pub use sql::{admin_multipolygons, admin_select, boundary_cells, buffered, find, find_boundary, write_osmconf, Wanted};

#[cfg(not(feature = "duckdb"))]
pub fn find(_: &str, _: Option<&Path>) -> Result<(Value, Value), String> {
    Err("boundary.place needs the cargo feature duckdb".into())
}
#[cfg(not(feature = "duckdb"))]
pub fn buffered(_: &Path, _: f64) -> Result<PathBuf, String> {
    Err("boundary.buffer_m needs the cargo feature duckdb".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_box_and_a_cell_as_duckosm_writes_them() {
        // duckOSM: area.h3_geojson("883969a40bfffff"), a cell of Monaco
        let want = r#"{"coordinates":[[[7.412613819645445,43.74028117100724],[7.411036828632863,43.73556243873496],[7.415842539332111,43.73247003208324],[7.4222255559768975,43.73409622204599],[7.423803272034207,43.73881502435256],[7.418997246428009,43.74190756668071],[7.412613819645445,43.74028117100724]]],"type":"Polygon"}"#;
        // h3o computes in another order than the H3 C library: the points differ by about 1e-14 degrees
        let got = super::h3_geojson("883969a40bfffff").unwrap();
        let want: serde_json::Value = serde_json::from_str(want).unwrap();
        let pts = |v: &serde_json::Value| -> Vec<f64> {
            v["coordinates"][0].as_array().unwrap().iter().flat_map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()]).collect()
        };
        let (a, b) = (pts(&want), pts(&got));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-12), "{a:?} {b:?}");
        assert!(super::bbox_geojson(&[7.4, 43.7, 7.3, 43.8]).is_err());
    }
}
