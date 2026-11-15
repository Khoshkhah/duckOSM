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
use crate::network::{global_junction, JunctionUses};
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

    /// The cut into the store (the browser's way: the tile files made in memory, a few tiles at a time) gives the same
    /// tile files as the cut into a folder.
    #[test]
    fn the_cut_into_the_store_equals_the_cut_into_a_folder() {
        let data = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
        let pbf = data.join("monaco.osm.pbf");
        let b = Boundary::parse(&std::fs::read_to_string(data.join("monaco.geojson")).unwrap()).unwrap();
        let tiles = crate::tiles::tiles(&b, 15);
        let open = || std::fs::File::open(&pbf).map_err(|e| e.to_string());
        let dir = std::env::temp_dir().join(format!("duckosm-cut-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        cut_to_tiles(open, &b, &tiles, &dir, true, 25.0).unwrap();
        let store = cut_to_store_build(open, &b, &tiles, true, 25.0).unwrap();
        let mut n = 0;
        for (i, (t, _)) in store.tiles.iter().enumerate() {
            let mut got = Vec::new();
            store.reader(i).read_to_end(&mut got).unwrap();
            let want = std::fs::read(dir.join(format!("{}_{}_{}.osm.pbf", t.z, t.x, t.y))).unwrap_or_default();
            assert!(got == want, "tile {t:?}: {} bytes in the store, {} in the folder", got.len(), want.len());
            n += usize::from(!got.is_empty());
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(n > 4, "the cut gave {n} tile files");
    }

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

/// The blobs of elements of one kind (sorted by id), BLOCK elements in each, as in a `.osm.pbf` file, each after a record
/// (first id, last id, number of elements: 20 bytes). The elements move into the blocks (no copy).
fn blobs(elems: Vec<Elem>, out: &mut Vec<u8>) -> Result<(), String> {
    let mut it = elems.into_iter().peekable();
    while it.peek().is_some() {
        let chunk: Vec<Elem> = it.by_ref().take(BLOCK).collect();
        // the record of the blob: its first and last id and its number of elements, so that a merge can copy it unread
        out.extend_from_slice(&chunk[0].id().to_le_bytes());
        out.extend_from_slice(&chunk[chunk.len() - 1].id().to_le_bytes());
        out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
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
fn by_tile(elems: Vec<(u32, Elem)>) -> Result<Vec<(u32, Vec<u8>)>, String> {
    groups(elems).into_iter().map(tile_blobs).collect()
}

/// [`by_tile`] with the tiles on all threads (the compression of the blocks is the work), for a caller on one thread: the
/// passes that decode the blocks on all threads call [`by_tile`].
fn by_tile_par(elems: Vec<(u32, Elem)>) -> Result<Vec<(u32, Vec<u8>)>, String> {
    groups(elems).into_par_iter().map(tile_blobs).collect()
}

/// The elements of each tile, in the order of the tiles, each tile in the order of the ids.
fn groups(mut elems: Vec<(u32, Elem)>) -> Vec<(u32, Vec<Elem>)> {
    elems.sort_by_key(|e| e.0); // stable: the order of the ids in each tile
    let mut groups: Vec<(u32, Vec<Elem>)> = Vec::new();
    for (t, e) in elems {
        match groups.last_mut() {
            Some(g) if g.0 == t => g.1.push(e),
            _ => groups.push((t, vec![e])),
        }
    }
    groups
}

fn tile_blobs((t, group): (u32, Vec<Elem>)) -> Result<(u32, Vec<u8>), String> {
    let mut bytes = Vec::new();
    blobs(group, &mut bytes)?;
    Ok((t, bytes))
}

/// The part files of the tiles. The blobs of a part wait in memory until a limit, then go to the end of their files.
struct Parts {
    files: Files,
    /// the blobs that wait, and their bytes (a lock: a pass reads a part while it adds to others)
    waiting: std::sync::Mutex<(FxHashMap<(u32, &'static str), Vec<u8>>, usize)>,
}

/// Where the part files are: files in a temporary folder (the build by tiles: a file is open only while it is written,
/// because the tiles are more than a process can keep open), or chunks of one temporary file of the store
/// ([`cut_to_store`]: the browser has no folders, only the store in OPFS), or in memory (the browser: the parts of a few
/// tiles, read on the thread that owns the OPFS file, so that the other threads can make their tile files).
enum Files {
    Dir(PathBuf),
    Store(Chunks),
    Mem(std::sync::Mutex<FxHashMap<(u32, &'static str), std::sync::Arc<[u8]>>>),
}

/// The bytes of blobs that wait in memory before they go to the part files.
const WAITING: usize = 64 << 20;

/// The tile of the part with all relations (pass 2).
const ALL: u32 = u32::MAX;

impl Parts {
    fn path(dir: &Path, tile: u32, part: &str) -> PathBuf {
        dir.join(format!("{tile}.{part}"))
    }
    fn add(&self, tile: u32, part: &'static str, mut blobs: Vec<u8>) -> Result<(), String> {
        let full = {
            let mut w = self.waiting.lock().expect("the parts");
            w.1 += blobs.len();
            w.0.entry((tile, part)).or_default().append(&mut blobs);
            w.1 > WAITING
        };
        if full {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&self) -> Result<(), String> {
        let e = |e: io::Error| format!("a part file of the tile cut: {e}");
        let waiting = std::mem::take(&mut *self.waiting.lock().expect("the parts")).0;
        for ((tile, part), bytes) in waiting {
            match &self.files {
                Files::Dir(dir) => std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(Self::path(dir, tile, part))
                    .and_then(|mut f| f.write_all(&bytes))
                    .map_err(e)?,
                Files::Store(c) => c.append((tile, part), &bytes).map_err(e)?,
                Files::Mem(m) => {
                    let mut m = m.lock().expect("the parts");
                    let old = m.remove(&(tile, part)).unwrap_or_else(|| std::sync::Arc::from(&[][..]));
                    m.insert((tile, part), [&old[..], &bytes[..]].concat().into());
                }
            }
        }
        Ok(())
    }
    /// The bytes of a part (after [`Parts::flush`]), or None when it has none.
    fn reader(&self, tile: u32, part: &'static str) -> Option<Box<dyn Read + Send + '_>> {
        match &self.files {
            Files::Dir(dir) => std::fs::File::open(Self::path(dir, tile, part)).ok().map(|f| Box::new(f) as Box<dyn Read + Send>),
            Files::Store(c) => c.reader((tile, part)).map(|r| Box::new(r) as Box<dyn Read + Send>),
            Files::Mem(m) => m.lock().expect("the parts").get(&(tile, part)).map(|b| Box::new(io::Cursor::new(b.clone())) as Box<dyn Read + Send>),
        }
    }
    fn has(&self, tile: u32, part: &'static str) -> bool {
        match &self.files {
            Files::Dir(dir) => Self::path(dir, tile, part).exists(),
            Files::Store(c) => c.index.lock().expect("the index").contains_key(&(tile, part)),
            Files::Mem(m) => m.lock().expect("the parts").contains_key(&(tile, part)),
        }
    }
    fn remove(&self, tile: u32, part: &'static str) {
        match &self.files {
            Files::Dir(dir) => {
                let _ = std::fs::remove_file(Self::path(dir, tile, part));
            }
            Files::Store(c) => {
                c.index.lock().expect("the index").remove(&(tile, part));
            }
            Files::Mem(m) => {
                m.lock().expect("the parts").remove(&(tile, part));
            }
        }
    }
}

/// Files as chunks of one temporary file of the store: each file is the list of its chunks (place, length). A removed
/// file keeps its bytes in the temporary file until the end.
pub struct Chunks {
    file: std::sync::Mutex<(Box<dyn crate::spill::Blob>, u64)>,
    index: std::sync::Mutex<FxHashMap<(u32, &'static str), Vec<(u64, u64)>>>,
}

impl Chunks {
    fn new(store: &dyn crate::spill::Store) -> io::Result<Chunks> {
        Ok(Chunks { file: std::sync::Mutex::new((store.create()?, 0)), index: Default::default() })
    }
    fn append(&self, key: (u32, &'static str), bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut f = self.file.lock().expect("the file");
        let at = f.1;
        f.0.seek(SeekFrom::Start(at))?;
        f.0.write_all(bytes)?;
        f.1 += bytes.len() as u64;
        self.index.lock().expect("the index").entry(key).or_default().push((at, bytes.len() as u64));
        Ok(())
    }
    /// All the bytes of a file: one read for each chunk.
    fn read_all(&self, key: (u32, &'static str)) -> io::Result<Option<Vec<u8>>> {
        let Some(chunks) = self.index.lock().expect("the index").get(&key).cloned() else { return Ok(None) };
        let mut b = vec![0u8; chunks.iter().map(|c| c.1 as usize).sum()];
        let (mut f, mut at) = (self.file.lock().expect("the file"), 0);
        for (pos, len) in chunks {
            f.0.seek(SeekFrom::Start(pos))?;
            f.0.read_exact(&mut b[at..at + len as usize])?;
            at += len as usize;
        }
        Ok(Some(b))
    }
    fn reader(&self, key: (u32, &'static str)) -> Option<ChunkReader<'_>> {
        let chunks = self.index.lock().expect("the index").get(&key)?.clone();
        Some(ChunkReader { file: &self.file, chunks, i: 0, off: 0 })
    }
}

/// The bytes of a file of [`Chunks`], in order.
pub struct ChunkReader<'a> {
    file: &'a std::sync::Mutex<(Box<dyn crate::spill::Blob>, u64)>,
    chunks: Vec<(u64, u64)>,
    i: usize,
    off: u64,
}

impl Read for ChunkReader<'_> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        while self.chunks.get(self.i).is_some_and(|c| self.off == c.1) {
            self.i += 1;
            self.off = 0;
        }
        let Some(&(at, len)) = self.chunks.get(self.i) else { return Ok(0) };
        let n = (len - self.off).min(b.len() as u64) as usize;
        let mut f = self.file.lock().expect("the file");
        f.0.seek(SeekFrom::Start(at + self.off))?;
        f.0.read_exact(&mut b[..n])?;
        self.off += n as u64;
        Ok(n)
    }
}

/// A writer of a file of [`Chunks`]: each write is a chunk (put a `BufWriter` in front).
struct ChunkWriter<'a>(&'a Chunks, (u32, &'static str));

impl Write for ChunkWriter<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.append(self.1, b)?;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The blobs of a part file, with their records: (first id, last id, number of elements, the blob as in a `.osm.pbf`).
struct Blobs<'a>(Option<io::BufReader<Box<dyn Read + Send + 'a>>>);

type Blob = (i64, i64, u32, Vec<u8>);

impl<'a> Blobs<'a> {
    fn open(parts: &'a Parts, tile: u32, part: &'static str) -> Blobs<'a> {
        Blobs(parts.reader(tile, part).map(|f| io::BufReader::with_capacity(1 << 20, f)))
    }
    fn next(&mut self) -> Result<Option<Blob>, String> {
        let Some(r) = self.0.as_mut() else { return Ok(None) };
        let mut head = [0u8; 20];
        match r.read_exact(&mut head) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.to_string()),
        }
        let i = |at: usize| i64::from_le_bytes(head[at..at + 8].try_into().expect("8 bytes"));
        let n = u32::from_le_bytes(head[16..20].try_into().expect("4 bytes"));
        let blob = read_block(r)?.ok_or("a part file ends in a record")?;
        Ok(Some((i(0), i(8), n, blob)))
    }
}

/// The elements of a part file, one at a time, in the order of the file. The blocks are decoded on all threads, a few
/// blocks for each thread at a time (the relations of France: three passes of about 2 s each on one thread before).
struct PartReader<'a> {
    blobs: Blobs<'a>,
    buf: std::collections::VecDeque<Elem>,
}

impl<'a> PartReader<'a> {
    fn open(parts: &'a Parts, tile: u32, part: &'static str) -> PartReader<'a> {
        PartReader { blobs: Blobs::open(parts, tile, part), buf: Default::default() }
    }
    fn peek(&mut self) -> Result<Option<&Elem>, String> {
        if self.buf.is_empty() {
            let mut blobs = Vec::new();
            while blobs.len() < 4 * rayon::current_num_threads() {
                match self.blobs.next()? {
                    None => break,
                    Some((_, _, _, blob)) => blobs.push(blob),
                }
            }
            let decoded: Vec<Vec<Elem>> = blobs
                .par_iter()
                .map_init(Buffers::default, |b, blob| {
                    let mut all = All::default();
                    decode_block(blob, [true; 3], b, &mut all)?;
                    Ok(all.0)
                })
                .collect::<Result<_, String>>()?;
            self.buf.extend(decoded.into_iter().flatten());
        }
        Ok(self.buf.front())
    }
    fn peek_id(&mut self) -> Result<Option<i64>, String> {
        Ok(self.peek()?.map(Elem::id))
    }
    fn next(&mut self) -> Result<Option<Elem>, String> {
        self.peek()?;
        Ok(self.buf.pop_front())
    }
    /// The next elements as one batch (the blocks of one decode), in the order of the file.
    fn next_batch(&mut self) -> Result<Option<Vec<Elem>>, String> {
        self.peek()?;
        Ok((!self.buf.is_empty()).then(|| self.buf.drain(..).collect()))
    }
}

/// An element to the file of a tile, but one with the id of the element before (a node of the tile that a part of the
/// extra elements also has).
fn put<W: Write>(w: &mut PbfWriter<W>, n: &mut CutCounts, last: &mut Option<i64>, e: Elem) -> Result<(), String> {
    if *last == Some(e.id()) {
        return Ok(());
    }
    *last = Some(e.id());
    match e {
        Elem::Node(x) => {
            n.nodes += 1;
            w.node(x)
        }
        Elem::Way(x) => {
            n.ways += 1;
            w.way(x)
        }
        Elem::Relation(x) => {
            n.relations += 1;
            w.relation(x)
        }
    }
}

/// One kind of the file of a tile: the blobs of the main part, merged in the order of the ids with the elements of the
/// extra part. A blob that no extra element falls into is copied as bytes; only the others are decoded and made again.
fn merge_kind<W: Write>(w: &mut PbfWriter<W>, n: &mut CutCounts, parts: &Parts, tile: u32, main: &'static str, extra: &'static str, kind: u8) -> Result<(), String> {
    let (mut a, mut x, mut b) = (Blobs::open(parts, tile, main), PartReader::open(parts, tile, extra), Buffers::default());
    let mut last = None;
    while let Some((first, end, count, blob)) = a.next()? {
        while x.peek_id()?.is_some_and(|id| id < first) {
            let e = x.next()?.expect("peeked");
            put(w, n, &mut last, e)?;
        }
        if x.peek_id()?.is_none_or(|id| id > end) {
            w.blocks(&blob)?;
            match kind {
                0 => n.nodes += count as usize,
                1 => n.ways += count as usize,
                _ => n.relations += count as usize,
            }
            last = Some(end);
            continue;
        }
        let mut all = All::default();
        decode_block(&blob, [true; 3], &mut b, &mut all)?;
        for e in all.0 {
            while x.peek_id()?.is_some_and(|id| id < e.id()) {
                let e = x.next()?.expect("peeked");
                put(w, n, &mut last, e)?;
            }
            put(w, n, &mut last, e)?;
        }
    }
    while let Some(e) = x.next()? {
        put(w, n, &mut last, e)?;
    }
    Ok(())
}

/// The file of a tile from its parts, written to `out`: the nodes, the ways, the relations, each kind merged in the order
/// of the ids with no doubles. Gives the numbers of elements.
/// Step 5 of the cut in the browser: this thread reads the parts of the next tiles (about [`MEMORY_TILES`] bytes) from
/// the store in OPFS (its file belongs to this thread), all threads make their tile files in memory, and this thread
/// writes them to the store as the part "tile". Kentucky (16 threads): 8.0 s one tile at a time before.
fn finish_tiles_in_memory(parts: &Parts, n: usize, timestamp: Option<i64>) -> Result<Vec<CutCounts>, String> {
    let Files::Store(chunks) = &parts.files else { return Err("the tile cut in memory needs the store".into()) };
    let e = |e: io::Error| format!("a part file of the tile cut: {e}");
    let mut counts = vec![CutCounts::default(); n];
    let mut next = 0;
    while next < n {
        let mem = Parts { files: Files::Mem(Default::default()), waiting: Default::default() };
        let (mut batch, mut bytes) = (Vec::new(), 0usize);
        while next < n && (batch.is_empty() || bytes < MEMORY_TILES) {
            let t = next as u32;
            next += 1;
            let (has, at) = (parts.has(t, "nodes"), bytes);
            for part in ["nodes", "ways", "extra-nodes", "extra-ways", "relations"] {
                if let (true, Some(b)) = (has, chunks.read_all((t, part)).map_err(e)?) {
                    bytes += b.len();
                    let Files::Mem(m) = &mem.files else { unreachable!() };
                    m.lock().expect("the parts").insert((t, part), b.into());
                }
                parts.remove(t, part);
            }
            if has {
                batch.push((bytes - at, t));
            } else {
                parts.remove(t, "junctions");
            }
        }
        // the largest tiles first, each tile its own task: a large tile does not end the batch alone
        batch.sort_unstable_by(|a, b| b.cmp(a));
        let made: Vec<(u32, Vec<u8>, CutCounts)> = batch
            .par_iter()
            .with_max_len(1)
            .map(|&(_, t)| {
                let mut out = Vec::new();
                let c = finish_tile(&mem, t, &mut out, timestamp)?;
                Ok((t, out, c))
            })
            .collect::<Result<_, String>>()?;
        for (t, out, c) in made {
            chunks.append((t, "tile"), &out).map_err(e)?;
            counts[t as usize] = c;
        }
    }
    Ok(counts)
}

/// The bytes of the parts of the tiles that [`finish_tiles_in_memory`] reads at once.
const MEMORY_TILES: usize = 128 << 20;

fn finish_tile<W: Write>(parts: &Parts, tile: u32, out: W, timestamp: Option<i64>) -> Result<CutCounts, String> {
    let mut w = PbfWriter::new(io::BufWriter::with_capacity(1 << 20, out), timestamp)?;
    let mut n = CutCounts::default();
    for (kind, main, extra) in [(0, "nodes", "extra-nodes"), (1, "ways", "extra-ways"), (2, "relations", "none")] {
        merge_kind(&mut w, &mut n, parts, tile, main, extra, kind)?;
        parts.remove(tile, main);
        parts.remove(tile, extra);
    }
    w.finish()?.flush().map_err(|e| e.to_string())?;
    Ok(n)
}

/// Pass 1: the runs of the nodes inside, the nodes inside with their tiles, and whether the block has ways or relations.
struct Nodes<'a> {
    boundary: &'a Boundary,
    z: u8,
    index: &'a TileIndex,
    /// the border of a tile, in metres: a node inside the area this near another tile goes to `near` with that tile
    border_m: f64,
    runs: BlockRuns,
    inside: Vec<(u32, Elem)>,
    near: Vec<(i64, u32)>,
    other: bool,
    span: Span,
}

/// The tiles of a cut, of one zoom or of mixed zooms (no tile in another): the tile of a point is the tile of the finest
/// zoom that holds it, found from the finest zoom up.
struct TileIndex {
    /// the zooms of the tiles, the finest first
    zooms: Vec<u8>,
    map: FxHashMap<(u8, u32, u32), u32>,
}

impl TileIndex {
    fn new(tiles: &[Tile]) -> TileIndex {
        let mut zooms: Vec<u8> = tiles.iter().map(|t| t.z).collect();
        zooms.sort_unstable_by(|a, b| b.cmp(a));
        zooms.dedup();
        TileIndex { zooms, map: tiles.iter().enumerate().map(|(i, t)| ((t.z, t.x, t.y), i as u32)).collect() }
    }

    /// The tile that holds the point (`Tile::of` of its zoom: `Tile::of` of a finer zoom, shifted, is the same tile).
    fn of(&self, lon: f64, lat: f64) -> Option<u32> {
        let fine = Tile::of(*self.zooms.first()?, lon, lat);
        self.zooms.iter().find_map(|&z| self.map.get(&(z, fine.x >> (fine.z - z), fine.y >> (fine.z - z))).copied())
    }
}

pub use crate::tiles::near_tile;

/// The border of the tiles of a build by tiles, in metres: the connector of the dangling paths (`snap_m`) joins a dangling
/// end to the nearest node within `snap_m`, so a tile with all the ways within twice `snap_m` of it (and 5 m more) makes the
/// same connectors at its nodes as the whole build.
pub fn border_m(snap_m: f64) -> f64 {
    2.0 * snap_m + 5.0
}

impl RawSink for Nodes<'_> {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        widen(&mut self.span, id);
        let tile = if self.boundary.contains(lon, lat) {
            self.index.of(lon, lat)
        } else {
            None
        };
        if let Some(t) = tile {
            let tags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
            self.inside.push((t, Elem::Node(RawNode { id, lon, lat, tags })));
            // the other tiles whose border holds the node
            if self.border_m > 0.0 {
                let me = Tile::of(self.z, lon, lat);
                for (dx, dy) in [(-1i64, -1i64), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let (x, y) = (me.x as i64 + dx, me.y as i64 + dy);
                    if x < 0 || y < 0 {
                        continue;
                    }
                    let other = Tile { z: self.z, x: x as u32, y: y as u32 };
                    if let Some(&o) = self.index.map.get(&(other.z, other.x, other.y)) {
                        if near_tile(other, lon, lat, self.border_m) {
                            self.near.push((id, o));
                        }
                    }
                }
            }
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
    /// the nodes in the border of other tiles (sorted (node, tile)): a way with such a node goes to those tiles too
    near: &'a [(i64, u32)],
    runs: BlockRuns,
    kept: Vec<(u32, Elem)>,
    multi: Vec<(i64, u32)>,
    outside: Vec<(i64, u32)>,
    /// the uses of the nodes by the kept ways that are built highways (node, flags of `JunctionUses::of_way`)
    uses: Vec<(i64, u8)>,
    /// the nodes outside the area of the kept ways: (node, way, the first tile of the way), for their home tile
    homes: Vec<(i64, i64, u32)>,
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
        // the home tile of a node outside the area comes from the tiles of the nodes of the ways, not from the borders
        let first = self.tiles.first().copied();
        if !self.near.is_empty() && first.is_some() {
            self.tiles.extend(refs.iter().flat_map(|r| tiles_in(self.near, r.0)));
            self.tiles.sort_unstable();
            self.tiles.dedup();
        }
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
        let tags = crate::model::Tags::from_pairs(way.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        JunctionUses::of_way(&tags, &way.refs, &mut self.uses);
        let first = first.expect("a tile");
        self.homes.extend(refs.iter().filter(|r| r.1.is_none()).map(|r| (r.0, id, first)));
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

/// A use of a node by a way, 8 bytes: the node id with the 2 bits of its flags (`JunctionUses::of_way`) below it, so the
/// rows sort by the node.
struct Use;
impl Codec<(i64, u8)> for Use {
    fn put(&self, r: &(i64, u8), out: &mut Vec<u8>) {
        out.extend_from_slice(&((r.0 << 2) | (r.1 & 3) as i64).to_le_bytes());
    }
    fn get(&self, b: &mut &[u8]) -> (i64, u8) {
        let v = i64::from_le_bytes(b[..8].try_into().expect("8 bytes"));
        *b = &b[8..];
        (v >> 2, (v & 3) as u8)
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
    parts: &Parts,
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

/// The start of a junctions file: the name and the version of the format. Then one row for each node outside the tile that
/// is a junction in the whole area or that the tile holds: the node id (i64, little endian) and its flags ([`JUNCTION`],
/// [`ROAD`], [`HOME`]), in the order of the node ids.
pub const JUNCTIONS_MAGIC: &[u8; 8] = b"DOSMJN02";
/// The node is a junction in the whole area (`main.global_junctions`).
pub const JUNCTION: u8 = 1;
/// The junction is a road junction (`is_road_junction`).
pub const ROAD: u8 = 2;
/// The node is outside the area, and this tile is its home tile: the tile has all ways of the node, and holds it.
pub const HOME: u8 = 4;

/// The junctions file of each tile: for each (node, tile) of `outside` (sorted), the junction of the node in the whole area
/// (from `uses`, the uses of the nodes by all kept ways, sorted by node), and whether the tile is the home tile of the node
/// (`home`, sorted (node, tile)), to the part "junctions" of the tile. A tile has only some of the ways of a node outside
/// it, so it can miss a junction there.
fn outside_junctions(
    outside: impl Iterator<Item = io::Result<(i64, i64)>>,
    uses: impl Iterator<Item = io::Result<(i64, i64)>>,
    home: &[(i64, u32)],
    parts: &Parts,
) -> Result<(), String> {
    let io = |e: io::Error| format!("a temporary file of the tile cut: {e}");
    let mut uses = uses.peekable();
    let (mut last, mut junction, mut prev) = (None, None, None);
    for p in outside {
        let (n, t) = p.map_err(io)?;
        if prev == Some((n, t)) {
            continue;
        }
        prev = Some((n, t));
        if last != Some(n) {
            last = Some(n);
            while uses.next_if(|x| x.as_ref().is_ok_and(|x| x.0 < n)).is_some() {}
            let mut flags = Vec::new();
            while let Some(x) = uses.next_if(|x| x.as_ref().is_ok_and(|x| x.0 == n)) {
                flags.push(x.map_err(io)?.1 as u8);
            }
            if let Some(Err(_)) = uses.peek() {
                return Err(io(uses.next().expect("peeked").expect_err("an error")));
            }
            junction = global_junction(&JunctionUses::of(flags.into_iter()));
        }
        let mut flags = match junction {
            Some(true) => JUNCTION | ROAD,
            Some(false) => JUNCTION,
            None => 0,
        };
        if home.binary_search(&(n, t as u32)).is_ok() {
            flags |= HOME;
        }
        if flags != 0 {
            let mut row = n.to_le_bytes().to_vec();
            row.push(flags);
            parts.add(t as u32, "junctions", row)?;
        }
    }
    parts.flush()
}

/// The junctions file of a tile from its part (no part: a file with no rows).
fn write_junctions(parts: &Parts, tile: u32, out: &Path) -> Result<(), String> {
    let e = |e: io::Error| format!("{}: {e}", out.display());
    let mut f = io::BufWriter::new(std::fs::File::create(out).map_err(e)?);
    f.write_all(JUNCTIONS_MAGIC).map_err(e)?;
    if let Some(mut p) = parts.reader(tile, "junctions") {
        io::copy(&mut p, &mut f).map_err(e)?;
    }
    parts.remove(tile, "junctions");
    f.flush().map_err(e)
}

/// Read a junctions file of the tile cut: (node id, flags [`JUNCTION`], [`ROAD`], [`HOME`]).
pub fn read_junctions(path: &Path) -> Result<Vec<(i64, u8)>, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if b.len() < 8 || &b[..8] != JUNCTIONS_MAGIC || (b.len() - 8) % 9 != 0 {
        return Err(format!("{}: not a junctions file of this version of the tile cut", path.display()));
    }
    Ok(b[8..].chunks(9).map(|r| (i64::from_le_bytes(r[..8].try_into().unwrap()), r[8])).collect())
}

/// The positions of the nodes of `ids` in a PBF file (the home nodes of a tile: the build of the tile holds them).
pub fn node_positions<R: Read + Send>(r: R, ids: &crate::util::FxHashSet<i64>) -> Result<Vec<(i64, (f64, f64))>, String> {
    struct Find<'a>(&'a crate::util::FxHashSet<i64>, Vec<(i64, (f64, f64))>);
    impl RawSink for Find<'_> {
        fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
            if self.0.contains(&id) {
                self.1.push((id, (lon, lat)));
            }
        }
        fn way<'a>(&mut self, _: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = i64>) {}
        fn relation<'a>(
            &mut self,
            _: i64,
            _: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
        ) {
        }
    }
    let mut out = Vec::new();
    let mut r = r;
    let mut buf = Buffers::default();
    while let Some(b) = read_block(&mut r)? {
        let mut s = Find(ids, Vec::new());
        decode_block(&b, [true, false, false], &mut buf, &mut s)?;
        out.extend(s.1);
    }
    Ok(out)
}

/// Cut the region file that `open` reads (a new reader for each pass) to the tiles of `tiles` inside `boundary` (one zoom
/// with a border; else also mixed zooms, no tile in another):
/// the file of a tile goes into `dir` as `z_x_y.osm.pbf`; a tile with no node inside has no file. Gives the numbers of
/// elements of each tile. `multipolygons`: complete the multipolygons of each tile with all their members (osmium's smart
/// cut). Only `raw.*`, the features and the building faces read them; the road network does not, and without them the
/// cut of France reads 60 of 80 million extra nodes less.
pub fn cut_to_tiles<R: Read + Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    tiles: &[Tile],
    dir: &Path,
    multipolygons: bool,
    border_m: f64,
) -> Result<Vec<CutCounts>, String> {
    let tmp = std::env::temp_dir().join(format!("duckosm-tiles-{}-{}", std::process::id(), dir.as_os_str().len()));
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let complete: &[&str] = if multipolygons { &["multipolygon"] } else { &[] };
    let cut = cut(open, boundary, tiles, complete, border_m, Files::Dir(tmp.clone()), Some(dir), true);
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(cut?.0)
}

/// The tile files of [`cut_to_store`], in one temporary file of the store.
pub struct TileFiles {
    parts: Parts,
    /// The tiles of the cut (the order of the list of the cut) and the numbers of elements of each; a tile with no node
    /// inside has no file.
    pub tiles: Vec<(Tile, CutCounts)>,
}

impl TileFiles {
    /// The `.osm.pbf` file of tile `i` of [`TileFiles::tiles`] (empty for a tile with no file).
    pub fn reader(&self, i: usize) -> impl Read + Send + '_ {
        self.parts.reader(i as u32, "tile").unwrap_or_else(|| Box::new(io::empty()))
    }
}

/// [`cut_to_tiles`] into the temporary store (`spill::temp_store`: a folder natively, OPFS in the browser), with no
/// junctions files: quack by tiles (`crate::quack::features_by_tiles`). `complete`: the types of the relations to complete
/// with all their members (quack: `multipolygon` and `boundary`), each one in one tile only: the first tile (in the order
/// of `tiles`) with a member.
pub fn cut_to_store<R: Read + Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    tiles: &[Tile],
    complete: &[&str],
) -> Result<TileFiles, String> {
    let chunks = Chunks::new(&*crate::spill::temp_store()).map_err(|e| format!("a temporary file of the tile cut: {e}"))?;
    let (counts, parts) = cut(open, boundary, tiles, complete, 0.0, Files::Store(chunks), None, false)?;
    Ok(TileFiles { parts, tiles: tiles.iter().copied().zip(counts).collect() })
}

/// The cut of the build by tiles ([`cut_to_tiles`]: each multipolygon complete in every tile with a member, the junctions)
/// into the temporary store: the cut where no folder is (the browser). The tile files are the part "tile" of each tile.
pub fn cut_to_store_build<R: Read + Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    tiles: &[Tile],
    multipolygons: bool,
    border_m: f64,
) -> Result<TileFiles, String> {
    let chunks = Chunks::new(&*crate::spill::temp_store()).map_err(|e| format!("a temporary file of the tile cut: {e}"))?;
    let complete: &[&str] = if multipolygons { &["multipolygon"] } else { &[] };
    let (counts, parts) = cut(open, boundary, tiles, complete, border_m, Files::Store(chunks), None, true)?;
    Ok(TileFiles { parts, tiles: tiles.iter().copied().zip(counts).collect() })
}

/// The cut of [`cut_to_tiles`] and [`cut_to_store`]: the part files in `files`; the tile files into `dir` with their
/// junctions files, or with no `dir` into `files` as the part "tile" of each tile. `build`: the cut of the build by tiles
/// (each relation to complete in every tile with a member, and the junctions); else quack's cut (each one in its first
/// tile only, no junctions).
fn cut<R: Read + Seek + Send>(
    open: impl Fn() -> Result<R, String>,
    boundary: &Boundary,
    tiles: &[Tile],
    complete: &[&str],
    border_m: f64,
    files: Files,
    dir: Option<&Path>,
    build: bool,
) -> Result<(Vec<CutCounts>, Parts), String> {
    let io = |e: io::Error| format!("a temporary file of the tile cut: {e}");
    let (timestamp, sorted) = header(open()?)?;
    if !sorted {
        return Err("the region file is not sorted by type and id (Sort.Type_then_ID)".into());
    }
    let parts = Parts { files, waiting: Default::default() };
    let Some(z) = tiles.first().map(|t| t.z) else {
        return Ok((Vec::new(), parts));
    };
    if border_m > 0.0 && tiles.iter().any(|t| t.z != z) {
        return Err("the tile cut with a border needs tiles of one zoom".into());
    }
    let index = TileIndex::new(tiles);
    let store = crate::spill::temp_store();
    let store = &*store;
    // 1. each node inside to the node part of its tile; the book of the nodes
    let (mut nodes, mut node_blocks, mut ways_at) = (Book::default(), Vec::new(), None);
    let mut near: Vec<(i64, u32)> = Vec::new();
    each_block(
        &mut open()?,
        |b, buf| {
            let mut s = Nodes {
                boundary,
                z,
                index: &index,
                border_m,
                runs: BlockRuns::new(),
                inside: Vec::new(),
                near: Vec::new(),
                other: false,
                span: EMPTY,
            };
            decode_block(b, [true, true, true], buf, &mut s)?;
            Ok((s.runs, by_tile(s.inside)?, s.other, s.span, s.near))
        },
        |(runs, blobs, other, span, mut n), at| {
            near.append(&mut n);
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
    tidy(&mut near);
    crate::util::step(&format!("tile cut 1: the nodes inside ({} runs, {} MB; {} nodes in the border of another tile)", nodes.len(), nodes.size() >> 20, near.len()));
    // 2. each way with a node inside to the way part of its tiles; the nodes that a tile needs from outside it (to a
    // temporary list); the relations, whole (to a temporary file)
    let ways_at = ways_at.unwrap_or(0);
    let (mut ways, mut multi) = (Book::default(), Vec::new());
    let mut outside = Writer::new(store).map_err(io)?;
    let mut uses = Writer::new(store).map_err(io)?;
    // the nodes outside the area are few (the border of the area): in memory
    let mut homes: Vec<(i64, i64, u32)> = Vec::new();
    let (mut way_blocks, mut rel_blocks) = (Vec::new(), Vec::new());
    let mut r = open()?;
    r.seek(SeekFrom::Start(ways_at)).map_err(|e| e.to_string())?;
    each_block(
        &mut r,
        |b, buf| {
            let mut s = Ways {
                nodes: &nodes,
                near: &near,
                runs: BlockRuns::new(),
                kept: Vec::new(),
                multi: Vec::new(),
                outside: Vec::new(),
                uses: Vec::new(),
                homes: Vec::new(),
                rels: Vec::new(),
                tiles: Vec::new(),
                spans: [EMPTY; 2],
            };
            decode_block(b, [false, true, true], buf, &mut s)?;
            let mut rels = Vec::new();
            blobs(s.rels, &mut rels)?;
            Ok((s.runs, by_tile(s.kept)?, s.multi, (s.outside, s.uses, s.homes), rels, s.spans))
        },
        |(runs, kept, mut m, (o, u, h), rels, spans), at| {
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
            for (n, f) in u {
                uses.push(&Use, &(n, f)).map_err(io)?;
            }
            homes.extend(h);
            parts.add(ALL, "relations.all", rels)?;
            Ok(true)
        },
    )?;
    parts.flush()?;
    ways.finish();
    tidy(&mut multi);
    // the home tile of each node outside the area: no tile holds it, so the first tile (in the order of the tiles) of the ways
    // at the node gets all these ways, and holds the node (its turns, its rows)
    homes.sort_unstable();
    let (mut home_ways, mut home_nodes): (Vec<(i64, u32)>, Vec<(i64, u32)>) = (Vec::new(), Vec::new());
    for run in homes.chunk_by(|a, b| a.0 == b.0) {
        let home = run.iter().map(|h| h.2).min().expect("a run");
        home_nodes.push((run[0].0, home));
        for &(_, w, first) in run {
            let multi_tiles: Vec<u32> = tiles_in(&multi, w).collect();
            let in_home = if multi_tiles.is_empty() { first == home } else { multi_tiles.contains(&home) };
            if !in_home {
                home_ways.push((w, home));
            }
        }
    }
    drop(homes);
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
        let mut rd = PartReader::open(&parts, ALL, "relations.all");
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
        v.par_sort_unstable();
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
    // the relations of each batch on all threads (France: 3.5 s on one thread before), the results in the order of the file
    let mut rd = PartReader::open(&parts, ALL, "relations.all");
    while let Some(batch) = rd.next_batch()? {
        let found: Vec<(bool, Vec<u32>, Vec<i64>)> = batch
            .par_iter()
            .filter_map(|e| match e {
                Elem::Relation(r) => Some(r),
                _ => None,
            })
            .map(|r| {
                // the tiles of this relation, with no doubles (a route has thousands of members in a few tiles)
                let (mut tiles, mut children) = (Vec::new(), Vec::new());
                for &(m, t, _) in &r.members {
                    match t {
                        MemberType::Node if node_members.binary_search(&m).is_ok() => tiles.extend(nodes.get(m)),
                        MemberType::Way if way_members.binary_search(&m).is_ok() => way_tiles(m, &mut tiles),
                        MemberType::Relation => children.push(m),
                        _ => {}
                    }
                }
                tiles.sort_unstable();
                tiles.dedup();
                (r.tags.iter().any(|(k, v)| k == "type" && complete.contains(&v.as_str())), tiles, children)
            })
            .collect();
        let rels = batch.iter().filter_map(|e| match e {
            Elem::Relation(r) => Some(r.id),
            _ => None,
        });
        for (id, (multipolygon, tiles, children)) in rels.zip(found) {
            if multipolygon {
                multipolygons.push(id);
            }
            parents.extend(children.into_iter().map(|m| (m, id)));
            direct.extend(tiles.into_iter().map(|t| (id, t)));
        }
    }
    drop(rd);
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
            // quack needs each one whole in one tile only, its first: a boundary of a country has a million nodes and is
            // in hundreds of tiles
            for t in tiles_in(&direct, r.id).take(if build { usize::MAX } else { 1 }) {
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
            for (t, bytes) in by_tile_par(std::mem::take(&mut waiting))? {
                parts.add(t, "relations", bytes)?;
            }
        }
        Ok(())
    })?;
    for (t, bytes) in by_tile_par(waiting)? {
        parts.add(t, "relations", bytes)?;
    }
    parts.remove(ALL, "relations.all");
    drop((direct, chosen, multipolygons));
    more.extend(home_ways);
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
    let mut outside = sort_bytes(outside.finish().map_err(io)?, |b| Pair.get(&mut &b[..]), 64 << 20, store).map_err(io)?;
    crate::util::step(&format!("tile cut 4a: the nodes from outside, sorted ({} rows)", outside.len));
    let mut r = open()?;
    take_blocks(&mut r, &node_blocks, 0, outside.read_ref(&Pair).map_err(io)?, &parts, "extra-nodes")?;
    crate::util::step("tile cut 4b: the extra nodes");
    // the junctions files: only the build by tiles reads them
    if build {
        let uses = uses.finish().map_err(io)?;
        let n_uses = uses.len;
        let uses = sort_bytes(uses, |b| i64::from_le_bytes(b[..8].try_into().expect("8 bytes")), 64 << 20, store).map_err(io)?;
        crate::util::step(&format!("tile cut 4c: the uses of the nodes, sorted ({n_uses} rows)"));
        home_nodes.sort_unstable();
        let uses = uses.read(&Use).map_err(io)?.map(|u| u.map(|(n, f)| (n, f as i64)));
        outside_junctions(outside.read(&Pair).map_err(io)?, uses, &home_nodes, &parts)?;
        crate::util::step("tile cut 4d: the junctions files");
    } else {
        drop(uses);
    }
    take_blocks(&mut r, &way_blocks, 1, more.iter().map(|&(w, t)| Ok((w, t as i64))), &parts, "extra-ways")?;
    parts.flush()?;
    drop(more);
    crate::util::step("tile cut 4: the extra elements");
    // 5. each tile from its parts (the tiles at the same time; in the browser one at a time, because a file in OPFS
    // belongs to the thread that opened it)
    let finish = |t: u32| -> Result<CutCounts, String> {
        if !parts.has(t, "nodes") {
            for part in ["ways", "extra-nodes", "extra-ways", "relations", "junctions"] {
                parts.remove(t, part);
            }
            return Ok(CutCounts::default());
        }
        let tile = &tiles[t as usize];
        match (dir, &parts.files) {
            (Some(dir), _) => {
                let name = format!("{}_{}_{}", tile.z, tile.x, tile.y);
                write_junctions(&parts, t, &dir.join(format!("{name}.junctions")))?;
                let out = dir.join(format!("{name}.osm.pbf"));
                let file = std::fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
                finish_tile(&parts, t, file, timestamp)
            }
            (None, Files::Store(c)) => finish_tile(&parts, t, ChunkWriter(c, (t, "tile")), timestamp),
            (None, _) => Err("the tile cut has no folder for the tile files".into()),
        }
    };
    // the store with no folder: in memory in the browser, and in the tests (the test of the browser's way)
    let counts: Vec<CutCounts> = if dir.is_none() && (cfg!(target_arch = "wasm32") || cfg!(test)) {
        finish_tiles_in_memory(&parts, tiles.len(), timestamp)?
    } else {
        (0..tiles.len() as u32).into_par_iter().map(finish).collect::<Result<_, String>>()?
    };
    crate::util::step("tile cut 5: the tile files");
    Ok((counts, parts))
}
