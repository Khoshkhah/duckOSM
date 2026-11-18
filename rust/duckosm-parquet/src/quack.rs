//! The rows of `duckosm-rust quack` (docs/rust/quack.md) as Arrow batches with the columns of QuackOSM, `feature_id`
//! (VARCHAR), `tags` (MAP(VARCHAR, VARCHAR)) and `geometry` (WKB), and the GeoParquet file of them, with QuackOSM's
//! file options (zstd level 3, row groups of 100 000 rows).
use super::{err, map_type, ParWriter};
use arrow_array::builder::{BinaryBuilder, MapBuilder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use duckosm_core::quack::{Feature, StrSort, ROW_GROUP};
use parquet::basic::Compression;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use std::io::Write;
use std::sync::Arc;

#[cfg(feature = "zstd")]
fn compression() -> Compression {
    Compression::ZSTD(parquet::basic::ZstdLevel::try_new(3).expect("zstd level 3"))
}

#[cfg(not(feature = "zstd"))]
fn compression() -> Compression {
    super::compression()
}

pub fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("feature_id", DataType::Utf8, false),
        Field::new("tags", map_type(), true),
        Field::new("geometry", DataType::Binary, false),
    ]))
}

/// The columns of a tile table: QuackOSM's, and `owner` (true in the one tile that owns the row: the union of tile
/// tables keeps the rows with `owner`).
pub fn tile_schema() -> SchemaRef {
    let mut f: Vec<Field> = schema().fields().iter().map(|f| f.as_ref().clone()).collect();
    f.push(Field::new("owner", DataType::Boolean, false));
    Arc::new(Schema::new(f))
}

/// The rows of a tile table as one Arrow batch: each row with its `owner` (`Feature::owner`).
pub fn tile_batch(rows: &[Feature]) -> Result<RecordBatch, String> {
    let b = batch_of(rows.iter())?;
    let owner = arrow_array::BooleanArray::from(rows.iter().map(|r| r.owner).collect::<Vec<bool>>());
    let mut cols = b.columns().to_vec();
    cols.push(Arc::new(owner));
    RecordBatch::try_new(tile_schema(), cols).map_err(err)
}

/// The rows as one Arrow batch.
pub fn batch(rows: &[Feature]) -> Result<RecordBatch, String> {
    batch_of(rows.iter())
}

fn batch_of<'a>(rows: impl Iterator<Item = &'a Feature>) -> Result<RecordBatch, String> {
    let mut id = StringBuilder::new();
    let mut tags = MapBuilder::new(None, StringBuilder::new(), StringBuilder::new());
    let mut geom = BinaryBuilder::new();
    for f in rows {
        id.append_value(&f.feature_id);
        for (k, v) in &f.tags {
            tags.keys().append_value(k);
            tags.values().append_value(v);
        }
        tags.append(true).map_err(err)?;
        geom.append_value(&f.geometry);
    }
    let cols: Vec<ArrayRef> = vec![Arc::new(id.finish()), Arc::new(tags.finish()), Arc::new(geom.finish())];
    RecordBatch::try_new(schema(), cols).map_err(err)
}

/// The rows on their way to the output. Without the sort, each part goes on at once. With the sort (QuackOSM's
/// default), the rows wait on disk in `duckosm_core::quack::StrSort`, and [`Rows::sorted`] gives them in the order of
/// `str_order`, a row group at a time.
pub struct Rows {
    sort: Option<StrSort>,
}

impl Rows {
    pub fn new(sort: bool) -> Rows {
        Rows { sort: sort.then(StrSort::default) }
    }

    /// The batches of `rows` (pieces of 20 000 rows, made on all threads) to give on at once (no sort), or none (the
    /// rows wait for the sort).
    pub fn push(&mut self, rows: Vec<Feature>) -> Result<Vec<RecordBatch>, String> {
        use rayon::prelude::*;
        if let Some(s) = self.sort.as_mut() {
            s.push(rows)?;
            return Ok(Vec::new());
        }
        rows.par_chunks(20_000).map(batch).collect()
    }

    /// Give the waiting rows in the order of the sort, in batches of a row group.
    pub fn sorted(self, mut give: impl FnMut(RecordBatch) -> Result<(), String>) -> Result<(), String> {
        match self.sort {
            // each row group in pieces of 20 000 rows, made on all threads (the writer joins them into one row group)
            Some(s) => s.finish(|rows| {
                use rayon::prelude::*;
                let parts: Vec<RecordBatch> = rows.par_chunks(20_000).map(batch).collect::<Result<_, _>>()?;
                parts.into_iter().try_for_each(&mut give)
            }),
            None => Ok(()),
        }
    }
}

