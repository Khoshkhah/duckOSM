//! The union of tables that the tiles of a large build share: each file is sorted by a key, and two tiles give the same
//! rows for one key. The merge reads the files at the same time and keeps the first row of each key.

use crate::{compression, ParWriter, ROW_GROUP};
use arrow_array::cast::AsArray;
use arrow_array::types::{Int32Type, Int64Type};
use arrow_array::{Array, ArrayRef, RecordBatch};
use parquet::arrow::arrow_reader::{ParquetRecordBatchReader, ParquetRecordBatchReaderBuilder};
use parquet::file::properties::WriterProperties;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// A file and its place: the batch that it reads now (its number in the pool of batches) and the row.
struct Cursor {
    reader: ParquetRecordBatchReader,
    batch: usize,
    row: usize,
    keys: Vec<Vec<i64>>,
}

fn open(p: &Path) -> Result<ParquetRecordBatchReaderBuilder<std::fs::File>, String> {
    ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(err)
}

/// The properties of a file like `meta`: the compression and the blocks of the build, and the metadata (GeoParquet).
fn props_of(meta: &ParquetRecordBatchReaderBuilder<std::fs::File>) -> WriterProperties {
    let kv = meta.metadata().file_metadata().key_value_metadata().cloned();
    WriterProperties::builder()
        .set_compression(compression())
        .set_max_row_group_row_count(Some(ROW_GROUP))
        .set_key_value_metadata(kv.map(|v| v.into_iter().filter(|k| k.key != "ARROW:schema").collect()))
        .build()
}

/// Write the rows of `input` whose BIGINT column `column` passes `keep` to `out`, with the schema and the metadata of
/// `input`. Returns the number of rows.
pub fn filter_file<W: Write + Send>(input: &Path, column: &str, keep: impl Fn(i64) -> bool, out: W) -> Result<usize, String> {
    let meta = open(input)?;
    let schema = meta.schema().clone();
    let mut w = ParWriter::try_new(out, schema, Some(props_of(&meta))).map_err(err)?;
    let mut n = 0;
    for b in meta.build().map_err(err)? {
        let b = b.map_err(err)?;
        let c = b.column_by_name(column).ok_or(format!("no column {column}"))?.as_primitive::<Int64Type>();
        let mask: arrow_array::BooleanArray = c.iter().map(|v| Some(v.is_some_and(&keep))).collect();
        let b = arrow_select::filter::filter_record_batch(&b, &mask).map_err(err)?;
        n += b.num_rows();
        if b.num_rows() > 0 {
            w.write(&b).map_err(err)?;
        }
    }
    w.close().map_err(err)?;
    Ok(n)
}

/// A row of a table of a tile ([`split_file`]): another tile writes it, this tile owns it, or no tile alone owns it.
pub const OTHER: u8 = 0;
pub const MINE: u8 = 1;
pub const BORDER: u8 = 2;

/// Split the table file `path` of a tile by `class` (the class of each row of a block: [`OTHER`], [`MINE`], [`BORDER`]): the
/// file keeps its own rows, and the border rows go to `<name>.border.parquet` (when there are some). Returns the numbers
/// of own and border rows.
pub fn split_file(path: &Path, mut class: impl FnMut(&RecordBatch) -> Result<Vec<u8>, String>) -> Result<(usize, usize), String> {
    let meta = open(path)?;
    let schema = meta.schema().clone();
    let props = props_of(&meta);
    let mine_path = path.with_extension("mine");
    let border_path = path.with_extension("border.parquet");
    let mut mine = ParWriter::try_new(std::io::BufWriter::new(std::fs::File::create(&mine_path).map_err(err)?), schema.clone(), Some(props.clone())).map_err(err)?;
    let mut border = None;
    let (mut n_mine, mut n_border) = (0, 0);
    for b in meta.build().map_err(err)? {
        let b = b.map_err(err)?;
        let c = class(&b)?;
        let mask = |k: u8| arrow_array::BooleanArray::from(c.iter().map(|&x| x == k).collect::<Vec<bool>>());
        let m = arrow_select::filter::filter_record_batch(&b, &mask(MINE)).map_err(err)?;
        if m.num_rows() > 0 {
            n_mine += m.num_rows();
            mine.write(&m).map_err(err)?;
        }
        let x = arrow_select::filter::filter_record_batch(&b, &mask(BORDER)).map_err(err)?;
        if x.num_rows() > 0 {
            n_border += x.num_rows();
            if border.is_none() {
                let f = std::io::BufWriter::new(std::fs::File::create(&border_path).map_err(err)?);
                border = Some(ParWriter::try_new(f, schema.clone(), Some(props.clone())).map_err(err)?);
            }
            border.as_mut().expect("made").write(&x).map_err(err)?;
        }
    }
    mine.close().map_err(err)?;
    if let Some(w) = border {
        w.close().map_err(err)?;
    }
    std::fs::rename(&mine_path, path).map_err(err)?;
    Ok((n_mine, n_border))
}

