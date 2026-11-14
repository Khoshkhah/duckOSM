//! `quack_features(path, threads, parquet, sort)`: a DuckDB table function in Rust that gives the rows of QuackOSM's
//! features table, made by `duckosm_core::quack` (docs/rust/quack.md): `feature_id`, `tags` and `geometry` (WKB as BLOB;
//! DuckDB's own `ST_GeomFromWKB` makes the GEOMETRY). A thread makes the rows while DuckDB's insert writes them, so the
//! `.duckdb` file is written with no Parquet file between (QuackOSM writes its Parquet file first and loads it).
use crate::read_osm::fill_list;
use arrow_array::cast::AsArray;
use arrow_array::RecordBatch;
use duckdb::core::{DataChunkHandle, Inserter, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab};
use duckdb::Connection;
use duckosm_core::quack::Counts;
use duckosm_parquet::quack::{QuackWriter, Rows};
use std::error::Error;
use std::path::Path;
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Mutex;

type Part = Result<RecordBatch, String>;

pub struct Bind {
    path: String,
    threads: usize,
    parquet: String,
    sort: bool,
}

/// The batches from the thread, the batch that DuckDB reads now, and its next row.
pub struct Init {
    rows: Mutex<(Receiver<Part>, Option<RecordBatch>, usize)>,
}

struct QuackFeatures;

impl VTab for QuackFeatures {
    type InitData = Init;
    type BindData = Bind;

    fn bind(bind: &BindInfo) -> Result<Bind, Box<dyn Error>> {
        let text = || LogicalTypeHandle::from(LogicalTypeId::Varchar);
        bind.add_result_column("feature_id", text());
        bind.add_result_column("tags", LogicalTypeHandle::map(&text(), &text()));
        bind.add_result_column("geometry", LogicalTypeHandle::from(LogicalTypeId::Blob));
        Ok(Bind {
            path: bind.get_parameter(0).to_string(),
            threads: bind.get_parameter(1).to_int64().max(1) as usize,
            parquet: bind.get_parameter(2).to_string(),
            sort: bind.get_parameter(3).to_string() == "true",
        })
    }

    fn init(init: &InitInfo) -> Result<Init, Box<dyn Error>> {
        let bind = unsafe { &*init.get_bind_data::<Bind>() };
        init.set_max_threads(1);
        let (path, threads, parquet, sort) = (bind.path.clone(), bind.threads, bind.parquet.clone(), bind.sort);
        // a few batches wait in the channel: the memory stays small when DuckDB writes slower than the thread makes rows
        let (tx, rx) = sync_channel::<Part>(4);
        std::thread::spawn(move || {
            let run = || -> Result<(), String> {
                let f = std::fs::File::open(&path).map_err(|e| format!("{path}: {e}"))?;
                // with the format geoparquet too: the same batches also go to the GeoParquet file
                let mut w = match parquet.is_empty() {
                    true => None,
                    false => {
                        let out = std::fs::File::create(&parquet).map_err(|e| format!("{parquet}: {e}"))?;
                        Some(QuackWriter::new(std::io::BufWriter::new(out))?)
                    }
                };
                let mut give = |b: RecordBatch| -> Result<(), String> {
                    if let Some(w) = w.as_mut() {
                        w.write(&b)?;
                    }
                    tx.send(Ok(b)).map_err(|_| "quack_features: the query stopped".to_string())
                };
                let mut rows = Rows::new(sort);
                duckosm_core::quack::features(std::io::BufReader::new(f), threads, |part| match rows.push(part)? {
                    Some(b) => give(b),
                    None => Ok(()),
                })?;
                for b in rows.sorted() {
                    give(b?)?;
                }
                if let Some(w) = w {
                    w.close()?;
                }
                Ok(())
            };
            if let Err(e) = run() {
                let _ = tx.send(Err(e));
            }
        });
        Ok(Init { rows: Mutex::new((rx, None, 0)) })
    }

    fn func(func: &TableFunctionInfo<Self>, out: &mut DataChunkHandle) -> Result<(), Box<dyn Error>> {
        let mut guard = func.get_init_data().rows.lock().unwrap();
        let (rx, part, next) = &mut *guard;
        while part.as_ref().is_none_or(|b| *next >= b.num_rows()) {
            match rx.recv() {
                Ok(Ok(b)) => {
                    *part = Some(b);
                    *next = 0;
                }
                Ok(Err(e)) => return Err(e.into()),
                // the thread is done
                Err(_) => {
                    out.set_len(0);
                    return Ok(());
                }
            }
        }
        let b = part.as_ref().unwrap();
        let (pos, end) = (*next, (*next + out.flat_vector(0).capacity()).min(b.num_rows()));
        let (ids, geometry) = (b.column(0).as_string::<i32>(), b.column(2).as_binary::<i32>());
        let (id_out, geometry_out) = (out.flat_vector(0), out.flat_vector(2));
        for r in pos..end {
            id_out.insert(r - pos, ids.value(r));
            geometry_out.insert(r - pos, geometry.value(r));
        }
        fill_list(&mut out.list_vector(1), b.column(1), pos, end);
        out.set_len(end - pos);
        *next = end;
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        let text = || LogicalTypeHandle::from(LogicalTypeId::Varchar);
        Some(vec![text(), LogicalTypeHandle::from(LogicalTypeId::Bigint), text(), LogicalTypeHandle::from(LogicalTypeId::Boolean)])
    }
}

/// duckosm-rust quack with the `.duckdb` output: the table `main.quackosm` (QuackOSM's table name) of a new file `db`,
/// from the `.osm.pbf` file `pbf`, with DuckDB's GEOMETRY type (`ST_GeomFromWKB` of DuckDB's core, no extension). With
/// `parquet`, the same rows also go to that GeoParquet file. With `sort`, the rows are in the order of
/// `duckosm_core::quack::str_order` (QuackOSM's default). Returns the rows of each kind.
pub fn write_quack(db: &Path, pbf: &Path, threads: usize, parquet: Option<&Path>, sort: bool) -> Result<Counts, String> {
    let err = |e: duckdb::Error| format!("main.quackosm: {e}");
    let _ = std::fs::remove_file(db);
    let con = Connection::open(db).map_err(err)?;
    con.register_table_function::<QuackFeatures>("quack_features").map_err(err)?;
    let text = |p: &Path| p.display().to_string().replace('\'', "''");
    con.execute_batch(&format!(
        "SET preserve_insertion_order = {sort};
         CREATE TABLE main.quackosm AS
         SELECT feature_id, tags, ST_GeomFromWKB(geometry) AS geometry FROM quack_features('{}', {threads}, '{}', {sort});
         CHECKPOINT;",
        text(pbf),
        parquet.map(text).unwrap_or_default()
    ))
    .map_err(err)?;
    con.query_row(
        "SELECT count(*) FILTER (WHERE starts_with(feature_id, 'node/')), count(*) FILTER (WHERE starts_with(feature_id, 'way/')),
                count(*) FILTER (WHERE starts_with(feature_id, 'relation/')) FROM main.quackosm",
        [],
        |r| {
            Ok(Counts {
                nodes: r.get::<_, i64>(0)? as usize,
                ways: r.get::<_, i64>(1)? as usize,
                relations: r.get::<_, i64>(2)? as usize,
            })
        },
    )
    .map_err(err)
}
