//! Rows in temporary files for the passes of the network step (docs/design/network_stream.md, part B): a pass reads the
//! rows of a file one at a time and writes its result to a new file, so the edges of a mode do not stay in memory. The
//! same code runs natively (files in a folder) and in the browser (a [`Store`] of the page, for example OPFS).
//!
//! [`sort`] puts the rows of a file in the order of a key with a memory budget: it sorts blocks of rows that fit the
//! budget, writes each block to a file, and merges the blocks. The sort is stable.

use crate::model::{Attrs, Edge};
use crate::util::FxHashMap;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::Arc;

/// The temporary store is the store of pbfcut: one store for the tile cut and the build, so a page sets it one time
/// ([`set_temp_store`], for example a store in OPFS). Natively, the files go into the temporary folder of the system
/// (pbfcut `store::set_temp_dir` changes it).
pub use pbfcut::store::{set_temp_store, temp_store, with_temp_store, Blob, DirStore, MemStore, Store, StoreMaker};

/// How a row is written as bytes and read back. `put` can add to the codec (for example a new shared value), `get`
/// only reads it. A pass reads one file and writes the next with the same codec, so `put` takes `&self`.
pub trait Codec<T> {
    fn put(&self, row: &T, out: &mut Vec<u8>);
    fn get(&self, b: &mut &[u8]) -> T;
}

/// The rows of a temporary file: each row is its length (4 bytes) and its bytes. With `snappy`, the rows are in blocks
/// of about [`BLOCK`] bytes, each block compressed with Snappy (pure Rust, also in the browser) and written as its
/// length (4 bytes) and its bytes; a row is in one block.
pub struct Rows {
    blob: Box<dyn Blob>,
    pub len: usize,
    /// The bytes of the rows (not compressed).
    pub bytes: u64,
    snappy: bool,
}

/// The size of a block of rows of a file with Snappy.
const BLOCK: usize = 1 << 20;

/// Writes rows to a new temporary file.
pub struct Writer {
    w: BufWriter<Box<dyn Blob>>,
    buf: Vec<u8>,
    len: usize,
    bytes: u64,
    /// The rows of the block that waits for its compression (Snappy files only).
    block: Option<Vec<u8>>,
}

impl Writer {
    pub fn new(store: &dyn Store) -> io::Result<Self> {
        Self::with_buffer(store, 1 << 20)
    }

    /// A writer with a buffer of `bytes` (a table of many writers, as [`Buckets`], keeps one buffer for each writer).
    pub fn with_buffer(store: &dyn Store, bytes: usize) -> io::Result<Self> {
        Ok(Writer {
            w: BufWriter::with_capacity(bytes, store.create()?),
            buf: Vec::new(),
            len: 0,
            bytes: 0,
            block: None,
        })
    }

    /// A writer of a file with Snappy blocks: the rows of a sort repeat much (the same keys and attributes), and the
    /// disk holds 3 to 5 times less.
    pub fn snappy(store: &dyn Store) -> io::Result<Self> {
        let mut w = Self::new(store)?;
        w.block = Some(Vec::with_capacity(BLOCK + (BLOCK >> 3)));
        Ok(w)
    }

    fn write_block(w: &mut BufWriter<Box<dyn Blob>>, block: &mut Vec<u8>) -> io::Result<()> {
        let z = snap::raw::Encoder::new().compress_vec(block).map_err(io::Error::other)?;
        w.write_all(&(z.len() as u32).to_le_bytes())?;
        w.write_all(&z)?;
        block.clear();
        Ok(())
    }

    pub fn push<T>(&mut self, codec: &impl Codec<T>, row: &T) -> io::Result<()> {
        let mut buf = std::mem::take(&mut self.buf);
        buf.clear();
        codec.put(row, &mut buf);
        let r = self.push_bytes(&buf);
        self.buf = buf;
        r
    }

    fn push_bytes(&mut self, b: &[u8]) -> io::Result<()> {
        match &mut self.block {
            None => {
                self.w.write_all(&(b.len() as u32).to_le_bytes())?;
                self.w.write_all(b)?;
            }
            Some(block) => {
                block.extend_from_slice(&(b.len() as u32).to_le_bytes());
                block.extend_from_slice(b);
                if block.len() >= BLOCK {
                    Self::write_block(&mut self.w, block)?;
                }
            }
        }
        self.len += 1;
        self.bytes += 4 + b.len() as u64;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<Rows> {
        let snappy = self.block.is_some();
        if let Some(block) = &mut self.block {
            if !block.is_empty() {
                Self::write_block(&mut self.w, block)?;
            }
        }
        let blob = self.w.into_inner().map_err(|e| e.into_error())?;
        Ok(Rows {
            blob,
            len: self.len,
            bytes: self.bytes,
            snappy,
        })
    }
}

/// Reads the bytes of each row from the start of a file.
pub struct RawIter<R = Box<dyn Blob>> {
    r: BufReader<R>,
    left: usize,
    /// The block that is read and the place in it (Snappy files only).
    block: Option<(Vec<u8>, usize)>,
}

impl<R: Read> RawIter<R> {
    fn new(r: R, len: usize, snappy: bool) -> Self {
        RawIter { r: BufReader::with_capacity(1 << 20, r), left: len, block: snappy.then(|| (Vec::new(), 0)) }
    }

    /// The bytes of the next row (its length is read first) at the end of `data`.
    fn append_into(&mut self, data: &mut Vec<u8>) -> io::Result<bool> {
        if self.left == 0 {
            return Ok(false);
        }
        self.left -= 1;
        let start = data.len();
        match &mut self.block {
            None => {
                let mut n = [0u8; 4];
                self.r.read_exact(&mut n)?;
                data.resize(start + u32::from_le_bytes(n) as usize, 0);
                self.r.read_exact(&mut data[start..])?;
            }
            Some((block, at)) => {
                if *at == block.len() {
                    let mut n = [0u8; 4];
                    self.r.read_exact(&mut n)?;
                    let mut z = vec![0u8; u32::from_le_bytes(n) as usize];
                    self.r.read_exact(&mut z)?;
                    *block = snap::raw::Decoder::new().decompress_vec(&z).map_err(io::Error::other)?;
                    *at = 0;
                }
                let n = u32::from_le_bytes(block[*at..*at + 4].try_into().expect("4 bytes")) as usize;
                data.extend_from_slice(&block[*at + 4..*at + 4 + n]);
                *at += 4 + n;
            }
        }
        Ok(true)
    }

    fn next_into(&mut self, buf: &mut Vec<u8>) -> io::Result<bool> {
        buf.clear();
        self.append_into(buf)
    }
}

impl Rows {
    fn raw(mut self) -> io::Result<RawIter> {
        self.blob.seek(SeekFrom::Start(0))?;
        Ok(RawIter::new(self.blob, self.len, self.snappy))
    }

