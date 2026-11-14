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
/// than [`FAN`] files merge in groups into temporary files first.
pub fn union_sorted<W: Write + Send>(inputs: &[PathBuf], key: &[&str], out: W) -> Result<usize, String> {
    if inputs.len() <= FAN {
        return merge(inputs, key, out);
    }
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("duckosm-union-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(err)?;
    let result = (|| {
        let mut parts = Vec::new();
        for (k, group) in inputs.chunks(FAN).enumerate() {
            let p = dir.join(format!("{k}.parquet"));
            merge(group, key, std::io::BufWriter::new(std::fs::File::create(&p).map_err(err)?))?;
            parts.push(p);
        }
        union_sorted(&parts, key, out)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn merge<W: Write + Send>(inputs: &[PathBuf], key: &[&str], out: W) -> Result<usize, String> {
    let open = |p: &Path| -> Result<ParquetRecordBatchReaderBuilder<std::fs::File>, String> {
        ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(err)
    };
    let Some(first) = inputs.first() else {
        return Ok(0);
    };
    let meta = open(first)?;
    let schema = meta.schema().clone();
    let kv = meta.metadata().file_metadata().key_value_metadata().cloned();
    let props = WriterProperties::builder()
        .set_compression(compression())
        .set_max_row_group_row_count(Some(ROW_GROUP))
        .set_key_value_metadata(kv.map(|v| v.into_iter().filter(|k| k.key != "ARROW:schema").collect()))
        .build();
    let mut w = ParWriter::try_new(out, schema.clone(), Some(props)).map_err(err)?;

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
        if last.as_ref() != Some(&k) {
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
        let n = union_sorted(&inputs, &["node_id"], std::fs::File::create(&out).unwrap()).unwrap();
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
