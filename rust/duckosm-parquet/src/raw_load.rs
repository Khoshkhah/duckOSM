//! The load step (docs/design/streaming_load.md): duckOSM's `raw.nodes`, `raw.ways` and `raw.relations` from a
//! `.osm.pbf` file, written as Parquet while the file is read, in a memory budget. The memory does not grow with the
//! size of the file: only the blocks of one round and one open row group of each table are in memory.

use super::{err, ROW_GROUP};
use arrow_array::builder::{BinaryBuilder, Float64Builder, Int64Builder, ListBuilder, MapBuilder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use duckosm_core::graph::MemberType;
use duckosm_core::raw::RawSink;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::io::Write;
use std::sync::Arc;

/// The three tables, in the order of [`RawPart::batches`]. [`load_raw`] writes each table as files
/// `<table>_<shard>.parquet` (one shard for each thread), so DuckDB reads `<table>_*.parquet`.
pub const RAW_TABLES: [&str; 3] = ["raw_nodes", "raw_ways", "raw_relations"];

type Tags = MapBuilder<StringBuilder, StringBuilder>;

/// The rows of one block as Arrow columns. All strings of a column are in one buffer, so a tag costs its bytes and
/// two offsets, not two allocations.
pub struct RawColumns {
    n_id: Int64Builder,
    n_lat: Float64Builder,
    n_lon: Float64Builder,
    n_tags: Tags,
    w_id: Int64Builder,
    w_tags: Tags,
    w_refs: ListBuilder<Int64Builder>,
    r_id: Int64Builder,
    r_tags: Tags,
    r_refs: ListBuilder<Int64Builder>,
    r_roles: ListBuilder<StringBuilder>,
    r_types: ListBuilder<StringBuilder>,
    refs: Vec<i64>,
    /// The points and the WKB of the line of one way, kept for the next way: a new list for each way took the lock of the
    /// global allocator of the browser build millions of times.
    pts: Vec<duckosm_core::geom::Pt>,
    wkb: Vec<u8>,
    /// The tables to fill (nodes, ways, relations); the elements of the others are skipped.
    keep: [bool; 3],
    /// The column `line` of the ways, from these coordinates (see [`write_area_raw`]).
    line: Option<(Arc<NodeParts>, BinaryBuilder)>,
}

/// The tags as a DuckDB `MAP(VARCHAR, VARCHAR)`; no tags: NULL, as `ST_READOSM`.
fn tags<'a>(b: &mut Tags, it: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
    let mut any = false;
    for (k, v) in it {
        b.keys().append_value(k);
        b.values().append_value(v);
        any = true;
    }
    b.append(any)
        .expect("the keys and the values have the same length");
}

impl Default for RawColumns {
    fn default() -> Self {
        let map = || MapBuilder::new(None, StringBuilder::new(), StringBuilder::new());
        RawColumns {
            n_id: Int64Builder::new(),
            n_lat: Float64Builder::new(),
            n_lon: Float64Builder::new(),
            n_tags: map(),
            w_id: Int64Builder::new(),
            w_tags: map(),
            w_refs: ListBuilder::new(Int64Builder::new()),
            r_id: Int64Builder::new(),
            r_tags: map(),
            r_refs: ListBuilder::new(Int64Builder::new()),
            r_roles: ListBuilder::new(StringBuilder::new()),
            r_types: ListBuilder::new(StringBuilder::new()),
            refs: Vec::new(),
            pts: Vec::new(),
            wkb: Vec::new(),
            keep: [true; 3],
            line: None,
        }
    }
}

impl RawColumns {
    /// Columns for only one table (0 nodes, 1 ways, 2 relations): a pass that makes one table builds nothing else.
    pub fn only(table: usize) -> Self {
        let mut c = RawColumns::default();
        c.keep = [table == 0, table == 1, table == 2];
        c
    }

    /// Columns for the ways and the relations, without the column `line` (see [`write_area_raw`], which adds it).
    pub fn ways_and_relations() -> Self {
        let mut c = RawColumns::default();
        c.keep = [false, true, true];
        c
    }