    /// Read the rows, one at a time, and keep the file for a next read.
    pub fn read_ref<'a, T, C: Codec<T>>(
        &'a mut self,
        codec: &'a C,
    ) -> io::Result<impl Iterator<Item = io::Result<T>> + 'a> {
        self.blob.seek(SeekFrom::Start(0))?;
        let mut raw = RawIter::new(&mut self.blob, self.len, self.snappy);
        let mut buf = Vec::new();
        Ok(std::iter::from_fn(move || match raw.next_into(&mut buf) {
            Ok(true) => Some(Ok(codec.get(&mut &buf[..]))),
            Ok(false) => None,
            Err(e) => Some(Err(e)),
        }))
    }

    /// Read the rows, one at a time. The file is gone after the last row.
    pub fn read<'c, T, C: Codec<T>>(
        self,
        codec: &'c C,
    ) -> io::Result<impl Iterator<Item = io::Result<T>> + 'c> {
        let mut raw = self.raw()?;
        let mut buf = Vec::new();
        Ok(std::iter::from_fn(move || match raw.next_into(&mut buf) {
            Ok(true) => Some(Ok(codec.get(&mut &buf[..]))),
            Ok(false) => None,
            Err(e) => Some(Err(e)),
        }))
    }
}

/// The rows of `rows` in the order of `key` (stable), with the key from the decoded row. See [`sort_bytes`].
pub fn sort<T, K: Ord + Send, C: Codec<T>>(
    rows: Rows,
    codec: &C,
    key: impl Fn(&T) -> K,
    budget: usize,
    store: &dyn Store,
) -> io::Result<Rows> {
    sort_bytes(rows, |b| key(&codec.get(&mut &b[..])), budget, store)
}

/// The rows of `rows` in the order of `key` (stable), with the key from the bytes of the row: a codec with its key
/// fields at fixed places at the start of the row gives a key with no decode of the rest. Each sorted block holds about
/// `budget` bytes of memory: the bytes of its rows in one buffer, and for each row its key and its place.
pub fn sort_bytes<K: Ord + Send>(
    rows: Rows,
    key: impl Fn(&[u8]) -> K,
    budget: usize,
    store: &dyn Store,
) -> io::Result<Rows> {
    sort_bytes_where(rows, |_| true, key, budget, store)
}

/// [`sort_bytes`] of the rows for which `keep` is true. `keep` sees each row one time, in the order of the file.
pub fn sort_bytes_where<K: Ord + Send>(
    rows: Rows,
    keep: impl FnMut(&[u8]) -> bool,
    key: impl Fn(&[u8]) -> K,
    budget: usize,
    store: &dyn Store,
) -> io::Result<Rows> {
    let mut runs = sorted_runs(rows, keep, &key, budget, store)?;
    if runs.len() == 1 {
        return Ok(runs.pop().expect("one run"));
    }
    merge(runs, &key, store, false)
}

/// [`sort_bytes`] for rows that are read one time: the rows in the order of `key` (stable), merged from the sorted runs
/// while they are read. No merged copy is written: one write and one read of all rows less, and no time with the runs
/// and their copy on the disk together (the cut of France: part 4c had 1.5 GB of runs and wrote 1.5 GB more). The key
/// can borrow: the rows live as long as it does.
pub fn sorted<'a, K: Ord + Send + 'a, T: 'a, C: Codec<T>>(
    rows: Rows,
    key: impl Fn(&[u8]) -> K + 'a,
    budget: usize,
    store: &dyn Store,
    codec: &'a C,
) -> io::Result<impl Iterator<Item = io::Result<T>> + 'a> {
    let runs = sorted_runs(rows, |_| true, &key, budget, store)?;
    merged(runs, key, codec)
}

/// The rows of sorted `runs`, merged while they are read: the smallest (key, run) first, as in `merge`.
fn merged<'a, K: Ord + Send + 'a, T: 'a, C: Codec<T>>(
    runs: Vec<Rows>,
    key: impl Fn(&[u8]) -> K + 'a,
    codec: &'a C,
) -> io::Result<impl Iterator<Item = io::Result<T>> + 'a> {
    let mut its: Vec<RawIter> = runs.into_iter().map(Rows::raw).collect::<io::Result<_>>()?;
    let mut heads: Vec<Vec<u8>> = vec![Vec::new(); its.len()];
    let mut heap = BinaryHeap::new();
    for (i, it) in its.iter_mut().enumerate() {
        if it.next_into(&mut heads[i])? {
            heap.push(Reverse((key(&heads[i]), i)));
        }
    }
    // the smallest (key, run) first, as in `merge`: the same order as `sort_bytes`
    let mut failed = false;
    Ok(std::iter::from_fn(move || {
        if failed {
            return None;
        }
        let Reverse((_, i)) = heap.pop()?;
        let row = codec.get(&mut &heads[i][..]);
        match its[i].next_into(&mut heads[i]) {
            Ok(true) => heap.push(Reverse((key(&heads[i]), i))),
            Ok(false) => {}
            Err(e) => {
                failed = true;
                return Some(Err(e));
            }
        }
        Some(Ok(row))
    }))
}

/// The sorted runs of [`sort_bytes_where`] (at least one; one run needs no merge).
fn sorted_runs<K: Ord + Send>(
    rows: Rows,
    mut keep: impl FnMut(&[u8]) -> bool,
    key: &impl Fn(&[u8]) -> K,
    budget: usize,
    store: &dyn Store,
) -> io::Result<Vec<Rows>> {
    let mut raw = rows.raw()?;
    let mut runs: Vec<Rows> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    let mut index: Vec<(K, usize, usize)> = Vec::new(); // key, start and end of the row in `data`
    let flush = |data: &mut Vec<u8>, index: &mut Vec<(K, usize, usize)>, runs: &mut Vec<Rows>| -> io::Result<()> {
        runs.push(write_run(data, index, store, false)?);
        Ok(())
    };
    let row = std::mem::size_of::<(K, usize, usize)>();
    loop {
        let start = data.len();
        if !raw.append_into(&mut data)? {
            break;
        }
        if !keep(&data[start..]) {
            data.truncate(start);
            continue;
        }
        index.push((key(&data[start..]), start, data.len()));
        if data.len() + index.len() * row >= budget {
            flush(&mut data, &mut index, &mut runs)?;
            // each run is an open file: at most MERGE_FAN of them (France: hundreds of runs, more files than a process can
            // open). The runs so far are the first rows of the input, so their merge keeps the sort stable.
            if runs.len() == MERGE_FAN {
                let all = std::mem::take(&mut runs);
                runs.push(merge(all, key, store, false)?);
            }
        }
    }
    drop(raw);
    // the last block (also the only block, also when it is empty)
    if runs.is_empty() || !index.is_empty() {
        flush(&mut data, &mut index, &mut runs)?;
    }
    Ok(runs)
}

/// One sorted run of the rows in `data` (`index`: the key, the start and the end of each row); `data` and `index` are empty after.
fn write_run<K: Ord + Send>(data: &mut Vec<u8>, index: &mut Vec<(K, usize, usize)>, store: &dyn Store, snappy: bool) -> io::Result<Rows> {
    // stable; on all threads when the crate has rayon (the feature `pbf`)
    #[cfg(feature = "pbf")]
    {
        use rayon::slice::ParallelSliceMut;
        index.par_sort_by(|a, b| a.0.cmp(&b.0));
    }
    #[cfg(not(feature = "pbf"))]
    index.sort_by(|a, b| a.0.cmp(&b.0));
    let mut w = if snappy { Writer::snappy(store)? } else { Writer::new(store)? };
    for (_, a, b) in index.drain(..) {
        w.push_bytes(&data[a..b])?;
    }
    data.clear();
    w.finish()
}

