//! The load step (docs/design/streaming_load.md): duckOSM's `raw.nodes`, `raw.ways` and `raw.relations` from a
//! `.osm.pbf` file, written as Parquet while the file is read, in a memory budget. The memory does not grow with the
//! size of the file: only the blocks of one round and one open row group of each table are in memory.

use super::{err, ROW_GROUP};
use arrow_array::builder::{Float64Builder, Int64Builder, ListBuilder, MapBuilder, StringBuilder};
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
    /// The tables to fill (nodes, ways, relations); the elements of the others are skipped.
    keep: [bool; 3],
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
            keep: [true; 3],
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
        let ways = RecordBatch::try_from_iter_with_nullable(vec![
            ("osm_id", a(&mut self.w_id), false),
            ("tags", Arc::new(self.w_tags.finish()) as ArrayRef, true),
            ("refs", Arc::new(self.w_refs.finish()), false),
        ])
        .map_err(err)?;
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
        let empty = RawColumns::default().finish()?;
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

#[cfg(test)]
mod tests {
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
