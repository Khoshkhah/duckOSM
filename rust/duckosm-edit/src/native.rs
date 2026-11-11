//! The database of the program: the bundled DuckDB (the version of duckosm-duckdb), and the native build of duckosm-build.

use crate::Db;
use duckdb::{AccessMode, Config, Connection};
use std::path::{Path, PathBuf};

pub struct NativeDb {
    path: PathBuf,
    con: Option<Connection>,
}

impl NativeDb {
    pub fn new(path: &Path) -> NativeDb {
        NativeDb { path: path.to_path_buf(), con: None }
    }

    fn con(&self) -> Result<&Connection, String> {
        self.con.as_ref().ok_or_else(|| "the database is not open".into())
    }
}

/// The build of a config text: the YAML in a file of its own, read by duckosm-build's config reader in the folder of the build. A step
/// that the native build does not have is in the summary as "skipped".
fn read_config(config: &str, dir: &Path) -> Result<duckosm_build::config::FromConfig, String> {
    let file = std::env::temp_dir().join(format!("duckosm-edit-{}-config.yaml", std::process::id()));
    std::fs::write(&file, config).map_err(|e| format!("{}: {e}", file.display()))?;
    // ponytail: the config's relative paths are for the folder of the build, so the process goes there; the server does nothing else
    std::env::set_current_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let r = duckosm_build::config::read(&file, false, false);
    let _ = std::fs::remove_file(&file);
    r
}

impl Db for NativeDb {
    fn open(&mut self) -> Result<(), String> {
        let cfg = Config::default().access_mode(AccessMode::ReadOnly).map_err(|e| e.to_string())?;
        let con = Connection::open_with_flags(&self.path, cfg).map_err(|e| format!("{}: {e}", self.path.display()))?;
        con.execute_batch("INSTALL spatial; LOAD spatial;").map_err(|e| format!("the spatial extension of DuckDB: {e}"))?;
        self.con = Some(con);
        Ok(())
    }

    fn close(&mut self) {
        self.con = None;
    }

    fn execute(&mut self, sql: &str) -> Result<(), String> {
        self.con()?.execute_batch(sql).map_err(|e| e.to_string())
    }

    fn texts(&mut self, sql: &str) -> Result<Vec<Option<String>>, String> {
        let con = self.con()?;
        let mut st = con.prepare(sql).map_err(|e| e.to_string())?;
        let rows = st.query_map([], |r| r.get::<_, Option<String>>(0)).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    fn can_build(&mut self, config: &str, dir: &Path) -> Result<(), String> {
        read_config(config, dir).map(|_| ())
    }

    fn build(&mut self, config: &str, dir: &Path, fixes: &Path, db: &Path) -> Result<String, String> {
        let c = read_config(config, dir)?;
        let mut p = c.params;
        p.osm_fixes = Some(std::fs::canonicalize(fixes).map_err(|e| format!("{}: {e}", fixes.display()))?);
        // the build writes the open database again: the same folder and name, the .duckdb only
        p.name = db.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "area".into());
        (p.duckdb, p.geoparquet) = (true, false);
        let out = db.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let sum = duckosm_build::build(&c.pbf, out, &p)?;
        crate::native::record(db, config, dir)?;
        let mut lines: Vec<String> = sum.lines();
        lines.extend(c.skipped.iter().map(|s| format!("skipped: {s}")));
        Ok(lines.join("\n"))
    }
}

