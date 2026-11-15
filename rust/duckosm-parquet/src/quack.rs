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
        Self::with(out, schema(), Vec::new())
    }

    /// The writer of a tile table ([`tile_schema`]), with more key-value metadata (the tile and the input file).
    pub fn tile(out: W, meta: Vec<(String, String)>) -> Result<Self, String> {
        Self::with(out, tile_schema(), meta)
    }

    fn with(out: W, schema: SchemaRef, meta: Vec<(String, String)>) -> Result<Self, String> {
        let geo = r#"{"version":"1.0.0","primary_column":"geometry","columns":{"geometry":{"encoding":"WKB","geometry_types":[]}}}"#;
        let mut kv = vec![KeyValue::new("geo".to_string(), geo.to_string())];
        kv.extend(meta.into_iter().map(|(k, v)| KeyValue::new(k, v)));
        let props = WriterProperties::builder()
            .set_compression(compression())
            .set_max_row_group_row_count(Some(ROW_GROUP))
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
            Ok(Feature { feature_id: id.value(i).to_string(), tags, geometry, center, owner: true })
        })
        .collect()
}

/// The batches of a GeoParquet file of quack in memory (a part of the join of quack by tiles in the browser).
/// The quack parts (GeoParquet files of [`QuackWriter`], each one in memory) as one file, in their order, with no sort:
/// each row group of each part is copied as bytes (`append_column`), with no decode and no new encode (the join of the
/// browser: 23.8 s for Sweden on a laptop when it decoded and encoded each row again). The schema and the key-value
/// metadata (the GeoParquet `geo` and the Arrow schema) are those of the first part. Gives the number of rows.
pub fn concat_quack<W: Write + Send>(parts: impl Iterator<Item = Result<Vec<u8>, String>>, out: W) -> Result<usize, String> {
    use parquet::column::writer::ColumnCloseResult;
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use parquet::file::writer::SerializedFileWriter;
    let (mut out, mut w, mut rows) = (Some(out), None::<SerializedFileWriter<W>>, 0usize);
    for p in parts {
        let bytes = bytes::Bytes::from(p?);
        let reader = SerializedFileReader::new(bytes.clone()).map_err(err)?;
        let meta = reader.metadata();
        if w.is_none() {
            let file = meta.file_metadata();
            let props = WriterProperties::builder().set_key_value_metadata(file.key_value_metadata().cloned()).build();
            let out = out.take().expect("one writer");
            w = Some(SerializedFileWriter::new(out, file.schema_descr().root_schema_ptr(), Arc::new(props)).map_err(err)?);
        }
        let w = w.as_mut().expect("the writer");
        for rg in meta.row_groups() {
            let mut g = w.next_row_group().map_err(err)?;
            for col in rg.columns() {
                let close = ColumnCloseResult {
                    bytes_written: col.compressed_size() as u64,
                    rows_written: rg.num_rows() as u64,
                    metadata: col.clone(),
                    bloom_filter: None,
                    column_index: None,
                    offset_index: None,
                };
                g.append_column(&bytes, close).map_err(err)?;
            }
            g.close().map_err(err)?;
            rows += rg.num_rows() as usize;
        }
    }
    match (w, out) {
        (Some(w), _) => {
            w.close().map_err(err)?;
            Ok(rows)
        }
        // no part: an empty table
        (None, Some(out)) => QuackWriter::new(out)?.close(),
        (None, None) => unreachable!("the writer takes the output"),
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
