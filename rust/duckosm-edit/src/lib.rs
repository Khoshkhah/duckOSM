//! `duckosm edit` (docs/design/edit_page.md): a page on 127.0.0.1 that edits the OSM fixes of a duckOSM database. The page is roadstyle's
//! standard dashboard of the database (`page`), with duckOSM's fixes editor (web/fixes_editor/) in its panel. The server answers only on
//! 127.0.0.1, with the rules of roadstyle-serve: a request must name this server in `Host` (no DNS rebinding), a write must come from this
//! server's page (`Origin`) and have a `Content-Length`, a body is at most `MAX_BODY` bytes, and the request line and the headers at most
//! `MAX_HEAD` bytes. The one file that the page writes is the fixes file. The editor's SQL runs on the database, read only and with no
//! access to files (`LOCK`). Build again runs the build of the database again with the fixes file, and the page then has the new network.
//!
//! The routes: `GET /` the page; `GET /fixes_editor/<file>` the editor; `GET /ask/questions.js`, `GET /ask/schema.js` and `GET /ask/views.js` what Ask knows; `GET /api/info` the area, the tables and why Build again is off;
//! `GET` and `PUT /api/fixes` the fixes file; `POST /api/query` the rows of one SELECT as JSON; `POST /api/build` Build again.
//!
//! `duckosm ask` (docs/design/ask_page.md) is the same server with `Mode::Ask`: the page has roadstyle's data dock and Ask in place of the editor. It
//! adds `GET /roadstyle/<file>` (roadstyle's dashboard modules, which the page imports), `POST /api/sql` (a SELECT with its columns and types, for the
//! data dock) and `POST /api/table` (the changes of the fixes file as the table `extra.edits`, for Ask). Build again is off.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(feature = "duckdb")]
pub mod native;
pub mod page;
pub mod studio;

/// The largest body of a request (a fixes file, a query), and of the request line and the headers.
pub const MAX_BODY: usize = 16 << 20;
const MAX_HEAD: usize = 64 << 10;

/// After the page is made, the editor's queries can read the tables and nothing else: no file (COPY, read_csv, ATTACH, INSTALL) and no
/// change of a setting. The file is open read only.
pub const LOCK: &str = "SET enable_external_access = false; SET lock_configuration = true;";

/// What the server needs of the database. The program has the native DuckDB (`native`); the Python module gives the duckdb package of
/// Python (src/duckosm/edit.py), because the module has no DuckDB inside.
pub trait Db: Send {
    /// Open the file read only, with the spatial extension loaded.
    fn open(&mut self) -> Result<(), String>;
    /// Close the file, so that a build can write it again.
    fn close(&mut self);
    /// Run statements that give no rows.
    fn execute(&mut self, sql: &str) -> Result<(), String>;
    /// The first column of each row of a query, as text.
    fn texts(&mut self, sql: &str) -> Result<Vec<Option<String>>, String>;
    /// Why the build of `config` (YAML, as `main.visualization_metadata.build_config` has it) cannot run; Ok when it can.
    fn can_build(&mut self, _config: &str, _dir: &Path) -> Result<(), String> {
        Ok(())
    }
    /// Build the database `db` again from `config`, in the folder `dir` (the build's relative paths), with the fixes file `fixes`. The
    /// file is closed. Gives the build's summary.
    fn build(&mut self, config: &str, dir: &Path, fixes: &Path, db: &Path) -> Result<String, String>;
}

/// The rows of a query as JSON objects, one for each row, in the order of the columns (DuckDB's `to_json`).
pub fn rows(db: &mut dyn Db, sql: &str) -> Result<Vec<Value>, String> {
    db.texts(&format!("SELECT to_json(duckosm_row)::VARCHAR FROM ({sql}) duckosm_row"))?.into_iter()
        .map(|t| serde_json::from_str(&t.unwrap_or_else(|| "null".into())).map_err(|e| format!("a row: {e}"))).collect()
}

/// The database of one server, and what is made from it: the page, the tables, and the build's record.
struct State {
    db: Box<dyn Db>,
    page: String,
    tables: Value,
    /// The record of the build (`build_config`, `build_dir` of main.visualization_metadata), or why Build again is off.
    source: Result<(String, PathBuf), String>,
}

/// What makes the page from the open database and the area's name.
pub type Pager = fn(&mut dyn Db, &str) -> Result<String, String>;

/// What the page is for: the fixes editor (`duckosm edit`), or roadstyle's data dock and Ask (`duckosm ask`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Edit,
    Ask,
}

/// One server: the database, the fixes file, and the names that the server answers to.
pub struct Site {
    pub db: PathBuf,
    pub fixes: PathBuf,
    pub own: [String; 2],
    pub mode: Mode,
    state: Mutex<State>,
    /// What makes the page of the open database (`page::make`; the tests give a placeholder).
    pager: Pager,
    /// The fixes file is copied to `<fixes>.bak` before the page writes it the first time.
    backed_up: Mutex<bool>,
    /// A file that keeps the page of the database (`duckosm studio`: `<name>.map.html` in the folder of the area), or None.
    cache: Option<PathBuf>,
}