/// Write the build's record into `db` (main.visualization_metadata: `build_config`, the config as YAML, and `build_dir`, the folder of
/// the build), so that `duckosm edit` can build it again. `duckosm-rust --config` calls it after a build, and Build again after its build.
pub fn record(db: &Path, config: &str, dir: &Path) -> Result<(), String> {
    let con = Connection::open(db).map_err(|e| format!("{}: {e}", db.display()))?;
    con.execute_batch("ALTER TABLE main.visualization_metadata ADD COLUMN IF NOT EXISTS build_config VARCHAR;
                       ALTER TABLE main.visualization_metadata ADD COLUMN IF NOT EXISTS build_dir VARCHAR;")
        .map_err(|e| e.to_string())?;
    con.execute("UPDATE main.visualization_metadata SET build_config = ?, build_dir = ?", [config, &dir.display().to_string()])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The program's host of `duckosm studio`: the native engine builds the area (`duckosm-build`, the same as `duckosm-rust --config`), and the
/// page of the result opens the file with the bundled DuckDB. The DEM is the config's `elevation:` section.
pub struct NativeHost;

impl crate::studio::Host for NativeHost {
    fn db(&self, path: &Path) -> Box<dyn Db> {
        Box::new(NativeDb::new(path))
    }

    fn write(&self, db: &Path, sql: &str) -> Result<(), String> {
        let con = Connection::open(db).map_err(|e| format!("{}: {e}", db.display()))?;
        con.execute_batch(sql).map_err(|e| {
            let _ = con.execute_batch("ROLLBACK");
            e.to_string()
        })
    }

    fn buffer(&self, wkt: &str, epsg: u32, metres: f64) -> Result<String, String> {
        let con = Connection::open_in_memory().map_err(|e| e.to_string())?;
        con.execute_batch("INSTALL spatial; LOAD spatial;").map_err(|e| format!("the spatial extension of DuckDB: {e}"))?;
        let sql = format!("SELECT ST_AsGeoJSON(ST_Transform(ST_Buffer(ST_Transform(ST_GeomFromText('{wkt}'), 'EPSG:4326', 'EPSG:{epsg}', true), {metres}), 'EPSG:{epsg}', 'EPSG:4326', true))");
        con.query_row(&sql, [], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())
    }

    fn build(&self, config: &str, dir: &Path, db: &Path, log: &(dyn Fn(&str) + Sync)) -> Result<String, String> {
        let c = read_config(config, dir)?;
        let mut p = c.params;
        p.name = db.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "area".into());
        (p.duckdb, p.geoparquet) = (true, false);
        let out = db.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        // the steps of the engine go to `log` while the build runs
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        duckosm_build::set_progress(Some(Box::new(move |step| {
            let _ = tx.send(step.to_string());
        })));
        let sum = std::thread::scope(|s| {
            s.spawn(move || rx.iter().for_each(|line| log(&line)));
            let r = duckosm_build::build(&c.pbf, out, &p);
            duckosm_build::set_progress(None);       // the hook held the sender: the reader ends
            r
        })?;
        record(db, config, dir)?;
        let mut lines: Vec<String> = sum.lines();
        lines.extend(c.skipped.iter().map(|s| format!("skipped: {s}")));
        Ok(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use crate::studio::Host;
    use serde_json::json;

    /// Card 8's save (`page::save_levels_sql`, run by the host's `write`): the levels, the meta row and a head go into the tables; levels.csv
    /// with another number of edges stops the script, and the tables do not change.
    #[test]
    fn the_level_editor_save_writes_the_tables_and_stops_on_other_edges() {
        let dir = std::env::temp_dir().join(format!("duckosm-edit-level-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("a.duckdb");
        duckdb::Connection::open(&db).unwrap().execute_batch("CREATE SCHEMA visualization;
            CREATE TABLE visualization.edge_levels AS SELECT * FROM (VALUES (9007199254740993::BIGINT, 0, 0, 0, 0, NULL::DOUBLE, NULL::DOUBLE, NULL::VARCHAR, NULL::VARCHAR),
              (2, 0, 0, 0, 0, NULL, NULL, NULL, NULL)) t(edge_id, casing_start, casing_level, casing_end, fill_level, head_start_m, head_end_m, cap_start, cap_end);
            CREATE TABLE visualization.edge_levels_meta AS SELECT 2 AS n_edges, 'h' AS edge_hash, 'build' AS source, '' AS area, '' AS created;").unwrap();
        let files = json!({"levels.csv": "edge,casing_start,casing_level,casing_end,fill_level,head_start_m,head_end_m,cap_start,cap_end\n9007199254740993,1,1,1,1,,7,,\n2,0,0,0,0,,,,\n",
                           "heads.csv": "road,start_m,end_m,time\n9007199254740993,,7,2026-11-11 12:00\n", "edits.csv": "", "caps.csv": ""});
        let sql = crate::page::save_levels_sql(&dir.join("files"), &files, "monaco").unwrap();
        super::NativeHost.write(&db, &sql).unwrap();
        let con = duckdb::Connection::open(&db).unwrap();
        let one = |q: &str| con.query_row(q, [], |r| r.get::<_, String>(0)).unwrap();
        assert_eq!(one("SELECT fill_level::VARCHAR FROM visualization.edge_levels WHERE edge_id = 9007199254740993"), "1");
        assert_eq!(one("SELECT source || ' ' || area || ' ' || (created LIKE '20__-__-__T__:__:__+00:00')::VARCHAR FROM visualization.edge_levels_meta"), "area monaco true");
        assert_eq!(one("SELECT road::VARCHAR || ' ' || end_m FROM visualization.level_heads"), "9007199254740993 7");
        drop(con);
        let short = json!({"levels.csv": "edge,casing_start,casing_level,casing_end,fill_level\n2,3,3,3,3\n"});
        let err = super::NativeHost.write(&db, &crate::page::save_levels_sql(&dir.join("files"), &short, "monaco").unwrap()).unwrap_err();
        assert!(err.contains("levels.csv has 1 edges, the build has 2"), "{err}");
        let con = duckdb::Connection::open(&db).unwrap();
        assert_eq!(con.query_row("SELECT fill_level::VARCHAR FROM visualization.edge_levels WHERE edge_id = 2", [], |r| r.get::<_, String>(0)).unwrap(), "0", "no change");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
