//! Step 0 of a build by tiles: one read of the region file gives one area file for each tile ([`crate::tiles`]). Each tile
//! file has the elements that [`crate::cutstream::cut_where`] gives for the tile: the nodes inside the boundary and in the
//! tile, the ways with such a node, whole, the relations with a member there and their parent relations, and the members
//! of those multipolygons.
//!
//! A node has its place, so its tile is known when it is read. A way and a relation have only the ids of their members,
//! and they come after the nodes in the file: the cut keeps the tile of each node in memory, as runs of ids, compressed
//! ([`Book`]: nodes with near ids are usually made together, in one place). The ways get a book too. Each element goes
//! into a part file of its tile when it is read; nothing else of the elements stays in memory.
//!
//! 1. The nodes: each node inside goes to the node part of its tile; the book of the nodes.
//! 2. The ways and relations: each way with a node inside goes to the way part of each of its tiles; the nodes that a
//!    tile needs from outside it go to a temporary list; the relations go whole to a temporary file.
//! 3. The relations: their tiles (a member there, their parents), each to the relation part of each of its tiles; the
//!    members of the multipolygons.
//! 4. The blocks with a node from outside a tile or a member way of a multipolygon: each such element goes to the extra
//!    part of its tile.
//! 5. Each tile: its parts, merged in the order of the ids, give its file; the parts go.

use crate::boundary::Boundary;
use crate::cutfile::{file_blob, nodes_block, relations_block, ways_block, PbfWriter, BLOCK};
use crate::cutstream::{each_block, each_block_at, header, widen, CutCounts, Span, EMPTY};
use crate::graph::{MemberType, Relation};
use crate::pbfraw::{decode_block, read_block, Buffers};
use crate::raw::{RawNode, RawSink, RawWay};
use crate::spill::{sort_bytes, Codec, Writer};
use crate::tiles::Tile;
use crate::util::FxHashMap;
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Ids with a tile, as runs (first id, last id, tile): ids that follow each other in the file, with no element of the
/// kind between them that is not in the tile. Ids in no run have no tile.
#[derive(Default)]
struct Runs {
    runs: Vec<(i64, i64, u32)>,
    /// the last element given to `push` was in the last run
    open: bool,
}

impl Runs {
    fn push(&mut self, id: i64, tile: Option<u32>) {
        match (tile, self.runs.last_mut()) {
            (Some(t), Some(r)) if self.open && r.2 == t => r.1 = id,
            (Some(t), _) => self.runs.push((id, id, t)),
            (None, _) => {}
        }
        self.open = tile.is_some();
    }

    #[cfg(test)]
    fn get(&self, id: i64) -> Option<u32> {
        let i = self.runs.partition_point(|r| r.0 <= id);
        let r = self.runs.get(i.checked_sub(1)?)?;
        (id <= r.1).then_some(r.2)
    }

    #[cfg(test)]
    /// Is an id of a run in `lo..=hi`?
    fn any_in(&self, lo: i64, hi: i64) -> bool {
        let i = self.runs.partition_point(|r| r.1 < lo);
        i < self.runs.len() && self.runs[i].0 <= hi
    }
}

/// The runs of a whole file, compressed (a phone book): pages of `PAGE` runs; in a page each run is three varints (the
/// gap from the end of the run before, the length, the tile). Only the first id and the place of each page are full
/// numbers. A look-up searches the pages, then reads at most `PAGE` runs. The last run stays open, so that the next
/// block can make it longer.
#[derive(Default)]
struct Book {
    pages: Vec<(i64, usize)>,
    bytes: Vec<u8>,
    in_page: usize,
    end: i64,
    last: Option<(i64, i64, u32)>,
    open: bool,
    runs: usize,
}


const PAGE: usize = 32;

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(b: &[u8], at: &mut usize) -> u64 {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let x = b[*at];
        *at += 1;
        v |= ((x & 0x7f) as u64) << shift;
        if x < 0x80 {
            return v;
        }
        shift += 7;
    }
}

impl Book {
    fn pack(&mut self, (first, last, tile): (i64, i64, u32)) {
        if self.in_page == PAGE || self.pages.is_empty() {
            self.pages.push((first, self.bytes.len()));
            self.in_page = 0;
            self.end = first;
        }
        put_varint(&mut self.bytes, first.abs_diff(self.end));
        put_varint(&mut self.bytes, last.abs_diff(first));
        put_varint(&mut self.bytes, tile as u64);
        self.end = last;
        self.in_page += 1;
        self.runs += 1;
    }

