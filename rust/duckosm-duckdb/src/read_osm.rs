//! `read_osm(path, kind)`: a DuckDB table function in Rust that gives the elements of one kind of a `.osm.pbf` file as
//! rows, with the columns of duckOSM's `raw.*` (docs/design/streaming_load.md):
//!
//! | kind | columns |
//! |---|---|
//! | `node` | `osm_id`, `lat`, `lon`, `tags` |
//! | `way` | `osm_id`, `tags`, `refs` (only the ways with 2 or more nodes) |
//! | `relation` | `osm_id`, `tags`, `refs`, `ref_roles`, `ref_types` (text; `raw.relations` casts it to the ENUM list) |
//!
//! It is `ST_READOSM(path) WHERE kind = ...` of DuckDB's spatial extension, in Rust. DuckDB calls it from all its
//! threads: each call takes a decoded block from a queue, or reads (at its place in the file, with no lock) and
//! decodes the next block, and copies up to one vector of rows into DuckDB's vectors. In a file sorted by type, a pass
//! reads only the blocks of its kind. DuckDB's own insert writes the table, so the memory is DuckDB's
//! `memory_limit` and one block for each thread.

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int64Type};
use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::DataType;
use duckdb::core::{DataChunkHandle, Inserter, ListVector, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab};
use duckosm_core::pbf::{blob_from_bytes, block_index, kind_range, Blob};
use duckosm_core::pbfraw::{decode_block, Buffers};
use duckosm_parquet::RawColumns;
use std::error::Error;
use std::io::BufReader;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;


/// The kinds, in the order of `RawPart::batches`.
const KINDS: [&str; 3] = ["node", "way", "relation"];

pub struct Bind {
    path: String,
    kind: usize,
}

/// A decoded block and the next row to give.
struct Cursor {
    rows: RecordBatch,
    pos: usize,
}

pub struct Init {
    kind: usize,
    file: std::fs::File,
    /// The start and the size of each block of the kind's range, and the next one to take.
    blocks: Vec<(u64, usize)>,
    next: AtomicUsize,
    waiting: Mutex<Vec<Cursor>>,
}

/// Read one block at its place in the file (its bytes). Each thread reads its own block, with no lock.
fn read_bytes(file: &std::fs::File, (at, len): (u64, usize)) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; len];
    #[cfg(unix)]
    std::os::unix::fs::FileExt::read_exact_at(file, &mut bytes, at).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        let mut done = 0;
        while done < len {
            let k = std::os::windows::fs::FileExt::seek_read(file, &mut bytes[done..], at + done as u64)
                .map_err(|e| e.to_string())?;
            if k == 0 {
                return Err("the file ends inside a block".into());
            }
            done += k;
        }
    }
    Ok(bytes)
}

/// Read one block at its place in the file, for the binary search of [`kind_range`].
fn read_block(file: &std::fs::File, b: (u64, usize)) -> Result<Blob, String> {
    blob_from_bytes(&read_bytes(file, b)?)
}

pub struct ReadOsm;

/// The next block with rows of the kind, or None at the end of the file.
fn next_block(init: &Init) -> Result<Option<Cursor>, Box<dyn Error>> {
    loop {
        if let Some(c) = init.waiting.lock().unwrap().pop() {
            return Ok(Some(c));
        }
        let i = init.next.fetch_add(1, Ordering::Relaxed);
        let Some(&b) = init.blocks.get(i) else {
            return Ok(None);
        };
        let mut cols = RawColumns::only(init.kind);
        let mut keep = [false; 3];
        keep[init.kind] = true;
        decode_block(&read_bytes(&init.file, b)?, keep, &mut Buffers::default(), &mut cols)?;
        let rows = cols.finish()?.batches[init.kind].clone();
        if rows.num_rows() > 0 {
            return Ok(Some(Cursor { rows, pos: 0 }));
        }
    }
}

/// Copy the rows `pos..end` of a list column into a DuckDB list vector: lists of BIGINT or of text (a NULL text item
/// stays NULL), and maps of text to text (a NULL map stays NULL).
pub(crate) fn fill_list(out: &mut ListVector, a: &ArrayRef, pos: usize, end: usize) {
    let off: &[i32] = match a.data_type() {
        DataType::Map(_, _) => a.as_map().value_offsets(),
        _ => a.as_list::<i32>().value_offsets(),
    };
    let (first, last) = (off[pos] as usize, off[end] as usize);
    let n = last - first;
    match a.data_type() {
        DataType::Map(_, _) => {
            let m = a.as_map();
            let entries = out.struct_child(n);
            let (keys, values) = (entries.child(0, n), entries.child(1, n));
            let (k, v) = (m.keys().as_string::<i32>(), m.values().as_string::<i32>());
            for j in first..last {
                keys.insert(j - first, k.value(j));
                values.insert(j - first, v.value(j));
            }
        }
        _ => match a.as_list::<i32>().values().data_type() {
            DataType::Int64 => unsafe {
                out.child(n)
                    .copy(&a.as_list::<i32>().values().as_primitive::<Int64Type>().values()[first..last]);
            },
            _ => {
                let s = a.as_list::<i32>().values().as_string::<i32>();
                let mut child = out.child(n);
                for j in first..last {
                    if s.is_null(j) {
                        child.set_null(j - first);
                    } else {
                        child.insert(j - first, s.value(j));
                    }
                }
            }
        },
    }
    for r in pos..end {
        if a.is_null(r) {
            out.set_null(r - pos);
        } else {
            out.set_entry(r - pos, off[r] as usize - first, (off[r + 1] - off[r]) as usize);
        }
    }
    out.set_len(n);
}

