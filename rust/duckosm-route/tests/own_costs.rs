//! Your own cost of each edge (`route::Own`, `routing_graph_own`, `column_costs`) on a small graph: edge 1 goes to 4 through
//! 2 (fast) or through 3 (slow).

use duckosm_route::route::{column_costs, routing_graph, routing_graph_own, Own};
use std::collections::HashMap;

fn db() -> duckdb::Connection {
    let con = duckdb::Connection::open_in_memory().unwrap();
    con.execute_batch(
        "CREATE SCHEMA driving;
         CREATE TABLE driving.edges (edge_id BIGINT, cost_s DOUBLE, length_m DOUBLE, toll DOUBLE, name VARCHAR);
         INSERT INTO driving.edges VALUES (1, 10, 100, NULL, 'a'), (2, 10, 100, 50, 'b'), (3, 20, 200, NULL, 'c'), (4, 10, 100, NULL, 'd');
         CREATE TABLE driving.edge_graph (from_edge BIGINT, to_edge BIGINT, cost DOUBLE);
         INSERT INTO driving.edge_graph VALUES (1, 2, 10), (1, 3, 10), (2, 4, 10), (3, 4, 20);",
    )
    .unwrap();
    con
}

fn path(g: &duckosm_route::graph::Graph) -> Option<Vec<i64>> {
    g.bidirectional_dijkstra(1, 4).unwrap().map(|(_, p)| p)
}

#[test]
fn own_costs_change_the_route() {
    let con = db();
    assert_eq!(path(&routing_graph(&con, "driving", "time", &[]).unwrap()), Some(vec![1, 2, 4]));
    // a toll on edge 2: the route goes through 3
    let toll = column_costs(&con, "driving", "toll").unwrap();
    assert_eq!(toll, HashMap::from([(2, 50.0)]));
    let g = routing_graph_own(&con, "driving", &[], &Own { costs: toll.clone(), skip: false }).unwrap();
    assert_eq!(path(&g), Some(vec![1, 3, 4]));
    assert_eq!(g.bidirectional_dijkstra(1, 4).unwrap().unwrap().0, 10.0 + 20.0);
    // skip: only the edges with a cost; 2 alone cannot reach 4
    let g = routing_graph_own(&con, "driving", &[], &Own { costs: toll, skip: true }).unwrap();
    assert!(!g.contains(1));
    let costs = HashMap::from([(1, 1.0), (3, 1.0), (4, 1.0)]);
    assert_eq!(path(&routing_graph_own(&con, "driving", &[], &Own { costs, skip: true }).unwrap()), Some(vec![1, 3, 4]));
}

#[test]
fn bad_own_costs() {
    let con = db();
    assert!(column_costs(&con, "driving", "nope").unwrap_err().contains("not 'time', 'length' or a column"));
    assert!(column_costs(&con, "driving", "name").unwrap_err().contains("not numeric"));
    let bad = Own { costs: HashMap::from([(2, -1.0)]), skip: false };
    assert!(routing_graph_own(&con, "driving", &[], &bad).is_err());
}