    /// Add the runs of the next block (made by `push` from an empty `Runs`; `first_open`: its first element had a tile).
    fn append(&mut self, next: Runs, first_open: bool) {
        let mut it = next.runs.into_iter();
        if let (Some(r), true) = (self.last.as_mut(), self.open && first_open) {
            if let Some(n) = it.next() {
                if n.2 == r.2 {
                    r.1 = n.1;
                } else if let Some(old) = self.last.replace(n) {
                    self.pack(old);
                }
            }
        }
        for n in it {
            if let Some(old) = self.last.replace(n) {
                self.pack(old);
            }
        }
        self.open = next.open;
    }

    fn finish(&mut self) {
        if let Some(old) = self.last.take() {
            self.pack(old);
        }
        self.bytes.shrink_to_fit();
        self.pages.shrink_to_fit();
    }

    /// The bytes of page `i`.
    fn page_bytes<T>(&self, i: usize, f: impl FnOnce(&[u8]) -> T) -> T {
        let end = self.pages.get(i + 1).map_or(self.bytes.len(), |p| p.1);
        f(&self.bytes[self.pages[i].1..end])
    }

    /// Scan the runs from the page that can hold `id`: `f(first, last, tile)` gives Some(answer) to stop, or None to go on.
    fn scan<T>(&self, id: i64, mut f: impl FnMut(i64, i64, u32) -> Option<T>) -> Option<T> {
        let mut i = self.pages.partition_point(|p| p.0 <= id).saturating_sub(1);
        while i < self.pages.len() {
            let found = self.page_bytes(i, |b| {
                let (mut at, mut end) = (0, self.pages[i].0);
                while at < b.len() {
                    let first = end + get_varint(b, &mut at) as i64;
                    let last = first + get_varint(b, &mut at) as i64;
                    let tile = get_varint(b, &mut at) as u32;
                    end = last;
                    if let Some(x) = f(first, last, tile) {
                        return Some(Some(x));
                    }
                }
                None
            });
            match found {
                Some(x) => return x,
                None => i += 1,
            }
        }
        None
    }

    fn get(&self, id: i64) -> Option<u32> {
        self.scan(id, |first, last, tile| {
            if first > id {
                Some(None)
            } else if id <= last {
                Some(Some(tile))
            } else {
                None
            }
        })
        .flatten()
    }

    #[cfg(test)]
    /// Is an id of a run in `lo..=hi`?
    fn any_in(&self, lo: i64, hi: i64) -> bool {
        self.scan(lo, |first, last, _| {
            if first > hi {
                Some(false)
            } else if last >= lo {
                Some(true)
            } else {
                None
            }
        })
        .unwrap_or(false)
    }

    fn len(&self) -> usize {
        self.runs
    }

    fn size(&self) -> usize {
        self.bytes.len() + self.pages.len() * 16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The book gives the tile of each id as the list of runs does, also across pages and blocks.
    #[test]
    fn the_book_finds_the_runs() {
        let mut want = Runs::default();
        let mut book = Book::default();
        let mut x: u64 = 7;
        let mut id = 1_000i64;
        for block in 0..20 {
            let mut b = BlockRuns::new();
            for _ in 0..200 {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                id += 1 + (x >> 60) as i64;
                let tile = match (x >> 33) % 5 {
                    0 => None,
                    k => Some((k as u32 + block) % 3),
                };
                b.push(id, tile);
                want.push(id, tile);
            }
            book.append(b.runs, b.first.unwrap_or(false));
        }
        book.finish();
        for probe in 990..=id + 5 {
            assert_eq!(book.get(probe), want.get(probe), "id {probe}");
            assert_eq!(book.any_in(probe, probe + 3), want.any_in(probe, probe + 3), "span {probe}");
        }
        assert!(book.len() <= want.runs.len());
    }
}

/// Runs made one block at a time, on many threads: the runs of the block, and whether its first element had a tile.
struct BlockRuns {
    runs: Runs,
    first: Option<bool>,
}

impl BlockRuns {
    fn new() -> BlockRuns {
        BlockRuns { runs: Runs::default(), first: None }
    }
    fn push(&mut self, id: i64, tile: Option<u32>) {
        self.first.get_or_insert(tile.is_some());
        self.runs.push(id, tile);
    }
}

/// (id, tile) pairs, sorted, with no doubles.
fn tidy(v: &mut Vec<(i64, u32)>) {
    v.sort_unstable();
    v.dedup();
    v.shrink_to_fit();
}

/// The tiles of `id` in the sorted pairs.
fn tiles_in(pairs: &[(i64, u32)], id: i64) -> impl Iterator<Item = u32> + '_ {
    let i = pairs.partition_point(|p| p.0 < id);
    pairs[i..].iter().take_while(move |p| p.0 == id).map(|p| p.1)
}

/// Is an id of the sorted pairs in `lo..=hi`?
fn pairs_in(pairs: &[(i64, u32)], lo: i64, hi: i64) -> bool {
    let i = pairs.partition_point(|p| p.0 < lo);
    i < pairs.len() && pairs[i].0 <= hi
}

