//! `duckosm gmns-map`: the lane-level page of a GMNS file (duckOSM's gmns_map.py, which called the Python lanestyle). The roads of the
//! level area as roadstyle draws them, each as wide as its lanes, with the lanes, lane lines, arrows, zebras and sidewalks on them:
//! lanestyle's Rust engine (`lanestyle::page::lane_page`).

use std::path::{Path, PathBuf};

/// The level area of `source_db` when none is given: `<source_db>.levels`, made by `duckosm levels`.
pub fn default_area(source_db: &Path) -> PathBuf {
    source_db.with_extension("levels")
}

/// Write the lane page of `gmns_db` to `out`: the lanes on the roads of the level `area` (default: `<source_db>.levels`) of `source_db`,
/// the core database the GMNS file was made from. Gives the path written.
pub fn write_map(gmns_db: &Path, out: &Path, source_db: &Path, area: Option<&Path>) -> Result<PathBuf, String> {
    let area = area.map_or_else(|| default_area(source_db), Path::to_path_buf);
    if !area.is_dir() {
        return Err(format!("no level area {}: make it with `duckosm levels {}`", area.display(), source_db.display()));
    }
    let text = |p: &Path| p.to_str().map(String::from).ok_or_else(|| format!("{}: not a UTF-8 path", p.display()));
    let (html, notes) = lanestyle::page::lane_page(&area, &text(gmns_db)?, &text(source_db)?, false, serde_json::Map::new())?;
    for n in notes {
        eprintln!("{n}");
    }
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(out, html).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(out.to_path_buf())
}
