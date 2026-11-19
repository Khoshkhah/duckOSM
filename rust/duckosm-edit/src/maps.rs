//! `duckosm viz`: the maps of a built database (duckOSM's viz.py, which drew them with mapstyle's render_map), here with roadstyle's
//! Rust engine: roadstyle's `duckosm.page` of the database (`page::prepare_map` reads it, roadstyle's page writer writes it). One page with
//! every mode (`<name>_map.html`) and one page for each mode with only that mode's edges and private roads (`<name>_<mode>_network.html`),
//! over the database's own base map.

use crate::page::{ground, page_html, prepare_map};
use crate::{rows, Db};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub const MODES: [&str; 3] = ["driving", "walking", "cycling"];

/// The options of `render_maps` (viz.py's keywords).
pub struct Viz {
    /// The modes that get a page of their own (default: every mode in the database).
    pub modes: Option<Vec<String>>,
    /// The start of the file names (default: the database's file name).
    pub name: Option<String>,
    pub out_dir: PathBuf,
    /// The first background (default: the database's own layers, blank).
    pub basemap: Option<String>,
    pub arrows: bool,
    /// The area's boundary (main.boundary) as a dashed line.
    pub boundary: bool,
    /// The page with every mode too.
    pub all_modes: bool,
    /// The Google 3D view (default: on when GOOGLE_MAPS_KEY is set).
    pub google_3d: Option<bool>,
}

impl Viz {
    pub fn new(out_dir: impl Into<PathBuf>) -> Viz {
        Viz { modes: None, name: None, out_dir: out_dir.into(), basemap: None, arrows: true, boundary: true, all_modes: true, google_3d: None }
    }
}

/// The modes of the open database that have `table` (driving, walking, cycling, in that order).
pub fn modes_with(db: &mut dyn Db, table: &str) -> Result<Vec<String>, String> {
    let have: Vec<String> = db.texts(&format!("SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name = '{table}'"))?.into_iter().flatten().collect();
    Ok(MODES.iter().filter(|m| have.iter().any(|h| h == *m)).map(|m| m.to_string()).collect())
}

/// The area's boundary as a GeoJSON FeatureCollection, or None when the database has no main.boundary (a whole-country PBF).
fn boundary(db: &mut dyn Db) -> Option<Value> {
    let feats: Vec<Value> = rows(db, "SELECT ST_AsGeoJSON(geom) AS g FROM main.boundary WHERE geom IS NOT NULL").ok()?.into_iter()
        .filter_map(|r| r["g"].as_str().and_then(|t| serde_json::from_str::<Value>(t).ok()))
        .map(|g| json!({"type": "Feature", "properties": {}, "geometry": g})).collect();
    (!feats.is_empty()).then(|| json!({"type": "FeatureCollection", "features": feats}))
}

/// The page keys of the environment: the Google Maps key (GOOGLE_MAPS_KEY) for Street View and the Google 3D view.
pub(crate) fn env_keys() -> Map<String, Value> {
    let mut m = Map::new();
    if let Ok(k) = std::env::var("GOOGLE_MAPS_KEY") {
        if !k.is_empty() {
            m.insert("street_view_key".into(), json!(k));
        }
    }
    m
}

/// The page of the map of `mode` (with `only`: that mode's roads alone) of the open database, roadstyle's `duckosm.page(db, mode, only=...)`.
/// `args` over the defaults (the dashboard with no panel).
pub fn map_page(db: &mut dyn Db, place: &str, mode: &str, only: bool, mut args: Value) -> Result<String, String> {
    let base = json!({"mode": mode, "theme": "liberty", "paths": "google", "place": place});
    // duckosm.page(dashboard=False): the dashboard without a panel, the map and the toolbox
    for (k, v) in [("template", json!("dashboard")), ("dashboard", json!({"panel": null})), ("state", json!("live"))] {
        if args.get(k).is_none() {
            args[k] = v;
        }
    }
    // the stored levels when the file has them, else the engine solves them (render_map's rule)
    let p = prepare_map(db, mode, only.then_some(mode), base, args, Value::Null, false)?;
    let mut extra = env_keys();
    extra.extend(ground(db, "duckosm viz")?);
    page_html(p, extra, "duckosm viz")
}

/// Write the maps of the built database `path` into `v.out_dir`; return the paths written.
pub fn render_maps(db: &mut dyn Db, path: &Path, v: &Viz) -> Result<Vec<PathBuf>, String> {
    db.open()?;
    let present = modes_with(db, "edges")?;
    let mut modes = v.modes.clone().unwrap_or_else(|| present.clone());
    if v.modes.is_none() {
        modes.sort();                       // viz.py: ORDER BY schema_name
    }
    let unknown: Vec<&String> = modes.iter().filter(|m| !present.contains(m)).collect();
    if !unknown.is_empty() {
        return Err(format!("mode(s) {unknown:?} not in {} (present: {present:?})", path.display()));
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let name = v.name.clone().unwrap_or_else(|| stem.clone());
    let mut args = json!({"arrows": v.arrows});
    if v.google_3d.unwrap_or_else(|| !env_keys().is_empty()) {
        args["google_3d"] = json!(true);
    }
    if let Some(b) = &v.basemap {
        args["basemap"] = json!(b);
    }
    if v.boundary {
        if let Some(fc) = boundary(db) {
            args["boundary"] = fc;
        }
    }
    std::fs::create_dir_all(&v.out_dir).map_err(|e| format!("{}: {e}", v.out_dir.display()))?;
    let mut pages: Vec<(String, PathBuf)> = Vec::new();
    if v.all_modes {
        pages.push(("all".into(), v.out_dir.join(format!("{name}_map.html"))));
    }
    pages.extend(modes.iter().map(|m| (m.clone(), v.out_dir.join(format!("{name}_{m}_network.html")))));
    for (mode, out) in &pages {
        let mut a = args.clone();
        a["name"] = json!(format!("{stem} ({mode}, google)"));
        let html = map_page(db, &stem, mode, mode != "all", a)?;
        std::fs::write(out, html).map_err(|e| format!("{}: {e}", out.display()))?;
        eprintln!("  Map: {}", out.display());
    }
    db.close();
    Ok(pages.into_iter().map(|(_, p)| p).collect())
}
