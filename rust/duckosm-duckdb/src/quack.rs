//! `quack_features(path, threads)`: a DuckDB table function in Rust that gives the rows of QuackOSM's features table,
//! made by `duckosm_core::quack` (docs/rust/quack.md): `feature_id`, `tags` and `geometry` (WKB as BLOB; DuckDB's own
//! `ST_GeomFromWKB` makes the GEOMETRY). A thread makes the rows while DuckDB's insert writes them, so the `.duckdb`
//! file is written with no Parquet file between (QuackOSM writes its Parquet file first and loads it).
use duckdb::core::{DataChunkHandle, Inserter, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab};
use duckdb::Connection;
use duckosm_core::quack::{Counts, Feature};
use std::error::Error;
use std::path::Path;
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Mutex;

type Rows = Result<Vec<Feature>, String>;

pub struct Bind {
    path: String,
    threads: usize,
    parquet: String,
}

/// The rows from the thread, the part that DuckDB reads now, and the next row of it.
pub struct Init {
    rows: Mutex<(Receiver<Rows>, Vec<Feature>, usize)>,
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
        })
    }

    fn init(init: &InitInfo) -> Result<Init, Box<dyn Error>> {
        let bind = unsafe { &*init.get_bind_data::<Bind>() };
        init.set_max_threads(1);
        let (path, threads, parquet) = (bind.path.clone(), bind.threads, bind.parquet.clone());
        // a few parts wait in the channel: the memory stays small when DuckDB writes slower than the thread makes rows
        let (tx, rx) = sync_channel::<Rows>(4);
        std::thread::spawn(move || {
            let run = || -> Result<(), String> {
                let f = std::fs::File::open(&path).map_err(|e| format!("{path}: {e}"))?;
                // with the format geoparquet too: the same rows also go to the GeoParquet file
                let mut w = match parquet.is_empty() {
                    true => None,
                    false => {
                        let out = std::fs::File::create(&parquet).map_err(|e| format!("{parquet}: {e}"))?;
                        Some(duckosm_parquet::quack::QuackWriter::new(std::io::BufWriter::new(out))?)
                    }
                };
                duckosm_core::quack::features(std::io::BufReader::new(f), threads, |rows| {
                    if let Some(w) = w.as_mut() {
                        w.push(rows.clone())?;
                    }
                    tx.send(Ok(rows)).map_err(|_| "quack_features: the query stopped".to_string())
                })?;
                if let Some(w) = w {
                    w.close()?;
                }
                Ok(())
            };
            if let Err(e) = run() {
                let _ = tx.send(Err(e));
            }
        });
        Ok(Init { rows: Mutex::new((rx, Vec::new(), 0)) })
    }

    fn func(func: &TableFunctionInfo<Self>, out: &mut DataChunkHandle) -> Result<(), Box<dyn Error>> {
        let mut guard = func.get_init_data().rows.lock().unwrap();
        let (rx, part, next) = &mut *guard;
        while *next >= part.len() {
            match rx.recv() {
                Ok(Ok(rows)) => {
                    *part = rows;
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
        let end = (*next + out.flat_vector(0).capacity()).min(part.len());
        let rows = &part[*next..end];
        let ids = out.flat_vector(0);
        let geometry = out.flat_vector(2);
        for (i, f) in rows.iter().enumerate() {
            ids.insert(i, f.feature_id.as_str());
            geometry.insert(i, f.geometry.as_slice());
        }
        let mut tags = out.list_vector(1);
        let n: usize = rows.iter().map(|f| f.tags.len()).sum();
        let entries = tags.struct_child(n);
        let (keys, values) = (entries.child(0, n), entries.child(1, n));
        let mut at = 0;
        for (i, f) in rows.iter().enumerate() {
            tags.set_entry(i, at, f.tags.len());
            for (k, v) in &f.tags {
                keys.insert(at, k.as_str());
                values.insert(at, v.as_str());
                at += 1;
            }
        }
        tags.set_len(n);
        out.set_len(rows.len());
        *next = end;
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        let text = || LogicalTypeHandle::from(LogicalTypeId::Varchar);
        Some(vec![text(), LogicalTypeHandle::from(LogicalTypeId::Bigint), text()])
    }
}

/// duckosm-rust quack with the `.duckdb` output: the table `main.quackosm` (QuackOSM's table name) of a new file `db`,
/// from the `.osm.pbf` file `pbf`, with DuckDB's GEOMETRY type (`ST_GeomFromWKB` of DuckDB's core, no extension). With
/// `parquet`, the same rows also go to that GeoParquet file. Returns the rows of each kind.
pub fn write_quack(db: &Path, pbf: &Path, threads: usize, parquet: Option<&Path>) -> Result<Counts, String> {
    let err = |e: duckdb::Error| format!("main.quackosm: {e}");
    let _ = std::fs::remove_file(db);
    let con = Connection::open(db).map_err(err)?;
    con.register_table_function::<QuackFeatures>("quack_features").map_err(err)?;
    let text = |p: &Path| p.display().to_string().replace('\'', "''");
    con.execute_batch(&format!(
        "SET preserve_insertion_order = false;
         CREATE TABLE main.quackosm AS
         SELECT feature_id, tags, ST_GeomFromWKB(geometry) AS geometry FROM quack_features('{}', {threads}, '{}');
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
