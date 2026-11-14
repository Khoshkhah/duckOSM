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
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A temporary file: it is read from the start after it is written.
pub trait Blob: Read + Write + Seek + Send {}
impl<T: Read + Write + Seek + Send> Blob for T {}

/// Where the temporary files are made.
pub trait Store {
    fn create(&self) -> io::Result<Box<dyn Blob>>;
}

/// The files in memory (the tests, and the browser until it has a store in OPFS).
pub struct MemStore;

impl Store for MemStore {
    fn create(&self) -> io::Result<Box<dyn Blob>> {
        Ok(Box::new(io::Cursor::new(Vec::new())))
    }
}

/// The files in a folder. Each file is removed when it is dropped.
pub struct DirStore(pub PathBuf);

struct TempFile {
    f: std::fs::File,
    path: PathBuf,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Read for TempFile {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.f.read(b)
    }
}

impl Write for TempFile {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.f.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.f.flush()
    }
}

impl Seek for TempFile {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.f.seek(p)
    }
}

/// A maker of the store that [`temp_store`] gives on this thread.
pub type StoreMaker = std::rc::Rc<dyn Fn() -> Box<dyn Store>>;

thread_local! {
    static STORE: std::cell::RefCell<Option<StoreMaker>> = const { std::cell::RefCell::new(None) };
}

/// Set the store of the temporary files of the steps that run on this thread, or `None` for the default of
/// [`temp_store`]. The browser gives a store in OPFS, so that the temporary files are not in the memory of the tab.
pub fn set_temp_store(maker: Option<StoreMaker>) {
    STORE.with(|s| *s.borrow_mut() = maker);
}

/// The store of the network step: the store that [`set_temp_store`] gave on this thread, else a folder for temporary
/// files natively and memory in the browser.
pub fn temp_store() -> Box<dyn Store> {
    if let Some(maker) = STORE.with(|s| s.borrow().clone()) {
        return maker();
    }
    #[cfg(target_arch = "wasm32")]
    return Box::new(MemStore);
    #[cfg(not(target_arch = "wasm32"))]
    Box::new(DirStore(std::env::temp_dir()))
}

