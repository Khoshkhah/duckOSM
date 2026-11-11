//! The load step written directly into DuckDB (way D, docs/design/streaming_load.md): the decode threads append the
//! rows of `raw.nodes`, `raw.ways` and `raw.relations` to the tables, each thread on a connection of its own. No file is
//! written between the decode and DuckDB.

use duckdb::Connection;
use duckosm_parquet::{load_parts, with_free_slot, LoadSummary, RawPart, BLOCKS_SHARE};
use std::path::Path;
use std::sync::Mutex;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Move an Arrow 60 batch (duckosm-parquet) to the Arrow 58 of the duckdb crate. The two versions use the same C
/// structs (the Arrow C data interface), so the buffers move and nothing is copied.
fn to_duckdb_arrow(
    b: arrow_array::RecordBatch,
) -> Result<duckdb::arrow::record_batch::RecordBatch, String> {
    use arrow_array::Array;
    let data = arrow_array::StructArray::from(b).into_data();
    let (a, s) = arrow_array::ffi::to_ffi(&data).map_err(err)?;
    // SAFETY: both are the #[repr(C)] structs of the Arrow C data interface; the size check is at compile time
    let (a, s): (
        duckdb::arrow::ffi::FFI_ArrowArray,
        duckdb::arrow::ffi::FFI_ArrowSchema,
    ) = unsafe { (std::mem::transmute(a), std::mem::transmute(s)) };
    let d = unsafe { duckdb::arrow::ffi::from_ffi(a, &s) }.map_err(err)?;
    Ok(duckdb::arrow::array::StructArray::from(d).into())
}

/// The rows of one transaction of a connection: two row groups of DuckDB.
const COMMIT_ROWS: usize = 2 * 122_880;

const TABLES: [&str; 3] = ["nodes", "ways", "relations_load"];

/// The tables of `raw.*`, with the types of ST_READOSM. The relations come into `raw.relations_load`, with
/// `ref_types` as text (the appender gives text); [`FINISH`] makes `raw.relations` with the ENUM list.
const CREATE: &str = "CREATE SCHEMA IF NOT EXISTS raw;
CREATE OR REPLACE TABLE raw.nodes (osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR, VARCHAR));
CREATE OR REPLACE TABLE raw.ways (osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[]);
CREATE OR REPLACE TABLE raw.relations_load (osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[],
                                            ref_roles VARCHAR[], ref_types VARCHAR[]);";

/// `ALTER TABLE ... TYPE ENUM` cannot be stored in the file (a serialization error), and `CREATE TABLE AS` can.
const FINISH: &str = "CREATE OR REPLACE TABLE raw.relations AS
  SELECT osm_id, tags, refs, ref_roles, CAST(ref_types AS ENUM('node', 'way', 'relation')[]) AS ref_types
  FROM raw.relations_load;
DROP TABLE raw.relations_load;";

/// Append the rows of one part, on the connection of this thread.
fn append(con: &Connection, part: RawPart) -> Result<[usize; 3], String> {
    let mut rows = [0; 3];
    for ((t, b), n) in TABLES.iter().zip(part.batches).zip(rows.iter_mut()) {
        if b.num_rows() == 0 {
            continue;
        }
        *n = b.num_rows();
        let mut app = con.appender_to_db(t, "raw").map_err(err)?;
        app.append_record_batch(to_duckdb_arrow(b)?).map_err(err)?;
        app.flush().map_err(err)?;
    }
    Ok(rows)
}

/// Load `raw.*` of the `.osm.pbf` file `pbf` into the database `db` (the tables are made again), with at most about
/// `budget` bytes of memory and `threads` threads (None: all cores). Half of the budget is for the blocks in work and
/// half is DuckDB's `memory_limit`.
pub fn load_raw_duckdb(
    pbf: &Path,
    db: &Path,
    budget: usize,
    threads: Option<usize>,
) -> Result<LoadSummary, String> {
    let f = std::fs::File::open(pbf).map_err(|e| format!("{}: {e}", pbf.display()))?;
    duckosm_core::pbf::with_threads(threads, || {
        let threads = duckosm_core::pbf::threads();
        let con = Connection::open(db).map_err(err)?;
        let duck = (budget as f64 * (1.0 - BLOCKS_SHARE)) as usize;
        con.execute_batch(&format!(
            "SET memory_limit = '{duck}B'; SET threads = {threads}; SET preserve_insertion_order = false; {CREATE}"
        ))
        .map_err(err)?;
        // one connection for each thread, each in one transaction for the whole load
        let mut cons = Vec::with_capacity(threads);
        for _ in 0..threads {
            let c = con.try_clone().map_err(err)?;
            c.execute_batch("BEGIN TRANSACTION").map_err(err)?;
            cons.push(Mutex::new((c, [0usize; 3], 0usize)));
        }
        let blocks_budget = (budget as f64 * BLOCKS_SHARE) as usize;
        let (blocks, ratio, most_in_flight) = load_parts(
            std::io::BufReader::new(f),
            blocks_budget,
            threads,
            |part| {
                with_free_slot(&cons, |(c, rows, open)| {
                    let n = append(c, part)?;
                    for (a, b) in rows.iter_mut().zip(n) {
                        *a += b;
                    }
                    *open += n.iter().sum::<usize>();
                    // DuckDB keeps the rows of an open transaction in memory; at the commit it writes the full row
                    // groups into the file. So a connection commits after COMMIT_ROWS rows.
                    if *open >= COMMIT_ROWS {
                        c.execute_batch("COMMIT; BEGIN TRANSACTION").map_err(err)?;
                        *open = 0;
                    }
                    Ok(())
                })
            },
        )?;
        let mut rows = [0; 3];
        for m in cons {
            let (c, r, _) = m.into_inner().map_err(err)?;
            c.execute_batch("COMMIT").map_err(err)?;
            for (a, b) in rows.iter_mut().zip(r) {
                *a += b;
            }
        }
        con.execute_batch(FINISH).map_err(err)?;
        Ok(LoadSummary {
            rows,
            blocks,
            ratio,
            most_in_flight,
        })
    })?
}