/// Join the files of `inputs` (one schema) into `out`: their blocks of rows one after the other, copied as bytes (no decode,
/// no new compression; as the tool parquet-concat of the parquet crate). The file has the metadata (GeoParquet) of the
/// first input. `consume`: each input goes when it is copied. Returns the number of rows.
pub fn concat_files(inputs: &[PathBuf], consume: bool, out: &Path) -> Result<usize, String> {
    use parquet::column::writer::ColumnCloseResult;
    use parquet::file::metadata::ParquetMetaDataReader;
    use parquet::file::writer::SerializedFileWriter;
    let Some(first) = inputs.first() else {
        return Ok(0);
    };
    let meta = |p: &Path| -> Result<(std::fs::File, parquet::file::metadata::ParquetMetaData), String> {
        let f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let m = ParquetMetaDataReader::new().parse_and_finish(&f).map_err(err)?;
        Ok((f, m))
    };
    let (_, m0) = meta(first)?;
    let kv = m0.file_metadata().key_value_metadata().cloned();
    let props = Arc::new(WriterProperties::builder().set_compression(compression()).set_key_value_metadata(kv).build());
    let schema = m0.file_metadata().schema_descr().root_schema_ptr();
    let f = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut w = SerializedFileWriter::new(std::io::BufWriter::new(f), schema.clone(), props).map_err(err)?;
    let mut n = 0;
    for p in inputs {
        let (input, m) = meta(p)?;
        if m.file_metadata().schema_descr().root_schema_ptr() != schema {
            return Err(format!("{}: another schema than {}", p.display(), first.display()));
        }
        for rg in m.row_groups() {
            let mut rg_out = w.next_row_group().map_err(err)?;
            for column in rg.columns() {
                let result = ColumnCloseResult {
                    bytes_written: column.compressed_size() as _,
                    rows_written: rg.num_rows() as _,
                    metadata: column.clone(),
                    bloom_filter: None,
                    column_index: None,
                    offset_index: None,
                };
                rg_out.append_column(&input, result).map_err(err)?;
            }
            rg_out.close().map_err(err)?;
            n += rg.num_rows() as usize;
        }
        if consume {
            let _ = std::fs::remove_file(p);
        }
    }
    w.close().map_err(err)?;
    Ok(n)
}

/// The key of each row of a batch: the values of the columns `key` (BIGINT or INTEGER; NULL is the smallest).
fn keys_of(b: &RecordBatch, key: &[&str]) -> Result<Vec<Vec<i64>>, String> {
    let cols: Vec<&ArrayRef> = key.iter().map(|k| b.column_by_name(k).ok_or(format!("no column {k}"))).collect::<Result<_, _>>()?;
    Ok((0..b.num_rows())
        .map(|i| {
            cols.iter()
                .map(|c| match (c.is_valid(i), c.as_primitive_opt::<Int64Type>(), c.as_primitive_opt::<Int32Type>()) {
                    (false, _, _) => i64::MIN,
                    (_, Some(a), _) => a.value(i),
                    (_, _, Some(a)) => a.value(i) as i64,
                    _ => i64::MIN,
                })
                .collect()
        })
        .collect())
}

/// The files that one merge reads at the same time: each open file holds one decoded block of rows (142 Kentucky tiles at
/// one time were 786 MiB).
const FAN: usize = 16;

/// Merge the files of `inputs` (the same table of each tile, each sorted by `key`) into `out`: one row for each key, the
/// first one. The file has the schema and the metadata (GeoParquet) of the first input. Returns the number of rows. More
/// than [`FAN`] files merge in groups into temporary files first. `consume`: each input file goes when its group is merged
/// (the disk keeps less at one time).
pub fn union_sorted<W: Write + Send>(inputs: &[PathBuf], key: &[&str], consume: bool, out: W) -> Result<usize, String> {
    sorted(inputs, key, true, consume, out)
}

/// Merge the files of `inputs` (each sorted by `key`) into `out` with all their rows, sorted by `key`. Returns the number of
/// rows. `consume`: as for [`union_sorted`].
pub fn merge_sorted<W: Write + Send>(inputs: &[PathBuf], key: &[&str], consume: bool, out: W) -> Result<usize, String> {
    sorted(inputs, key, false, consume, out)
}