    /// Columns for the ways and the relations, and the column `line` of the ways from `coords`.
    pub fn lines(coords: Arc<NodeParts>) -> Self {
        let mut c = RawColumns::default();
        c.keep = [false, true, true];
        c.line = Some((coords, BinaryBuilder::new()));
        c
    }
}

impl RawSink for RawColumns {
    fn node<'a>(
        &mut self,
        id: i64,
        lat: f64,
        lon: f64,
        t: &mut dyn Iterator<Item = (&'a str, &'a str)>,
    ) {
        if !self.keep[0] {
            return;
        }
        self.n_id.append_value(id);
        self.n_lat.append_value(lat);
        self.n_lon.append_value(lon);
        tags(&mut self.n_tags, t);
    }

    fn way<'a>(
        &mut self,
        id: i64,
        t: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        refs: &mut dyn Iterator<Item = i64>,
    ) {
        if !self.keep[1] {
            return;
        }
        self.refs.clear();
        self.refs.extend(refs);
        // duckOSM's raw.ways has only the ways with 2 or more nodes
        if self.refs.len() < 2 {
            return;
        }
        self.w_id.append_value(id);
        tags(&mut self.w_tags, t);
        self.w_refs.values().append_slice(&self.refs);
        self.w_refs.append(true);
        if let Some((coords, line)) = &mut self.line {
            // as write_raw_ways_to: a node that has no coordinates is left out; fewer than 2 points: NULL
            self.pts.clear();
            self.pts.extend(self.refs.iter().filter_map(|r| coords.get(r).copied()));
            if self.pts.len() >= 2 {
                super::line_wkb_into(&self.pts, &mut self.wkb);
                line.append_value(&self.wkb);
            } else {
                line.append_null();
            }
        }
    }

    fn relation<'a>(
        &mut self,
        id: i64,
        t: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        if !self.keep[2] {
            return;
        }
        self.r_id.append_value(id);
        tags(&mut self.r_tags, t);
        for (m, ty, role) in members {
            self.r_refs.values().append_value(m);
            // no role: NULL, as ST_READOSM
            self.r_roles
                .values()
                .append_option((!role.is_empty()).then_some(role));
            self.r_types.values().append_value(match ty {
                MemberType::Node => "node",
                MemberType::Way => "way",
                MemberType::Relation => "relation",
            });
        }
        self.r_refs.append(true);
        self.r_roles.append(true);
        self.r_types.append(true);
    }
}

/// The rows of one block: one batch for each table (a batch can have 0 rows).
pub struct RawPart {
    pub batches: [RecordBatch; 3],
}

impl RawPart {
    /// The memory of the Arrow buffers of the part.
    pub fn bytes(&self) -> usize {
        self.batches.iter().map(|b| b.get_array_memory_size()).sum()
    }
}

impl RawColumns {
    /// The three batches, with the column names and types of the `raw_*.parquet` files of duckOSM (as
    /// `write_raw_nodes_to` and the others, without the `line` column of the ways).
    pub fn finish(mut self) -> Result<RawPart, String> {
        let a = |x: &mut dyn arrow_array::builder::ArrayBuilder| -> ArrayRef { x.finish() };
        let nodes = RecordBatch::try_from_iter_with_nullable(vec![
            ("osm_id", a(&mut self.n_id), false),
            ("lat", a(&mut self.n_lat), false),
            ("lon", a(&mut self.n_lon), false),
            ("tags", Arc::new(self.n_tags.finish()) as ArrayRef, true),
        ])
        .map_err(err)?;
        let mut way_cols = vec![
            ("osm_id", a(&mut self.w_id), false),
            ("tags", Arc::new(self.w_tags.finish()) as ArrayRef, true),
            ("refs", Arc::new(self.w_refs.finish()), false),
        ];
        if let Some((_, line)) = &mut self.line {
            way_cols.push(("line", Arc::new(line.finish()), true));
        }
        let ways = RecordBatch::try_from_iter_with_nullable(way_cols).map_err(err)?;
        let relations = RecordBatch::try_from_iter_with_nullable(vec![
            ("osm_id", a(&mut self.r_id), false),
            ("tags", Arc::new(self.r_tags.finish()) as ArrayRef, true),
            ("refs", Arc::new(self.r_refs.finish()), false),
            ("ref_roles", Arc::new(self.r_roles.finish()), false),
            ("ref_types", Arc::new(self.r_types.finish()), false),
        ])
        .map_err(err)?;
        Ok(RawPart {
            batches: [nodes, ways, relations],
        })
    }
}