impl Site {
    /// Open `db` and make the page. `fixes`: the fixes file (it need not exist); `port`: the port of the server.
    pub fn new(db: Box<dyn Db>, db_path: &Path, fixes: &Path, port: u16) -> Result<Site, String> {
        Site::with_mode(db, db_path, fixes, port, Mode::Edit)
    }

    /// As `new`, for the page of `mode`.
    pub fn with_mode(db: Box<dyn Db>, db_path: &Path, fixes: &Path, port: u16, mode: Mode) -> Result<Site, String> {
        let pager: Pager = if mode == Mode::Ask { page::make_ask } else { page::make };
        Site::with_pager(db, db_path, fixes, port, mode, pager)
    }

    /// As `with_mode`, with the function that makes the page (the tests of `studio` give a placeholder).
    pub fn with_pager(db: Box<dyn Db>, db_path: &Path, fixes: &Path, port: u16, mode: Mode, pager: Pager) -> Result<Site, String> {
        Site::with_cache(db, db_path, fixes, port, mode, pager, None)
    }

    /// As `with_pager`, and the page is kept in the file `cache`: a later open reads the file when it is newer than the database, and does not make the page
    /// again (Burnaby: 12 s and 45 MB to make). A database that changed (a build, Build again) makes the page and the file again.
    pub fn with_cache(mut db: Box<dyn Db>, db_path: &Path, fixes: &Path, port: u16, mode: Mode, pager: Pager, cache: Option<PathBuf>) -> Result<Site, String> {
        let (page, tables, source) = load(db.as_mut(), db_path, pager, cache.as_deref())?;
        Ok(Site { db: db_path.to_path_buf(), fixes: fixes.to_path_buf(), own: [format!("127.0.0.1:{port}"), format!("localhost:{port}")], mode,
                  state: Mutex::new(State { db, page, tables, source }), pager, backed_up: Mutex::new(false), cache })
    }

    /// The page of the open database (the HTML that `GET /` gives).
    pub fn page(&self) -> String {
        self.state.lock().unwrap().page.clone()
    }

    /// The address of the page.
    pub fn url(&self) -> String {
        format!("http://{}/", self.own[0])
    }

    fn info(&self) -> Value {
        let s = self.state.lock().unwrap();
        let name = self.db.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        json!({"name": name, "db": self.db.display().to_string(), "fixes": self.fixes.display().to_string(), "exists": self.fixes.exists(),
               "tables": s.tables, "build_off": s.source.as_ref().err().cloned().unwrap_or_default()})
    }

    /// Build again: the database closes, the build writes it again, and the page is made again.
    fn rebuild(&self) -> Result<String, String> {
        if self.mode == Mode::Ask {
            return Err("duckosm ask does not build. Use duckosm edit to change the data.".into());
        }
        let mut s = self.state.lock().unwrap();
        let (config, dir) = s.source.clone()?;
        if !self.fixes.exists() {
            return Err(format!("{}: the file is not there: add a change first", self.fixes.display()));
        }
        s.db.close();
        let built = s.db.build(&config, &dir, &self.fixes, &self.db);
        // the database opens again also after a failed build (the old file, or what the build left)
        let (page, tables, source) = load(s.db.as_mut(), &self.db, self.pager, self.cache.as_deref())?;
        (s.page, s.tables, s.source) = (page, tables, source);
        built
    }

    fn query(&self, sql: &str) -> Result<String, String> {
        let mut s = self.state.lock().unwrap();
        Ok(Value::Array(rows(s.db.as_mut(), sql)?).to_string())
    }

    /// A SELECT for the data dock: `{"columns": [{name, type}], "rows": [[...]]}`. A 64-bit integer (an edge id) goes as a text, so that the
    /// page keeps all its digits (JSON numbers of a browser have 53 bits): the page makes a BigInt of it from the type.
    /// ponytail: a result with two columns of one name is refused by DuckDB (it needs a name for each).
    fn sql(&self, sql: &str) -> Result<String, String> {
        let sql = sql.trim().trim_end_matches(';');
        let mut s = self.state.lock().unwrap();
        let db = s.db.as_mut();
        let cols = rows(db, &format!("SELECT column_name AS name, column_type AS type FROM (DESCRIBE {sql})"))?;
        let q = |n: &str| format!("\"{}\"", n.replace('"', "\"\""));
        let list: Vec<String> = cols.iter().map(|c| {
            let (n, t) = (c["name"].as_str().unwrap_or_default(), c["type"].as_str().unwrap_or_default());
            if matches!(t, "BIGINT" | "UBIGINT" | "HUGEINT" | "UHUGEINT") { format!("CAST({} AS VARCHAR) AS {}", q(n), q(n)) } else { q(n) }
        }).collect();
        let data = rows(db, &format!("SELECT {} FROM ({sql}) duckosm_sql", list.join(", ")))?;
        let rows: Vec<Value> = data.into_iter().map(|r| Value::Array(r.as_object().map(|o| o.values().cloned().collect()).unwrap_or_default())).collect();
        Ok(json!({"columns": cols, "rows": rows}).to_string())
    }