/// A sort with no unsorted file: [`Sorter::push`] keeps the rows in memory up to `budget` bytes and writes each full block as a
/// sorted run, and [`Sorter::sorted`] merges the runs while they are read. The disk holds one copy of the rows at most; with
/// [`sort_bytes`], the unsorted file and the runs (or the runs and their merged copy) are on the disk together (the levels after
/// the union of Kentucky by tiles: 2.86 GB in the browser). The order is the order of [`sort_bytes`] (stable).
pub struct Sorter<'s, K, F: Fn(&[u8]) -> K> {
    key: F,
    budget: usize,
    store: &'s dyn Store,
    data: Vec<u8>,
    index: Vec<(K, usize, usize)>,
    runs: Vec<Rows>,
    buf: Vec<u8>,
    snappy: bool,
}

impl<'s, K: Ord + Send, F: Fn(&[u8]) -> K> Sorter<'s, K, F> {
    pub fn new(key: F, budget: usize, store: &'s dyn Store) -> Self {
        Sorter { key, budget, store, data: Vec::new(), index: Vec::new(), runs: Vec::new(), buf: Vec::new(), snappy: false }
    }

    /// The runs (and their merges) in files with Snappy blocks ([`Writer::snappy`]): less disk for rows that repeat.
    pub fn snappy(mut self) -> Self {
        self.snappy = true;
        self
    }

    pub fn push<T>(&mut self, codec: &impl Codec<T>, row: &T) -> io::Result<()> {
        let mut buf = std::mem::take(&mut self.buf);
        buf.clear();
        codec.put(row, &mut buf);
        let start = self.data.len();
        self.data.extend_from_slice(&buf);
        self.buf = buf;
        self.index.push(((self.key)(&self.data[start..]), start, self.data.len()));
        if self.data.len() + self.index.len() * std::mem::size_of::<(K, usize, usize)>() >= self.budget {
            self.runs.push(write_run(&mut self.data, &mut self.index, self.store, self.snappy)?);
            // at most MERGE_FAN open runs; the runs so far are the first rows, so their merge keeps the sort stable
            if self.runs.len() == MERGE_FAN {
                let all = std::mem::take(&mut self.runs);
                self.runs.push(merge(all, &self.key, self.store, self.snappy)?);
            }
        }
        Ok(())
    }

    /// The rows in the order of the key, merged from the runs while they are read.
    pub fn sorted<'a, T: 'a, C: Codec<T>>(mut self, codec: &'a C) -> io::Result<impl Iterator<Item = io::Result<T>> + 'a>
    where
        K: 'a,
        F: 'a,
    {
        if self.runs.is_empty() || !self.index.is_empty() {
            self.runs.push(write_run(&mut self.data, &mut self.index, self.store, self.snappy)?);
        }
        let runs = std::mem::take(&mut self.runs);
        merged(runs, self.key, codec)
    }
}

/// The runs that a merge of [`sort_bytes`] reads at once.
const MERGE_FAN: usize = 64;

/// The merge of sorted runs: the smallest (key, run) first; a run is in the order of the input, so the result is stable.
fn merge<K: Ord>(runs: Vec<Rows>, key: &impl Fn(&[u8]) -> K, store: &dyn Store, snappy: bool) -> io::Result<Rows> {
    let mut its: Vec<RawIter> = runs.into_iter().map(Rows::raw).collect::<io::Result<_>>()?;
    let mut heap = BinaryHeap::new();
    let mut heads: Vec<Vec<u8>> = vec![Vec::new(); its.len()];
    for (i, it) in its.iter_mut().enumerate() {
        if it.next_into(&mut heads[i])? {
            heap.push(Reverse((key(&heads[i]), i)));
        }
    }
    let mut w = if snappy { Writer::snappy(store)? } else { Writer::new(store)? };
    while let Some(Reverse((_, i))) = heap.pop() {
        w.push_bytes(&heads[i])?;
        if its[i].next_into(&mut heads[i])? {
            heap.push(Reverse((key(&heads[i]), i)));
        }
    }
    w.finish()
}

pub(crate) fn put_u32(o: &mut Vec<u8>, v: u32) {
    o.extend_from_slice(&v.to_le_bytes());
}
pub(crate) fn put_i64(o: &mut Vec<u8>, v: i64) {
    o.extend_from_slice(&v.to_le_bytes());
}
pub(crate) fn put_f32(o: &mut Vec<u8>, v: f32) {
    o.extend_from_slice(&v.to_le_bytes());
}
pub(crate) fn take<const N: usize>(b: &mut &[u8]) -> [u8; N] {
    let (h, t) = b.split_at(N);
    *b = t;
    h.try_into().unwrap()
}
pub(crate) fn get_u32(b: &mut &[u8]) -> u32 {
    u32::from_le_bytes(take(b))
}
pub(crate) fn get_i64(b: &mut &[u8]) -> i64 {
    i64::from_le_bytes(take(b))
}
pub(crate) fn get_f32(b: &mut &[u8]) -> f32 {
    f32::from_le_bytes(take(b))
}
pub(crate) fn get_f64(b: &mut &[u8]) -> f64 {
    f64::from_le_bytes(take(b))
}

/// Values that many rows share, stored one time: a row has the number of the value.
struct Shared<T: ?Sized> {
    values: Vec<Arc<T>>,
    ids: FxHashMap<usize, u32>, // the address of the shared value -> its number
}

impl<T: ?Sized> Default for Shared<T> {
    fn default() -> Self {
        Shared {
            values: Vec::new(),
            ids: FxHashMap::default(),
        }
    }
}

impl<T: ?Sized> Shared<T> {
    fn id(&mut self, v: &Arc<T>) -> u32 {
        let values = &mut self.values;
        *self
            .ids
            .entry(Arc::as_ptr(v) as *const () as usize)
            .or_insert_with(|| {
                values.push(v.clone());
                (values.len() - 1) as u32
            })
    }
}

/// [`Edge`] as bytes. The tags of the way (`Arc<Attrs>`), the access value and the functional type are shared by many
/// edges: the codec keeps them, and an edge has their numbers. The codec keeps each shared value alive, so its address
/// is not used again for another value.
#[derive(Default)]
pub struct EdgeCodec(RefCell<EdgeValues>);

#[derive(Default)]
struct EdgeValues {
    attrs: Shared<Attrs>,
    access: Shared<str>,
    ftypes: Vec<&'static str>,
}

const NONE: u32 = u32::MAX;

impl EdgeValues {
    fn ftype_id(&mut self, t: &'static str) -> u32 {
        match self
            .ftypes
            .iter()
            .position(|&x| std::ptr::eq(x, t) || x == t)
        {
            Some(i) => i as u32,
            None => {
                self.ftypes.push(t);
                (self.ftypes.len() - 1) as u32
            }
        }
    }
}

impl EdgeCodec {
    /// The `edge_id` of an edge from its bytes: the first 8 bytes.
    pub fn edge_id(b: &[u8]) -> i64 {
        i64::from_le_bytes(b[..8].try_into().unwrap())
    }

    /// The `source` and the `target` of an edge from its bytes: after `edge_id` and `seq`.
    pub fn ends(b: &[u8]) -> (i64, i64) {
        (
            i64::from_le_bytes(b[12..20].try_into().unwrap()),
            i64::from_le_bytes(b[20..28].try_into().unwrap()),
        )
    }

