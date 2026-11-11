//! The load step (docs/design/streaming_load.md): duckOSM's `raw.nodes`, `raw.ways` and `raw.relations` from a
//! `.osm.pbf` file, written as Parquet while the file is read, in a memory budget. The memory does not grow with the
//! size of the file: only the blocks of one round and one open row group of each table are in memory.

use super::{compression, err, ROW_GROUP};
use arrow_array::builder::{Float64Builder, Int64Builder, ListBuilder, MapBuilder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use duckosm_core::graph::MemberType;
use duckosm_core::raw::RawSink;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::io::Write;
use std::sync::Arc;

/// The file names of the three tables, in the order of [`RawPart::batches`].
pub const RAW_FILES: [&str; 3] = [
    "raw_nodes.parquet",
    "raw_ways.parquet",
    "raw_relations.parquet",
];

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
        }
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

/// The encodings of the `raw_*` files. The ids (`osm_id`, and the node ids in `refs`) are sorted or close together, so
/// `DELTA_BINARY_PACKED` stores small differences. `BYTE_STREAM_SPLIT` puts the same byte of each coordinate together,
/// so zstd compresses the coordinates better. DuckDB reads both encodings.
fn raw_props() -> WriterProperties {
    use parquet::basic::Encoding;
    use parquet::schema::types::ColumnPath;
    let mut b = WriterProperties::builder()
        .set_compression(compression())
        .set_max_row_group_row_count(Some(ROW_GROUP));
    for (path, enc) in [
        (vec!["osm_id"], Encoding::DELTA_BINARY_PACKED),
        (vec!["refs", "list", "item"], Encoding::DELTA_BINARY_PACKED),
        (vec!["lat"], Encoding::BYTE_STREAM_SPLIT),
        (vec!["lon"], Encoding::BYTE_STREAM_SPLIT),
    ] {
        let c = ColumnPath::new(path.into_iter().map(String::from).collect());
        b = b
            .set_column_dictionary_enabled(c.clone(), false)
            .set_column_encoding(c, enc);
    }
    b.build()
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

/// The share of the budget for the blocks in work. The open row groups of the three tables get the rest.
const BLOCKS_SHARE: f64 = 0.5;
/// The memory of one block in work, as a multiple of its Arrow part: the decompressed block and the builders (they
/// grow by doubling) are in memory while the part is made. Measured: about 3.2 for the blocks of Monaco.
const BLOCK_COST: f64 = 4.0;

/// Load duckOSM's `raw.*` tables from a `.osm.pbf` file (`reader`) into the three files of [`RAW_FILES`] (each from
/// `file`), with at most about `budget` bytes of memory. `threads` is the number of threads that decode blocks.
///
/// The memory of a block in work is about `BLOCK_COST` times its Arrow part, and the part is about `ratio` times the
/// size of the block in the file. The size in the file is known before the block is decoded, so a round takes the
/// blocks that fit in the budget with the largest `ratio` so far. The first round has one block (no ratio yet). The
/// budget cannot be less than one block: a round always takes one block.
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
    use std::cell::Cell;
    let blocks_budget = budget as f64 * BLOCKS_SHARE;
    let group_bytes = (budget as f64 * (1.0 - BLOCKS_SHARE)) as usize / (2 * RAW_FILES.len());
    let files = [
        file(RAW_FILES[0])?,
        file(RAW_FILES[1])?,
        file(RAW_FILES[2])?,
    ];
    let mut w = RawWriter::new(files, group_bytes)?;
    let ratio = Cell::new(0.0f64);
    let most = Cell::new(1usize);
    let cost = |size: usize| size as f64 * (1.0 + BLOCK_COST * ratio.get());
    let blocks = duckosm_core::pbf::stream_raw(
        reader,
        |size, round| {
            let ok = ratio.get() > 0.0
                && round.len() < 2 * threads.max(1)
                && round.iter().map(|&s| cost(s)).sum::<f64>() + cost(size) <= blocks_budget;
            if ok {
                most.set(most.get().max(round.len() + 1));
            }
            ok
        },
        RawColumns::default,
        RawColumns::finish,
        |part, size| {
            if part.batches.iter().any(|b| b.num_rows() > 0) && size > 0 {
                ratio.set(ratio.get().max(part.bytes() as f64 / size as f64));
            }
            w.write(part)
        },
    )?;
    Ok(LoadSummary {
        rows: w.finish()?,
        blocks,
        ratio: ratio.get(),
        most_in_flight: most.get(),
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
