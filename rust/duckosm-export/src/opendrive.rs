//! The ASAM OpenDRIVE export (.xodr), the port of src/duckosm/opendrive.py. docs/design/opendrive_export.md.
//!
//! Phase 1 (the core db): one road for each directed edge, a reference line of straight pieces in a metric CRS and the
//! driving lanes on the right; no junctions. Phase 2 (`junctions`, a GMNS db): the links are roads, and at each junction
//! node each movement is a connecting road.

use crate::stream::each_chunk;
use crate::{data_utm_crs, has_table, open, quoteattr, Out, Result};
use duckdb::Connection;
use std::path::{Path, PathBuf};

/// The lane width (metres) when OSM has no width:lanes.
const LANE_W: &str = "3.25";

/// The proj4 text of the geoReference header, as `_geo_ref`. A CRS that is not EPSG:3006 or a WGS84 UTM zone: the text
/// of PROJ's `projinfo` (pyproj's `to_proj4` uses the same PROJ function), else `+init=<crs> +units=m +no_defs`.
fn geo_ref(crs: &str) -> String {
    if crs == "EPSG:3006" {
        return "+proj=tmerc +lat_0=0 +lon_0=15 +k=0.9996 +x_0=500000 +y_0=0 +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs".into();
    }
    let up = crs.to_uppercase();
    let code = up.strip_prefix("EPSG:").unwrap_or(&up);
    if let Ok(c) = code.parse::<i64>() {
        if (32601..=32760).contains(&c) && c % 100 <= 60 {
            let south = if c >= 32701 { " +south" } else { "" };
            return format!("+proj=utm +zone={}{south} +datum=WGS84 +units=m +no_defs", c % 100);
        }
    }
    // ponytail: projinfo can give a text that differs from pyproj's for an unusual CRS; the default (UTM) is exact
    if let Ok(o) = std::process::Command::new("projinfo").args(["-o", "PROJ", "-q", crs]).output() {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if o.status.success() && s.starts_with("+proj") {
            return s;
        }
    }
    format!("+init={} +units=m +no_defs", crs.to_lowercase())
}

fn coords(wkt: &str) -> Vec<(f64, f64)> {
    let (Some(a), Some(b)) = (wkt.find('('), wkt.rfind(')')) else { return vec![] };
    wkt[a + 1..b]
        .split(',')
        .filter_map(|p| {
            let mut it = p.split_whitespace().map(|v| v.parse::<f64>());
            match (it.next(), it.next()) {
                (Some(Ok(x)), Some(Ok(y))) => Some((x, y)),
                _ => None,
            }
        })
        .collect()
}

/// The straight pieces of a line: (s, x, y, heading, length), and the total length.
fn segments(pts: &[(f64, f64)]) -> (Vec<(f64, f64, f64, f64, f64)>, f64) {
    let mut segs = Vec::new();
    let mut s = 0.0;
    for w in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        let length = (x1 - x0).hypot(y1 - y0);
        if length <= 0.0 {
            continue;
        }
        segs.push((s, x0, y0, (y1 - y0).atan2(x1 - x0), length));
        s += length;
    }
    (segs, s)
}

/// The file `out`, written one line at a time. The header needs the extent of all points, so the file starts with a space
/// as large as the longest possible header, and `finish` writes the header there and moves the body back. The disk then
/// holds one copy of the file, not a body file and the file.
struct Body {
    o: Out,
    path: PathBuf,
    reserved: usize,
    ext: [f64; 4],
}

/// The header of the file for the extent `ext` (west, south, east, north).
fn header(name: &str, crs: &str, ext: [f64; 4]) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<OpenDRIVE>\n\t<header revMajor=\"1\" revMinor=\"7\" name={} version=\"1.00\" \
         north=\"{:.2}\" south=\"{:.2}\" east=\"{:.2}\" west=\"{:.2}\">\n\t\t<geoReference><![CDATA[{}]]></geoReference>\n\t</header>\n",
        quoteattr(name),
        ext[3],
        ext[1],
        ext[2],
        ext[0],
        geo_ref(crs)
    )
}

