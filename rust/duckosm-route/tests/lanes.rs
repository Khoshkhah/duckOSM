//! `lane-graph` and `route-lanes` equal duckOSM's Python output (2026-11-17). The input is the lanes and the movements of the
//! GMNS file that `duckosm gmns` makes of Monaco (driving; the columns that the lane graph reads), in
//! tests/data/golden_route/monaco_gmns_{lane,movement}.parquet. The expected lane graph: its count and the MD5 of its rows; the
//! routes: tests/data/golden_route/monaco_route_lanes.json (`route_lanes` of 12 random lane pairs).

use std::path::PathBuf;

#[test]
fn lane_graph_and_routes_equal_duckosm() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let data = root.join("tests/data/golden_route");
    let out = std::env::temp_dir().join(format!("duckosm-route-lanes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let db = out.join("monaco_gmns.duckdb");
    {
        let con = duckdb::Connection::open(&db).unwrap();
        con.execute_batch(&format!(
            "INSTALL spatial; LOAD spatial; CREATE SCHEMA gmns_driving;
             CREATE TABLE gmns_driving.lane AS SELECT lane_id, link_id, lane_num, ST_GeomFromWKB(geom) AS geom FROM read_parquet('{}');
             CREATE TABLE gmns_driving.movement AS SELECT * FROM read_parquet('{}');",
            data.join("monaco_gmns_lane.parquet").display(),
            data.join("monaco_gmns_movement.parquet").display()
        ))
        .unwrap();
    }
    let (lanes, lc, turn) = duckosm_route::lanes::build_lane_graph(&db, "driving").unwrap();
    assert_eq!((lanes, lc + turn), (3321, 4935));
    let con = duckdb::Connection::open(&db).unwrap();
    let md5: String = con
        .query_row(
            "SELECT md5(string_agg(concat_ws('|', from_lane, to_lane, kind, cost), ',' ORDER BY from_lane, to_lane, kind, cost)) FROM lane_driving.lane_edges",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(md5, "174b93b68e73e4d5ebff9b36ca5f3846");
    drop(con);

    let golden: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(data.join("monaco_route_lanes.json")).unwrap()).unwrap();
    for r in golden.as_array().unwrap() {
        let (a, b) = (r["from"].as_str().unwrap(), r["to"].as_str().unwrap());
        let mut got = serde_json::json!({"from": a, "to": b});
        for (k, v) in duckosm_route::lanes::route_lanes(&db, a, b, "driving").unwrap().as_object().unwrap() {
            got[k] = v.clone();
        }
        assert_eq!(&got, r, "{a} -> {b}");
    }
    let _ = std::fs::remove_dir_all(&out);
}
