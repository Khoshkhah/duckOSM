//! The DEM sources that duckOSM knows, and the ones that cover an area (duckOSM `dem_sources.py`). The list is
//! `src/duckosm/data/dem_sources.json`, the same file that the Python package and the page "Ground" of Studio read.
//! `render_tables` of the Python module (the tables of docs/reference/elevation-sources.md) stays in Python.

use serde_json::Value;
use std::sync::OnceLock;

const CATALOG: &str = include_str!("../../../src/duckosm/data/dem_sources.json");

/// All the entries of `dem_sources.json`, in the order of the file.
pub fn load() -> &'static [Value] {
    static SOURCES: OnceLock<Vec<Value>> = OnceLock::new();
    SOURCES.get_or_init(|| {
        let doc: Value = serde_json::from_str(CATALOG).expect("dem_sources.json is valid JSON");
        doc["sources"].as_array().cloned().unwrap_or_default()
    })
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn in_ring(lon: f64, lat: f64, ring: &[Value]) -> bool {
    let mut inside = false;
    for (i, a) in ring.iter().enumerate() {
        let b = &ring[(i + 1) % ring.len()];
        let (x1, y1, x2, y2) = (num(&a[0]), num(&a[1]), num(&b[0]), num(&b[1]));
        if (y1 > lat) != (y2 > lat) && lon < (x2 - x1) * (lat - y1) / (y2 - y1) + x1 {
            inside = !inside;
        }
    }
    inside
}

/// True when the whole box `[west, south, east, north]` is inside the coverage of the source and touches none of its
/// `exclude` boxes. A polygon is coarse: the four corners must be inside it.
pub fn covers(source: &Value, bbox: [f64; 4]) -> bool {
    let [w, s, e, n] = bbox;
    let cov = &source["coverage"];
    let ok = match cov["bbox"].as_array() {
        Some(b) => num(&b[0]) <= w && e <= num(&b[2]) && num(&b[1]) <= s && n <= num(&b[3]),
        None => {
            let ring = cov["polygon"].as_array().map_or(&[][..], Vec::as_slice);
            [w, e].iter().all(|&x| [s, n].iter().all(|&y| in_ring(x, y, ring)))
        }
    };
    let touches = |b: &Value| w < num(&b[2]) && num(&b[0]) < e && s < num(&b[3]) && num(&b[1]) < n;
    ok && !cov["exclude"].as_array().is_some_and(|x| x.iter().any(touches))
}

/// The sources that cover the box, best first: the order is `rank`, then the resolution, then the order of the file. A
/// source with `suggest: false` is left out unless `include_all`; the fallback (AWS Terrain Tiles) unless `include_fallback`.
pub fn sources_for_bbox(bbox: [f64; 4], include_fallback: bool, include_all: bool) -> Vec<&'static Value> {
    let mut keep: Vec<&Value> = load()
        .iter()
        .filter(|s| covers(s, bbox))
        .filter(|s| include_all || s["suggest"].as_bool().unwrap_or(true))
        .filter(|s| include_fallback || !s["fallback"].as_bool().unwrap_or(false))
        .collect();
    // a stable sort keeps the order of the file on a tie
    keep.sort_by(|a, b| num(&a["rank"]).total_cmp(&num(&b["rank"])).then(num(&a["resolution_m"]).total_cmp(&num(&b["resolution_m"]))));
    keep
}

/// The entry with this `id`.
pub fn get(id: &str) -> Option<&'static Value> {
    load().iter().find(|s| s["id"] == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values of tests/test_dem_sources.py.
    #[test]
    fn as_python() {
        let ids = |b, f, a| sources_for_bbox(b, f, a).iter().map(|s| s["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(ids([7.41, 43.73, 7.44, 43.75], false, false)[..3], ["eudtm", "fabdem", "copernicus-glo30"]);
        assert!(ids([7.41, 43.73, 7.44, 43.75], true, false).contains(&"aws-terrain-tiles".to_string()));
        assert_eq!(ids([18.0, 59.3, 18.1, 59.35], false, false)[0], "se-lantmateriet-1m");
        assert_eq!(get("fabdem").unwrap()["commercial_use"], false);
        assert!(get("nope").is_none());
    }
}
