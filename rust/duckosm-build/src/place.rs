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

/// Write a boundary as a FeatureCollection of one feature, `props` its properties (duckOSM `area.write_boundary`).
pub fn write_boundary(geometry: Value, props: Value, out: &Path) -> Result<(), String> {
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let doc = json!({"type": "FeatureCollection", "features": [{"type": "Feature", "properties": props, "geometry": geometry}]});
    std::fs::write(out, doc.to_string()).map_err(|e| format!("{}: {e}", out.display()))
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

    /// The admin boundary called `name` in the borders of `pbf` (OSM relations with `boundary=administrative`, read by GDAL as
    /// `ogr2ogr ... multipolygons` does), as duckOSM `area._from_pbf` and `admin_match_sql` choose it: case, accents and
    /// hyphens ignored, `name:en` too, an exact name first, then the largest area.
    fn from_pbf(pbf: &Path, name: &str) -> Result<Option<(Value, Value)>, String> {
        let conf = std::env::temp_dir().join(format!("duckosm-osmconf-{}.ini", std::process::id()));
        std::fs::write(&conf, OSMCONF).map_err(|e| e.to_string())?;
        let con = spatial()?;
        let read = con.execute_batch(&format!(
            "CREATE TEMP TABLE admin_boundaries AS
             SELECT TRY_CAST(COALESCE(osm_id, osm_way_id) AS BIGINT) AS osm_id, name,
                    NULLIF(regexp_extract(other_tags, '\"name:en\"=>\"([^\"]*)\"', 1), '') AS name_en,
                    TRY_CAST(admin_level AS INTEGER) AS admin_level, geom::GEOMETRY AS geometry
             FROM ST_Read('{}', layer = 'multipolygons', open_options = ['CONFIG_FILE={}'])
             WHERE boundary = 'administrative'",
            quote(pbf),
            quote(&conf)
        ));
        let _ = std::fs::remove_file(&conf);
        read.map_err(|e| format!("the borders of {}: {e}", pbf.display()))?;
        let norm = |x: &str| format!("replace(strip_accents(lower({x})), '-', ' ')");
        let (q, nm, en) = (norm("$1"), norm("name"), norm("coalesce(name_en, '')"));
        let sql = format!(
            "SELECT ST_AsGeoJSON(geom), osm_id, name, admin_level, ST_Area_Spheroid(ST_FlipCoordinates(geom)) / 1e6 FROM (
               SELECT geometry AS geom, osm_id, name, admin_level FROM admin_boundaries
               WHERE {nm} LIKE '%' || {q} || '%' OR {en} LIKE '%' || {q} || '%'
               ORDER BY ({nm} = {q} OR {en} = {q}) DESC, ST_Area(geometry) DESC LIMIT 1)"
        );
        let mut st = con.prepare(&sql).map_err(err)?;
        let mut rows = st.query(params![name]).map_err(err)?;
        let Some(r) = rows.next().map_err(err)? else {
            return Ok(None);
        };
        let geom: String = r.get(0).map_err(err)?;
        let area: f64 = r.get(4).map_err(err)?;
        let file = pbf.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let info = json!({
            "name": r.get::<_, Option<String>>(2).map_err(err)?,
            "osm_id": r.get::<_, Option<i64>>(1).map_err(err)?,
            "admin_level": r.get::<_, Option<i64>>(3).map_err(err)?,
            "area_km2": (area * 100.0).round() / 100.0,
            "source": format!("PBF {file}"),
        });
        Ok(Some((serde_json::from_str(&geom).map_err(|e| e.to_string())?, info)))
    }

    /// The first polygon that OpenStreetMap's Nominatim finds for `name` (duckOSM `area._from_nominatim`). None when it
    /// finds none or cannot be reached.
    fn from_nominatim(name: &str) -> Result<Option<(Value, Value)>, String> {
        let con = spatial()?;
        // Nominatim asks each program to name itself
        let ua = format!("duckOSM/{} (+https://github.com/Khoshkhah/duckOSM)", env!("CARGO_PKG_VERSION"));
        let q: String = name.bytes().map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        }).collect();
        let url = format!("https://nominatim.openstreetmap.org/search?q={q}&format=geojson&polygon_geojson=1&limit=5");
        let text: Result<String, _> = con
            .execute_batch(&format!("INSTALL httpfs; LOAD httpfs; CREATE SECRET (TYPE http, EXTRA_HTTP_HEADERS MAP {{'User-Agent': '{ua}'}});"))
            .and_then(|_| con.query_row(&format!("SELECT content FROM read_text('{url}')"), [], |r| r.get(0)));
        let text = match text {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  Nominatim not reachable: {e}");
                return Ok(None);
            }
        };
        let doc: Value = serde_json::from_str(&text).map_err(|e| format!("Nominatim: {e}"))?;
        for f in doc["features"].as_array().into_iter().flatten() {
            if matches!(f["geometry"]["type"].as_str(), Some("Polygon" | "MultiPolygon")) {
                let geom = f["geometry"].to_string();
                let area: f64 = con
                    .query_row("SELECT ST_Area_Spheroid(ST_FlipCoordinates(ST_GeomFromGeoJSON(?))) / 1e6", params![geom], |r| r.get(0))
                    .map_err(err)?;
                let p = &f["properties"];
                let info = json!({
                    "name": if p["display_name"].is_null() { p["name"].clone() } else { p["display_name"].clone() },
                    "osm_id": p["osm_id"].clone(),
                    "admin_level": null,
                    "area_km2": (area * 100.0).round() / 100.0,
                    "source": "Nominatim",
                });
                return Ok(Some((f["geometry"].clone(), info)));
            }
        }
        Ok(None)
    }

    /// The boundary of the place `name` (duckOSM `area.find_boundary`): the borders of `pbf` first, then Nominatim.
    pub fn find(name: &str, pbf: &Path) -> Result<(Value, Value), String> {
        if let Some(hit) = from_pbf(pbf, name)? {
            return Ok(hit);
        }
        eprintln!("  no border matching {name:?} in {}", pbf.display());
        from_nominatim(name)?.ok_or(format!(
            "no boundary found for {name:?} in {} or on Nominatim. Try a more specific name (e.g. 'Södermalm, Stockholm').",
            pbf.display()
        ))
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
pub use sql::{boundary_cells, buffered, find};

#[cfg(not(feature = "duckdb"))]
pub fn find(_: &str, _: &Path) -> Result<(Value, Value), String> {
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
