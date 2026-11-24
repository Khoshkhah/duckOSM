//! `duckosm hdmap` (docs/design/hdmap_simready.md) with a fake SimReady: a shell script that checks the input folder,
//! keeps its arguments and writes the output file.
#![cfg(unix)]

use duckdb::Connection;
use duckosm_export::hdmap_run::{run_hdmap, write_input, HdmapArea};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hdmap_run").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A routing database with edges 1 and 2, the raw tables, a DEM grid and a fixes file; and its GMNS database. `extra_link`:
/// a link that is not an edge of the routing database; `chains`: the link lists of gmns_driving.chains.
fn dbs(d: &Path, extra_link: bool, chains: &str) -> (PathBuf, PathBuf) {
    let (db, gmns) = (d.join("area.duckdb"), d.join("area_gmns.duckdb"));
    let c = Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE SCHEMA driving; CREATE SCHEMA raw;
         CREATE TABLE driving.edges(edge_id BIGINT); INSERT INTO driving.edges VALUES (1), (2);
         CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR, VARCHAR)); INSERT INTO raw.nodes VALUES (10, 43.7, 7.41, MAP{'barrier': 'kerb'});
         CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[]);
         CREATE TABLE raw.relations(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR));
         CREATE TABLE main.elevation_grid(x DOUBLE, y DOUBLE, z DOUBLE); INSERT INTO main.elevation_grid VALUES (7.4, 43.7, 12.5);
         CREATE TABLE main.fixes(yaml VARCHAR, path VARCHAR, sha256 VARCHAR); INSERT INTO main.fixes VALUES ('ways: []', 'f.yaml', 'x');
         CREATE TABLE main.osm_source(osm_timestamp TIMESTAMP); INSERT INTO main.osm_source VALUES ('2026-10-14 10:05:32');",
    )
    .unwrap();
    let g = Connection::open(&gmns).unwrap();
    g.execute_batch(&format!(
        "CREATE SCHEMA gmns_driving;
         CREATE TABLE gmns_driving.link(link_id BIGINT, refs BIGINT[], z_vertices DOUBLE[]);
         INSERT INTO gmns_driving.link VALUES (1, [10, 11], [1.0, 2.0]), (2, [11, 12], NULL){};
         CREATE TABLE gmns_driving.chains(chain_id BIGINT, link_ids BIGINT[]);
         INSERT INTO gmns_driving.chains SELECT row_number() OVER (), unnest([{chains}]);
         CREATE TABLE gmns_driving.config(duckosm_version VARCHAR); INSERT INTO gmns_driving.config VALUES ('duckosm 0.8.0');
         CREATE TABLE main.elevation_metadata(ele_column VARCHAR, sha256 VARCHAR); INSERT INTO main.elevation_metadata VALUES ('ele', 'ground1');",
        if extra_link { ", (3, NULL, NULL)" } else { "" }
    ))
    .unwrap();
    (db, gmns)
}

/// A fake `duckosm-sim`: it needs `build`, an input folder with input.json, and writes `--out` and its arguments.
fn fake_sim(d: &Path) -> PathBuf {
    let p = d.join("duckosm-sim");
    std::fs::write(
        &p,
        "#!/bin/sh\n[ \"$1\" = build ] || exit 3\necho \"$@\" > \"$(dirname \"$0\")/args.txt\"\n\
         [ -f \"$3/input.json\" ] || exit 4\necho hdmap > \"$5\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
fn writes_the_input_and_runs_simready() {
    let d = dir("ok");
    let (db, gmns) = dbs(&d, false, "[1], [2]");
    let (sim, out) = (fake_sim(&d), d.join("area_hdmap.duckdb"));
    let s = run_hdmap(&db, &gmns, &out, &["lanelet2".into(), "opendrive".into()], Some(&sim), true, &HdmapArea { jurisdiction: Some("MC".into()), driving_side: None }).unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "hdmap\n");
    let names: Vec<&str> = s.files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["elevation_grid.parquet", "fixes.yaml", "gmns.duckdb", "raw_nodes.parquet", "raw_relations.parquet", "raw_ways.parquet"]
    );
    let json = std::fs::read_to_string(s.input.join("input.json")).unwrap();
    for want in ["\"contract\": 1", "\"area\": \"area\"", "\"duckosm_version\": \"duckosm 0.8.0\"", "\"ground_id\": \"ground1\"", "\"osm_timestamp\": \"2026-10-14 10:05:32\"",
                 "\"jurisdiction\": \"MC\"", "\"driving_side\": null", "\"gmns.duckdb\": \""] {
        assert!(json.contains(want), "{want} not in {json}");
    }
    assert_eq!(std::fs::read_to_string(s.input.join("fixes.yaml")).unwrap(), "ways: []");
    let args = std::fs::read_to_string(d.join("args.txt")).unwrap();
    assert!(args.ends_with("--formats lanelet2,opendrive\n"), "{args}");
}

