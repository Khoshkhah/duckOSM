//! Step 0 of a build with a boundary: the cut of the region file to the area, written to a `.osm.pbf` file while the
//! region file is read (docs/concepts/offline-online.md, step 0). The later steps then read the area file as a whole
//! file. The cut keeps only lists of ids in memory, never the elements: each element goes from a block of the region
//! file straight into a block of the area file.
//!
//! The rules are those of [`crate::pbf::read_area_raw`] (osmium's `smart` cut): the nodes inside the boundary; each way
//! with a node inside, whole (with its nodes outside); the relations with a member node inside or a member way kept,
//! and their parent relations; the member ways and nodes of the multipolygons among them. The file has the same
//! elements as [`crate::cutfile::write_pbf`] of that cut, with all their tags.
//!
//! The region file must be sorted by type, then id (`Sort.Type_then_ID`, as the files of Geofabrik), so that the
//! elements arrive in the order of the area file. Four passes read the file: two over the blocks of nodes, two over
//! the blocks of ways and relations.
//! 1. The nodes: the ids of the nodes inside.
//! 2. The ways and relations: the ids of the kept ways, the ids of their nodes outside, the members of the relations.
//!    Then the relations to keep, and the member ways and nodes of the multipolygons (an extra pass over the ways
//!    when a member way is not kept yet).
//! 3. The nodes again: each node on a list goes to the area file.
//! 4. The ways and relations again: each kept way, then each kept relation, goes to the area file.

use crate::boundary::Boundary;
use crate::cutfile::PbfWriter;
use crate::graph::{MemberType, Relation};
use crate::pbfraw::{decode_block, read_block, Buffers};
use crate::raw::{RawNode, RawSink, RawWay};
use crate::util::FxHashSet;
use rayon::prelude::*;
use std::io::{Read, Seek, SeekFrom, Write};

/// The numbers of elements in the area file.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CutCounts {
    pub nodes: usize,
    pub ways: usize,
    pub relations: usize,
}

/// Sorted ids with a directory, as `network::Coords`: a look-up reads one entry of the directory (about one for
/// each 64 ids, so it stays in the cache of the processor) and searches the ids of one range. 8 bytes for each id.
struct IdSet {
    ids: Vec<i64>,
    dir: Vec<u32>,
    min: i64,
    shift: u32,
}

impl IdSet {
    /// From lists of ids (for example one for each block): one list with the exact size, sorted, with no doubles.
    fn new(parts: Vec<Vec<i64>>) -> IdSet {
        let mut ids = Vec::with_capacity(parts.iter().map(Vec::len).sum());
        for p in parts {
            ids.extend(p);
        }
        ids.sort_unstable();
        ids.dedup();
        ids.shrink_to_fit();
        let (Some(&first), Some(&last)) = (ids.first(), ids.last()) else {
            return IdSet { ids, dir: Vec::new(), min: 0, shift: 0 };
        };
        let (min, span) = (first, last.abs_diff(first));
        let ranges = (ids.len() / 64).max(1) as u64;
        let mut shift = 0;
        while shift < 63 && (span >> shift) >= ranges {
            shift += 1;
        }
        let range = |id: i64| (id.abs_diff(min) >> shift) as usize;
        let mut dir = Vec::with_capacity(range(last) + 2);
        for (i, &id) in ids.iter().enumerate() {
            while dir.len() <= range(id) {
                dir.push(i as u32);
            }
        }
        dir.push(ids.len() as u32);
        IdSet { ids, dir, min, shift }
    }

    fn contains(&self, id: i64) -> bool {
        if id < self.min || self.ids.is_empty() {
            return false;
        }
        let b = (id.abs_diff(self.min) >> self.shift) as usize;
        match (self.dir.get(b), self.dir.get(b + 1)) {
            (Some(&lo), Some(&hi)) => self.ids[lo as usize..hi as usize].binary_search(&id).is_ok(),
            _ => false,
        }
    }
}

/// Read the blocks of `r` in batches, decode each batch on all threads with `per_block`, and give the results to
/// `done` in the order of the file. `stop`: after a batch, end when it says so. Gives the number of blocks read.
fn each_block<R: Read, T: Send>(
    r: &mut R,
    mut limit: Option<usize>,
    per_block: impl Fn(&[u8], &mut Buffers) -> Result<T, String> + Sync,
    mut done: impl FnMut(T, u64) -> Result<bool, String>,
) -> Result<(), String> {
    let n = 2 * rayon::current_num_threads();
    let mut at = 0u64;
    loop {
        let mut batch = Vec::with_capacity(n);
        while batch.len() < n && limit != Some(0) {
            match read_block(r)? {
                Some(b) => {
                    limit = limit.map(|l| l - 1);
                    batch.push(b)
                }
                None => break,
            }
        }
        if batch.is_empty() {
            return Ok(());
        }
        let out: Vec<T> = batch
            .par_iter()
            .map_init(Buffers::default, |buf, b| per_block(b, buf))
            .collect::<Result<_, _>>()?;
        for (t, b) in out.into_iter().zip(&batch) {
            let start = at;
            at += b.len() as u64;
            if !done(t, start)? {
                return Ok(());
            }
        }
    }
}

