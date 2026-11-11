//! `duckosm studio`: a page on 127.0.0.1 that makes the data of an area, step by step: the area, the OSM data, the settings, the ground (the
//! DEM), the build, and the result. The result is the page of `duckosm edit` (roadstyle's dashboard with the editor), served at `/edit/`: this
//! module has no copy of it. The steps are parts in `web/` (`build_steps/`, `ground_step/`), which duckOSM Studio (the browser app) loads too.
//!
//! The server follows the rules of `lib.rs` (`read_request`): only 127.0.0.1, this server's name in `Host`, a write from this server's page.
//! The files that it writes are in the folder of the area, `<out>/<name>/`: `boundary.geojson`, `dem.tif`, `config.yaml`, the PBF file of a
//! download, and the `<name>.duckdb` of the build. A local PBF file is read from the path that the page gives (the page is on the same computer).
//!
//! The routes: `GET /` the page; `GET /parts/<file>` the parts; `GET /vendor/<file>` MapLibre; `GET /roadstyle/<file>` roadstyle's modules and
//! style; `GET /api/info`; `GET /api/geofabrik` the list of the Geofabrik extracts; `POST /api/file` (the path of a local PBF file: its size);
//! `PUT /api/dem/<name>` the DEM of the area; `POST /api/build` start the build; `GET /api/job` the state of the build; `GET /edit/...` the
//! page of `duckosm edit` for the area that was built last.

use crate::{inside, read_request, Db, Mode, Pager, Req, Site};
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
}

/// The files of the page: its own, the parts of this repository's `web/`, and the catalog of the DEM sources.
const PARTS: &[(&str, &str, &str)] = &[
    ("studio.js", "text/javascript", include_str!("../page/studio.js")),
    ("studio.css", "text/css", include_str!("../page/studio.css")),
    ("ground_step/ground_step.js", "text/javascript", include_str!("../../../web/ground_step/ground_step.js")),
    ("ground_step/ground_core.js", "text/javascript", include_str!("../../../web/ground_step/ground_core.js")),
    ("ground_step/ground_step.css", "text/css", include_str!("../../../web/ground_step/ground_step.css")),
    ("build_steps/core.js", "text/javascript", include_str!("../../../web/build_steps/core.js")),
    ("build_steps/area_step.js", "text/javascript", include_str!("../../../web/build_steps/area_step.js")),
    ("build_steps/data_step.js", "text/javascript", include_str!("../../../web/build_steps/data_step.js")),
    ("build_steps/settings_step.js", "text/javascript", include_str!("../../../web/build_steps/settings_step.js")),
    ("build_steps/build_step.js", "text/javascript", include_str!("../../../web/build_steps/build_step.js")),
    ("build_steps/result_step.js", "text/javascript", include_str!("../../../web/build_steps/result_step.js")),
    ("build_steps/steps.css", "text/css", include_str!("../../../web/build_steps/steps.css")),
    ("data/dem_sources.json", "application/json", include_str!("../../../src/duckosm/data/dem_sources.json")),
];
const PAGE: &str = include_str!("../page/studio.html");
const MAPLIBRE_JS: &str = roadstyle::assets::text!("vendor/maplibre-gl.js");
const MAPLIBRE_CSS: &str = roadstyle::assets::text!("vendor/maplibre-gl.css");
/// The default list of the extracts. A mirror or a file can take its place (`--geofabrik-index`).
pub const GEOFABRIK_INDEX: &str = "https://download.geofabrik.de/index-v1.json";

/// The state of the build, as the page reads it (`GET /api/job`).
#[derive(Default)]
struct Job {
    /// `idle`, `downloading`, `building`, `done` or `error`.
    state: &'static str,
    name: String,
    /// The database file of the area.
    db: String,
    steps: Vec<String>,
    bytes: u64,
    total: u64,
    text: String,
    error: String,
    started: Option<Instant>,
}

impl Job {
    fn json(&self) -> Value {
        json!({"state": if self.state.is_empty() { "idle" } else { self.state }, "name": self.name, "db": self.db, "steps": self.steps, "bytes": self.bytes,
               "total": self.total, "text": self.text, "error": self.error, "seconds": self.started.map_or(0.0, |t| t.elapsed().as_secs_f64())})
    }
}

/// One server: the folder of the areas, the program's host, the build, and the page of the area that was built last.
pub struct Studio {
    pub out: PathBuf,
    pub own: [String; 2],
    port: u16,
    host: Box<dyn Host>,
    /// Where the list of Geofabrik extracts comes from: a URL or a file.
    index: String,
    index_text: Mutex<Option<String>>,
    job: Mutex<Job>,
    edit: Mutex<Option<Arc<Site>>>,
    pager: Pager,
}