fn sorted<W: Write + Send>(inputs: &[PathBuf], key: &[&str], one: bool, consume: bool, out: W) -> Result<usize, String> {
    let gone = |group: &[PathBuf]| {
        if consume {
            group.iter().for_each(|p| {
                let _ = std::fs::remove_file(p);
            });
        }
    };
    if inputs.len() <= FAN {
        let n = merge(inputs, key, one, out)?;
        gone(inputs);
        return Ok(n);
    }
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("duckosm-union-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(err)?;
    let result = (|| {
        let mut parts = Vec::new();
        for (k, group) in inputs.chunks(FAN).enumerate() {
            let p = dir.join(format!("{k}.parquet"));
            merge(group, key, one, std::io::BufWriter::new(std::fs::File::create(&p).map_err(err)?))?;
            gone(group);
            parts.push(p);
        }
        // the group files are temporary: each goes when the next merge has read it
        sorted(&parts, key, one, true, out)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Merge at most [`FAN`] files; `one`: one row for each key.
fn merge<W: Write + Send>(inputs: &[PathBuf], key: &[&str], one: bool, out: W) -> Result<usize, String> {
    let Some(first) = inputs.first() else {
        return Ok(0);
    };
    let meta = open(first)?;
    let schema = meta.schema().clone();
    let mut w = ParWriter::try_new(out, schema.clone(), Some(props_of(&meta))).map_err(err)?;

    let mut pool: Vec<RecordBatch> = Vec::new();
    let mut cursors: Vec<Cursor> = Vec::new();
    let mut heap: BinaryHeap<Reverse<(Vec<i64>, usize)>> = BinaryHeap::new();
    // the next batch with rows of cursor `c`, to the pool; false at the end of its file
    let next_batch = |c: &mut Cursor, pool: &mut Vec<RecordBatch>| -> Result<bool, String> {
        for b in c.reader.by_ref() {
            let b = b.map_err(err)?;
            if b.num_rows() > 0 {
                c.keys = keys_of(&b, key)?;
                c.batch = pool.len();
                c.row = 0;
                pool.push(b);
                return Ok(true);
            }
        }
        Ok(false)
    };
    for p in inputs {
        let mut c = Cursor { reader: open(p)?.build().map_err(err)?, batch: 0, row: 0, keys: Vec::new() };
        if next_batch(&mut c, &mut pool)? {
            heap.push(Reverse((c.keys[0].clone(), cursors.len())));
        }
        cursors.push(c);
    }
    let (mut take, mut n, mut last): (Vec<(usize, usize)>, usize, Option<Vec<i64>>) = (Vec::new(), 0, None);
    // write the rows of `take` and keep in the pool only the batches that the cursors still read
    let flush = |take: &mut Vec<(usize, usize)>, pool: &mut Vec<RecordBatch>, cursors: &mut [Cursor], w: &mut ParWriter<W>| -> Result<(), String> {
        if !take.is_empty() {
            let cols: Vec<ArrayRef> = (0..schema.fields().len())
                .map(|k| {
                    let arrays: Vec<&dyn Array> = pool.iter().map(|b| b.column(k).as_ref()).collect();
                    arrow_select::interleave::interleave(&arrays, take).map_err(err)
                })
                .collect::<Result<_, _>>()?;
            w.write(&RecordBatch::try_new(schema.clone(), cols).map_err(err)?).map_err(err)?;
            take.clear();
        }
        let mut kept = Vec::new();
        for c in cursors.iter_mut() {
            if c.batch < pool.len() {
                kept.push(pool[c.batch].clone());
                c.batch = kept.len() - 1;
            }
        }
        *pool = kept;
        Ok(())
    };
    while let Some(Reverse((k, i))) = heap.pop() {
        let c = &mut cursors[i];
        if !one || last.as_ref() != Some(&k) {
            take.push((c.batch, c.row));
            n += 1;
            last = Some(k);
        }
        c.row += 1;
        let more = c.row < c.keys.len() || next_batch(c, &mut pool)?;
        if more {
            heap.push(Reverse((c.keys[c.row].clone(), i)));
        } else {
            c.batch = usize::MAX; // the end of the file: its batches can go
        }
        if take.len() >= ROW_GROUP {
            flush(&mut take, &mut pool, &mut cursors, &mut w)?;
        }
    }
    flush(&mut take, &mut pool, &mut cursors, &mut w)?;
    w.close().map_err(err)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three tiles that share some keys, merged in groups (FAN of more files): each key one time, sorted.
    #[test]
    fn one_row_for_each_key() {
        let dir = std::env::temp_dir().join(format!("duckosm-merge-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tiles: Vec<Vec<(i64, bool)>> = (0..FAN as i64 + 3).map(|t| vec![(t, true), (t + 1, false), (100, true)]).collect();
        let inputs: Vec<PathBuf> = tiles
            .iter()
            .enumerate()
            .map(|(k, rows)| {
                let p = dir.join(format!("{k}.parquet"));
                crate::write_global_junctions_to(std::fs::File::create(&p).unwrap(), rows).unwrap();
                p
            })
            .collect();
        let out = dir.join("out.parquet");
        let n = union_sorted(&inputs, &["node_id"], true, std::fs::File::create(&out).unwrap()).unwrap();
        assert!(inputs.iter().all(|p| !p.exists()), "the merged files go");
        let mut got = Vec::new();
        for b in ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&out).unwrap()).unwrap().build().unwrap() {
            let b = b.unwrap();
            got.extend(b.column(0).as_primitive::<Int64Type>().values().iter().copied());
        }
        std::fs::remove_dir_all(&dir).unwrap();
        let want: Vec<i64> = (0..=FAN as i64 + 3).chain([100]).collect();
        assert_eq!((n, got), (want.len(), want));
    }
}