    /// The table `extra.<name>` from `{"name", "columns": [{name, type}], "rows": [[...]]}`: the page's own data, which Ask reads
    /// (`extra` is an attached file database, `page::make_ask`). Names are words, types are from a short list, and values are written as SQL
    /// literals here, so the body is data and never SQL.
    fn table(&self, body: &[u8]) -> Result<(), String> {
        let t: Value = serde_json::from_slice(body).map_err(|e| format!("the table: {e}"))?;
        let word = |w: &str| !w.is_empty() && w.len() <= 63 && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        let name = t["name"].as_str().filter(|n| word(n)).ok_or("the table needs a name of letters, digits and _")?;
        let cols = t["columns"].as_array().filter(|c| !c.is_empty()).ok_or("the table needs columns")?;
        let mut ddl = Vec::new();
        let mut types = Vec::new();
        for c in cols {
            let (n, ty) = (c["name"].as_str().unwrap_or_default(), c["type"].as_str().unwrap_or_default());
            if !word(n) || !matches!(ty, "VARCHAR" | "BIGINT" | "DOUBLE" | "BOOLEAN" | "TIMESTAMPTZ") {
                return Err(format!("the column {n:?} {ty:?} is not allowed"));
            }
            ddl.push(format!("{n} {ty}"));
            types.push(ty);
        }
        let lit = |v: &Value, ty: &str| -> Result<String, String> {
            Ok(match v {
                Value::Null => "NULL".into(),
                Value::Bool(b) if ty == "BOOLEAN" => b.to_string().to_uppercase(),
                Value::Number(n) if matches!(ty, "BIGINT" | "DOUBLE") => n.to_string(),
                Value::String(x) if ty == "VARCHAR" => format!("'{}'", x.replace('\'', "''")),
                Value::String(x) if ty == "BIGINT" => x.parse::<i64>().map_err(|_| format!("{x:?} is not a BIGINT"))?.to_string(),
                Value::String(x) if ty == "TIMESTAMPTZ" => format!("TRY_CAST('{}' AS TIMESTAMPTZ)", x.replace('\'', "''")),
                _ => return Err(format!("a value {v} does not fit {ty}")),
            })
        };
        let rows = t["rows"].as_array().ok_or("the table needs rows")?;
        let mut s = self.state.lock().unwrap();
        s.db.execute(&format!("CREATE OR REPLACE TABLE extra.{name} ({})", ddl.join(", ")))?;
        for chunk in rows.chunks(500) {
            let vals = chunk.iter().map(|r| {
                let r = r.as_array().filter(|r| r.len() == types.len()).ok_or("a row has not the number of columns")?;
                Ok(format!("({})", r.iter().zip(&types).map(|(v, ty)| lit(v, ty)).collect::<Result<Vec<_>, String>>()?.join(", ")))
            }).collect::<Result<Vec<_>, String>>()?;
            s.db.execute(&format!("INSERT INTO extra.{name} VALUES {}", vals.join(", ")))?;
        }
        Ok(())
    }

    /// Write the fixes file (the first write of this server keeps the file before it as `<fixes>.bak`).
    fn save(&self, body: &[u8]) -> Result<(), String> {
        let mut once = self.backed_up.lock().unwrap();
        if !*once && self.fixes.exists() {
            let bak = PathBuf::from(format!("{}.bak", self.fixes.display()));
            std::fs::copy(&self.fixes, &bak).map_err(|e| format!("{}: {e}", bak.display()))?;
        }
        *once = true;
        std::fs::write(&self.fixes, body).map_err(|e| format!("{}: {e}", self.fixes.display()))
    }
}

/// Open the database, make the page, read the tables and the build's record, then lock the database (`LOCK`).
fn load(db: &mut dyn Db, path: &Path, pager: Pager, cache: Option<&Path>) -> Result<(String, Value, Result<(String, PathBuf), String>), String> {
    db.open()?;
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "area".into());
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    // the kept page is used when it is newer than the database and than this program (a new program can make another page)
    let program = std::env::current_exe().ok().and_then(|p| modified(&p));
    let kept = cache.filter(|c| matches!((modified(c), modified(path)), (Some(c), Some(d)) if c >= d && program.is_none_or(|p| c >= p)))
        .and_then(|c| std::fs::read_to_string(c).ok());
    let page = match kept {
        Some(page) => page,
        None => {
            let page = pager(db, &name)?;
            if let Some(c) = cache {
                if let Err(e) = std::fs::write(c, &page) {
                    eprintln!("duckosm: the page is not kept in {}: {e}", c.display());
                }
            }
            page
        }
    };
    let tables: Vec<Value> = rows(db, "SELECT table_schema AS mode, table_name AS \"table\" FROM information_schema.tables WHERE table_catalog = current_database()")?;
    let source = source(db);
    db.execute(LOCK)?;
    Ok((page, Value::Array(tables), source))
}

