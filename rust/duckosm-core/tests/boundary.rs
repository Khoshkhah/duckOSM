//! The Rust cut equals osmium's `complete_ways` cut: tests/data/monaco.osm.pbf is duckOSM's osmium cut of
//! monaco_sample.osm.pbf with monaco.geojson. The Rust cut of the sample gives the same ways (ids, nodes, tags)
//! and the same coordinates for their nodes, so the same network.

use duckosm_core::boundary::Boundary;
use duckosm_core::graph::Relation;
use duckosm_core::pbf;
use std::path::PathBuf;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data")
        .join(name)
}

#[test]
fn rust_cut_equals_osmium_cut() {
    let b = Boundary::parse(&std::fs::read_to_string(data("monaco.geojson")).unwrap()).unwrap();
    assert_eq!(b.name.as_deref(), Some("Monaco"));
    let (ways, coords, rels) = pbf::read_area_path(&data("monaco_sample.osm.pbf"), &b).unwrap();
    let cut = data("monaco.osm.pbf");
    let want = pbf::read_ways(&cut).unwrap();
    let want_coords = pbf::read_coords(&cut, &want).unwrap();
    let all = pbf::read_ways(&data("monaco_sample.osm.pbf")).unwrap();
    assert!(
        ways.len() < all.len(),
        "the cut keeps fewer ways than the sample"
    );
    assert_eq!(ways.len(), want.len(), "ways");
    for (a, w) in ways.iter().zip(&want) {
        assert_eq!((a.id, &a.refs), (w.id, &w.refs));
        let (mut ta, mut tw): (Vec<_>, Vec<_>) =
            (a.tags.osm_pairs().collect(), w.tags.osm_pairs().collect());
        ta.sort();
        tw.sort();
        assert_eq!(ta, tw);
    }
    // all restrictions of the file, so also the ones of the cut (the graph step keeps the ones that it can use)
    let key = |r: &[Relation]| {
        r.iter()
            .map(|r| (r.id, r.tags.clone(), r.members.clone()))
            .collect::<Vec<_>>()
    };
    let (rels, file) = (
        key(&rels),
        key(&pbf::read_restrictions(&data("monaco_sample.osm.pbf")).unwrap()),
    );
    assert_eq!(rels, file);
    for r in key(&pbf::read_restrictions(&cut).unwrap()) {
        assert!(rels.contains(&r), "restriction {}", r.0);
    }
    for n in want.iter().flat_map(|w| &w.refs) {
        assert_eq!(coords.get(n), want_coords.get(n), "node {n}");
    }
}
