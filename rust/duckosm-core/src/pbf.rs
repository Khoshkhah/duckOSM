//! Read an `.osm.pbf` file: the highway ways first, then the coordinates of the nodes they use.

use crate::boundary::Boundary;
use crate::filter;
use crate::geom::Pt;
use crate::graph::{MemberType, Relation};
use crate::model::{OtherTags, Tags, Way};
use crate::network::Coords;
use crate::raw::{Raw, RawNode, RawTags, RawWay};
use crate::util::{FxHashMap, FxHashSet};
use osmpbf::{Blob, BlobDecode, ByteOffset, Element, PrimitiveBlock};
pub use osmpbf::{BlobReader, ElementReader};
use rayon::prelude::*;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

fn open(path: &Path) -> Result<ElementReader<std::io::BufReader<std::fs::File>>, String> {
    ElementReader::from_path(path).map_err(|e| e.to_string())
}

fn from_bytes(pbf: &[u8]) -> ElementReader<Cursor<&[u8]>> {
    ElementReader::new(Cursor::new(pbf))
}

/// Pass 1: all highway ways and ferry lines, sorted by id.
pub fn read_ways(path: &Path) -> Result<Vec<Way>, String> {
    ways_of(open(path)?)
}

/// Pass 1 on the bytes of a `.osm.pbf` file (WebAssembly has no file system).
pub fn read_ways_bytes(pbf: &[u8]) -> Result<Vec<Way>, String> {
    ways_of(from_bytes(pbf))
}

/// Pass 2: the coordinates of the nodes in `ways`.
pub fn read_coords(path: &Path, ways: &[Way]) -> Result<Coords, String> {
    coords_of(open(path)?, ways)
}

/// Pass 2 on the bytes of a `.osm.pbf` file.
pub fn read_coords_bytes(pbf: &[u8], ways: &[Way]) -> Result<Coords, String> {
    coords_of(from_bytes(pbf), ways)
}

/// Pass 3: the `type=restriction` relations (the turn restrictions), sorted by id.
pub fn read_restrictions(path: &Path) -> Result<Vec<Relation>, String> {
    restrictions_of(open(path)?)
}

/// Pass 3 on the bytes of a `.osm.pbf` file.
pub fn read_restrictions_bytes(pbf: &[u8]) -> Result<Vec<Relation>, String> {
    restrictions_of(from_bytes(pbf))
}

/// Pass 3 on any reader (the browser reads a file piece by piece).
pub fn restrictions_of<R: Read + Send>(reader: ElementReader<R>) -> Result<Vec<Relation>, String> {
    let mut rels = reader
        .par_map_reduce(
            |el| match el {
                Element::Relation(r) => restriction(&r).into_iter().collect(),
                _ => Vec::new(),
            },
            Vec::new,
            |mut a, mut b| {
                a.append(&mut b);
                a
            },
        )
        .map_err(|e| e.to_string())?;
    rels.sort_by_key(|r| r.id);
    Ok(rels)
}

/// A `type=restriction` relation (duckOSM: `map_extract(tags, 'type')[1] = 'restriction'`).
fn restriction(r: &osmpbf::Relation) -> Option<Relation> {
    (r.tags().find(|t| t.0 == "type").map(|t| t.1) == Some("restriction")).then(|| relation(r))
}

/// A relation with all its tags and members.
fn relation(r: &osmpbf::Relation) -> Relation {
    Relation {
        id: r.id(),
        tags: r
            .tags()
            .map(|(k, x)| (k.to_string(), x.to_string()))
            .collect(),
        members: r
            .members()
            .map(|m| {
                let t = match m.member_type {
                    osmpbf::RelMemberType::Node => MemberType::Node,
                    osmpbf::RelMemberType::Way => MemberType::Way,
                    osmpbf::RelMemberType::Relation => MemberType::Relation,
                };
                (m.member_id, t, m.role().unwrap_or("").to_string())
            })
            .collect(),
    }
}