impl Body {
    fn new(out: &Path, name: &str, crs: &str) -> Result<Body> {
        let reserved = header(name, crs, [-f64::MAX; 4]).len();
        let mut o = Out::create(out, false)?;
        o.put(&" ".repeat(reserved))?;
        Ok(Body { o, path: out.to_path_buf(), reserved, ext: [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY] })
    }

    fn line(&mut self, l: &str) -> Result<()> {
        self.o.put(l)?;
        self.o.put("\n")
    }

    fn points(&mut self, pts: &[(f64, f64)]) {
        for &(x, y) in pts {
            self.ext = [self.ext[0].min(x), self.ext[1].min(y), self.ext[2].max(x), self.ext[3].max(y)];
        }
    }

    fn planview(&mut self, segs: &[(f64, f64, f64, f64, f64)]) -> Result<()> {
        self.line("\t\t<planView>")?;
        for (s, x, y, hdg, length) in segs {
            self.line(&format!(
                "\t\t\t<geometry s=\"{s:.4}\" x=\"{x:.4}\" y=\"{y:.4}\" hdg=\"{hdg:.6}\" length=\"{length:.4}\"><line/></geometry>"
            ))?;
        }
        self.line("\t\t</planView>")
    }

    fn lanes(&mut self, n: i64) -> Result<()> {
        self.line("\t\t<lanes>")?;
        self.line("\t\t\t<laneSection s=\"0\">")?;
        self.line("\t\t\t\t<center><lane id=\"0\" type=\"none\" level=\"false\"/></center>")?;
        self.line("\t\t\t\t<right>")?;
        for i in 1..=n {
            self.line(&format!("\t\t\t\t\t<lane id=\"-{i}\" type=\"driving\" level=\"false\">"))?;
            self.line(&format!("\t\t\t\t\t\t<width sOffset=\"0\" a=\"{LANE_W}\" b=\"0\" c=\"0\" d=\"0\"/>"))?;
            self.line("\t\t\t\t\t</lane>")?;
        }
        self.line("\t\t\t\t</right>")?;
        self.line("\t\t\t</laneSection>")?;
        self.line("\t\t</lanes>")
    }

    /// The header at the start of the file, the body moved back to it, and the end tag. An error removes the file.
    fn finish(self, name: &str, crs: &str) -> Result<()> {
        use std::io::{Read, Seek, SeekFrom, Write};
        let Body { o, path, reserved, ext } = self;
        let r = o.finish().and_then(|_| {
            if ext[0] == f64::INFINITY {
                return Err("no road with a geometry of two or more points".to_string());
            }
            let err = |e: std::io::Error| format!("{}: {e}", path.display());
            let head = header(name, crs, ext);
            let mut f = std::fs::OpenOptions::new().read(true).write(true).open(&path).map_err(err)?;
            let len = f.metadata().map_err(err)?.len();
            f.write_all(head.as_bytes()).map_err(err)?;
            // the body moves back by `reserved - head.len()` bytes; the target is before the source, so the copy goes forward
            let (mut from, mut to) = (reserved as u64, head.len() as u64);
            let mut buf = vec![0u8; 1 << 20];
            while from < len {
                let n = buf.len().min((len - from) as usize);
                f.seek(SeekFrom::Start(from)).map_err(err)?;
                f.read_exact(&mut buf[..n]).map_err(err)?;
                f.seek(SeekFrom::Start(to)).map_err(err)?;
                f.write_all(&buf[..n]).map_err(err)?;
                from += n as u64;
                to += n as u64;
            }
            f.write_all(b"</OpenDRIVE>\n").map_err(err)?;
            f.set_len(to + 13).map_err(err)
        });
        if r.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        r
    }
}

fn tx(col: &str) -> String {
    format!("ST_AsText(ST_Transform({col}, 'EPSG:4326', ?, always_xy := true))")
}

