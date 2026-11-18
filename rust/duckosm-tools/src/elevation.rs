//! `elevation`: sample a DEM at every node and every edge vertex of a built database and write the heights in place
//! (duckOSM `elevation.py`). It adds `<mode>.nodes.ele`, `<mode>.edges.z_from`, `z_to` and `z_vertices`, and one row of
//! `main.elevation_metadata` for each height column. `--suffix dsm` writes `ele_dsm`, ... next to `ele`.
//!
//! The DEM is a GeoTIFF in EPSG:4326 with 32-bit float values (the reader of the build, `duckosm_build::elevation::Dem`),
//! a local file or an HTTP address. `--source` downloads a global model with DuckDB's httpfs: the 1° tiles of Copernicus
//! GLO-30 (AWS, no account), or EU-DTM from OpenTopography (`OPENTOPOGRAPHY_API_KEY`). The ground rule
//! (docs/concepts/ground-reference.md): a `--dem` file in `ele` is the area's reference ground, so it needs its product,
//! vertical datum and license, or `--not-reference`.

use crate::Result;
use duckdb::Connection;
use duckosm_build::elevation::{round_cm, sha256_file, Dem};
use std::collections::HashMap;
use std::path::Path;

pub struct Options {
    pub dem: Option<String>,
    pub source: String,
    pub modes: Vec<String>,
    pub nodata_fill: f64,
    pub suffix: String,
    pub product: Option<String>,
    pub vertical_datum: Option<String>,
    pub license: Option<String>,
    pub reference: bool,
}

#[derive(Debug)]
/// What the command did: the source, the nodes and the nodes with no value of each mode, the column, the totals.
pub struct Stats {
    pub source: String,
    pub modes: Vec<(String, usize, usize)>,
    pub column: String,
    pub n_nodes: usize,
    pub n_nodata: usize,
}

/// A global model that `--source` downloads, with the values of `main.elevation_metadata` (duckOSM `PROVIDERS`).
struct Provider {
    name: &'static str,
    /// The entry of the catalog (dem_sources.json): its coverage.
    catalog_id: &'static str,
    source: &'static str,
    uri: &'static str,
    product: &'static str,
    vertical_datum: &'static str,
    license: &'static str,
    priority: i32,
}

const PROVIDERS: [Provider; 2] = [
    Provider {
        name: "copernicus",
        catalog_id: "copernicus-glo30",
        source: "Copernicus GLO-30",
        uri: "s3://copernicus-dem-30m",
        product: "DSM",
        vertical_datum: "EGM2008",
        license: "Copernicus open (attribution)",
        priority: 0,
    },
    Provider {
        name: "eudtm",
        catalog_id: "eudtm",
        source: "Continental Europe DTM 30m (EU_DTM)",
        uri: "https://portal.opentopography.org/API/globaldem?demtype=EU_DTM",
        product: "DTM",
        vertical_datum: "EGM2008",
        license: "EU-DTM via OpenTopography (see provider terms)",
        priority: 10,
    },
];

const COP_BASE: &str = "https://copernicus-dem-30m.s3.amazonaws.com";
const OPENTOPO_API: &str = "https://portal.opentopography.org/API/globaldem";

/// One row of `main.elevation_metadata` without the counts and the time.
struct Meta {
    source: String,
    source_type: &'static str,
    uri: String,
    resolution_m: f64,
    product: String,
    dem_crs: String,
    vertical_datum: String,
    license: String,
    sha256: Option<String>,
}

/// Where the heights come from: one raster, or the 1° tiles of Copernicus GLO-30 (downloaded once, when a point needs them).
enum Sampler {
    File(Dem),
    Tiles { tiles: HashMap<(i32, i32), Option<Dem>>, bbox: [f64; 4] },
}

