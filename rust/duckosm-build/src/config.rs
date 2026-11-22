//! Read a duckOSM config file (YAML) into the parameters of a build.
//!
//! The keys and their defaults are those of duckOSM (`src/duckosm/config.py`). The rules:
//! - A key that duckOSM does not know is an error (duckOSM only warns, but it is almost always a typo).
//! - A key that changes the result and that this program does not support is an error.
//! - Every step that duckOSM runs, this program runs too (the maps of `viz.enabled` with duckosm-edit's maps.rs), so the
//!   list `skipped` stays empty.
//! - A key that only tunes duckOSM's DuckDB (batches) has no effect here. It is in the list `ignored`.
//!   `memory_limit` and `threads` set the DuckDB of the load of `raw.*` (`read_osm`).

use crate::levels::Method;
use crate::Params;
use duckosm_core::filter::Mode;
use duckosm_core::building_faces::{FaceEdit, FaceType};
use duckosm_core::fixes::{Elem, NewNode, NewWay, OsmFixes, TagEdit, TurnFix, WayFix};
use duckosm_core::network::Options;
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

/// A config file read into a build.
#[derive(Clone, Debug)]
pub struct FromConfig {
    pub pbf: PathBuf,
    /// The output folder.
    pub out: PathBuf,
    pub params: Params,
    /// Steps of duckOSM that this program does not run (only with `strict = false`).
    pub skipped: Vec<String>,
    /// Keys that have no effect in this program.
    pub ignored: Vec<String>,
}

const TOP: [&str; 16] = [
    "name",
    "output_path",
    "osm_overrides",
    "modes",
    "options",
    "source",
    "boundary",
    "clip",
    "validation",
    "report",
    "viz",
    "multimodal",
    "pbf_path",
    "boundary_path",
    "h3_cell",
    "elevation",
];
const OPTIONS: [&str; 25] = [
    "engine",
    "build_graph",
    "h3_indexing",
    "h3_resolution",
    "tile_indexing",
    "merge_segments",
    "global_junctions",
    "infer_separate_sidewalks",
    "functional_types",
    "cycling_dismount",
    "process_speeds",
    "extract_restrictions",
    "calculate_costs",
    "memory_limit",
    "threads",
    "simplify_batches",
    "boundary_cells",
    "boundary_cell_resolutions",
    "chains",
    "build_features",
    "levels",
    "bus_routes",
    "building_faces",
    "sea",
    "clip_strategy",
];
const SOURCE: [&str; 3] = ["type", "pbf_path", "source_db"];
const BOUNDARY: [&str; 5] = ["path", "place", "bbox", "h3_cell", "buffer_m"];
const CLIP: [&str; 5] = [
    "predicate",
    "keep_largest_component",
    "min_component_edges",
    "connectivity_rescue",
    "connect_snap_m",
];
/// Removed checks: every road stays (there is no component filter), so a network can have separate parts. A config that
/// has them gets a warning.
const VALIDATION_REMOVED: [&str; 2] = ["assert_single_component", "assert_no_stranded_named"];
const VALIDATION: [&str; 7] = [
    "enabled",
    "fail_on_error",
    VALIDATION_REMOVED[0],
    VALIDATION_REMOVED[1],
    "assert_unique_node_id",
    "assert_way_length_conserved",
    "warn_layer_without_structure",
];
const REPORT: [&str; 1] = ["enabled"];
const VIZ: [&str; 2] = ["enabled", "basemap"];
const MULTIMODAL: [&str; 5] = ["enabled", "transfer_s", "transfer_costs", "node_costs", "only_listed_nodes"];
/// The area's DEM, its ground reference (docs/concepts/ground-reference.md). The command line `--dem` wins.
const ELEVATION: [&str; 5] = ["dem", "product", "vertical_datum", "license", "reference"];

struct Section<'a> {
    name: &'a str,
    map: Option<&'a yaml_rust2::yaml::Hash>,
}

impl<'a> Section<'a> {
    fn get(&self, key: &str) -> Option<&'a Yaml> {
        self.map
            .and_then(|m| m.get(&Yaml::String(key.to_string())))
            .filter(|v| !v.is_null())
    }
    fn bool(&self, key: &str, default: bool) -> Result<bool, String> {
        match self.get(key) {
            None => Ok(default),
            Some(Yaml::Boolean(b)) => Ok(*b),
            Some(v) => Err(format!(
                "{}.{key}: must be true or false, not {v:?}",
                self.name
            )),
        }
    }
    fn f64(&self, key: &str, default: f64) -> Result<f64, String> {
        match self.get(key) {
            None => Ok(default),
            Some(Yaml::Integer(i)) => Ok(*i as f64),
            Some(Yaml::Real(r)) => r
                .parse()
                .map_err(|_| format!("{}.{key}: not a number", self.name)),
            Some(v) => Err(format!("{}.{key}: must be a number, not {v:?}", self.name)),
        }
    }
    fn str(&self, key: &str) -> Result<Option<String>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Yaml::String(s)) => Ok(Some(s.clone())),
            Some(v) => Err(format!("{}.{key}: must be a text, not {v:?}", self.name)),
        }
    }
    fn check_keys(&self, known: &[&str]) -> Result<(), String> {
        if let Some(m) = self.map {
            for k in m.keys() {
                let k = k.as_str().unwrap_or("?");
                if !known.contains(&k) {
                    let prefix = if self.name.is_empty() {
                        String::new()
                    } else {
                        format!("{}.", self.name)
                    };
                    return Err(format!("config: unknown key '{prefix}{k}' (duckOSM does not know it either; a typo?)"));
                }
            }
        }
        Ok(())
    }
}

fn section<'a>(root: &'a yaml_rust2::yaml::Hash, name: &'a str) -> Result<Section<'a>, String> {
    match root.get(&Yaml::String(name.to_string())) {
        None | Some(Yaml::Null) => Ok(Section { name, map: None }),
        Some(Yaml::Hash(h)) => Ok(Section { name, map: Some(h) }),
        Some(_) => Err(format!(
            "config: '{name}' must be a section (a map of keys)"
        )),
    }
}

/// The options of `duckosm build` (docs/reference/cli.md) that override a config file: each one that is `Some`. Without a config file
/// they are the whole config.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub pbf: Option<String>,
    pub source_db: Option<String>,
    pub output: Option<String>,
    pub boundary: Option<String>,
    pub h3_cell: Option<String>,
    pub modes: Option<Vec<String>>,
    pub fixes: Option<String>,
    pub graph: Option<bool>,
    pub h3_index: Option<bool>,
    pub h3_resolution: Option<i64>,
    pub features: Option<bool>,
    pub levels: Option<bool>,
}