/// The build's record in the database: `build_config` (the config as YAML) and `build_dir` (the folder of the build) of
/// main.visualization_metadata, which `duckosm build` and `duckosm-rust --config` write. Err: why Build again is off.
fn source(db: &mut dyn Db) -> Result<(String, PathBuf), String> {
    const NONE: &str = "This database has no record of its build (the config and the PBF file). Build it again with duckosm build, \
                        or with duckosm-rust --config, then Build again works here.";
    let has = rows(db, "SELECT count(*) AS n FROM information_schema.columns WHERE table_schema = 'main' AND table_name = 'visualization_metadata' \
                        AND column_name IN ('build_config', 'build_dir')")?;
    if has.first().and_then(|r| r["n"].as_i64()) != Some(2) {
        return Err(NONE.into());
    }
    let r = rows(db, "SELECT build_config, build_dir FROM main.visualization_metadata LIMIT 1")?;
    let (Some(config), Some(dir)) = (r.first().and_then(|r| r["build_config"].as_str()), r.first().and_then(|r| r["build_dir"].as_str())) else {
        return Err(NONE.into());
    };
    let dir = PathBuf::from(dir);
    db.can_build(config, &dir).map_err(|e| format!("Build again cannot run: {e}"))?;
    Ok((config.to_string(), dir))
}

/// The files of the fixes editor, from the library `duckosm-editor` (web/fixes_editor/ of this repository).
fn editor_file(name: &str) -> Option<&'static str> {
    duckosm_editor::file(name).map(|f| f.text)
}

/// The files of Ask that the page and duckOSM Studio share, in the program (web/ask/ of this repository).
fn ask_file(name: &str) -> Option<&'static str> {
    Some(match name {
        "questions.js" => include_str!("../../../web/ask/questions.js"),
        "schema.js" => include_str!("../../../web/ask/schema.js"),
        "views.js" => include_str!("../../../web/ask/views.js"),
        _ => return None,
    })
}

/// Bind 127.0.0.1:`port` (0: a free port).
pub fn bind(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("127.0.0.1:{port}: {e}"))
}

/// Answer the requests of `listener` until the program stops.
pub fn serve(site: Arc<Site>, listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        let site = site.clone();
        std::thread::spawn(move || {
            let r = read_request(stream, &site.own).and_then(|req| req.map_or(Ok(()), |mut req| site.route(&mut req)));
            if let Err(e) = r {
                eprintln!("duckosm edit: {e}");
            }
        });
    }
}

/// One request that passed the rules of the server: its method, its path (no query, no first `/`), its headers, and the stream to answer on.
pub struct Req {
    pub method: String,
    pub path: String,
    /// The query of the address (after `?`, before `#`), or empty.
    pub query: String,
    headers: Vec<(String, String)>,
    reader: BufReader<TcpStream>,
    out: TcpStream,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|h| h.0.eq_ignore_ascii_case(name)).map(|h| h.1.as_str())
    }

    /// A GET or a HEAD.
    pub fn read(&self) -> bool {
        matches!(self.method.as_str(), "GET" | "HEAD")
    }

    /// Answer. A HEAD has no body.
    pub fn say(&mut self, status: &str, kind: &str, body: &[u8]) -> Result<(), String> {
        let body = if self.method == "HEAD" { &[][..] } else { body };
        reply(&mut self.out, status, kind, body)
    }

    /// The body of a write (PUT, POST): from this server's page only (a browser sends Origin with a PUT and a POST), with a Content-Length
    /// (a chunked body is refused: its size is known only at its end) of at most `MAX_BODY`. `Ok(None)`: the request was refused and answered.
    pub fn write_body(&mut self, own: &[String; 2]) -> Result<Option<Vec<u8>>, String> {
        if self.header("origin").is_some_and(|o| !own.iter().any(|h| o == format!("http://{h}"))) {
            self.say("403 Forbidden", "text/plain", b"not this server's page")?;
            return Ok(None);
        }
        let Some(length) = self.header("content-length").and_then(|v| v.parse::<usize>().ok()) else {
            self.say("411 Length Required", "text/plain", b"a write needs a Content-Length")?;
            return Ok(None);
        };
        if length > MAX_BODY {
            self.say("413 Content Too Large", "text/plain", b"more than 16 MB")?;
            return Ok(None);
        }
        let mut body = vec![0u8; length];
        self.reader.read_exact(&mut body).map_err(|e| e.to_string())?;
        Ok(Some(body))
    }
}