#[test]
fn the_input_folder_goes_away_after_the_run() {
    let d = dir("clean");
    let (db, gmns) = dbs(&d, false, "[1, 2]");
    let s = run_hdmap(&db, &gmns, &d.join("area_hdmap.duckdb"), &[], Some(&fake_sim(&d)), false, &HdmapArea::default()).unwrap();
    assert!(!s.input.exists());
}

#[test]
fn stops_without_simready() {
    let d = dir("nosim");
    let (db, gmns) = dbs(&d, false, "[1], [2]");
    let e = run_hdmap(&db, &gmns, &d.join("o.duckdb"), &[], Some(&d.join("missing")), false, &HdmapArea::default()).err().unwrap();
    assert!(e.starts_with("SimReady is not installed"), "{e}");
    assert!(!d.join("o_input").exists(), "no input is written when SimReady is not there");
}

#[test]
fn stops_on_a_gmns_database_of_another_area() {
    let d = dir("other");
    let (db, gmns) = dbs(&d, true, "[1], [2], [3]");
    let e = write_input(&db, &gmns, &d.join("in"), &HdmapArea::default()).err().unwrap();
    assert!(e.contains("not of this routing database"), "{e}");
}

#[test]
fn stops_without_heights() {
    let d = dir("noheights");
    let (db, gmns) = dbs(&d, false, "[1], [2]");
    Connection::open(&gmns).unwrap().execute_batch("DROP TABLE main.elevation_metadata").unwrap();
    let e = write_input(&db, &gmns, &d.join("in"), &HdmapArea::default()).err().unwrap();
    assert!(e.contains("duckosm elevation"), "{e}");
}

#[test]
fn the_ways_get_their_line() {
    let d = dir("line");
    let (db, gmns) = dbs(&d, false, "[1], [2]");
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "INSERT INTO raw.nodes VALUES (11, 43.7, 7.411, MAP{}), (12, 43.7, 7.412, MAP{});
             INSERT INTO raw.ways VALUES (100, MAP{}, [10, 11, 99, 12]), (101, MAP{}, [10])",
        )
        .unwrap();
    write_input(&db, &gmns, &d.join("in"), &HdmapArea::default()).unwrap();
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("INSTALL spatial; LOAD spatial;").unwrap();
    let p = d.join("in").join("raw_ways.parquet").display().to_string();
    let wkt: Vec<Option<String>> = c
        .prepare(&format!("SELECT ST_AsText(ST_GeomFromWKB(line)) FROM '{p}' ORDER BY osm_id"))
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    // node 99 has no coordinates: left out; one node: no line
    assert_eq!(wkt, [Some("LINESTRING (7.41 43.7, 7.411 43.7, 7.412 43.7)".to_string()), None]);
}

#[test]
fn stops_when_a_link_is_in_no_chain_or_in_two() {
    let d = dir("chains");
    let (db, gmns) = dbs(&d, false, "[1]");
    assert!(write_input(&db, &gmns, &d.join("in"), &HdmapArea::default()).err().unwrap().contains("in no chain or in more than one"));
    let d = dir("chains2");
    let (db, gmns) = dbs(&d, false, "[1, 2], [2]");
    assert!(write_input(&db, &gmns, &d.join("in"), &HdmapArea::default()).err().unwrap().contains("in no chain or in more than one"));
}
