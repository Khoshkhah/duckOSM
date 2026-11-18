//! `graphs::to_petgraph` (cargo feature `petgraph`) on Monaco: the junction graph has the junctions and the roads of the tables
//! (networkx's `to_networkx_nodes`), the turn graph the nodes and the arcs of the router (networkx's `to_networkx`), and petgraph's
//! Dijkstra finds the cost of the router's route.
#![cfg(feature = "petgraph")]

use std::path::PathBuf;

#[test]
fn petgraph_graphs_of_monaco() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("duckosm-route-petgraph-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let s = root.join("tests/data");
    let cfg = format!(
        "name: monaco\noutput_path: {}\nsource: {{type: pbf, pbf_path: {}/monaco.osm.pbf}}\nboundary: {{path: {}/monaco.geojson}}\nmodes: [driving]\noptions:\n  sea: false\n  levels: false\n  build_features: false\n",
        out.display(),
        s.display(),
        s.display()
    );
    std::fs::write(out.join("config.yaml"), cfg).unwrap();
    let c = duckosm_build::config::read(&out.join("config.yaml"), false, true).unwrap();
    let mut p = c.params;
    p.geoparquet = false;
    duckosm_build::build(&c.pbf, &c.out, &p).unwrap();
    let db = out.join("monaco.duckdb");

    let g = duckosm_route::graphs::to_petgraph(&db, "driving", "time").unwrap();
    let con = duckosm_route::route::open(&db).unwrap();
    let one = |q: &str| con.query_row(q, [], |r| r.get::<_, i64>(0)).unwrap() as usize;
    let junctions = one("SELECT count(*) FROM (SELECT node_id FROM driving.nodes WHERE geom IS NOT NULL UNION SELECT source FROM driving.edges UNION SELECT target FROM driving.edges)");
    assert_eq!(g.junctions.node_count(), junctions);
    assert_eq!(g.junctions.edge_count(), one("SELECT count(*) FROM driving.edges"));
    let router = duckosm_route::route::routing_graph(&con, "driving", "time", &[]).unwrap();
    assert_eq!(g.turns.node_count(), router.n_nodes());
    assert_eq!(g.turns.edge_count(), router.n_arcs());
    assert!(g.turns.node_count() > 2000);

    // a route: the router's cost equals petgraph's Dijkstra
    let ids: Vec<i64> = g.turns.node_weights().copied().collect();
    let a = ids[0];
    let (b, cost) = ids[ids.len() / 2..].iter().find_map(|&b| router.dijkstra(a, b).unwrap().map(|(c, _)| (b, c))).expect("a route");
    let dist = petgraph::algo::dijkstra(&g.turns, g.turn_index[&a], Some(g.turn_index[&b]), |e| *e.weight());
    assert!((dist[&g.turn_index[&b]] - cost).abs() < 1e-6, "{} {cost}", dist[&g.turn_index[&b]]);
    let _ = std::fs::remove_dir_all(&out);
}