/// The properties of the `raw_*` files. They are temporary (DuckDB copies them into `raw.*`, then they are deleted), so
/// they have no compression: zstd on one thread was most of the time of the step, and DuckDB had to decompress it again
/// (France: 182 s with zstd). The plain encodings are also the fastest for DuckDB to read (DELTA_BINARY_PACKED and
/// BYTE_STREAM_SPLIT made the copy of the nodes 116 s, not 47 s).
fn raw_props() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(parquet::basic::Compression::UNCOMPRESSED)
        .set_max_row_group_row_count(Some(ROW_GROUP))
        .build()
}

/// The three Parquet files, written part by part. A table closes its row group when the row group holds
/// `group_bytes` in memory (or [`ROW_GROUP`] rows), so each writer holds at most about `group_bytes`.
pub struct RawWriter<W: Write + Send> {
    writers: Vec<ArrowWriter<W>>,
    group_bytes: usize,
    rows: [usize; 3],
}

impl<W: Write + Send> RawWriter<W> {
    pub fn new(files: [W; 3], group_bytes: usize) -> Result<Self, String> {
        Self::with_columns(files, group_bytes, RawColumns::default())
    }

    /// The writer of the files of `empty` (with or without the column `line`).
    fn with_columns(files: [W; 3], group_bytes: usize, empty: RawColumns) -> Result<Self, String> {
        let empty = empty.finish()?;
        let props = raw_props();
        let writers = files
            .into_iter()
            .zip(&empty.batches)
            .map(|(f, b)| ArrowWriter::try_new(f, b.schema(), Some(props.clone())).map_err(err))
            .collect::<Result<_, _>>()?;
        Ok(RawWriter {
            writers,
            group_bytes: group_bytes.max(1),
            rows: [0; 3],
        })
    }

    pub fn write(&mut self, part: RawPart) -> Result<(), String> {
        for (i, b) in part.batches.into_iter().enumerate() {
            if b.num_rows() == 0 {
                continue;
            }
            self.rows[i] += b.num_rows();
            let w = &mut self.writers[i];
            w.write(&b).map_err(err)?;
            drop(b);
            if w.memory_size() >= self.group_bytes {
                w.flush().map_err(err)?;
            }
        }
        Ok(())
    }

    /// The memory that the writers hold now (the open row groups).
    pub fn memory(&self) -> usize {
        self.writers.iter().map(|w| w.memory_size()).sum()
    }

    /// Close the files. Returns the rows of each table.
    pub fn finish(self) -> Result<[usize; 3], String> {
        for w in self.writers {
            w.close().map_err(err)?;
        }
        Ok(self.rows)
    }
}

/// A memory size as DuckDB writes it: `1GB` (10^9 bytes), `512MB`, `2GiB` (2^31 bytes), `100KB`, or a number of
/// bytes.
pub fn parse_memory(s: &str) -> Result<usize, String> {
    let t = s.trim();
    let split = t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let num: f64 = num.trim().parse().map_err(|_| {
        format!("the memory size '{s}' is not a number with a unit (for example 2GB)")
    })?;
    let m: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" | "bytes" => 1.0,
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024f64.powi(2),
        "gib" => 1024f64.powi(3),
        "tib" => 1024f64.powi(4),
        u => return Err(format!("the memory size '{s}' has an unknown unit '{u}'")),
    };
    Ok((num * m) as usize)
}