/// A name of an area is a folder name: letters, digits, `_`, `-` and `.`, not starting with a dot.
fn is_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && !n.starts_with('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
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

impl Studio {
    /// A server for the folder `out` (it is made when it is not there). `index`: the list of the extracts (URL or file); None: Geofabrik's.
    pub fn new(host: Box<dyn Host>, out: &Path, port: u16, index: Option<String>) -> Result<Studio, String> {
        Studio::with_pager(host, out, port, index, crate::page::make)
    }

    /// As `new`, with the function that makes the page of the result (the tests give a placeholder).
    pub fn with_pager(host: Box<dyn Host>, out: &Path, port: u16, index: Option<String>, pager: Pager) -> Result<Studio, String> {
        std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
        let out = std::fs::canonicalize(out).map_err(|e| format!("{}: {e}", out.display()))?;
        Ok(Studio { out, own: [format!("127.0.0.1:{port}"), format!("localhost:{port}")], port, host, index: index.unwrap_or_else(|| GEOFABRIK_INDEX.into()),
                    index_text: Mutex::new(None), job: Mutex::new(Job::default()), edit: Mutex::new(None), pager })
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

    /// Download `url` to `to` (kept as `<to>.part` until it is whole). The job has the bytes so far.
    fn download(&self, url: &str, to: &Path) -> Result<(), String> {
        let mut resp = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
        let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
        {
            let mut j = self.job.lock().unwrap();
            (j.bytes, j.total) = (0, total);
        }
        let part = PathBuf::from(format!("{}.part", to.display()));
        let mut file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("{url}: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
            self.job.lock().unwrap().bytes += n as u64;
        }
        drop(file);
        std::fs::rename(&part, to).map_err(|e| format!("{}: {e}", to.display()))
    }

    /// The file of an area's folder that the name `<name>/<file>` is, or None.
    fn area_dir(&self, name: &str) -> Option<PathBuf> {
        is_name(name).then(|| self.out.join(name))
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
                (dir.join(file), Some(url.to_string()))
            }
            _ => return Err("the request has no data: a Geofabrik extract or a file".into()),
        };
        let dem = b["dem"].is_object().then(|| b["dem"].clone());
        if dem.is_some() && !dir.join("dem.tif").is_file() {
            return Err("the request has a DEM, but the server has no dem.tif for this area: send it with PUT /api/dem/<name> first".into());
        }
        {
            let mut j = self.job.lock().unwrap();
            if matches!(j.state, "downloading" | "building") {
                return Err(format!("the build of {} runs: wait for it", j.name));
            }
            *j = Job { state: "building", name: name.clone(), db: dir.join(format!("{name}.duckdb")).display().to_string(), started: Some(Instant::now()), ..Job::default() };
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
            match r {
                Ok(text) => (j.state, j.text) = ("done", text),
                Err(e) => (j.state, j.error) = ("error", e),
            }
        });
        Ok(())
    }

    fn step(&self, line: &str) {
        let mut j = self.job.lock().unwrap();
        if j.steps.len() < 500 {
            j.steps.push(line.to_string());
        }
    }

    /// The work of one build: the download, the config, the build, and the page of the result.
    fn run(&self, name: &str, dir: &Path, pbf: &Path, url: Option<&str>, dem: Option<&Value>, settings: &str) -> Result<String, String> {
        if let Some(url) = url {
            if pbf.is_file() {
                self.step(&format!("The folder has {} already: not downloaded again", pbf.file_name().unwrap_or_default().to_string_lossy()));
            } else {
                self.job.lock().unwrap().state = "downloading";
                self.step(&format!("Download {url}"));
                self.download(url, pbf)?;
                self.job.lock().unwrap().state = "building";
            }
        }
        // the page of the area that was built before closes its database: the build may write the same file
        if let Some(old) = self.edit.lock().unwrap().take() {
            old.close();
        }
        let config = config_text(name, &pbf.display().to_string(), dem, settings);
        std::fs::write(dir.join("config.yaml"), &config).map_err(|e| e.to_string())?;
        let db = dir.join(format!("{name}.duckdb"));
        let text = self.host.build(&config, dir, &db, &|line| self.step(line))?;
        self.step("Make the page of the result");
        let site = Site::with_pager(self.host.db(&db), &db, &dir.join("osm_fixes.yaml"), self.port, Mode::Edit, self.pager)?;
        *self.edit.lock().unwrap() = Some(Arc::new(site));
        Ok(text)
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
                None => req.say("404 Not Found", "text/plain", b"no area is built yet"),
            };
        }
        if path.is_empty() || path == "index.html" || path.starts_with("parts/") || path.starts_with("vendor/") || path.starts_with("roadstyle/") {
            if !read {
                return req.say("405 Method Not Allowed", "text/plain", b"the page's files are read only");
            }
            let found: Option<(&str, &[u8])> = if path.is_empty() || path == "index.html" {
                Some(("text/html; charset=utf-8", PAGE.as_bytes()))
            } else if let Some(f) = path.strip_prefix("parts/") {
                PARTS.iter().find(|p| p.0 == f).map(|p| (p.1, p.2.as_bytes()))
            } else if let Some(f) = path.strip_prefix("vendor/") {
                match f { "maplibre-gl.js" => Some(("text/javascript", MAPLIBRE_JS.as_bytes())), "maplibre-gl.css" => Some(("text/css", MAPLIBRE_CSS.as_bytes())), _ => None }
            } else {
                let f = &path["roadstyle/".len()..];
                let kind = if f.ends_with(".js") { Some("text/javascript") } else if f == "dashboard.css" { Some("text/css") } else { None };
                kind.zip(inside(Path::new(""), f)).and_then(|(k, _)| roadstyle::assets::get(&format!("static/dashboard/{f}")).map(|b| (k, b)))
            };
            return match found {
                Some((kind, body)) => req.say("200 OK", kind, body),
                None => req.say("404 Not Found", "text/plain", b"not found"),
            };
        }
        let api = path.strip_prefix("api/").unwrap_or("").to_string();
        match (req.method.as_str(), api.as_str()) {
            ("GET" | "HEAD", "info") => {
                let info = json!({"out": self.out.display().to_string(), "geofabrik": self.index, "edit": self.edit.lock().unwrap().is_some()});
                req.say("200 OK", "application/json", info.to_string().as_bytes())
            }
            ("GET" | "HEAD", "job") => {
                let j = self.job.lock().unwrap().json();
                req.say("200 OK", "application/json", j.to_string().as_bytes())
            }
            ("GET" | "HEAD", "geofabrik") => match self.geofabrik() {
                Ok(t) => req.say("200 OK", "application/json", t.as_bytes()),
                Err(e) => req.say("502 Bad Gateway", "text/plain", e.as_bytes()),
            },
            ("POST", "file" | "build") | ("PUT", _) if api == "file" || api == "build" || api.starts_with("dem/") => {
                let own = self.own.clone();
                let Some(body) = req.write_body(&own)? else { return Ok(()) };
                let r: Result<String, String> = match (req.method.as_str(), api.as_str()) {
                    ("POST", "file") => {
                        let p = PathBuf::from(String::from_utf8_lossy(&body).trim());
                        std::fs::metadata(&p).ok().filter(|m| m.is_file()).map(|m| json!({"size": m.len()}).to_string())
                            .ok_or_else(|| format!("{}: not a file", p.display()))
                    }
                    ("POST", "build") => self.start(&body).map(|_| "{}".to_string()),
                    ("PUT", a) if a.starts_with("dem/") => self.dem(&a["dem/".len()..], &body).map(|_| "{}".to_string()),
                    _ => Err("not this method".into()),
                };
                match r {
                    Ok(t) => req.say("200 OK", "application/json", t.as_bytes()),
                    Err(e) => req.say("400 Bad Request", "text/plain", e.as_bytes()),
                }
            }
            (_, "info" | "job" | "geofabrik" | "file" | "build") => req.say("405 Method Not Allowed", "text/plain", b"not this method"),
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
        let out = s.out.clone();
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
        for f in ["parts/studio.js", "parts/ground_step/ground_step.js", "parts/build_steps/area_step.js", "parts/data/dem_sources.json", "vendor/maplibre-gl.js", "roadstyle/search.js", "roadstyle/dashboard.css"] {
            assert!(get(f).starts_with("HTTP/1.1 200"), "{f}");
        }
        for f in ["parts/../Cargo.toml", "parts/nothing.js", "vendor/other.js", "roadstyle/../x.js", "roadstyle/dashboard.html", "edit/", "api/nothing"] {
            assert!(get(f).starts_with("HTTP/1.1 404"), "{f}");
        }
        assert!(ask(&format!("PUT /parts/studio.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n")).starts_with("HTTP/1.1 405"));
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
    fn a_name_is_a_folder_name() {
        for ok in ["monaco", "tartu-1", "a.b_c"] { assert!(is_name(ok), "{ok}"); }
        for bad in ["", ".", "..", ".hidden", "a/b", "a b", "ä", &"x".repeat(65)] { assert!(!is_name(bad), "{bad}"); }
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
        assert!(job.contains("read ways") && job.contains("elevation grid"), "the steps of the engine are in the job: {job}");
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