/// The name of an area from a file: `maps/monaco-latest.osm.pbf` is `monaco-latest` (also without .geojson, .json and .duckdb).
fn stem(path: &str) -> String {
    let mut name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    for suffix in [".pbf", ".osm", ".geojson", ".json", ".duckdb"] {
        if let Some(n) = name.strip_suffix(suffix) {
            name = n.to_string();
        }
    }
    name
}

/// The config of `duckosm build`: the text of the config file `base` (or a new config) with the options of `o` in it, as YAML text for
/// [`read_text`]. A new config is named after the output file, else the boundary, the PBF or the parent database, and its output is
/// `<name>.duckdb` in the current folder.
pub fn with_overrides(base: Option<&Path>, o: &Overrides) -> Result<String, String> {
    let mut root = match base {
        Some(path) => {
            let text = duckosm_core::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            match YamlLoader::load_from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?.into_iter().next() {
                None | Some(Yaml::Null) => yaml_rust2::yaml::Hash::new(),
                Some(Yaml::Hash(h)) => h,
                Some(_) => return Err("config: the file must be a map of keys".into()),
            }
        }
        None => {
            let from = o.output.as_deref().or(o.boundary.as_deref()).or(o.pbf.as_deref()).or(o.source_db.as_deref());
            let name = stem(from.ok_or("provide --config, or --pbf, or --source-db")?);
            let mut h = yaml_rust2::yaml::Hash::new();
            h.insert(Yaml::String("name".into()), Yaml::String(name.clone()));
            h.insert(Yaml::String("output_path".into()), Yaml::String(format!("{name}.duckdb")));
            h
        }
    };
    let text = |v: &str| Yaml::String(v.into());
    fn set(root: &mut yaml_rust2::yaml::Hash, section: Option<&str>, k: &str, v: Yaml) {
        let map = match section {
            None => root,
            Some(sec) => {
                let e = root.entry(Yaml::String(sec.into())).or_insert_with(|| Yaml::Hash(Default::default()));
                if !matches!(e, Yaml::Hash(_)) {
                    *e = Yaml::Hash(Default::default());
                }
                match e {
                    Yaml::Hash(h) => h,
                    _ => unreachable!(),
                }
            }
        };
        map.insert(Yaml::String(k.into()), v);
    }
    if let Some(p) = &o.pbf {
        set(&mut root, Some("source"), "type", text("pbf"));
        set(&mut root, Some("source"), "pbf_path", text(p));
    }
    if let Some(p) = &o.source_db {
        set(&mut root, Some("source"), "type", text("duckdb"));
        set(&mut root, Some("source"), "source_db", text(p));
    }
    if let Some(p) = &o.output {
        set(&mut root, None, "output_path", text(p));
    }
    if let Some(p) = &o.boundary {
        set(&mut root, Some("boundary"), "path", text(p));
    }
    if let Some(c) = &o.h3_cell {
        set(&mut root, Some("boundary"), "h3_cell", text(c));
    }
    if let Some(m) = &o.modes {
        set(&mut root, None, "modes", Yaml::Array(m.iter().map(|m| text(m)).collect()));
    }
    if let Some(f) = &o.fixes {
        set(&mut root, None, "osm_overrides", text(f));
    }
    for (k, v) in [("build_graph", o.graph), ("h3_indexing", o.h3_index), ("build_features", o.features), ("levels", o.levels)] {
        if let Some(v) = v {
            set(&mut root, Some("options"), k, Yaml::Boolean(v));
        }
    }
    if let Some(r) = o.h3_resolution {
        set(&mut root, Some("options"), "h3_resolution", Yaml::Integer(r));
    }
    let mut out = String::new();
    YamlEmitter::new(&mut out).dump(&Yaml::Hash(root)).map_err(|e| e.to_string())?;
    Ok(out)
}

/// [`read`] of a config text (`label` names it in the messages).
pub fn read_text(text: &str, label: &str, pbf_is_cut: bool) -> Result<FromConfig, String> {
    let file = std::env::temp_dir().join(format!("duckosm-config-{}-{label}.yaml", std::process::id()));
    std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    let r = read(&file, pbf_is_cut, false);
    let _ = std::fs::remove_file(&file);
    r
}

