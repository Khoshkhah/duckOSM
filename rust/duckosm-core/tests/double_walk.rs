//! A way that walks one segment twice in the same direction (docs/design.md, section 5, "A way that walks
//! a segment twice"). tests/data/ky_double_walk.osm.pbf is way 1214406652 of Kentucky, a golf cart path. It
//! passes the nodes 36, 37, 38 and 39 two times, and the nodes 74, 75 and 76 there and back. duckOSM
//! stops with "duplicate (osm_id, source, target)" on it, so there is no golden: the test checks the
//! result against its own rules.

use duckosm_core::filter::Mode;
use duckosm_core::network::{build_network, Options};
use duckosm_core::pbf;
use std::collections::HashSet;

#[test]
fn double_walk_gives_unique_keys_and_keeps_every_arc() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data/ky_double_walk.osm.pbf");
    let ways = pbf::read_ways(&path).unwrap();
    let coords = pbf::read_coords(&path, &ways).unwrap();
    let opts = Options {
        keep_largest_component: false,
        connect_paths: false,
        build_graph: false,
        extract_restrictions: false,
        ..Options::default()
    };
    let net = build_network(&ways, &coords, Mode::Walking, &opts).unwrap();
    let keys: HashSet<_> = net
        .edges
        .iter()
        .map(|e| (e.osm_id, e.source, e.target))
        .collect();
    assert_eq!(keys.len(), net.edges.len(), "(osm_id, source, target) is unique");
    let ids: HashSet<_> = net.edges.iter().map(|e| e.edge_id).collect();
    assert_eq!(ids.len(), net.edges.len(), "edge_id is unique");
    // no arc is lost: the forward edges are as long as the way
    let m: f64 = net.edges.iter().filter(|e| !e.is_reverse).map(|e| e.length_m as f64).sum();
    let way: f64 = ways[0]
        .refs
        .windows(2)
        .map(|p| duckosm_core::geom::haversine(*coords.get(&p[0]).unwrap(), *coords.get(&p[1]).unwrap()))
        .sum();
    assert!((m - way).abs() < 1.0, "forward length {m} m, way {way} m");
}