/// The GeoParquet file of the rows: [`QuackWriter::write`] the batches, then [`QuackWriter::close`].
pub struct QuackWriter<W: Write + Send> {
    w: ParWriter<W>,
    count: usize,
}

impl<W: Write + Send> QuackWriter<W> {
    pub fn new(out: W) -> Result<Self, String> {
        Self::with(out, schema(), Vec::new(), ROW_GROUP)
    }

    /// The writer of a sorted part of quack by tiles: row groups of [`PART_GROUP`] rows, so the merge of the parts
    /// ([`SortedPart`]) holds only one small row group of each part.
    pub fn part(out: W) -> Result<Self, String> {
        Self::with(out, schema(), Vec::new(), PART_GROUP)
    }

    /// The writer of a tile table ([`tile_schema`]), with more key-value metadata (the tile and the input file).
    pub fn tile(out: W, meta: Vec<(String, String)>) -> Result<Self, String> {
        Self::with(out, tile_schema(), meta, ROW_GROUP)
    }

    fn with(out: W, schema: SchemaRef, meta: Vec<(String, String)>, group: usize) -> Result<Self, String> {
        let geo = r#"{"version":"1.0.0","primary_column":"geometry","columns":{"geometry":{"encoding":"WKB","geometry_types":[]}}}"#;
        let mut kv = vec![KeyValue::new("geo".to_string(), geo.to_string())];
        kv.extend(meta.into_iter().map(|(k, v)| KeyValue::new(k, v)));
        let props = WriterProperties::builder()
            .set_compression(compression())
            .set_max_row_group_row_count(Some(group))
            .set_key_value_metadata(Some(kv))
            .build();
        let mut w = ParWriter::try_new(out, schema, Some(props)).map_err(err)?;
        // the geometry column is most of a row group: several row groups at the same time use the threads
        w.set_groups_at_once(4);
        Ok(QuackWriter { w, count: 0 })
    }

    pub fn write(&mut self, b: &RecordBatch) -> Result<(), String> {
        self.count += b.num_rows();
        self.w.write(b).map_err(err)
    }

    /// Close the file. Returns the number of rows.
    pub fn close(self) -> Result<usize, String> {
        self.w.close().map_err(err)?;
        Ok(self.count)
    }
}

/// Read a tile table (a file of [`QuackWriter::tile`]): its batches, each with the columns of [`tile_schema`].
pub fn read_tile_table(path: &std::path::Path, mut f: impl FnMut(RecordBatch) -> Result<(), String>) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .with_batch_size(20_000)
        .build()
        .map_err(err)?;
    for b in reader {
        let b = b.map_err(err)?;
        if b.schema().field_with_name("owner").is_err() {
            return Err(format!("{}: not a tile table of quack (no column owner)", path.display()));
        }
        f(b)?;
    }
    Ok(())
}

/// The rows of a batch of a tile table that `owner` marks, with QuackOSM's columns only (the union of tile tables).
pub fn owned_rows(b: &RecordBatch) -> Result<RecordBatch, String> {
    use arrow_array::cast::AsArray;
    let owner = b.column_by_name("owner").ok_or("a tile table with no column owner")?.as_boolean().clone();
    let kept = arrow_select::filter::filter_record_batch(b, &owner).map_err(err)?;
    RecordBatch::try_new(schema(), kept.columns()[..3].to_vec()).map_err(err)
}

/// The rows of a batch with QuackOSM's columns as [`Feature`]s (for the sort): the center is the center of the box of
/// the geometry, as [`Feature::center`].
pub fn features_of(b: &RecordBatch) -> Result<Vec<Feature>, String> {
    use arrow_array::cast::AsArray;
    use arrow_array::Array;
    let id = b.column(0).as_string::<i32>();
    let tags = b.column(1).as_map();
    let geom = b.column(2).as_binary::<i32>();
    (0..b.num_rows())
        .map(|i| {
            let m = tags.value(i);
            let (k, v) = (m.column(0).as_string::<i32>(), m.column(1).as_string::<i32>());
            let tags = (0..m.len()).map(|j| (k.value(j).to_string(), v.value(j).to_string())).collect();
            let geometry = geom.value(i).to_vec();
            let center = duckosm_core::quack::wkb_center(&geometry).ok_or("a geometry that is not the WKB of quack")?;
            Ok(Feature { feature_id: id.value(i).to_string(), tags, geometry, center, owner: true, ..Default::default() })
        })
        .collect()
}

/// The rows in a row group of a sorted part of quack by tiles ([`QuackWriter::part`]).
pub const PART_GROUP: usize = 2048;

