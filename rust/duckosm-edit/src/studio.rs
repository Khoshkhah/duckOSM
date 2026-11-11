//! `duckosm studio`: a page on 127.0.0.1 that makes the data of an area, step by step: the area, the OSM data, the settings, the ground (the
//! DEM), the build, and the result. The result is the page of `duckosm edit` (roadstyle's dashboard with the editor), served at `/edit/`: this
//! module has no copy of it. The steps are parts in `web/` (`build_steps/`, `ground_step/`), which duckOSM Studio (the browser app) loads too.
//!
//! The server follows the rules of `lib.rs` (`read_request`): only 127.0.0.1, this server's name in `Host`, a write from this server's page.
//! The files that it writes are in the folder of the area, `<out>/<name>/`: `boundary.geojson`, `dem.tif`, `config.yaml`, the PBF file of a
//! download, and the `<name>.duckdb` of the build. A local PBF file is read from the path that the page gives (the page is on the same computer).
//!
//! The routes: `GET /` the page (roadstyle's dashboard, made by `page::make_studio`); `GET /api/style/<name>` and `GET /tiles/<name>/...` the map of the open area (its tiled style and tiles); `GET /parts/<file>` the parts; `GET /api/info`; `GET /api/geofabrik` the list of the Geofabrik extracts; `GET /api/areas` the saved areas and `GET /api/area/<name>` the boundary and the config of one; `POST /api/folders` (the folders inside a folder, for the folder picker) and `POST /api/folder` (use another folder for the areas); `POST /api/buffer` (an area and a distance: the area grown by it); `POST /api/open` (the name of a saved area: its result page at `/edit/`); `POST /api/delete` (the name of a saved area: its folder is deleted); `POST /api/ground` (a built area with a DEM, or with none: it is built again); `GET /api/terrain/<name>` (the terrain file of the area's ground); `POST /api/file` (the path of a local PBF file: its size); `GET /api/pbf` and `POST /api/pbfget` (the OSM extracts in `<out>/osm/`: listed, downloaded); `POST /api/demfiles` and `POST /api/demget` (the DEM files of a model for a box: listed, downloaded);
//! `PUT /api/dem/<name>` the DEM of the area; `POST /api/build` start the build; `GET /api/job` the state of the build; `GET /edit/...` the
//! page of `duckosm edit` for the area that was built last.

use crate::{read_request, Db, Mode, Pager, Req, Site};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What the build needs of the program: the native program has DuckDB and the engine (`native::NativeHost`); the Python module gives the
/// build of `duckosm build` (src/duckosm/studio.py).
pub trait Host: Send + Sync {
    /// The database of the finished area, for the page of the result.
    fn db(&self, path: &Path) -> Box<dyn Db>;
    /// Build the area from `config` (YAML) in the folder `dir` (the config's relative paths) into the database `db`. `log` gets a line for each
    /// step that ends. Gives the summary of the build. The DEM is in the config (`elevation:`).
    fn build(&self, config: &str, dir: &Path, db: &Path, log: &(dyn Fn(&str) + Sync)) -> Result<String, String>;
    /// The buffer of an area: `wkt` is a MULTIPOLYGON in EPSG:4326; it grows by `metres` in the UTM zone `epsg`. Gives the GeoJSON geometry (text) in EPSG:4326.
    fn buffer(&self, _wkt: &str, _epsg: u32, _metres: f64) -> Result<String, String> {
        Err("this program cannot buffer an area".into())
    }
}

/// The files of the page: its own, the parts of this repository's `web/`, and the catalog of the DEM sources.
const PARTS: &[(&str, &str, &str)] = &[
    ("ground_step/ground_step.js", "text/javascript", include_str!("../../../web/ground_step/ground_step.js")),
    ("ground_step/ground_core.js", "text/javascript", include_str!("../../../web/ground_step/ground_core.js")),
    ("ground_step/ground_step.css", "text/css", include_str!("../../../web/ground_step/ground_step.css")),
    ("build_steps/core.js", "text/javascript", include_str!("../../../web/build_steps/core.js")),
    ("build_steps/area_step.js", "text/javascript", include_str!("../../../web/build_steps/area_step.js")),
    ("build_steps/data_step.js", "text/javascript", include_str!("../../../web/build_steps/data_step.js")),
    ("build_steps/settings_step.js", "text/javascript", include_str!("../../../web/build_steps/settings_step.js")),
    ("build_steps/build_step.js", "text/javascript", include_str!("../../../web/build_steps/build_step.js")),
    ("build_steps/result_step.js", "text/javascript", include_str!("../../../web/build_steps/result_step.js")),
    ("build_steps/visualization_step.js", "text/javascript", include_str!("../../../web/build_steps/visualization_step.js")),
    ("build_steps/folder_step.js", "text/javascript", include_str!("../../../web/build_steps/folder_step.js")),
    ("build_steps/steps.css", "text/css", include_str!("../../../web/build_steps/steps.css")),
    ("data/dem_sources.json", "application/json", include_str!("../../../src/duckosm/data/dem_sources.json")),
];
/// The default list of the extracts. A mirror or a file can take its place (`--geofabrik-index`).
pub const GEOFABRIK_INDEX: &str = "https://download.geofabrik.de/index-v1.json";

/// The state of the build, as the page reads it (`GET /api/job`).
#[derive(Default)]
struct Job {
    /// `idle`, `downloading`, `building`, `done` or `error`.
    state: &'static str,
    name: String,
    /// The database file of the area, and the folder of the area with the files that the build made (when it is done).
    db: String,
    dir: String,
    files: Vec<(String, u64)>,
    steps: Vec<String>,
    /// The seconds since the start of the build at which each step began (the page shows the time of each step).
    step_secs: Vec<f64>,
    bytes: u64,
    total: u64,
    text: String,
    error: String,
    started: Option<Instant>,
    /// When the build ended (well or not): the time of the build stops there.
    ended: Option<Instant>,
}

impl Job {
    fn json(&self) -> Value {
        json!({"state": if self.state.is_empty() { "idle" } else { self.state }, "name": self.name, "db": self.db, "dir": self.dir, "files": self.files.iter().map(|(n, b)| json!({"name": n, "size": b})).collect::<Vec<_>>(), "steps": self.steps, "step_secs": self.step_secs, "bytes": self.bytes,
               "total": self.total, "text": self.text, "error": self.error, "seconds": self.started.map_or(0.0, |t| self.ended.unwrap_or_else(Instant::now).duration_since(t).as_secs_f64())})
    }
}

/// One server: the folder of the areas, the program's host, the build, and the page of the area that was built last.
pub struct Studio {
    /// The folder of the areas. The person can change it on the page (`POST /api/folder`).
    out: Mutex<PathBuf>,
    pub own: [String; 2],
    port: u16,
    host: Box<dyn Host>,
    /// Where the list of Geofabrik extracts comes from: a URL or a file.
    index: String,
    index_text: Mutex<Option<String>>,
    /// The page: roadstyle's dashboard with the script of the steps (`page::make_studio`), made once.
    page: String,
    job: Mutex<Job>,
    /// The download of an OSM extract (step 3), apart from the build: `name` is the file, `dir` its path when it is whole.
    dl: Mutex<Job>,
    edit: Mutex<Option<Arc<Site>>>,
    /// The name of the area whose map page is made now (Open makes it in the background), or empty.
    opening: Mutex<String>,
    /// The area that is open now (the last Open or build), or empty: the page of the steps shows it again when it comes back from the map.
    current: Mutex<String>,
    /// The map of the open area: its name, its tiled style and its tiles (`page::make_tiled`). The page puts the style on its map, and asks this server for the tiles.
    tiled: Mutex<Option<(String, String, Arc<roadstyle::tiles::Tiles>)>>,
    pager: Pager,
}

/// The date `back` days ago, UTC, as Geofabrik writes it in the name of a dated file: yymmdd.
fn yymmdd(back: u64) -> String {
    let days = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) / 86400).saturating_sub(back) as i64;
    // the civil date from the days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719468;
    let (era, doe) = (z.div_euclid(146097), z.rem_euclid(146097));
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let (doy, y) = (doe - (365 * yoe + yoe / 4 - yoe / 100), yoe + era * 400);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    format!("{:02}{:02}{:02}", (if m <= 2 { y + 1 } else { y }) % 100, m, d)
}

/// A name of an area is a folder name: letters, digits, `_`, `-` and `.`, not starting with a dot.
fn is_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && !n.starts_with('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// The files of a DEM model that the server can download for a box, as (address, file name): one file for each 1° tile of Copernicus GLO-30 and
/// GLO-90 (AWS, no account), or one cut of the box for a model in `CUT_MODELS` (the address is the STAC search). The others have only their web page.
pub fn dem_tiles(source: &str, bbox: [f64; 4]) -> Vec<(String, String)> {
    if let Some(m) = CUT_MODELS.iter().find(|m| m.0 == source) {
        let b = bbox.map(|v| (v * 10_000.0).round() / 10_000.0);
        return vec![(format!("{}&bbox={},{},{},{}", m.1, b[0], b[1], b[2], b[3]), cut_file(source, b))];
    }
    let (bucket, code) = match source {
        "copernicus-glo30" => ("copernicus-dem-30m", "10"),
        "copernicus-glo90" => ("copernicus-dem-90m", "30"),
        _ => return Vec::new(),
    };
    let span = |a: f64, b: f64| (a.floor() as i32)..=((b.ceil() as i32 - 1).max(a.floor() as i32));
    let mut out = Vec::new();
    for lat in span(bbox[1], bbox[3]) {
        for lon in span(bbox[0], bbox[2]) {
            let name = format!("Copernicus_DSM_COG_{code}_{}{:02}_00_{}{:03}_00_DEM", if lat < 0 { 'S' } else { 'N' }, lat.abs(), if lon < 0 { 'W' } else { 'E' }, lon.abs());
            out.push((format!("https://{bucket}.s3.amazonaws.com/{name}/{name}.tif"), format!("{name}.tif")));
        }
    }
    out
}

/// The files of an area's folder and of its folders (the cut PBF in `pbf/`, the drawing levels in `<name>.levels/`), with their path in the folder, sorted.
fn folder_files(dir: &Path) -> Vec<(String, u64)> {
    let mut files: Vec<(String, u64)> = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        match e.metadata() {
            Ok(m) if m.is_file() => files.push((n, m.len())),
            Ok(m) if m.is_dir() => files.extend(std::fs::read_dir(e.path()).into_iter().flatten().flatten().filter_map(|f| {
                f.metadata().ok().filter(|m| m.is_file()).map(|m| (format!("{n}/{}", f.file_name().to_string_lossy()), m.len()))
            })),
            _ => {}
        }
    }
    files.sort();
    files
}