/// The ways of `wanted` (sorted) with their nodes.
struct Wanted<'a> {
    wanted: &'a [i64],
    found: Vec<(i64, Vec<i64>)>,
}

impl RawSink for Wanted<'_> {
    fn node<'a>(&mut self, _: i64, _: f64, _: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {}
    fn way<'a>(&mut self, id: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        if self.wanted.binary_search(&id).is_ok() {
            self.found.push((id, refs.collect()));
        }
    }
    fn relation<'a>(
        &mut self,
        _: i64,
        _: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
    }
}

/// An element of the region file, whole.
#[derive(Clone)]
enum Elem {
    Node(RawNode),
    Way(RawWay),
    Relation(Relation),
}

impl Elem {
    fn id(&self) -> i64 {
        match self {
            Elem::Node(n) => n.id,
            Elem::Way(w) => w.id,
            Elem::Relation(r) => r.id,
        }
    }
}

/// The elements of a block, whole (all kinds that `decode_block` keeps).
#[derive(Default)]
struct All(Vec<Elem>);

impl RawSink for All {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        let tags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.0.push(Elem::Node(RawNode { id, lon, lat, tags }));
    }
    fn way<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        let tags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.0.push(Elem::Way(RawWay { id, tags, refs: refs.collect() }));
    }
    fn relation<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        let tags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let members = members.map(|(m, t, r)| (m, t, r.to_string())).collect();
        self.0.push(Elem::Relation(Relation { id, tags, members }));
    }
}

/// The blobs of elements of one kind (sorted by id), BLOCK elements in each, as in a `.osm.pbf` file. The elements move
/// into the blocks (no copy).
fn blobs(elems: Vec<Elem>, out: &mut Vec<u8>) -> Result<(), String> {
    let mut it = elems.into_iter().peekable();
    while it.peek().is_some() {
        let chunk: Vec<Elem> = it.by_ref().take(BLOCK).collect();
        let block = match &chunk[0] {
            Elem::Node(_) => nodes_block(&chunk.into_iter().filter_map(|e| if let Elem::Node(n) = e { Some(n) } else { None }).collect::<Vec<_>>()),
            Elem::Way(_) => ways_block(&chunk.into_iter().filter_map(|e| if let Elem::Way(w) = e { Some(w) } else { None }).collect::<Vec<_>>()),
            Elem::Relation(_) => relations_block(
                &chunk.into_iter().filter_map(|e| if let Elem::Relation(r) = e { Some(r) } else { None }).collect::<Vec<_>>(),
            ),
        };
        file_blob(out, "OSMData", &block)?;
    }
    Ok(())
}

/// Elements with their tiles to blobs for each tile (on the thread of the block): (tile, blobs). The elements move.
fn by_tile(mut elems: Vec<(u32, Elem)>) -> Result<Vec<(u32, Vec<u8>)>, String> {
    elems.sort_by_key(|e| e.0); // stable: the order of the ids in each tile
    let mut out = Vec::new();
    let mut it = elems.into_iter().peekable();
    while let Some(&(t, _)) = it.peek() {
        let group: Vec<Elem> = std::iter::from_fn(|| it.next_if(|e| e.0 == t)).map(|e| e.1).collect();
        let mut bytes = Vec::new();
        blobs(group, &mut bytes)?;
        out.push((t, bytes));
    }
    Ok(out)
}

