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
    /// each batch of row groups is a small Parquet file in memory, and [`BigFile`] copies its row groups into the file:
    /// parquet's writer counts the place in a `usize`, 32 bits in the browser, so a file over 4 GiB had a wrong footer.
    /// The same path natively, so the browser and the computer write the same bytes.
    file: BigFile<W>,
    parquet_schema: parquet::schema::types::TypePtr,
    props: Arc<WriterProperties>,
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
        let file = BigFile::new(out).map_err(parquet::errors::ParquetError::General)?;
        let (parquet_schema, props) = (parquet_schema.root_schema_ptr(), Arc::new(props));
        Ok(ParWriter { file, parquet_schema, props, schema, groups: 0, pending: Vec::new(), rows: 0, max_rows, full: Vec::new(), at_once: 1 })
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
        if self.full.is_empty() {
            return Ok(());
        }
        let mut mini = SerializedFileWriter::new(Vec::new(), self.parquet_schema.clone(), self.props.clone())?;
        let (factory, first) = (ArrowRowGroupWriterFactory::new(&mini, Arc::clone(&self.schema)), 0);
        let mut jobs = Vec::new();
        for (j, batches) in std::mem::take(&mut self.full).into_iter().enumerate() {
            let writers = factory.create_column_writers(first + j)?;
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
        let file = &mut mini;
        for n in columns {
            let mut group = file.next_row_group()?;
            for c in chunks.by_ref().take(n) {
                c.append_to_row_group(&mut group)?;
            }
            group.close()?;
            self.groups += 1;
        }
        self.file.append(bytes::Bytes::from(mini.into_inner()?)).map_err(parquet::errors::ParquetError::General)?;
        Ok(())
    }

    /// Write the last row group and the footer, and flush (as `ArrowWriter::close`).
    pub fn close(mut self) -> Result<()> {
        self.flush()?;
        self.write_full()?;
        // no row group: the schema and the metadata of an empty file
        if self.groups == 0 {
            let empty = SerializedFileWriter::new(Vec::new(), self.parquet_schema.clone(), self.props.clone())?;
            self.file.append(bytes::Bytes::from(empty.into_inner()?)).map_err(parquet::errors::ParquetError::General)?;
        }
        self.file.close().map(|_| ()).map_err(parquet::errors::ParquetError::General)
    }
}

/// A Parquet file made of the row groups of other Parquet files (each one whole, in memory), copied as bytes. It counts
/// the place in the file with 64 bits: the writer of parquet (`TrackedWrite`) counts it in a `usize`, which in the
/// browser (wasm32) wraps at 4 GiB, so the footer of a larger file had wrong offsets (France: DuckDB said
/// "TProtocolException: Invalid data"). The page indexes and the bloom filters are not copied. The first file gives the
/// schema, the key-value metadata and the version.
pub struct BigFile<W: Write> {
    out: W,
    at: u64,
    file: Option<parquet::file::metadata::FileMetaData>,
    groups: Vec<parquet::file::metadata::RowGroupMetaData>,
    rows: i64,
}

impl<W: Write> BigFile<W> {
    pub fn new(mut out: W) -> Result<Self, String> {
        out.write_all(b"PAR1").map_err(|e| e.to_string())?;
        Ok(BigFile { out, at: 4, file: None, groups: Vec::new(), rows: 0 })
    }

    /// Copy the row groups of the Parquet file `bytes`.
    pub fn append(&mut self, bytes: bytes::Bytes) -> Result<(), String> {
        use parquet::file::metadata::{ParquetMetaDataReader, RowGroupMetaData};
        let meta = ParquetMetaDataReader::new().parse_and_finish(&bytes).map_err(|e| e.to_string())?;
        if self.file.is_none() {
            self.file = Some(meta.file_metadata().clone());
        }
        for rg in meta.row_groups() {
            let start = self.at;
            let mut cols = Vec::with_capacity(rg.columns().len());
            for c in rg.columns() {
                let (from, len) = c.byte_range();
                self.out.write_all(&bytes[from as usize..(from + len) as usize]).map_err(|e| e.to_string())?;
                let shift = self.at as i64 - from as i64;
                cols.push(
                    c.clone()
                        .into_builder()
                        .set_data_page_offset(c.data_page_offset() + shift)
                        .set_dictionary_page_offset(c.dictionary_page_offset().map(|o| o + shift))
                        .set_index_page_offset(c.index_page_offset().map(|o| o + shift))
                        .set_offset_index_offset(None)
                        .set_offset_index_length(None)
                        .set_column_index_offset(None)
                        .set_column_index_length(None)
                        .set_bloom_filter_offset(None)
                        .set_bloom_filter_length(None)
                        .build()
                        .map_err(|e| e.to_string())?,
                );
                self.at += len;
            }
            let g = RowGroupMetaData::builder(rg.schema_descr_ptr())
                .set_num_rows(rg.num_rows())
                .set_total_byte_size(rg.total_byte_size())
                .set_column_metadata(cols)
                .set_file_offset(start as i64)
                .set_ordinal(self.groups.len() as i32)
                .build()
                .map_err(|e| e.to_string())?;
            self.rows += rg.num_rows();
            self.groups.push(g);
        }
        Ok(())
    }

    /// The rows of the row groups so far.
    pub fn rows(&self) -> i64 {
        self.rows
    }

    /// Write the footer; gives the output back.
    pub fn close(mut self) -> Result<W, String> {
        use parquet::file::metadata::{FileMetaData, ParquetMetaData, ParquetMetaDataWriter};
        let f = self.file.take().ok_or("a Parquet file with no input file")?;
        let file = FileMetaData::new(
            f.version(),
            self.rows,
            f.created_by().map(str::to_string),
            f.key_value_metadata().cloned(),
            f.schema_descr_ptr(),
            f.column_orders().cloned(),
        );
        let meta = ParquetMetaData::new(file, std::mem::take(&mut self.groups));
        ParquetMetaDataWriter::new(&mut self.out, &meta).finish().map_err(|e| e.to_string())?;
        self.out.flush().map_err(|e| e.to_string())?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{ArrayRef, Int64Array, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;

    /// With one or more row groups at the same time, the file is the same, and it has the rows, the row groups and the
    /// key-value metadata of the file of `ArrowWriter` (not its bytes: the page indexes are not copied, see [`BigFile`]).
    #[test]
    fn same_rows_as_arrow_writer() {
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
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
        let read = |b: &[u8]| {
            let r = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::copy_from_slice(b)).unwrap();
            let meta = r.metadata().clone();
            let rows: Vec<RecordBatch> = r.build().unwrap().map(|x| x.unwrap()).collect();
            let groups: Vec<i64> = meta.row_groups().iter().map(|g| g.num_rows()).collect();
            (arrow_select::concat::concat_batches(&schema, &rows).unwrap(), groups, meta.file_metadata().key_value_metadata().cloned())
        };
        let want = read(&want);
        let mut first: Option<Vec<u8>> = None;
        for k in [1, 3, 10] {
            let mut got = Vec::new();
            let mut w = ParWriter::try_new(&mut got, schema.clone(), Some(props())).unwrap();
            w.set_groups_at_once(k);
            batches.iter().for_each(|b| w.write(b).unwrap());
            w.close().unwrap();
            assert!(read(&got) == want, "k = {k}: the rows, row groups or metadata differ from ArrowWriter");
            assert!(first.get_or_insert_with(|| got.clone()) == &got, "k = {k}: another file than with k = 1");
        }
    }
}
