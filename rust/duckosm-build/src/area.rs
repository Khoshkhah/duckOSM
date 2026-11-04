//! The tables of the whole area, after the modes (docs/tables.md, group 1): `main.global_junctions`,
//! `main.boundary`, `main.visualization_metadata` and `mm.transfers`, as duckOSM writes them. The files are
//! `<table>_<schema>.parquet`, so a reader of `<table>_<mode>.parquet` takes them without a change.

use crate::levels::{self, LevelEdges, Method};
use duckosm_core::filter::Mode;
use duckosm_core::fixes::OsmFixes;
use duckosm_core::geom::Pt;
use duckosm_core::lanes::{self, LaneEdges};
use duckosm_core::model::{Node, Way};
use duckosm_core::network::{self, Coords, Options};
use duckosm_parquet::{self as pq, Metadata, Prop, Transfer};
use serde_json::Value;
use std::io::Write;

/// What the area tables are made from.
pub struct Area<'a> {
    /// The boundary file (GeoJSON text), if the area has one.
    pub boundary: Option<&'a str>,
    /// The ways as read (after the fixes): the global junctions.
    pub ways: &'a [Way],
    /// The coordinates of the nodes: the box of the area when there is no boundary.
    pub coords: &'a Coords,
    /// The nodes of each mode that was built: the transfers and the time zone.
    pub nodes: &'a [(Mode, Vec<Node>)],
    pub opts: &'a Options,
    /// The build time, microseconds since 1970 (UTC). The caller gives it: WebAssembly has no clock in Rust.
    pub built_at: i64,
    /// The entry of the area's Geofabrik region in the time zone table (`www/timezones.json`, JSON text): the
    /// page gives it, because the browser build has no world map of the zones. None: the map of `tzf-rs`
    /// (the cargo feature `tzf`, natively), else no time zone.
    pub timezone_entry: Option<&'a str>,
}

/// duckOSM `multimodal.transfer_s`: the seconds of a change of mode.
const TRANSFER_S: f64 = 60.0;

pub fn write_area_tables<W: Write + Send>(
    a: &Area,
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<(), String> {
    if a.opts.global_junctions {
        let mut gj: Vec<(i64, bool)> = network::global_junctions(a.ways).into_iter().collect();
        gj.sort_unstable();
        pq::write_global_junctions_to(file("global_junctions_main.parquet")?, &gj)?;
    }
    let shape = match a.boundary {
        Some(text) => {
            let (props, polys, multi, json) = boundary(text)?;
            pq::write_boundary_to(
                file("boundary_main.parquet")?,
                &props,
                &pq::polygons_wkb(&polys, multi),
                multi,
            )?;
            Some((polys, json))
        }
        None => None,
    };
    pq::write_metadata_to(
        file("visualization_metadata_main.parquet")?,
        &metadata(a, shape.as_ref()),
    )?;
    // duckOSM's multimodal step stops before it writes mm.* when edges.cost_s is missing, and the build goes on
    if a.opts.calculate_costs {
        pq::write_transfers_to(file("transfers_mm.parquet")?, &transfers(a.nodes))?;
    }
    Ok(())
}

/// The first feature of a boundary file: its properties, typed as GDAL types them for DuckDB's `ST_Read`, its
/// polygons, whether it is a MultiPolygon, and its geometry as GeoJSON text.
#[allow(clippy::type_complexity)]
fn boundary(text: &str) -> Result<(Vec<(String, Prop)>, Vec<Vec<Vec<Pt>>>, bool, String), String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("boundary: {e}"))?;
    let feature = match doc["type"].as_str() {
        Some("FeatureCollection") => doc["features"]
            .get(0)
            .cloned()
            .ok_or("boundary: no feature")?,
        Some("Feature") => doc,
        _ => serde_json::json!({ "type": "Feature", "properties": {}, "geometry": doc }),
    };
    let props = feature["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            let p = match v {
                Value::Number(n) if n.is_i64() => match i32::try_from(n.as_i64().unwrap()) {
                    Ok(i) => Prop::Int(i),
                    Err(_) => Prop::BigInt(n.as_i64().unwrap()),
                },
                Value::Number(n) => Prop::Double(n.as_f64().unwrap_or(f64::NAN)),
                Value::Bool(b) => Prop::Bool(*b),
                Value::String(s) => Prop::Text(Some(s.clone())),
                Value::Null => Prop::Text(None),
                other => Prop::Text(Some(other.to_string())),
            };
            (k.clone(), p)
        })
        .collect();
    let g = &feature["geometry"];
    let ring = |r: &Value| -> Vec<Pt> {
        r.as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| Some((c[0].as_f64()?, c[1].as_f64()?)))
            .collect()
    };
    let polygon =
        |p: &Value| -> Vec<Vec<Pt>> { p.as_array().into_iter().flatten().map(ring).collect() };
    let (polys, multi) = match g["type"].as_str() {
        Some("Polygon") => (vec![polygon(&g["coordinates"])], false),
        Some("MultiPolygon") => (
            g["coordinates"]
                .as_array()
                .into_iter()
                .flatten()
                .map(polygon)
                .collect(),
            true,
        ),
        _ => return Err("boundary: the geometry is not a Polygon or a MultiPolygon".into()),
    };
    Ok((props, polys, multi, g.to_string()))
}

