//! The check page of a GIS export (`gis-debug`), the port of src/duckosm/gis_debug.py. The export file (a GeoPackage, or
//! a folder of shapefiles) is read through GDAL (DuckDB's `ST_Read`), as a GIS program reads it. The page
//! (src/duckosm/templates/gis_debug.html, the same file as Python) has a map of each layer and the checks: the number of
//! features, the CRS, `edge_id` exact, and with a source db, the `edge_id` of each mode against the db.
//!
//! The text of the checks is as geopandas gives it: the type of `edge_id` is the name of the pandas dtype.

use crate::{open, Result};
use duckdb::Connection;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

const TEMPLATE: &str = include_str!("../../../src/duckosm/templates/gis_debug.html");

/// The road classes: key, label, color, line width. The order of the list is the drawing order.
const CLASSES: [(&str, &str, &str, f64); 13] = [
    ("path", "Foot / path", "#6f9e79", 1.0),
    ("cycleway", "Cycleway", "#5aa0d8", 1.2),
    ("service", "Service", "#5f7180", 1.0),
    ("track", "Track", "#a98a5f", 1.0),
    ("living_street", "Living street", "#8fa3b0", 1.4),
    ("residential", "Residential", "#9fb0bd", 1.4),
    ("unclassified", "Unclassified", "#9aa7b2", 1.4),
    ("tertiary", "Tertiary", "#d9dee2", 1.8),
    ("secondary", "Secondary", "#f4e08a", 2.1),
    ("primary", "Primary", "#f2c14e", 2.5),
    ("trunk", "Trunk", "#e8934a", 2.9),
    ("motorway", "Motorway", "#e8663c", 3.3),
    ("other", "Other", "#7a8894", 1.2),
];
/// The grid of the map coordinates.
const Q: f64 = 4096.0;

fn class_index(hw: Option<&str>) -> i64 {
    let idx = |k: &str| CLASSES.iter().position(|c| c.0 == k).unwrap() as i64;
    let Some(h) = hw.filter(|h| !h.is_empty()) else { return idx("other") };
    let h = h.split(';').next().unwrap().trim();
    let h = h.strip_suffix("_link").unwrap_or(h);
    let c = match h {
        "motorway" | "trunk" | "primary" | "secondary" | "tertiary" | "residential" | "unclassified" | "living_street" | "service"
        | "track" | "cycleway" => h,
        "footway" | "path" | "pedestrian" | "steps" | "bridleway" => "path",
        "road" => "unclassified",
        _ => "other",
    };
    idx(c)
}

/// A JSON value, written as Python's `json.dumps` writes it (ASCII only, the floats as Python's `repr`).
#[derive(Clone)]
pub enum J {
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

fn jstr(s: &str, o: &mut String) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '\u{8}' => o.push_str("\\b"),
            '\u{c}' => o.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut b = [0u16; 2];
                for u in c.encode_utf16(&mut b) {
                    o.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => o.push(c),
        }
    }
    o.push('"');
}

impl J {
    /// `indent` None: `separators=(",", ":")`; Some(n): `indent=n`.
    pub fn dump(&self, indent: Option<usize>) -> String {
        let mut o = String::new();
        self.write(&mut o, indent, 0);
        o
    }
    fn write(&self, o: &mut String, ind: Option<usize>, lvl: usize) {
        let nl = |o: &mut String, l: usize| {
            if let Some(n) = ind {
                o.push('\n');
                o.push_str(&" ".repeat(n * l));
            }
        };
        match self {
            J::Null => o.push_str("null"),
            J::Int(i) => o.push_str(&i.to_string()),
            J::Float(f) => o.push_str(&crate::sumo::py_float(*f)),
            J::Str(s) => jstr(s, o),
            J::Arr(v) if v.is_empty() => o.push_str("[]"),
            J::Obj(v) if v.is_empty() => o.push_str("{}"),
            J::Arr(v) => {
                o.push('[');
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    nl(o, lvl + 1);
                    x.write(o, ind, lvl + 1);
                }
                nl(o, lvl);
                o.push(']');
            }
            J::Obj(v) => {
                o.push('{');
                for (i, (k, x)) in v.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    nl(o, lvl + 1);
                    jstr(k, o);
                    o.push_str(if ind.is_some() { ": " } else { ":" });
                    x.write(o, ind, lvl + 1);
                }
                nl(o, lvl);
                o.push('}');
            }
        }
    }
    pub fn get(&self, k: &str) -> Option<&J> {
        match self {
            J::Obj(v) => v.iter().find(|x| x.0 == k).map(|x| &x.1),
            _ => None,
        }
    }
}