    /// The `osm_id` of an edge from its bytes: after `edge_id`, `seq`, `source` and `target`.
    pub fn osm_id(b: &[u8]) -> i64 {
        i64::from_le_bytes(b[28..36].try_into().unwrap())
    }
}

impl Codec<Edge> for EdgeCodec {
    fn put(&self, e: &Edge, o: &mut Vec<u8>) {
        let mut v = self.0.borrow_mut();
        put_i64(o, e.edge_id);
        put_u32(o, e.seq.unwrap_or(NONE));
        put_i64(o, e.source);
        put_i64(o, e.target);
        put_i64(o, e.osm_id);
        put_u32(o, v.attrs.id(&e.attrs));
        put_u32(o, e.access.as_ref().map_or(NONE, |a| v.access.id(a)));
        put_u32(o, e.ftype.map_or(NONE, |t| v.ftype_id(t)));
        let flags = e.oneway as u8
            | (e.is_reverse as u8) << 1
            | (e.lanes.is_some() as u8) << 2
            | match e.dismount {
                None => 0,
                Some(false) => 1 << 3,
                Some(true) => 2 << 3,
            }
            | (e.maxspeed_kmh.is_some() as u8) << 5
            | (e.cost_s.is_some() as u8) << 6;
        o.push(flags);
        if let Some(l) = e.lanes {
            o.extend_from_slice(&l.to_le_bytes());
        }
        put_f32(o, e.length_m);
        if let Some(v) = e.maxspeed_kmh {
            put_f32(o, v);
        }
        if let Some(v) = e.cost_s {
            put_f32(o, v);
        }
        put_u32(o, e.refs.len() as u32);
        for &r in &e.refs {
            put_i64(o, r);
        }
        put_u32(o, e.geometry.len() as u32);
        for p in &e.geometry {
            o.extend_from_slice(&p.0.to_le_bytes());
            o.extend_from_slice(&p.1.to_le_bytes());
        }
    }

    fn get(&self, b: &mut &[u8]) -> Edge {
        let v = self.0.borrow();
        let edge_id = get_i64(b);
        let seq = Some(get_u32(b)).filter(|&s| s != NONE);
        let (source, target, osm_id) = (get_i64(b), get_i64(b), get_i64(b));
        let attrs = v.attrs.values[get_u32(b) as usize].clone();
        let access = Some(get_u32(b))
            .filter(|&a| a != NONE)
            .map(|a| v.access.values[a as usize].clone());
        let ftype = Some(get_u32(b))
            .filter(|&t| t != NONE)
            .map(|t| v.ftypes[t as usize]);
        let flags = take::<1>(b)[0];
        let lanes = (flags & 4 != 0).then(|| i32::from_le_bytes(take(b)));
        let length_m = get_f32(b);
        let maxspeed_kmh = (flags & 32 != 0).then(|| get_f32(b));
        let cost_s = (flags & 64 != 0).then(|| get_f32(b));
        let n = get_u32(b) as usize;
        let refs = (0..n).map(|_| get_i64(b)).collect();
        let n = get_u32(b) as usize;
        let geometry = (0..n).map(|_| (get_f64(b), get_f64(b))).collect();
        Edge {
            edge_id,
            seq,
            source,
            target,
            osm_id,
            attrs,
            oneway: flags & 1 != 0,
            lanes,
            access,
            refs,
            geometry,
            is_reverse: flags & 2 != 0,
            length_m,
            dismount: match (flags >> 3) & 3 {
                0 => None,
                1 => Some(false),
                _ => Some(true),
            },
            maxspeed_kmh,
            cost_s,
            ftype,
        }
    }
}

#[cfg(test)]
mod sorter_tests {
    use super::*;

    /// A row of one i64.
    struct I64;
    impl Codec<i64> for I64 {
        fn put(&self, r: &i64, out: &mut Vec<u8>) {
            out.extend_from_slice(&r.to_le_bytes());
        }
        fn get(&self, b: &mut &[u8]) -> i64 {
            get_i64(b)
        }
    }

    /// The sorter (no unsorted file; more runs than MERGE_FAN with this budget) gives the rows of `sort_bytes`, in the same stable
    /// order: the key is the row divided by 10, so the rows of one key keep their order.
    #[test]
    fn the_sorter_equals_sort_bytes() {
        let store = temp_store();
        let key = |b: &[u8]| i64::from_le_bytes(b[..8].try_into().unwrap()).div_euclid(10);
        let rows: Vec<i64> = (0..5000).map(|i| (i * 7919) % 5000 - 1300).collect();
        for snappy in [false, true] {
            let mut w = Writer::new(&*store).unwrap();
            let mut s = Sorter::new(key, 200, &*store);
            if snappy {
                s = s.snappy();
            }
            for k in &rows {
                w.push(&I64, k).unwrap();
                s.push(&I64, k).unwrap();
            }
            let want: Vec<i64> = sort_bytes(w.finish().unwrap(), key, 200, &*store).unwrap().read(&I64).unwrap().map(|r| r.unwrap()).collect();
            let got: Vec<i64> = s.sorted(&I64).unwrap().map(|r| r.unwrap()).collect();
            assert_eq!((got.len(), &got), (5000, &want), "snappy {snappy}");
        }
        assert_eq!(Sorter::new(key, 200, &*store).sorted(&I64).unwrap().count(), 0);
    }