/// A file at an HTTP address, read by parts (range requests) as a file on disk: a cloud-optimized GeoTIFF gives only the parts that a reader
/// asks for. The parts are kept in blocks of 1 MB. ponytail: the blocks stay in memory for one cut; a reader of a whole large file needs a limit.
pub struct HttpFile {
    url: String,
    len: u64,
    pos: u64,
    blocks: std::collections::HashMap<u64, Vec<u8>>,
}

impl HttpFile {
    const BLOCK: u64 = 1 << 20;

    pub fn open(url: &str) -> Result<HttpFile, String> {
        let h = ureq::head(url).call().map_err(|e| format!("{url}: {e}"))?;
        let len = h.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).ok_or_else(|| format!("{url}: no size"))?;
        Ok(HttpFile { url: url.to_string(), len, pos: 0, blocks: Default::default() })
    }

    fn block(&mut self, i: u64) -> std::io::Result<&Vec<u8>> {
        if !self.blocks.contains_key(&i) {
            let (from, last) = (i * Self::BLOCK, ((i + 1) * Self::BLOCK).min(self.len) - 1);
            let mut resp = ureq::get(&self.url).header("Range", format!("bytes={from}-{last}")).call().map_err(std::io::Error::other)?;
            let body = resp.body_mut().with_config().limit(Self::BLOCK + 1).read_to_vec().map_err(std::io::Error::other)?;
            self.blocks.insert(i, body);
        }
        Ok(&self.blocks[&i])
    }
}

impl Read for HttpFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let (i, at) = (self.pos / Self::BLOCK, (self.pos % Self::BLOCK) as usize);
        let b = self.block(i)?;
        let n = (b.len().saturating_sub(at)).min(buf.len());
        buf[..n].copy_from_slice(&b[at..at + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl std::io::Seek for HttpFile {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.pos = match to {
            std::io::SeekFrom::Start(p) => p,
            std::io::SeekFrom::End(d) => (self.len as i64 + d).max(0) as u64,
            std::io::SeekFrom::Current(d) => (self.pos as i64 + d).max(0) as u64,
        };
        Ok(self.pos)
    }
}

/// The models whose part for an area the server cuts out of one large cloud file (a STAC catalog gives the file): the catalog's id, the STAC
/// search, the asset and the projection. The file name of a cut names its box.
const CUT_MODELS: [(&str, &str, &str, u32); 1] = [("ca-nrcan-hrdem", "https://datacube.services.geo.ca/stac/api/search?collections=hrdem-mosaic-1m", "dtm", 3979)];

/// The file of the cut of a model for a box (`<source>_<west>_<south>_<east>_<north>.tif`, 4 decimals).
fn cut_file(source: &str, b: [f64; 4]) -> String {
    let n = |v: f64| format!("{v:.4}").replace('-', "m");
    format!("{source}_{}_{}_{}_{}.tif", n(b[0]), n(b[1]), n(b[2]), n(b[3]))
}

/// The box [west, south, east, north] of all the positions of a GeoJSON value, or None when it has none.
pub fn bbox_of(v: &Value) -> Option<[f64; 4]> {
    fn walk(v: &Value, b: &mut [f64; 4]) {
        match v {
            Value::Array(a) if a.len() >= 2 && a[0].is_number() && a[1].is_number() => {
                let (x, y) = (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0));
                *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, b)),
            Value::Object(o) => o.iter().filter(|(k, _)| matches!(k.as_str(), "coordinates" | "geometry" | "features" | "geometries")).for_each(|(_, x)| walk(x, b)),
            _ => {}
        }
    }
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    walk(v, &mut b);
    b[0].is_finite().then_some(b)
}

/// A text as a YAML string (a JSON string is one).
fn q(s: &str) -> String {
    Value::String(s.into()).to_string()
}

/// The config of a build: the keys of the place (the name, the output folder, the input, the boundary and the DEM) and then the settings of
/// the page (the keys of the config of `duckosm build`, as text). `pbf`: the path of the PBF file; `dem`: the DEM's data (`product`,
/// `vertical_datum`, `license`, `not_reference`) when the folder has `dem.tif`.
pub fn config_text(name: &str, pbf: &str, dem: Option<&Value>, settings: &str) -> String {
    let mut t = format!("name: {}\noutput_path: .\nsource:\n  type: pbf\n  pbf_path: {}\nboundary:\n  path: boundary.geojson\n", q(name), q(pbf));
    if let Some(d) = dem {
        t.push_str("elevation:\n  dem: dem.tif\n");
        for (key, field) in [("product", "product"), ("vertical_datum", "vertical_datum"), ("license", "license")] {
            if let Some(v) = d[field].as_str().filter(|v| !v.is_empty()) {
                t.push_str(&format!("  {key}: {}\n", q(v)));
            }
        }
        t.push_str(&format!("  reference: {}\n", !d["not_reference"].as_bool().unwrap_or(false)));
    }
    t.push_str(settings);
    if !settings.ends_with('\n') {
        t.push('\n');
    }
    t
}

/// The parts of a config of `config_text`: the PBF path, and the settings (the keys after `name`, `output_path`, `source`, `boundary` and `elevation`).
pub fn config_parts(config: &str) -> (Option<String>, String) {
    let mut pbf = None;
    let (mut settings, mut skip) = (String::new(), false);
    for line in config.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            let key = line.split(':').next().unwrap_or_default();
            skip = matches!(key, "name" | "output_path" | "source" | "boundary" | "elevation");
        }
        if let Some(v) = line.trim_start().strip_prefix("pbf_path:").filter(|_| skip) {
            pbf = serde_json::from_str::<String>(v.trim()).ok().or_else(|| Some(v.trim().to_string()));
        }
        if !skip {
            settings.push_str(line);
            settings.push('\n');
        }
    }
    (pbf, settings)
}

impl Studio {
    /// A server for the folder `out` (it is made when it is not there). `index`: the list of the extracts (URL or file); None: Geofabrik's.
    pub fn new(host: Box<dyn Host>, out: &Path, port: u16, index: Option<String>) -> Result<Studio, String> {
        Studio::with_pager(host, out, port, index, crate::page::make_result)
    }

    /// As `new`, with the function that makes the page of the result (the tests give a placeholder).
    pub fn with_pager(host: Box<dyn Host>, out: &Path, port: u16, index: Option<String>, pager: Pager) -> Result<Studio, String> {
        std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
        let out = std::fs::canonicalize(out).map_err(|e| format!("{}: {e}", out.display()))?;
        Ok(Studio { out: Mutex::new(out), own: [format!("127.0.0.1:{port}"), format!("localhost:{port}")], port, host, index: index.unwrap_or_else(|| GEOFABRIK_INDEX.into()),
                    index_text: Mutex::new(None), page: crate::page::make_studio()?, job: Mutex::new(Job::default()), dl: Mutex::new(Job::default()), edit: Mutex::new(None), opening: Mutex::new(String::new()), current: Mutex::new(String::new()), tiled: Mutex::new(None), pager })
    }

    /// The folder of the areas now.
    pub fn out(&self) -> PathBuf {
        self.out.lock().unwrap().clone()
    }

    pub fn url(&self) -> String {
        format!("http://{}/", self.own[0])
    }

    /// The list of the Geofabrik extracts (GeoJSON, as `index-v1.json`): read once.
    fn geofabrik(&self) -> Result<String, String> {
        let mut cache = self.index_text.lock().unwrap();
        if cache.is_none() {
            let text = if self.index.starts_with("http://") || self.index.starts_with("https://") {
                ureq::get(&self.index).call().map_err(|e| format!("{}: {e}", self.index))?
                    .body_mut().with_config().limit(256 << 20).read_to_string().map_err(|e| format!("{}: {e}", self.index))?
            } else {
                std::fs::read_to_string(&self.index).map_err(|e| format!("{}: {e}", self.index))?
            };
            *cache = Some(text);
        }
        Ok(cache.clone().unwrap())
    }