/// Read the request line and the headers (at most `MAX_HEAD` bytes, up to the empty line) and check the rules that hold for every route: an HTTP
/// request, and this server's name in `Host` (`own`). `Ok(None)`: the request was refused and answered.
pub fn read_request(stream: TcpStream, own: &[String; 2]) -> Result<Option<Req>, String> {
    let mut out = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut head = Vec::new();
    {
        let mut limited = (&mut reader).take(MAX_HEAD as u64);
        loop {
            let start = head.len();
            if limited.read_until(b'\n', &mut head).map_err(|e| e.to_string())? == 0 {
                if limited.limit() == 0 {
                    reply(&mut out, "431 Request Header Fields Too Large", "text/plain", b"headers too large")?;
                    return Ok(None);
                }
                break;
            }
            if head[start..].iter().all(u8::is_ascii_whitespace) {
                break;
            }
        }
    }
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(&head) {
        Ok(httparse::Status::Complete(_)) => {}
        Err(httparse::Error::TooManyHeaders) => return reply(&mut out, "431 Request Header Fields Too Large", "text/plain", b"too many headers").map(|_| None),
        _ => return reply(&mut out, "400 Bad Request", "text/plain", b"not an HTTP request").map(|_| None),
    }
    let headers: Vec<(String, String)> = req.headers.iter().filter_map(|h| Some((h.name.to_string(), std::str::from_utf8(h.value).ok()?.trim().to_string()))).collect();
    // this server only: a page of another site reaches 127.0.0.1 through DNS rebinding with its own name in Host
    if !headers.iter().any(|h| h.0.eq_ignore_ascii_case("host") && own.iter().any(|o| *o == h.1)) {
        reply(&mut out, "403 Forbidden", "text/plain", b"not this server's Host")?;
        return Ok(None);
    }
    let target = req.path.unwrap_or("");
    let path = target.split(['?', '#']).next().unwrap_or("/").trim_start_matches('/').to_string();
    let query = target.split('#').next().unwrap_or("").split_once('?').map_or("", |q| q.1).to_string();
    Ok(Some(Req { method: req.method.unwrap_or("").to_string(), path, query, headers, reader, out }))
}

impl Site {
    /// Answer one request.
    pub fn route(&self, req: &mut Req) -> Result<(), String> {
        let site = self;
        let (method, path) = (req.method.clone(), req.path.clone());
        let read = req.read();
        // the page and the editor's files: read only
        // roadstyle's dashboard modules that the page of `duckosm ask` imports (the data dock and Ask): only .js files of the dashboard, read only
        if let Some(f) = path.strip_prefix("roadstyle/") {
            let file = read.then(|| inside(Path::new(""), f)).flatten().filter(|_| f.ends_with(".js"));
            return match file.and_then(|_| roadstyle::assets::get(&format!("static/dashboard/{f}"))) {
                Some(b) => req.say("200 OK", "text/javascript", b),
                None => req.say("404 Not Found", "text/plain", b"not found"),
            };
        }
        if path.is_empty() || path == "index.html" || path.starts_with("fixes_editor/") || path.starts_with("ask/") {
            if !read {
                return req.say("405 Method Not Allowed", "text/plain", b"the page's files are read only");
            }
            if let Some(f) = path.strip_prefix("ask/") {
                return match ask_file(f) {
                    Some(t) => req.say("200 OK", "text/javascript", t.as_bytes()),
                    None => req.say("404 Not Found", "text/plain", b"not found"),
                };
            }
            if let Some(f) = path.strip_prefix("fixes_editor/") {
                return match editor_file(f) {
                    Some(t) => req.say("200 OK", kind(f), t.as_bytes()),
                    None => req.say("404 Not Found", "text/plain", b"not found"),
                };
            }
            let page = site.state.lock().unwrap().page.clone();
            return req.say("200 OK", "text/html; charset=utf-8", page.as_bytes());
        }
        let api = path.strip_prefix("api/").unwrap_or("");
        match (method.as_str(), api) {
            ("GET" | "HEAD", "info") => req.say("200 OK", "application/json", site.info().to_string().as_bytes()),
            ("GET" | "HEAD", "fixes") => match std::fs::read(&site.fixes) {
                Ok(b) => req.say("200 OK", "text/yaml; charset=utf-8", &b),
                Err(_) => req.say("404 Not Found", "text/plain", b"no fixes file yet"),
            },
            ("PUT", "fixes") | ("POST", "query" | "build" | "sql" | "table") => {
                let Some(body) = req.write_body(&site.own)? else { return Ok(()) };
                match api {
                    "fixes" => match site.save(&body) {
                        Ok(()) => req.say("200 OK", "text/plain", b"saved"),
                        Err(e) => req.say("500 Internal Server Error", "text/plain", e.as_bytes()),
                    },
                    "query" => match site.query(&String::from_utf8_lossy(&body)) {
                        Ok(j) => req.say("200 OK", "application/json", j.as_bytes()),
                        Err(e) => req.say("400 Bad Request", "text/plain", e.as_bytes()),
                    },
                    "sql" => match site.sql(&String::from_utf8_lossy(&body)) {
                        Ok(j) => req.say("200 OK", "application/json", j.as_bytes()),
                        Err(e) => req.say("400 Bad Request", "text/plain", e.as_bytes()),
                    },
                    "table" if site.mode == Mode::Ask => match site.table(&body) {
                        Ok(()) => req.say("200 OK", "text/plain", b"saved"),
                        Err(e) => req.say("400 Bad Request", "text/plain", e.as_bytes()),
                    },
                    "table" => req.say("404 Not Found", "text/plain", b"not found"),
                    _ => {
                        let r = site.rebuild();
                        let j = match &r { Ok(t) => json!({"ok": true, "text": t}), Err(e) => json!({"ok": false, "error": e}) };
                        req.say("200 OK", "application/json", j.to_string().as_bytes())
                    }
                }
            }
            (_, "info" | "fixes" | "query" | "build" | "sql" | "table") => req.say("405 Method Not Allowed", "text/plain", b"not this method"),
            _ => req.say("404 Not Found", "text/plain", b"not found"),
        }
    }

