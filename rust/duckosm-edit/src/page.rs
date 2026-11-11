//! The page of `duckosm edit`: roadstyle's standard dashboard of the duckOSM database (the template "dashboard", in its default layout, with its panel,
//! toolbox and phone sheet), as roadstyle's own `duckosm.page(db, dashboard=True)` makes it. roadstyle_io::duckosm has the SQL that reads
//! the file (the roads of every mode with their stored levels, and the base map's `features.*`); this module runs it on the database of
//! the server, and roadstyle's page writer writes the page. The page's own script (page/edit.js) puts the fixes editor into the panel.

use crate::{rows, Db};
use roadstyle::duckosm as d;
use serde_json::{json, Value};

/// The host part of the page: the editor in the dashboard's panel, the database and the fixes file through the server (page/edit.js),
/// and the classes and colours that the editor takes from its host (page/edit.css).
const SCRIPT: &str = include_str!("../page/edit.js");
const STYLE: &str = include_str!("../page/edit.css");
/// The host part of the page of `duckosm ask` (page/ask.js): roadstyle's data dock and Ask, and the changes of the fixes file as a table.
const ASK_SCRIPT: &str = include_str!("../page/ask.js");
/// The host part of the page of `duckosm studio` (page/studio.js, page/studio.css): the steps in the dashboard's panel.
const STUDIO_SCRIPT: &str = include_str!("../page/studio.js");
const STUDIO_STYLE: &str = include_str!("../page/studio.css");
const MAPLIBRE_JS: &str = roadstyle::assets::text!("vendor/maplibre-gl.js");
const MAPLIBRE_CSS: &str = roadstyle::assets::text!("vendor/maplibre-gl.css");

/// The rows of a query as arrays of their values, in the order of the columns.
fn values(db: &mut dyn Db, sql: &str) -> Result<Vec<Vec<Value>>, String> {
    Ok(rows(db, sql)?.into_iter().map(|r| r.as_object().map(|o| o.values().cloned().collect()).unwrap_or_default()).collect())
}

/// The rows of a SELECT as Parquet bytes (the geometry as WKB): DuckDB writes a file, and the file is read and deleted.
fn parquet(db: &mut dyn Db, sql: &str) -> Result<Vec<u8>, String> {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("duckosm-edit-{}-{n}.parquet", std::process::id()));
    let name = file.display().to_string().replace('\'', "''");
    db.execute(&format!("COPY ({sql}) TO '{name}' (FORMAT parquet)"))?;
    let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()));
    let _ = std::fs::remove_file(&file);
    bytes
}

/// The page of `duckosm edit`: the open database `db` (the area `name`) with the fixes editor.
pub fn make(db: &mut dyn Db, name: &str) -> Result<String, String> {
    make_with(db, name, SCRIPT, STYLE)
}

/// The page of `duckosm ask`: the same map, with roadstyle's data dock and Ask. `extra` is a small database file in the temp folder for the page's
/// own tables (the changes of the fixes file): DuckDB does not attach an in-memory database next to a read-only one. The file of the area stays read
/// only, and the lock after this step does not stop a write to `extra`.
/// ponytail: the file stays in the temp folder after Ctrl+C (a few kB); the next run of the same process id writes it again.
pub fn make_ask(db: &mut dyn Db, name: &str) -> Result<String, String> {
    let file = std::env::temp_dir().join(format!("duckosm-ask-{}.duckdb", std::process::id()));
    for f in [file.clone(), file.with_extension("duckdb.wal")] {
        let _ = std::fs::remove_file(f);
    }
    db.execute(&format!("ATTACH '{}' AS extra (READ_WRITE)", file.display().to_string().replace('\'', "''")))?;
    make_with(db, name, ASK_SCRIPT, "")
}