impl Sampler {
    /// The height at a point, or None outside the DEM or on nodata.
    fn sample(&mut self, con: &Connection, lon: f64, lat: f64) -> Option<f64> {
        match self {
            Sampler::File(d) => d.sample(lon, lat),
            Sampler::Tiles { tiles, bbox } => {
                let key = (lat.floor() as i32, lon.floor() as i32);
                tiles.entry(key).or_insert_with(|| copernicus_tile(con, key, *bbox)).as_ref()?.sample(lon, lat)
            }
        }
    }
}

/// The tile of Copernicus GLO-30 whose south-west corner is (lat, lon), with the pixels over `bbox` only. None when the
/// tile does not exist (sea) or cannot be read; then its points get the fill value, as in duckOSM.
fn copernicus_tile(con: &Connection, (lat, lon): (i32, i32), bbox: [f64; 4]) -> Option<Dem> {
    let name = format!(
        "Copernicus_DSM_COG_10_{}{:02}_00_{}{:03}_00_DEM",
        if lat >= 0 { 'N' } else { 'S' },
        lat.abs(),
        if lon >= 0 { 'E' } else { 'W' },
        lon.abs()
    );
    let url = format!("{COP_BASE}/{name}/{name}.tif");
    match download(con, &url).and_then(|b| Dem::window(std::io::Cursor::new(b), &url, bbox, 1)) {
        Ok(d) => Some(d),
        Err(e) => {
            eprintln!("  copernicus tile missing at {lat},{lon} (ocean/void?) — its nodes get nodata_fill ({e})");
            None
        }
    }
}

/// The bytes at an HTTP address, with DuckDB's httpfs.
fn download(con: &Connection, url: &str) -> Result<Vec<u8>> {
    con.execute_batch("INSTALL httpfs; LOAD httpfs;").map_err(|e| format!("DuckDB's httpfs: {e}"))?;
    con.query_row(&format!("SELECT content FROM read_blob('{}')", url.replace('\'', "''")), [], |r| r.get(0))
        .map_err(|e| format!("{url}: {e}"))
}

/// Python's repr of a list of strings: `['a', 'b']`.
fn py_list(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", "))
}

fn sql<T: duckdb::types::FromSql>(con: &Connection, q: &str) -> Result<Vec<T>> {
    let mut st = con.prepare(q).map_err(|e| e.to_string())?;
    let rows = st.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
    rows.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())
}

/// The mode schemas to write (duckOSM `_mode_schemas`): `modes`, or every schema with an `edges` table, by name.
fn mode_schemas(con: &Connection, modes: &[String]) -> Result<Vec<String>> {
    let present: Vec<String> = sql(
        con,
        "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name = 'edges' \
         AND schema_name NOT IN ('information_schema', 'pg_catalog', 'main', 'raw') ORDER BY schema_name",
    )?;
    let chosen = if modes.is_empty() { present.clone() } else { modes.to_vec() };
    if chosen.is_empty() {
        return Err("no mode schemas with an 'edges' table in this db".into());
    }
    let unknown: Vec<String> = chosen.iter().filter(|m| !present.contains(m)).cloned().collect();
    if !unknown.is_empty() {
        let have = if present.is_empty() { "none".to_string() } else { py_list(&present) };
        return Err(format!("mode(s) {} not found (present: {have})", py_list(&unknown)));
    }
    Ok(chosen)
}