/// The part files of the tiles in a temporary folder. The blobs of a part wait in memory until a limit, then go to the
/// end of their files (a file is open only while it is written: the tiles are more than a process can keep open).
struct Parts {
    dir: PathBuf,
    waiting: FxHashMap<(u32, &'static str), Vec<u8>>,
    bytes: usize,
}

/// The bytes of blobs that wait in memory before they go to the part files.
const WAITING: usize = 64 << 20;

impl Parts {
    fn path(&self, tile: u32, part: &str) -> PathBuf {
        self.dir.join(format!("{tile}.{part}"))
    }
    fn add(&mut self, tile: u32, part: &'static str, mut blobs: Vec<u8>) -> Result<(), String> {
        self.bytes += blobs.len();
        self.waiting.entry((tile, part)).or_default().append(&mut blobs);
        if self.bytes > WAITING {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        for ((tile, part), bytes) in std::mem::take(&mut self.waiting) {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path(tile, part))
                .and_then(|mut f| f.write_all(&bytes))
                .map_err(|e| format!("a part file of the tile cut: {e}"))?;
        }
        self.bytes = 0;
        Ok(())
    }
}

/// The elements of a part file, one at a time, in the order of the file.
struct PartReader {
    r: Option<io::BufReader<std::fs::File>>,
    buf: std::collections::VecDeque<Elem>,
    b: Buffers,
}

impl PartReader {
    fn open(path: &Path) -> PartReader {
        let r = std::fs::File::open(path).ok().map(|f| io::BufReader::with_capacity(1 << 20, f));
        PartReader { r, buf: Default::default(), b: Buffers::default() }
    }
    fn peek(&mut self) -> Result<Option<&Elem>, String> {
        while self.buf.is_empty() {
            let Some(r) = self.r.as_mut() else { return Ok(None) };
            match read_block(r)? {
                None => {
                    self.r = None;
                    return Ok(None);
                }
                Some(block) => {
                    let mut all = All::default();
                    decode_block(&block, [true; 3], &mut self.b, &mut all)?;
                    self.buf.extend(all.0);
                }
            }
        }
        Ok(self.buf.front())
    }
    fn next(&mut self) -> Result<Option<Elem>, String> {
        self.peek()?;
        Ok(self.buf.pop_front())
    }
}

/// The file of a tile from its parts: the nodes of the two node parts, then the ways of the two way parts, each merged in
/// the order of the ids with no doubles, then the relations. Gives the numbers of elements.
fn finish_tile(parts: &Parts, tile: u32, out: &Path, timestamp: Option<i64>) -> Result<CutCounts, String> {
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut w = PbfWriter::new(io::BufWriter::with_capacity(1 << 20, file), timestamp)?;
    let mut n = CutCounts::default();
    for pair in [["nodes", "extra-nodes"], ["ways", "extra-ways"], ["relations", "relations"]] {
        let (mut a, mut b) = (PartReader::open(&parts.path(tile, pair[0])), PartReader::open(&parts.path(tile, pair[1])));
        if pair[0] == pair[1] {
            b = PartReader { r: None, buf: Default::default(), b: Buffers::default() };
        }
        let mut last = None;
        loop {
            let ida = a.peek()?.map(Elem::id);
            let idb = b.peek()?.map(Elem::id);
            let e = match (ida, idb) {
                (None, None) => break,
                (Some(x), Some(y)) if y < x => b.next()?,
                (Some(_), _) => a.next()?,
                (None, Some(_)) => b.next()?,
            }
            .expect("peeked");
            if last == Some(e.id()) {
                continue;
            }
            last = Some(e.id());
            match e {
                Elem::Node(x) => {
                    n.nodes += 1;
                    w.node(x)?
                }
                Elem::Way(x) => {
                    n.ways += 1;
                    w.way(x)?
                }
                Elem::Relation(x) => {
                    n.relations += 1;
                    w.relation(x)?
                }
            }
        }
        for part in pair {
            let _ = std::fs::remove_file(parts.path(tile, part));
        }
    }
    w.finish()?.flush().map_err(|e| e.to_string())?;
    Ok(n)
}

/// Pass 1: the runs of the nodes inside, the nodes inside with their tiles, and whether the block has ways or relations.
struct Nodes<'a> {
    boundary: &'a Boundary,
    z: u8,
    index: &'a FxHashMap<(u32, u32), u32>,
    runs: BlockRuns,
    inside: Vec<(u32, Elem)>,
    other: bool,
    span: Span,
}

impl RawSink for Nodes<'_> {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        widen(&mut self.span, id);
        let tile = if self.boundary.contains(lon, lat) {
            let t = Tile::of(self.z, lon, lat);
            self.index.get(&(t.x, t.y)).copied()
        } else {
            None
        };
        if let Some(t) = tile {
            let tags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
            self.inside.push((t, Elem::Node(RawNode { id, lon, lat, tags })));
        }
        self.runs.push(id, tile);
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

/// Pass 2: the ways with their tiles (the tiles of their nodes inside), the nodes that each tile of a way needs from
/// outside the tile, the runs of the ways, and the relations, whole.
struct Ways<'a> {
    nodes: &'a Book,
    runs: BlockRuns,
    kept: Vec<(u32, Elem)>,
    multi: Vec<(i64, u32)>,
    outside: Vec<(i64, u32)>,
    rels: Vec<Elem>,
    tiles: Vec<u32>,
    spans: [Span; 2],
}

impl RawSink for Ways<'_> {
    fn node<'a>(&mut self, _: i64, _: f64, _: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {}
    fn way<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        widen(&mut self.spans[0], id);
        let refs: Vec<(i64, Option<u32>)> = refs.map(|n| (n, self.nodes.get(n))).collect();
        self.tiles.clear();
        self.tiles.extend(refs.iter().filter_map(|r| r.1));
        self.tiles.sort_unstable();
        self.tiles.dedup();
        match self.tiles.len() {
            0 => {
                self.runs.push(id, None);
                return;
            }
            1 => self.runs.push(id, Some(self.tiles[0])),
            _ => {
                self.runs.push(id, None);
                self.multi.extend(self.tiles.iter().map(|&t| (id, t)));
            }
        }
        let way = RawWay {
            id,
            tags: tags.map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            refs: refs.iter().map(|r| r.0).collect(),
        };
        for &t in &self.tiles {
            self.outside.extend(refs.iter().filter(|r| r.1 != Some(t)).map(|r| (r.0, t)));
        }
        let (last, rest) = self.tiles.split_last().expect("a tile");
        for &t in rest {
            self.kept.push((t, Elem::Way(way.clone())));
        }
        self.kept.push((*last, Elem::Way(way)));
    }
    fn relation<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        widen(&mut self.spans[1], id);
        let tags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let members = members.map(|(m, t, r)| (m, t, r.to_string())).collect();
        self.rels.push(Elem::Relation(Relation { id, tags, members }));
    }
}

