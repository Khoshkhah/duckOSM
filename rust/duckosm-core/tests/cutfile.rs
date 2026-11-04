//! The cut file (`cutfile.rs`): the Monaco raw cut, written as `.osm.pbf` and `.osm` and read again, has the same
//! nodes (coordinate bits), ways and relations as the cut. osmium reads both files, if it is installed.

use duckosm_core::boundary::Boundary;
use duckosm_core::cutfile::{write_pbf, write_xml};
use duckosm_core::graph::MemberType;
use duckosm_core::pbf;
use duckosm_core::raw::Raw;
use osmpbf::{Element, ElementReader, RelMemberType};
use std::path::{Path, PathBuf};
use std::process::Command;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data")
        .join(name)
}

type Tags = Vec<(String, String)>;
type Member = (i64, MemberType, String);
type Node = (i64, u64, u64, Tags);
type Way = (i64, Tags, Vec<i64>);
type Rel = (i64, Tags, Vec<Member>);

fn tags<'a>(t: impl Iterator<Item = (&'a str, &'a str)>) -> Tags {
    t.map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// The elements of a `.osm.pbf` file, in file order.
fn read(path: &Path) -> (Vec<Node>, Vec<Way>, Vec<Rel>) {
    let (mut nodes, mut ways, mut rels) = (vec![], vec![], vec![]);
    ElementReader::from_path(path)
        .unwrap()
        .for_each(|el| match el {
            Element::DenseNode(n) => {
                nodes.push((n.id(), n.lon().to_bits(), n.lat().to_bits(), tags(n.tags())))
            }
            Element::Node(n) => {
                nodes.push((n.id(), n.lon().to_bits(), n.lat().to_bits(), tags(n.tags())))
            }
            Element::Way(w) => ways.push((w.id(), tags(w.tags()), w.refs().collect())),
            Element::Relation(r) => rels.push((
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
    (nodes, ways, rels)
}

fn cut() -> (Raw, Option<i64>) {
    let region = data("monaco_sample.osm.pbf");
    let b = Boundary::parse(&std::fs::read_to_string(data("monaco.geojson")).unwrap()).unwrap();
    let open = || pbf::BlobReader::from_path(&region).map_err(|e| e.to_string());
    let (.., raw) = pbf::read_area_raw(open, &b).unwrap();
    let t = raw.timestamp;
    (raw, t)
}

fn same(path: &Path, raw: &Raw) {
    let (n, w, r) = read(path);
    assert_eq!(n.len(), raw.nodes.len());
    for (a, b) in n.iter().zip(&raw.nodes) {
        assert_eq!(
            (a.0, a.1, a.2, &a.3),
            (b.id, b.lon.to_bits(), b.lat.to_bits(), &b.tags)
        );
    }
    assert_eq!(w.len(), raw.ways.len());
    for (a, b) in w.iter().zip(&raw.ways) {
        assert_eq!((a.0, &a.1, &a.2), (b.id, &b.tags, &b.refs));
    }
    assert_eq!(r.len(), raw.relations.len());
    for (a, b) in r.iter().zip(&raw.relations) {
        assert_eq!((a.0, &a.1, &a.2), (b.id, &b.tags, &b.members));
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("duckosm-cutfile-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn osmium(args: &[&str]) -> Option<String> {
    let o = Command::new("osmium").args(args).output().ok()?;
    assert!(
        o.status.success(),
        "osmium {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

#[test]
fn written_pbf_reads_back_the_cut() {
    let (raw, ts) = cut();
    let path = tmp("cut.osm.pbf");
    let t = std::time::Instant::now();
    write_pbf(&raw, ts, &mut std::fs::File::create(&path).unwrap()).unwrap();
    eprintln!(
        "write_pbf: {:.3} s, {} bytes",
        t.elapsed().as_secs_f64(),
        std::fs::metadata(&path).unwrap().len()
    );
    same(&path, &raw);
    // the cut is the osmium smart cut (tests/raw.rs), so the file has its elements, too
    assert_eq!(
        read(&path).1.len(),
        read(&data("monaco_smart.osm.pbf")).1.len()
    );
    // the header: the sort flag and the time of the region file
    if let Some(info) = osmium(&["fileinfo", "-e", path.to_str().unwrap()]) {
        assert!(info.contains("Sort.Type_then_ID"), "{info}");
        assert!(
            info.contains("osmosis_replication_timestamp=2026-09-29T20:22:51Z"),
            "{info}"
        );
        assert!(
            info.contains(&format!("Number of nodes: {}", raw.nodes.len())),
            "{info}"
        );
        assert!(
            info.contains(&format!("Number of ways: {}", raw.ways.len())),
            "{info}"
        );
        assert!(
            info.contains(&format!("Number of relations: {}", raw.relations.len())),
            "{info}"
        );
        assert!(
            info.contains("Objects ordered (by type and id): yes"),
            "{info}"
        );
    }
}

#[test]
fn written_xml_reads_back_the_cut() {
    let (raw, _) = cut();
    let xml = tmp("cut.osm");
    write_xml(&raw, &mut std::fs::File::create(&xml).unwrap()).unwrap();
    // osmium reads the XML and writes a PBF; its elements are the cut's
    let back = tmp("from_xml.osm.pbf");
    if osmium(&[
        "cat",
        "-f",
        "pbf",
        "-o",
        back.to_str().unwrap(),
        "--overwrite",
        xml.to_str().unwrap(),
    ])
    .is_some()
    {
        same(&back, &raw);
    }
}
