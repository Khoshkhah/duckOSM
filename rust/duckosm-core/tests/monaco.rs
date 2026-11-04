//! Golden test: the driving network of Monaco must equal the output of duckOSM 0.6.0 (DuckDB 1.5.3),
//! built with tests/data/golden_monaco.yaml (no OSM fixes). Every edge row is compared.

use duckosm_core::network::{build_driving, Options};
use duckosm_core::pbf;
use std::collections::HashMap;
use std::path::PathBuf;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data")
        .join(name)
}

#[test]
fn monaco_driving_equals_duckosm() {
    let ways = pbf::read_ways(&data("monaco.osm.pbf")).unwrap();
    let coords = pbf::read_coords(&data("monaco.osm.pbf"), &ways).unwrap();
    let net = build_driving(&ways, &coords, &Options::default()).unwrap();

    let golden = std::fs::read_to_string(data("monaco_driving_edges.tsv")).unwrap();
    let mut want: HashMap<i64, Vec<&str>> = HashMap::new();
    for line in golden.lines().skip(1) {
        let f: Vec<&str> = line.split('\t').collect();
        want.insert(f[0].parse().unwrap(), f);
    }
    assert_eq!(net.edges.len(), want.len(), "number of edges");
    let mut bad = Vec::new();
    for e in &net.edges {
        let Some(f) = want.get(&e.edge_id) else {
            bad.push(format!("edge_id {} is not in the golden file", e.edge_id));
            continue;
        };
        let got = [
            e.source.to_string(),
            e.target.to_string(),
            e.osm_id.to_string(),
            (e.is_reverse as i32).to_string(),
            e.lanes.map_or(String::new(), |l| l.to_string()),
            format!("{:08x}", e.length_m.to_bits()),
            e.edge_ref().unwrap_or_else(|| "None".into()),
            e.attrs.highway.clone(),
        ];
        if got.iter().map(String::as_str).ne(f[1..].iter().copied()) {
            bad.push(format!(
                "edge_id {}: got {:?}, want {:?}",
                e.edge_id,
                got,
                &f[1..]
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} different rows, first: {:?}",
        bad.len(),
        &bad[..bad.len().min(5)]
    );
    assert_eq!(net.nodes.len(), 1719);
    assert_eq!(net.nodes.iter().filter(|n| n.node_id < 0).count(), 1);
}