/// The ids of `wanted` (sorted) of one kind that a block has.
struct Present<'a> {
    kind: usize,
    wanted: &'a [i64],
    found: Vec<i64>,
}

impl Present<'_> {
    fn see(&mut self, kind: usize, id: i64) {
        if kind == self.kind && self.wanted.binary_search(&id).is_ok() {
            self.found.push(id);
        }
    }
}

impl RawSink for Present<'_> {
    fn node<'a>(&mut self, id: i64, _: f64, _: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        self.see(0, id);
    }
    fn way<'a>(&mut self, id: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = i64>) {
        self.see(1, id);
    }
    fn relation<'a>(
        &mut self,
        _: i64,
        _: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
    }
}

/// The ids of `wanted` (sorted) of one kind (0 nodes, 1 ways) that the file has: only the blocks whose span holds one.
fn present<R: Read + Seek>(r: &mut R, blocks: &[(u64, Span)], kind: usize, wanted: &[i64]) -> Result<Vec<i64>, String> {
    let at: Vec<u64> = blocks
        .iter()
        .filter(|(_, (lo, hi))| {
            let i = wanted.partition_point(|&w| w < *lo);
            i < wanted.len() && wanted[i] <= *hi
        })
        .map(|b| b.0)
        .collect();
    let mut kinds = [false; 3];
    kinds[kind] = true;
    let mut found = Vec::new();
    each_block_at(
        r,
        &at,
        |b, buf| {
            let mut s = Present { kind, wanted, found: Vec::new() };
            decode_block(b, kinds, buf, &mut s)?;
            Ok(s.found)
        },
        |mut f| {
            found.append(&mut f);
            Ok(())
        },
    )?;
    found.sort_unstable();
    Ok(found)
}

/// The elements of a block whose ids are in `wanted` (sorted pairs (id, tile)), each with its tiles.
struct Take<'a> {
    wanted: &'a [(i64, u32)],
    out: Vec<(u32, Elem)>,
    all: All,
}

impl Take<'_> {
    fn take(&mut self) {
        for e in std::mem::take(&mut self.all.0) {
            for t in tiles_in(self.wanted, e.id()) {
                self.out.push((t, e.clone()));
            }
        }
    }
}

impl RawSink for Take<'_> {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        if pairs_in(self.wanted, id, id) {
            self.all.node(id, lat, lon, tags);
            self.take();
        }
    }
    fn way<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        if pairs_in(self.wanted, id, id) {
            self.all.way(id, tags, refs);
            self.take();
        }
    }
    fn relation<'a>(
        &mut self,
        _: i64,
        _: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
    }
}

/// Two numbers, 16 bytes: (id, tile).
struct Pair;
impl Codec<(i64, i64)> for Pair {
    fn put(&self, r: &(i64, i64), out: &mut Vec<u8>) {
        out.extend_from_slice(&r.0.to_le_bytes());
        out.extend_from_slice(&r.1.to_le_bytes());
    }
    fn get(&self, b: &mut &[u8]) -> (i64, i64) {
        (i64::from_le_bytes(b[..8].try_into().expect("8 bytes")), i64::from_le_bytes(b[8..16].try_into().expect("8 bytes")))
    }
}