impl VTab for ReadOsm {
    type InitData = Init;
    type BindData = Bind;

    fn bind(bind: &BindInfo) -> Result<Bind, Box<dyn Error>> {
        let kind_text = bind.get_parameter(1).to_string();
        let kind = KINDS
            .iter()
            .position(|k| *k == kind_text)
            .ok_or_else(|| format!("read_osm: the kind must be node, way or relation, not '{kind_text}'"))?;
        let t = |id| LogicalTypeHandle::from(id);
        let text = || t(LogicalTypeId::Varchar);
        let columns: Vec<(&str, LogicalTypeHandle)> = match kind {
            0 => vec![
                ("osm_id", t(LogicalTypeId::Bigint)),
                ("lat", t(LogicalTypeId::Double)),
                ("lon", t(LogicalTypeId::Double)),
                ("tags", LogicalTypeHandle::map(&text(), &text())),
            ],
            1 => vec![
                ("osm_id", t(LogicalTypeId::Bigint)),
                ("tags", LogicalTypeHandle::map(&text(), &text())),
                ("refs", LogicalTypeHandle::list(&t(LogicalTypeId::Bigint))),
            ],
            _ => vec![
                ("osm_id", t(LogicalTypeId::Bigint)),
                ("tags", LogicalTypeHandle::map(&text(), &text())),
                ("refs", LogicalTypeHandle::list(&t(LogicalTypeId::Bigint))),
                ("ref_roles", LogicalTypeHandle::list(&text())),
                ("ref_types", LogicalTypeHandle::list(&text())),
            ],
        };
        for (name, ty) in columns {
            bind.add_result_column(name, ty);
        }
        Ok(Bind {
            path: bind.get_parameter(0).to_string(),
            kind,
        })
    }

    fn init(init: &InitInfo) -> Result<Init, Box<dyn Error>> {
        let bind = unsafe { &*init.get_bind_data::<Bind>() };
        init.set_max_threads(256);
        let open = || std::fs::File::open(&bind.path).map_err(|e| format!("{}: {e}", bind.path));
        let file = open()?;
        let index = block_index(BufReader::new(open()?))?;
        // a sorted file: only the blocks of the kind (a binary search decodes some blocks to find the borders)
        let range = kind_range(&index, bind.kind, |i| read_block(&file, index.blocks[i]))?;
        Ok(Init {
            kind: bind.kind,
            file,
            blocks: index.blocks[range].to_vec(),
            next: AtomicUsize::new(0),
            waiting: Mutex::new(Vec::new()),
        })
    }

    fn func(func: &TableFunctionInfo<Self>, out: &mut DataChunkHandle) -> Result<(), Box<dyn Error>> {
        let init = func.get_init_data();
        let Some(mut c) = next_block(init)? else {
            out.set_len(0);
            return Ok(());
        };
        let rows = c.rows.num_rows();
        let n = (rows - c.pos).min(out.flat_vector(0).capacity());
        let (pos, end) = (c.pos, c.pos + n);
        for (i, col) in c.rows.columns().iter().enumerate() {
            match col.data_type() {
                DataType::Int64 => unsafe {
                    out.flat_vector(i)
                        .copy(&col.as_primitive::<Int64Type>().values()[pos..end]);
                },
                DataType::Float64 => unsafe {
                    out.flat_vector(i)
                        .copy(&col.as_primitive::<Float64Type>().values()[pos..end]);
                },
                _ => fill_list(&mut out.list_vector(i), col, pos, end),
            }
        }
        out.set_len(n);
        c.pos = end;
        if c.pos < rows {
            init.waiting.lock().unwrap().push(c);
        }
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        let text = || LogicalTypeHandle::from(LogicalTypeId::Varchar);
        Some(vec![text(), text()])
    }
}

/// Register `read_osm(path, kind)` on the connection.
pub fn register(con: &duckdb::Connection) -> duckdb::Result<()> {
    con.register_table_function::<ReadOsm>("read_osm")
}

/// The SQL that makes duckOSM's `raw.nodes`, `raw.ways` and `raw.relations` from `path` with `read_osm` (the types of
/// `ST_READOSM`). `path` must be quoted for SQL.
pub fn raw_sql(path: &str) -> String {
    format!(
        "CREATE SCHEMA IF NOT EXISTS raw;
CREATE OR REPLACE TABLE raw.nodes AS SELECT * FROM read_osm('{path}', 'node');
CREATE OR REPLACE TABLE raw.ways AS SELECT * FROM read_osm('{path}', 'way');
CREATE OR REPLACE TABLE raw.relations AS
  SELECT osm_id, tags, refs, ref_roles, CAST(ref_types AS ENUM('node', 'way', 'relation')[]) AS ref_types
  FROM read_osm('{path}', 'relation');"
    )
}
