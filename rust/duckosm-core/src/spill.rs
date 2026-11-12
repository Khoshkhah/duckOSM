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

/// The store of the network step: a folder for temporary files natively, memory in the browser (WebAssembly has no
/// files until the page gives a store in OPFS).
pub fn temp_store() -> Box<dyn Store> {
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
pub struct RawIter {
    r: BufReader<Box<dyn Blob>>,
    left: usize,
}

impl RawIter {
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
}

impl Rows {
    fn raw(mut self) -> io::Result<RawIter> {
        self.blob.seek(SeekFrom::Start(0))?;
        Ok(RawIter {
            r: BufReader::with_capacity(1 << 20, self.blob),
            left: self.len,
        })
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

/// The rows of `rows` in the order of `key` (stable). Each sorted block holds rows of at most `budget` bytes (the
/// bytes in the file; in memory a row also has its key and about 32 bytes more).
pub fn sort<T, K: Ord, C: Codec<T>>(
    rows: Rows,
    codec: &C,
    key: impl Fn(&T) -> K,
    budget: usize,
    store: &dyn Store,
) -> io::Result<Rows> {
    let total = rows.len;
    let mut raw = rows.raw()?;
    let mut runs: Vec<Rows> = Vec::new();
    let mut block: Vec<(K, Vec<u8>)> = Vec::new();
    let mut size = 0usize;
    let flush = |block: &mut Vec<(K, Vec<u8>)>, runs: &mut Vec<Rows>| -> io::Result<()> {
        block.sort_by(|a, b| a.0.cmp(&b.0)); // stable
        let mut w = Writer::new(store)?;
        for (_, b) in block.drain(..) {
            w.push_bytes(&b)?;
        }
        runs.push(w.finish()?);
        Ok(())
    };
    let mut buf = Vec::new();
    while raw.next_into(&mut buf)? {
        size += buf.len() + 4;
        block.push((key(&codec.get(&mut &buf[..])), std::mem::take(&mut buf)));
        if size >= budget {
            flush(&mut block, &mut runs)?;
            size = 0;
        }
    }
    drop(raw);
    if runs.is_empty() {
        // one block: no merge
        flush(&mut block, &mut runs)?;
        return Ok(runs.pop().unwrap());
    }
    if !block.is_empty() {
        flush(&mut block, &mut runs)?;
    }
    // the merge: the smallest (key, run) first; a run is in the order of the input, so the result is stable
    let mut its: Vec<RawIter> = runs.into_iter().map(Rows::raw).collect::<io::Result<_>>()?;
    let mut heap = BinaryHeap::new();
    let mut heads: Vec<Vec<u8>> = vec![Vec::new(); its.len()];
    for (i, it) in its.iter_mut().enumerate() {
        if it.next_into(&mut heads[i])? {
            heap.push(Reverse((key(&codec.get(&mut &heads[i][..])), i)));
        }
    }
    let mut w = Writer::new(store)?;
    while let Some(Reverse((_, i))) = heap.pop() {
        w.push_bytes(&heads[i])?;
        if its[i].next_into(&mut heads[i])? {
            heap.push(Reverse((key(&codec.get(&mut &heads[i][..])), i)));
        }
    }
    let out = w.finish()?;
    debug_assert_eq!(out.len, total);
    Ok(out)
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
        let rows = sort(w.finish().unwrap(), &c, |e: &Edge| e.edge_id, 1000, &store).unwrap();
        assert_eq!(rows.read(&c).unwrap().count(), 100);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir(&dir).unwrap();
    }
}