fn obj(v: Vec<(&str, J)>) -> J {
    J::Obj(v.into_iter().map(|(k, x)| (k.to_string(), x)).collect())
}

fn s(v: &str) -> J {
    J::Str(v.into())
}

/// A number with a comma between the thousands, as Python's `{n:,}`.
fn thousands(n: usize) -> String {
    let d = n.to_string();
    let mut o = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            o.push(',');
        }
        o.push(c);
    }
    o
}

/// The parts of a WKB geometry: (type, the point lists). Line: one list; polygon: its rings; multi: each part.
fn wkb(b: &[u8]) -> Vec<(u32, Vec<Vec<(f64, f64)>>)> {
    struct R<'a>(&'a [u8], usize, bool);
    impl R<'_> {
        fn u32(&mut self) -> u32 {
            let v: [u8; 4] = self.0[self.1..self.1 + 4].try_into().unwrap();
            self.1 += 4;
            if self.2 { u32::from_le_bytes(v) } else { u32::from_be_bytes(v) }
        }
        fn f64(&mut self) -> f64 {
            let v: [u8; 8] = self.0[self.1..self.1 + 8].try_into().unwrap();
            self.1 += 8;
            if self.2 { f64::from_le_bytes(v) } else { f64::from_be_bytes(v) }
        }
        fn pts(&mut self, dims: usize, n: usize) -> Vec<(f64, f64)> {
            (0..n)
                .map(|_| {
                    let p = (self.f64(), self.f64());
                    for _ in 2..dims {
                        self.f64();
                    }
                    p
                })
                .collect()
        }
    }
    fn geom(r: &mut R, out: &mut Vec<(u32, Vec<Vec<(f64, f64)>>)>) {
        r.2 = r.0[r.1] == 1;
        r.1 += 1;
        let t = r.u32();
        let (base, dims) = match (t & 0x0fff_ffff, t & 0xc000_0000) {
            (t, 0) => (t % 1000, if (1000..3000).contains(&t) { 3 } else if t >= 3000 { 4 } else { 2 }),
            (t, f) => (t, 2 + (f & 0x8000_0000 != 0) as usize + (f & 0x4000_0000 != 0) as usize),
        };
        match base {
            1 => {
                let p = r.pts(dims, 1);
                out.push((1, if p[0].0.is_nan() { vec![] } else { vec![p] }));
            }
            2 => {
                let n = r.u32() as usize;
                out.push((2, vec![r.pts(dims, n)]));
            }
            3 => {
                let n = r.u32() as usize;
                let rings = (0..n).map(|_| {
                    let k = r.u32() as usize;
                    r.pts(dims, k)
                });
                out.push((3, rings.collect()));
            }
            4..=7 => {
                let n = r.u32();
                for _ in 0..n {
                    geom(r, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if !b.is_empty() {
        geom(&mut R(b, 0, true), &mut out);
    }
    out
}

fn shapely_type(t: &str) -> String {
    match t {
        "POINT" => "Point",
        "LINESTRING" => "LineString",
        "POLYGON" => "Polygon",
        "MULTIPOINT" => "MultiPoint",
        "MULTILINESTRING" => "MultiLineString",
        "MULTIPOLYGON" => "MultiPolygon",
        "GEOMETRYCOLLECTION" => "GeometryCollection",
        o => o,
    }
    .to_string()
}

/// (kind, mode) of a layer name: edges_<mode>, nodes_<mode>, ...boundary.
fn parse_layer(name: &str) -> (&'static str, Option<String>) {
    for kind in ["edges", "nodes"] {
        if let Some(i) = name.find(&format!("{kind}_")) {
            return (kind, Some(name[i + kind.len() + 1..].to_string()));
        }
    }
    if name == "boundary" || name.ends_with("_boundary") {
        return ("boundary", None);
    }
    ("other", None)
}

fn e(e: duckdb::Error) -> String {
    e.to_string()
}

struct Layer {
    name: String,
    kind: &'static str,
    mode: Option<String>,
    /// a FROM item that reads the layer
    from: String,
    geom: String,
    epsg: Option<i64>,
    crs_name: Option<String>,
}

/// The format and the layers of a .gpkg file (or another file of GDAL) or a folder of shapefiles.
fn discover(con: &Connection, path: &Path) -> Result<(String, Vec<Layer>)> {
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let mut files: Vec<(PathBuf, bool)> = Vec::new(); // (file, a shapefile: one layer, no layer name)
    let fmt = if path.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|x| format!("{}: {x}", path.display()))?
            .flatten()
            .map(|d| d.path())
            .filter(|p| p.extension().is_some_and(|x| x == "shp"))
            .collect();
        v.sort();
        files.extend(v.into_iter().map(|p| (p, true)));
        "Shapefile".to_string()
    } else {
        files.push((path.to_path_buf(), false));
        match path.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase().as_str() {
            "gpkg" => "GeoPackage".into(),
            x => x.to_uppercase(),
        }
    };
    let mut layers = Vec::new();
    for (f, shp) in files {
        let mut st = con
            .prepare(&format!(
                "SELECT l.name, l.geometry_fields[1].name, l.geometry_fields[1].crs.auth_name, l.geometry_fields[1].crs.auth_code, \
                 l.geometry_fields[1].crs.name FROM (SELECT unnest(layers) l FROM ST_Read_Meta('{}'))",
                q(&f)
            ))
            .map_err(e)?;
        let rows: Vec<(String, Option<String>, Option<String>, Option<String>, Option<String>)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map_err(e)?
            .collect::<std::result::Result<_, _>>()
            .map_err(e)?;
        for (lname, geom, auth, code, cname) in rows {
            let name = if shp { f.file_stem().unwrap().to_string_lossy().to_string() } else { lname.clone() };
            let from = if shp { format!("ST_Read('{}')", q(&f)) } else { format!("ST_Read('{}', layer = '{}')", q(&f), lname.replace('\'', "''")) };
            let (kind, mode) = parse_layer(&name);
            let epsg = (auth.as_deref() == Some("EPSG")).then(|| code.and_then(|c| c.parse().ok())).flatten();
            layers.push(Layer { name, kind, mode, from, geom: geom.unwrap_or_else(|| "geom".into()), epsg, crs_name: cname });
        }
    }
    Ok((fmt, layers))
}

/// The dtype name, the values of `edge_id` as Python's `str()` of the pandas column, and the number of NULLs. An integer
/// column with a NULL is float64 in pandas (a NULL is "nan").
fn edge_ids(con: &Connection, l: &Layer, dtype: &str) -> Result<(String, Vec<Option<String>>, usize)> {
    let mut st = con.prepare(&format!("SELECT edge_id::VARCHAR, TRY_CAST(edge_id AS DOUBLE) FROM {}", l.from)).map_err(e)?;
    let v: Vec<(Option<String>, Option<f64>)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1).ok().flatten()))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
    let nulls = v.iter().filter(|x| x.0.is_none()).count();
    let has_null = nulls > 0;
    let int = |w: &str| if has_null { "float64".to_string() } else { w.to_string() };
    let dt = match dtype {
        "BIGINT" | "HUGEINT" | "UBIGINT" => int("int64"),
        "INTEGER" | "UINTEGER" => int("int32"),
        "SMALLINT" => int("int16"),
        "DOUBLE" => "float64".into(),
        "FLOAT" => "float32".into(),
        "VARCHAR" => "object".into(),
        o => o.to_lowercase(),
    };
    let vals = if dt.starts_with("float") {
        v.into_iter().map(|(_, f)| Some(f.map(crate::sumo::py_float).unwrap_or_else(|| "nan".into()))).collect()
    } else {
        v.into_iter().map(|x| x.0).collect()
    };
    Ok((dt, vals, nulls))
}

