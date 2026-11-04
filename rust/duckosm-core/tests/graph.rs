//! Golden test of the edge graph and the turn restrictions: every row of `edge_graph`, `turn_permission`,
//! `turn_restrictions` and `turn_path_restrictions` must equal duckOSM (tests/data/monaco_graph_summary.tsv,
//! made by tools/make_golden_graph.py). Variants: `sample` (no component filter) and `cut` (the cut file,
//! with the component filter, which also prunes the graph tables).

use duckosm_core::filter::Mode;
use duckosm_core::fixes::OsmFixes;
use duckosm_core::graph::Graph;
use duckosm_core::network::{build_network_with, Options};
use duckosm_core::{pbf, sidewalks};
use std::collections::HashMap;
use std::path::PathBuf;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data")
        .join(name)
}

/// The 64-bit FNV-1a hash.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// The text of a NULL, a text and a FLOAT, as in tools/make_golden_graph.py.
fn opt(v: Option<&str>) -> String {
    v.unwrap_or("\\N").to_string()
}

fn float(v: Option<f32>) -> String {
    v.map_or("\\N".to_string(), |f| format!("{:08x}", f.to_bits()))
}

/// (rows, checksum) of each table of a graph.
fn sums(g: &Graph) -> Vec<(&'static str, usize, u64)> {
    fn sum<T>(
        name: &'static str,
        rows: &[T],
        text: impl Fn(&T) -> String,
    ) -> (&'static str, usize, u64) {
        let s = rows.iter().fold(0u64, |a, r| a.wrapping_add(fnv(&text(r))));
        (name, rows.len(), s)
    }
    let mut out = Vec::new();
    if let Some(v) = &g.edge_graph {
        out.push(sum("edge_graph", v, |r| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                r.from_edge,
                r.to_edge,
                r.to_edge,
                float(r.cost),
                r.uses
            )
        }));
    }
    if let Some(v) = &g.turn_permission {
        out.push(sum("turn_permission", v, |r| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.from_edge,
                r.to_edge,
                r.allowed,
                opt(r.vehicles.as_deref()),
                opt(r.except_vehicles.as_deref()),
                opt(r.condition.as_deref()),
                r.restriction_id
            )
        }));
    }
    if let Some(v) = &g.turn_restrictions {
        out.push(sum("turn_restrictions", v, |r| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.restriction_id,
                r.restriction_type,
                opt(r.except_vehicles.as_deref()),
                opt(r.applies_to.as_deref()),
                opt(r.condition.as_deref()),
                r.via_node,
                r.from_edge_id,
                r.to_edge_id
            )
        }));
    }
    if let Some(v) = &g.turn_path_restrictions {
        out.push(sum("turn_path_restrictions", v, |r| {
            let via: Vec<String> = r.via_edges.iter().map(|x| x.to_string()).collect();
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t[{}]\t{}",
                r.restriction_id,
                r.restriction_type,
                opt(r.except_vehicles.as_deref()),
                opt(r.applies_to.as_deref()),
                opt(r.condition.as_deref()),
                r.from_edge,
                via.join(","),
                r.to_edge
            )
        }));
    }
    out
}

#[test]
fn edge_graph_equals_duckosm() {
    let golden = std::fs::read_to_string(data("monaco_graph_summary.tsv")).unwrap();
    let mut want: HashMap<(String, String, String), (usize, u64)> = HashMap::new();
    for line in golden.lines().skip(1) {
        let f: Vec<&str> = line.split('\t').collect();
        let key = (f[0].to_string(), f[1].to_string(), f[2].to_string());
        want.insert(
            key,
            (
                f[3].parse().unwrap(),
                u64::from_str_radix(f[4], 16).unwrap(),
            ),
        );
    }
    let mut bad = Vec::new();
    let mut seen = 0;
    for (variant, file) in [
        ("sample", "monaco_sample.osm.pbf"),
        ("cut", "monaco.osm.pbf"),
    ] {
        let path = data(file);
        let mut ways = pbf::read_ways(&path).unwrap();
        let coords = pbf::read_coords(&path, &ways).unwrap();
        let relations = pbf::read_restrictions(&path).unwrap();
        sidewalks::infer_separate_sidewalks(&mut ways, &coords);
        let opts = Options {
            keep_largest_component: variant == "cut",
            ..Options::default()
        };
        for mode in [Mode::Driving, Mode::Walking, Mode::Cycling] {
            let net = build_network_with(
                &ways,
                &coords,
                mode,
                &opts,
                &OsmFixes::default(),
                &relations,
            )
            .unwrap();
            for (table, rows, sum) in sums(&net.graph) {
                seen += 1;
                let key = (
                    variant.to_string(),
                    mode.name().to_string(),
                    table.to_string(),
                );
                match want.get(&key) {
                    Some(&w) if w == (rows, sum) => {}
                    w => bad.push(format!("{key:?}: got ({rows}, {sum:x}), want {w:x?}")),
                }
            }
        }
    }
    assert!(bad.is_empty(), "{} tables differ: {:#?}", bad.len(), bad);
    assert_eq!(seen, want.len(), "the number of tables");
}
