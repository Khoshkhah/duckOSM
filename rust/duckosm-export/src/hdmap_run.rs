//! `duckosm hdmap`: the HD map of an area, made by SimReady (docs/design/hdmap_simready.md). duckOSM writes SimReady's input
//! folder from the routing database and the GMNS database, runs SimReady's program (`duckosm-sim build`) and leaves its
//! output. No SimReady code is here: duckOSM only writes files and runs a program.

use crate::Result;
use duckdb::Connection;
use std::path::{Path, PathBuf};

/// The number of the input folder's layout. SimReady stops when it does not know it.
pub const CONTRACT: i64 = 1;

/// The values of input.json that SimReady's project row needs and the databases do not hold: the ISO 3166 code of the
/// area (`jurisdiction`) and the side of the road that traffic drives on (`driving_side`, `right` or `left`). None: null.
#[derive(Default, Clone)]
pub struct HdmapArea {
    pub jurisdiction: Option<String>,
    pub driving_side: Option<String>,
}

/// What one run did.
pub struct HdmapSummary {
    pub input: PathBuf,
    pub out: PathBuf,
    pub files: Vec<(String, String)>,
}

fn sql_str(p: &Path) -> String {
    p.display().to_string().replace('\'', "''")
}

/// SimReady's program: `simready`, else `$DUCKOSM_SIM`, else `duckosm-sim` on the `PATH`.
pub fn find_simready(simready: Option<&Path>) -> Result<PathBuf> {
    let not_found = "SimReady is not installed: give --simready or set DUCKOSM_SIM";
    if let Some(p) = simready.map(Path::to_path_buf).or_else(|| std::env::var_os("DUCKOSM_SIM").map(PathBuf::from)) {
        return if p.is_file() { Ok(p) } else { Err(format!("{not_found} ({} is not a file)", p.display())) };
    }
    std::env::var_os("PATH")
        .and_then(|paths| std::env::split_paths(&paths).map(|d| d.join("duckosm-sim")).find(|p| p.is_file()))
        .ok_or_else(|| not_found.to_string())
}

/// The checks of the design ("Step 1"): one area, the chains, the GMNS values, the raw OSM tables. Each failure stops the
/// run with its message.
fn check(con: &Connection) -> Result<()> {
    let count = |sql: &str| -> Result<i64> { con.query_row(sql, [], |r| r.get(0)).map_err(|e| format!("{e}\n{sql}")) };
    let table = |db: &str, sch: &str, t: &str| -> Result<bool> {
        Ok(count(&format!(
            "SELECT count(*) FROM duckdb_tables() WHERE database_name = '{db}' AND schema_name = '{sch}' AND table_name = '{t}'"
        ))? > 0)
    };
    let column = |t: &str, c: &str| -> Result<bool> {
        Ok(count(&format!(
            "SELECT count(*) FROM duckdb_columns() WHERE database_name = 'g' AND schema_name = 'gmns_driving' \
             AND table_name = '{t}' AND column_name = '{c}'"
        ))? > 0)
    };
    if !table("g", "gmns_driving", "link")? {
        return Err("the GMNS database has no gmns_driving.link: make it with `duckosm gmns --lane-source rules`".into());
    }
    if !table("g", "gmns_driving", "chains")? {
        return Err("the GMNS database has no gmns_driving.chains: make it again with this duckOSM version".into());
    }
    for (t, c) in [("link", "refs"), ("link", "z_vertices"), ("config", "duckosm_version")] {
        if !column(t, c)? {
            return Err(format!("the GMNS database has no {t}.{c}: make it again with this duckOSM version"));
        }
    }
    // each driving link in exactly one chain: none left out, none twice
    let bad = count(
        "SELECT count(*) FROM (SELECT link_id FROM g.gmns_driving.link EXCEPT SELECT unnest(link_ids) FROM g.gmns_driving.chains)",
    )? + count(
        "SELECT count(*) - count(DISTINCT x) FROM (SELECT unnest(link_ids) AS x FROM g.gmns_driving.chains)",
    )?;
    if bad > 0 {
        return Err("gmns_driving.chains: a driving link is in no chain or in more than one".into());
    }
    if !table("s", "driving", "edges")? {
        return Err("the routing database has no driving.edges".into());
    }
    let private = if table("s", "driving", "private_edges")? { " UNION ALL SELECT edge_id FROM s.driving.private_edges" } else { "" };
    let foreign = count(&format!(
        "SELECT count(*) FROM g.gmns_driving.link WHERE link_id NOT IN (SELECT edge_id FROM s.driving.edges{private})"
    ))?;
    if foreign > 0 {
        return Err(format!("the GMNS database is not of this routing database: {foreign} links are not its edges"));
    }
    for t in ["nodes", "ways", "relations"] {
        if !table("s", "raw", t)? {
            return Err(format!(
                "the routing database has no raw.{t}: build it from the PBF (a database cut from a parent has no raw tables)"
            ));
        }
    }
    Ok(())
}