/// What the load did.
#[derive(Debug, Default, Clone)]
pub struct LoadSummary {
    /// The rows of `raw.nodes`, `raw.ways` and `raw.relations`.
    pub rows: [usize; 3],
    pub blocks: usize,
    /// The largest ratio of the Arrow bytes of a part to the size of its block in the file, and the most blocks that
    /// one round decoded.
    pub ratio: f64,
    pub most_in_flight: usize,
}

/// The share of the budget for the blocks in work. The rest is for the store of the rows: the open row groups of the
/// Parquet shards, or DuckDB.
pub const BLOCKS_SHARE: f64 = 0.5;
/// The memory of one block in work, as a multiple of its Arrow part: the decompressed block and the builders (they
/// grow by doubling) are in memory while the part is made. Measured: about 3.2 for the blocks of Monaco.
const BLOCK_COST: f64 = 4.0;

/// Decode all blocks of a `.osm.pbf` file (`reader`) into parts on `threads` threads, and give each part to `consume`
/// on the thread that made it, so the parts are also stored in parallel. The blocks in work use at most about
/// `blocks_budget` bytes. Returns the number of blocks, the largest ratio and the most blocks of one round.
///
/// The memory of a block in work is about `BLOCK_COST` times its Arrow part, and the part is about `ratio` times the
/// size of the block in the file. The size in the file is known before the block is decoded, so a round takes the
/// blocks that fit in the budget with the largest `ratio` so far. The first round has one block (no ratio yet). The
/// budget cannot be less than one block: a round always takes one block.
#[cfg(feature = "pbf")]
pub fn load_parts<R: std::io::Read + Send>(
    reader: R,
    blocks_budget: usize,
    threads: usize,
    consume: impl Fn(RawPart) -> Result<(), String> + Sync,
) -> Result<(usize, f64, usize), String> {
    use std::cell::Cell;
    let threads = threads.max(1);
    let ratio = Cell::new(0.0f64);
    let most = Cell::new(1usize);
    let cost = |size: usize| size as f64 * (1.0 + BLOCK_COST * ratio.get());
    let blocks = duckosm_core::pbf::stream_raw(
        reader,
        |size, round| {
            let ok = ratio.get() > 0.0
                && round.len() < 2 * threads
                && round.iter().map(|&s| cost(s)).sum::<f64>() + cost(size) <= blocks_budget as f64;
            if ok {
                most.set(most.get().max(round.len() + 1));
            }
            ok
        },
        RawColumns::default,
        |c: RawColumns| {
            let part = c.finish()?;
            let bytes = if part.batches.iter().any(|b| b.num_rows() > 0) {
                part.bytes()
            } else {
                0
            };
            consume(part)?;
            Ok(bytes)
        },
        |bytes, size| {
            if bytes > 0 && size > 0 {
                ratio.set(ratio.get().max(bytes as f64 / size as f64));
            }
            Ok(())
        },
    )?;
    Ok((blocks, ratio.get(), most.get()))
}

/// Give `part` to the first of `slots` that no other thread uses now, and run `f` on it. With at least as many slots
/// as threads, a thread always finds a free slot.
pub fn with_free_slot<S, T>(
    slots: &[std::sync::Mutex<S>],
    f: impl FnOnce(&mut S) -> Result<T, String>,
) -> Result<T, String> {
    loop {
        for s in slots {
            if let Ok(mut s) = s.try_lock() {
                return f(&mut s);
            }
        }
        std::thread::yield_now();
    }
}