fn make_with(db: &mut dyn Db, name: &str, script: &str, style: &str) -> Result<String, String> {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let catalog = db.texts("SELECT current_database()")?.into_iter().next().flatten().unwrap_or_default();
    let [tq, cq] = d::catalog_sql(&catalog);
    let cat = d::Catalog { tables: values(db, &tq)?.iter().map(|r| (s(&r[0]), s(&r[1]))).collect(),
                           columns: values(db, &cq)?.iter().map(|r| (s(&r[0]), s(&r[1]), s(&r[2]))).collect() };
    // the stored levels of the build (visualization.edge_levels); a file with none: the tags rule (no solve: the page opens at once)
    let tags = !cat.tables.iter().any(|(a, b)| a == "visualization" && b == "edge_levels");
    let run = d::Run::new("duckosm_edit").map_err(|e| e.to_string())?;
    let [e, r, drop_e] = d::roads_sql(&cat, tags, None, &run)?;
    db.execute(&e)?;
    let meta = if tags { None } else { values(db, &d::levels_meta_sql(&cat)).ok().and_then(|v| v.into_iter().next()) };
    db.execute(&r)?;
    db.execute(&drop_e)?;
    let levels = if tags { json!("tags") } else {
        let row = meta.as_ref().map(|m| (m[0].as_str().unwrap_or_default(), m[1].as_i64().unwrap_or_default(), m[2].as_str().unwrap_or_default()));
        d::levels_keyword(row).map_err(|e| e.to_string())?
    };
    // the base map of the duckOSM file: its look gives the layers (layers.yaml), and each layer's first query that runs makes its table
    let base = json!({"mode": "all", "theme": "liberty", "paths": "google", "place": name});
    let look = |args: Value| -> Result<(roadstyle::look::Options, roadstyle::Look), String> {
        let o = roadstyle::look::Options::from_value(&json!({"base_map": base, "args": args})).map_err(|e| e.to_string())?;
        let l = roadstyle::Look::resolve(&o).map_err(|e| e.to_string())?;
        Ok((o, l))
    };
    let specs = look(json!({}))?.1.sections.base_map.as_ref().and_then(|b| b.layers_yaml["layers"].as_array().cloned()).unwrap_or_default();
    let mut made = Vec::new();
    for l in d::features_sql(&cat, &specs) {
        if l.sql.iter().any(|q| db.execute(&format!("CREATE OR REPLACE TEMP TABLE {} AS {q}", run.feature(l.spec))).is_ok()) {
            made.push(l.spec);
        }
    }
    let roads = parquet(db, &d::roads_select("all", &run).map_err(|e| e.to_string())?)?;
    let features = d::features_select(&made, &run).map(|q| parquet(db, &q)).transpose()?;
    db.execute(&run.drop_sql(&made))?;
    // roadstyle's dashboard page (roadstyle's duckosm.page(dashboard=True)) in roadstyle's default layout (the panel a card over the map),
    // the clicked road in the panel (no tooltip)
    let (o, lk) = look(json!({"name": name, "template": "dashboard", "tooltip": null, "state": "live", "levels": levels,
                             "page_script": script, "page_style": style}))?;
    let given = roadstyle::page_options::given_of(&lk, &o.args);
    let roads = roadstyle::read::read_roads(roads, "geometry").map_err(|e| e.to_string())?;
    let features = features.map(roadstyle::read_features).transpose().map_err(|e| e.to_string())?;
    let page = roadstyle::page_options::page_from(roads, features, &lk, &o, &given).map_err(|e| e.to_string())?;
    for w in &page.warnings {
        eprintln!("duckosm edit: the map: {w}");
    }
    Ok(page.html.replacen("__MAPLIBRE_CSS__", MAPLIBRE_CSS, 1).replacen("__MAPLIBRE_JS__", &MAPLIBRE_JS.replace("</script>", "<\\/script>"), 1))
}

/// The page of `duckosm studio`: roadstyle's standard dashboard (the same template, toolbox and panel as the page of `duckosm edit`) for an area with no
/// roads yet. The panel starts empty; the script (page/studio.js) puts the steps into it, as cards. MapLibre and roadstyle's own files come from the program.
pub fn make_studio() -> Result<String, String> {
    let args = json!({"name": "duckOSM Studio", "template": "dashboard", "tooltip": null, "state": "live", "tiles": false, "page_script": STUDIO_SCRIPT, "page_style": STUDIO_STYLE});
    let o = roadstyle::look::Options::from_value(&json!({"args": args})).map_err(|e| e.to_string())?;
    let lk = roadstyle::Look::resolve(&o).map_err(|e| e.to_string())?;
    let given = roadstyle::page_options::given_of(&lk, &o.args);
    let page = roadstyle::page_options::page_from(roadstyle::RoadTable::empty(), None, &lk, &o, &given).map_err(|e| e.to_string())?;
    for w in &page.warnings {
        eprintln!("duckosm studio: the map: {w}");
    }
    Ok(page.html.replacen("__MAPLIBRE_CSS__", MAPLIBRE_CSS, 1).replacen("__MAPLIBRE_JS__", &MAPLIBRE_JS.replace("</script>", "<\\/script>"), 1))
}
