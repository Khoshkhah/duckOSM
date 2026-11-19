//! `duckosm quack --osm2pgsql` on `tests/data/monaco.osm.pbf` equals `osm2pgsql --hstore` of osm2pgsql 2.3.1 with its
//! default style (the golden file `tests/data/monaco_osm2pgsql.jsonl.gz`, made from osm2pgsql's tables in PostGIS with
//! `tests/osm2pgsql/golden.sql`): the same rows in each table, with the same `osm_id`, columns and tags, and the same
//! geometry, byte for byte (EPSG:3857). The whole file, the file by tiles of zoom 12, and each tile alone with the join
//! (the browser). With the OSM fixes, exactly the rows of the fixed objects change.
#![cfg(feature = "duckdb")]

use serde_json::Value;
use std::collections::BTreeMap;
use std::io::BufRead;

/// (table, osm_id, WKB as hex) -> (columns that are not NULL, tags)
type Rows = BTreeMap<(String, i64, String), (Value, Value)>;

fn data(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data").join(name)
}

/// `way_area` is a real (32 bits): PostGIS writes 3459.61, DuckDB 3459.610107421875 for the same number.
fn real(mut cols: Value) -> Value {
    if let Some(v) = cols.get_mut("way_area") {
        *v = serde_json::json!(v.as_f64().unwrap() as f32);
    }
    cols
}

fn golden() -> Rows {
    let r = std::io::BufReader::new(flate2::read::GzDecoder::new(std::fs::File::open(data("monaco_osm2pgsql.jsonl.gz")).unwrap()));
    let mut rows = Rows::new();
    for line in r.lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let key = (v[0].as_str().unwrap().to_string(), v[1].as_i64().unwrap(), v[4].as_str().unwrap().to_string());
        assert!(rows.insert(key, (real(v[2].clone()), v[3].clone())).is_none(), "two equal rows");
    }
    rows
}

/// The rows of the four GeoParquet files `<base>_<table>.parquet`, read with DuckDB.
fn ours(base: &std::path::Path) -> Rows {
    let con = duckdb::Connection::open_in_memory().unwrap();
    let mut rows = Rows::new();
    for t in ["planet_osm_point", "planet_osm_line", "planet_osm_polygon", "planet_osm_roads"] {
        let file = format!("{}_{t}.parquet", base.display());
        let sql = format!(
            "SELECT osm_id, to_json(COLUMNS(* EXCLUDE (osm_id, tags, way)))::VARCHAR, to_json(tags)::VARCHAR, lower(hex(ST_AsWKB(way))) FROM read_parquet('{file}')"
        );
        // one JSON object of the columns: DuckDB gives each column; join them by name
        let names: Vec<String> = {
            let mut st = con.prepare(&format!("SELECT * EXCLUDE (osm_id, tags, way) FROM read_parquet('{file}') LIMIT 0")).unwrap();
            st.execute([]).unwrap();
            st.column_names()
        };
        let mut st = con.prepare(&sql).unwrap();
        let mut q = st.query([]).unwrap();
        while let Some(r) = q.next().unwrap() {
            let id: i64 = r.get(0).unwrap();
            let mut cols = serde_json::Map::new();
            for (i, n) in names.iter().enumerate() {
                if let Some(v) = r.get::<_, Option<String>>(1 + i).unwrap() {
                    cols.insert(n.clone(), serde_json::from_str(&v).unwrap());
                }
            }
            let tags: Value = serde_json::from_str(&r.get::<_, String>(1 + names.len()).unwrap()).unwrap();
            let wkb: String = r.get(2 + names.len()).unwrap();
            assert!(rows.insert((t.to_string(), id, wkb), (real(Value::Object(cols)), tags)).is_none(), "two equal rows");
        }
    }
    rows
}

enum Mode {
    Whole,
    Tiles,
    TilesAlone,
}