/// Pass 4: the blocks with an element of `pairs` (a sorted file of (id, tile)), read and decoded a batch at a time on all
/// threads; each such element goes to the part `part` of each of its tiles.
fn take_blocks<R: Read + Seek>(
    r: &mut R,
    blocks: &[(u64, Span)],
    kind: usize,
    pairs: impl Iterator<Item = io::Result<(i64, i64)>>,
    parts: &mut Parts,
    part: &'static str,
) -> Result<(), String> {
    let io = |e: io::Error| format!("a temporary file of the tile cut: {e}");
    let mut pairs = pairs.peekable();
    let mut kinds = [false; 3];
    kinds[kind] = true;
    let n = 2 * rayon::current_num_threads();
    let (mut i, mut pos) = (0, None);
    while i < blocks.len() {
        let mut batch: Vec<(Vec<u8>, Vec<(i64, u32)>)> = Vec::with_capacity(n);
        while i < blocks.len() && batch.len() < n {
            let (at, (lo, hi)) = blocks[i];
            i += 1;
            while pairs.next_if(|x| x.as_ref().is_ok_and(|x| x.0 < lo)).is_some() {}
            let mut want = Vec::new();
            while let Some(x) = pairs.next_if(|x| x.as_ref().is_ok_and(|x| x.0 <= hi)) {
                let (id, t) = x.map_err(io)?;
                want.push((id, t as u32));
            }
            if let Some(Err(_)) = pairs.peek() {
                return Err(io(pairs.next().expect("peeked").expect_err("an error")));
            }
            want.dedup();
            if want.is_empty() {
                continue;
            }
            if pos != Some(at) {
                r.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
            }
            let b = read_block(r)?.ok_or("a block after the end of the file")?;
            pos = Some(at + b.len() as u64);
            batch.push((b, want));
        }
        let out: Vec<Vec<(u32, Vec<u8>)>> = batch
            .par_iter()
            .map_init(Buffers::default, |buf, (b, want)| {
                let mut s = Take { wanted: want, out: Vec::new(), all: All::default() };
                decode_block(b, kinds, buf, &mut s)?;
                by_tile(s.out)
            })
            .collect::<Result<_, _>>()?;
        for (t, bytes) in out.into_iter().flatten() {
            parts.add(t, part, bytes)?;
        }
    }
    Ok(())
}

