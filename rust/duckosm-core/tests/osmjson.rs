//! The OSM JSON reader gives the same data as the PBF reader: the Monaco PBF is written as OSM JSON (as Overpass
//! writes it: coordinates with 7 decimals, tags as an object, members with type, ref and role), read back, and
//! compared with the PBF reader: the ways, their tags, the coordinate bits and the restriction relations. The cut
//! of OSM JSON to a boundary keeps the same ways as the cut of the PBF reader.

use duckosm_core::boundary::Boundary;
use duckosm_core::osmjson::OsmJson;
use duckosm_core::pbf;
use osmpbf::{Element, ElementReader, RelMemberType};
use std::fmt::Write;
use std::path::PathBuf;

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn tags<'a>(t: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    let kv: Vec<String> = t
        .map(|(k, v)| format!("{}:{}", json_str(k), json_str(v)))
        .collect();
    format!("{{{}}}", kv.join(","))
}

/// A coordinate with 7 decimals, from decimicrodegrees.
fn deg7(d: i32) -> String {
    let v = (d as i64).abs();
    format!(
        "{}{}.{:07}",
        if d < 0 { "-" } else { "" },
        v / 10_000_000,
        v % 10_000_000
    )
}

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data")
        .join(name)
}

/// The PBF file as OSM JSON.
fn to_json(path: &PathBuf) -> String {
    let mut els: Vec<String> = Vec::new();
    ElementReader::from_path(path)
        .unwrap()
        .for_each(|el| {
            let mut s = String::new();
            match el {
                Element::Node(n) => write!(
                    s,
                    r#"{{"type":"node","id":{},"lat":{},"lon":{}}}"#,
                    n.id(),
                    deg7(n.decimicro_lat()),
                    deg7(n.decimicro_lon())
                ),
                Element::DenseNode(n) => write!(
                    s,
                    r#"{{"type":"node","id":{},"lat":{},"lon":{}}}"#,
                    n.id(),
                    deg7(n.decimicro_lat()),
                    deg7(n.decimicro_lon())
                ),
                Element::Way(w) => {
                    let refs: Vec<String> = w.refs().map(|r| r.to_string()).collect();
                    write!(
                        s,
                        r#"{{"type":"way","id":{},"nodes":[{}],"tags":{}}}"#,
                        w.id(),
                        refs.join(","),
                        tags(w.tags())
                    )
                }
                Element::Relation(r) => {
                    let m: Vec<String> = r
                        .members()
                        .map(|m| {
                            let t = match m.member_type {
                                RelMemberType::Node => "node",
                                RelMemberType::Way => "way",
                                RelMemberType::Relation => "relation",
                            };
                            format!(
                                r#"{{"type":"{t}","ref":{},"role":{}}}"#,
                                m.member_id,
                                json_str(m.role().unwrap())
                            )
                        })
                        .collect();
                    write!(
                        s,
                        r#"{{"type":"relation","id":{},"members":[{}],"tags":{}}}"#,
                        r.id(),
                        m.join(","),
                        tags(r.tags())
                    )
                }
            }
            .unwrap();
            els.push(s);
        })
        .unwrap();
    format!(r#"{{"version":0.6,"elements":[{}]}}"#, els.join(","))
}

#[test]
fn osm_json_equals_pbf() {
    let path = data("monaco.osm.pbf");
    let json = to_json(&path);
    let data = OsmJson::read(json.as_bytes()).unwrap();
    let ways = pbf::read_ways(&path).unwrap();
    assert_eq!(data.ways.len(), ways.len());
    for (a, b) in data.ways.iter().zip(&ways) {
        assert_eq!((a.id, &a.refs), (b.id, &b.refs));
        let (mut ta, mut tb): (Vec<_>, Vec<_>) =
            (a.tags.osm_pairs().collect(), b.tags.osm_pairs().collect());
        ta.sort();
        tb.sort();
        assert_eq!(ta, tb, "tags of way {}", a.id);
    }
    let (ca, cb) = (
        data.coords(&data.ways),
        pbf::read_coords(&path, &ways).unwrap(),
    );
    assert_eq!(ca.len(), cb.len());
    for w in &ways {
        for n in &w.refs {
            let (a, b) = (ca.get(n), cb.get(n));
            assert_eq!(
                a.map(|p| (p.0.to_bits(), p.1.to_bits())),
                b.map(|p| (p.0.to_bits(), p.1.to_bits())),
                "node {n}"
            );
        }
    }
    let rels = pbf::read_restrictions(&path).unwrap();
    assert_eq!(data.relations.len(), rels.len());
    assert!(!rels.is_empty());
    for (a, b) in data.relations.iter().zip(&rels) {
        let (mut ta, mut tb) = (a.tags.clone(), b.tags.clone());
        ta.sort();
        tb.sort();
        assert_eq!((a.id, ta, &a.members), (b.id, tb, &b.members));
    }
}

#[test]
fn osm_json_cut_equals_pbf_cut() {
    let sample = data("monaco_sample.osm.pbf");
    let b = Boundary::parse(&std::fs::read_to_string(data("monaco.geojson")).unwrap()).unwrap();
    let mut json = OsmJson::read(to_json(&sample).as_bytes()).unwrap();
    let all = json.ways.len();
    json.cut(&b);
    let (ways, _, _) = pbf::read_area_path(&sample, &b).unwrap();
    assert!(json.ways.len() < all);
    let ids = |w: &[duckosm_core::model::Way]| w.iter().map(|w| w.id).collect::<Vec<_>>();
    assert_eq!(ids(&json.ways), ids(&ways));
}