fn pairs<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> RawTags {
    tags.map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// The tags of the nodes of a block that `wanted` takes, for the nodes that have tags.
fn node_tags(block: &PrimitiveBlock, wanted: impl Fn(i64) -> bool) -> Vec<(i64, RawTags)> {
    let mut v = Vec::new();
    for g in block.groups() {
        for n in g.dense_nodes() {
            if wanted(n.id()) && n.tags().next().is_some() {
                v.push((n.id(), pairs(n.tags())));
            }
        }
        for n in g.nodes() {
            if wanted(n.id()) && n.tags().next().is_some() {
                v.push((n.id(), pairs(n.tags())));
            }
        }
    }
    v
}

/// The nodes of a block.
fn each_node(block: &PrimitiveBlock, mut f: impl FnMut(i64, Pt)) {
    for g in block.groups() {
        for n in g.dense_nodes() {
            f(n.id(), (n.lon(), n.lat()));
        }
        for n in g.nodes() {
            f(n.id(), (n.lon(), n.lat()));
        }
    }
}

fn data(blob: &Blob) -> Result<Option<PrimitiveBlock>, String> {
    match blob.decode().map_err(|e| e.to_string())? {
        BlobDecode::OsmData(b) => Ok(Some(b)),
        _ => Ok(None),
    }
}

/// The next blobs of the file: enough to keep all threads busy, and few, because pass 1 stops in a batch.
fn batch<R: Read + Send>(blobs: &mut BlobReader<R>) -> Result<Vec<Blob>, String> {
    blobs
        .take(2 * rayon::current_num_threads())
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}

/// The data of an area, as osmium's `complete_ways` cut gives it to the network step: the highway ways and ferry
/// lines with at least one node inside the boundary, kept whole, the coordinates of their nodes and of the nodes
/// inside, and all `type=restriction` relations of the file (as [`read_restrictions`]: the graph step keeps the
/// ones that it can use). `open` gives a new reader for each pass:
/// 1. The node blocks: the nodes inside, and where each block is and the range of its node ids. In a file sorted
///    by type (`Sort.Type_then_ID`, as the files of Geofabrik), the pass stops at the first block of ways.
/// 2. The blocks that are not only nodes: the ways with a node inside, and the restrictions.
/// 3. Only the node blocks that have a node of these ways outside the boundary: the coordinates of these nodes.
pub fn read_area<R: Read + Seek + Send>(
    open: impl Fn() -> Result<BlobReader<R>, String>,
    boundary: &Boundary,
) -> Result<(Vec<Way>, Coords, Vec<Relation>), String> {
    let (w, c, r, _) = cut(open, boundary, true, false)?;
    Ok((w, c, r))
}

/// [`read_area`], and also duckOSM's `raw.*` of the area as osmium's `smart` cut gives it (docs/features.md): all
/// nodes inside with their tags, all ways with a node inside, the relations that have a member inside or a member
/// way kept and their parent relations, and the member ways and nodes of the multipolygon relations among them.
/// The ways get all their nodes. Pass 2 also keeps all relations and where each block of ways is; a pass between 2
/// and 3 reads only the blocks of ways that hold a member way of a multipolygon. The network input is the same as
/// from [`read_area`].
pub fn read_area_raw<R: Read + Seek + Send>(
    open: impl Fn() -> Result<BlobReader<R>, String>,
    boundary: &Boundary,
) -> Result<(Vec<Way>, Coords, Vec<Relation>, Raw), String> {
    let (w, c, r, raw) = cut(open, boundary, true, true)?;
    Ok((w, c, r, raw.unwrap_or_default()))
}

/// A block of the file: where it starts and the range of ids of its nodes (or of its ways).
#[derive(Clone, Copy)]
struct Range {
    at: ByteOffset,
    first: i64,
    last: i64,
}

/// The blocks that hold one of the sorted `ids`.
fn blocks_with(blocks: &[Range], ids: &[i64]) -> Vec<ByteOffset> {
    blocks
        .iter()
        .filter(|b| {
            let i = ids.partition_point(|&n| n < b.first);
            i < ids.len() && ids[i] <= b.last
        })
        .map(|b| b.at)
        .collect()
}

/// Decode the blocks at `wanted` on all threads, and `f` on each.
fn each_block<R: Read + Seek + Send, T: Send>(
    blobs: &mut BlobReader<R>,
    wanted: &[ByteOffset],
    f: impl Fn(&PrimitiveBlock) -> T + Sync + Send,
) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    for part in wanted.chunks(2 * rayon::current_num_threads()) {
        let b = part
            .iter()
            .map(|&at| blobs.blob_from_offset(at))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let found = b
            .into_par_iter()
            .map(|blob| Ok(data(&blob)?.map(|block| f(&block))))
            .collect::<Result<Vec<_>, String>>()?;
        out.extend(found.into_iter().flatten());
    }
    Ok(out)
}

/// The network input of an area, and `raw.*` when it was asked for.
type Cut = (Vec<Way>, Coords, Vec<Relation>, Option<Raw>);

