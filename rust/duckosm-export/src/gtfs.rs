//! The stops of a GTFS feed as GMNS `location` rows: the port of src/duckosm/gtfs.py (`read_stops`) and of
//! `_add_transit_stops` in src/duckosm/gmns.py, with the same SQL and the same choice of link.

use crate::gmns::Ctx;
use crate::shapely as shp;
use crate::Result;
use duckdb::Connection;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

type P = (f64, f64);

const FILES: [&str; 4] = ["stops.txt", "stop_times.txt", "trips.txt", "routes.txt"];

/// A stop of a feed: `loc_type` in OpenStreetMap's words (`bus_stop`, `tram_stop`, `train_station`, `entrance`, ...).
pub struct Stop {
    pub stop_id: String,
    pub name: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub loc_type: String,
    pub is_bus: bool,
}

/// The OSM-style name of the stop a GTFS route type calls at (basic types 0-12, the extended ones by their hundreds).
fn kind(t: i32) -> &'static str {
    let basic = match t {
        0 | 5 => Some("tram_stop"),
        1 => Some("subway_station"),
        2 | 12 => Some("train_station"),
        3 | 11 => Some("bus_stop"),
        4 => Some("ferry_terminal"),
        6 => Some("cable_car_station"),
        7 => Some("funicular_station"),
        _ => None,
    };
    let extended = || match t / 100 {
        1 => Some("train_station"),
        2 | 7 => Some("bus_stop"),
        4 => Some("subway_station"),
        9 => Some("tram_stop"),
        10 => Some("ferry_terminal"),
        _ => None,
    };
    basic.or_else(|| if t >= 100 { extended() } else { None }).unwrap_or("transit_stop")
}

fn u16_at(b: &[u8], i: usize) -> usize {
    u16::from_le_bytes([b[i], b[i + 1]]) as usize
}

fn u32_at(b: &[u8], i: usize) -> usize {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize
}

/// The files of a zip archive whose base name is in `want`: (base name, contents). Stored and deflated files only.
// ponytail: no zip64 (an archive or a file over 4 GB) and no encryption; a feed that has them gives an error.
fn unzip(path: &Path, want: &[&str]) -> Result<Option<Vec<(String, Vec<u8>)>>> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(eocd) = (0..b.len().saturating_sub(21)).rev().take(65557).find(|&i| b[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]) else {
        return Ok(None);
    };
    let (n, mut at) = (u16_at(&b, eocd + 10), u32_at(&b, eocd + 16));
    let bad = || format!("{}: a damaged zip file", path.display());
    let mut out = Vec::new();
    for _ in 0..n {
        if b.get(at..at + 46).is_none_or(|h| h[..4] != [0x50, 0x4b, 0x01, 0x02]) {
            return Err(bad());
        }
        let (method, size, name_len) = (u16_at(&b, at + 10), u32_at(&b, at + 20), u16_at(&b, at + 28));
        let (local, next) = (u32_at(&b, at + 42), at + 46 + name_len + u16_at(&b, at + 30) + u16_at(&b, at + 32));
        let name = String::from_utf8_lossy(&b[at + 46..at + 46 + name_len]).to_string();
        at = next;
        let base = name.rsplit('/').next().unwrap_or("").to_string();
        if !want.contains(&base.as_str()) {
            continue;
        }
        if size == 0xFFFF_FFFF || local == 0xFFFF_FFFF {
            return Err(format!("{}: {name}: a zip64 file is not read", path.display()));
        }
        let start = local + 30 + u16_at(&b, local + 26) + u16_at(&b, local + 28);
        let data = b.get(start..start + size).ok_or_else(bad)?;
        let bytes = match method {
            0 => data.to_vec(),
            8 => {
                let mut v = Vec::new();
                flate2::read::DeflateDecoder::new(data).read_to_end(&mut v).map_err(|e| format!("{}: {name}: {e}", path.display()))?;
                v
            }
            m => return Err(format!("{}: {name}: zip compression method {m} is not read", path.display())),
        };
        out.push((base, bytes));
    }
    Ok(Some(out))
}