    /// The file to download, as the page of Studio does it (www/app/regions.js): "<region>-latest.osm.pbf" is a redirect that Geofabrik answers after about 30 s each time, but
    /// the dated file of a day that is there answers in 0.5 s. So ask for the files of the last 5 days (UTC) at the same time, wait 1.5 s, and take the newest that has answered:
    /// it is what "latest" is, and all parts are of one version. If none has answered, "latest" itself (the redirect is followed). Gives (the URL to read, its size, ranges?).
    fn resolve(url: &str) -> Result<(String, u64, bool), String> {
        use ureq::ResponseExt;
        let quick = ureq::Agent::new_with_config(ureq::config::Config::builder().timeout_global(Some(std::time::Duration::from_millis(1500))).http_status_as_error(false).build());
        let info = |h: &ureq::http::Response<ureq::Body>| {
            let get = |n: &str| h.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
            (get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0u64), get("accept-ranges").is_some_and(|v| v.contains("bytes")))
        };
        if url.contains("-latest.") {
            let tries: Vec<_> = (0..5).map(|back| {
                let (agent, dated) = (quick.clone(), url.replacen("-latest.", &format!("-{}.", yymmdd(back)), 1));
                std::thread::spawn(move || agent.head(&dated).call().ok().filter(|h| h.status().as_u16() == 200).map(|h| (dated, h)))
            }).collect();
            for t in tries {
                if let Some((dated, h)) = t.join().ok().flatten() {
                    let (total, ranges) = info(&h);
                    return Ok((dated, total, ranges));
                }
            }
        }
        // a server that does not answer HEAD (a small mirror): one plain download, with no size known
        let Ok(h) = ureq::head(url).call() else {
            return Ok((url.to_string(), 0, false));
        };
        let (total, ranges) = info(&h);
        Ok((h.get_uri().to_string(), total, ranges))                   // the address after the redirect: every part asks the same file
    }

    /// Download `url` to `to` (kept as `<to>.part` until it is whole). The job has the bytes so far. A server that gives ranges and a file of 16 MB or more: 8 connections (as
    /// Studio), each with its own part of the file (Geofabrik gives about 0.5 MB/s to a connection). Else one connection.
    /// ponytail: a part that fails is tried again 3 times from where it stopped.
    fn download(&self, url: &str, to: &Path) -> Result<(), String> {
        const CONNECTIONS: u64 = 8;
        let (url, total, ranges) = Self::resolve(url)?;
        let url = url.as_str();
        {
            let mut j = self.dl.lock().unwrap();
            (j.bytes, j.total) = (0, total);
        }
        let part = PathBuf::from(format!("{}.part", to.display()));
        if ranges && total >= 16 << 20 {
            let file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
            file.set_len(total).map_err(|e| format!("{}: {e}", part.display()))?;
            drop(file);
            let n = CONNECTIONS.min(total / (4 << 20)).max(1);
            let step = total.div_ceil(n);
            std::thread::scope(|s| {
                let handles: Vec<_> = (0..n).map(|i| {
                    let (from, to_byte) = (i * step, ((i + 1) * step).min(total) - 1);
                    let part = &part;
                    s.spawn(move || self.fetch_range(url, part, from, to_byte))
                }).collect();
                handles.into_iter().try_for_each(|h| h.join().map_err(|_| "a download thread stopped".to_string())?)
            })?;
        } else {
            let mut resp = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
            let mut file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
            let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = reader.read(&mut buf).map_err(|e| format!("{url}: {e}"))?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
                self.dl.lock().unwrap().bytes += n as u64;
            }
        }
        std::fs::rename(&part, to).map_err(|e| format!("{}: {e}", to.display()))
    }

    /// The bytes `from` to `last` of `url`, written at the same place of the file `part`. A failure is tried again from the byte where it stopped.
    fn fetch_range(&self, url: &str, part: &Path, from: u64, last: u64) -> Result<(), String> {
        use std::io::{Seek, SeekFrom};
        let mut pos = from;
        let mut tries = 0;
        while pos <= last {
            let mut one = || -> Result<(), String> {
                let mut resp = ureq::get(url).header("Range", format!("bytes={pos}-{last}")).call().map_err(|e| format!("{url}: {e}"))?;
                if resp.status().as_u16() != 206 {
                    return Err(format!("{url}: the server did not give the range (HTTP {})", resp.status()));
                }
                let mut file = std::fs::OpenOptions::new().write(true).open(part).map_err(|e| format!("{}: {e}", part.display()))?;
                file.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
                let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
                let mut buf = vec![0u8; 1 << 16];
                while pos <= last {
                    let want = ((last - pos + 1) as usize).min(buf.len());
                    let n = reader.read(&mut buf[..want]).map_err(|e| format!("{url}: {e}"))?;
                    if n == 0 {
                        return Err(format!("{url}: the connection ended at byte {pos}"));
                    }
                    file.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
                    pos += n as u64;
                    self.dl.lock().unwrap().bytes += n as u64;
                }
                Ok(())
            };
            if let Err(e) = one() {
                tries += 1;
                if tries > 3 {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(500 * tries));
            }
        }
        Ok(())
    }

    /// The file of an area's folder that the name `<name>/<file>` is, or None.
    fn area_dir(&self, name: &str) -> Option<PathBuf> {
        is_name(name).then(|| self.out().join(name))
    }

    /// Start a build from the request of the page (`POST /api/build`):
    /// `{"name", "boundary": GeoJSON, "data": {"kind": "geofabrik", "url"} | {"kind": "file", "path"}, "settings": YAML text, "dem": {...} | null}`.
    fn start(self: &Arc<Self>, body: &[u8]) -> Result<(), String> {
        let b: Value = serde_json::from_slice(body).map_err(|e| format!("the request: {e}"))?;
        let name = b["name"].as_str().unwrap_or_default().to_string();
        let dir = self.area_dir(&name).ok_or("the name of the area is letters, digits, _, - and . (at most 64)")?;
        if !b["boundary"].is_object() {
            return Err("the request has no boundary".into());
        }
        let data = b["data"].clone();
        let (pbf, url) = match data["kind"].as_str() {
            Some("file") => {
                let p = PathBuf::from(data["path"].as_str().unwrap_or_default());
                if !p.is_file() {
                    return Err(format!("{}: the file is not there", p.display()));
                }
                (p, None)
            }
            Some("geofabrik") => {
                let url = data["url"].as_str().filter(|u| u.starts_with("http://") || u.starts_with("https://")).ok_or("the extract has no URL")?;
                let file = url.rsplit('/').next().filter(|f| f.ends_with(".pbf") && is_name(f)).ok_or("the URL does not end with a .pbf file")?;
                let pbf = self.osm_dir().join(file);
                if !pbf.is_file() {
                    return Err(format!("{file} is not in the folder: Get it in step 3 first"));
                }
                (pbf, Some(url.to_string()))
            }
            _ => return Err("the request has no data: a Geofabrik extract or a file".into()),
        };
        let dem = b["dem"].is_object().then(|| b["dem"].clone());
        // a DEM from the downloaded files of a model: the server joins them and cuts them to the area (2 pixels more on each side) into dem.tif
        if let Some(files) = dem.as_ref().and_then(|d| d["files"].as_array()) {
            let src = self.dem_dir(dem.as_ref().and_then(|d| d["source"].as_str()).unwrap_or_default())?;
            let paths = files.iter().map(|f| f.as_str().filter(|f| is_name(f)).map(|f| src.join(f)).ok_or("a DEM file name is letters, digits, _, - and .".to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            let bbox = bbox_of(&b["boundary"]).ok_or("the boundary has no position")?;
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            duckosm_build::elevation::write_area_dem(&paths.iter().map(PathBuf::as_path).collect::<Vec<_>>(), bbox, 2, &dir.join("dem.tif"))?;
        }
        if dem.is_some() && !dir.join("dem.tif").is_file() {
            return Err("the request has a DEM, but the server has no dem.tif for this area: send it with PUT /api/dem/<name> first".into());
        }
        {
            let mut j = self.job.lock().unwrap();
            if matches!(j.state, "downloading" | "building") {
                return Err(format!("the build of {} runs: wait for it", j.name));
            }
            *j = Job { state: "building", name: name.clone(), db: dir.join(format!("{name}.duckdb")).display().to_string(), dir: dir.display().to_string(), started: Some(Instant::now()), ..Job::default() };
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let boundary = if b["boundary"]["type"] == "FeatureCollection" { b["boundary"].clone() } else {
            json!({"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {"name": name}, "geometry": b["boundary"]}]})
        };
        std::fs::write(dir.join("boundary.geojson"), boundary.to_string()).map_err(|e| e.to_string())?;
        let me = self.clone();
        let settings = b["settings"].as_str().unwrap_or_default().to_string();
        std::thread::spawn(move || {
            let r = me.run(&name, &dir, &pbf, url.as_deref(), dem.as_ref(), &settings);
            let mut j = me.job.lock().unwrap();
            j.ended = Some(Instant::now());
            match r {
                Ok(text) => {
                    (j.state, j.text) = ("done", text);
                    // the files of the area (the folder, one level): what the build made and what it downloaded
                    let files = folder_files(&dir);
                    j.files = files;
                }
                Err(e) => (j.state, j.error) = ("error", e),
            }
        });
        Ok(())
    }

    fn step(&self, line: &str) {
        let mut j = self.job.lock().unwrap();
        if j.steps.len() < 500 {
            let t = j.started.map_or(0.0, |t| t.elapsed().as_secs_f64());
            j.steps.push(line.to_string());
            j.step_secs.push(t);
        }
    }

    /// The work of one build: the config, the build, and the page of the result. The OSM file is on the disk (step 3 downloads an extract).
    fn run(&self, name: &str, dir: &Path, pbf: &Path, url: Option<&str>, dem: Option<&Value>, settings: &str) -> Result<String, String> {
        if let Some(url) = url {
            self.step(&format!("Read {} (downloaded in step 3 from {url})", pbf.display()));
        }
        // the page of the area that was built before closes its database: the build may write the same file
        if let Some(old) = self.edit.lock().unwrap().take() {
            old.close();
        }
        let config = config_text(name, &pbf.display().to_string(), dem, settings);
        std::fs::write(dir.join("config.yaml"), &config).map_err(|e| e.to_string())?;
        let db = dir.join(format!("{name}.duckdb"));
        self.step(&format!("Build from {} into {}", pbf.display(), db.display()));
        let text = self.host.build(&config, dir, &db, &|line| self.step(line))?;
        self.step(&format!("The terrain of {name} (from the ground of the database, if it has one)"));
        self.make_terrain(dir, name)?;
        self.make_tiled(dir, name)?;
        self.step(&format!("Visualization: make the map page of {name} (the roads and the layers of the database on a blank base map)"));
        let site = Site::with_cache(self.host.db(&db), &db, &dir.join("osm_fixes.yaml"), self.port, Mode::Edit, self.pager, Some(dir.join(format!("{name}.map.html"))))?;
        *self.edit.lock().unwrap() = Some(Arc::new(site));
        *self.current.lock().unwrap() = name.to_string();
        Ok(text)
    }

    /// The folders inside `path` (the folder of the areas when `path` is empty), for the page's folder picker: `{"path", "parent", "dirs": [names]}` (hidden folders are left out).
    fn folders(&self, path: &str) -> Result<String, String> {
        let want = if path.trim().is_empty() { self.out() } else { PathBuf::from(path.trim()) };
        let p = std::fs::canonicalize(&want).map_err(|_| format!("{}: not a folder", want.display()))?;
        if !p.is_dir() {
            return Err(format!("{}: not a folder", p.display()));
        }
        let mut dirs: Vec<String> = std::fs::read_dir(&p).map_err(|e| format!("{}: {e}", p.display()))?.flatten()
            .filter(|e| e.metadata().is_ok_and(|m| m.is_dir())).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('.')).collect();
        dirs.sort();
        dirs.truncate(500);
        Ok(json!({"path": p.display().to_string(), "parent": p.parent().map(|d| d.display().to_string()), "dirs": dirs}).to_string())
    }

    /// Use another folder for the areas: `{"path", "create": bool}`. A folder that is not there is made only when `create` is true.
    fn set_folder(&self, body: &[u8]) -> Result<String, String> {
        let b: Value = serde_json::from_slice(body).map_err(|e| format!("the request: {e}"))?;
        let path = PathBuf::from(b["path"].as_str().unwrap_or_default().trim());
        if path.as_os_str().is_empty() {
            return Err("the request has no folder".into());
        }
        if !path.exists() && b["create"].as_bool() == Some(true) {
            std::fs::create_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let p = std::fs::canonicalize(&path).map_err(|_| format!("{}: the folder is not there", path.display()))?;
        if !p.is_dir() {
            return Err(format!("{}: not a folder", p.display()));
        }
        *self.out.lock().unwrap() = p.clone();
        Ok(json!({"out": p.display().to_string()}).to_string())
    }

    /// The areas of the folder: each folder `<name>/` that has `<name>.duckdb`, newest first. The config is in the answer (the page reads the networks from it).
    fn areas(&self) -> Value {
        let mut list: Vec<(u64, Value)> = std::fs::read_dir(self.out()).into_iter().flatten().flatten().filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let (dir, db) = (e.path(), e.path().join(format!("{name}.duckdb")));
            let meta = std::fs::metadata(&db).ok().filter(|m| m.is_file() && is_name(&name))?;
            let secs = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            Some((secs, json!({"name": name, "size": meta.len(), "modified": secs, "dem": dir.join("dem.tif").is_file(), "config": std::fs::read_to_string(dir.join("config.yaml")).unwrap_or_default()})))
        }).collect();
        list.sort_by(|a, b| b.0.cmp(&a.0));
        Value::Array(list.into_iter().map(|x| x.1).collect())
    }

    /// The boundary and the config of a saved area, to load them into the steps again.
    fn area_info(&self, name: &str) -> Result<String, String> {
        let dir = self.area_dir(name).filter(|d| d.join(format!("{name}.duckdb")).is_file()).ok_or("no such area")?;
        let boundary: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("boundary.geojson")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let files: Vec<Value> = folder_files(&dir).into_iter().map(|(n, b)| json!({"name": n, "size": b})).collect();
        Ok(json!({"name": name, "boundary": boundary, "config": std::fs::read_to_string(dir.join("config.yaml")).unwrap_or_default(),
                  "dir": dir.display().to_string(), "db": dir.join(format!("{name}.duckdb")).display().to_string(), "files": files,
                  "ground": std::fs::read_to_string(dir.join(format!("{name}.terrain.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())}).to_string())
    }

    /// The ground of a built area (`POST /api/ground`, `{"name", "dem": {...} | null}`): the area is built again from its own config (the same OSM file and
    /// settings) with this DEM, or with none. The DEM is as in `POST /api/build` (the files of a model, or a dem.tif that `PUT /api/dem/<name>` gave).
    fn ground(self: &Arc<Self>, body: &[u8]) -> Result<(), String> {
        let b: Value = serde_json::from_slice(body).map_err(|e| format!("the request: {e}"))?;
        let name = b["name"].as_str().unwrap_or_default();
        let dir = self.area_dir(name).filter(|d| d.join(format!("{name}.duckdb")).is_file()).ok_or("build the area first")?;
        let config = std::fs::read_to_string(dir.join("config.yaml")).map_err(|e| format!("{}: {e}", dir.join("config.yaml").display()))?;
        let (pbf, settings) = config_parts(&config);
        let pbf = pbf.ok_or("the config of the area has no pbf_path")?;
        let boundary: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("boundary.geojson")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let req = json!({"name": name, "boundary": boundary, "data": {"kind": "file", "path": pbf}, "settings": settings, "dem": b["dem"]});
        self.start(req.to_string().as_bytes())
    }

    /// No open area (`POST /api/close`): `GET /` gives the page with no area again.
    fn close(&self) {
        self.current.lock().unwrap().clear();
        if let Some(old) = self.edit.lock().unwrap().take() {
            old.close();
        }
    }

    /// The tiled map of an area (`page::make_tiled`) with its own connection to the database: call it when no page of the area has the database open.
    fn make_tiled(&self, dir: &Path, name: &str) -> Result<(), String> {
        let mut db = self.host.db(&dir.join(format!("{name}.duckdb")));
        db.open()?;
        let made = crate::page::make_tiled(db.as_mut(), name, &format!("__ORIGIN__/tiles/{name}/"));
        db.close();
        let (style, tiles) = made?;
        *self.tiled.lock().unwrap() = Some((name.to_string(), style, Arc::new(tiles)));
        Ok(())
    }

    /// The terrain file of an area: roadstyle's terrain archive (`terrain_pmtiles`) of `main.elevation_grid`, kept in `<name>.terrain.pmtiles`
    /// with the facts of the ground (`main.elevation_metadata`) in `<name>.terrain.json`. A file newer than the database is used as it is.
    /// Nothing when the area has no grid. It opens the database for itself: call it when no page of the area has the database open.
    fn make_terrain(&self, dir: &Path, name: &str) -> Result<(), String> {
        let (db_path, file, facts) = (dir.join(format!("{name}.duckdb")), dir.join(format!("{name}.terrain.pmtiles")), dir.join(format!("{name}.terrain.json")));
        let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if matches!((modified(&file), modified(&db_path)), (Some(a), Some(b)) if a >= b) && facts.is_file() {
            return Ok(());
        }
        let mut db = self.host.db(&db_path);
        db.open()?;
        let made = (|| -> Result<Option<(Value, Vec<u8>)>, String> {
            let n = crate::rows(db.as_mut(), "SELECT count(*) AS n FROM information_schema.tables WHERE table_schema = 'main' AND table_name IN ('elevation_grid', 'elevation_metadata')")?;
            if n.first().and_then(|r| r["n"].as_i64()) != Some(2) {
                return Ok(None);
            }
            let cells = db.texts("SELECT to_json({c: list(col), r: list(\"row\"), x: list(lon), y: list(lat), z: list(z)})::VARCHAR FROM main.elevation_grid")?;
            let v: Value = serde_json::from_str(cells.first().cloned().flatten().as_deref().unwrap_or("null")).map_err(|e| format!("the ground grid: {e}"))?;
            let ints = |k: &str| v[k].as_array().map(|a| a.iter().map(|x| x.as_i64().unwrap_or(0) as i32).collect::<Vec<_>>()).unwrap_or_default();
            let floats = |k: &str| v[k].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect::<Vec<_>>()).unwrap_or_default();
            let g = roadstyle::tiles::terrain::Ground::from_cells(&ints("c"), &ints("r"), &floats("x"), &floats("y"), &floats("z"))?;
            let meta = crate::rows(db.as_mut(), "SELECT sha256 AS id, source, product, vertical_datum, resolution_m, license FROM main.elevation_metadata LIMIT 1")?;
            let meta = meta.into_iter().next().unwrap_or(Value::Null);
            Ok(Some((meta.clone(), roadstyle::tiles::terrain::terrain_pmtiles(&g, &meta))))
        })();
        db.close();
        match made? {
            Some((meta, bytes)) => {
                std::fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
                std::fs::write(&facts, meta.to_string()).map_err(|e| format!("{}: {e}", facts.display()))
            }
            None => {
                let _ = (std::fs::remove_file(&file), std::fs::remove_file(&facts));
                Ok(())
            }
        }
    }

    /// Delete a saved area (`POST /api/delete`, its name): its folder and all the files in it. Only a folder with `<name>.duckdb` is an area; the
    /// downloaded extracts (`osm/`) and DEM files (`dem/`) stay. The page asks the person first.
    fn delete(&self, name: &str) -> Result<(), String> {
        let dir = self.area_dir(name).filter(|d| d.join(format!("{name}.duckdb")).is_file()).ok_or("no such area")?;
        {
            let j = self.job.lock().unwrap();
            if j.state == "building" && j.name == name {
                return Err(format!("{name} is building: wait for it"));
            }
        }
        if let Some(old) = self.edit.lock().unwrap().take() {      // the map page may have the database open
            old.close();
        }
        let mut cur = self.current.lock().unwrap();
        if *cur == name {
            cur.clear();
        }
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))
    }

    /// Open the page of the result for a saved area (the page of `duckosm edit`, at `/edit/`). It answers at once: the page is made in the background
    /// (or read from `<name>.map.html`), and `/edit/` says "Preparing the map" until it is there.
    fn open(self: &Arc<Self>, name: &str) -> Result<(), String> {
        let dir = self.area_dir(name).filter(|d| d.join(format!("{name}.duckdb")).is_file()).ok_or("no such area")?;
        if matches!(self.job.lock().unwrap().state, "downloading" | "building") {
            return Err("a build runs: wait for it".into());
        }
        if let Some(old) = self.edit.lock().unwrap().take() {
            old.close();
        }
        *self.opening.lock().unwrap() = name.to_string();
        *self.current.lock().unwrap() = name.to_string();
        let (me, name) = (self.clone(), name.to_string());
        std::thread::spawn(move || {
            let db = dir.join(format!("{name}.duckdb"));
            if let Err(e) = me.make_terrain(&dir, &name) {
                eprintln!("duckosm studio: {name}: the terrain: {e}");
            }
            if let Err(e) = me.make_tiled(&dir, &name) {
                eprintln!("duckosm studio: {name}: the map: {e}");
            }
            let site = Site::with_cache(me.host.db(&db), &db, &dir.join("osm_fixes.yaml"), me.port, Mode::Edit, me.pager, Some(dir.join(format!("{name}.map.html"))));
            let mut opening = me.opening.lock().unwrap();
            if *opening != name {
                return;      // another area was opened meanwhile
            }
            match site {
                Ok(s) => *me.edit.lock().unwrap() = Some(Arc::new(s)),
                Err(e) => eprintln!("duckosm studio: {name}: {e}"),
            }
            opening.clear();
        });
        Ok(())
    }

    /// The buffer around an area, as Studio makes it (duckOSM's `boundary.buffer_m`): `{"geometry": Polygon | MultiPolygon, "metres": 0 to 50000}` gives `{"geometry"}`.
    /// The SQL text has numbers only: the page's JSON is turned to WKT here.
    fn buffer(&self, body: &[u8]) -> Result<String, String> {
        let b: Value = serde_json::from_slice(body).map_err(|e| format!("the request: {e}"))?;
        let metres = b["metres"].as_f64().filter(|m| (0.0..=50000.0).contains(m)).ok_or("the buffer is 0 to 50000 metres")?;
        let polys: Vec<&Value> = match b["geometry"]["type"].as_str() {
            Some("Polygon") => vec![&b["geometry"]["coordinates"]],
            Some("MultiPolygon") => b["geometry"]["coordinates"].as_array().ok_or("no coordinates")?.iter().collect(),
            _ => return Err("the geometry is a Polygon or a MultiPolygon".into()),
        };
        let (mut w, mut s_, mut e, mut n) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        let mut wkt = Vec::new();
        for p in polys {
            let mut rings = Vec::new();
            for r in p.as_array().ok_or("a polygon has rings")? {
                let mut pts = Vec::new();
                for c in r.as_array().ok_or("a ring has points")? {
                    let (x, y) = (c[0].as_f64().ok_or("a point has numbers")?, c[1].as_f64().ok_or("a point has numbers")?);
                    (w, s_, e, n) = (w.min(x), s_.min(y), e.max(x), n.max(y));
                    pts.push(format!("{x} {y}"));
                }
                rings.push(format!("({})", pts.join(",")));
            }
            wkt.push(format!("({})", rings.join(",")));
        }
        if wkt.is_empty() {
            return Err("the area has no polygon".into());
        }
        let zone = (((w + e) / 2.0 + 180.0) / 6.0).floor().clamp(0.0, 59.0) as u32 + 1;
        let epsg = if n + s_ >= 0.0 { 32600 } else { 32700 } + zone;
        let g = self.host.buffer(&format!("MULTIPOLYGON({})", wkt.join(",")), epsg, metres)?;
        Ok(format!("{{\"geometry\":{g}}}"))
    }

    /// The folder of the downloaded OSM extracts: `<out>/osm/`. One extract serves each area that it covers.
    fn osm_dir(&self) -> PathBuf {
        self.out().join("osm")
    }

    /// The extracts in the folder and the download now (`GET /api/pbf`): `{"folder", "files": [{"file", "size"}], "dl": the state of the download}`.
    fn pbf_list(&self) -> String {
        let dir = self.osm_dir();
        let mut files: Vec<(String, u64)> = std::fs::read_dir(&dir).into_iter().flatten().flatten().filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.ends_with(".osm.pbf").then(|| e.metadata().ok().filter(|m| m.is_file()).map(|m| (n, m.len()))).flatten()
        }).collect();
        files.sort();
        json!({"folder": dir.display().to_string(), "files": files.iter().map(|(f, n)| json!({"file": f, "size": n})).collect::<Vec<_>>(), "dl": self.dl.lock().unwrap().json()}).to_string()
    }

    /// Start the download of an extract into the folder (`POST /api/pbfget`, the address of the `.osm.pbf` file). The page reads the progress with `GET /api/pbf`.
    fn pbf_get(self: &Arc<Self>, body: &[u8]) -> Result<String, String> {
        let url = String::from_utf8_lossy(body).trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("the address of the extract is not http or https".into());
        }
        let file = url.rsplit('/').next().filter(|f| f.ends_with(".osm.pbf") && is_name(f)).ok_or("the address does not end with a .osm.pbf file")?.to_string();
        let to = self.osm_dir().join(&file);
        {
            let mut d = self.dl.lock().unwrap();
            if d.state == "downloading" {
                return Err(format!("{} is downloading: wait for it, or let it end", d.name));
            }
            *d = Job { state: "downloading", name: file.clone(), started: Some(Instant::now()), ..Job::default() };
        }
        std::fs::create_dir_all(self.osm_dir()).map_err(|e| e.to_string())?;
        let me = self.clone();
        std::thread::spawn(move || {
            let r = if to.is_file() { Ok(()) } else { me.download(&url, &to) };
            let mut d = me.dl.lock().unwrap();
            d.ended = Some(Instant::now());
            match r {
                Ok(()) => (d.state, d.dir) = ("done", to.display().to_string()),
                Err(e) => (d.state, d.error) = ("error", e),
            }
        });
        Ok("{}".into())
    }

    /// The folder of the downloaded files of a DEM model: `<out>/dem/<source>/`.
    fn dem_dir(&self, source: &str) -> Result<PathBuf, String> {
        is_name(source).then(|| self.out().join("dem").join(source)).ok_or_else(|| format!("{source}: not a model name"))
    }

    /// The DEM files of a model for an area (`POST /api/demfiles`, `{"source", "bbox"}`): `{"folder", "tiles": [{"url", "file", "have", "size"}], "local": [{"file", "size"}]}`.
    /// `tiles` are the files that the server can download for the box; `local` are all the `.tif` files in the folder of the model (a file that the person put there too).
    fn dem_files(&self, body: &[u8]) -> Result<String, String> {
        let b: Value = serde_json::from_slice(body).map_err(|e| format!("the request: {e}"))?;
        let source = b["source"].as_str().unwrap_or_default();
        let dir = self.dem_dir(source)?;
        let bbox: [f64; 4] = serde_json::from_value(b["bbox"].clone()).map_err(|_| "the request has no bbox [west, south, east, north]".to_string())?;
        let size = |f: &Path| std::fs::metadata(f).ok().filter(|m| m.is_file()).map(|m| m.len());
        let tiles: Vec<Value> = dem_tiles(source, bbox).into_iter().map(|(url, file)| {
            let have = size(&dir.join(&file));
            json!({"url": url, "file": file, "have": have.is_some(), "size": have.unwrap_or(0)})
        }).collect();
        let mut local: Vec<(String, u64)> = std::fs::read_dir(&dir).into_iter().flatten().flatten().filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            (n.ends_with(".tif") || n.ends_with(".tiff")).then(|| size(&e.path()).map(|s| (n, s))).flatten()
        }).collect();
        local.sort();
        Ok(json!({"folder": dir.display().to_string(), "tiles": tiles, "local": local.iter().map(|(f, n)| json!({"file": f, "size": n})).collect::<Vec<_>>()}).to_string())
    }

    /// Download the files of `tiles` that the folder of the model does not have (`POST /api/demget`, the same request as `demfiles`). A tile that
    /// the model does not have (all sea: 404) is left out. Gives the answer of `demfiles`.
    fn dem_get(&self, body: &[u8]) -> Result<String, String> {
        let b: Value = serde_json::from_slice(body).map_err(|e| format!("the request: {e}"))?;
        let source = b["source"].as_str().unwrap_or_default();
        let dir = self.dem_dir(source)?;
        let bbox: [f64; 4] = serde_json::from_value(b["bbox"].clone()).map_err(|_| "the request has no bbox [west, south, east, north]".to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for (url, file) in dem_tiles(source, bbox) {
            let to = dir.join(&file);
            if to.is_file() {
                continue;
            }
            if let Some(m) = CUT_MODELS.iter().find(|m| m.0 == source) {
                // the STAC item that covers the box, its asset (a cloud-optimized GeoTIFF), and the cut of the box: at most 4,000 pixels on the long side
                let text = ureq::get(&url).call().map_err(|e| format!("{url}: {e}"))?.body_mut().read_to_string().map_err(|e| format!("{url}: {e}"))?;
                let items: Value = serde_json::from_str(&text).map_err(|e| format!("{url}: {e}"))?;
                let covers = |i: &Value| i["bbox"].as_array().is_some_and(|b| b.len() == 4 && b[0].as_f64() <= Some(bbox[0]) && b[1].as_f64() <= Some(bbox[1])
                    && b[2].as_f64() >= Some(bbox[2]) && b[3].as_f64() >= Some(bbox[3]));
                let item = items["features"].as_array().and_then(|f| f.iter().find(|i| covers(i))).ok_or("no single file of the model covers the area")?;
                let href = item["assets"][m.2]["href"].as_str().ok_or("the catalog item has no DEM file")?;
                let part = dir.join(format!("{file}.part"));
                duckosm_build::elevation::cut_projected(HttpFile::open(href)?, href, m.3, bbox, 4000, &part)?;
                std::fs::rename(&part, &to).map_err(|e| format!("{}: {e}", to.display()))?;
                continue;
            }
            let mut resp = match ureq::get(&url).call() {
                Ok(r) => r,
                Err(ureq::Error::StatusCode(404)) => continue,
                Err(e) => return Err(format!("{url}: {e}")),
            };
            let part = dir.join(format!("{file}.part"));
            let mut out = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
            std::io::copy(&mut resp.body_mut().with_config().limit(u64::MAX).reader(), &mut out).map_err(|e| format!("{url}: {e}"))?;
            std::fs::rename(&part, &to).map_err(|e| format!("{}: {e}", to.display()))?;
        }
        self.dem_files(body)
    }

    fn dem(&self, name: &str, body: &[u8]) -> Result<(), String> {
        let dir = self.area_dir(name).ok_or("the name of the area is letters, digits, _, - and . (at most 64)")?;
        // a GeoTIFF starts with II*\0 or MM\0* (or the BigTIFF 43)
        if !(body.starts_with(b"II*\0") || body.starts_with(b"MM\0*") || body.starts_with(b"II+\0") || body.starts_with(b"MM\0+")) {
            return Err("the file is not a GeoTIFF".into());
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("dem.tif"), body).map_err(|e| e.to_string())
    }

    /// Answer one request.
    pub fn route(self: &Arc<Self>, req: &mut Req) -> Result<(), String> {
        let path = req.path.clone();
        let read = req.read();
        if path == "edit" {
            return req.say("301 Moved Permanently\r\nLocation: /edit/", "text/plain", b"");
        }
        if let Some(rest) = path.strip_prefix("edit/") {
            let site = self.edit.lock().unwrap().clone();
            return match site {
                Some(s) => {
                    req.path = rest.to_string();
                    s.route(req)
                }
                None if !self.opening.lock().unwrap().is_empty() => {
                    let name = self.opening.lock().unwrap().clone();
                    let page = format!("<!doctype html><meta charset=utf-8><meta http-equiv=refresh content=1><title>Preparing the map</title><body style=\"font:15px system-ui;margin:40px\">Preparing the map of {name}…</body>");
                    req.say("200 OK", "text/html; charset=utf-8", page.as_bytes())
                }
                None => req.say("404 Not Found", "text/plain", b"no area is built yet"),
            };
        }
        if let Some(rest) = path.strip_prefix("tiles/") {
            // the tiles of the open area's map: tiles/<name>/<source>/<z>/<x>/<y>, in MLT (one tile has every source of the style)
            let parts: Vec<&str> = rest.split('/').collect();
            let t = self.tiled.lock().unwrap().clone();
            let zxy = (parts.len() >= 4).then(|| { let n = parts.len(); (parts[n - 3].parse::<u32>(), parts[n - 2].parse::<u32>(), parts[n - 1].trim_end_matches(".mlt").trim_end_matches(".pbf").parse::<u32>()) });
            return match (t.filter(|t| parts.first() == Some(&t.0.as_str())), zxy) {
                (Some(t), Some((Ok(z), Ok(x), Ok(y)))) => match t.2.tile_in(roadstyle::tiles::TileFormat::Mlt, z, x, y) {      // the style's source has "encoding": "mlt"
                    Ok(b) => req.say("200 OK", "application/octet-stream", &b),
                    Err(e) => req.say("500 Internal Server Error", "text/plain", e.to_string().as_bytes()),
                },
                _ => req.say("404 Not Found", "text/plain", b"no such tile"),
            };
        }
        if path.is_empty() || path == "index.html" || path.starts_with("parts/") {
            if !read {
                return req.say("405 Method Not Allowed", "text/plain", b"the page's files are read only");
            }
            let found: Option<(&str, &[u8])> = if path.is_empty() || path == "index.html" {
                Some(("text/html; charset=utf-8", self.page.as_bytes()))
            } else {
                PARTS.iter().find(|p| p.0 == &path["parts/".len()..]).map(|p| (p.1, p.2.as_bytes()))
            };
            return match found {
                Some((kind, body)) => req.say("200 OK", kind, body),
                None => req.say("404 Not Found", "text/plain", b"not found"),
            };
        }
        let api = path.strip_prefix("api/").unwrap_or("").to_string();
        match (req.method.as_str(), api.as_str()) {
            ("GET" | "HEAD", "info") => {
                let info = json!({"out": self.out().display().to_string(), "geofabrik": self.index, "edit": self.edit.lock().unwrap().is_some(), "opening": *self.opening.lock().unwrap(), "area": *self.current.lock().unwrap(),
                                 "tiled": self.tiled.lock().unwrap().as_ref().map(|t| t.0.clone())});
                req.say("200 OK", "application/json", info.to_string().as_bytes())
            }
            ("GET" | "HEAD", "job") => {
                let j = self.job.lock().unwrap().json();
                req.say("200 OK", "application/json", j.to_string().as_bytes())
            }
            ("GET" | "HEAD", "areas") => req.say("200 OK", "application/json", self.areas().to_string().as_bytes()),
            ("GET" | "HEAD", "pbf") => req.say("200 OK", "application/json", self.pbf_list().as_bytes()),
            ("GET" | "HEAD", a) if a.starts_with("style/") => {
                let t = self.tiled.lock().unwrap().clone();
                match t.filter(|t| t.0 == a["style/".len()..]) {
                    Some(t) => req.say("200 OK", "application/json", t.1.as_bytes()),
                    None => req.say("404 Not Found", "text/plain", b"this area is not open"),
                }
            }
            ("GET" | "HEAD", a) if a.starts_with("terrain/") => {
                let name = &a["terrain/".len()..];
                match self.area_dir(name).map(|d| d.join(format!("{name}.terrain.pmtiles"))).and_then(|f| std::fs::read(f).ok()) {
                    Some(b) => req.say("200 OK", "application/octet-stream", &b),
                    None => req.say("404 Not Found", "text/plain", b"no terrain for this area"),
                }
            }
            ("GET" | "HEAD", a) if a.starts_with("area/") => match self.area_info(&a["area/".len()..]) {
                Ok(t) => req.say("200 OK", "application/json", t.as_bytes()),
                Err(e) => req.say("404 Not Found", "text/plain", e.as_bytes()),
            },
            ("GET" | "HEAD", "geofabrik") => match self.geofabrik() {
                Ok(t) => req.say("200 OK", "application/json", t.as_bytes()),
                Err(e) => req.say("502 Bad Gateway", "text/plain", e.as_bytes()),
            },
            ("POST", "file" | "build" | "open" | "buffer" | "folders" | "folder" | "demfiles" | "demget" | "pbfget" | "delete" | "ground" | "close") | ("PUT", _) if matches!(api.as_str(), "file" | "build" | "open" | "buffer" | "folders" | "folder" | "demfiles" | "demget" | "pbfget" | "delete" | "ground" | "close") || api.starts_with("dem/") => {
                let own = self.own.clone();
                let Some(body) = req.write_body(&own)? else { return Ok(()) };
                let r: Result<String, String> = match (req.method.as_str(), api.as_str()) {
                    ("POST", "file") => {
                        let p = PathBuf::from(String::from_utf8_lossy(&body).trim());
                        std::fs::metadata(&p).ok().filter(|m| m.is_file()).map(|m| json!({"size": m.len()}).to_string())
                            .ok_or_else(|| format!("{}: not a file", p.display()))
                    }
                    ("POST", "build") => self.start(&body).map(|_| "{}".to_string()),
                    ("POST", "buffer") => self.buffer(&body),
                    ("POST", "folders") => self.folders(&String::from_utf8_lossy(&body)),
                    ("POST", "folder") => self.set_folder(&body),
                    ("POST", "demfiles") => self.dem_files(&body),
                    ("POST", "demget") => self.dem_get(&body),
                    ("POST", "pbfget") => self.pbf_get(&body),
                    ("POST", "ground") => self.ground(&body).map(|_| "{}".to_string()),
                    ("POST", "close") => { self.close(); Ok("{}".to_string()) }
                    ("POST", "delete") => self.delete(String::from_utf8_lossy(&body).trim()).map(|_| "{}".to_string()),
                    ("POST", "open") => self.open(String::from_utf8_lossy(&body).trim()).map(|_| "{}".to_string()),
                    ("PUT", a) if a.starts_with("dem/") => self.dem(&a["dem/".len()..], &body).map(|_| "{}".to_string()),
                    _ => Err("not this method".into()),
                };
                match r {
                    Ok(t) => req.say("200 OK", "application/json", t.as_bytes()),
                    Err(e) => req.say("400 Bad Request", "text/plain", e.as_bytes()),
                }
            }
            (_, "info" | "job" | "geofabrik" | "areas" | "file" | "build" | "open" | "buffer" | "folders" | "folder" | "demfiles" | "demget" | "pbfget" | "pbf" | "delete" | "ground" | "close") => req.say("405 Method Not Allowed", "text/plain", b"not this method"),
            _ => req.say("404 Not Found", "text/plain", b"not found"),
        }
    }
}

/// Answer the requests of `listener` until the program stops.
pub fn serve(studio: Arc<Studio>, listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        let studio = studio.clone();
        std::thread::spawn(move || {
            let r = read_request(stream, &studio.own).and_then(|req| req.map_or(Ok(()), |mut req| studio.route(&mut req)));
            if let Err(e) = r {
                eprintln!("duckosm studio: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn config_parts_give_the_pbf_and_the_settings() {
        let c = super::config_text("a", "/x/m.osm.pbf", Some(&json!({"product": "DTM"})), "modes: [driving]\noptions:\n  levels: true\n");
        let (pbf, settings) = super::config_parts(&c);
        assert_eq!(pbf.as_deref(), Some("/x/m.osm.pbf"));
        assert_eq!(settings, "modes: [driving]\noptions:\n  levels: true\n");
        assert_eq!(super::config_text("a", "/x/m.osm.pbf", None, &settings), super::config_text("a", "/x/m.osm.pbf", None, "modes: [driving]\noptions:\n  levels: true\n"));
    }

    #[test]
    fn dem_tiles_cover_the_box() {
        let t = super::dem_tiles("copernicus-glo30", [7.40, 43.72, 7.44, 43.76]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].1, "Copernicus_DSM_COG_10_N43_00_E007_00_DEM.tif");
        assert!(t[0].0.starts_with("https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_N43_00_E007_00_DEM/"));
        let t = super::dem_tiles("copernicus-glo90", [-0.5, -1.2, 0.5, 0.0]);
        let names: Vec<_> = t.iter().map(|x| x.1.as_str()).collect();
        assert_eq!(names, ["Copernicus_DSM_COG_30_S02_00_W001_00_DEM.tif", "Copernicus_DSM_COG_30_S02_00_E000_00_DEM.tif",
                           "Copernicus_DSM_COG_30_S01_00_W001_00_DEM.tif", "Copernicus_DSM_COG_30_S01_00_E000_00_DEM.tif"]);
        let t = super::dem_tiles("ca-nrcan-hrdem", [-123.02411, 49.18, -122.8926, 49.2997]);
        assert_eq!(t[0].1, "ca-nrcan-hrdem_m123.0241_49.1800_m122.8926_49.2997.tif");
        assert!(t[0].0.contains("collections=hrdem-mosaic-1m&bbox=-123.0241,49.18,-122.8926,49.2997"), "{}", t[0].0);
        assert!(super::dem_tiles("fabdem", [0.0, 0.0, 1.0, 1.0]).is_empty());
    }

    #[test]
    fn bbox_of_geojson() {
        let v = serde_json::json!({"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {"name": "x"}, "geometry": {"type": "MultiPolygon", "coordinates": [[[[7.4, 43.7], [7.5, 43.7], [7.5, 43.8], [7.4, 43.7]]]]}}]});
        assert_eq!(super::bbox_of(&v), Some([7.4, 43.7, 7.5, 43.8]));
        assert_eq!(super::bbox_of(&serde_json::json!({"type": "Polygon", "coordinates": []})), None);
    }

    use super::*;
    use std::io::Read;
    use std::net::TcpStream;

    struct FakeDb;
    impl Db for FakeDb {
        fn open(&mut self) -> Result<(), String> { Ok(()) }
        fn close(&mut self) {}
        fn execute(&mut self, _: &str) -> Result<(), String> { Ok(()) }
        fn texts(&mut self, sql: &str) -> Result<Vec<Option<String>>, String> {
            Ok(if sql.contains("count(*) AS n") { vec![Some(r#"{"n": 0}"#.into())] } else { vec![] })
        }
        fn build(&mut self, _: &str, _: &Path, _: &Path, _: &Path) -> Result<String, String> { Err("no".into()) }
    }

    /// A build that writes a mark file in the folder, and gives the config back as its summary.
    struct FakeHost(Arc<Mutex<Vec<String>>>);
    impl Host for FakeHost {
        fn db(&self, _: &Path) -> Box<dyn Db> { Box::new(FakeDb) }
        fn buffer(&self, wkt: &str, epsg: u32, metres: f64) -> Result<String, String> {
            Ok(format!("{{\"type\":\"Polygon\",\"coordinates\":[],\"note\":\"{epsg} {metres} {}\"}}", wkt.len()))
        }
        fn build(&self, config: &str, dir: &Path, db: &Path, log: &(dyn Fn(&str) + Sync)) -> Result<String, String> {
            log("a step");
            self.0.lock().unwrap().push(config.to_string());
            if config.contains("fail") { return Err("the build failed".into()); }
            std::fs::write(db, b"db").map_err(|e| e.to_string())?;
            Ok(format!("built in {}", dir.display()))
        }
    }

    fn server(tag: &str, index: Option<String>) -> (PathBuf, impl Fn(&str) -> String, u16, Arc<Mutex<Vec<String>>>) {
        let out = std::env::temp_dir().join(format!("duckosm-studio-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let listener = crate::bind(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = Studio::with_pager(Box::new(FakeHost(seen.clone())), &out, port, index, |_, _| Ok("the result page".into())).unwrap();
        let out = s.out();
        std::thread::spawn(move || serve(Arc::new(s), listener));
        let ask = move |req: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(req.as_bytes()).ok();
            let mut text = String::new();
            s.read_to_string(&mut text).ok();
            text
        };
        (out, ask, port, seen)
    }

    fn post(port: u16, ask: &impl Fn(&str) -> String, method: &str, path: &str, body: &str) -> String {
        ask(&format!("{method} /api/{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Length: {}\r\n\r\n{body}", body.len()))
    }

    #[test]
    fn the_config_has_the_place_then_the_settings() {
        let t = config_text("monaco", "/data/m.osm.pbf", None, "modes: [driving]\n");
        assert_eq!(t, "name: \"monaco\"\noutput_path: .\nsource:\n  type: pbf\n  pbf_path: \"/data/m.osm.pbf\"\nboundary:\n  path: boundary.geojson\nmodes: [driving]\n");
        let d = json!({"product": "DTM", "vertical_datum": "EGM2008", "license": "CC-BY 4.0", "not_reference": false});
        let t = config_text("m", "p", Some(&d), "modes: [walking]");
        assert!(t.contains("elevation:\n  dem: dem.tif\n  product: \"DTM\"\n  vertical_datum: \"EGM2008\"\n  license: \"CC-BY 4.0\"\n  reference: true\n"), "{t}");
        assert!(t.ends_with("modes: [walking]\n"));
        let t = config_text("m", "p\"x", Some(&json!({"not_reference": true})), "");
        assert!(t.contains("pbf_path: \"p\\\"x\"") && t.contains("  reference: false\n") && !t.contains("product"), "{t}");
    }

    #[test]
    fn the_page_and_the_parts_are_read_only() {
        let (_, ask, port, _) = server("page", None);
        let get = |p: &str| ask(&format!("GET /{p} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"));
        assert!(get("").contains("<title>duckOSM Studio"));
        for f in ["parts/ground_step/ground_step.js", "parts/build_steps/area_step.js", "parts/data/dem_sources.json"] {
            assert!(get(f).starts_with("HTTP/1.1 200"), "{f}");
        }
        for f in ["parts/../Cargo.toml", "parts/nothing.js", "parts/studio.js", "vendor/maplibre-gl.js", "edit/", "api/nothing"] {
            assert!(get(f).starts_with("HTTP/1.1 404"), "{f}");
        }
        assert!(ask(&format!("PUT /parts/data/dem_sources.json HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n")).starts_with("HTTP/1.1 405"));
        assert!(ask("GET / HTTP/1.1\r\nHost: evil.example\r\n\r\n").starts_with("HTTP/1.1 403"));
        assert!(get("edit").contains("Location: /edit/"));
    }

    #[test]
    fn a_dem_goes_in_the_folder_of_the_area() {
        let (out, ask, port, _) = server("dem", None);
        assert!(post(port, &ask, "PUT", "dem/monaco", "not a tiff").starts_with("HTTP/1.1 400"));
        assert!(post(port, &ask, "PUT", "dem/..", "II*\0abc").starts_with("HTTP/1.1 400"));
        assert!(!out.join("monaco").exists());
        assert!(post(port, &ask, "PUT", "dem/monaco", "II*\0abc").starts_with("HTTP/1.1 200"));
        assert_eq!(std::fs::read(out.join("monaco/dem.tif")).unwrap(), b"II*\0abc");
        let other = ask(&format!("PUT /api/dem/monaco HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://evil.example\r\nContent-Length: 1\r\n\r\nx"));
        assert!(other.starts_with("HTTP/1.1 403"));
    }

    #[test]
    fn a_build_runs_the_host_and_makes_the_page_of_the_result() {
        let (out, ask, port, seen) = server("build", None);
        let pbf = out.join("m.osm.pbf");
        std::fs::write(&pbf, b"pbf").unwrap();
        let area = json!({"type": "Polygon", "coordinates": [[[0, 0], [1, 0], [1, 1], [0, 0]]]});
        let req = |name: &str, settings: &str| json!({"name": name, "boundary": area, "data": {"kind": "file", "path": pbf}, "settings": settings}).to_string();
        for bad in [json!({"name": "../x", "boundary": area, "data": {"kind": "file", "path": pbf}}), json!({"name": "a", "data": {"kind": "file", "path": pbf}}),
                    json!({"name": "a", "boundary": area, "data": {"kind": "file", "path": "/nowhere.pbf"}}), json!({"name": "a", "boundary": area, "data": {"kind": "ftp"}}),
                    json!({"name": "a", "boundary": area, "data": {"kind": "file", "path": pbf}, "dem": {"product": "DTM"}})] {
            assert!(post(port, &ask, "POST", "build", &bad.to_string()).starts_with("HTTP/1.1 400"), "{bad}");
        }
        assert!(seen.lock().unwrap().is_empty() && std::fs::read_dir(&out).unwrap().count() == 1, "a refused request makes nothing but the PBF of the test");
        assert!(post(port, &ask, "POST", "build", &req("monaco", "modes: [driving]\n")).starts_with("HTTP/1.1 200"));
        let mut job = String::new();
        for _ in 0..100 {
            job = ask(&format!("GET /api/job HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
            if job.contains(r#""state":"done""#) || job.contains(r#""state":"error""#) { break; }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(job.contains(r#""state":"done""#) && job.contains("a step") && job.contains("built in"), "{job}");
        assert!(out.join("monaco/boundary.geojson").is_file() && out.join("monaco/config.yaml").is_file() && out.join("monaco/monaco.duckdb").is_file());
        assert!(std::fs::read_to_string(out.join("monaco/boundary.geojson")).unwrap().contains("FeatureCollection"));
        assert!(seen.lock().unwrap()[0].ends_with("modes: [driving]\n"));
        // the page of the result is the page of `duckosm edit`, under /edit/
        assert!(ask(&format!("GET /edit/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")).ends_with("the result page"));
        assert!(ask(&format!("GET /edit/api/info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")).contains("monaco"));
        // a build that fails: the state is error with the text of the host
        assert!(post(port, &ask, "POST", "build", &req("bad", "fail: 1\n")).starts_with("HTTP/1.1 200"));
        for _ in 0..100 {
            job = ask(&format!("GET /api/job HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
            if job.contains(r#""state":"error""#) { break; }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(job.contains(r#""state":"error""#) && job.contains("the build failed"), "{job}");
    }

    #[test]
    fn a_file_is_checked_and_the_extracts_come_from_a_file() {
        let list = std::env::temp_dir().join(format!("duckosm-studio-index-{}.json", std::process::id()));
        std::fs::write(&list, r#"{"features": []}"#).unwrap();
        let (out, ask, port, _) = server("file", Some(list.display().to_string()));
        std::fs::write(out.join("a.osm.pbf"), b"1234").unwrap();
        assert!(post(port, &ask, "POST", "file", &out.join("a.osm.pbf").display().to_string()).ends_with(r#"{"size":4}"#));
        assert!(post(port, &ask, "POST", "file", "/nowhere.osm.pbf").starts_with("HTTP/1.1 400"));
        assert!(ask(&format!("GET /api/geofabrik HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")).ends_with(r#"{"features": []}"#));
    }

    #[test]
    fn saved_areas_are_listed_loaded_and_opened() {
        let (out, ask, port, _) = server("areas", None);
        let get = |p: &str| ask(&format!("GET /api/{p} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
        assert!(get("areas").ends_with("[]"));
        for (n, t) in [("old", 1), ("new", 2)] {
            std::fs::create_dir_all(out.join(n)).unwrap();
            std::fs::write(out.join(n).join(format!("{n}.duckdb")), vec![0u8; t]).unwrap();
            std::fs::write(out.join(n).join("config.yaml"), format!("name: {n}\nmodes: [driving]\n")).unwrap();
            std::fs::write(out.join(n).join("boundary.geojson"), r#"{"type":"FeatureCollection","features":[]}"#).unwrap();
        }
        std::fs::create_dir_all(out.join("no-db")).unwrap();
        let list = get("areas");
        let names: Vec<_> = serde_json::from_str::<Value>(list.split("\r\n\r\n").nth(1).unwrap()).unwrap().as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names.len(), 2, "{list}");
        assert!(names.contains(&"old".to_string()) && names.contains(&"new".to_string()) && !list.contains("no-db"));
        assert!(get("area/old").contains("\"boundary\"") && get("area/old").contains("modes: [driving]"));
        for bad in ["area/..", "area/no-db", "area/nothing"] {
            assert!(get(bad).starts_with("HTTP/1.1 404"), "{bad}");
        }
        assert!(post(port, &ask, "POST", "open", "no-db").starts_with("HTTP/1.1 400"));
        assert!(post(port, &ask, "POST", "open", "old").starts_with("HTTP/1.1 200"));
        // Open answers at once; the page is made in the background ("Preparing the map" until then), and kept in old/old.map.html
        let page = (0..100).map(|_| { std::thread::sleep(std::time::Duration::from_millis(20)); ask(&format!("GET /edit/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")) })
            .find(|r| !r.contains("Preparing the map")).unwrap();
        assert!(page.ends_with("the result page"), "{page}");
        assert_eq!(std::fs::read_to_string(out.join("old").join("old.map.html")).unwrap(), "the result page");
    }

    #[test]
    fn the_buffer_turns_the_area_to_wkt_and_picks_the_utm_zone() {
        let (_, ask, port, _) = server("buffer", None);
        let area = json!({"type": "Polygon", "coordinates": [[[7.4, 43.7], [7.5, 43.7], [7.5, 43.8], [7.4, 43.7]]]});
        let r = post(port, &ask, "POST", "buffer", &json!({"geometry": area, "metres": 100}).to_string());
        assert!(r.contains("32632 100"), "Monaco is in the zone 32 north: {r}");
        let south = json!({"type": "Polygon", "coordinates": [[[-70.0, -33.0], [-69.9, -33.0], [-69.9, -32.9], [-70.0, -33.0]]]});
        assert!(post(port, &ask, "POST", "buffer", &json!({"geometry": south, "metres": 5}).to_string()).contains("32719 5"), "the southern zone has 327..");
        for bad in [json!({"geometry": area, "metres": -1}), json!({"geometry": area, "metres": 50001}), json!({"geometry": {"type": "Point", "coordinates": [0, 0]}, "metres": 5}),
                    json!({"geometry": {"type": "Polygon", "coordinates": [[["x", 1]]]}, "metres": 5})] {
            assert!(post(port, &ask, "POST", "buffer", &bad.to_string()).starts_with("HTTP/1.1 400"), "{bad}");
        }
    }

    /// A server that gives ranges (and counts the connections that ask for one): the download of a file of 20 MB is made of four ranges and is the same file.
    #[test]
    fn a_big_file_is_downloaded_in_four_ranges() {
        let data: Vec<u8> = (0..20_000_000u32).map(|i| (i % 251) as u8).collect();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let (d, r) = (data.clone(), ranges.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (d, r) = (d.clone(), r.clone());
                std::thread::spawn(move || {
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut out = stream;
                    let mut head = String::new();
                    loop {
                        let mut line = String::new();
                        if std::io::BufRead::read_line(&mut reader, &mut line).unwrap_or(0) == 0 || line.trim().is_empty() { break; }
                        head.push_str(&line);
                    }
                    let range = head.lines().find_map(|l| l.strip_prefix("Range: bytes=").or_else(|| l.strip_prefix("range: bytes=")));
                    if head.starts_with("HEAD") {
                        write!(out, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n", d.len()).ok();
                    } else if let Some(rg) = range {
                        let (a, b) = rg.trim().split_once('-').unwrap();
                        let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
                        r.lock().unwrap().push((a, b));
                        write!(out, "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\nConnection: close\r\n\r\n", b - a + 1, d.len()).ok();
                        out.write_all(&d[a..=b]).ok();
                    } else {
                        write!(out, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", d.len()).ok();
                        out.write_all(&d).ok();
                    }
                });
            }
        });
        let (out, _, _, _) = server("download", None);
        let listener = crate::bind(0).unwrap();
        let s = Studio::with_pager(Box::new(FakeHost(Arc::new(Mutex::new(Vec::new())))), &out, listener.local_addr().unwrap().port(), None, |_, _| Ok("page".into())).unwrap();
        let to = out.join("big.osm.pbf");
        s.download(&format!("http://127.0.0.1:{port}/big.osm.pbf"), &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), data, "the file is whole");
        let mut got = ranges.lock().unwrap().clone();
        got.sort();
        assert_eq!(got.len(), 4, "20 MB are four parts of 5 MB (the part is at least 4 MB): {got:?}");
        assert!(got[0].0 == 0 && got[3].1 == data.len() - 1 && got.windows(2).all(|w| w[0].1 + 1 == w[1].0), "the ranges are one after the other: {got:?}");
        assert_eq!(s.dl.lock().unwrap().bytes, data.len() as u64, "the download counts every byte");
        assert!(!out.join("big.osm.pbf.part").exists());
    }

    #[test]
    fn the_folder_of_the_areas_can_change() {
        let (out, ask, port, _) = server("folder", None);
        std::fs::create_dir_all(out.join("sub/inner")).unwrap();
        std::fs::create_dir_all(out.join("sub/.hidden")).unwrap();
        let r = post(port, &ask, "POST", "folders", &out.join("sub").display().to_string());
        assert!(r.contains(r#""dirs":["inner"]"#) && r.contains("\"parent\""), "a hidden folder is not listed: {r}");
        assert!(post(port, &ask, "POST", "folders", "/nowhere/at/all").starts_with("HTTP/1.1 400"));
        assert!(post(port, &ask, "POST", "folders", "").contains(&out.display().to_string()), "empty: the folder of the areas");
        let area = out.join("sub/inner/monaco");
        std::fs::create_dir_all(&area).unwrap();
        std::fs::write(area.join("monaco.duckdb"), b"db").unwrap();
        let get = |p: &str| ask(&format!("GET /api/{p} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
        assert!(get("areas").ends_with("[]"));
        let set = |b: Value| post(port, &ask, "POST", "folder", &b.to_string());
        assert!(set(json!({"path": out.join("sub/inner")})).contains("inner"));
        assert!(get("areas").contains("monaco") && get("info").contains("sub/inner"), "the areas are listed from the new folder");
        assert!(set(json!({"path": out.join("new-one")})).starts_with("HTTP/1.1 400"), "a folder that is not there is not made without create");
        assert!(set(json!({"path": out.join("new-one"), "create": true})).starts_with("HTTP/1.1 200") && out.join("new-one").is_dir());
        assert!(get("areas").ends_with("[]"));
        assert!(set(json!({"path": out.join("sub/inner/monaco/monaco.duckdb")})).starts_with("HTTP/1.1 400"), "a file is not a folder");
    }

    #[test]
    fn the_date_of_a_dated_file() {
        assert_eq!(yymmdd(0).len(), 6);
        assert!(yymmdd(0) >= yymmdd(1) && yymmdd(1) > yymmdd(3), "older days have smaller dates (inside one month and year)");
    }

    #[test]
    fn a_name_is_a_folder_name() {
        for ok in ["monaco", "tartu-1", "a.b_c"] { assert!(is_name(ok), "{ok}"); }
        for bad in ["", ".", "..", ".hidden", "a/b", "a b", "ä", &"x".repeat(65)] { assert!(!is_name(bad), "{bad}"); }
    }

    /// The native host buffers an area with the bundled DuckDB, in the UTM zone: 200 m around a small triangle makes it larger (the box grows by about 400 m).
    #[cfg(feature = "duckdb")]
    #[test]
    fn the_native_host_buffers_an_area() {
        use crate::studio::Host;
        let g = crate::native::NativeHost.buffer("MULTIPOLYGON(((7.41 43.73,7.43 43.73,7.43 43.75,7.41 43.73)))", 32632, 200.0).unwrap();
        let v: Value = serde_json::from_str(&g).unwrap();
        let xs: Vec<f64> = v["coordinates"][0].as_array().unwrap().iter().map(|p| p[0].as_f64().unwrap()).collect();
        let width = xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
        assert!(v["type"] == "Polygon" && width > 0.02 + 0.003, "{g}");      // 0.02 degrees before; 200 m on both sides is 0.005 degrees more
    }

    /// The program's host builds Monaco from a request of the page, with the DEM of the engine's tests: the database has the heights, the grid and the
    /// metadata, and the record of the build. `monaco_sample.osm.pbf` is cut to the boundary by the engine itself.
    #[cfg(feature = "duckdb")]
    #[test]
    fn the_native_host_builds_monaco_with_a_dem() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
        let tif = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duckosm-build/tests/data/monaco-copernicus-glo30.tif");
        let out = std::env::temp_dir().join(format!("duckosm-studio-native-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let listener = crate::bind(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let s = Studio::with_pager(Box::new(crate::native::NativeHost), &out, port, None, |_, _| Ok("the result page".into())).unwrap();
        std::thread::spawn(move || serve(Arc::new(s), listener));
        let ask = |req: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(req.as_bytes()).ok();
            let mut text = String::new();
            s.read_to_string(&mut text).ok();
            text
        };
        let boundary: Value = serde_json::from_str(&std::fs::read_to_string(data.join("monaco.geojson")).unwrap()).unwrap();
        let tif_bytes = std::fs::read(&tif).unwrap();
        let put = format!("PUT /api/dem/monaco HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n\r\n", tif_bytes.len());
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(put.as_bytes()).and_then(|_| s.write_all(&tif_bytes)).unwrap();
        let mut reply = String::new();
        s.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        let req = json!({"name": "monaco", "boundary": boundary, "data": {"kind": "file", "path": data.join("monaco_sample.osm.pbf")},
                         "settings": "modes: [driving]\noptions:\n  levels: false\n  sea: false\n  bus_routes: false\n",
                         "dem": {"product": "DTM", "vertical_datum": "EGM2008", "license": "test", "not_reference": false}}).to_string();
        assert!(post(port, &ask, "POST", "build", &req).starts_with("HTTP/1.1 200"));
        let mut job = String::new();
        for _ in 0..1200 {
            job = ask(&format!("GET /api/job HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
            if job.contains(r#""state":"done""#) || job.contains(r#""state":"error""#) { break; }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(job.contains(r#""state":"done""#), "{job}");
        assert!(job.contains("build driving") && job.contains("elevation grid"), "the steps of the engine are in the job: {job}");
        let con = duckdb::Connection::open_with_flags(out.join("monaco/monaco.duckdb"), duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly).unwrap()).unwrap();
        let n = |sql: &str| con.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert!(n("SELECT count(*) FROM driving.edges") > 1000 && n("SELECT count(*) FROM main.elevation_grid WHERE z IS NOT NULL") > 100);
        assert!(n("SELECT count(*) FROM driving.nodes WHERE ele IS NOT NULL") > 100);
        assert_eq!(n("SELECT count(*) FROM main.elevation_metadata WHERE product = 'DTM' AND vertical_datum = 'EGM2008' AND license = 'test'"), 1);
        let record: String = con.query_row("SELECT build_config FROM main.visualization_metadata", [], |r| r.get(0)).unwrap();
        assert!(record.contains("dem: dem.tif") && record.contains("modes: [driving]"), "{record}");
        assert!(ask(&format!("GET /edit/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")).ends_with("the result page"));
    }
}