/// The bytes of one row group of a Parquet file in memory, at their place in the file (the reader asks only for the
/// pages of that row group).
struct Window {
    at: u64,
    bytes: bytes::Bytes,
    len: u64,
}

impl parquet::file::reader::Length for Window {
    fn len(&self) -> u64 {
        self.len
    }
}

impl parquet::file::reader::ChunkReader for Window {
    type T = bytes::buf::Reader<bytes::Bytes>;

    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        use bytes::Buf;
        let len = (self.at + self.bytes.len() as u64).saturating_sub(start) as usize;
        Ok(self.get_bytes(start, len)?.reader())
    }

    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<bytes::Bytes> {
        let s = start
            .checked_sub(self.at)
            .map(|s| s as usize)
            .filter(|s| s + length <= self.bytes.len());
        let s = s.ok_or_else(|| {
            parquet::errors::ParquetError::General(format!(
                "bytes {start}+{length} not in the row group"
            ))
        })?;
        Ok(self.bytes.slice(s..s + length))
    }
}

/// A sorted part of quack by tiles (a file of [`QuackWriter::part`]) in the merge of the join: its rows one at a time.
/// At the start it holds one row group. When its row group is done, it reads the next row groups, one for each thread,
/// and decodes them on all threads (the merge takes the rows of the tiles of one column of tiles at a time, so only those
/// parts read more).
pub struct SortedPart<R> {
    r: R,
    len: u64,
    meta: parquet::arrow::arrow_reader::ArrowReaderMetadata,
    next_group: usize,
    batch: Option<RecordBatch>,
    row: usize,
    /// the centers of the rows of the row group
    centers: Vec<duckosm_core::geom::Pt>,
    /// the next row groups, decoded, with their centers
    ahead: std::collections::VecDeque<(RecordBatch, Vec<duckosm_core::geom::Pt>)>,
}

impl<R: std::io::Read + std::io::Seek> SortedPart<R> {
    pub fn open(mut r: R) -> Result<Self, String> {
        use std::io::SeekFrom;
        let io = |e: std::io::Error| format!("a part of quack: {e}");
        let len = r.seek(SeekFrom::End(0)).map_err(io)?;
        let mut tail = [0u8; 8];
        r.seek(SeekFrom::Start(
            len.checked_sub(8)
                .ok_or("a part of quack that is not Parquet")?,
        ))
        .map_err(io)?;
        r.read_exact(&mut tail).map_err(io)?;
        if &tail[4..] != b"PAR1" {
            return Err("a part of quack that is not Parquet".into());
        }
        let n = u32::from_le_bytes(tail[..4].try_into().unwrap()) as u64;
        let mut footer = vec![0u8; n as usize];
        r.seek(SeekFrom::Start(
            len.checked_sub(8 + n)
                .ok_or("a part of quack with a bad footer")?,
        ))
        .map_err(io)?;
        r.read_exact(&mut footer).map_err(io)?;
        let meta = parquet::file::metadata::ParquetMetaDataReader::decode_metadata(&footer)
            .map_err(err)?;
        let meta = parquet::arrow::arrow_reader::ArrowReaderMetadata::try_new(
            Arc::new(meta),
            Default::default(),
        )
        .map_err(err)?;
        let mut p = SortedPart {
            r,
            len,
            meta,
            next_group: 0,
            batch: None,
            row: 0,
            centers: Vec::new(),
            ahead: Default::default(),
        };
        p.load()?;
        Ok(p)
    }

    /// The number of rows of the part.
    pub fn rows(&self) -> usize {
        self.meta.metadata().file_metadata().num_rows() as usize
    }

    /// Read the next row group that has rows (none at the end of the part).
    fn load(&mut self) -> Result<(), String> {
        let first = self.next_group == 0;
        self.load_groups(if first { 1 } else { rayon::current_num_threads() })
    }