    /// A file with Snappy blocks gives its rows back, also over the ends of the blocks and with rows of each length.
    #[test]
    fn snappy_blocks_give_the_rows_back() {
        let store = temp_store();
        let rows: Vec<Vec<u8>> = (0..40_000u32).map(|i| (0..i % 300).map(|j| (i ^ j) as u8).collect()).collect();
        let mut w = Writer::snappy(&*store).unwrap();
        for r in &rows {
            w.push_bytes(r).unwrap();
        }
        let mut f = w.finish().unwrap();
        assert!(f.bytes > 3 * BLOCK as u64, "more than one block");
        for _ in 0..2 {
            f.blob.seek(SeekFrom::Start(0)).unwrap();
            let mut it = RawIter::new(&mut f.blob, f.len, f.snappy);
            let mut buf = Vec::new();
            let mut got = Vec::new();
            while it.next_into(&mut buf).unwrap() {
                got.push(buf.clone());
            }
            assert_eq!(got, rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(i: i64, attrs: &Arc<Attrs>) -> Edge {
        Edge {
            edge_id: i * 7919 % 1000,
            seq: (i % 3 != 0).then_some(i as u32),
            source: i,
            target: i + 1,
            osm_id: 100 + i % 4,
            attrs: attrs.clone(),
            oneway: i % 2 == 0,
            lanes: (i % 5 == 0).then_some(2),
            access: (i % 7 == 0).then(|| Arc::from("bus")),
            refs: (0..(i % 4 + 2)).collect(),
            geometry: (0..(i % 4 + 2))
                .map(|k| (k as f64 * 0.5, i as f64))
                .collect(),
            is_reverse: i % 3 == 0,
            length_m: i as f32 * 1.5,
            dismount: [None, Some(false), Some(true)][(i % 3) as usize],
            maxspeed_kmh: (i % 2 == 1).then_some(50.0),
            cost_s: (i % 2 == 1).then_some(i as f32),
            ftype: (i % 4 == 1).then_some("dismount"),
        }
    }

    fn key(e: &Edge) -> String {
        format!("{e:?}")
    }

    #[test]
    fn edges_come_back_equal_and_sorted() {
        let attrs = Arc::new(Attrs {
            highway: "primary".into(),
            name: Some("A".into()),
            maxspeed: None,
            surface: None,
            junction: None,
            layer: None,
            bridge: None,
            tunnel: None,
            service: None,
        });
        let edges: Vec<Edge> = (0..500).map(|i| edge(i, &attrs)).collect();
        let codec = EdgeCodec::default();
        for (store, budget) in [(&MemStore as &dyn Store, 1 << 30), (&MemStore, 2000)] {
            let mut w = Writer::new(store).unwrap();
            for e in &edges {
                w.push(&codec, e).unwrap();
            }
            let rows = w.finish().unwrap();
            let sorted = sort(rows, &codec, |e: &Edge| e.osm_id, budget, store).unwrap();
            let got: Vec<String> = sorted
                .read(&codec)
                .unwrap()
                .map(|e| key(&e.unwrap()))
                .collect();
            let mut want = edges.clone();
            want.sort_by_key(|e| e.osm_id); // stable, as the sort of the files
            assert_eq!(got, want.iter().map(key).collect::<Vec<_>>());
        }
        assert_eq!(codec.0.borrow().attrs.values.len(), 1);
    }

    /// A sort with more runs than MERGE_FAN: the runs merge while they build up; the order is the key, then the input.
    #[test]
    fn a_sort_with_many_runs_is_stable() {
        let store = MemStore;
        let mut w = Writer::new(&store).unwrap();
        for i in 0..20_000u32 {
            w.push(&join::Request, &((i as i64 * 7919) % 1000, i, 0)).unwrap();
        }
        let rows = w.finish().unwrap();
        let sorted = sort_bytes(rows, join::node_id, 4096, &store).unwrap();
        let got: Vec<(i64, u32)> = sorted.read(&join::Request).unwrap().map(|r| r.map(|x| (x.0, x.1)).unwrap()).collect();
        let mut want: Vec<(i64, u32)> = (0..20_000u32).map(|i| ((i as i64 * 7919) % 1000, i)).collect();
        want.sort();
        assert_eq!(got, want);
    }

    /// `sorted` gives the rows of `sort_bytes` in the same order: with one run, and with more runs than MERGE_FAN (equal
    /// keys keep the order of the input).
    #[test]
    fn sorted_equals_sort_bytes() {
        let store = MemStore;
        let make = || {
            let mut w = Writer::new(&store).unwrap();
            for i in 0..20_000u32 {
                w.push(&join::Request, &((i as i64 * 7919) % 1000, i, 0)).unwrap();
            }
            w.finish().unwrap()
        };
        for budget in [4096, 64 << 20] {
            let want: Vec<(i64, u32, u32)> =
                sort_bytes(make(), join::node_id, budget, &store).unwrap().read(&join::Request).unwrap().map(Result::unwrap).collect();
            let got: Vec<(i64, u32, u32)> = sorted(make(), join::node_id, budget, &store, &join::Request).unwrap().map(Result::unwrap).collect();
            assert_eq!(got.len(), 20_000);
            assert_eq!(got, want, "budget {budget}");
        }
    }

    #[test]
    fn files_in_a_folder_are_removed() {
        let dir = std::env::temp_dir().join(format!("spill-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = DirStore(dir.clone());
        let mut w = Writer::new(&store).unwrap();
        let c = EdgeCodec::default();
        let attrs = Arc::new(Attrs {
            highway: "service".into(),
            name: None,
            maxspeed: None,
            surface: None,
            junction: None,
            layer: None,
            bridge: None,
            tunnel: None,
            service: None,
        });
        for i in 0..100 {
            w.push(&c, &edge(i, &attrs)).unwrap();
        }
        // Unix: no name in the folder while the file is open, so a stopped process leaves nothing
        #[cfg(unix)]
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let rows = sort(w.finish().unwrap(), &c, |e: &Edge| e.edge_id, 1000, &store).unwrap();
        assert_eq!(rows.read(&c).unwrap().count(), 100);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir(&dir).unwrap();
    }
}

/// A join on disk of the nodes of ways with the coordinates of the nodes (docs/concepts/memory-by-step.md): the nodes of
/// the ways and the coordinates, both sorted by node id, give the points of each way in the order of the ways. The memory
/// is that of the sorts. The rows are fixed bytes with the sort key at the start.
/// Rows partitioned by the range of an `i64` key (`key >> shift`) into temporary files: a large table is then sorted one
/// small part at a time in memory, on all threads, instead of by an external merge on one thread. Each row is written
/// once and read once. Write with [`Buckets::push`]; [`Buckets::finish`] gives the parts in the order of their ranges,
/// and [`SortedParts`] reads them back, each part sorted.
#[cfg(feature = "pbf")]
pub struct Buckets {
    shift: u32,
    parts: std::collections::BTreeMap<i64, Writer>,
}

#[cfg(feature = "pbf")]
impl Buckets {
    pub fn new(shift: u32) -> Self {
        Buckets { shift, parts: Default::default() }
    }

    pub fn push<T>(&mut self, store: &dyn Store, key: i64, codec: &impl Codec<T>, row: &T) -> io::Result<()> {
        let part = match self.parts.entry(key >> self.shift) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            // a buffer of 256 KB: quack on Kentucky has about 100 parts of requests (1 MB each: 100 MB)
            std::collections::btree_map::Entry::Vacant(e) => e.insert(Writer::with_buffer(store, 256 << 10)?),
        };
        part.push(codec, row)
    }

    /// The parts, in the order of their key ranges.
    pub fn finish(self) -> io::Result<Vec<Rows>> {
        self.parts.into_values().map(Writer::finish).collect()
    }
}

/// The rows of the parts of [`Buckets::finish`], one part at a time, each sorted by `key` on all threads (not stable).
/// [`SortedParts::next_into`] fills a buffer of the caller: the buffer is used again for each part, so the memory is one
/// block of the largest part (a new vector for each part left the freed blocks of each size in the cache of the
/// allocator: France, a footprint of 11 GB for 1.4 GB of rows). The file of a part is gone when it is read.
#[cfg(feature = "pbf")]
pub struct SortedParts<'c, C, F> {
    parts: std::vec::IntoIter<Rows>,
    codec: &'c C,
    key: F,
}

#[cfg(feature = "pbf")]
impl<'c, C, F> SortedParts<'c, C, F> {
    pub fn new(parts: Vec<Rows>, codec: &'c C, key: F) -> Self {
        SortedParts { parts: parts.into_iter(), codec, key }
    }

    /// The next part, sorted, in `buf` (emptied first); false when there is no more part.
    pub fn next_into<T: Send, K: Ord + Send>(&mut self, buf: &mut Vec<T>) -> io::Result<bool>
    where
        C: Codec<T>,
        F: Fn(&T) -> K + Sync,
    {
        use rayon::slice::ParallelSliceMut;
        let Some(rows) = self.parts.next() else {
            return Ok(false);
        };
        buf.clear();
        buf.reserve(rows.len);
        for r in rows.read(self.codec)? {
            buf.push(r?);
        }
        let key = &self.key;
        buf.par_sort_unstable_by(|a, b| key(a).cmp(&key(b)));
        Ok(true)
    }
}

#[cfg(all(test, feature = "pbf"))]
mod bucket_tests {
    use super::*;

    #[test]
    fn buckets_give_all_rows_in_key_order() {
        let store = temp_store();
        let mut b = Buckets::new(4);
        let keys: Vec<i64> = (0..1000).map(|i| (i * 7919) % 1000 - 300).collect();
        for &k in &keys {
            b.push(&*store, k, &join::Request, &(k, 0, 0)).unwrap();
        }
        let mut parts = SortedParts::new(b.finish().unwrap(), &join::Request, |r: &(i64, u32, u32)| r.0);
        let (mut buf, mut got) = (Vec::new(), Vec::new());
        while parts.next_into(&mut buf).unwrap() {
            got.extend(buf.iter().map(|r| r.0));
        }
        let mut want = keys.clone();
        want.sort();
        assert_eq!(got, want);
    }
}

/// A line in few bytes, with the same bits again: when each point is on the grid of an OSM file (1e-7 degrees, [`join::e7`]),
/// a byte 1 and the steps from point to point as zigzag varints (about 3 bytes for each point, not 16); otherwise a byte 0
/// and the f64 values. [`line_at`] reads it.
pub fn put_line(g: &[(f64, f64)], out: &mut Vec<u8>) {
    let grid: Option<Vec<(i32, i32)>> = g.iter().map(|p| Some((join::e7(p.0)?, join::e7(p.1)?))).collect();
    let Some(grid) = grid else {
        out.push(0);
        for p in g {
            out.extend_from_slice(&p.0.to_bits().to_le_bytes());
            out.extend_from_slice(&p.1.to_bits().to_le_bytes());
        }
        return;
    };
    out.push(1);
    let mut put = |v: i64| {
        let mut v = ((v << 1) ^ (v >> 63)) as u64;
        while v >= 0x80 {
            out.push(v as u8 | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    };
    let mut last = (0i64, 0i64);
    for (x, y) in grid {
        put(x as i64 - last.0);
        put(y as i64 - last.1);
        last = (x as i64, y as i64);
    }
}

/// The line of [`put_line`]: `b` is all of its bytes.
pub fn line_at(b: &[u8]) -> Vec<(f64, f64)> {
    let f = |c: &[u8]| f64::from_bits(u64::from_le_bytes(c.try_into().expect("8 bytes")));
    if b[0] == 0 {
        return b[1..].chunks_exact(16).map(|c| (f(&c[..8]), f(&c[8..]))).collect();
    }
    let (mut at, mut out, mut last) = (1, Vec::new(), (0i64, 0i64));
    let get = |at: &mut usize| {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let c = b[*at];
            *at += 1;
            v |= ((c & 0x7f) as u64) << shift;
            if c < 0x80 {
                break;
            }
            shift += 7;
        }
        (v >> 1) as i64 ^ -((v & 1) as i64)
    };
    let n = |v: i64| 1e-9 * (100 * v) as f64;
    while at < b.len() {
        last.0 += get(&mut at);
        last.1 += get(&mut at);
        out.push((n(last.0), n(last.1)));
    }
    out
}

#[cfg(test)]
mod line_tests {
    #[test]
    fn a_line_gives_the_same_bits_again() {
        let osm = |n: i64| 1e-9 * (100 * n) as f64;
        for g in [
            vec![(osm(74_123_456), osm(437_380_000)), (osm(74_123_999), osm(437_379_000)), (osm(-1_799_999_999), osm(899_999_999))],
            vec![(7.41234561, 43.738), (7.5, 43.7)],
            vec![(osm(1), osm(2)), (0.1 + 0.2, osm(3))],
        ] {
            let mut b = Vec::new();
            super::put_line(&g, &mut b);
            let back = super::line_at(&b);
            assert_eq!(back.iter().map(|p| (p.0.to_bits(), p.1.to_bits())).collect::<Vec<_>>(), g.iter().map(|p| (p.0.to_bits(), p.1.to_bits())).collect::<Vec<_>>());
        }
    }
}

pub mod join {
    use super::{sort_bytes, Codec, Rows, Store, Writer};
    use std::io;

    fn i64_at(b: &[u8], at: usize) -> i64 {
        i64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
    }
    fn f64_at(b: &[u8], at: usize) -> f64 {
        f64::from_bits(u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes")))
    }
    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
    }

    /// A node of a way: (node id, number of the way, place in the way).
    pub struct Request;
    impl Codec<(i64, u32, u32)> for Request {
        fn put(&self, r: &(i64, u32, u32), out: &mut Vec<u8>) {
            out.extend_from_slice(&r.0.to_le_bytes());
            out.extend_from_slice(&r.1.to_le_bytes());
            out.extend_from_slice(&r.2.to_le_bytes());
        }
        fn get(&self, b: &mut &[u8]) -> (i64, u32, u32) {
            (i64_at(b, 0), u32_at(b, 8), u32_at(b, 12))
        }
    }

    /// The coordinate in units of 1e-7 degrees, when it gives the same bits again: the coordinates of an OSM file are
    /// `1e-9 * (100 * n)` (granularity 100, `pbfraw`).
    pub fn e7(x: f64) -> Option<i32> {
        let n = (x * 1e7).round();
        if !(n.abs() < 2_147_483_648.0) {
            return None;
        }
        let n = n as i32;
        ((1e-9 * (100 * n as i64) as f64).to_bits() == x.to_bits()).then_some(n)
    }

    /// lon and lat: two i32 of 1e-7 degrees when both give the same bits again (8 bytes), otherwise the two f64 (16 bytes).
    fn put_lon_lat(lon: f64, lat: f64, out: &mut Vec<u8>) {
        match (e7(lon), e7(lat)) {
            (Some(a), Some(b)) => {
                out.extend_from_slice(&a.to_le_bytes());
                out.extend_from_slice(&b.to_le_bytes());
            }
            _ => {
                out.extend_from_slice(&lon.to_bits().to_le_bytes());
                out.extend_from_slice(&lat.to_bits().to_le_bytes());
            }
        }
    }

    fn lon_lat_at(b: &[u8], at: usize) -> (f64, f64) {
        if b.len() - at == 8 {
            let n = |at| 1e-9 * (100 * i32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes")) as i64) as f64;
            (n(at), n(at + 4))
        } else {
            (f64_at(b, at), f64_at(b, at + 8))
        }
    }

    /// A node and its coordinates: (id, lon, lat). The coordinates keep their bits.
    pub struct Position;
    impl Codec<(i64, f64, f64)> for Position {
        fn put(&self, r: &(i64, f64, f64), out: &mut Vec<u8>) {
            out.extend_from_slice(&r.0.to_le_bytes());
            put_lon_lat(r.1, r.2, out);
        }
        fn get(&self, b: &mut &[u8]) -> (i64, f64, f64) {
            let (lon, lat) = lon_lat_at(b, 8);
            (i64_at(b, 0), lon, lat)
        }
    }

    /// A point of a way: (number of the way, place in the way, lon, lat). The coordinates keep their bits.
    pub struct Point;
    impl Codec<(u32, u32, f64, f64)> for Point {
        fn put(&self, r: &(u32, u32, f64, f64), out: &mut Vec<u8>) {
            out.extend_from_slice(&r.0.to_le_bytes());
            out.extend_from_slice(&r.1.to_le_bytes());
            put_lon_lat(r.2, r.3, out);
        }
        fn get(&self, b: &mut &[u8]) -> (u32, u32, f64, f64) {
            let (lon, lat) = lon_lat_at(b, 8);
            (u32_at(b, 0), u32_at(b, 4), lon, lat)
        }
    }

    /// The node id at the start of a request or a position.
    pub fn node_id(b: &[u8]) -> i64 {
        i64_at(b, 0)
    }

    /// The points of the ways of `requests` (in any order) from `positions` (sorted by node id, read again later), sorted by
    /// the number of the way and the place. A node with no coordinates gives no point.
    pub fn points(requests: Rows, positions: &mut Rows, budget: usize, store: &dyn Store) -> io::Result<Rows> {
        let requests = sort_bytes(requests, node_id, budget, store)?;
        let mut points = Writer::new(store)?;
        let mut pos = positions.read_ref(&Position)?.peekable();
        for r in requests.read(&Request)? {
            let (node, way, place) = r?;
            while pos.next_if(|p| p.as_ref().is_ok_and(|p| p.0 < node)).is_some() {}
            match pos.peek() {
                Some(Ok(p)) if p.0 == node => points.push(&Point, &(way, place, p.1, p.2))?,
                Some(Err(_)) => return Err(pos.next().expect("peeked").expect_err("an error")),
                _ => {}
            }
        }
        drop(pos);
        sort_by_way(points.finish()?, budget, store)
    }

    fn sort_by_way(points: Rows, budget: usize, store: &dyn Store) -> io::Result<Rows> {
        sort_bytes(points, |b| (u32_at(b, 0), u32_at(b, 4)), budget, store)
    }

    type RequestIter = std::iter::Peekable<Box<dyn Iterator<Item = io::Result<(i64, u32, u32)>>>>;

    /// The join of [`points`] without a file of the positions: the nodes come one at a time in the order of their ids, as
    /// the nodes of an area file sorted by type and id, and the requests, sorted by node, wait for them. The nodes of
    /// `extra` (in any order; the new nodes of the fixes) go in their place in the order of the ids.
    pub struct Merge {
        requests: RequestIter,
        points: Writer,
        extra: Vec<(i64, f64, f64)>,
        last: i64,
        in_order: bool,
    }

    impl Merge {
        pub fn new(requests: Rows, mut extra: Vec<(i64, f64, f64)>, budget: usize, store: &dyn Store) -> io::Result<Self> {
            let requests = sort_bytes(requests, node_id, budget, store)?;
            let requests: Box<dyn Iterator<Item = io::Result<(i64, u32, u32)>>> = Box::new(requests.read(&Request)?);
            extra.sort_unstable_by_key(|n| std::cmp::Reverse(n.0));
            Ok(Merge { requests: requests.peekable(), points: Writer::new(store)?, extra, last: i64::MIN, in_order: true })
        }

        /// The next node of the file.
        pub fn node(&mut self, id: i64, lon: f64, lat: f64) -> io::Result<()> {
            while self.extra.last().is_some_and(|n| n.0 < id) {
                let n = self.extra.pop().expect("a node");
                self.put(n.0, n.1, n.2)?;
            }
            self.put(id, lon, lat)
        }

        fn put(&mut self, id: i64, lon: f64, lat: f64) -> io::Result<()> {
            if !self.in_order {
                return Ok(());
            }
            if id <= self.last {
                self.in_order = false;
                return Ok(());
            }
            self.last = id;
            let r = &mut self.requests;
            while r.next_if(|r| r.as_ref().is_ok_and(|r| r.0 < id)).is_some() {}
            while let Some(x) = r.next_if(|r| r.as_ref().is_ok_and(|r| r.0 == id)) {
                let (_, way, place) = x?;
                self.points.push(&Point, &(way, place, lon, lat))?;
            }
            if let Some(Err(_)) = r.peek() {
                return Err(r.next().expect("peeked").expect_err("an error"));
            }
            Ok(())
        }

        /// The points, sorted by the number of the way and the place, or None when the nodes did not come in the order of
        /// their ids (then use [`points`] with a file of the positions).
        pub fn finish(mut self, budget: usize, store: &dyn Store) -> io::Result<Option<Rows>> {
            while let Some(n) = self.extra.pop() {
                self.put(n.0, n.1, n.2)?;
            }
            if !self.in_order {
                return Ok(None);
            }
            drop(self.requests);
            sort_by_way(self.points.finish()?, budget, store).map(Some)
        }
    }

    /// The node ids of one part of the requests of [`BucketJoin`]: 2^27 ids (France: about 100 parts).
    #[cfg(feature = "pbf")]
    pub const NODE_SHIFT: u32 = 27;
    /// The ways of one part of the points of [`BucketJoin`]: 2^20 ways (France: about 70 parts of about 13 million points).
    #[cfg(feature = "pbf")]
    pub const WAY_SHIFT: u32 = 20;

    #[cfg(feature = "pbf")]
    type RequestParts = super::SortedParts<'static, Request, fn(&(i64, u32, u32)) -> i64>;
    #[cfg(feature = "pbf")]
    type PointParts = super::SortedParts<'static, Point, fn(&(u32, u32, f64, f64)) -> (u32, u32)>;

    /// The join of [`Merge`] with buckets ([`super::Buckets`]): the requests are in parts by node id ([`NODE_SHIFT`]); when the
    /// nodes of the file reach a part, the part is sorted in memory on all threads and its file goes. The points go into parts
    /// by the number of the way ([`WAY_SHIFT`]). Each row is written once and read once, and no external sort merges on one
    /// thread; at a time, the disk holds the requests that wait and the points so far (about one row for each node of a way).
    #[cfg(feature = "pbf")]
    pub struct BucketJoin<'s> {
        store: &'s dyn Store,
        parts: RequestParts,
        buf: Vec<(i64, u32, u32)>,
        at: usize,
        done: bool,
        points: super::Buckets,
        extra: Vec<(i64, f64, f64)>,
        last: i64,
        in_order: bool,
    }

    #[cfg(feature = "pbf")]
    impl<'s> BucketJoin<'s> {
        /// `requests`: the parts of [`super::Buckets`] with the key `node id` and the shift [`NODE_SHIFT`]; `extra`: as in [`Merge`].
        pub fn new(requests: Vec<Rows>, mut extra: Vec<(i64, f64, f64)>, store: &'s dyn Store) -> Self {
            extra.sort_unstable_by_key(|n| std::cmp::Reverse(n.0));
            BucketJoin {
                store,
                parts: super::SortedParts::new(requests, &Request, |r| r.0),
                buf: Vec::new(),
                at: 0,
                done: false,
                points: super::Buckets::new(WAY_SHIFT),
                extra,
                last: i64::MIN,
                in_order: true,
            }
        }

        /// The next node of the file.
        pub fn node(&mut self, id: i64, lon: f64, lat: f64) -> io::Result<()> {
            while self.extra.last().is_some_and(|n| n.0 < id) {
                let n = self.extra.pop().expect("a node");
                self.put(n.0, n.1, n.2)?;
            }
            self.put(id, lon, lat)
        }

        fn put(&mut self, id: i64, lon: f64, lat: f64) -> io::Result<()> {
            if !self.in_order {
                return Ok(());
            }
            if id <= self.last {
                self.in_order = false;
                return Ok(());
            }
            self.last = id;
            loop {
                while self.at < self.buf.len() && self.buf[self.at].0 < id {
                    self.at += 1;
                }
                if self.at < self.buf.len() || self.done {
                    break;
                }
                // the part is used up: the next one (its file goes when it is read)
                self.at = 0;
                self.done = !self.parts.next_into(&mut self.buf)?;
            }
            while self.at < self.buf.len() && self.buf[self.at].0 == id {
                let (_, way, place) = self.buf[self.at];
                self.points.push(self.store, way as i64, &Point, &(way, place, lon, lat))?;
                self.at += 1;
            }
            Ok(())
        }

        /// The points in the order of the way and the place, or None when the nodes did not come in the order of their ids.
        pub fn finish(mut self) -> io::Result<Option<BucketPoints>> {
            while let Some(n) = self.extra.pop() {
                self.put(n.0, n.1, n.2)?;
            }
            if !self.in_order {
                return Ok(None);
            }
            drop(self.buf);
            Ok(Some(BucketPoints { parts: super::SortedParts::new(self.points.finish()?, &Point, |p| (p.0, p.1)), buf: Vec::new(), at: 0 }))
        }
    }

    /// The points of [`BucketJoin`], in the order of the way and the place: one part at a time, sorted in memory.
    #[cfg(feature = "pbf")]
    pub struct BucketPoints {
        parts: PointParts,
        buf: Vec<(u32, u32, f64, f64)>,
        at: usize,
    }

    #[cfg(feature = "pbf")]
    impl Iterator for BucketPoints {
        type Item = io::Result<(u32, u32, f64, f64)>;
        fn next(&mut self) -> Option<Self::Item> {
            while self.at == self.buf.len() {
                self.at = 0;
                match self.parts.next_into(&mut self.buf) {
                    Ok(true) => {}
                    Ok(false) => return None,
                    Err(e) => return Some(Err(e)),
                }
            }
            self.at += 1;
            Some(Ok(self.buf[self.at - 1]))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn rows<T>(codec: &impl Codec<T>, items: &[T], store: &dyn Store) -> Rows {
            let mut w = Writer::new(store).unwrap();
            for x in items {
                w.push(codec, x).unwrap();
            }
            w.finish().unwrap()
        }

        /// The join by buckets gives the points of the merge, in the same order, also over the ends of the parts.
        #[cfg(feature = "pbf")]
        #[test]
        fn bucket_join_equals_merge() {
            let store = super::super::temp_store();
            let store = &*store;
            // nodes 1 to 3000 apart by 2^25 (parts of 2^27 ids), ways of 3 to 6 nodes
            let nodes: Vec<(i64, f64, f64)> = (1..3000i64).map(|i| (i << 25, 7.0 + i as f64 * 1e-4, 43.0 + i as f64 * 1e-5)).collect();
            let mut reqs = Vec::new();
            for w in 0..2500u32 {
                for k in 0..(3 + w % 4) {
                    reqs.push((((w as i64 * 7 + k as i64 * 13) % 2999 + 1) << 25, w, k));
                }
            }
            let extra = vec![(5 << 25 | 1, 8.0, 44.0)];
            reqs.push((5 << 25 | 1, 1, 9));
            let mut m = Merge::new(rows(&Request, &reqs, store), extra.clone(), 1 << 10, store).unwrap();
            let mut b = super::super::Buckets::new(NODE_SHIFT);
            for r in &reqs {
                b.push(store, r.0, &Request, r).unwrap();
            }
            let mut j = BucketJoin::new(b.finish().unwrap(), extra, store);
            for n in &nodes {
                m.node(n.0, n.1, n.2).unwrap();
                j.node(n.0, n.1, n.2).unwrap();
            }
            let want: Vec<_> = m.finish(1 << 10, store).unwrap().unwrap().read(&Point).unwrap().map(|p| p.unwrap()).collect();
            let got: Vec<_> = j.finish().unwrap().unwrap().map(|p| p.unwrap()).collect();
            assert!(want.len() > 8000);
            assert_eq!(got, want);
        }

        /// The merge gives the points of the join with a file of the positions; nodes out of order give None.
        #[test]
        fn merge_equals_points() {
            let store = super::super::temp_store();
            let store = &*store;
            let nodes: Vec<(i64, f64, f64)> = (0..2000).map(|i| (i * 3, i as f64 * 0.5, -(i as f64))).collect();
            let extra = vec![(4, 9.0, 9.0), (-7, 1.0, 2.0), (99_999, 3.0, 4.0)];
            let mut reqs = Vec::new();
            for w in 0..50u32 {
                for k in 0..40u32 {
                    reqs.push((((w * 131 + k * 17) % 6100) as i64, w, k));
                }
            }
            reqs.extend([(4, 50, 0), (-7, 50, 1), (99_999, 50, 2), (5, 50, 3)]);
            let mut all: Vec<_> = nodes.iter().chain(&extra).copied().collect();
            all.sort_by_key(|n| n.0);
            let mut pos = rows(&Position, &all, store);
            let want = points(rows(&Request, &reqs, store), &mut pos, 1 << 10, store).unwrap();
            let want: Vec<_> = want.read(&Point).unwrap().map(Result::unwrap).collect();
            let mut m = Merge::new(rows(&Request, &reqs, store), extra.clone(), 1 << 10, store).unwrap();
            for &(id, lon, lat) in &nodes {
                m.node(id, lon, lat).unwrap();
            }
            let got = m.finish(1 << 10, store).unwrap().unwrap();
            let got: Vec<_> = got.read(&Point).unwrap().map(Result::unwrap).collect();
            assert!(want.len() > 500);
            assert_eq!(got, want);
            let mut m = Merge::new(rows(&Request, &reqs, store), Vec::new(), 1 << 10, store).unwrap();
            for id in [3, 9, 6] {
                m.node(id, 0.0, 0.0).unwrap();
            }
            assert!(m.finish(1 << 10, store).unwrap().is_none());
        }

        /// The coordinates come back with the same bits: those of an OSM file in 8 bytes, others in 16.
        #[test]
        fn coordinates_keep_their_bits() {
            let osm = |n: i64| 1e-9 * (100 * n) as f64;
            for (lon, lat, bytes) in [
                (osm(-1_234_567_891), osm(523_456_789), 16),
                (osm(1_800_000_000), osm(-900_000_000), 16),
                (osm(0), osm(-1), 16),
                (-0.0, 0.1, 24),
                (f64::NAN, 1.0, 24),
            ] {
                let mut out = Vec::new();
                Position.put(&(7, lon, lat), &mut out);
                assert_eq!(out.len(), bytes, "{lon} {lat}");
                let (id, a, b) = Position.get(&mut &out[..]);
                assert_eq!((id, a.to_bits(), b.to_bits()), (7, lon.to_bits(), lat.to_bits()));
                let mut out = Vec::new();
                Point.put(&(3, 4, lon, lat), &mut out);
                let (w, k, a, b) = Point.get(&mut &out[..]);
                assert_eq!((w, k, a.to_bits(), b.to_bits()), (3, 4, lon.to_bits(), lat.to_bits()));
            }
        }
    }
}