/// Cut the region file that `open` reads (a new reader for each pass) to the tiles of `tiles` (one zoom) inside `boundary`:
/// the file of a tile goes into `dir` as `z_x_y.osm.pbf`; a tile with no node inside has no file. Gives the numbers of
/// elements of each tile.
pub fn cut_to_tiles<R: Read + Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    tiles: &[Tile],
    dir: &Path,
) -> Result<Vec<CutCounts>, String> {
    let io = |e: io::Error| format!("a temporary file of the tile cut: {e}");
    let (timestamp, sorted) = header(open()?)?;
    if !sorted {
        return Err("the region file is not sorted by type and id (Sort.Type_then_ID)".into());
    }
    let Some(z) = tiles.first().map(|t| t.z) else {
        return Ok(Vec::new());
    };
    if tiles.iter().any(|t| t.z != z) {
        return Err("the tile cut needs tiles of one zoom".into()); // ponytail: one zoom; mixed zooms need a quadtree look-up
    }
    let index: FxHashMap<(u32, u32), u32> = tiles.iter().enumerate().map(|(i, t)| ((t.x, t.y), i as u32)).collect();
    let store = crate::spill::temp_store();
    let store = &*store;
    let tmp = std::env::temp_dir().join(format!("duckosm-tiles-{}-{}", std::process::id(), dir.as_os_str().len()));
    std::fs::create_dir_all(&tmp).map_err(io)?;
    let mut parts = Parts { dir: tmp.clone(), waiting: FxHashMap::default(), bytes: 0 };
    // 1. each node inside to the node part of its tile; the book of the nodes
    let (mut nodes, mut node_blocks, mut ways_at) = (Book::default(), Vec::new(), None);
    each_block(
        &mut open()?,
        |b, buf| {
            let mut s = Nodes { boundary, z, index: &index, runs: BlockRuns::new(), inside: Vec::new(), other: false, span: EMPTY };
            decode_block(b, [true, true, true], buf, &mut s)?;
            Ok((s.runs, by_tile(s.inside)?, s.other, s.span))
        },
        |(runs, blobs, other, span), at| {
            if span != EMPTY {
                node_blocks.push((at, span));
            }
            nodes.append(runs.runs, runs.first.unwrap_or(false));
            for (t, bytes) in blobs {
                parts.add(t, "nodes", bytes)?;
            }
            if other {
                ways_at = Some(at);
            }
            Ok(!other)
        },
    )?;
    nodes.finish();
    crate::util::step(&format!("tile cut 1: the nodes inside ({} runs, {} MB)", nodes.len(), nodes.size() >> 20));
    // 2. each way with a node inside to the way part of its tiles; the nodes that a tile needs from outside it (to a
    // temporary list); the relations, whole (to a temporary file)
    let ways_at = ways_at.unwrap_or(0);
    let (mut ways, mut multi) = (Book::default(), Vec::new());
    let mut outside = Writer::new(store).map_err(io)?;
    let rels_path = tmp.join("relations.all");
    let (mut way_blocks, mut rel_blocks) = (Vec::new(), Vec::new());
    let mut rels_bytes = Vec::new();
    let mut r = open()?;
    r.seek(SeekFrom::Start(ways_at)).map_err(|e| e.to_string())?;
    each_block(
        &mut r,
        |b, buf| {
            let mut s = Ways {
                nodes: &nodes,
                runs: BlockRuns::new(),
                kept: Vec::new(),
                multi: Vec::new(),
                outside: Vec::new(),
                rels: Vec::new(),
                tiles: Vec::new(),
                spans: [EMPTY; 2],
            };
            decode_block(b, [false, true, true], buf, &mut s)?;
            let mut rels = Vec::new();
            blobs(s.rels, &mut rels)?;
            Ok((s.runs, by_tile(s.kept)?, s.multi, s.outside, rels, s.spans))
        },
        |(runs, kept, mut m, o, rels, spans), at| {
            if spans[0] != EMPTY {
                way_blocks.push((ways_at + at, spans[0]));
                ways.append(runs.runs, runs.first.unwrap_or(false));
            }
            if spans[1] != EMPTY {
                rel_blocks.push((ways_at + at, spans[1]));
            }
            for (t, bytes) in kept {
                parts.add(t, "ways", bytes)?;
            }
            multi.append(&mut m);
            for (n, t) in o {
                outside.push(&Pair, &(n, t as i64)).map_err(io)?;
            }
            rels_bytes.extend_from_slice(&rels);
            if rels_bytes.len() > WAITING {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&rels_path)
                    .and_then(|mut f| f.write_all(&rels_bytes))
                    .map_err(io)?;
                rels_bytes.clear();
            }
            Ok(true)
        },
    )?;
    std::fs::OpenOptions::new().create(true).append(true).open(&rels_path).and_then(|mut f| f.write_all(&rels_bytes)).map_err(io)?;
    drop(rels_bytes);
    ways.finish();
    tidy(&mut multi);
    crate::util::step(&format!(
        "tile cut 2: the ways ({} runs, {} MB; {} pairs of ways in more tiles)",
        ways.len(),
        ways.size() >> 20,
        multi.len()
    ));
    // 3. the relations with a member node inside a tile or a member way kept in a tile (only the members that the file
    // has: a run can hold the id of an element that the file does not have), their parents, and the members of the
    // multipolygons
    let each_rel = |f: &mut dyn FnMut(Relation) -> Result<(), String>| -> Result<(), String> {
        let mut rd = PartReader::open(&rels_path);
        while let Some(e) = rd.next()? {
            if let Elem::Relation(r) = e {
                f(r)?;
            }
        }
        Ok(())
    };
    let (mut member_nodes, mut member_ways) = (Vec::new(), Vec::new());
    each_rel(&mut |r| {
        for &(m, t, _) in &r.members {
            match t {
                MemberType::Node => member_nodes.push(m),
                MemberType::Way => member_ways.push(m),
                MemberType::Relation => {}
            }
        }
        Ok(())
    })?;
    for v in [&mut member_nodes, &mut member_ways] {
        v.sort_unstable();
        v.dedup();
    }
    crate::util::step(&format!("tile cut 3a: the members of the relations ({} nodes, {} ways)", member_nodes.len(), member_ways.len()));
    let node_members = present(&mut open()?, &node_blocks, 0, &member_nodes)?;
    drop(member_nodes);
    let way_members = present(&mut open()?, &way_blocks, 1, &member_ways)?;
    drop(member_ways);
    let way_tiles = |id: i64, out: &mut Vec<u32>| match ways.get(id) {
        Some(t) => out.push(t),
        None => out.extend(tiles_in(&multi, id)),
    };
    let (mut direct, mut parents, mut multipolygons) = (Vec::new(), Vec::new(), Vec::new());
    let mut buf = Vec::new();
    each_rel(&mut |r| {
        if r.tags.iter().any(|(k, v)| k == "type" && v == "multipolygon") {
            multipolygons.push(r.id);
        }
        // the tiles of this relation, with no doubles (a route has thousands of members in a few tiles)
        buf.clear();
        for &(m, t, _) in &r.members {
            match t {
                MemberType::Node if node_members.binary_search(&m).is_ok() => buf.extend(nodes.get(m)),
                MemberType::Way if way_members.binary_search(&m).is_ok() => way_tiles(m, &mut buf),
                MemberType::Relation => parents.push((m, r.id)),
                _ => {}
            }
        }
        buf.sort_unstable();
        buf.dedup();
        direct.extend(buf.iter().map(|&t| (r.id, t)));
        Ok(())
    })?;
    crate::util::step(&format!("tile cut 3b: the members in the file ({} nodes, {} ways)", node_members.len(), way_members.len()));
    drop((node_members, way_members));
    tidy(&mut direct);
    parents.sort_unstable();
    let mut chosen: BTreeSet<(i64, u32)> = direct.iter().copied().collect();
    let mut queue = direct.clone();
    while let Some((rel, t)) = queue.pop() {
        let at = parents.partition_point(|p| p.0 < rel);
        for &(_, parent) in parents[at..].iter().take_while(|p| p.0 == rel) {
            if chosen.insert((parent, t)) {
                queue.push((parent, t));
            }
        }
    }
    drop(parents);
    let chosen: Vec<(i64, u32)> = chosen.into_iter().collect();
    crate::util::step(&format!("tile cut 3c: the tiles of the relations ({} pairs)", chosen.len()));
    // each chosen relation to its tiles; the members of the multipolygons with a member in a tile
    let mut more: Vec<(i64, u32)> = Vec::new();
    let mut waiting: Vec<(u32, Elem)> = Vec::new();
    let mut waiting_bytes = 0usize;
    each_rel(&mut |r| {
        if multipolygons.binary_search(&r.id).is_ok() {
            for t in tiles_in(&direct, r.id) {
                for &(m, ty, _) in &r.members {
                    match ty {
                        MemberType::Node => outside.push(&Pair, &(m, t as i64)).map_err(io)?,
                        MemberType::Way => more.push((m, t)),
                        MemberType::Relation => {}
                    }
                }
            }
        }
        let id = r.id;
        let size = 64 + 40 * r.members.len() + 48 * r.tags.len();
        let e = Elem::Relation(r);
        for t in tiles_in(&chosen, id) {
            waiting.push((t, e.clone()));
            waiting_bytes += size;
        }
        // the waiting relations by their size: a few have tens of thousands of members (boundaries, long routes)
        if waiting_bytes >= WAITING {
            waiting_bytes = 0;
            for (t, bytes) in by_tile(std::mem::take(&mut waiting))? {
                parts.add(t, "relations", bytes)?;
            }
        }
        Ok(())
    })?;
    for (t, bytes) in by_tile(waiting)? {
        parts.add(t, "relations", bytes)?;
    }
    let _ = std::fs::remove_file(&rels_path);
    drop((direct, chosen, multipolygons));
    tidy(&mut more);
    if !more.is_empty() {
        // the nodes of these ways: only the blocks with one of them
        let mut wanted: Vec<i64> = more.iter().map(|m| m.0).collect();
        wanted.dedup();
        let at: Vec<u64> = way_blocks
            .iter()
            .filter(|(_, (lo, hi))| {
                let i = wanted.partition_point(|&w| w < *lo);
                i < wanted.len() && wanted[i] <= *hi
            })
            .map(|b| b.0)
            .collect();
        let mut err = None;
        each_block_at(
            &mut open()?,
            &at,
            |b, buf| {
                let mut s = Wanted { wanted: &wanted, found: Vec::new() };
                decode_block(b, [false, true, false], buf, &mut s)?;
                Ok(s.found)
            },
            |found| {
                for (w, refs) in found {
                    for t in tiles_in(&more, w) {
                        for &n in &refs {
                            if let Err(e) = outside.push(&Pair, &(n, t as i64)) {
                                err.get_or_insert(e);
                            }
                        }
                    }
                }
                Ok(())
            },
        )?;
        err.map_or(Ok(()), Err).map_err(io)?;
    }
    // the indexes go: the next parts read lists of ids and tiles
    drop((nodes, ways, multi));
    crate::util::step("tile cut 3: the relations and the members of the multipolygons");
    // 4. the nodes from outside the tiles and the member ways of the multipolygons, to the extra parts of their tiles
    let outside = sort_bytes(outside.finish().map_err(io)?, |b| Pair.get(&mut &b[..]), 64 << 20, store).map_err(io)?;
    crate::util::step(&format!("tile cut 4a: the nodes from outside, sorted ({} rows)", outside.len));
    let mut r = open()?;
    take_blocks(&mut r, &node_blocks, 0, outside.read(&Pair).map_err(io)?, &mut parts, "extra-nodes")?;
    take_blocks(&mut r, &way_blocks, 1, more.iter().map(|&(w, t)| Ok((w, t as i64))), &mut parts, "extra-ways")?;
    parts.flush()?;
    drop(more);
    crate::util::step("tile cut 4: the extra elements");
    // 5. each tile from its parts (the tiles at the same time)
    let counts: Vec<CutCounts> = (0..tiles.len() as u32)
        .into_par_iter()
        .map(|t| {
            if !parts.path(t, "nodes").exists() {
                for part in ["ways", "extra-nodes", "extra-ways", "relations"] {
                    let _ = std::fs::remove_file(parts.path(t, part));
                }
                return Ok(CutCounts::default());
            }
            let tile = &tiles[t as usize];
            finish_tile(&parts, t, &dir.join(format!("{}_{}_{}.osm.pbf", tile.z, tile.x, tile.y)), timestamp)
        })
        .collect::<Result<_, String>>()?;
    let _ = std::fs::remove_dir_all(&tmp);
    crate::util::step("tile cut 5: the tile files");
    Ok(counts)
}