/// `read_stops`: the stops of the GTFS feeds `paths` (zip files or folders). Stops (`location_type` 0 or empty) and station
/// entrances (2) only. With `stop_times.txt`, `trips.txt` and `routes.txt` a stop has the kind of the routes that call there
/// (a stop no trip calls at is `transit_stop`); without them every stop is a bus stop.
pub fn read_stops(paths: &[PathBuf]) -> Result<Vec<Stop>> {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut all = Vec::new();
    for path in paths {
        let tmp = std::env::temp_dir().join(format!("duckosm-gtfs-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let d = if path.is_dir() {
            path.clone()
        } else {
            let files = unzip(path, &FILES)?.ok_or_else(|| format!("{}: not a GTFS zip file or folder", path.display()))?;
            if !files.iter().any(|f| f.0 == "stops.txt") {
                return Err(format!("{}: no stops.txt: not a GTFS feed", path.display()));
            }
            std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
            for (n, bytes) in files {
                std::fs::write(tmp.join(n), bytes).map_err(|e| e.to_string())?;
            }
            tmp.clone()
        };
        let got = stops_of(&d, path);
        if d == tmp {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        all.extend(got?);
    }
    Ok(all)
}

/// The stops of one unpacked feed `d`.
fn stops_of(d: &Path, path: &Path) -> Result<Vec<Stop>> {
    let has = |f: &str| d.join(f).exists();
    if !has("stops.txt") {
        return Err(format!("{}: no stops.txt: not a GTFS feed", path.display()));
    }
    let con = Connection::open_in_memory().map_err(|e| e.to_string())?;
    let rd = |f: &str| {
        format!("read_csv('{}', all_varchar = true, header = true, null_padding = true, ignore_errors = true)", d.join(f).display().to_string().replace('\'', "''"))
    };
    let err = |e: duckdb::Error| e.to_string();
    let mut st = con.prepare(&format!("DESCRIBE SELECT * FROM {}", rd("stops.txt"))).map_err(err)?;
    let cols: Vec<String> = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
    let lt = if cols.iter().any(|c| c == "location_type") { "location_type" } else { "NULL" };
    let sql = format!(
        "SELECT stop_id, stop_name AS name, stop_lat::DOUBLE AS lat, stop_lon::DOUBLE AS lon,
            coalesce(try_cast({lt} AS INTEGER), 0) AS ltype FROM {}
            WHERE try_cast(stop_lat AS DOUBLE) IS NOT NULL AND try_cast(stop_lon AS DOUBLE) IS NOT NULL
              AND coalesce(try_cast({lt} AS INTEGER), 0) IN (0, 2)",
        rd("stops.txt")
    );
    let mut st = con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let rows: Vec<(String, Option<String>, f64, f64, i32)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
    let mut kinds: HashMap<String, String> = HashMap::new();
    let default = if ["stop_times.txt", "trips.txt", "routes.txt"].iter().all(|f| has(f)) {
        let sql = format!(
            "SELECT st.stop_id, list(DISTINCT try_cast(r.route_type AS INTEGER))::VARCHAR
                FROM {} st JOIN {} t ON t.trip_id = st.trip_id
                JOIN {} r ON r.route_id = t.route_id GROUP BY 1",
            rd("stop_times.txt"),
            rd("trips.txt"),
            rd("routes.txt")
        );
        let mut st = con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
        let got: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
        drop(st);
        for (stop_id, types) in got {
            // the list as text: [3, 700, NULL]
            let mut names: Vec<&str> = types.trim_matches(['[', ']']).split(',').filter_map(|x| x.trim().parse::<i32>().ok()).map(kind).collect();
            names.sort();
            names.dedup();
            let k = if names.contains(&"bus_stop") { "bus_stop" } else { names.first().copied().unwrap_or("transit_stop") };
            kinds.insert(stop_id, k.to_string());
        }
        "transit_stop"
    } else {
        "bus_stop" // no service data: assume a bus network
    };
    Ok(rows
        .into_iter()
        .map(|(stop_id, name, lat, lon, ltype)| {
            let loc_type = if ltype == 2 { "entrance".to_string() } else { kinds.get(&stop_id).cloned().unwrap_or_else(|| default.to_string()) };
            Stop { is_bus: loc_type == "bus_stop", stop_id, name, lat, lon, loc_type }
        })
        .collect())
}

/// `_add_transit_stops`: the stops as `location` rows of schema `sch`, each on the nearest link within `max_m` metres (driving:
/// the bus stops only). Of two links equally near (to 0.5 m), the one the stop is on the kerb side of; then the first in the
/// order of shapely's STRtree. An OSM `bus_stop` row within 25 m of a GTFS bus stop is the same stop and is removed.
pub(crate) fn add_transit_stops(c: &Ctx, sch: &str, mode: &str, stops: &[Stop], drive_side: &str, max_m: f64) -> Result<()> {
    let todo: Vec<&Stop> = stops.iter().filter(|s| mode != "driving" || s.is_bus).collect();
    let sql = format!("SELECT link_id, from_node_id, length, ST_AsText(geom) FROM {sch}.link");
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let links: Vec<(i64, i64, f64, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| e.to_string())?;
    if todo.is_empty() || links.is_empty() {
        return Ok(());
    }
    let scale = (todo[0].lat.to_radians().cos() * 111320.0, 111320.0);
    let geoms: Vec<Vec<P>> = links.iter().map(|l| shp::wkt_points(&l.3).into_iter().map(|(x, y)| (x * scale.0, y * scale.1)).collect()).collect();
    let envs: Vec<(f64, f64, f64, f64)> = geoms
        .iter()
        .map(|l| l.iter().fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |e, p| (e.0.min(p.0), e.1.min(p.1), e.2.max(p.0), e.3.max(p.1))))
        .collect();
    let order = shp::strtree_order(&envs);
    let kerb = if drive_side == "right" { -1.0 } else { 1.0 };
    let (mut rows, mut far) = (Vec::new(), 0);
    for s in &todo {
        let p = (s.lon * scale.0, s.lat * scale.1);
        let mut best: Option<((f64, u8), usize, f64)> = None;
        for &i in &order {
            let (e, g) = (envs[i], &geoms[i]);
            if e.0 > p.0 + max_m || e.2 < p.0 - max_m || e.1 > p.1 + max_m || e.3 < p.1 - max_m || shp::point_line_distance(p, g) > max_m {
                continue;
            }
            let (t, len) = (shp::project(g, p), shp::length(g));
            let (a, b) = (shp::interpolate(g, (t - 1.0).max(0.0)), shp::interpolate(g, (t + 1.0).min(len)));
            let q = shp::interpolate(g, t);
            let side = (b.0 - a.0) * (p.1 - q.1) - (b.1 - a.1) * (p.0 - q.0); // > 0: left of the link
            let key = (shp::py_round(shp::point_line_distance(p, g) * 2.0, 0) / 2.0, if side * kerb > 0.0 { 0 } else { 1 });
            if best.as_ref().is_none_or(|b| key < b.0) {
                best = Some((key, i, shp::project(g, p) / len));
            }
        }
        let Some((_, i, f)) = best else {
            far += 1;
            continue;
        };
        let (link_id, from_node, length, _) = &links[i];
        rows.push((format!("gtfs_{}_{link_id}", s.stop_id), *link_id, *from_node, f * length, s.lon, s.lat, &s.loc_type, &s.stop_id, &s.name));
    }
    if !rows.is_empty() {
        c.exec(
            "CREATE OR REPLACE TEMP TABLE _gtfs_loc(loc_id VARCHAR, link_id BIGINT, ref_node_id BIGINT, lr DOUBLE, x_coord DOUBLE,
             y_coord DOUBLE, loc_type VARCHAR, gtfs_stop_id VARCHAR, name VARCHAR)",
        )?;
        {
            let mut app = c.con.appender("_gtfs_loc").map_err(|e| e.to_string())?;
            for r in &rows {
                app.append_row(duckdb::params![r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8]).map_err(|e| e.to_string())?;
            }
            app.flush().map_err(|e| e.to_string())?;
        }
        c.exec(&format!(
            "INSERT INTO {sch}.location SELECT loc_id, link_id, ref_node_id, lr, x_coord, y_coord,
             NULL::DOUBLE, loc_type, 1::BIGINT, gtfs_stop_id, NULL::BIGINT, name, ST_Point(x_coord, y_coord)
             FROM _gtfs_loc;
             DROP TABLE _gtfs_loc;
             DELETE FROM {sch}.location o WHERE o.gtfs_stop_id IS NULL AND o.loc_type = 'bus_stop' AND EXISTS (
             SELECT 1 FROM {sch}.location g WHERE g.gtfs_stop_id IS NOT NULL AND g.loc_type = 'bus_stop'
               AND sqrt(pow((o.x_coord - g.x_coord) * cos(radians(o.y_coord)) * 111320, 2)
                        + pow((o.y_coord - g.y_coord) * 111320, 2)) < 25)"
        ))?;
    }
    let far_txt = if far > 0 { format!(", {far} farther than {max_m} m from any") } else { String::new() };
    eprintln!("GTFS[{mode}]: {} of {} stops on a link{far_txt}", rows.len(), todo.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(kind(3), "bus_stop");
        assert_eq!(kind(700), "bus_stop");
        assert_eq!(kind(2), "train_station");
        assert_eq!(kind(109), "train_station");
        assert_eq!(kind(8), "transit_stop");
        assert_eq!(kind(1500), "transit_stop");
    }
}
