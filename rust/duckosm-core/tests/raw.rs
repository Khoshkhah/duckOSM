//! The Rust `raw` cut equals osmium's `smart` cut: tests/data/monaco_smart.osm.pbf is duckOSM's osmium cut of
//! monaco_sample.osm.pbf with monaco.geojson when it builds the base-map layers (the `raw.*` of the Monaco golden).
//! The Rust cut of the sample gives the same nodes, ways and relations, with the same tags and coordinate bits.

use duckosm_core::boundary::Boundary;
use duckosm_core::graph::MemberType;
use duckosm_core::pbf;
use osmpbf::{Element, ElementReader, RelMemberType};
use std::path::PathBuf;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data")
        .join(name)
}

type Tags = Vec<(String, String)>;
type Member = (i64, MemberType, String);

#[derive(Default)]
struct File {
    nodes: Vec<(i64, u64, u64, Tags)>,
    ways: Vec<(i64, Tags, Vec<i64>)>,
    relations: Vec<(i64, Tags, Vec<Member>)>,
}

fn tags<'a>(t: impl Iterator<Item = (&'a str, &'a str)>) -> Tags {
    t.map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn read(path: &PathBuf) -> File {
    let mut f = File::default();
    ElementReader::from_path(path)
        .unwrap()
        .for_each(|el| match el {
            Element::DenseNode(n) => {
                f.nodes
                    .push((n.id(), n.lon().to_bits(), n.lat().to_bits(), tags(n.tags())))
            }
            Element::Node(n) => {
                f.nodes
                    .push((n.id(), n.lon().to_bits(), n.lat().to_bits(), tags(n.tags())))
            }
            Element::Way(w) => f.ways.push((w.id(), tags(w.tags()), w.refs().collect())),
            Element::Relation(r) => f.relations.push((
                r.id(),
                tags(r.tags()),
                r.members()
                    .map(|m| {
                        let t = match m.member_type {
                            RelMemberType::Node => MemberType::Node,
                            RelMemberType::Way => MemberType::Way,
                            RelMemberType::Relation => MemberType::Relation,
                        };
                        (m.member_id, t, m.role().unwrap().to_string())
                    })
                    .collect(),
            )),
        })
        .unwrap();
    f
}

#[test]
fn rust_raw_cut_equals_osmium_smart_cut() {
    check(
        &data("monaco_sample.osm.pbf"),
        &data("monaco.geojson"),
        &data("monaco_smart.osm.pbf"),
    );
}

/// Another area: RAW_CHECK="region.osm.pbf boundary.geojson osmium_smart_cut.osm.pbf" cargo test --release --test raw -- --ignored
#[test]
#[ignore]
fn rust_raw_cut_equals_osmium_smart_cut_of_another_area() {
    let v = std::env::var("RAW_CHECK")
        .expect("RAW_CHECK: the region file, the boundary and osmium's smart cut");
    let p: Vec<PathBuf> = v.split_whitespace().map(PathBuf::from).collect();
    check(&p[0], &p[1], &p[2]);
}

fn check(region: &PathBuf, boundary: &PathBuf, osmium_cut: &PathBuf) {
    let b = Boundary::parse(&std::fs::read_to_string(boundary).unwrap()).unwrap();
    let open = || pbf::BlobReader::from_path(region).map_err(|e| e.to_string());
    let t = std::time::Instant::now();
    let (ways, _, _, raw) = pbf::read_area_raw(open, &b).unwrap();
    eprintln!("raw cut: {:.2} s", t.elapsed().as_secs_f64());
    let want = read(osmium_cut);
    // the network input is the one of read_area
    let (ways2, _, _) = pbf::read_area_path(region, &b).unwrap();
    assert_eq!(
        ways.iter().map(|w| w.id).collect::<Vec<_>>(),
        ways2.iter().map(|w| w.id).collect::<Vec<_>>()
    );

    let ids = |v: &mut dyn Iterator<Item = i64>| v.collect::<Vec<_>>();
    let (got_n, want_n) = (
        ids(&mut raw.nodes.iter().map(|n| n.id)),
        ids(&mut want.nodes.iter().map(|n| n.0)),
    );
    let missing: Vec<_> = want_n
        .iter()
        .filter(|n| got_n.binary_search(n).is_err())
        .take(5)
        .collect();
    let extra: Vec<_> = got_n
        .iter()
        .filter(|n| want_n.binary_search(n).is_err())
        .take(5)
        .collect();
    assert_eq!(
        got_n.len(),
        want_n.len(),
        "nodes: missing {missing:?}, extra {extra:?}"
    );
    for (a, w) in raw.nodes.iter().zip(&want.nodes) {
        assert_eq!(
            (a.id, a.lon.to_bits(), a.lat.to_bits(), &a.tags),
            (w.0, w.1, w.2, &w.3)
        );
    }
    let (got_w, want_w) = (
        ids(&mut raw.ways.iter().map(|w| w.id)),
        ids(&mut want.ways.iter().map(|w| w.0)),
    );
    let missing: Vec<_> = want_w
        .iter()
        .filter(|n| got_w.binary_search(n).is_err())
        .take(5)
        .collect();
    let extra: Vec<_> = got_w
        .iter()
        .filter(|n| want_w.binary_search(n).is_err())
        .take(5)
        .collect();
    assert_eq!(
        got_w.len(),
        want_w.len(),
        "ways: missing {missing:?}, extra {extra:?}"
    );
    for (a, w) in raw.ways.iter().zip(&want.ways) {
        assert_eq!((a.id, &a.tags, &a.refs), (w.0, &w.1, &w.2));
    }
    let (got_r, want_r) = (
        ids(&mut raw.relations.iter().map(|r| r.id)),
        ids(&mut want.relations.iter().map(|r| r.0)),
    );
    let missing: Vec<_> = want_r
        .iter()
        .filter(|n| got_r.binary_search(n).is_err())
        .take(5)
        .collect();
    let extra: Vec<_> = got_r
        .iter()
        .filter(|n| want_r.binary_search(n).is_err())
        .take(5)
        .collect();
    assert_eq!(
        got_r.len(),
        want_r.len(),
        "relations: missing {missing:?}, extra {extra:?}"
    );
    for (a, w) in raw.relations.iter().zip(&want.relations) {
        assert_eq!((a.id, &a.tags), (w.0, &w.1));
        // osmium keeps all members of a relation, also the ones that are not in the cut
        assert_eq!(a.members, w.2, "relation {}", a.id);
    }
}