/// The data of the page, as `build_payload`.
pub fn build_payload(path: &Path, source_db: Option<&Path>, name: Option<&str>) -> Result<J> {
    let con = Connection::open_in_memory().map_err(e)?;
    con.execute_batch("INSTALL spatial; LOAD spatial;").map_err(|x| format!("the spatial extension of DuckDB: {x}"))?;
    let (fmt, layers) = discover(&con, path)?;
    if layers.is_empty() {
        return Err(format!("no layers found in {}", path.display()));
    }
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for l in &layers {
        let g = &l.geom;
        let b: (Option<f64>, Option<f64>, Option<f64>, Option<f64>) = con
            .query_row(
                &format!(
                    "SELECT min(ST_XMin({g})), min(ST_YMin({g})), max(ST_XMax({g})), max(ST_YMax({g})) FROM {} WHERE {g} IS NOT NULL AND NOT ST_IsEmpty({g})",
                    l.from
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(e)?;
        if let (Some(a), Some(b2), Some(c), Some(d)) = b {
            if [a, b2, c, d].iter().all(|v| v.is_finite()) {
                minx = minx.min(a);
                miny = miny.min(b2);
                maxx = maxx.max(c);
                maxy = maxy.max(d);
            }
        }
    }
    if minx.is_infinite() {
        return Err("export has no drawable geometry".into());
    }
    // one equirectangular projection for all layers (cos-lat corrected), then a grid of about Q x Q
    let midlat = (miny + maxy) / 2.0;
    let kx = match midlat.to_radians().cos() {
        0.0 => 1.0,
        k => k,
    };
    let (pw, ph) = ((maxx - minx) * kx, maxy - miny);
    let scale = Q / pw.max(ph).max(1e-9);
    let qx = |x: f64| ((x - minx) * kx * scale).round_ties_even() as i64;
    let qy = |y: f64| ((maxy - y) * scale).round_ties_even() as i64;

    // the edge_id of each mode in the source db
    let mut src_ids: HashMap<String, HashSet<String>> = HashMap::new();
    if let Some(db) = source_db {
        let c = open(db)?;
        let mut st = c
            .prepare(
                "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name='edges' \
                 AND schema_name NOT IN ('information_schema','pg_catalog','main','raw')",
            )
            .map_err(e)?;
        let modes: Vec<String> = st.query_map([], |r| r.get(0)).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
        for m in modes {
            let mut st = c.prepare(&format!("SELECT CAST(edge_id AS VARCHAR) FROM \"{m}\".edges")).map_err(e)?;
            let ids: HashSet<String> = st.query_map([], |r| r.get(0)).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
            src_ids.insert(m, ids);
        }
    }

    let mut geo: Vec<(String, Vec<J>, Vec<J>)> = Vec::new(); // mode, edges, nodes (in the order of the first layer)
    let mut boundary: Vec<J> = Vec::new();
    let mut qa: Vec<J> = Vec::new();
    let (mut total, mut crs_set, mut worst) = (0i64, BTreeSet::new(), 0u8);
    let mut eid_crit = false;
    let (mut rt_done, mut rt_ok) = (false, true);
    for l in &layers {
        let cols: Vec<(String, String)> = {
            let mut st = con.prepare(&format!("DESCRIBE SELECT * FROM {}", l.from)).map_err(e)?;
            let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
            v
        };
        let n: i64 = con.query_row(&format!("SELECT count(*) FROM {}", l.from), [], |r| r.get(0)).map_err(e)?;
        total += n;
        let crs_txt = match (l.epsg, &l.crs_name) {
            (Some(c), _) => format!("EPSG:{c}"),
            // ponytail: pyproj's text of a CRS with no EPSG code is not copied; the name of the CRS stands in for it
            (None, Some(nm)) => nm.chars().take(24).collect(),
            (None, None) => "none".into(),
        };
        crs_set.insert(crs_txt.clone());
        let g = &l.geom;
        let mut st = con.prepare(&format!("SELECT DISTINCT ST_GeometryType({g})::VARCHAR FROM {} WHERE {g} IS NOT NULL", l.from)).map_err(e)?;
        let types: BTreeSet<String> = st
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(e)?
            .map(|t| t.map(|t| shapely_type(&t)))
            .collect::<std::result::Result<_, _>>()
            .map_err(e)?;
        let mut notes: Vec<(&str, String)> = Vec::new();
        let mut bump = |lvl: u8| worst = worst.max(lvl);
        if l.epsg != Some(4326) {
            notes.push(("crit", format!("CRS is {crs_txt}, expected EPSG:4326")));
            bump(2);
        }
        if n == 0 {
            notes.push(("warn", "empty layer".into()));
            bump(1);
        }
        let eid_col = cols.iter().find(|c| c.0 == "edge_id");
        let (mut eid_dtype, mut eid_sample, mut ids) = (J::Null, J::Null, None);
        if l.kind == "edges" {
            match eid_col {
                None => {
                    notes.push(("crit", "no edge_id column".into()));
                    bump(2);
                }
                Some((_, dtype)) => {
                    let (dt, vals, nulls) = edge_ids(&con, l, dtype)?;
                    eid_dtype = s(&dt);
                    if n > 0 {
                        eid_sample = s(vals[0].as_deref().unwrap_or("None"));
                    }
                    if dt.starts_with("float") {
                        notes.push(("crit", "edge_id is float — precision lost (ids corrupted)".into()));
                        bump(2);
                    } else if dt == "object" {
                        notes.push(("ok", "edge_id exact (text — shapefile-safe)".into()));
                    } else {
                        notes.push(("ok", format!("edge_id exact ({dt})")));
                    }
                    if n > 0 && nulls > 0 {
                        notes.push(("crit", "null edge_id(s)".into()));
                        bump(2);
                    }
                    let text: Vec<String> = vals.iter().map(|v| v.clone().unwrap_or_else(|| "None".into())).collect();
                    let set: HashSet<String> = text.iter().cloned().collect();
                    if n > 0 && set.len() < text.len() {
                        notes.push(("warn", "duplicate edge_id(s)".into()));
                        bump(1);
                    }
                    ids = Some(set);
                }
            }
        }
        // the edge_id against the db
        let mut roundtrip = J::Null;
        if let (true, Some(src)) = (l.kind == "edges", l.mode.as_ref().and_then(|m| src_ids.get(m))) {
            let exp = ids.clone().unwrap_or_default();
            let (missing, extra) = (src.difference(&exp).count(), exp.difference(src).count());
            roundtrip = obj(vec![
                ("src", J::Int(src.len() as i64)),
                ("exp", J::Int(exp.len() as i64)),
                ("missing", J::Int(missing as i64)),
                ("extra", J::Int(extra as i64)),
            ]);
            rt_done = true;
            if missing > 0 || extra > 0 {
                rt_ok = false;
                notes.push(("crit", format!("round-trip: {missing} missing, {extra} extra vs db")));
                bump(2);
            } else {
                notes.push(("ok", format!("round-trip: all {} ids match db", thousands(src.len()))));
            }
        }
        eid_crit |= notes.iter().any(|(sev, t)| *sev == "crit" && t.contains("edge_id"));
        qa.push(obj(vec![
            ("name", s(&l.name)),
            ("kind", s(l.kind)),
            ("mode", l.mode.as_deref().map(s).unwrap_or(J::Null)),
            ("features", J::Int(n)),
            ("geom", s(&if types.is_empty() { "—".to_string() } else { types.into_iter().collect::<Vec<_>>().join(", ") })),
            ("crs", s(&crs_txt)),
            ("eid_dtype", eid_dtype),
            ("eid_sample", eid_sample),
            ("notes", J::Arr(notes.into_iter().map(|(sv, t)| obj(vec![("sev", s(sv)), ("text", J::Str(t))])).collect())),
            ("roundtrip", roundtrip),
        ]));

        // the geometry on the grid of the map
        if l.kind == "boundary" || l.kind == "edges" || l.kind == "nodes" {
            let hw = if l.kind == "edges" && cols.iter().any(|c| c.0 == "highway") { "highway::VARCHAR" } else { "NULL::VARCHAR" };
            let mut st = con.prepare(&format!("SELECT {hw}, ST_AsWKB({g}) FROM {}", l.from)).map_err(e)?;
            let rows: Vec<(Option<String>, Option<Vec<u8>>)> =
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
            let bucket = |geo: &mut Vec<(String, Vec<J>, Vec<J>)>| -> usize {
                let m = l.mode.clone().unwrap_or_default();
                match geo.iter().position(|x| x.0 == m) {
                    Some(i) => i,
                    None => {
                        geo.push((m, vec![], vec![]));
                        geo.len() - 1
                    }
                }
            };
            match l.kind {
                "edges" => {
                    let i = bucket(&mut geo);
                    for (h, w) in rows {
                        let ci = class_index(h.as_deref());
                        for (t, parts) in wkb(&w.unwrap_or_default()) {
                            if t != 2 {
                                continue;
                            }
                            let mut flat = vec![J::Int(ci)];
                            for (x, y) in &parts[0] {
                                flat.push(J::Int(qx(*x)));
                                flat.push(J::Int(qy(*y)));
                            }
                            if flat.len() > 3 {
                                geo[i].1.push(J::Arr(flat));
                            }
                        }
                    }
                }
                "nodes" => {
                    let i = bucket(&mut geo);
                    for (_, w) in rows {
                        let g = wkb(&w.unwrap_or_default());
                        // a Point only (not a part of a MultiPoint)
                        if let [(1, p)] = g.as_slice() {
                            if let Some(&[(x, y)]) = p.first().map(|v| v.as_slice()) {
                                geo[i].2.push(J::Int(qx(x)));
                                geo[i].2.push(J::Int(qy(y)));
                            }
                        }
                    }
                }
                _ => {
                    for (_, w) in rows {
                        for (t, rings) in wkb(&w.unwrap_or_default()) {
                            if t != 3 || rings.is_empty() {
                                continue;
                            }
                            let flat: Vec<J> = rings[0].iter().flat_map(|(x, y)| [J::Int(qx(*x)), J::Int(qy(*y))]).collect();
                            if flat.len() > 4 {
                                boundary.push(J::Arr(flat));
                            }
                        }
                    }
                }
            }
        }
    }
    let mut modes_present: Vec<&str> = geo.iter().filter(|g| !g.1.is_empty() || !g.2.is_empty()).map(|g| g.0.as_str()).collect();
    modes_present.sort();
    let verdict = ["pass", "warn", "fail"][worst as usize];
    let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let stem = path.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    Ok(obj(vec![
        (
            "meta",
            obj(vec![
                ("file", s(&file)),
                ("format", s(&fmt)),
                ("source_db", source_db.and_then(|p| p.file_name()).map(|f| s(&f.to_string_lossy())).unwrap_or(J::Null)),
                ("name", s(name.unwrap_or(&stem))),
            ]),
        ),
        ("proj", obj(vec![("minx", J::Float(minx)), ("maxy", J::Float(maxy)), ("kx", J::Float(kx)), ("scale", J::Float(scale))])),
        ("grid", obj(vec![("w", J::Int((pw * scale).round_ties_even() as i64)), ("h", J::Int((ph * scale).round_ties_even() as i64))])),
        ("verdict", s(verdict)),
        (
            "summary",
            obj(vec![
                ("layers", J::Int(qa.len() as i64)),
                ("features", J::Int(total)),
                ("crs", s(if crs_set.len() == 1 { crs_set.iter().next().unwrap() } else { "mixed" })),
                ("edge_id", s(if worst == 2 && eid_crit { "corrupted" } else { "exact" })),
                ("roundtrip", s(if rt_done && rt_ok { "pass" } else if rt_done { "fail" } else { "n/a" })),
                ("modes", J::Arr(modes_present.into_iter().map(s).collect())),
            ]),
        ),
        (
            "classes",
            J::Arr(CLASSES.iter().map(|(k, lb, c, w)| obj(vec![("key", s(k)), ("label", s(lb)), ("color", s(c)), ("width", J::Float(*w))])).collect()),
        ),
        ("layers", J::Arr(qa)),
        ("geo", J::Obj(geo.into_iter().map(|(m, ed, nd)| (m, obj(vec![("edges", J::Arr(ed)), ("nodes", J::Arr(nd))]))).collect())),
        ("boundary", J::Arr(boundary)),
    ]))
}

/// The page as `render_html(payload, standalone=True)`.
pub fn render_html(payload: &J) -> String {
    let file = match payload.get("meta").and_then(|m| m.get("file")) {
        Some(J::Str(f)) => f.clone(),
        _ => String::new(),
    };
    let title = format!("GIS export debug · {file}");
    let content = TEMPLATE.replace("/*__PAYLOAD__*/null", &payload.dump(None)).replace("__TITLE__", &title);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{title}</title>\n</head>\n<body>\n{content}\n</body>\n</html>\n"
    )
}

/// Write the page of the export `path` to `out`. Returns the data of the page.
pub fn write_debug(path: &Path, source_db: Option<&Path>, out: &Path, name: Option<&str>) -> Result<J> {
    let payload = build_payload(path, source_db, name)?;
    crate::write_text(out, &render_html(&payload), false)?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_as_python() {
        let j = obj(vec![("a", J::Arr(vec![J::Int(1), J::Float(0.5), J::Float(2.0)])), ("b", s("x—\"é")), ("c", J::Obj(vec![])), ("d", J::Null)]);
        assert_eq!(j.dump(None), r#"{"a":[1,0.5,2.0],"b":"x\u2014\"\u00e9","c":{},"d":null}"#);
        assert_eq!(j.dump(Some(1)), "{\n \"a\": [\n  1,\n  0.5,\n  2.0\n ],\n \"b\": \"x\\u2014\\\"\\u00e9\",\n \"c\": {},\n \"d\": null\n}");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(999), "999");
    }
}