/// The input folder `dir` of SimReady (the design, "Step 1"). Returns the files with their SHA-256.
pub fn write_input(db: &Path, gmns_db: &Path, dir: &Path, area_opts: &HdmapArea) -> Result<Vec<(String, String)>> {
    if let Some(s) = area_opts.driving_side.as_deref().filter(|s| *s != "right" && *s != "left") {
        return Err(format!("driving side {s:?}: give right or left"));
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let con = Connection::open_in_memory().map_err(|e| e.to_string())?;
    let x = |sql: &str| con.execute_batch(sql).map_err(|e| format!("{e}\n{sql}"));
    x(&format!("ATTACH '{}' AS s (READ_ONLY); ATTACH '{}' AS g (READ_ONLY)", sql_str(db), sql_str(gmns_db)))?;
    check(&con)?;
    for t in ["nodes", "ways", "relations"] {
        x(&format!("COPY s.raw.{t} TO '{}' (FORMAT parquet)", sql_str(&dir.join(format!("raw_{t}.parquet")))))?;
    }
    let has = |sch: &str, t: &str| -> Result<bool> {
        con.query_row(
            "SELECT count(*) FROM duckdb_tables() WHERE database_name = 's' AND schema_name = ? AND table_name = ?",
            [sch, t],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(|e| e.to_string())
    };
    if has("main", "elevation_grid")? {
        x(&format!("COPY s.main.elevation_grid TO '{}' (FORMAT parquet)", sql_str(&dir.join("elevation_grid.parquet"))))?;
    }
    if has("main", "fixes")? {
        let yaml: Option<String> = con.query_row("SELECT yaml FROM s.main.fixes LIMIT 1", [], |r| r.get(0)).ok();
        if let Some(y) = yaml {
            std::fs::write(dir.join("fixes.yaml"), y).map_err(|e| e.to_string())?;
        }
    }
    let ground: String = con
        .query_row(
            "SELECT coalesce((SELECT first(sha256 ORDER BY ele_column) FROM g.main.elevation_metadata), '')",
            [],
            |r| r.get(0),
        )
        .unwrap_or_default();
    // the time of the OSM data (the replication time of the PBF header), when the build recorded it
    let osm_time: Option<String> = con
        .query_row("SELECT strftime(first(osm_timestamp), '%Y-%m-%d %H:%M:%S') FROM s.main.osm_source", [], |r| r.get(0))
        .ok()
        .flatten();
    let version: String = con
        .query_row("SELECT first(duckosm_version) FROM g.gmns_driving.config", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    x("DETACH s; DETACH g")?;
    std::fs::copy(gmns_db, dir.join("gmns.duckdb")).map_err(|e| format!("{}: {e}", gmns_db.display()))?;

    // ponytail: read_blob reads each file into memory to hash it; a streaming hash when an input outgrows the memory
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    names.sort();
    let mut files = Vec::new();
    for n in names {
        let h: String = con
            .query_row(&format!("SELECT sha256(content) FROM read_blob('{}')", sql_str(&dir.join(&n))), [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        files.push((n, h));
    }
    let area = db.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let opt = |v: &Option<String>| v.as_deref().map(|s| format!("\"{}\"", esc(s))).unwrap_or_else(|| "null".into());
    let list: Vec<String> = files.iter().map(|(n, h)| format!("    \"{}\": \"{h}\"", esc(n))).collect();
    let json = format!(
        "{{\n  \"contract\": {CONTRACT},\n  \"area\": \"{}\",\n  \"duckosm_version\": \"{}\",\n  \"ground_id\": \"{}\",\n  \
         \"osm_timestamp\": {},\n  \"jurisdiction\": {},\n  \"driving_side\": {},\n  \"files\": {{\n{}\n  }}\n}}\n",
        esc(&area),
        esc(&version),
        esc(&ground),
        opt(&osm_time),
        opt(&area_opts.jurisdiction),
        opt(&area_opts.driving_side),
        list.join(",\n")
    );
    std::fs::write(dir.join("input.json"), json).map_err(|e| e.to_string())?;
    Ok(files)
}

/// `duckosm hdmap DB GMNS_DB`: the input folder next to `out`, then `duckosm-sim build`. The folder goes away after a run
/// unless `keep_input`.
pub fn run_hdmap(
    db: &Path,
    gmns_db: &Path,
    out: &Path,
    formats: &[String],
    simready: Option<&Path>,
    keep_input: bool,
    area_opts: &HdmapArea,
) -> Result<HdmapSummary> {
    for p in [db, gmns_db] {
        if !p.is_file() {
            return Err(format!("{}: no such file", p.display()));
        }
    }
    let sim = find_simready(simready)?; // before the input is written: stop early when SimReady is not there
    let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "hdmap".into());
    let input = out.with_file_name(format!("{stem}_input"));
    let files = write_input(db, gmns_db, &input, area_opts)?;
    let mut cmd = std::process::Command::new(&sim);
    cmd.arg("build").arg("--input").arg(&input).arg("--out").arg(out);
    if !formats.is_empty() {
        cmd.arg("--formats").arg(formats.join(","));
    }
    let status = cmd.status().map_err(|e| format!("{}: {e}", sim.display()))?;
    if !keep_input {
        let _ = std::fs::remove_dir_all(&input);
    }
    if !status.success() {
        return Err(format!("SimReady stopped with {status}"));
    }
    Ok(HdmapSummary { input, out: out.to_path_buf(), files })
}