/// Load duckOSM's `raw.*` tables from a `.osm.pbf` file (`reader`) into Parquet files, with at most about `budget`
/// bytes of memory and `threads` threads ([`load_parts`]). Each thread writes the parts that it decodes into a shard
/// of its own: three files `<table>_<shard>.parquet` (each from `file`), so all threads write at the same time. The
/// rows of a table are the rows of all its files.
#[cfg(feature = "pbf")]
pub fn load_raw<R, W>(
    reader: R,
    budget: usize,
    threads: usize,
    mut file: impl FnMut(&str) -> Result<W, String>,
) -> Result<LoadSummary, String>
where
    R: std::io::Read + Send,
    W: Write + Send,
{
    use std::sync::Mutex;
    let threads = threads.max(1);
    let blocks_budget = (budget as f64 * BLOCKS_SHARE) as usize;
    let group_bytes =
        (budget as f64 * (1.0 - BLOCKS_SHARE)) as usize / (2 * RAW_TABLES.len() * threads);
    let mut writers = Vec::with_capacity(threads);
    for i in 0..threads {
        let f = |t: &str| format!("{t}_{i:03}.parquet");
        let files = [
            file(&f(RAW_TABLES[0]))?,
            file(&f(RAW_TABLES[1]))?,
            file(&f(RAW_TABLES[2]))?,
        ];
        writers.push(Mutex::new(RawWriter::new(files, group_bytes)?));
    }
    let (blocks, ratio, most_in_flight) = load_parts(reader, blocks_budget, threads, |part| {
        with_free_slot(&writers, |w| w.write(part))
    })?;
    let mut rows = [0; 3];
    for w in writers {
        let r = w.into_inner().map_err(err)?.finish()?;
        for (a, b) in rows.iter_mut().zip(r) {
            *a += b;
        }
    }
    Ok(LoadSummary {
        rows,
        blocks,
        ratio,
        most_in_flight,
    })
}

/// [`load_raw`] for a reader that only the calling thread may read and files that only the calling thread may write
/// (in the browser, a file in OPFS belongs to the thread that opened it). The other threads decode the blocks into
/// parts; the calling thread reads the blocks and writes the parts in the order of the file, one file for each table
/// (`files`: nodes, ways, relations). In memory: the blocks of one batch (two for each thread) and their parts, and
/// one open row group of about `group_bytes` for each table. Gives the rows of each table.
#[cfg(feature = "pbf")]
pub fn load_raw_here<R: std::io::Read, W: Write + Send>(
    mut reader: R,
    files: [W; 3],
    group_bytes: usize,
) -> Result<[usize; 3], String> {
    use duckosm_core::pbfraw::{decode_block, read_block, Buffers};
    use rayon::prelude::*;
    let mut w = RawWriter::new(files, group_bytes)?;
    let n = 2 * rayon::current_num_threads();
    loop {
        let mut batch = Vec::with_capacity(n);
        while batch.len() < n {
            match read_block(&mut reader)? {
                Some(b) => batch.push(b),
                None => break,
            }
        }
        if batch.is_empty() {
            break;
        }
        let parts: Vec<RawPart> = batch
            .par_iter()
            .map_init(Buffers::default, |buf, b| -> Result<RawPart, String> {
                let mut c = RawColumns::default();
                decode_block(b, [true; 3], buf, &mut c)?;
                c.finish()
            })
            .collect::<Result<_, _>>()?;
        for p in parts {
            w.write(p)?;
        }
    }
    w.finish()
}

/// The coordinates of the nodes of a file, in the parts of its blocks, each sorted by id and the parts in the order of the
/// ids (a file sorted by type and id). A look-up searches the part, then the node in it.
#[derive(Default)]
pub struct NodeParts(Vec<Vec<(i64, duckosm_core::geom::Pt)>>);

impl NodeParts {
    /// Parts that are not in the order of the ids become one sorted part.
    pub fn new(mut parts: Vec<Vec<(i64, duckosm_core::geom::Pt)>>) -> NodeParts {
        parts.retain(|p| !p.is_empty());
        let sorted = parts.iter().all(|p| p.windows(2).all(|w| w[0].0 < w[1].0))
            && parts.windows(2).all(|w| w[0].last().map(|x| x.0) < w[1].first().map(|x| x.0));
        if !sorted {
            let mut all: Vec<_> = parts.into_iter().flatten().collect();
            all.sort_unstable_by_key(|x| x.0);
            all.dedup_by_key(|x| x.0);
            parts = vec![all];
        }
        NodeParts(parts)
    }

