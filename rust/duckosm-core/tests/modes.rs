//! Golden test for the three modes. Input: the sample file of Monaco (no cut, no component filter).
//! The expected counts and checksums come from duckOSM 0.6.0 (DuckDB 1.5.3): tools/make_golden_summary.py.
//! The variants switch `merge_segments` and `global_junctions`; `_cf` also runs the component filter, and `_sw` the sidewalk rule, `_cp` the dangling-path connector, `_sp` the speeds and costs (the `_cf` variants have them too), `_ft` everything: sidewalk rule, connector, speeds and walk and cycle types. With `global_junctions` on, Monaco has no
//! chains to merge, so the variants with it off test the merge.

use duckosm_core::filter::Mode;
use duckosm_core::hash::{hash3, hash_int_int_str};
use duckosm_core::model::Edge;
use duckosm_core::network::{build_network, Options};
use duckosm_core::pbf;
use duckosm_core::sidewalks::infer_separate_sidewalks;
use std::path::PathBuf;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data")
        .join(name)
}

fn checksum(edges: &[Edge]) -> u64 {
    let mut x = 0u64;
    for e in edges {
        let b = (e.source as u64).wrapping_mul(1000003) ^ (e.target as u64);
        let mut c =
            ((e.lanes.unwrap_or(-1) + 1) as u64 & 0xFFFF) << 32 | e.length_m.to_bits() as u64;
        if e.dismount == Some(true) {
            c |= 1 << 48;
        }
        x ^= hash3(e.edge_id, b as i64, c as i64);
        if let Some(kmh) = e.maxspeed_kmh {
            let cost = e.cost_s.map_or(0, |v| v.to_bits() as i64);
            x ^= hash3(e.edge_id ^ 0x5555, kmh.to_bits() as i64, cost);
        }
        if let (Some(t), true) = (e.ftype, e.osm_id > 0) {
            x ^= hash_int_int_str(e.edge_id ^ 0xAAAA, 0, t);
        }
    }
    x
}

#[test]
fn three_modes_equal_duckosm() {
    // the sample file (no cut), and the file cut to the boundary (variants that end in _cf)
    let read = |name: &str| {
        let path = data(name);
        let ways = pbf::read_ways(&path).unwrap();
        let coords = pbf::read_coords(&path, &ways).unwrap();
        (ways, coords)
    };
    let (ways, coords) = read("monaco_sample.osm.pbf");
    let (cut_ways, cut_coords) = read("monaco.osm.pbf");
    let golden = std::fs::read_to_string(data("monaco_modes_summary.tsv")).unwrap();
    let mut bad = Vec::new();
    for line in golden.lines().skip(1) {
        let f: Vec<&str> = line.split('\t').collect();
        let (variant, mode) = (f[0], Mode::parse(f[1]).unwrap());
        let cf = variant.ends_with("_cf") || variant.ends_with("_cfcp");
        // the databases of the _cf, _cfcp and _sp variants have speeds and costs
        let speeds = cf || variant.ends_with("_sp") || variant.ends_with("_ft");
        let opts = Options {
            keep_largest_component: cf,
            cycling_dismount: true,
            merge_segments: variant.starts_with("merge"),
            global_junctions: variant.contains("_gj"),
            connect_paths: variant.ends_with("_cp")
                || variant.ends_with("_cfcp")
                || variant.ends_with("_ft"),
            snap_m: 10.0,
            process_speeds: speeds,
            calculate_costs: speeds,
            functional_types: variant.ends_with("_ft"),
            // the edge graph has its own golden test (graph.rs)
            build_graph: false,
            extract_restrictions: false,
            ways_tables: false,
        };
        let net = if cf {
            build_network(&cut_ways, &cut_coords, mode, &opts)
        } else if variant.ends_with("_sw") || variant.ends_with("_ft") {
            let mut w = ways.clone();
            let n = infer_separate_sidewalks(&mut w, &coords);
            assert_eq!(n, 46, "roads with sidewalk=separate");
            build_network(&w, &coords, mode, &opts)
        } else {
            build_network(&ways, &coords, mode, &opts)
        }
        .unwrap();
        let got = [
            net.edges.len().to_string(),
            net.private_edges.len().to_string(),
            net.nodes.len().to_string(),
            net.edge_id_map.len().to_string(),
            format!("{:x}", checksum(&net.edges)),
            format!("{:x}", checksum(&net.private_edges)),
        ];
        if got.iter().map(String::as_str).ne(f[2..].iter().copied()) {
            bad.push(format!(
                "{variant} {}: got {:?}, want {:?}",
                mode.name(),
                got,
                &f[2..]
            ));
        }
    }
    assert!(bad.is_empty(), "{} variants differ: {:#?}", bad.len(), bad);
}
