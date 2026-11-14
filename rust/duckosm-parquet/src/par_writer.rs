//! A Parquet writer that encodes the columns of a row group on all threads (docs/concepts/offline-online.md, step 4).
//! `ArrowWriter` encodes the columns one after the other on the calling thread, so the writing of the network files
//! used one core. Here each leaf column of a batch is encoded into a buffer on a thread of the pool, and the calling
//! thread appends the finished column chunks in their order. The file is written only on the calling thread (in the
//! browser, a file in OPFS belongs to the thread that opened it).
//!
//! The file is the same as the file of `ArrowWriter` with the same properties: the same row groups (batches are
//! collected up to `max_row_group_row_count` rows and cut there), the same encoders and the same metadata.

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use parquet::arrow::arrow_writer::{compute_leaves, ArrowColumnChunk, ArrowRowGroupWriterFactory};
use parquet::arrow::{add_encoded_arrow_schema_to_metadata, ArrowSchemaConverter};
use parquet::errors::Result;
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
use rayon::prelude::*;
use std::io::Write;
use std::sync::Arc;

pub struct ParWriter<W: Write + Send> {
    file: SerializedFileWriter<W>,
    factory: ArrowRowGroupWriterFactory,
    schema: SchemaRef,
    groups: usize,
    /// the rows of the open row group (as `ArrowWriter`: it collects batches up to the limit, and cuts a batch there)
    pending: Vec<RecordBatch>,
    rows: usize,
    max_rows: usize,
    /// the full row groups that wait to be encoded together (see [`ParWriter::set_groups_at_once`])
    full: Vec<Vec<RecordBatch>>,
    at_once: usize,
}

impl<W: Write + Send> ParWriter<W> {
    /// As `ArrowWriter::try_new`. A row group has `max_row_group_row_count` rows (a limit of bytes is not used).
    pub fn try_new(out: W, schema: SchemaRef, props: Option<WriterProperties>) -> Result<Self> {
        let mut props = props.unwrap_or_default();
        add_encoded_arrow_schema_to_metadata(&schema, &mut props);
        let max_rows = props.max_row_group_row_count().unwrap_or(usize::MAX);
        let parquet_schema = ArrowSchemaConverter::new().with_coerce_types(props.coerce_types()).convert(&schema)?;
        let file = SerializedFileWriter::new(out, parquet_schema.root_schema_ptr(), Arc::new(props))?;
        let factory = ArrowRowGroupWriterFactory::new(&file, Arc::clone(&schema));
        Ok(ParWriter { file, factory, schema, groups: 0, pending: Vec::new(), rows: 0, max_rows, full: Vec::new(), at_once: 1 })
    }

    /// Add the rows of `batch`; each full row group is written.
    pub fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let mut rest = batch.clone();
        while rest.num_rows() > 0 {
            let take = (self.max_rows - self.rows).min(rest.num_rows());
            self.pending.push(rest.slice(0, take));
            self.rows += take;
            rest = rest.slice(take, rest.num_rows() - take);
            if self.rows == self.max_rows {
                self.flush()?;
            }
        }
        Ok(())
    }

    /// Encode `k` row groups at the same time (1, the default: one row group at a time). A row group has few columns,
    /// and one large column (the geometry) is encoded on one thread; with `k` row groups, `k` of them are encoded at the
    /// same time. The memory is `k` row groups of rows. The file is the same.
    pub fn set_groups_at_once(&mut self, k: usize) {
        self.at_once = k.max(1);
    }

    /// The open row group is full or the last one: it waits with the others, until `at_once` of them are there.
    fn flush(&mut self) -> Result<()> {
        if self.rows > 0 {
            self.full.push(std::mem::take(&mut self.pending));
            self.rows = 0;
        }
        if self.full.len() >= self.at_once {
            self.write_full()?;
        }
        Ok(())
    }

    /// Write the waiting row groups: each leaf column of each row group on a thread of the pool, then the chunks in order.
    fn write_full(&mut self) -> Result<()> {
        let mut jobs = Vec::new();
        for (j, batches) in std::mem::take(&mut self.full).into_iter().enumerate() {
            let writers = self.factory.create_column_writers(self.groups + j)?;
            let mut leaves: Vec<Vec<_>> = (0..writers.len()).map(|_| Vec::new()).collect();
            for b in batches {
                let mut i = 0;
                for (field, array) in self.schema.fields().iter().zip(b.columns()) {
                    for leaf in compute_leaves(field, array)? {
                        leaves[i].push(leaf);
                        i += 1;
                    }
                }
            }
            jobs.push(writers.into_iter().zip(leaves).collect::<Vec<_>>());
        }
        let columns: Vec<usize> = jobs.iter().map(Vec::len).collect();
        let chunks: Vec<ArrowColumnChunk> = jobs
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .into_par_iter()
            .map(|(mut w, ls)| {
                for l in &ls {
                    w.write(l)?;
                }
                w.close()
            })
            .collect::<Result<_>>()?;
        let mut chunks = chunks.into_iter();
        for n in columns {
            let mut group = self.file.next_row_group()?;
            for c in chunks.by_ref().take(n) {
                c.append_to_row_group(&mut group)?;
            }
            group.close()?;
            self.groups += 1;
        }
        Ok(())
    }

    /// Write the last row group and the footer, and flush (as `ArrowWriter::close`).
    pub fn close(mut self) -> Result<()> {
        self.flush()?;
        self.write_full()?;
        self.file.close().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{ArrayRef, Int64Array, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;

    /// With one or more row groups at the same time, the file is the file of `ArrowWriter`.
    #[test]
    fn same_file_as_arrow_writer() {
        let schema = Arc::new(Schema::new(vec![Field::new("a", DataType::Int64, false), Field::new("s", DataType::Utf8, false)]));
        let batch = |from: i64, n: i64| {
            let a: ArrayRef = Arc::new(Int64Array::from_iter_values(from..from + n));
            let s: ArrayRef = Arc::new(StringArray::from_iter_values((from..from + n).map(|i| format!("row {i}"))));
            RecordBatch::try_new(schema.clone(), vec![a, s]).unwrap()
        };
        let batches = [batch(0, 130), batch(130, 70), batch(200, 345)];
        let props = || WriterProperties::builder().set_max_row_group_row_count(Some(100)).build();
        let mut want = Vec::new();
        let mut w = ArrowWriter::try_new(&mut want, schema.clone(), Some(props())).unwrap();
        batches.iter().for_each(|b| w.write(b).unwrap());
        w.close().unwrap();
        for k in [1, 3, 10] {
            let mut got = Vec::new();
            let mut w = ParWriter::try_new(&mut got, schema.clone(), Some(props())).unwrap();
            w.set_groups_at_once(k);
            batches.iter().for_each(|b| w.write(b).unwrap());
            w.close().unwrap();
            assert!(got == want, "k = {k}");
        }
    }
}
