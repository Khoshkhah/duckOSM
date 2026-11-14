//! The GeoParquet file of `duckosm-rust quack` (docs/rust/quack.md): the columns of QuackOSM, `feature_id` (VARCHAR),
//! `tags` (MAP(VARCHAR, VARCHAR)) and `geometry` (WKB), with QuackOSM's file options (zstd level 3, row groups of
//! 100 000 rows).
use super::{err, map_type, ParWriter};
use arrow_array::builder::{BinaryBuilder, MapBuilder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use duckosm_core::quack::Feature;
use parquet::basic::Compression;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use std::io::Write;
use std::sync::Arc;

const ROWS: usize = 100_000;

#[cfg(feature = "zstd")]
fn compression() -> Compression {
    Compression::ZSTD(parquet::basic::ZstdLevel::try_new(3).expect("zstd level 3"))
}

#[cfg(not(feature = "zstd"))]
fn compression() -> Compression {
    super::compression()
}

/// A writer of the features table: [`QuackWriter::push`] the features as they are made, then [`QuackWriter::close`].
pub struct QuackWriter<W: Write + Send> {
    w: ParWriter<W>,
    schema: Arc<Schema>,
    rows: Vec<Feature>,
    count: usize,
}

impl<W: Write + Send> QuackWriter<W> {
    pub fn new(out: W) -> Result<Self, String> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("feature_id", DataType::Utf8, false),
            Field::new("tags", map_type(), true),
            Field::new("geometry", DataType::Binary, false),
        ]));
        let geo = r#"{"version":"1.0.0","primary_column":"geometry","columns":{"geometry":{"encoding":"WKB","geometry_types":[]}}}"#;
        let props = WriterProperties::builder()
            .set_compression(compression())
            .set_max_row_group_row_count(Some(ROWS))
            .set_key_value_metadata(Some(vec![KeyValue::new("geo".to_string(), geo.to_string())]))
            .build();
        let w = ParWriter::try_new(out, schema.clone(), Some(props)).map_err(err)?;
        Ok(QuackWriter { w, schema, rows: Vec::new(), count: 0 })
    }

    pub fn push(&mut self, rows: Vec<Feature>) -> Result<(), String> {
        self.count += rows.len();
        self.rows.extend(rows);
        while self.rows.len() >= ROWS {
            let rest = self.rows.split_off(ROWS);
            let full = std::mem::replace(&mut self.rows, rest);
            self.write(&full)?;
        }
        Ok(())
    }

    fn write(&mut self, rows: &[Feature]) -> Result<(), String> {
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
        let batch = RecordBatch::try_new(self.schema.clone(), cols).map_err(err)?;
        self.w.write(&batch).map_err(err)
    }

    /// Write the last rows and close the file. Returns the number of rows.
    pub fn close(mut self) -> Result<usize, String> {
        let rest = std::mem::take(&mut self.rows);
        if !rest.is_empty() {
            self.write(&rest)?;
        }
        self.w.close().map_err(err)?;
        Ok(self.count)
    }
}