/// Read the config file. `pbf_is_cut`: the PBF is already cut to the boundary of the config (this
/// program does not cut). `_strict`: no effect now (every step of duckOSM runs here); it stays for the callers.
pub fn read(path: &Path, pbf_is_cut: bool, _strict: bool) -> Result<FromConfig, String> {
    let text = duckosm_core::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let docs = YamlLoader::load_from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let empty = yaml_rust2::yaml::Hash::new();
    let root = match docs.first() {
        None | Some(Yaml::Null) => &empty,
        Some(Yaml::Hash(h)) => h,
        Some(_) => return Err("config: the file must be a map of keys".into()),
    };
    let top = Section {
        name: "",
        map: Some(root),
    };
    top.check_keys(&TOP)?;
    let (options, source, boundary, clip) = (
        section(root, "options")?,
        section(root, "source")?,
        section(root, "boundary")?,
        section(root, "clip")?,
    );
    let (validation, report, viz, multimodal) = (
        section(root, "validation")?,
        section(root, "report")?,
        section(root, "viz")?,
        section(root, "multimodal")?,
    );
    options.check_keys(&OPTIONS)?;
    source.check_keys(&SOURCE)?;
    boundary.check_keys(&BOUNDARY)?;
    clip.check_keys(&CLIP)?;
    validation.check_keys(&VALIDATION)?;
    for k in VALIDATION_REMOVED.into_iter().filter(|k| validation.get(k).is_some()) {
        eprintln!("warning: config: validation.{k} is removed (a network can have separate parts): it has no effect");
    }
    report.check_keys(&REPORT)?;
    viz.check_keys(&VIZ)?;
    multimodal.check_keys(&MULTIMODAL)?;
    let elevation = section(root, "elevation")?;
    elevation.check_keys(&ELEVATION)?;
    let dem = elevation.str("dem")?.map(PathBuf::from);
    if let Some(d) = dem.as_ref().filter(|d| !d.exists()) {
        return Err(format!("DEM file not found: {}", d.display()));
    }

    // every step of duckOSM's config runs in this program: nothing is skipped (the field stays for the callers)
    let skipped: Vec<String> = Vec::new();

    // the input
    let stype = source.str("type")?.unwrap_or_else(|| "pbf".into());
    // source.type: duckdb: a clip of the tables of a parent database (clip.rs); `pbf` is then the parent database
    let clip_src = match stype.as_str() {
        "pbf" => false,
        "duckdb" => true,
        _ => return Err(format!("config: source.type: {stype} is not one of pbf, duckdb")),
    };
    let pbf = if clip_src {
        source.str("source_db")?.filter(|s| !s.is_empty()).ok_or("config: source.source_db is required for source.type: duckdb")?
    } else {
        source
            .str("pbf_path")?
            .or(top.str("pbf_path")?)
            .filter(|s| !s.is_empty())
            .ok_or("config: pbf_path (or source.pbf_path) is required")?
    };
    let pbf = PathBuf::from(pbf);
    if !pbf.exists() {
        return Err(format!("{} not found: {}", if clip_src { "source_db" } else { "PBF file" }, pbf.display()));
    }

    // the area: a boundary file, or a box, an H3 cell or a place name written as <name>.boundary.geojson next to the
    // database (duckOSM Config.materialize_boundary); then grown by buffer_m (place.rs)
    let mut boundary_path = boundary.str("path")?.or(top.str("boundary_path")?);
    let cell = boundary.str("h3_cell")?.or(top.str("h3_cell")?);
    let place = boundary.str("place")?;
    let bbox = match boundary.get("bbox") {
        None => None,
        Some(Yaml::Array(a)) => Some(
            a.iter()
                .map(|v| match v {
                    Yaml::Integer(i) => Ok(*i as f64),
                    Yaml::Real(r) => r.parse::<f64>().map_err(|_| ()),
                    _ => Err(()),
                })
                .collect::<Result<Vec<f64>, ()>>()
                .map_err(|_| format!("boundary.bbox must be [west, south, east, north], got {a:?}"))?,
        ),
        Some(v) => return Err(format!("boundary.bbox must be [west, south, east, north], got {v:?}")),
    };
    if boundary_path.is_none() && (bbox.is_some() || cell.is_some() || place.is_some()) {
        let cfg_name = top.str("name")?.unwrap_or_else(|| "default".into());
        let o = top.str("output_path")?.unwrap_or_else(|| "output.duckdb".into());
        let o = Path::new(&o);
        let dir = if o.extension().is_some_and(|e| e == "duckdb") { o.parent().unwrap_or(Path::new("")) } else { o };
        let (geom, info) = if let Some(b) = &bbox {
            (crate::place::bbox_geojson(b)?, serde_json::json!({"name": cfg_name, "source": "bbox"}))
        } else if let Some(c) = &cell {
            (crate::place::h3_geojson(c)?, serde_json::json!({"name": cfg_name, "source": format!("h3 {c}")}))
        } else {
            crate::place::find(place.as_deref().unwrap_or_default(), (!clip_src).then_some(pbf.as_path()))?
        };
        let file = dir.join(format!("{cfg_name}.boundary.geojson"));
        crate::place::write_boundary(geom, info, &file)?;
        boundary_path = Some(file.display().to_string());
    }
    let buffer_m = boundary.f64("buffer_m", 0.0)?;
    if let (Some(b), true) = (&boundary_path, buffer_m > 0.0) {
        boundary_path = Some(crate::place::buffered(Path::new(b), buffer_m)?.display().to_string());
    }
    let has_boundary = boundary_path.is_some();
    if clip_src && !has_boundary {
        return Err("config: a boundary is required to clip from a duckdb source".into());
    }
    // with a boundary, the program cuts the PBF to it (as osmium complete_ways), unless it is cut already
    let cut_to = boundary_path
        .clone()
        .filter(|_| !pbf_is_cut && !clip_src)
        .map(PathBuf::from);
    if let Some(b) = cut_to.as_ref().filter(|b| !b.exists()) {
        return Err(format!("boundary file not found: {}", b.display()));
    }
    // clip.predicate selects the edges of a clip from a database (source.type: duckdb); a PBF build has no use for it
    let predicate = clip.str("predicate")?.unwrap_or_else(|| "intersects".into());
    if !["intersects", "within", "centroid"].contains(&predicate.as_str()) {
        return Err(format!("clip.predicate: {predicate} is not one of centroid, intersects, within"));
    }
    let mut ignored: Vec<String> = Vec::new();
    if predicate != "intersects" {
        if !clip_src {
            ignored.push("clip.predicate (no effect on a build from a PBF)".to_string());
        }
    }
    let osm_fixes = top.str("osm_overrides")?.map(PathBuf::from);
    if let Some(f) = osm_fixes.as_ref().filter(|f| !f.exists()) {
        return Err(format!("OSM fixes file not found: {}", f.display()));
    }

    // the modes
    let modes: Vec<String> = match top.get("modes") {
        None => vec!["driving".into()],
        Some(Yaml::Array(a)) => a
            .iter()
            .map(|m| {
                m.as_str()
                    .map(str::to_string)
                    .ok_or("config: modes must be a list of names")
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("config: modes must be a list".into()),
    };
    let modes = Mode::parse_all(&modes)?;

    // the steps of duckOSM that are not here (on by default in duckOSM)
    let build_graph = options.bool("build_graph", true)?;
    // main.boundary_cells (place.rs): with a boundary, at options.boundary_cell_resolutions or else at h3_resolution
    let boundary_cells = if options.bool("boundary_cells", false)? && has_boundary && !clip_src {
        let rs: Vec<u8> = match options.get("boundary_cell_resolutions") {
            Some(Yaml::Array(a)) => a
                .iter()
                .map(|v| v.as_i64().filter(|r| (0..=15).contains(r)).map(|r| r as u8))
                .collect::<Option<_>>()
                .ok_or("options.boundary_cell_resolutions: a list of whole numbers from 0 to 15")?,
            None => vec![options.f64("h3_resolution", crate::h3::DEFAULT_RESOLUTION as f64)? as u8],
            Some(v) => return Err(format!("options.boundary_cell_resolutions: {v:?} is not a list")),
        };
        boundary_path.clone().map(|b| (PathBuf::from(b), rs))
    } else {
        None
    };
    // the H3 columns (h3.rs): on by default, at options.h3_resolution (default 8)
    let h3_resolution = if options.bool("h3_indexing", true)? {
        let r = options.f64("h3_resolution", crate::h3::DEFAULT_RESOLUTION as f64)?;
        if !(0.0..=15.0).contains(&r) || r.fract() != 0.0 {
            return Err(format!("config: options.h3_resolution: {r} is not a whole number from 0 to 15"));
        }
        Some(r as u8)
    } else {
        None
    };
    // a clip copies the tables of its parent: no base-map layers, bus routes, walls, H3 columns or DEM of its own (as in duckOSM).
    // Without a boundary the layers run on the raw.* of the whole file (the .duckdb output)
    let features = options.bool("build_features", true)? && !clip_src;
    let sea = match options.get("sea") {
        None => Some("overture".to_string()),
        Some(Yaml::Boolean(false)) => None,
        Some(Yaml::String(s)) if s == "false" => None,
        Some(Yaml::String(s)) => Some(s.clone()),
        Some(v) => return Err(format!("options.sea: {v:?} is not one of overture, false")),
    };
    // the sea (features.ocean) is a layer of features.*: it needs them
    let sea = sea.is_some() && features;
    let viz_on = if viz.bool("enabled", false)? { Some(viz.str("basemap")?) } else { None };
    // the steps on the .duckdb file after the tables (after.rs)
    let validation = if validation.bool("enabled", false)? {
        Some(crate::after::Validation {
            fail_on_error: validation.bool("fail_on_error", true)?,
            unique_node_id: validation.bool("assert_unique_node_id", true)?,
            way_length_conserved: validation.bool("assert_way_length_conserved", true)?,
            layer_without_structure: validation.bool("warn_layer_without_structure", true)?,
        })
    } else {
        None
    };
    let mut transfer_costs = Vec::new();
    match multimodal.get("transfer_costs") {
        None => {}
        Some(Yaml::Hash(h)) => {
            for (k, v) in h {
                let k = k.as_str().ok_or("multimodal.transfer_costs: the keys are texts such as walking->driving")?;
                let v = match v {
                    Yaml::Integer(i) => *i as f64,
                    Yaml::Real(r) => r.parse().map_err(|_| format!("multimodal.transfer_costs.{k}: not a number"))?,
                    _ => return Err(format!("multimodal.transfer_costs.{k}: must be a number")),
                };
                transfer_costs.push((k.to_string(), v));
            }
        }
        Some(_) => return Err("multimodal.transfer_costs: must be a map (walking->driving: 60)".into()),
    }
    let multimodal_on = multimodal.bool("enabled", false)?;
    for k in [
        "simplify_batches",
        "clip_strategy",
        // the network step is always Rust: an old config's `engine` has no effect
        "engine",
    ] {
        if options.get(k).is_some() {
            ignored.push(format!("options.{k} (no effect in this program)"));
        }
    }

    // the component filter is removed (2026-11-14): a road that does not connect to the main network is a real road
    if clip.bool("keep_largest_component", false)? {
        ignored.push("clip.keep_largest_component (the component filter is removed: every road stays)".to_string());
    }
    if clip.f64("min_component_edges", 1.0)? > 1.0 {
        ignored.push("clip.min_component_edges (the component filter is removed: every road stays)".to_string());
    }
    let opts = Options {
        cycling_dismount: options.bool("cycling_dismount", true)?,
        merge_segments: options.bool("merge_segments", true)?,
        global_junctions: options.bool("global_junctions", true)?,
        connect_paths: clip.bool("connectivity_rescue", true)?,
        snap_m: clip.f64("connect_snap_m", 10.0)?,
        connect_near: None,
        process_speeds: options.bool("process_speeds", true)?,
        calculate_costs: options.bool("calculate_costs", true)?,
        functional_types: options.bool("functional_types", true)?,
        build_graph,
        extract_restrictions: options.bool("extract_restrictions", true)?,
        ways_tables: true,
    };
    if opts.calculate_costs && !opts.process_speeds {
        return Err("config: options.calculate_costs needs options.process_speeds".into());
    }

    // the output: like duckOSM, a path that ends in .duckdb is the file, else it is the folder
    let name = top.str("name")?.unwrap_or_else(|| "default".into());
    let out_text = top.str("output_path")?.unwrap_or_else(|| "output.duckdb".into());
    let out_path = PathBuf::from(&out_text);
    let (out, name) = if out_path.extension().is_some_and(|e| e == "duckdb") {
        let stem = out_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or(name);
        (
            out_path.parent().map(Path::to_path_buf).unwrap_or_default(),
            stem,
        )
    } else {
        (out_path, name)
    };
    let out = if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    };
    let params = Params {
        modes,
        name: name.clone(),
        options: opts,
        bus_routes: options.bool("bus_routes", true)? && !clip_src,
        // the walls of the buildings (duckOSM options.building_faces, off by default): main.building_faces, with the wall rules of the fixes file; they need the area's boundary
        building_faces: options.bool("building_faces", false)? && has_boundary && !clip_src,
        // duckOSM: options.levels (on by default) solves the drawing order; off writes no table
        levels: if options.bool("levels", true)? {
            Method::Solve
        } else {
            Method::None
        },
        infer_sidewalks: options.bool("infer_separate_sidewalks", true)?,
        osm_fixes,
        features,
        sea,
        boundary: cut_to,
        // main.boundary and the metadata, also when the PBF is cut already (a missing file is an error there)
        area_boundary: boundary_path.clone().map(PathBuf::from).filter(|b| b.exists()),
        dem: dem.filter(|_| !clip_src),
        clip: clip_src.then(|| predicate.clone()),
        h3_resolution: h3_resolution.filter(|_| !clip_src),
        tile_indexing: options.bool("tile_indexing", true)? && !clip_src,
        dem_product: elevation.str("product")?,
        dem_vertical_datum: elevation.str("vertical_datum")?,
        dem_license: elevation.str("license")?,
        dem_not_reference: !elevation.bool("reference", true)?,
        // DuckDB's settings for the load of raw.* (read_osm)
        memory_limit: options.str("memory_limit")?,
        threads: match options.f64("threads", 0.0)? {
            t if t >= 1.0 && t.fract() == 0.0 => Some(t as usize),
            0.0 => None,
            t => return Err(format!("config: options.threads: {t} is not a whole number of 1 or more")),
        },
        after: crate::after::After {
            validation,
            report: if report.bool("enabled", false)? {
                // the head lines as duckOSM writes them (its paths as the config gives them)
                let db = Path::new(&out_text).join(format!("{name}.duckdb"));
                Some(crate::after::Report {
                    source: format!("{} ({})", if clip_src { "duckdb" } else { "pbf" }, pbf.display()),
                    boundary: boundary_path.clone(),
                    predicate: predicate.clone(),
                    output: if out_text.ends_with(".duckdb") { out_text.clone() } else { db.strip_prefix(".").unwrap_or(&db).display().to_string() },
                })
            } else {
                None
            },
            multimodal: multimodal_on,
            boundary_cells,
            chains: options.bool("chains", true)?,
            transfer_s: multimodal.f64("transfer_s", 60.0)?,
            transfer_costs,
            node_costs: match multimodal.get("node_costs") {
                None | Some(Yaml::Null) => None,
                Some(Yaml::String(p)) => Some(PathBuf::from(p)),
                Some(_) => return Err("multimodal.node_costs: must be the path of a CSV or Parquet file".into()),
            },
            only_listed_nodes: multimodal.bool("only_listed_nodes", false)?,
        },
        viz: viz_on,
        ..Params::default()
    };
    Ok(FromConfig {
        pbf,
        out,
        params,
        skipped,
        ignored,
    })
}

const FIX_TOP: [&str; 4] = ["overrides", "new_ways", "turn_restrictions", "extra"];
/// The keys of `extra` (an extension of duckosm: duckOSM reads only `overrides`, so it ignores the key `extra`).
const FIX_EXTRA: [&str; 2] = ["tag_edits", "building_faces"];
const FIX_FACE_EDIT: [&str; 7] = ["building", "from_node", "to_node", "type", "door", "note", "changed"];
/// `was`: what the data had before the change, kept by the page to show "was to now". `changed` (in each list): the date and the time of the
/// change, written by the fixes editor (docs/design/fixes_editor_time_merge.md). The build reads neither.
const FIX_TAG_EDIT: [&str; 7] = ["type", "id", "set", "delete", "note", "was", "changed"];
const FIX_RULE: [&str; 14] = [
    "osm_id",
    "oneway",
    "lanes",
    "lanes_forward",
    "lanes_backward",
    "layer",
    "exclude_modes",
    "remove",
    "tags",
    "keep_nodes",
    "turn_lanes_forward",
    "turn_lanes_backward",
    "note",
    "changed",
];
const FIX_NEW_WAY: [&str; 7] = ["id", "nodes", "tags", "tags_from", "source", "note", "changed"];
const FIX_TURN: [&str; 6] = ["from_way", "via_node", "to_way", "restriction", "note", "changed"];
/// The restriction types that the edge graph understands (duckOSM `_ALLOWED_RESTRICTIONS`).
const TURN_TYPES: [&str; 10] = [
    "no_u_turn",
    "no_left_turn",
    "no_right_turn",
    "no_straight_on",
    "no_entry",
    "no_exit",
    "only_straight_on",
    "only_left_turn",
    "only_right_turn",
    "only_u_turn",
];

/// Read a duckOSM OSM fixes file (`osm_overrides`). The rules for the arrows of the lanes and the
/// `turn_restrictions` give extra turn restrictions (the driving mode).
pub fn read_osm_fixes(path: &Path) -> Result<OsmFixes, String> {
    let text = duckosm_core::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_osm_fixes(&text, &path.display().to_string())
}

/// The same as [`read_osm_fixes`], from the text of the file (the browser has no files). `label` names the
/// file in the error messages.
pub fn parse_osm_fixes(text: &str, label: &str) -> Result<OsmFixes, String> {
    let err = |e: String| format!("{label}: {e}");
    let docs = YamlLoader::load_from_str(text).map_err(|e| err(e.to_string()))?;
    let empty = yaml_rust2::yaml::Hash::new();
    let root = match docs.first() {
        None | Some(Yaml::Null) => &empty,
        Some(Yaml::Hash(h)) => h,
        Some(_) => return Err(err("the file must be a map of keys".into())),
    };
    let int = |v: &Yaml, what: &str| {
        v.as_i64()
            .ok_or_else(|| err(format!("{what} must be an integer, not {v:?}")))
    };
    let keys = |h: &yaml_rust2::yaml::Hash, known: &[&str], what: &str| -> Result<(), String> {
        for k in h.keys() {
            let k = k.as_str().unwrap_or("?");
            if !known.contains(&k) {
                return Err(err(format!("unknown key '{k}' in {what}")));
            }
        }
        Ok(())
    };
    let tags = |v: Option<&Yaml>, what: &str| -> Result<Vec<(String, String)>, String> {
        let Some(v) = v else { return Ok(Vec::new()) };
        let h = v
            .as_hash()
            .ok_or_else(|| err(format!("{what} must be a map")))?;
        h.iter()
            .map(|(k, v)| {
                let val = match v {
                    Yaml::String(s) => s.clone(),
                    Yaml::Integer(i) => i.to_string(),
                    Yaml::Real(r) => r.clone(),
                    _ => {
                        return Err(err(format!(
                            "{what}: write the value of '{k:?}' as a text in quotes"
                        )))
                    }
                };
                Ok((k.as_str().unwrap_or("?").to_string(), val))
            })
            .collect()
    };
    keys(root, &FIX_TOP, "the file")?;
    let list = |key: &str| -> Result<&[Yaml], String> {
        match root.get(&Yaml::String(key.into())) {
            None | Some(Yaml::Null) => Ok(&[]),
            Some(Yaml::Array(a)) => Ok(a),
            Some(_) => Err(err(format!("{key} must be a list"))),
        }
    };
    let mut fixes = OsmFixes::default();
    for r in list("overrides")? {
        let h = r
            .as_hash()
            .ok_or_else(|| err("each rule in overrides must be a map".into()))?;
        keys(h, &FIX_RULE, "a rule of overrides")?;
        let s = Section {
            name: "overrides",
            map: Some(h),
        };
        let id = s
            .get("osm_id")
            .ok_or_else(|| err("a rule of overrides has no osm_id".into()))?;
        let osm_id = int(id, "osm_id")?;
        let what = format!("overrides (osm_id {osm_id})");
        let opt_int = |k: &str| {
            s.get(k)
                .map(|v| int(v, &format!("{what}.{k}")).map(|n| n as i32))
                .transpose()
        };
        let keep_nodes = match s.get("keep_nodes") {
            None => None,
            Some(Yaml::Array(a)) if a.len() == 2 => {
                Some((int(&a[0], "keep_nodes")?, int(&a[1], "keep_nodes")?))
            }
            Some(_) => return Err(err(format!("{what}.keep_nodes must be [from, to]"))),
        };
        let exclude_modes = match s.get("exclude_modes") {
            None => Vec::new(),
            Some(Yaml::Array(a)) => {
                let names: Vec<String> = a
                    .iter()
                    .map(|m| m.as_str().unwrap_or("?").to_string())
                    .collect();
                Mode::parse_all(&names).map_err(err)?
            }
            Some(_) => return Err(err(format!("{what}.exclude_modes must be a list"))),
        };
        fixes.ways.push(WayFix {
            osm_id,
            remove: s.bool("remove", false).map_err(err)?,
            keep_nodes,
            tags: tags(s.get("tags"), &format!("{what}.tags"))?,
            oneway: s
                .get("oneway")
                .map(|_| s.bool("oneway", false))
                .transpose()
                .map_err(err)?,
            lanes: opt_int("lanes")?,
            lanes_forward: opt_int("lanes_forward")?,
            lanes_backward: opt_int("lanes_backward")?,
            layer: opt_int("layer")?,
            exclude_modes,
            arrows: ["forward", "backward"]
                .iter()
                .filter_map(|d| {
                    let v = s.get(&format!("turn_lanes_{d}"))?;
                    let text = match v {
                        Yaml::String(t) => t.clone(),
                        Yaml::Integer(i) => i.to_string(),
                        _ => {
                            return Some(Err(err(format!("{what}.turn_lanes_{d} must be a text"))))
                        }
                    };
                    Some(Ok((format!("turn:lanes:{d}"), text)))
                })
                .collect::<Result<_, String>>()?,
        });
    }
    for n in list("new_ways")? {
        let h = n
            .as_hash()
            .ok_or_else(|| err("each way in new_ways must be a map".into()))?;
        keys(h, &FIX_NEW_WAY, "a way of new_ways")?;
        let s = Section {
            name: "new_ways",
            map: Some(h),
        };
        let id = int(
            s.get("id")
                .ok_or_else(|| err("a way of new_ways has no id".into()))?,
            "new_ways.id",
        )?;
        let what = format!("new_ways (id {id})");
        let nodes = s
            .get("nodes")
            .and_then(|v| v.as_vec())
            .ok_or_else(|| err(format!("{what}.nodes must be a list")))?
            .iter()
            .map(|x| match x {
                Yaml::Integer(i) => Ok(NewNode::Osm(*i)),
                Yaml::Array(p) if p.len() == 2 => {
                    let f = |v: &Yaml| match v {
                        Yaml::Integer(i) => Some(*i as f64),
                        Yaml::Real(r) => r.parse().ok(),
                        _ => None,
                    };
                    match (f(&p[0]), f(&p[1])) {
                        (Some(lon), Some(lat)) => Ok(NewNode::At((lon, lat))),
                        _ => Err(err(format!("{what}.nodes: [lon, lat] must be numbers"))),
                    }
                }
                _ => Err(err(format!(
                    "{what}.nodes: a node is an OSM node id or [lon, lat]"
                ))),
            })
            .collect::<Result<_, _>>()?;
        fixes.new_ways.push(NewWay {
            id,
            nodes,
            tags: tags(s.get("tags"), &format!("{what}.tags"))?,
            tags_from: s
                .get("tags_from")
                .map(|v| int(v, "tags_from"))
                .transpose()?,
        });
    }
    // duckOSM RestrictionProcessor._inject_overrides: the id is -(position), from -1. A rule without
    // the three ids, or with a type that the edge graph does not understand, is skipped.
    for (i, r) in list("turn_restrictions")?.iter().enumerate() {
        let h = r
            .as_hash()
            .ok_or_else(|| err("each rule in turn_restrictions must be a map".into()))?;
        keys(h, &FIX_TURN, "a rule of turn_restrictions")?;
        let s = Section {
            name: "turn_restrictions",
            map: Some(h),
        };
        let id = |k: &str| s.get(k).and_then(|v| v.as_i64());
        let (Some(from_way), Some(via_node), Some(to_way)) =
            (id("from_way"), id("via_node"), id("to_way"))
        else {
            continue;
        };
        let restriction = s
            .str("restriction")
            .map_err(err)?
            .unwrap_or_else(|| "no_u_turn".into());
        if !TURN_TYPES.contains(&restriction.as_str()) {
            continue;
        }
        fixes.turn_restrictions.push(TurnFix {
            id: -(i as i64 + 1),
            restriction,
            from_way,
            via_node,
            to_way,
        });
    }
    // `extra.tag_edits`: set and delete the tags of a way, a node or a relation (an extension: duckOSM ignores the key `extra`)
    match root.get(&Yaml::String("extra".into())) {
        None | Some(Yaml::Null) => {}
        Some(Yaml::Hash(x)) => {
            keys(x, &FIX_EXTRA, "extra")?;
            match x.get(&Yaml::String("tag_edits".into())) {
                None | Some(Yaml::Null) => {}
                Some(Yaml::Array(a)) => {
                    for (i, r) in a.iter().enumerate() {
                        let what = format!("extra.tag_edits[{i}]");
                        let h = r
                            .as_hash()
                            .ok_or_else(|| err(format!("{what} must be a map")))?;
                        keys(h, &FIX_TAG_EDIT, &what)?;
                        let get = |k: &str| h.get(&Yaml::String(k.into()));
                        let elem = match get("type").and_then(|v| v.as_str()) {
                            Some("way") => Elem::Way,
                            Some("node") => Elem::Node,
                            Some("relation") => Elem::Relation,
                            _ => {
                                return Err(err(format!(
                                    "{what}.type must be way, node or relation"
                                )))
                            }
                        };
                        let id = int(
                            get("id").ok_or_else(|| err(format!("{what} has no id")))?,
                            &format!("{what}.id"),
                        )?;
                        if !matches!(get("was"), None | Some(Yaml::Null) | Some(Yaml::Hash(_))) {
                            return Err(err(format!("{what}.was must be a map")));
                        }
                        let set = tags(get("set"), &format!("{what}.set"))?;
                        let delete = match get("delete") {
                            None | Some(Yaml::Null) => Vec::new(),
                            Some(Yaml::Array(d)) => d
                                .iter()
                                .map(|k| {
                                    k.as_str().map(String::from).ok_or_else(|| {
                                        err(format!("{what}.delete must be a list of keys"))
                                    })
                                })
                                .collect::<Result<_, _>>()?,
                            Some(_) => {
                                return Err(err(format!("{what}.delete must be a list of keys")))
                            }
                        };
                        fixes.tag_edits.push(TagEdit {
                            elem,
                            id,
                            set,
                            delete,
                        });
                    }
                }
                Some(_) => return Err(err("extra.tag_edits must be a list".into())),
            }
            // `extra.building_faces`: the type of a wall (`building_faces.rs`)
            match x.get(&Yaml::String("building_faces".into())) {
                None | Some(Yaml::Null) => {}
                Some(Yaml::Array(a)) => {
                    for (i, r) in a.iter().enumerate() {
                        let what = format!("extra.building_faces[{i}]");
                        let h = r
                            .as_hash()
                            .ok_or_else(|| err(format!("{what} must be a map")))?;
                        keys(h, &FIX_FACE_EDIT, &what)?;
                        let get = |k: &str| h.get(&Yaml::String(k.into()));
                        let id = |k: &str| -> Result<i64, String> {
                            int(
                                get(k).ok_or_else(|| err(format!("{what} has no {k}")))?,
                                &format!("{what}.{k}"),
                            )
                        };
                        let face_type = get("type")
                            .and_then(|v| v.as_str())
                            .and_then(FaceType::parse)
                            .ok_or_else(|| {
                                err(format!("{what}.type must be front, back, side, party or court"))
                            })?;
                        let door = match get("door") {
                            None | Some(Yaml::Null) => None,
                            Some(Yaml::String(d))
                                if !d.is_empty()
                                    && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
                            {
                                Some(d.clone())
                            }
                            Some(_) => {
                                return Err(err(format!(
                                    "{what}.door must be a word (letters, digits, _), for example main"
                                )))
                            }
                        };
                        fixes.face_edits.push(FaceEdit {
                            building: id("building")?,
                            from_node: id("from_node")?,
                            to_node: id("to_node")?,
                            face_type,
                            door,
                        });
                    }
                }
                Some(_) => return Err(err("extra.building_faces must be a list".into())),
            }
        }
        Some(_) => return Err(err("extra must be a map".into())),
    }
    Ok(fixes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("duckosm-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // a file of its own for each call: two tests with texts of one length ran at the same time on one file
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let p = dir.join(format!("c{}.yaml", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::write(&p, text).unwrap();
        p
    }

    fn pbf() -> String {
        format!(
            "{}/../../tests/data/monaco_sample.osm.pbf",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    #[test]
    fn osm_fixes_file() {
        use duckosm_core::network::build_network_with;
        use duckosm_core::pbf;
        let f = write(
            "overrides:\n  - osm_id: 93137596\n    lanes: 1\n    note: Larvotto\n  - osm_id: 1\n    \
             exclude_modes: [walking]\n    keep_nodes: [5, 6]\n    tags: {highway: residential, lanes: 2}\n    \
             turn_lanes_forward: left|through\nnew_ways:\n  - id: 9000000000001\n    source: nvdb:1\n    \
             nodes: [5, [7.4, 43.7]]\n    tags_from: 1\nturn_restrictions:\n  - from_way: 1\n  - {from_way: 2, via_node: 3, to_way: 4, restriction: no_left_turn}\n  \
             - {from_way: 5, via_node: 6, to_way: 7, restriction: bad_turn}\n",
        );
        let fixes = read_osm_fixes(&f).unwrap();
        assert_eq!(fixes.ways.len(), 2);
        assert_eq!(fixes.ways[0].lanes, Some(1));
        let r = &fixes.ways[1];
        assert_eq!(
            (r.exclude_modes.clone(), r.keep_nodes),
            (vec![Mode::Walking], Some((5, 6)))
        );
        assert_eq!(r.tags[1], ("lanes".to_string(), "2".to_string()));
        assert_eq!(fixes.new_ways[0].nodes.len(), 2);
        // the first rule has no via node and the third an unknown type: only the second is kept, with id -2
        let t = &fixes.turn_restrictions;
        assert_eq!(t.len(), 1);
        assert_eq!(
            (t[0].id, t[0].restriction.as_str(), t[0].via_node),
            (-2, "no_left_turn", 3)
        );
        let bad = write("overrides:\n  - osm_id: 1\n    lane: 2\n");
        assert!(read_osm_fixes(&bad).unwrap_err().contains("'lane'"));

        // Monaco: OSM has lanes=2 on this one-way road, the rule gives 1
        let pbf_path = PathBuf::from(pbf().replace("monaco_sample", "monaco"));
        let ways = pbf::read_ways(&pbf_path).unwrap();
        let coords = pbf::read_coords(&pbf_path, &ways).unwrap();
        let lanes = |fx: &OsmFixes| {
            let net =
                build_network_with(&ways, &coords, Mode::Driving, &Options::default(), fx, &[])
                    .unwrap();
            let l: Vec<_> = net
                .edges
                .iter()
                .filter(|e| e.osm_id == 93137596)
                .map(|e| e.lanes)
                .collect();
            (l, net.stats.osm_fixes)
        };
        let (before, _) = lanes(&OsmFixes::default());
        let (after, n) = lanes(&fixes);
        assert!(
            !before.is_empty() && before.iter().all(|l| *l == Some(2)),
            "{before:?}"
        );
        assert!(after.iter().all(|l| *l == Some(1)) && n == 1, "{after:?}");
    }

    #[test]
    fn elevation_section() {
        let tif = format!("{}/tests/data/monaco-copernicus-glo30.tif", env!("CARGO_MANIFEST_DIR"));
        let c = write(&format!(
            "pbf_path: {}\nelevation:\n  dem: {tif}\n  product: DTM\n  vertical_datum: EGM2008\n  license: open\n",
            pbf()
        ));
        let p = read(&c, false, false).unwrap().params;
        assert_eq!(p.dem, Some(PathBuf::from(&tif)));
        assert_eq!(
            (p.dem_product.as_deref(), p.dem_vertical_datum.as_deref(), p.dem_license.as_deref()),
            (Some("DTM"), Some("EGM2008"), Some("open"))
        );
        assert!(!p.dem_not_reference);
        let c = write(&format!("pbf_path: {}\nelevation:\n  dem: {tif}\n  reference: false\n", pbf()));
        assert!(read(&c, false, false).unwrap().params.dem_not_reference);
        let c = write(&format!("pbf_path: {}\nelevation:\n  dem: nope.tif\n", pbf()));
        assert!(read(&c, false, false).unwrap_err().contains("DEM file not found"));
        let c = write(&format!("pbf_path: {}\nelevation:\n  datum: EGM2008\n", pbf()));
        assert!(read(&c, false, false).unwrap_err().contains("elevation.datum"));
    }

    #[test]
    fn defaults_run_in_strict_mode() {
        let c = write(&format!("name: m\noutput_path: out\npbf_path: {}\n", pbf()));
        // nothing is skipped: without a boundary the base-map layers run on the raw.* of the whole file
        let r = read(&c, false, true).unwrap();
        assert!(r.skipped.is_empty(), "{:?}", r.skipped);
        assert!(r.params.features && r.params.sea);
        assert_eq!(r.params.h3_resolution, Some(8));
        assert_eq!(r.params.name, "m");
        assert!(r.params.options.merge_segments);
    }

    #[test]
    fn features_and_h3_with_a_boundary() {
        let c = write(&format!(
            "pbf_path: {}\nboundary:\n  path: x.geojson\noptions:\n  h3_resolution: 9\n",
            pbf()
        ));
        let r = read(&c, true, true).unwrap(); // strict: nothing is skipped
        assert!(r.skipped.is_empty() && r.params.features && r.params.sea);
        assert_eq!(r.params.h3_resolution, Some(9));
        let c = write(&format!(
            "pbf_path: {}\nboundary:\n  path: x.geojson\noptions:\n  sea: false\n  h3_indexing: false\n",
            pbf()
        ));
        let r = read(&c, true, true).unwrap();
        assert!(r.params.features && !r.params.sea && r.params.h3_resolution.is_none());
        let c = write(&format!("pbf_path: {}\noptions:\n  h3_resolution: 16\n", pbf()));
        assert!(read(&c, true, false).unwrap_err().contains("0 to 15"));
    }

    #[test]
    fn unknown_key_and_boundary() {
        let c = write(&format!(
            "pbf_path: {}\noptions:\n  merge_segment: false\n",
            pbf()
        ));
        assert!(read(&c, false, false)
            .unwrap_err()
            .contains("merge_segment"));
        let c = write(&format!(
            "pbf_path: {}\nboundary:\n  path: x.geojson\n",
            pbf()
        ));
        assert!(read(&c, false, false)
            .unwrap_err()
            .contains("boundary file not found"));
        let r = read(&c, true, false).unwrap();
        assert!(r.params.boundary.is_none());
    }

    #[test]
    fn all_off_is_strict_ok_and_output_file() {
        let c = write(&format!(
            "name: m\noutput_path: out/area.duckdb\nsource:\n  type: pbf\n  pbf_path: {}\nmodes: [driving, walking]\n\
             options:\n  build_graph: false\n  h3_indexing: false\n  extract_restrictions: false\n  build_features: false\n  \
             levels: false\n  bus_routes: false\n  merge_segments: false\n  threads: 4\n",
            pbf()
        ));
        let r = read(&c, false, true).unwrap();
        assert_eq!(r.params.name, "area");
        assert_eq!(r.out, PathBuf::from("out"));
        assert_eq!(r.params.modes, vec![Mode::Driving, Mode::Walking]);
        assert!(!r.params.options.merge_segments);
        // threads sets the DuckDB of the load of raw.* (read_osm), so it is not ignored
        assert_eq!(r.params.threads, Some(4));
        assert!(r.ignored.is_empty(), "{:?}", r.ignored);
    }

    #[test]
    fn extra_tag_edits() {
        let f = parse_osm_fixes(
            "overrides:\n  - osm_id: 1\n    lanes: 2\nextra:\n  tag_edits:\n    - type: way\n      id: 5\n      \
             set: {\"building:levels\": 4, \"roof:shape\": gabled}\n      delete: [\"roof:colour\"]\n      was: {\"building:levels\": null, \"roof:colour\": red}\n    - {type: node, id: 7, \
             set: {crossing: marked}}\n    - {type: relation, id: 9, delete: [building], note: why}\n",
            "t",
        )
        .unwrap();
        assert_eq!(f.ways.len(), 1);
        let e = &f.tag_edits;
        assert_eq!(e.len(), 3);
        assert_eq!((e[0].elem, e[0].id), (Elem::Way, 5));
        assert_eq!(
            e[0].set,
            [
                ("building:levels".to_string(), "4".to_string()),
                ("roof:shape".to_string(), "gabled".to_string())
            ]
        );
        assert_eq!(e[0].delete, ["roof:colour"]);
        assert_eq!((e[1].elem, e[1].id, e[1].set.len()), (Elem::Node, 7, 1));
        assert_eq!(
            (e[2].elem, e[2].delete.clone()),
            (Elem::Relation, vec!["building".to_string()])
        );
        // a file with only duckOSM keys has no tag edit
        assert!(
            parse_osm_fixes("overrides:\n  - osm_id: 1\n    lanes: 2\n", "t")
                .unwrap()
                .tag_edits
                .is_empty()
        );
        for (text, msg) in [
            ("extra:\n  other: 1\n", "unknown key 'other' in extra"),
            (
                "extra:\n  tag_edits:\n    - {type: area, id: 1}\n",
                "type must be way, node or relation",
            ),
            ("extra:\n  tag_edits:\n    - {type: way}\n", "has no id"),
            (
                "extra:\n  tag_edits:\n    - {type: way, id: 1, delete: x}\n",
                "delete must be a list of keys",
            ),
            (
                "extra:\n  tag_edits:\n    - {type: way, id: 1, nope: 1}\n",
                "unknown key 'nope'",
            ),
            ("extra: 3\n", "extra must be a map"),
            ("extra:\n  tag_edits:\n    - {type: way, id: 1, was: 3}\n", "was must be a map"),
        ] {
            let e = parse_osm_fixes(text, "t").unwrap_err();
            assert!(e.contains(msg), "{text:?}: {e}");
        }
    }

    #[test]
    fn the_time_of_a_change_is_read_and_not_used() {
        let t = "'2026-11-06T14:32:05+01:00'";
        let text = format!(
            "overrides:\n  - {{osm_id: 1, lanes: 2, changed: {t}}}\nnew_ways:\n  - {{id: 9000000000001, nodes: [5, [7.4, 43.7]], tags: {{highway: service}}, changed: {t}}}\n\
             turn_restrictions:\n  - {{from_way: 2, via_node: 3, to_way: 4, restriction: no_left_turn, changed: {t}}}\nextra:\n  tag_edits:\n    \
             - {{type: node, id: 7, set: {{crossing: marked}}, changed: {t}}}\n  building_faces:\n    - {{building: 5, from_node: 1, to_node: 2, type: front, changed: {t}}}\n"
        );
        let f = parse_osm_fixes(&text, "t").unwrap();
        assert_eq!((f.ways.len(), f.new_ways.len(), f.turn_restrictions.len(), f.tag_edits.len(), f.face_edits.len()), (1, 1, 1, 1, 1));
    }

    #[test]
    fn extra_building_faces() {
        let f = parse_osm_fixes(
            "extra:\n  building_faces:\n    - {building: 5, from_node: 1, to_node: 2, type: front}\n    - {building: 5, from_node: 2, to_node: 3, type: party, note: shared}\n",
            "t",
        )
        .unwrap();
        let e = &f.face_edits;
        assert_eq!(e.len(), 2);
        assert_eq!((e[0].building, e[0].from_node, e[0].to_node, e[0].face_type), (5, 1, 2, FaceType::Front));
        assert_eq!(e[1].face_type, FaceType::Party);
        assert_eq!((e[0].door.as_deref(), e[1].door.as_deref()), (None, None));
        let d = parse_osm_fixes("extra:\n  building_faces:\n    - {building: 5, from_node: 1, to_node: 2, type: side, door: main}\n", "t").unwrap();
        assert_eq!(d.face_edits[0].door.as_deref(), Some("main"));
        for (text, msg) in [
            ("extra:\n  building_faces:\n    - {building: 5, from_node: 1, to_node: 2, type: roof}\n", "type must be front, back, side, party or court"),
            ("extra:\n  building_faces:\n    - {building: 5, from_node: 1, type: front}\n", "has no to_node"),
            ("extra:\n  building_faces:\n    - {building: 5, from_node: 1, to_node: 2, type: front, x: 1}\n", "unknown key 'x'"),
            ("extra:\n  building_faces: 3\n", "extra.building_faces must be a list"),
            ("extra:\n  building_faces:\n    - {building: 5, from_node: 1, to_node: 2, type: front, door: 'a b'}\n", "door must be a word"),
        ] {
            let e = parse_osm_fixes(text, "t").unwrap_err();
            assert!(e.contains(msg), "{text:?}: {e}");
        }
    }
}