/// `main.visualization_metadata` (duckOSM importer.py): the center and the zoom of the box of the boundary
/// (else of all nodes), and the time zone at the node nearest the median of the nodes of a mode.
fn metadata(a: &Area, shape: Option<&(Vec<Vec<Vec<Pt>>>, String)>) -> Metadata {
    let pts: Vec<Pt> = match shape {
        Some((polys, _)) => polys.iter().flatten().flatten().copied().collect(),
        None => a.coords.iter().map(|(_, p)| *p).collect(),
    };
    let (mut w, mut s, mut e, mut n) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &pts {
        (w, s, e, n) = (w.min(p.0), s.min(p.1), e.max(p.0), n.max(p.1));
    }
    let width = e - w;
    // DuckDB: CAST(LEAST(14, GREATEST(1, LOG2(360 / width))) AS INTEGER), which rounds
    let initial_zoom = if width < 0.0001 {
        14
    } else {
        (360.0 / width).log2().clamp(1.0, 14.0).round() as i32
    };
    let boundary_geojson = match shape {
        Some((_, json)) => json.clone(),
        None => format!("{{\"type\":\"Polygon\",\"coordinates\":[[[{w},{s}],[{e},{s}],[{e},{n}],[{w},{n}],[{w},{s}]]]}}"),
    };
    Metadata {
        boundary_geojson,
        center_lat: (s + n) / 2.0,
        center_lon: (w + e) / 2.0,
        initial_zoom,
        timezone: lookup_point(a.nodes).and_then(|(x, y)| match a.timezone_entry {
            Some(entry) => zone_of_entry(entry, x, y),
            None => zone_of_map(x, y),
        }),
        built_at: a.built_at,
        version: format!("duckosm-rust {}", env!("CARGO_PKG_VERSION")),
    }
}

/// duckOSM `utils.add_timezone`: the point of the time zone is the node nearest (median x, median y) of the
/// nodes of driving (else of the first mode), without the virtual nodes.
fn lookup_point(nodes: &[(Mode, Vec<Node>)]) -> Option<(f64, f64)> {
    let (_, ns) = nodes
        .iter()
        .find(|(m, _)| *m == Mode::Driving)
        .or(nodes.first())?;
    let ns: Vec<&Node> = ns.iter().filter(|n| n.node_id > 0).collect();
    let median = |mut v: Vec<f64>| -> f64 {
        v.sort_by(f64::total_cmp);
        let m = v.len() / 2;
        if v.len() % 2 == 1 {
            v[m]
        } else {
            (v[m - 1] + v[m]) / 2.0
        }
    };
    if ns.is_empty() {
        return None;
    }
    let (mx, my) = (
        median(ns.iter().map(|n| n.lon).collect()),
        median(ns.iter().map(|n| n.lat).collect()),
    );
    let near = ns.iter().min_by(|a, b| {
        let d = |n: &Node| (n.lon - mx).powi(2) + (n.lat - my).powi(2);
        d(a).total_cmp(&d(b))
    })?;
    Some((near.lon, near.lat))
}

/// The zone at a point from the world map of `tzf-rs` (the native build).
#[cfg(feature = "tzf")]
fn zone_of_map(x: f64, y: f64) -> Option<String> {
    let tz = tzf_rs::DefaultFinder::new().get_tz_name(x, y).to_string();
    (!tz.is_empty()).then_some(tz)
}

#[cfg(not(feature = "tzf"))]
fn zone_of_map(_: f64, _: f64) -> Option<String> {
    None
}

