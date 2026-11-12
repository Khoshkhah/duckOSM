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
        Ok(ParWriter { file, factory, schema, groups: 0, pending: Vec::new(), rows: 0, max_rows })
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

    /// Write the open row group: each leaf column on a thread of the pool, then the chunks in order.
    fn flush(&mut self) -> Result<()> {
        if self.rows == 0 {
            return Ok(());
        }
        let writers = self.factory.create_column_writers(self.groups)?;
        let mut leaves: Vec<Vec<_>> = (0..writers.len()).map(|_| Vec::new()).collect();
        for b in self.pending.drain(..) {
            let mut i = 0;
            for (field, array) in self.schema.fields().iter().zip(b.columns()) {
                for leaf in compute_leaves(field, array)? {
                    leaves[i].push(leaf);
                    i += 1;
                }
            }
        }
        let chunks: Vec<ArrowColumnChunk> = writers
            .into_par_iter()
            .zip(leaves)
            .map(|(mut w, ls)| {
                for l in &ls {
                    w.write(l)?;
                }
                w.close()
            })
            .collect::<Result<_>>()?;
        let mut group = self.file.next_row_group()?;
        for c in chunks {
            c.append_to_row_group(&mut group)?;
        }
        group.close()?;
        self.groups += 1;
        self.rows = 0;
        Ok(())
    }

    /// Write the last row group and the footer, and flush (as `ArrowWriter::close`).
    pub fn close(mut self) -> Result<()> {
        self.flush()?;
        self.file.close().map(|_| ())
    }
}