    /// Close the database (a build can write the file).
    pub fn close(&self) {
        self.state.lock().unwrap().db.close();
    }

    /// Open the database again after `close` (another connection wrote the file: card 8 of `duckosm studio` saved the levels).
    pub fn reopen(&self) -> Result<(), String> {
        self.state.lock().unwrap().db.open()
    }
}

/// `rel` under `root`, or None for a path with `..`, a root or a drive in it.
pub fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    rel.components().all(|c| matches!(c, Component::Normal(_))).then(|| root.join(rel))
}

fn kind(file: &str) -> &'static str {
    match Path::new(file).extension().and_then(|e| e.to_str()).unwrap_or("") {
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

fn reply(out: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> Result<(), String> {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    out.write_all(head.as_bytes()).and_then(|_| out.write_all(body)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database with no tables: the queries give what `answers` has for them, and a build writes a mark.
    struct Fake {
        built: Arc<Mutex<Vec<String>>>,
        /// The statements that `execute` got.
        ran: Arc<Mutex<Vec<String>>>,
    }

    impl Db for Fake {
        fn open(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn close(&mut self) {}
        fn execute(&mut self, sql: &str) -> Result<(), String> {
            self.ran.lock().unwrap().push(sql.to_string());
            Ok(())
        }
        fn texts(&mut self, sql: &str) -> Result<Vec<Option<String>>, String> {
            Ok(if sql.contains("count(*) AS n") { vec![Some(r#"{"n": 2}"#.into())] }
               else if sql.contains("build_config") { vec![Some(r#"{"build_config": "name: a", "build_dir": "/tmp"}"#.into())] }
               else if sql.contains("FROM (SELECT 1 AS x) duckosm_row") { vec![Some(r#"{"x": 1}"#.into())] }
               else if sql.contains("(DESCRIBE SELECT 1 AS x)") { vec![Some(r#"{"name": "x", "type": "BIGINT"}"#.into())] }
               else if sql.contains("CAST(\"x\" AS VARCHAR) AS \"x\" FROM (SELECT 1 AS x)") { vec![Some(r#"{"x": "1"}"#.into())] }
               else { vec![] })
        }
        fn build(&mut self, config: &str, _dir: &Path, fixes: &Path, _db: &Path) -> Result<String, String> {
            self.built.lock().unwrap().push(format!("{config} with {}", std::fs::read_to_string(fixes).unwrap()));
            Ok("built".into())
        }
    }

    /// A server on a free port with the fake database (the page is a placeholder) and a fixes file in a new folder.
    fn server(tag: &str) -> (PathBuf, impl Fn(&str) -> String, u16, Arc<Mutex<Vec<String>>>) {
        let (fixes, ask, port, built, _) = server_with(tag, Mode::Edit);
        (fixes, ask, port, built)
    }

    #[allow(clippy::type_complexity)]
    fn server_with(tag: &str, mode: Mode) -> (PathBuf, impl Fn(&str) -> String, u16, Arc<Mutex<Vec<String>>>, Arc<Mutex<Vec<String>>>) {
        let dir = std::env::temp_dir().join(format!("duckosm-edit-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fixes = dir.join("osm_fixes.yaml");
        let _ = std::fs::remove_file(&fixes);
        let listener = bind(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let built = Arc::new(Mutex::new(Vec::new()));
        let ran = Arc::new(Mutex::new(Vec::new()));
        let site = Site { db: dir.join("area.duckdb"), fixes: fixes.clone(), own: [format!("127.0.0.1:{port}"), format!("localhost:{port}")], mode,
                          state: Mutex::new(State { db: Box::new(Fake { built: built.clone(), ran: ran.clone() }), page: "the page".into(), tables: json!([]),
                                                    source: Ok(("name: a".into(), dir.clone())) }),
                          pager: |_, _| Ok("the page".into()), backed_up: Mutex::new(false), cache: None };
        std::thread::spawn(move || serve(Arc::new(site), listener));
        let ask = move |req: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(req.as_bytes()).ok();
            let mut text = String::new();
            s.read_to_string(&mut text).ok();
            text
        };
        (fixes, ask, port, built, ran)
    }

    #[test]
    fn only_paths_inside_the_root() {
        let root = Path::new("/srv/area");
        assert_eq!(inside(root, "a.yaml"), Some(root.join("a.yaml")));
        assert!(inside(root, "../secret").is_none() && inside(root, "/etc/passwd").is_none() && inside(root, "a/../../b").is_none());
    }

    #[test]
    fn the_editor_is_in_the_program() {
        for f in ["fixes_panel.js", "fixes.js", "fixes_panel.css", "presets.json"] {
            assert!(editor_file(f).is_some_and(|t| !t.is_empty()), "{f}");
        }
        assert!(editor_file("README.md").is_none() && editor_file("../Cargo.toml").is_none());
    }

    #[test]
    fn save_and_read_the_fixes_file() {
        let (fixes, ask, port, _) = server("save");
        let host = format!("Host: 127.0.0.1:{port}\r\n");
        assert!(ask(&format!("GET / HTTP/1.1\r\n{host}\r\n")).ends_with("the page"));
        assert!(ask(&format!("GET /api/fixes HTTP/1.1\r\n{host}\r\n")).starts_with("HTTP/1.1 404"), "no file before the first save");
        let put = |body: &str| format!("PUT /api/fixes HTTP/1.1\r\n{host}Origin: http://127.0.0.1:{port}\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        assert!(ask(&put("overrides: []\n")).starts_with("HTTP/1.1 200"));
        assert_eq!(std::fs::read_to_string(&fixes).unwrap(), "overrides: []\n", "the first save makes the file");
        assert!(ask(&format!("GET /api/fixes HTTP/1.1\r\n{host}\r\n")).ends_with("overrides: []\n"));
        assert!(ask(&put("new_ways: []\n")).starts_with("HTTP/1.1 200"));
        assert_eq!(std::fs::read_to_string(&fixes).unwrap(), "new_ways: []\n");
        assert!(!PathBuf::from(format!("{}.bak", fixes.display())).exists(), "a file that this server made has no .bak");
        let info = ask(&format!("GET /api/info HTTP/1.1\r\n{host}\r\n"));
        assert!(info.contains("\"exists\":true") && info.contains("\"build_off\":\"\""), "{info}");
    }

    #[test]
    fn an_old_file_is_kept_before_the_first_save() {
        let (fixes, ask, port, _) = server("bak");
        std::fs::write(&fixes, "# my notes\noverrides: []\n").unwrap();
        let put = format!("PUT /api/fixes HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 3\r\n\r\nnew");
        assert!(ask(&put).starts_with("HTTP/1.1 200"));
        assert_eq!(std::fs::read_to_string(format!("{}.bak", fixes.display())).unwrap(), "# my notes\noverrides: []\n");
        assert!(ask(&put).starts_with("HTTP/1.1 200"));
        assert_eq!(std::fs::read_to_string(format!("{}.bak", fixes.display())).unwrap(), "# my notes\noverrides: []\n", "only the first save");
    }

    #[test]
    fn another_host_or_origin_is_refused() {
        let (fixes, ask, port, _) = server("host");
        assert!(ask("GET / HTTP/1.1\r\nHost: evil.example:80\r\n\r\n").starts_with("HTTP/1.1 403"));
        assert!(ask("GET / HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 403"));
        assert!(ask(&format!("GET / HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n")).starts_with("HTTP/1.1 200"));
        for (method, path) in [("PUT", "fixes"), ("POST", "query"), ("POST", "build")] {
            let req = format!("{method} /api/{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://evil.example\r\nContent-Length: 1\r\n\r\nx");
            assert!(ask(&req).starts_with("HTTP/1.1 403"), "{method} {path}");
        }
        assert!(!fixes.exists(), "a write of another page does not make the file");
    }

    #[test]
    fn a_write_to_another_path_is_refused() {
        let (fixes, ask, port, _) = server("path");
        let put = |path: &str| ask(&format!("PUT {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 1\r\n\r\nx"));
        for path in ["/", "/index.html", "/fixes_editor/fixes.js", "/ask/questions.js", "/ask/schema.js", "/ask/views.js", "/api/info"] {
            assert!(put(path).starts_with("HTTP/1.1 405"), "{path}");
        }
        for path in ["/api/fixes/../area.duckdb", "/api/other.yaml", "/area.duckdb", "/../osm_fixes.yaml"] {
            assert!(put(path).starts_with("HTTP/1.1 404"), "{path}");
        }
        assert!(ask(&format!("GET /fixes_editor/../Cargo.toml HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")).starts_with("HTTP/1.1 404"));
        assert!(!fixes.exists() && !fixes.with_file_name("area.duckdb").exists());
    }

    #[test]
    fn a_body_or_headers_too_large_are_refused() {
        let (fixes, ask, port, _) = server("size");
        let big = format!("PUT /api/fixes HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1);
        assert!(ask(&big).starts_with("HTTP/1.1 413"));
        let long = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Long: {}\r\n\r\n", "a".repeat(MAX_HEAD + 10));
        assert!(ask(&long).starts_with("HTTP/1.1 431"));
        let chunked = format!("PUT /api/fixes HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n0\r\n\r\n");
        assert!(ask(&chunked).starts_with("HTTP/1.1 411"));
        assert!(ask("NOT HTTP\r\n\r\n").starts_with("HTTP/1.1 400"));
        assert!(!fixes.exists());
    }

    #[test]
    fn the_files_of_ask_are_read_only_js_modules() {
        let (_, ask, port, _) = server("questions");
        let ok = ask(&format!("GET /ask/questions.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
        assert!(ok.starts_with("HTTP/1.1 200") && ok.contains("text/javascript") && ok.contains("export const QUESTIONS"));
        let schema = ask(&format!("GET /ask/schema.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
        assert!(schema.starts_with("HTTP/1.1 200") && schema.contains("text/javascript") && schema.contains("export function aboutText"));
        assert!(ask(&format!("GET /ask/other.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")).starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn roadstyles_modules_are_read_only_js_of_the_dashboard() {
        let (_, ask, port, _) = server("modules");
        let get = |path: &str| ask(&format!("GET /{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
        let ok = get("roadstyle/data/data_dock.js");
        assert!(ok.starts_with("HTTP/1.1 200") && ok.contains("text/javascript") && ok.contains("dataDock"));
        assert!(get("roadstyle/ask/ask_panel.js").contains("askPanel"));
        for bad in ["roadstyle/dashboard.css", "roadstyle/../Cargo.toml", "roadstyle/no/such.js", "roadstyle/"] {
            assert!(get(bad).starts_with("HTTP/1.1 404"), "{bad}");
        }
        let put = ask(&format!("PUT /roadstyle/data/data_dock.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n"));
        assert!(put.starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn sql_gives_the_columns_and_a_big_integer_as_a_text() {
        let (_, ask, port, _) = server("sql");
        let body = "SELECT 1 AS x";
        let r = ask(&format!("POST /api/sql HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://localhost:{port}\r\nContent-Length: {}\r\n\r\n{body};", body.len() + 1));
        assert!(r.ends_with(r#"{"columns":[{"name":"x","type":"BIGINT"}],"rows":[["1"]]}"#), "{r}");
        let other = ask(&format!("POST /api/sql HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://evil.example\r\nContent-Length: 1\r\n\r\nx"));
        assert!(other.starts_with("HTTP/1.1 403"));
    }

    #[test]
    fn a_table_is_made_in_ask_mode_only_from_data() {
        let (_, ask, port, _, ran) = server_with("table", Mode::Ask);
        let post = |path: &str, body: &str| ask(&format!("POST /api/{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://localhost:{port}\r\nContent-Length: {}\r\n\r\n{body}", body.len()));
        let ok = r#"{"name":"edits","columns":[{"name":"what","type":"VARCHAR"},{"name":"osm_id","type":"BIGINT"},{"name":"changed","type":"TIMESTAMPTZ"}],"rows":[["a'b",5,null],["c","7","2026-11-06T14:32:05+01:00"]]}"#;
        assert!(post("table", ok).starts_with("HTTP/1.1 200"));
        let ran = ran.lock().unwrap().clone();
        assert_eq!(ran[0], "CREATE OR REPLACE TABLE extra.edits (what VARCHAR, osm_id BIGINT, changed TIMESTAMPTZ)");
        assert_eq!(ran[1], "INSERT INTO extra.edits VALUES ('a''b', 5, NULL), ('c', 7, TRY_CAST('2026-11-06T14:32:05+01:00' AS TIMESTAMPTZ))");
        for bad in [r#"{"name":"edits; DROP","columns":[{"name":"a","type":"VARCHAR"}],"rows":[]}"#,
                    r#"{"name":"t","columns":[{"name":"a b","type":"VARCHAR"}],"rows":[]}"#,
                    r#"{"name":"t","columns":[{"name":"a","type":"VARCHAR); DROP TABLE x; --"}],"rows":[]}"#,
                    r#"{"name":"t","columns":[{"name":"a","type":"BIGINT"}],"rows":[["1; DROP"]]}"#,
                    r#"{"name":"t","columns":[{"name":"a","type":"VARCHAR"}],"rows":[["x","y"]]}"#, "not json"] {
            assert!(post("table", bad).starts_with("HTTP/1.1 400"), "{bad}");
        }
        assert_eq!(ran.len(), 2, "a refused table runs no statement");
        let (_, ask, port, _) = server("no-table");
        let edit = ask(&format!("POST /api/table HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://localhost:{port}\r\nContent-Length: {}\r\n\r\n{ok}", ok.len()));
        assert!(edit.starts_with("HTTP/1.1 404"), "the editor page has no table route");
    }

    #[test]
    fn ask_mode_does_not_build() {
        let (fixes, ask, port, built, _) = server_with("no-build", Mode::Ask);
        std::fs::write(&fixes, "overrides: []\n").unwrap();
        let r = ask(&format!("POST /api/build HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://localhost:{port}\r\nContent-Length: 0\r\n\r\n"));
        assert!(r.contains(r#""ok":false"#) && r.contains("does not build"), "{r}");
        assert!(built.lock().unwrap().is_empty());
    }

    #[test]
    fn a_query_and_a_build() {
        let (fixes, ask, port, built) = server("build");
        let post = |path: &str, body: &str| ask(&format!("POST /api/{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://localhost:{port}\r\nContent-Length: {}\r\n\r\n{body}", body.len()));
        assert!(post("query", "SELECT 1 AS x").ends_with(r#"[{"x":1}]"#));
        assert!(post("build", "").contains(r#""ok":false"#), "no fixes file: no build");
        std::fs::write(&fixes, "overrides: []\n").unwrap();
        assert!(post("build", "").contains(r#""ok":true"#));
        assert_eq!(*built.lock().unwrap(), vec!["name: a with overrides: []\n".to_string()]);
    }
}