impl Store for DirStore {
    fn create(&self) -> io::Result<Box<dyn Blob>> {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = self.0.join(format!(
            "duckosm-spill-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        // Unix: the name goes at once and the open file stays, so the system frees the file also when the process is
        // stopped (a stopped France run left 45 GB). Windows cannot remove an open file: there, the drop removes it.
        #[cfg(unix)]
        let _ = std::fs::remove_file(&path);
        Ok(Box::new(TempFile { f, path }))
    }
}

/// How a row is written as bytes and read back. `put` can add to the codec (for example a new shared value), `get`
/// only reads it. A pass reads one file and writes the next with the same codec, so `put` takes `&self`.
pub trait Codec<T> {
    fn put(&self, row: &T, out: &mut Vec<u8>);
    fn get(&self, b: &mut &[u8]) -> T;
}

/// The rows of a temporary file: each row is its length (4 bytes) and its bytes.
pub struct Rows {
    blob: Box<dyn Blob>,
    pub len: usize,
    pub bytes: u64,
}

/// Writes rows to a new temporary file.
pub struct Writer {
    w: BufWriter<Box<dyn Blob>>,
    buf: Vec<u8>,
    len: usize,
    bytes: u64,
}

impl Writer {
    pub fn new(store: &dyn Store) -> io::Result<Self> {
        Ok(Writer {
            w: BufWriter::with_capacity(1 << 20, store.create()?),
            buf: Vec::new(),
            len: 0,
            bytes: 0,
        })
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
        self.w.write_all(&(b.len() as u32).to_le_bytes())?;
        self.w.write_all(b)?;
        self.len += 1;
        self.bytes += 4 + b.len() as u64;
        Ok(())
    }

    pub fn finish(self) -> io::Result<Rows> {
        let blob = self.w.into_inner().map_err(|e| e.into_error())?;
        Ok(Rows {
            blob,
            len: self.len,
            bytes: self.bytes,
        })
    }
}

/// Reads the bytes of each row from the start of a file.
pub struct RawIter<R = Box<dyn Blob>> {
    r: BufReader<R>,
    left: usize,
}

impl<R: Read> RawIter<R> {
    fn next_into(&mut self, buf: &mut Vec<u8>) -> io::Result<bool> {
        if self.left == 0 {
            return Ok(false);
        }
        let mut n = [0u8; 4];
        self.r.read_exact(&mut n)?;
        buf.resize(u32::from_le_bytes(n) as usize, 0);
        self.r.read_exact(buf)?;
        self.left -= 1;
        Ok(true)
    }

    /// Add the bytes of the next row at the end of `data`.
    fn append_into(&mut self, data: &mut Vec<u8>) -> io::Result<bool> {
        if self.left == 0 {
            return Ok(false);
        }
        let mut n = [0u8; 4];
        self.r.read_exact(&mut n)?;
        let start = data.len();
        data.resize(start + u32::from_le_bytes(n) as usize, 0);
        self.r.read_exact(&mut data[start..])?;
        self.left -= 1;
        Ok(true)
    }
}

impl Rows {
    fn raw(mut self) -> io::Result<RawIter> {
        self.blob.seek(SeekFrom::Start(0))?;
        Ok(RawIter {
            r: BufReader::with_capacity(1 << 20, self.blob),
            left: self.len,
        })
    }

    /// Read the rows, one at a time, and keep the file for a next read.
    pub fn read_ref<'a, T, C: Codec<T>>(
        &'a mut self,
        codec: &'a C,
    ) -> io::Result<impl Iterator<Item = io::Result<T>> + 'a> {
        self.blob.seek(SeekFrom::Start(0))?;
        let mut raw = RawIter {
            r: BufReader::with_capacity(1 << 20, &mut self.blob),
            left: self.len,
        };
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
    mut keep: impl FnMut(&[u8]) -> bool,
    key: impl Fn(&[u8]) -> K,
    budget: usize,
    store: &dyn Store,
) -> io::Result<Rows> {
    let mut raw = rows.raw()?;
    let mut runs: Vec<Rows> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    let mut index: Vec<(K, usize, usize)> = Vec::new(); // key, start and end of the row in `data`
    let flush = |data: &mut Vec<u8>, index: &mut Vec<(K, usize, usize)>, runs: &mut Vec<Rows>| -> io::Result<()> {
        // stable; on all threads when the crate has rayon (the feature `pbf`)
        #[cfg(feature = "pbf")]
        {
            use rayon::slice::ParallelSliceMut;
            index.par_sort_by(|a, b| a.0.cmp(&b.0));
        }
        #[cfg(not(feature = "pbf"))]
        index.sort_by(|a, b| a.0.cmp(&b.0));
        let mut w = Writer::new(store)?;
        for (_, a, b) in index.drain(..) {
            w.push_bytes(&data[a..b])?;
        }
        data.clear();
        runs.push(w.finish()?);
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
                runs.push(merge(all, &key, store)?);
            }
        }
    }
    drop(raw);
    if runs.is_empty() {
        // one block: no merge
        flush(&mut data, &mut index, &mut runs)?;
        return Ok(runs.pop().unwrap());
    }
    if !index.is_empty() {
        flush(&mut data, &mut index, &mut runs)?;
    }
    drop((data, index));
    merge(runs, &key, store)
}

/// The runs that a merge of [`sort_bytes`] reads at once.
const MERGE_FAN: usize = 64;

/// The merge of sorted runs: the smallest (key, run) first; a run is in the order of the input, so the result is stable.
fn merge<K: Ord>(runs: Vec<Rows>, key: &impl Fn(&[u8]) -> K, store: &dyn Store) -> io::Result<Rows> {
    let mut its: Vec<RawIter> = runs.into_iter().map(Rows::raw).collect::<io::Result<_>>()?;
    let mut heap = BinaryHeap::new();
    let mut heads: Vec<Vec<u8>> = vec![Vec::new(); its.len()];
    for (i, it) in its.iter_mut().enumerate() {
        if it.next_into(&mut heads[i])? {
            heap.push(Reverse((key(&heads[i]), i)));
        }
    }
    let mut w = Writer::new(store)?;
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
            std::collections::btree_map::Entry::Vacant(e) => e.insert(Writer::new(store)?),
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
    fn e7(x: f64) -> Option<i32> {
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
