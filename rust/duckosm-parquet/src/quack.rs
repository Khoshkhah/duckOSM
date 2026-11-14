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

/// The rows as one Arrow batch.
pub fn batch(rows: &[Feature]) -> Result<RecordBatch, String> {
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
        let geo = r#"{"version":"1.0.0","primary_column":"geometry","columns":{"geometry":{"encoding":"WKB","geometry_types":[]}}}"#;
        let props = WriterProperties::builder()
            .set_compression(compression())
            .set_max_row_group_row_count(Some(ROW_GROUP))
            .set_key_value_metadata(Some(vec![KeyValue::new("geo".to_string(), geo.to_string())]))
            .build();
        let mut w = ParWriter::try_new(out, schema(), Some(props)).map_err(err)?;
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
