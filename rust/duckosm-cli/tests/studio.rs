//! The local dashboard (`duckosm studio`, duckosm-edit/src/studio.rs) builds an area by tiles with this program
//! (`duckosm tiles --config`): Monaco from a request of the page, with the DEM of the engine's tests. The database has the
//! heights, the grid and the metadata, and the record of the build.
#![cfg(feature = "duckdb")]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;

/// One HTTP request to the page; the whole answer.
fn ask(port: u16, req: &str, body: &[u8]) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(req.as_bytes()).and_then(|_| s.write_all(body)).unwrap();
    let mut text = String::new();
    s.read_to_string(&mut text).ok();
    text
}

#[test]
fn the_dashboard_builds_monaco_by_tiles_with_a_dem() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (data, tif) = (root.join("tests/data"), root.join("rust/duckosm-build/tests/data/monaco-copernicus-glo30.tif"));
    let out = std::env::temp_dir().join(format!("duckosm-studio-tiles-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let mut server = std::process::Command::new(env!("CARGO_BIN_EXE_duckosm"))
        .args(["studio", "--port", "0", "--no-open"])
        .arg(&out)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // the first line has the address of the page: "duckosm studio: http://127.0.0.1:<port>/ ..."
    let mut err = BufReader::new(server.stderr.take().unwrap());
    let mut line = String::new();
    err.read_line(&mut line).unwrap();
    // the program writes more lines: read them all, so a write to its stderr does not fail
    std::thread::spawn(move || std::io::copy(&mut err, &mut std::io::sink()));
    let port: u16 = line.split("127.0.0.1:").nth(1).and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next()).and_then(|p| p.parse().ok()).expect(&line);
    let tif_bytes = std::fs::read(&tif).unwrap();
    let put = format!("PUT /api/dem/monaco HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n\r\n", tif_bytes.len());
    assert!(ask(port, &put, &tif_bytes).starts_with("HTTP/1.1 200"));
    let boundary: Value = serde_json::from_str(&std::fs::read_to_string(data.join("monaco.geojson")).unwrap()).unwrap();
    let req = json!({"name": "monaco", "boundary": boundary, "data": {"kind": "file", "path": data.join("monaco_sample.osm.pbf")},
                     "settings": "modes: [driving]\noptions:\n  levels: false\n  sea: false\n  bus_routes: false\n",
                     "dem": {"product": "DTM", "vertical_datum": "EGM2008", "license": "test", "not_reference": false}}).to_string();
    let post = format!("POST /api/build HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Length: {}\r\n\r\n", req.len());
    assert!(ask(port, &post, req.as_bytes()).starts_with("HTTP/1.1 200"));
    let mut job = String::new();
    for _ in 0..1200 {
        job = ask(port, &format!("GET /api/job HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"), b"");
        if job.contains(r#""state":"done""#) || job.contains(r#""state":"error""#) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = server.kill();
    assert!(job.contains(r#""state":"done""#), "{job}");
    assert!(job.contains("tiles: cut to") && job.contains("tiles: the .duckdb file"), "the steps of the build by tiles are in the job: {job}");
    let db = out.join("monaco/monaco.duckdb");
    let con = duckdb::Connection::open_with_flags(&db, duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly).unwrap()).unwrap();
    let n = |sql: &str| con.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    assert!(n("SELECT count(*) FROM driving.edges") > 1000 && n("SELECT count(*) FROM main.elevation_grid WHERE z IS NOT NULL") > 100);
    assert!(n("SELECT count(*) FROM driving.nodes WHERE ele IS NOT NULL") > 100);
    assert_eq!(n("SELECT count(*) FROM main.elevation_metadata WHERE product = 'DTM' AND vertical_datum = 'EGM2008' AND license = 'test'"), 1);
    let record: String = con.query_row("SELECT build_config FROM main.visualization_metadata", [], |r| r.get(0)).unwrap();
    assert!(record.contains("dem: dem.tif") && record.contains("modes: [driving]"), "{record}");
    // only the .duckdb file: the tables of the union and the tiles went
    assert!(!out.join("monaco/union.tmp").exists() && !out.join("monaco/tiles").exists());
    let _ = std::fs::remove_dir_all(&out);
}