/// The zone at a point from an entry of the time zone table (tools/make_timezone_table.py): a name; null; or
/// `{"zones": [[name, MultiPolygon coordinates], ...]}`, the smaller zones first: the first zone whose part has
/// the point (the even-odd rule over the rings of a polygon), else the last.
fn zone_of_entry(entry: &str, x: f64, y: f64) -> Option<String> {
    let v: Value = serde_json::from_str(entry).ok()?;
    if let Some(name) = v.as_str() {
        return Some(name.to_string());
    }
    let zones = v["zones"].as_array()?;
    let pt = |c: &Value| Some((c[0].as_f64()?, c[1].as_f64()?));
    let in_polygon = |rings: &Value| -> bool {
        let mut inside = false;
        for ring in rings.as_array().into_iter().flatten() {
            let r: Vec<(f64, f64)> = ring
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(pt)
                .collect();
            for i in 0..r.len() {
                let (a, b) = (r[i], r[(i + r.len() - 1) % r.len()]);
                if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
                    inside = !inside;
                }
            }
        }
        inside
    };
    let found = zones
        .iter()
        .find(|z| z[1].as_array().into_iter().flatten().any(in_polygon))
        .or(zones.last())?;
    found[0].as_str().map(str::to_string)
}

/// `mm.transfers` (duckOSM `MultimodalBuilder`, the coarse version): at each node that walking shares with
/// driving or cycling, a change to that mode and a change back, each for `TRANSFER_S` seconds.
fn transfers(nodes: &[(Mode, Vec<Node>)]) -> Vec<Transfer> {
    let ids = |m: Mode| -> Option<Vec<i64>> {
        nodes.iter().find(|(x, _)| *x == m).map(|(_, ns)| {
            let mut v: Vec<i64> = ns.iter().map(|n| n.node_id).collect();
            v.sort_unstable();
            v.dedup();
            v
        })
    };
    let Some(walk) = ids(Mode::Walking) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (m, name, (to, back)) in [
        (Mode::Cycling, "cycling", ("bike_park", "bike_unpark")),
        (Mode::Driving, "driving", ("park", "retrieve")),
    ] {
        let Some(other) = ids(m) else { continue };
        for &n in walk.iter().filter(|n| other.binary_search(n).is_ok()) {
            out.push(Transfer {
                node_id: n,
                from_mode: "walking",
                to_mode: name,
                cost_s: TRANSFER_S,
                kind: to,
            });
            out.push(Transfer {
                node_id: n,
                from_mode: name,
                to_mode: "walking",
                cost_s: TRANSFER_S,
                kind: back,
            });
        }
    }
    out
}

/// `visualization.edge_levels` and `edge_levels_meta` (docs/levels.md) from the edges of all modes, with the method
/// of the options. `built_at` (microseconds since 1970, UTC) is the `created` time. Returns a summary line, or None
/// when the method is `none`.
pub fn write_levels<W: Write + Send>(
    edges: &LevelEdges,
    method: Method,
    built_at: i64,
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<Option<String>, String> {
    let Some(lv) = levels::compute(edges, method)? else {
        return Ok(None);
    };
    pq::write_edge_levels_to(file("edge_levels_visualization.parquet")?, &lv.rows)?;
    let meta = pq::LevelsMeta {
        source: method.name(),
        n_edges: lv.rows.len() as i64,
        edge_hash: lv.edge_hash.clone(),
        roadstyle_version: levels::ROADSTYLE_VERSION,
        created: iso_time(built_at),
    };
    pq::write_edge_levels_meta_to(file("edge_levels_meta_visualization.parquet")?, &meta)?;
    let mut line = format!(
        "levels ({}): {} edges, {} roads",
        method.name(),
        lv.rows.len(),
        lv.roads
    );
    if method == Method::Solve {
        line += &format!(
            ", {} stack pairs given up, {} order wishes not kept (best values of the stages: {:?})",
            lv.given_up, lv.orders_not_kept, lv.stages
        );
    }
    Ok(Some(line))
}

/// `driving.lane_profile` (docs/lane_profile.md), when the driving mode was built: `lane_profile_driving.parquet`.
/// `ways` are the ways after all modes. Returns a summary line.
pub fn write_lane_profile<W: Write + Send>(
    edges: &LaneEdges,
    ways: &[Way],
    fixes: &OsmFixes,
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<Option<String>, String> {
    let Some(rows) = lanes::lane_profile(edges, ways, fixes) else {
        return Ok(None);
    };
    pq::write_lane_profile_to(file("lane_profile_driving.parquet")?, &rows)?;
    Ok(Some(format!("lane profile: {} lanes", rows.len())))
}

/// Microseconds since 1970 as roadstyle writes a time: `2026-10-22T00:20:45+00:00`.
pub(crate) fn iso_time(micros: i64) -> String {
    let secs = micros.div_euclid(1_000_000);
    let (days, s) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // the civil date of a day number (Howard Hinnant's days_from_civil, inverted)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_time() {
        assert_eq!(super::iso_time(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(
            super::iso_time(1_792_628_445_000_000),
            "2026-10-22T00:20:45+00:00"
        );
    }
}