    /// The next row group: from the decoded ones, else read the next `n` row groups and decode them on all threads.
    fn load_groups(&mut self, n: usize) -> Result<(), String> {
        use arrow_array::cast::AsArray;
        use rayon::prelude::*;
        use std::io::SeekFrom;
        self.batch = None;
        self.row = 0;
        if self.ahead.is_empty() {
            // the bytes of the row groups, read on this thread (in the browser, a file belongs to its thread)
            let mut read = Vec::new();
            while read.len() < n && self.next_group < self.meta.metadata().num_row_groups() {
                let g = self.next_group;
                self.next_group += 1;
                let rg = self.meta.metadata().row_group(g);
                if rg.num_rows() == 0 {
                    continue;
                }
                let (mut at, mut end) = (u64::MAX, 0);
                for c in rg.columns() {
                    let (s, n) = c.byte_range();
                    (at, end) = (at.min(s), end.max(s + n));
                }
                let mut b = vec![0u8; (end - at) as usize];
                self.r.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
                self.r.read_exact(&mut b).map_err(|e| e.to_string())?;
                read.push((g, Window { at, bytes: b.into(), len: self.len }));
            }
            let meta = &self.meta;
            let decoded: Vec<_> = read
                .into_par_iter()
                .map(|(g, w)| {
                    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::new_with_metadata(w, meta.clone())
                        .with_row_groups(vec![g])
                        .with_batch_size(meta.metadata().row_group(g).num_rows() as usize)
                        .build()
                        .map_err(err)?;
                    let batches = reader.collect::<Result<Vec<_>, _>>().map_err(err)?;
                    let b = arrow_select::concat::concat_batches(&batches[0].schema(), &batches).map_err(err)?;
                    let geom = b.column(2).as_binary::<i32>();
                    let centers = (0..b.num_rows())
                        .map(|i| duckosm_core::quack::wkb_center(geom.value(i)).ok_or_else(|| "a geometry that is not the WKB of quack".to_string()))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok((b, centers))
                })
                .collect::<Result<_, String>>()?;
            self.ahead = decoded.into();
        }
        if let Some((b, centers)) = self.ahead.pop_front() {
            self.batch = Some(b);
            self.centers = centers;
        }
        Ok(())
    }

    /// The key of the merge of the row now ([`duckosm_core::quack::ordered`] x and y of its center), none at the end.
    pub fn key(&self) -> Option<(i64, i64)> {
        self.batch.as_ref().map(|_| {
            let c = self.centers[self.row];
            (duckosm_core::quack::ordered(c.0), duckosm_core::quack::ordered(c.1))
        })
    }

    /// Write the row now for the merge (`duckosm_core::quack::put_sorted_row`, number `seq`) into `out`, and go to the
    /// next row.
    pub fn put(&mut self, seq: u64, out: &mut Vec<u8>) -> Result<(), String> {
        use arrow_array::cast::AsArray;
        let b = self.batch.as_ref().ok_or("the end of a part of quack")?;
        let (i, map) = (self.row, b.column(1).as_map());
        let (k, v) = (map.keys().as_string::<i32>(), map.values().as_string::<i32>());
        let o = map.value_offsets();
        let tags = (o[i] as usize..o[i + 1] as usize).map(|j| (k.value(j), v.value(j)));
        let id = b.column(0).as_string::<i32>().value(i);
        duckosm_core::quack::put_sorted_row(seq, self.centers[i], id, tags, b.column(2).as_binary::<i32>().value(i), out);
        self.row += 1;
        if self.row == b.num_rows() {
            self.load()?;
        }
        Ok(())
    }
}

/// The batches of a GeoParquet file of quack in memory (a part of the join of quack by tiles in the browser).
/// The quack parts (GeoParquet files of [`QuackWriter`], each one in memory) as one file, in their order, with no sort:
/// each row group of each part is copied as bytes ([`crate::par_writer::BigFile`]), with no decode and no new encode (the join of the
/// browser: 23.8 s for Sweden on a laptop when it decoded and encoded each row again). The schema and the key-value
/// metadata (the GeoParquet `geo` and the Arrow schema) are those of the first part. Gives the number of rows.
pub fn concat_quack<W: Write + Send>(parts: impl Iterator<Item = Result<Vec<u8>, String>>, out: W) -> Result<usize, String> {
    use crate::par_writer::BigFile;
    let (mut out, mut file) = (Some(out), None::<BigFile<W>>);
    for p in parts {
        if file.is_none() {
            file = Some(BigFile::new(out.take().expect("one file"))?);
        }
        file.as_mut().expect("the file").append(bytes::Bytes::from(p?))?;
    }
    match (file, out) {
        (Some(f), _) => {
            let rows = f.rows() as usize;
            f.close()?;
            Ok(rows)
        }
        // no part: an empty table
        (None, Some(out)) => QuackWriter::new(out)?.close(),
        (None, None) => unreachable!("the file takes the output"),
    }
}

pub fn read_quack_bytes(b: Vec<u8>, mut f: impl FnMut(RecordBatch) -> Result<(), String>) -> Result<(), String> {
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(b))
        .map_err(err)?
        .with_batch_size(20_000)
        .build()
        .map_err(err)?;
    for x in reader {
        f(x.map_err(err)?)?;
    }
    Ok(())
}