/// The browser's way: the tile cut, each tile of zoom 12 alone (`quack_osm2pgsql_tile_to`), then the join.
fn tiles_alone(pbf: &std::path::Path, base: &std::path::Path) {
    use duckosm_core::quack;
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(pbf).map_err(|e| e.to_string())?));
    let tiles = quack::tiles_of_file(&open, quack::TileSize::Zoom(12)).unwrap();
    let world = duckosm_core::boundary::Boundary::parse(quack::WORLD).unwrap();
    let files = duckosm_core::tilecut::cut_to_store(&open, &world, &tiles, &["multipolygon", "boundary"]).unwrap();
    let mut parts = Vec::new();
    for (i, (tile, c)) in files.tiles.iter().enumerate() {
        if c.nodes == 0 {
            continue;
        }
        let mut out = Vec::new();
        let open_tile = || Ok(std::io::BufReader::new(files.reader(i)));
        duckosm_build::quack_osm2pgsql::quack_osm2pgsql_tile_to(open_tile, *tile, &mut out, None).unwrap();
        parts.push(std::io::Cursor::new(out));
    }
    assert!(parts.len() > 1, "more than one tile");
    let outs: Vec<_> = (1..=4u8)
        .map(|t| std::fs::File::create(format!("{}_{}.parquet", base.display(), duckosm_build::quack_osm2pgsql::table_name(t))).unwrap())
        .collect();
    duckosm_build::quack_osm2pgsql::quack_osm2pgsql_join_to(parts, true, outs).unwrap();
}

fn run(mode: Mode, fixes: Option<&str>) -> Rows {
    let pbf = data("monaco.osm.pbf");
    let dir = std::env::temp_dir().join(format!("quack_osm2pgsql_{}_{}", std::process::id(), rand_name()));
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("monaco_osm2pgsql");
    let fixes = fixes.map(|f| duckosm_build::config::read_osm_fixes(&data(f)).unwrap());
    let tiles = match mode {
        Mode::Whole => None,
        Mode::Tiles => Some(duckosm_build::TileSize::Zoom(12)),
        Mode::TilesAlone => {
            tiles_alone(&pbf, &base);
            None
        }
    };
    if !matches!(mode, Mode::TilesAlone) {
        duckosm_build::quack_osm2pgsql::quack_osm2pgsql(&pbf, Some(&base), None, None, true, tiles, fixes.as_ref()).unwrap();
    }
    let rows = ours(&base);
    std::fs::remove_dir_all(&dir).ok();
    rows
}

fn rand_name() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// The (table, osm_id) of the rows that are not in both.
fn diff(want: &Rows, got: &Rows) -> Vec<(String, i64)> {
    let mut d: Vec<(String, i64)> = want
        .iter()
        .filter(|(k, v)| got.get(*k) != Some(v))
        .chain(got.iter().filter(|(k, v)| want.get(*k) != Some(v)))
        .map(|(k, _)| (k.0.clone(), k.1))
        .collect();
    d.sort();
    d.dedup();
    d
}

fn check(mode: Mode) {
    let (want, got) = (golden(), run(mode, None));
    assert_eq!(want.len(), 3959 + 3919 + 1837 + 422);
    let d = diff(&want, &got);
    if let Some((t, id)) = d.first() {
        let show = |r: &Rows| r.iter().filter(|(k, _)| &k.0 == t && k.1 == *id).map(|(k, v)| format!("{} {} {}", k.2, v.0, v.1)).collect::<Vec<_>>();
        eprintln!("{t} {id}\nosm2pgsql: {:?}\nduckOSM:   {:?}", show(&want), show(&got));
    }
    assert!(d.is_empty(), "rows that differ from osm2pgsql: {d:?}");
}

#[test]
fn monaco_equals_osm2pgsql() {
    check(Mode::Whole);
}

#[test]
fn monaco_by_tiles_equals_osm2pgsql() {
    check(Mode::Tiles);
}

#[test]
fn monaco_tiles_alone_equal_osm2pgsql() {
    check(Mode::TilesAlone);
}

/// The OSM fixes of the quack tests change exactly the rows of the objects that they fix.
#[test]
fn monaco_fixes_change_their_rows() {
    let d = diff(&golden(), &run(Mode::Whole, Some("osm_fixes_quack.yaml")));
    let ids: std::collections::BTreeSet<i64> = d.iter().map(|r| r.1).collect();
    let want = [-393226, 4227277, 21911886, 23715051, 25722685, 503462470, 9000000000001, 9000000000002];
    assert_eq!(ids, want.into_iter().collect(), "{d:?}");
}