    pub fn get(&self, id: &i64) -> Option<&duckosm_core::geom::Pt> {
        let k = self.0.partition_point(|p| p[0].0 <= *id).checked_sub(1)?;
        let part = &self.0[k];
        part.binary_search_by_key(id, |x| x.0).ok().map(|i| &part[i].1)
    }
}

/// duckOSM's `raw.*` of an area file (the output of step 0) with the changes of the build (`patch`: the OSM fixes, the
/// tags of the network's ways), as the `raw_*.parquet` files of the area build (with the column `line` of the ways, as
/// `write_raw_ways_to`). Three passes read the file, on the calling thread: the nodes, the nodes of the ways, then the
/// ways and the relations; all threads decode and patch. The lines come from a join on disk (temporary files of
/// `spill::temp_store`): the nodes of the ways and the coordinates, both sorted by node id, give the points of each way
/// in the order of the ways. In memory: the sorts (64 MiB each), one batch of blocks and one open row group of each
/// table. Gives the rows of each table; for the ways, also those with fewer than 2 nodes, which
/// are not written (as `Raw` counts them).
#[cfg(feature = "pbf")]
pub fn write_area_raw<R: std::io::Read, W: Write + Send>(
    open: impl Fn() -> Result<R, String>,
    patch: &duckosm_core::raw::RawPatch,
    files: [W; 3],
    group_bytes: usize,
) -> Result<[usize; 3], String> {
    use duckosm_core::cutstream::each_block;
    use duckosm_core::geom::Pt;
    use duckosm_core::pbfraw::decode_block;
    use duckosm_core::raw::{sync_way, Patched, RawTags};
    fn pairs(t: &RawTags) -> Vec<(&str, &str)> {
        t.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
    }
    struct Patching<'p> {
        cols: RawColumns,
        patch: &'p duckosm_core::raw::RawPatch<'p>,
        coords: Vec<(i64, Pt)>,
        ways: usize,
        /// the tags of a way of the network, kept for the next one (see RawColumns::pts)
        tags: RawTags,
    }
    impl RawSink for Patching<'_> {
        fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
            match self.patch.node(id) {
                Patched::Same => self.cols.node(id, lat, lon, t),
                Patched::Removed => return,
                Patched::Fixed(n) => {
                    self.cols.node(id, lat, lon, &mut n.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                }
            }
            self.coords.push((id, (lon, lat)));
        }
        fn way<'a>(
            &mut self,
            id: i64,
            t: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            refs: &mut dyn Iterator<Item = i64>,
        ) {
            let net = self.patch.net_way(id);
            let (mut tags, fixed): (RawTags, _) = match self.patch.way(id) {
                Patched::Removed => return,
                Patched::Fixed(w) => (w.tags.clone(), Some(w)),
                Patched::Same if net.is_none() => {
                    self.ways += 1;
                    return self.cols.way(id, t, refs);
                }
                Patched::Same => {
                    let mut b = std::mem::take(&mut self.tags);
                    b.clear();
                    b.extend(t.map(|(k, v)| (k.to_string(), v.to_string())));
                    (b, None)
                }
            };
            self.ways += 1;
            if let Some(n) = net {
                sync_way(&mut tags, n);
            }
            let mut t = tags.iter().map(|(k, v)| (k.as_str(), v.as_str()));
            match fixed {
                Some(w) => self.cols.way(id, &mut t, &mut w.refs.iter().copied()),
                None => self.cols.way(id, &mut t, refs),
            }
            self.tags = tags;
        }
        fn relation<'a>(
            &mut self,
            id: i64,
            t: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
        ) {
            match self.patch.relation(id) {
                Patched::Same => self.cols.relation(id, t, members),
                Patched::Removed => {}
                Patched::Fixed(r) => self.cols.relation(
                    id,
                    &mut r.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                    &mut r.members.iter().map(|(m, ty, role)| (*m, *ty, role.as_str())),
                ),
            }
        }
    }
    use arrow_array::cast::AsArray;
    use duckosm_core::spill::{self, join, Writer};
    const BUDGET: usize = 64 << 20; // each sort holds about this much (spill::sort_bytes)
    let io = |e: std::io::Error| format!("a temporary file of raw.*: {e}");
    let store = spill::temp_store();
    let store = &*store;
    let mut w = RawWriter::with_columns(files, group_bytes, RawColumns::lines(Arc::new(NodeParts::default())))?;
    let mut rows = [0usize; 3];
    // pass 1: the nodes of each way that is written, with the number of the way (the order of the ways) and the place in it
    let requests = || -> Result<spill::Rows, String> {
        let mut requests = Writer::new(store).map_err(io)?;
        let mut number = 0u32;
        let mut ask = |ways: &RecordBatch, requests: &mut Writer| -> Result<(), String> {
            let refs = ways.column(2).as_list::<i32>();
            for i in 0..ways.num_rows() {
                let ids = refs.value(i);
                for (k, id) in ids.as_primitive::<arrow_array::types::Int64Type>().values().iter().enumerate() {
                    requests.push(&join::Request, &(*id, number, k as u32)).map_err(io)?;
                }
                number = number.checked_add(1).ok_or("raw.*: more than 2^32 ways")?;
            }
            Ok(())
        };
        each_block(
            &mut open()?,
            |b, buf| {
                let mut s = Patching { cols: RawColumns::only(1), patch, coords: Vec::new(), ways: 0, tags: Vec::new() };
                decode_block(b, [false, true, false], buf, &mut s)?;
                s.cols.finish()
            },
            |part, _| {
                ask(&part.batches[1], &mut requests)?;
                Ok(true)
            },
        )?;
        let mut c = RawColumns::only(1);
        for nw in patch.new_ways() {
            c.way(nw.id, &mut std::iter::empty(), &mut nw.refs.iter().copied());
        }
        ask(&c.finish()?.batches[1], &mut requests)?;
        requests.finish().map_err(io)
    };
    // the nodes of the file, then the new nodes of the fixes
    let each_node = |table: bool, f: &mut dyn FnMut(Option<RawPart>, &[(i64, Pt)], bool) -> Result<(), String>| {
        each_block(
            &mut open()?,
            |b, buf| {
                let mut s = Patching { cols: RawColumns::only(0), patch, coords: Vec::new(), ways: 0, tags: Vec::new() };
                decode_block(b, [true, false, false], buf, &mut s)?;
                Ok((if table { Some(s.cols.finish()?) } else { None }, s.coords))
            },
            |(part, c), _| {
                f(part, &c, false)?;
                Ok(true)
            },
        )?;
        let mut s = Patching { cols: RawColumns::only(0), patch, coords: Vec::new(), ways: 0, tags: Vec::new() };
        for n in patch.new_nodes() {
            s.cols.node(n.id, n.lat, n.lon, &mut pairs(&n.tags).into_iter());
            s.coords.push((n.id, (n.lon, n.lat)));
        }
        f(if table { Some(s.cols.finish()?) } else { None }, &s.coords, true)
    };
    // pass 2: the nodes, and the join of the requests with them as they come (the nodes of a file sorted by type and id come in
    // the order of the ids), with no file of the positions (that was 20.5 million rows on Kentucky)
    let new: Vec<_> = patch.new_nodes().map(|n| (n.id, n.lon, n.lat)).collect();
    let mut merge = join::Merge::new(requests()?, new, BUDGET, store).map_err(io)?;
    each_node(true, &mut |part, c, new| {
        let part = part.expect("the table");
        rows[0] += part.batches[0].num_rows();
        w.write(part)?;
        for &(id, (lon, lat)) in c.iter().filter(|_| !new) {
            merge.node(id, lon, lat).map_err(io)?;
        }
        Ok(())
    })?;
    // the join: the points of the ways, in the order of the ways; a node with no coordinates gives no point (as before: it is left
    // out of the line)
    let points = match merge.finish(BUDGET, store).map_err(io)? {
        Some(points) => points,
        None => {
            // a file not sorted by id: the positions to a temporary file, sorted
            let mut positions = Writer::new(store).map_err(io)?;
            each_node(false, &mut |_, c, _| {
                for &(id, (lon, lat)) in c {
                    positions.push(&join::Position, &(id, lon, lat)).map_err(io)?;
                }
                Ok(())
            })?;
            let positions = positions.finish().map_err(io)?;
            let mut positions = spill::sort_bytes(positions, join::node_id, BUDGET, store).map_err(io)?;
            join::points(requests()?, &mut positions, BUDGET, store).map_err(io)?
        }
    };
    // pass 3: the ways with their lines (the points come in the order of the ways), and the relations
    let mut points = points.read(&join::Point).map_err(io)?.peekable();
    let (mut number, mut pts, mut wkb) = (0u32, Vec::<Pt>::new(), Vec::<u8>::new());
    let mut lines = |mut part: RawPart| -> Result<RawPart, String> {
        let ways = &part.batches[1];
        let mut line = BinaryBuilder::new();
        for _ in 0..ways.num_rows() {
            pts.clear();
            while let Some(p) = points.next_if(|p| p.as_ref().is_ok_and(|p| p.0 == number)) {
                let p = p.map_err(io)?;
                pts.push((p.2, p.3));
            }
            if let Some(Err(_)) = points.peek() {
                return Err(io(points.next().expect("peeked").expect_err("an error")));
            }
            // as write_raw_ways_to: fewer than 2 points is NULL
            if pts.len() >= 2 {
                super::line_wkb_into(&pts, &mut wkb);
                line.append_value(&wkb);
            } else {
                line.append_null();
            }
            number = number.wrapping_add(1);
        }
        let line: ArrayRef = Arc::new(line.finish());
        part.batches[1] = RecordBatch::try_from_iter_with_nullable(vec![
            ("osm_id", ways.column(0).clone(), false),
            ("tags", ways.column(1).clone(), true),
            ("refs", ways.column(2).clone(), false),
            ("line", line, true),
        ])
        .map_err(err)?;
        Ok(part)
    };
    each_block(
        &mut open()?,
        |b, buf| {
            let mut s = Patching { cols: RawColumns::ways_and_relations(), patch, coords: Vec::new(), ways: 0, tags: Vec::new() };
            decode_block(b, [false, true, true], buf, &mut s)?;
            Ok((s.cols.finish()?, s.ways))
        },
        |(part, ways), _| {
            rows[1] += ways;
            rows[2] += part.batches[2].num_rows();
            w.write(lines(part)?)?;
            Ok(true)
        },
    )?;
    let mut c = RawColumns::ways_and_relations();
    for nw in patch.new_ways() {
        let mut tags = nw.tags.clone();
        if let Some(n) = patch.net_way(nw.id) {
            sync_way(&mut tags, n);
        }
        c.way(nw.id, &mut pairs(&tags).into_iter(), &mut nw.refs.iter().copied());
        rows[1] += 1;
    }
    w.write(lines(c.finish()?)?)?;
    w.finish()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    /// The writer on the calling thread gives the same rows as `load_raw`, on Monaco.
    #[cfg(feature = "pbf")]
    #[test]
    fn load_raw_here_has_the_rows_of_load_raw() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
        let file = || std::io::BufReader::new(std::fs::File::open(&path).unwrap());
        let want = load_raw(file(), 64 << 20, 2, |_| Ok(Vec::new())).unwrap().rows;
        let got = load_raw_here(file(), [Vec::new(), Vec::new(), Vec::new()], 16 << 20).unwrap();
        assert!(want.iter().all(|&n| n > 0));
        assert_eq!(got, want);
    }

    use super::*;

    #[test]
    fn memory_sizes() {
        assert_eq!(parse_memory("2GB").unwrap(), 2_000_000_000);
        assert_eq!(parse_memory("512 MiB").unwrap(), 512 << 20);
        assert_eq!(parse_memory("1.5GB").unwrap(), 1_500_000_000);
        assert_eq!(parse_memory("4096").unwrap(), 4096);
        assert!(parse_memory("2 GX").is_err());
        assert!(parse_memory("lots").is_err());
    }
}