fn e(e: duckdb::Error) -> String {
    e.to_string()
}

pub struct Summary {
    pub roads: usize,
    pub connecting_roads: usize,
    pub junctions: usize,
    pub crs: String,
}

/// The straight pieces of a road with two or more points and a length, with its points in the extent. None: no road.
fn road(b: &mut Body, wkt: Option<&str>) -> Option<(Vec<(f64, f64, f64, f64, f64)>, f64)> {
    let pts = coords(wkt.unwrap_or(""));
    if pts.len() < 2 {
        return None;
    }
    let (segs, total) = segments(&pts);
    if segs.is_empty() {
        return None;
    }
    b.points(&pts);
    Some((segs, total))
}

fn phase1(con: &Connection, mode: &str, crs: &str, b: &mut Body) -> Result<Summary> {
    let have_z: i64 = con
        .query_row(
            "SELECT count(*) FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'edges' AND column_name = 'z_from'",
            [mode],
            |r| r.get(0),
        )
        .map_err(e)?;
    let zsel = if have_z > 0 { ", z_from::DOUBLE, z_to::DOUBLE" } else { ", NULL::DOUBLE, NULL::DOUBLE" };
    let sql = format!("SELECT edge_id, COALESCE(lanes, 1)::BIGINT, name, {}{zsel} FROM {mode}.edges WHERE geometry IS NOT NULL", tx("geometry"));
    let mut roads = 0;
    each_chunk(con, &sql, [crs], |c| {
        for r in 0..c.len {
            let Some((segs, total)) = road(b, c.str(3, r)) else { continue };
            b.line(&format!(
                "\t<road name={} length=\"{total:.4}\" id=\"{}\" junction=\"-1\">",
                quoteattr(c.str(2, r).unwrap_or("")),
                c.i64(0, r).unwrap_or_default()
            ))?;
            b.planview(&segs)?;
            if let (Some(z0), Some(z1)) = (c.f64(4, r), c.f64(5, r)) {
                // a straight ramp from z_from to z_to: after planView, before lanes (the order of the schema)
                let slope = if total > 0.0 { (z1 - z0) / total } else { 0.0 };
                b.line("\t\t<elevationProfile>")?;
                b.line(&format!("\t\t\t<elevation s=\"0\" a=\"{z0:.4}\" b=\"{slope:.6}\" c=\"0\" d=\"0\"/>"))?;
                b.line("\t\t</elevationProfile>")?;
            }
            b.lanes(c.i64(1, r).unwrap_or(1).max(1))?;
            b.line("\t</road>")?;
            roads += 1;
        }
        Ok(())
    })?;
    Ok(Summary { roads, connecting_roads: 0, junctions: 0, crs: crs.into() })
}