/// Pass 1: the ids of the nodes inside, and whether the block has ways or relations.
struct Inside<'a> {
    boundary: &'a Boundary,
    ids: Vec<i64>,
    other: bool,
}

impl RawSink for Inside<'_> {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        if self.boundary.contains(lon, lat) {
            self.ids.push(id);
        }
    }
    fn way<'a>(&mut self, _: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = i64>) {
        self.other = true;
    }
    fn relation<'a>(
        &mut self,
        _: i64,
        _: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        self.other = true;
    }
}

/// A relation as pass 2 keeps it: its members, with no tags and roles.
struct Rel {
    id: i64,
    multipolygon: bool,
    members: Vec<(i64, MemberType)>,
}

/// Pass 2: the ways with a node inside (`wanted`: else only the ways whose ids are in it), their nodes outside, and
/// the relations.
struct Kept<'a> {
    inside: &'a IdSet,
    wanted: Option<&'a IdSet>,
    ways: Vec<i64>,
    outside: Vec<i64>,
    rels: Vec<Rel>,
    refs: Vec<i64>,
}

impl RawSink for Kept<'_> {
    fn node<'a>(&mut self, _: i64, _: f64, _: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {}
    fn way<'a>(&mut self, id: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        self.refs.clear();
        self.refs.extend(refs);
        let keep = match self.wanted {
            Some(w) => w.contains(id),
            None => self.refs.iter().any(|&n| self.inside.contains(n)),
        };
        if keep {
            self.ways.push(id);
            let inside = self.inside;
            self.outside.extend(self.refs.iter().copied().filter(|&n| !inside.contains(n)));
        }
    }
    fn relation<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        if self.wanted.is_some() {
            return;
        }
        let mut multipolygon = false;
        for (k, v) in tags {
            multipolygon |= k == "type" && v == "multipolygon";
        }
        self.rels.push(Rel {
            id,
            multipolygon,
            members: members.map(|(m, t, _)| (m, t)).collect(),
        });
    }
}

/// Passes 3 and 4: the elements on the lists, whole.
struct Out<'a> {
    nodes_in: [&'a IdSet; 2],
    ways_in: &'a IdSet,
    rels_in: &'a FxHashSet<i64>,
    nodes: Vec<RawNode>,
    ways: Vec<RawWay>,
    rels: Vec<Relation>,
}

impl RawSink for Out<'_> {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        if self.nodes_in.iter().any(|s| s.contains(id)) {
            self.nodes.push(RawNode {
                id,
                lon,
                lat,
                tags: tags.map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            });
        }
    }
    fn way<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        if self.ways_in.contains(id) {
            self.ways.push(RawWay {
                id,
                tags: tags.map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                refs: refs.collect(),
            });
        }
    }
    fn relation<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        if self.rels_in.contains(&id) {
            self.rels.push(Relation {
                id,
                tags: tags.map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                members: members.map(|(m, t, r)| (m, t, r.to_string())).collect(),
            });
        }
    }
}

/// The replication time of the region file's header, and whether the file is sorted by type, then id.
fn header<R: Read + Send>(r: R) -> Result<(Option<i64>, bool), String> {
    let err = |e: osmpbf::Error| e.to_string();
    let mut blobs = osmpbf::BlobReader::new(r);
    match blobs.next().transpose().map_err(err)? {
        Some(h) => match h.decode().map_err(err)? {
            osmpbf::BlobDecode::OsmHeader(h) => Ok((
                h.osmosis_replication_timestamp(),
                h.optional_features().iter().any(|f| f == "Sort.Type_then_ID"),
            )),
            _ => Err("the PBF file has no header".into()),
        },
        None => Err("the PBF file is empty".into()),
    }
}