/// [`read_area`]; `use_sort: false` reads the file as if it had no sort flag (for the test); `raw`: also `raw.*`.
fn cut<R: Read + Seek + Send>(
    open: impl Fn() -> Result<BlobReader<R>, String>,
    boundary: &Boundary,
    use_sort: bool,
    raw: bool,
) -> Result<Cut, String> {
    let err = |e: osmpbf::Error| e.to_string();
    // pass 1
    let mut blobs = open()?;
    let mut timestamp = None;
    let sorted = match blobs.next().transpose().map_err(err)? {
        Some(h) => match h.decode().map_err(err)? {
            BlobDecode::OsmHeader(h) => {
                timestamp = h.osmosis_replication_timestamp();
                use_sort
                    && h.optional_features()
                        .iter()
                        .any(|f| f == "Sort.Type_then_ID")
            }
            _ => false,
        },
        None => return Err("the PBF file is empty".into()),
    };
    let (mut blocks, mut inside, mut inside_tags, mut ways_at) =
        (Vec::new(), Vec::new(), Vec::new(), None);
    'pass1: loop {
        let b = batch(&mut blobs)?;
        if b.is_empty() {
            break;
        }
        let found = b
            .into_par_iter()
            .map(|blob| -> Result<_, String> {
                let Some(block) = data(&blob)? else {
                    return Ok(None);
                };
                let at = blob.offset().ok_or("the PBF reader gave no offset")?;
                let mut nb = Range {
                    at,
                    first: i64::MAX,
                    last: i64::MIN,
                };
                let mixed = block
                    .groups()
                    .any(|g| g.ways().next().is_some() || g.relations().next().is_some());
                let mut pts = Vec::new();
                each_node(&block, |id, p| {
                    nb.first = nb.first.min(id);
                    nb.last = nb.last.max(id);
                    if boundary.contains(p.0, p.1) {
                        pts.push((id, p));
                    }
                });
                let tags = if raw && !pts.is_empty() {
                    let here: FxHashSet<i64> = pts.iter().map(|n| n.0).collect();
                    node_tags(&block, |id| here.contains(&id))
                } else {
                    Vec::new()
                };
                Ok(Some((nb, mixed, pts, tags)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (nb, mixed, mut pts, mut tags) in found.into_iter().flatten() {
            if nb.first <= nb.last {
                blocks.push((nb, mixed));
                inside.append(&mut pts);
                inside_tags.append(&mut tags);
            }
            if mixed && sorted {
                ways_at = Some(nb.at);
                break 'pass1;
            }
        }
    }
    let ids: FxHashSet<i64> = inside.iter().map(|n| n.0).collect();
    // pass 2: from the first block of ways, or all blocks that are not only nodes
    let only_nodes: FxHashSet<u64> = blocks.iter().filter(|b| !b.1).map(|b| b.0.at.0).collect();
    let blocks: Vec<Range> = blocks.into_iter().map(|b| b.0).collect();
    let mut blobs = open()?;
    if let Some(at) = ways_at {
        blobs.seek(at).map_err(err)?;
    }
    let (mut ways, mut rels) = (Vec::new(), Vec::new());
    let (mut raw_ways, mut all_rels, mut way_blocks) = (Vec::new(), Vec::new(), Vec::new());
    loop {
        let b = batch(&mut blobs)?;
        if b.is_empty() {
            break;
        }
        let found = b
            .into_par_iter()
            .filter(|blob| blob.offset().is_none_or(|o| !only_nodes.contains(&o.0)))
            .map(|blob| -> Result<_, String> {
                let (mut w, mut r, mut rw, mut ar) =
                    (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                let mut wb = Range {
                    at: blob.offset().unwrap_or(ByteOffset(0)),
                    first: i64::MAX,
                    last: i64::MIN,
                };
                if let Some(block) = data(&blob)? {
                    for g in block.groups() {
                        for x in g.ways() {
                            wb.first = wb.first.min(x.id());
                            wb.last = wb.last.max(x.id());
                            if !x.refs().any(|n| ids.contains(&n)) {
                                continue;
                            }
                            w.extend(way(&x));
                            if raw {
                                rw.push(raw_way(&x));
                            }
                        }
                        for x in g.relations() {
                            r.extend(restriction(&x));
                            if raw {
                                ar.push(relation(&x));
                            }
                        }
                    }
                }
                Ok((w, r, rw, ar, wb))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (mut w, mut r, mut rw, mut ar, wb) in found {
            ways.append(&mut w);
            rels.append(&mut r);
            raw_ways.append(&mut rw);
            all_rels.append(&mut ar);
            if wb.first <= wb.last {
                way_blocks.push(wb);
            }
        }
    }
    ways.sort_by_key(|w| w.id);
    rels.sort_by_key(|r| r.id);
    // the network's nodes outside: the nodes inside are known already (a new way of the OSM fixes can use one, as
    // with the osmium cut)
    let outside = |refs: &mut dyn Iterator<Item = i64>| {
        let mut v: Vec<i64> = refs.filter(|n| !ids.contains(n)).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let missing = outside(&mut ways.iter().flat_map(|w| w.refs.iter().copied()));
    let mut raw_out = None;
    let mut wanted_nodes = missing.clone();
    if raw {
        // the relations of the cut (osmium smart): a member inside or a member way kept, then their parents
        let kept: FxHashSet<i64> = raw_ways.iter().map(|w| w.id).collect();
        let direct = |r: &Relation| {
            r.members.iter().any(|m| match m.1 {
                MemberType::Node => ids.contains(&m.0),
                MemberType::Way => kept.contains(&m.0),
                MemberType::Relation => false,
            })
        };
        let mut chosen: FxHashSet<i64> = all_rels
            .iter()
            .filter(|r| direct(r))
            .map(|r| r.id)
            .collect();
        loop {
            let parents: Vec<i64> = all_rels
                .iter()
                .filter(|r| !chosen.contains(&r.id))
                .filter(|r| {
                    r.members
                        .iter()
                        .any(|m| m.1 == MemberType::Relation && chosen.contains(&m.0))
                })
                .map(|r| r.id)
                .collect();
            if parents.is_empty() {
                break;
            }
            chosen.extend(parents);
        }
        // the members of the multipolygons with a member inside or a member way kept
        let (mut more_ways, mut more_nodes) = (Vec::new(), Vec::new());
        for r in all_rels.iter().filter(|r| {
            r.tags
                .iter()
                .any(|t| t.0 == "type" && t.1 == "multipolygon")
                && direct(r)
        }) {
            for m in &r.members {
                match m.1 {
                    MemberType::Way if !kept.contains(&m.0) => more_ways.push(m.0),
                    MemberType::Node if !ids.contains(&m.0) => more_nodes.push(m.0),
                    _ => {}
                }
            }
        }
        all_rels.retain(|r| chosen.contains(&r.id));
        all_rels.sort_by_key(|r| r.id);
        more_ways.sort_unstable();
        more_ways.dedup();
        // the pass for the member ways: only the blocks of ways that hold one
        let mut blobs = open()?;
        let found = each_block(&mut blobs, &blocks_with(&way_blocks, &more_ways), |block| {
            let mut v = Vec::new();
            for g in block.groups() {
                v.extend(
                    g.ways()
                        .filter(|x| more_ways.binary_search(&x.id()).is_ok())
                        .map(|x| raw_way(&x)),
                );
            }
            v
        })?;
        raw_ways.extend(found.into_iter().flatten());
        raw_ways.sort_by_key(|w| w.id);
        let mut all = outside(
            &mut raw_ways
                .iter()
                .flat_map(|w| w.refs.iter().copied())
                .chain(more_nodes),
        );
        // the network's ways are among the raw ways, so `all` has the nodes of `missing`
        std::mem::swap(&mut wanted_nodes, &mut all);
        let tags: FxHashMap<i64, RawTags> = inside_tags.into_iter().collect();
        let mut nodes: Vec<RawNode> = inside
            .iter()
            .map(|&(id, p)| RawNode {
                id,
                lon: p.0,
                lat: p.1,
                tags: tags.get(&id).cloned().unwrap_or_default(),
            })
            .collect();
        nodes.shrink_to_fit();
        raw_out = Some(Raw {
            nodes,
            ways: raw_ways,
            relations: all_rels,
            timestamp,
        });
    }
    // pass 3: the node blocks with a missing node
    let mut blobs = open()?;
    let found = each_block(&mut blobs, &blocks_with(&blocks, &wanted_nodes), |block| {
        let (mut net, mut rn) = (Vec::new(), Vec::new());
        each_node(block, |id, p| {
            if missing.binary_search(&id).is_ok() {
                net.push((id, p));
            }
            if raw && wanted_nodes.binary_search(&id).is_ok() {
                rn.push(RawNode {
                    id,
                    lon: p.0,
                    lat: p.1,
                    tags: Vec::new(),
                });
            }
        });
        if raw && !rn.is_empty() {
            let mut tags =
                node_tags(block, |id| wanted_nodes.binary_search(&id).is_ok()).into_iter();
            let mut next = tags.next();
            // node_tags keeps the order of each_node: dense nodes, then nodes
            for n in &mut rn {
                if next.as_ref().is_some_and(|t| t.0 == n.id) {
                    n.tags = next.take().unwrap().1;
                    next = tags.next();
                }
            }
        }
        (net, rn)
    })?;
    let mut nodes = inside;
    for (mut net, mut rn) in found {
        nodes.append(&mut net);
        if let Some(r) = raw_out.as_mut() {
            r.nodes.append(&mut rn);
        }
    }
    if let Some(r) = raw_out.as_mut() {
        r.nodes.sort_by_key(|n| n.id);
    }
    Ok((ways, Coords::new(nodes), rels, raw_out))
}

/// [`read_area`] on a file.
pub fn read_area_path(
    path: &Path,
    boundary: &Boundary,
) -> Result<(Vec<Way>, Coords, Vec<Relation>), String> {
    read_area(
        || BlobReader::from_path(path).map_err(|e| e.to_string()),
        boundary,
    )
}

/// A way with all its tags, for `raw.ways`.
fn raw_way(w: &osmpbf::Way) -> RawWay {
    RawWay {
        id: w.id(),
        tags: pairs(w.tags()),
        refs: w.refs().collect(),
    }
}

/// A highway way or ferry line that the reader keeps.
fn way(w: &osmpbf::Way) -> Option<Way> {
    let tags = Tags::from_pairs(w.tags());
    filter::is_kept_by_reader(&tags).then(|| Way {
        id: w.id(),
        refs: w.refs().collect(),
        tags,
        other: OtherTags::of(w.tags()),
    })
}

fn ways_of<R: Read + Send>(reader: ElementReader<R>) -> Result<Vec<Way>, String> {
    let mut ways = reader
        .par_map_reduce(
            |el| match el {
                Element::Way(w) => way(&w).into_iter().collect(),
                _ => Vec::new(),
            },
            Vec::new,
            |mut a, mut b| {
                a.append(&mut b);
                a
            },
        )
        .map_err(|e| e.to_string())?;
    ways.sort_by_key(|w| w.id);
    ways.shrink_to_fit(); // the merge of the blocks leaves up to half of the capacity unused
    Ok(ways)
}

fn coords_of<R: Read + Send>(reader: ElementReader<R>, ways: &[Way]) -> Result<Coords, String> {
    // the exact size at once: a set that grows by doubling keeps the old and the new table
    let mut needed: FxHashSet<i64> = FxHashSet::default();
    needed.reserve(ways.iter().map(|w| w.refs.len()).sum());
    needed.extend(ways.iter().flat_map(|w| w.refs.iter().copied()));
    let nodes = reader
        .par_map_reduce(
            |el| match el {
                Element::Node(n) if needed.contains(&n.id()) => vec![(n.id(), (n.lon(), n.lat()))],
                Element::DenseNode(n) if needed.contains(&n.id()) => {
                    vec![(n.id(), (n.lon(), n.lat()))]
                }
                _ => Vec::new(),
            },
            Vec::new,
            |mut a, mut b| {
                a.append(&mut b);
                a
            },
        )
        .map_err(|e| e.to_string())?;
    drop(needed);
    Ok(Coords::new(nodes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file without the sort flag: pass 1 reads all blocks, pass 2 skips the blocks of only nodes. The result
    /// is the same.
    #[test]
    fn cut_without_the_sort_flag() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
        let b = Boundary::parse(&std::fs::read_to_string(data.join("monaco.geojson")).unwrap())
            .unwrap();
        let path = data.join("monaco_sample.osm.pbf");
        let open = || BlobReader::from_path(&path).map_err(|e| e.to_string());
        let (w1, c1, r1, _) = cut(open, &b, true, false).unwrap();
        let (w2, c2, r2, _) = cut(open, &b, false, false).unwrap();
        let ways = |w: &[Way]| w.iter().map(|w| (w.id, w.refs.clone())).collect::<Vec<_>>();
        assert!(!w1.is_empty());
        assert_eq!(ways(&w1), ways(&w2));
        assert_eq!(c1.len(), c2.len());
        for n in w1.iter().flat_map(|w| &w.refs) {
            assert_eq!(c1.get(n), c2.get(n));
        }
        let rels = |r: &[Relation]| r.iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(rels(&r1), rels(&r2));
    }
}