/// Phase 2. The movements, the junction nodes and the road-to-road links stay in DuckDB temp tables (they spill to disk), not
/// in this program: its memory does not grow with the area.
fn junctions(con: &Connection, g: &str, crs: &str, b: &mut Body) -> Result<Summary> {
    // a junction: a node where an inbound link has two or more outbound choices, or where three or more inbound links meet.
    // At a node that is not a junction: road to road, the first movement (in the order of the table) wins
    con.execute_batch(&format!(
        "CREATE TEMP TABLE _mv AS SELECT rowid AS rid, node_id, ib_link_id AS ib, ob_link_id AS ob FROM {g}.movement
           WHERE geometry IS NOT NULL AND ib_link_id IS NOT NULL AND ob_link_id IS NOT NULL;
         CREATE TEMP TABLE _jn AS SELECT node_id FROM (SELECT node_id, ib, count(DISTINCT ob) AS n FROM _mv GROUP BY ALL)
           GROUP BY node_id HAVING count(*) >= 3 OR max(n) >= 2;
         CREATE TEMP TABLE _succ AS SELECT ib, arg_min(ob, rid) AS ob FROM _mv ANTI JOIN _jn USING (node_id) GROUP BY ib;
         CREATE TEMP TABLE _pred AS SELECT ob, arg_min(ib, rid) AS ib FROM _mv ANTI JOIN _jn USING (node_id) GROUP BY ob;"
    ))
    .map_err(e)?;
    let sql = format!(
        "SELECT l.link_id, l.from_node_id, l.to_node_id, COALESCE(l.lanes, 1)::BIGINT, l.name, {}, jf.node_id IS NOT NULL,
           jt.node_id IS NOT NULL, p.ib, s.ob
         FROM {g}.link l LEFT JOIN _jn jf ON jf.node_id = l.from_node_id LEFT JOIN _jn jt ON jt.node_id = l.to_node_id
           LEFT JOIN _pred p ON p.ob = l.link_id LEFT JOIN _succ s ON s.ib = l.link_id
         WHERE l.geom IS NOT NULL ORDER BY l.rowid",
        tx("l.geom")
    );
    let mut roads = 0;
    each_chunk(con, &sql, [crs], |c| {
        for r in 0..c.len {
            let Some((segs, total)) = road(b, c.str(5, r)) else { continue };
            let (lid, fnode, tnode) = (c.i64(0, r).unwrap_or_default(), c.i64(1, r).unwrap_or_default(), c.i64(2, r).unwrap_or_default());
            let mut link = Vec::new();
            if c.bool(6, r) == Some(true) {
                link.push(format!("\t\t\t<predecessor elementType=\"junction\" elementId=\"{fnode}\"/>"));
            } else if let Some(p) = c.i64(8, r) {
                link.push(format!("\t\t\t<predecessor elementType=\"road\" elementId=\"{p}\" contactPoint=\"end\"/>"));
            }
            if c.bool(7, r) == Some(true) {
                link.push(format!("\t\t\t<successor elementType=\"junction\" elementId=\"{tnode}\"/>"));
            } else if let Some(s) = c.i64(9, r) {
                link.push(format!("\t\t\t<successor elementType=\"road\" elementId=\"{s}\" contactPoint=\"start\"/>"));
            }
            b.line(&format!("\t<road name={} length=\"{total:.4}\" id=\"{lid}\" junction=\"-1\">", quoteattr(c.str(4, r).unwrap_or(""))))?;
            if !link.is_empty() {
                b.line("\t\t<link>")?;
                for l in &link {
                    b.line(l)?;
                }
                b.line("\t\t</link>")?;
            }
            b.planview(&segs)?;
            b.lanes(c.i64(3, r).unwrap_or(1).max(1))?;
            b.line("\t</road>")?;
            roads += 1;
        }
        Ok(())
    })?;
    // a connecting road for each movement at a junction
    let sql = format!(
        "SELECT m.node_id, m.ib_link_id, m.ob_link_id, {} FROM {g}.movement m SEMI JOIN _jn USING (node_id)
         WHERE m.geometry IS NOT NULL AND m.ib_link_id IS NOT NULL AND m.ob_link_id IS NOT NULL ORDER BY m.rowid",
        tx("ST_GeomFromText(m.geometry)")
    );
    // (node, connecting road, inbound road) of each connecting road: in a second connection, as a write in the connection of
    // the stream would end the stream
    let scratch = con.try_clone().map_err(e)?;
    scratch.execute_batch("CREATE TEMP TABLE _conn(node_id BIGINT, ci BIGINT, ib BIGINT)").map_err(e)?;
    let mut ci: i64 = 0;
    {
        let mut app = scratch.appender_to_catalog_and_db("_conn", "temp", "main").map_err(e)?;
        each_chunk(con, &sql, [crs], |c| {
            for r in 0..c.len {
                let Some(w) = c.str(3, r).filter(|w| !w.is_empty()) else { continue };
                let Some((segs, total)) = road(b, Some(w)) else { continue };
                let (node, ib, ob) = (c.i64(0, r).unwrap_or_default(), c.i64(1, r).unwrap_or_default(), c.i64(2, r).unwrap_or_default());
                b.line(&format!("\t<road name=\"\" length=\"{total:.4}\" id=\"c{ci}\" junction=\"{node}\">"))?;
                b.line("\t\t<link>")?;
                b.line(&format!("\t\t\t<predecessor elementType=\"road\" elementId=\"{ib}\" contactPoint=\"end\"/>"))?;
                b.line(&format!("\t\t\t<successor elementType=\"road\" elementId=\"{ob}\" contactPoint=\"start\"/>"))?;
                b.line("\t\t</link>")?;
                b.planview(&segs)?;
                b.lanes(1)?;
                b.line("\t</road>")?;
                app.append_row(duckdb::params![node, ci, ib]).map_err(e)?;
                ci += 1;
            }
            Ok(())
        })?;
        app.flush().map_err(e)?;
    }
    // the junctions in the order of their first connecting road
    let (mut junctions, mut cur, mut k) = (0, None, 0);
    each_chunk(&scratch, "SELECT node_id, ci, ib FROM _conn ORDER BY min(ci) OVER (PARTITION BY node_id), ci", [], |c| {
        for r in 0..c.len {
            let (node, cid, ib) = (c.i64(0, r).unwrap_or_default(), c.i64(1, r).unwrap_or_default(), c.i64(2, r).unwrap_or_default());
            if cur != Some(node) {
                if cur.is_some() {
                    b.line("\t</junction>")?;
                }
                b.line(&format!("\t<junction id=\"{node}\" name=\"\">"))?;
                (cur, k, junctions) = (Some(node), 0, junctions + 1);
            }
            b.line(&format!("\t\t<connection id=\"{k}\" incomingRoad=\"{ib}\" connectingRoad=\"c{cid}\" contactPoint=\"start\">"))?;
            b.line("\t\t\t<laneLink from=\"-1\" to=\"-1\"/>")?;
            b.line("\t\t</connection>")?;
            k += 1;
        }
        Ok(())
    })?;
    if cur.is_some() {
        b.line("\t</junction>")?;
    }
    con.execute_batch("DROP TABLE _mv; DROP TABLE _jn; DROP TABLE _succ; DROP TABLE _pred").map_err(e)?;
    Ok(Summary { roads, connecting_roads: ci as usize, junctions, crs: crs.into() })
}