/// Cut the region file that `open` reads (a new reader for each pass) to `boundary`, and write the area to `out` as
/// a `.osm.pbf` file. Gives `out` back, and the numbers of elements.
pub fn cut_to_pbf<R: Read + Seek + Send, W: Write>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    out: W,
) -> Result<(W, CutCounts), String> {
    let (timestamp, sorted) = header(open()?)?;
    if !sorted {
        return Err("the region file is not sorted by type and id (Sort.Type_then_ID)".into());
    }
    // pass 1: the nodes inside; it stops after the first block with ways or relations
    let (mut parts, mut node_blocks, mut ways_at) = (Vec::new(), 0usize, None);
    each_block(&mut open()?, None, |b, buf| {
        let mut s = Inside { boundary, ids: Vec::new(), other: false };
        decode_block(b, [true, true, true], buf, &mut s)?;
        Ok((s.ids, s.other))
    }, |(ids, other), at| {
        parts.push(ids);
        node_blocks += 1;
        if other {
            ways_at = Some(at);
        }
        Ok(!other)
    })?;
    let inside = IdSet::new(parts);
    let ways_at = ways_at.unwrap_or(0);
    // pass 2: the kept ways, their nodes outside, and the relations
    let pass2 = |wanted: Option<&IdSet>| -> Result<(Vec<Vec<i64>>, Vec<Vec<i64>>, Vec<Rel>), String> {
        let mut r = open()?;
        r.seek(SeekFrom::Start(ways_at)).map_err(|e| e.to_string())?;
        let (mut ways, mut outside, mut rels) = (Vec::new(), Vec::new(), Vec::new());
        each_block(&mut r, None, |b, buf| {
            let mut s = Kept { inside: &inside, wanted, ways: Vec::new(), outside: Vec::new(), rels: Vec::new(), refs: Vec::new() };
            decode_block(b, [false, true, true], buf, &mut s)?;
            Ok((s.ways, s.outside, s.rels))
        }, |(w, o, mut rl), _| {
            ways.push(w);
            outside.push(o);
            rels.append(&mut rl);
            Ok(true)
        })?;
        Ok((ways, outside, rels))
    };
    let (way_parts, mut outside, rels) = pass2(None)?;
    let mut kept = IdSet::new(way_parts);
    // the relations: a member node inside or a member way kept, then their parents
    let direct = |r: &Rel| {
        r.members.iter().any(|&(m, t)| match t {
            MemberType::Node => inside.contains(m),
            MemberType::Way => kept.contains(m),
            MemberType::Relation => false,
        })
    };
    let mut chosen: FxHashSet<i64> = rels.iter().filter(|r| direct(r)).map(|r| r.id).collect();
    loop {
        let parents: Vec<i64> = rels
            .iter()
            .filter(|r| !chosen.contains(&r.id))
            .filter(|r| r.members.iter().any(|&(m, t)| t == MemberType::Relation && chosen.contains(&m)))
            .map(|r| r.id)
            .collect();
        if parents.is_empty() {
            break;
        }
        chosen.extend(parents);
    }
    // the members of the multipolygons with a member inside or a member way kept
    let (mut more_ways, mut more_nodes) = (Vec::new(), Vec::new());
    for r in rels.iter().filter(|r| r.multipolygon && direct(r)) {
        for &(m, t) in &r.members {
            match t {
                MemberType::Way if !kept.contains(m) => more_ways.push(m),
                MemberType::Node if !inside.contains(m) => more_nodes.push(m),
                _ => {}
            }
        }
    }
    drop(rels);
    outside.push(more_nodes);
    if !more_ways.is_empty() {
        let wanted = IdSet::new(vec![more_ways]);
        let (found, more_outside, _) = pass2(Some(&wanted))?;
        let mut all = vec![std::mem::take(&mut kept.ids)];
        all.extend(found);
        kept = IdSet::new(all);
        outside.extend(more_outside);
    }
    let outside = IdSet::new(outside);
    let mut w = PbfWriter::new(out, timestamp)?;
    let mut counts = CutCounts::default();
    // pass 3: the nodes; pass 4: the ways and relations
    let emit = |r: &mut R, limit: Option<usize>, kinds: [bool; 3], w: &mut PbfWriter<W>, counts: &mut CutCounts| {
        each_block(r, limit, |b, buf| {
            let mut s = Out {
                nodes_in: [&inside, &outside],
                ways_in: &kept,
                rels_in: &chosen,
                nodes: Vec::new(),
                ways: Vec::new(),
                rels: Vec::new(),
            };
            decode_block(b, kinds, buf, &mut s)?;
            Ok((s.nodes, s.ways, s.rels))
        }, |(n, x, rl), _| {
            counts.nodes += n.len();
            counts.ways += x.len();
            counts.relations += rl.len();
            for e in n {
                w.node(e)?;
            }
            for e in x {
                w.way(e)?;
            }
            for e in rl {
                w.relation(e)?;
            }
            Ok(true)
        })
    };
    emit(&mut open()?, Some(node_blocks), [true, false, false], &mut w, &mut counts)?;
    let mut r = open()?;
    r.seek(SeekFrom::Start(ways_at)).map_err(|e| e.to_string())?;
    emit(&mut r, None, [false, true, true], &mut w, &mut counts)?;
    Ok((w.finish()?, counts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The cut that streams writes the same bytes as the cut in memory (`read_area_raw`, then `cutfile::write_pbf`).
    #[test]
    fn the_same_file_as_the_cut_in_memory() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
        let pbf = dir.join("monaco.osm.pbf");
        let boundary = Boundary::parse(&std::fs::read_to_string(dir.join("monaco.geojson")).unwrap()).unwrap();
        let (_, _, _, raw) = crate::pbf::read_area_raw(
            || crate::pbf::BlobReader::from_path(&pbf).map_err(|e| e.to_string()),
            &boundary,
        )
        .unwrap();
        let mut want = Vec::new();
        crate::cutfile::write_pbf(&raw, raw.timestamp, &mut want).unwrap();
        let open = || std::fs::File::open(&pbf).map(std::io::BufReader::new).map_err(|e| e.to_string());
        let (got, n) = cut_to_pbf(open, &boundary, Vec::new()).unwrap();
        assert_eq!((n.nodes, n.ways, n.relations), (raw.nodes.len(), raw.ways.len(), raw.relations.len()));
        assert!(n.nodes > 0 && n.ways > 0 && n.relations > 0);
        assert!(got == want, "the files differ: {} and {} bytes", got.len(), want.len());
    }
}
