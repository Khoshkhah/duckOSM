//! Each Rust export of Monaco equals the output of the Python export (src/duckosm/), made before the port.
//!
//! The test builds Monaco (three modes, its boundary, the base-map layers so that raw.* stays in the file) into
//! `<target>/tmp/duckosm-exports/golden.duckdb`. The golden files in tests/data/exports/ are the output of the Python
//! exports on that database: tools/make_golden_exports.py. Run this test once, then the tool, to make them again.
//! The test needs DuckDB's spatial extension (the first `INSTALL` downloads it).

use duckosm_build::{build, LevelsMethod, NetworkMode, Params};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
}

/// The Monaco database, built once for all tests of this file.
fn db() -> &'static Path {
    static DB: OnceLock<PathBuf> = OnceLock::new();
    DB.get_or_init(|| {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-exports");
        let p = Params {
            modes: vec![NetworkMode::Driving, NetworkMode::Walking, NetworkMode::Cycling],
            geoparquet: false,
            duckdb: true,
            name: "golden".into(),
            boundary: Some(data().join("monaco.geojson")),
            levels: LevelsMethod::None,
            bus_routes: false,
            features: true,
            built_at: Some(0),
            ..Params::default()
        };
        build(&data().join("monaco_sample.osm.pbf"), &out, &p).unwrap();
        out.join("golden.duckdb")
    })
}

fn out(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckosm-exports-out");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// The text of a file; gunzipped when its name ends with .gz (the gzip header has a time and a file name).
fn text(p: &Path) -> String {
    let b = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    if p.extension().is_some_and(|e| e == "gz") {
        let mut s = String::new();
        flate2::read::GzDecoder::new(&b[..]).read_to_string(&mut s).unwrap();
        s
    } else {
        String::from_utf8(b).unwrap()
    }
}

/// The two files are equal: else the first line that differs.
fn same(got: &Path, golden: &str) {
    let (a, b) = (text(got), text(&data().join("exports").join(golden)));
    if a != b {
        let (n, (x, y)) = a.lines().zip(b.lines()).enumerate().find(|(_, (x, y))| x != y).unwrap_or((0, ("", "")));
        panic!("{golden} differs from the Python export (lines {} and {}), line {}:\n rust:   {x}\n python: {y}", a.lines().count(), b.lines().count(), n + 1);
    }
}

#[test]
fn matsim_equals_python() {
    let o = out("monaco_network.xml.gz");
    duckosm_export::matsim::to_matsim(db(), &o, "driving", None, true).unwrap();
    same(&o, "monaco_network.xml.gz");
    let o = out("monaco_all_network.xml");
    duckosm_export::matsim::to_matsim(db(), &o, "all", Some("EPSG:3035"), false).unwrap();
    same(&o, "monaco_all_network.xml.gz");
}

#[test]
fn railml_equals_python() {
    let o = out("monaco.railml.xml");
    duckosm_export::railml::to_railml(db(), &o).unwrap();
    same(&o, "monaco.railml.xml");
}