/// Write an OpenDRIVE file from the database `db`, as `to_opendrive`. `crs`: None = the UTM zone of the data.
/// `with_junctions`: `db` is a GMNS db (`gmns_<mode>.link` and `.movement`).
pub fn to_opendrive(db: &Path, out: &Path, mode: &str, crs: Option<&str>, with_junctions: bool) -> Result<Summary> {
    let con = open(db)?;
    let name = db.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let (table, geom) = if with_junctions { (format!("gmns_{mode}.link"), "geom") } else { (format!("{mode}.edges"), "geometry") };
    if with_junctions && !has_table(&con, &format!("gmns_{mode}"), "movement")? {
        return Err(format!("--junctions needs a GMNS db (no 'gmns_{mode}.movement' in {}): make it with duckosm gmns and give its path, or in Python give the network to duckosm.hdmap.to_opendrive", db.display()));
    }
    if !with_junctions && !has_table(&con, mode, "edges")? {
        return Err(format!("no '{mode}.edges' in {} — is this a built duckOSM db with mode '{mode}'?", db.display()));
    }
    let crs = match crs {
        Some(c) => c.to_string(),
        None => data_utm_crs(&con, &table, geom)?,
    };
    let mut b = Body::new(out, name, &crs)?;
    let s = if with_junctions { junctions(&con, &format!("gmns_{mode}"), &crs, &mut b) } else { phase1(&con, mode, &crs, &mut b) };
    match s {
        Ok(s) => {
            b.finish(name, &crs)?;
            Ok(s)
        }
        Err(x) => {
            let p = b.path.clone();
            drop(b);
            let _ = std::fs::remove_file(p);
            Err(x)
        }
    }
}
