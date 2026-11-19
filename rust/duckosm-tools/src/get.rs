//! `duckosm get`: the OSM file of an area in one command (docs/exports/osm-file.md). The boundary (a name, an OSM id,
//! a box, an H3 cell or a GeoJSON file), the smallest Geofabrik extract that covers it (downloaded to a cache folder),
//! and the cut of the extract to the boundary, with the OSM fixes when `--fixes` is given.

use crate::Result;
use clap::Parser;
use duckosm_build::place::{bbox_geojson, find_boundary, h3_geojson, write_boundary, Wanted};
use serde_json::json;
use std::path::PathBuf;

/// duckosm get: choose, download and cut the OSM data of an area. Writes <out>/<name>.osm.pbf and
/// <out>/<name>.geojson; then `duckosm build --pbf <out>/<name>.osm.pbf --boundary <out>/<name>.geojson`.
#[derive(Parser)]
#[command(name = "duckosm get", version)]
struct GetCli {
    /// The name of the area, for OpenStreetMap's Nominatim search (for example "Monaco" or "Tartu, Estonia").
    place: Option<String>,
    /// One OSM boundary relation, by its id (for a name with more than one match).
    #[arg(long)]
    osm_id: Option<i64>,
    /// A box: west,south,east,north in degrees.
    #[arg(long, value_name = "W,S,E,N", allow_hyphen_values = true)]
    bbox: Option<String>,
    /// An H3 cell (hex text).
    #[arg(long)]
    h3_cell: Option<String>,
    /// A boundary file (GeoJSON, docs/reference/boundary.md).
    #[arg(long)]
    boundary: Option<PathBuf>,
    /// The output folder.
    #[arg(short, long, default_value = ".")]
    out: PathBuf,
    /// The name of the files (default: from the place, the boundary file or the H3 cell; "area" for a box).
    #[arg(long)]
    name: Option<String>,
    /// The OSM fixes file of `duckosm build --fixes` (YAML): the cut PBF has the fixes in it.
    #[arg(long, value_name = "FILE")]
    fixes: Option<PathBuf>,
    /// The folder of the downloads (default: $DUCKOSM_CACHE, else ~/.cache/duckosm).
    #[arg(long)]
    cache: Option<PathBuf>,
    /// Download the extract and the list of extracts again, also when they are in the cache.
    #[arg(long)]
    refresh: bool,
    /// The list of extracts (an address or a file, as Geofabrik's index-v1.json).
    #[arg(long, hide = true)]
    index: Option<String>,
}

/// A file name: letters, digits, `_` and `-`, lower case (`duckosm boundary` names its file the same way).
fn slug(text: &str) -> String {
    let s: String = text.split(',').next().unwrap_or_default().trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
    if s.is_empty() { "area".into() } else { s }
}

pub fn run(args: &[String]) -> Result<()> {
    let cli = GetCli::parse_from(args);
    let given = [cli.place.is_some() || cli.osm_id.is_some(), cli.bbox.is_some(), cli.h3_cell.is_some(), cli.boundary.is_some()];
    if given.iter().filter(|&&g| g).count() != 1 {
        return Err("give one area: a NAME (or --osm-id), --bbox, --h3-cell or --boundary".into());
    }
    let t = std::time::Instant::now();
    // 1. the boundary
    let (geom, info, name) = if let Some(file) = &cli.boundary {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        if !text.trim_start().starts_with('{') {
            return Err(format!("{}: --boundary needs a GeoJSON file (docs/reference/boundary.md); duckosm cut also reads .poly, WKT and W,S,E,N", file.display()));
        }
        let stem = file.file_name().and_then(|n| n.to_str()).unwrap_or("area").split('.').next().unwrap_or("area").to_string();
        (None, text, stem)
    } else if let Some(b) = &cli.bbox {
        let v: Vec<f64> = b.split(',').map(|x| x.trim().parse::<f64>().map_err(|_| format!("--bbox {b}: not W,S,E,N"))).collect::<Result<_>>()?;
        (Some(bbox_geojson(&v)?), json!({"name": "area"}).to_string(), "area".to_string())
    } else if let Some(c) = &cli.h3_cell {
        (Some(h3_geojson(c)?), json!({"name": c}).to_string(), c.clone())
    } else {
        let want = match (&cli.place, cli.osm_id) {
            (_, Some(id)) => Wanted::OsmId(id),
            (Some(n), None) => Wanted::Name(n),
            (None, None) => unreachable!(),
        };
        let (g, info) = find_boundary(want, None, false)?;
        let name = cli.place.clone().unwrap_or_else(|| info["name"].as_str().unwrap_or("area").to_string());
        eprintln!("  boundary: {}, OSM relation {}, {} km², from {}", info["name"].as_str().unwrap_or_default(), info["osm_id"], info["area_km2"], info["source"].as_str().unwrap_or_default());
        (Some(g), info.to_string(), name)
    };
    let name = slug(cli.name.as_deref().unwrap_or(&name));
    std::fs::create_dir_all(&cli.out).map_err(|e| format!("{}: {e}", cli.out.display()))?;
    let boundary = cli.out.join(format!("{name}.geojson"));
    match geom {
        Some(g) => write_boundary(g, serde_json::from_str(&info).map_err(|e| e.to_string())?, &boundary)?,
        // a boundary file is copied as it is, unless it is the output file
        None if std::path::absolute(&boundary).ok() != cli.boundary.as_ref().and_then(|b| std::path::absolute(b).ok()) => {
            std::fs::write(&boundary, &info).map_err(|e| format!("{}: {e}", boundary.display()))?
        }
        None => {}
    }
    let text = std::fs::read_to_string(&boundary).map_err(|e| format!("{}: {e}", boundary.display()))?;
    let area = duckosm_build::get::check_boundary(&text)?;
    // 2. and 3. the extract, from the cache or downloaded
    let cache = cli.cache.clone().unwrap_or_else(duckosm_build::get::cache_dir);
    let (region, _) = duckosm_build::get::region_file(&area, &cache, cli.index.as_deref(), cli.refresh)?;
    // 4. the cut, with the fixes
    let pbf = cli.out.join(format!("{name}.osm.pbf"));
    let n = duckosm_build::get::cut_with_fixes(&region, &text, &pbf, cli.fixes.as_deref())?;
    if n.nodes == 0 {
        eprintln!("warning: the cut has no nodes: is the boundary in longitude, latitude order (docs/reference/boundary.md)?");
    }
    eprintln!(
        "  cut{}: {} nodes, {} ways, {} relations",
        if cli.fixes.is_some() { " with the fixes" } else { "" },
        n.nodes,
        n.ways,
        n.relations
    );
    println!("wrote {} and {} in {:.1} s", pbf.display(), boundary.display(), t.elapsed().as_secs_f64());
    println!("next: duckosm build --pbf {} --boundary {}", pbf.display(), boundary.display());
    Ok(())
}