/// The box (w, s, e, n) of the nodes of the modes (duckOSM `_db_bbox`).
fn nodes_bbox(con: &Connection, modes: &[String]) -> Result<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for m in modes {
        let r: (Option<f64>, Option<f64>, Option<f64>, Option<f64>) = con
            .query_row(
                &format!("SELECT min(ST_X(geom)), min(ST_Y(geom)), max(ST_X(geom)), max(ST_Y(geom)) FROM {m}.nodes WHERE geom IS NOT NULL"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(|e| e.to_string())?;
        if let (Some(w), Some(s), Some(e), Some(n)) = r {
            b = Some(b.map_or([w, s, e, n], |b| [b[0].min(w), b[1].min(s), b[2].max(e), b[3].max(n)]));
        }
    }
    b.ok_or_else(|| "no node geometry to sample".to_string())
}

/// The box of the nodes and of the edge vertices: the pixels that the DEM keeps in memory.
fn points_bbox(con: &Connection, modes: &[String], geo: &HashMap<String, String>, nodes: [f64; 4]) -> Result<[f64; 4]> {
    let mut b = nodes;
    for m in modes {
        let g = &geo[m];
        let r: (Option<f64>, Option<f64>, Option<f64>, Option<f64>) = con
            .query_row(&format!("SELECT min(ST_XMin({g})), min(ST_YMin({g})), max(ST_XMax({g})), max(ST_YMax({g})) FROM {m}.edges"), [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(|e| e.to_string())?;
        if let (Some(w), Some(s), Some(e), Some(n)) = r {
            b = [b[0].min(w), b[1].min(s), b[2].max(e), b[3].max(n)];
        }
    }
    Ok(b)
}

/// The provider of `--source` (duckOSM `_resolve_provider`): `auto` is the provider of the highest priority that covers
/// the box and can be used (EU-DTM needs an API key); Copernicus covers the world.
fn resolve_provider(source: &str, bbox: [f64; 4]) -> Result<&'static Provider> {
    if source != "auto" {
        return PROVIDERS.iter().find(|p| p.name == source).ok_or_else(|| {
            format!("unknown --source '{source}'; available: auto, {} (or pass --dem <file>)", PROVIDERS.map(|p| p.name).join(", "))
        });
    }
    let usable = |p: &Provider| p.name != "eudtm" || std::env::var("OPENTOPOGRAPHY_API_KEY").is_ok_and(|k| !k.is_empty());
    let mut by_priority: Vec<&Provider> = PROVIDERS.iter().collect();
    by_priority.sort_by_key(|p| -p.priority);
    Ok(by_priority
        .into_iter()
        .find(|p| crate::dem_sources::get(p.catalog_id).is_some_and(|s| crate::dem_sources::covers(s, bbox)) && usable(p))
        .unwrap_or(&PROVIDERS[0]))
}

/// The sampler and the metadata of `--dem` or of a provider. `window`: the box of all the points.
fn make_sampler(con: &Connection, o: &Options, provider: Option<&Provider>, nodes: [f64; 4], window: [f64; 4]) -> Result<(Sampler, Meta)> {
    if let Some(dem) = &o.dem {
        let remote = dem.contains("://");
        let d = if remote {
            Dem::window(std::io::Cursor::new(download(con, dem)?), dem, window, 1)?
        } else {
            Dem::open_window(Path::new(dem), window, 1)?
        };
        let meta = Meta {
            source: Path::new(dem).file_name().map_or(dem.clone(), |f| f.to_string_lossy().into_owned()),
            source_type: "file",
            uri: if remote { dem.clone() } else { std::fs::canonicalize(dem).map_or(dem.clone(), |p| p.to_string_lossy().into_owned()) },
            resolution_m: d.resolution_m,
            product: o.product.clone().unwrap_or_else(|| "unknown".into()),
            dem_crs: "EPSG:4326".into(),
            vertical_datum: o.vertical_datum.clone().unwrap_or_else(|| "unknown".into()),
            license: o.license.clone().unwrap_or_else(|| "unknown".into()),
            sha256: if remote { None } else { Some(sha256_file(Path::new(dem))?) },
        };
        return Ok((Sampler::File(d), meta));
    }
    let p = provider.expect("a provider when there is no --dem");
    let meta = |sha256| Meta {
        source: p.source.into(),
        source_type: "download",
        uri: p.uri.into(),
        resolution_m: 30.0,
        product: p.product.into(),
        dem_crs: "EPSG:4326".into(),
        vertical_datum: p.vertical_datum.into(),
        license: p.license.into(),
        sha256,
    };
    if p.name == "eudtm" {
        // a GeoTIFF of the box (plus 0.02°) from OpenTopography's Global DEM API
        let key = std::env::var("OPENTOPOGRAPHY_API_KEY").map_err(|_| "eudtm needs the OPENTOPOGRAPHY_API_KEY env var")?;
        let [w, s, e, n] = nodes;
        let pad = 0.02;
        let url = format!(
            "{OPENTOPO_API}?demtype=EU_DTM&south={}&north={}&west={}&east={}&outputFormat=GTiff&API_Key={key}",
            s - pad,
            n + pad,
            w - pad,
            e + pad
        );
        eprintln!("  fetching EU_DTM from OpenTopography for bbox {w:.2},{s:.2},{e:.2},{n:.2} …");
        let bytes = download(con, &url).map_err(|e| e.replace(&key, "***"))?;
        let hash: String = con.query_row("SELECT sha256(?::BLOB)", [&bytes], |r| r.get(0)).map_err(|e| e.to_string())?;
        let d = Dem::window(std::io::Cursor::new(bytes), "EU_DTM", window, 1)
            .map_err(|e| format!("eudtm: OpenTopography returned no usable raster for this bbox (check API key / coverage) — {e}"))?;
        return Ok((Sampler::File(d), meta(Some(hash))));
    }
    // the streamed tiles: no whole file is read, so there is no ground id
    Ok((Sampler::Tiles { tiles: HashMap::new(), bbox: window }, meta(None)))
}

/// Add the heights to the database `db` in place (duckOSM `to_elevation`).
pub fn to_elevation(db: &Path, o: &Options) -> Result<Stats> {
    if let Some(p) = o.product.as_deref().filter(|p| !matches!(*p, "DTM" | "DSM")) {
        return Err(format!("the DEM product must be DTM (bare earth) or DSM (surface), not {p}"));
    }
    if o.dem.is_some() && o.suffix.is_empty() && o.reference && !(o.product.is_some() && o.vertical_datum.is_some() && o.license.is_some()) {
        return Err("the DEM is the area's reference ground: give its product, its vertical datum and its license (--product, \
                    --vertical-datum, --license), or mark it as not the reference (--not-reference)"
            .into());
    }
    // the suffix goes into the SQL, so it must be a name
    let suffix = o.suffix.trim().trim_start_matches('_');
    let ok = suffix.is_empty()
        || (suffix.starts_with(|c: char| c.is_ascii_alphabetic()) && suffix.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    if !ok {
        return Err(format!("--suffix must be a bare identifier (letters/digits/_), got '{suffix}'"));
    }
    let sfx = if suffix.is_empty() { String::new() } else { format!("_{suffix}") };
    let (ele, zf, zt, zv) = (format!("ele{sfx}"), format!("z_from{sfx}"), format!("z_to{sfx}"), format!("z_vertices{sfx}"));

    let con = Connection::open(db).map_err(|e| format!("{}: {e}", db.display()))?;
    con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;").map_err(|e| format!("the spatial extension of DuckDB: {e}"))?;
    let chosen = mode_schemas(&con, &o.modes)?;
    let mut geo = HashMap::new();
    for m in &chosen {
        let n: i64 = con
            .query_row(
                "SELECT count(*) FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'edges' AND column_name = 'geometry'",
                [m],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        geo.insert(m.clone(), if n > 0 { "geometry" } else { "geom" }.to_string());
    }
    let bbox = nodes_bbox(&con, &chosen)?;
    let provider = match o.dem {
        Some(_) => None,
        None => {
            let p = resolve_provider(&o.source, bbox)?;
            if o.source == "auto" {
                eprintln!("  source=auto → {} (bbox {})", p.name, bbox.map(|v| format!("{v:.2}")).join(", "));
            }
            Some(p)
        }
    };
    let window = points_bbox(&con, &chosen, &geo, bbox)?;
    let (mut sampler, meta) = make_sampler(&con, o, provider, bbox, window)?;
    let fill = o.nodata_fill;
    let err = |e: duckdb::Error| e.to_string();

    let mut stats = Stats { source: meta.source.clone(), modes: Vec::new(), column: ele.clone(), n_nodes: 0, n_nodata: 0 };
    for m in &chosen {
        let nodes: Vec<(i64, f64, f64)> = {
            let mut st = con.prepare(&format!("SELECT node_id, ST_X(geom), ST_Y(geom) FROM {m}.nodes WHERE geom IS NOT NULL")).map_err(err)?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(err)?;
            rows.collect::<std::result::Result<_, _>>().map_err(err)?
        };
        if nodes.is_empty() {
            stats.modes.push((m.clone(), 0, 0));
            continue;
        }
        let mut nodata = 0;
        // ADD IF NOT EXISTS: DuckDB stops a DROP or an ALTER of a column of a table with an index, but allows ADD
        con.execute_batch(&format!(
            "ALTER TABLE {m}.nodes ADD COLUMN IF NOT EXISTS {ele} DOUBLE; CREATE OR REPLACE TEMP TABLE _ele(node_id BIGINT, ele DOUBLE);"
        ))
        .map_err(err)?;
        {
            let mut app = con.appender_to_catalog_and_db("_ele", "temp", "main").map_err(err)?;
            for &(id, x, y) in &nodes {
                let z = sampler.sample(&con, x, y).unwrap_or_else(|| {
                    nodata += 1;
                    fill
                });
                // to the cm: a DEM is accurate to about a metre
                app.append_row(duckdb::params![id, round_cm(z)]).map_err(err)?;
            }
        }
        con.execute_batch(&format!(
            "UPDATE {m}.nodes n SET {ele} = (SELECT ele FROM _ele t WHERE t.node_id = n.node_id);
             ALTER TABLE {m}.edges ADD COLUMN IF NOT EXISTS {zf} DOUBLE;
             ALTER TABLE {m}.edges ADD COLUMN IF NOT EXISTS {zt} DOUBLE;
             UPDATE {m}.edges e SET {zf} = (SELECT {ele} FROM {m}.nodes n WHERE n.node_id = e.source), \
             {zt} = (SELECT {ele} FROM {m}.nodes n WHERE n.node_id = e.target);"
        ))
        .map_err(err)?;

        // the heights at the vertices, from the text of the geometry as duckOSM reads them
        let g = &geo[m];
        let edges: Vec<(i64, String)> = {
            let mut st = con.prepare(&format!("SELECT edge_id, ST_AsText({g}) FROM {m}.edges WHERE {g} IS NOT NULL")).map_err(err)?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
            rows.collect::<std::result::Result<_, _>>().map_err(err)?
        };
        if !edges.is_empty() {
            con.execute_batch("CREATE OR REPLACE TEMP TABLE _zv_flat(edge_id BIGINT, i INTEGER, z DOUBLE);").map_err(err)?;
            {
                let mut app = con.appender_to_catalog_and_db("_zv_flat", "temp", "main").map_err(err)?;
                for (id, wkt) in &edges {
                    let (a, b) = (wkt.find('(').map_or(0, |i| i + 1), wkt.rfind(')').unwrap_or(wkt.len()));
                    for (i, pair) in wkt[a..b].split(',').enumerate() {
                        let mut xy = pair.split_whitespace().map(|v| v.parse::<f64>());
                        let (Some(Ok(x)), Some(Ok(y))) = (xy.next(), xy.next()) else {
                            return Err(format!("{m}.edges {id}: a vertex that is not 'x y': {pair}"));
                        };
                        let z = round_cm(sampler.sample(&con, x, y).unwrap_or(fill));
                        app.append_row(duckdb::params![id, i as i32, z]).map_err(err)?;
                    }
                }
            }
            con.execute_batch(&format!(
                "ALTER TABLE {m}.edges ADD COLUMN IF NOT EXISTS {zv} DOUBLE[];
                 CREATE OR REPLACE TEMP TABLE _zv AS SELECT edge_id, list(z ORDER BY i) AS z FROM _zv_flat GROUP BY edge_id;"
            ))
            .map_err(err)?;
            // DuckDB updates a list column as a delete and an insert, and then updates an R-tree index row by row: 64 s for
            // the cycling edges of Tartu, 10 s without the index. So the R-tree indexes go before the update and come back
            // after it (from their own SQL), also when the update fails.
            let rtree: Vec<(String, String)> = {
                let mut st = con
                    .prepare("SELECT index_name, sql FROM duckdb_indexes() WHERE schema_name = ? AND table_name = 'edges' AND sql ILIKE '%USING RTREE%'")
                    .map_err(err)?;
                let rows = st.query_map([m], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
                rows.collect::<std::result::Result<_, _>>().map_err(err)?
            };
            for (name, _) in &rtree {
                con.execute_batch(&format!("DROP INDEX {m}.{name}")).map_err(err)?;
            }
            let updated = con.execute_batch(&format!("UPDATE {m}.edges e SET {zv} = (SELECT z FROM _zv t WHERE t.edge_id = e.edge_id)"));
            for (_, sql) in &rtree {
                con.execute_batch(sql).map_err(err)?;
            }
            updated.map_err(err)?;
        }
        eprintln!("  [{m}] {} nodes sampled ({nodata} nodata/fill) -> {ele}", nodes.len());
        stats.modes.push((m.clone(), nodes.len(), nodata));
        stats.n_nodes += nodes.len();
        stats.n_nodata += nodata;
    }
    write_metadata(&con, &meta, fill, stats.n_nodes, stats.n_nodata, &ele)?;
    con.execute_batch("CHECKPOINT").map_err(err)?;
    Ok(stats)
}

/// One row of `main.elevation_metadata` for each height column (duckOSM `_write_metadata`): a new run of `ele` replaces its
/// row and keeps the row of `ele_dsm`.
fn write_metadata(con: &Connection, m: &Meta, fill: f64, n_nodes: usize, n_nodata: usize, column: &str) -> Result<()> {
    con.execute_batch(
        "CREATE TABLE IF NOT EXISTS main.elevation_metadata(
            ele_column VARCHAR, source VARCHAR, source_type VARCHAR, uri VARCHAR,
            resolution_m DOUBLE, product VARCHAR, dem_crs VARCHAR, vertical_datum VARCHAR,
            license VARCHAR, nodata_fill DOUBLE, n_nodes BIGINT, n_nodata BIGINT,
            sampled_at VARCHAR, duckosm_version VARCHAR, sha256 VARCHAR);
         ALTER TABLE main.elevation_metadata ADD COLUMN IF NOT EXISTS ele_column VARCHAR;
         ALTER TABLE main.elevation_metadata ADD COLUMN IF NOT EXISTS sha256 VARCHAR;
         UPDATE main.elevation_metadata SET ele_column = 'ele' WHERE ele_column IS NULL;",
    )
    .map_err(|e| e.to_string())?;
    con.execute("DELETE FROM main.elevation_metadata WHERE ele_column = ?", [column]).map_err(|e| e.to_string())?;
    con.execute(
        "INSERT INTO main.elevation_metadata
            (ele_column, source, source_type, uri, resolution_m, product, dem_crs, vertical_datum,
             license, nodata_fill, n_nodes, n_nodata, sampled_at, duckosm_version, sha256)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?, strftime(make_timestamp(?), '%Y-%m-%dT%H:%M:%S+00:00'), ?, ?)",
        duckdb::params![
            column,
            m.source,
            m.source_type,
            m.uri,
            m.resolution_m,
            m.product,
            m.dem_crs,
            m.vertical_datum,
            m.license,
            fill,
            n_nodes as i64,
            n_nodata as i64,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as i64),
            concat!("duckosm-rust ", env!("CARGO_PKG_VERSION")),
            m.sha256,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
