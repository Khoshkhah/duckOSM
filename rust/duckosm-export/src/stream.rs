//! Read the rows of a query in chunks (DuckDB's streaming result): only one chunk of about 2048 rows is in the memory
//! of this program, never the whole result. `Statement::query` of duckdb-rs keeps the whole result.

use crate::Result;
use duckdb::arrow::array::{Array, ArrayRef, AsArray, StructArray};
use duckdb::arrow::compute::cast;
use duckdb::arrow::datatypes::{DataType, Float64Type, Int64Type};
use duckdb::{Connection, Params};

/// One chunk of rows. The integer columns are i64, the decimal and float columns f64, the text columns UTF-8.
pub struct Chunk {
    cols: Vec<ArrayRef>,
    pub len: usize,
}

impl Chunk {
    fn new(a: &StructArray) -> Result<Chunk> {
        Ok(Chunk { cols: a.columns().iter().map(cast_col).collect::<Result<_>>()?, len: a.len() })
    }

    /// The column `c`, as DuckDB gives it after the casts of [`Chunk`].
    pub fn col(&self, c: usize) -> &ArrayRef {
        &self.cols[c]
    }

    pub fn i64(&self, c: usize, r: usize) -> Option<i64> {
        let a = self.cols[c].as_primitive::<Int64Type>();
        a.is_valid(r).then(|| a.value(r))
    }

    pub fn f64(&self, c: usize, r: usize) -> Option<f64> {
        let a = self.cols[c].as_primitive::<Float64Type>();
        a.is_valid(r).then(|| a.value(r))
    }

    pub fn str(&self, c: usize, r: usize) -> Option<&str> {
        let a = self.cols[c].as_string::<i32>();
        a.is_valid(r).then(|| a.value(r))
    }

    pub fn bool(&self, c: usize, r: usize) -> Option<bool> {
        let a = self.cols[c].as_boolean();
        a.is_valid(r).then(|| a.value(r))
    }
}

/// A column with the types of [`Chunk`]: i64 for the integers, f64 for the decimals and floats, UTF-8 for the text.
pub fn cast_col(c: &ArrayRef) -> Result<ArrayRef> {
    let to = match c.data_type() {
        t if t.is_integer() => DataType::Int64,
        t if t.is_floating() || matches!(t, DataType::Decimal128(..) | DataType::Decimal256(..)) => DataType::Float64,
        DataType::LargeUtf8 | DataType::Utf8View => DataType::Utf8,
        _ => return Ok(c.clone()),
    };
    cast(c, &to).map_err(|e| e.to_string())
}

/// Run `sql` and give its rows to `f`, one chunk at a time, in the order of the result.
pub fn each_chunk<P: Params>(con: &Connection, sql: &str, params: P, mut f: impl FnMut(&Chunk) -> Result<()>) -> Result<()> {
    let mut st = con.prepare(sql).map_err(|e| e.to_string())?;
    // `stream_arrow` runs the query as a stream; its iterator panics on an error, `step` gives the error
    drop(st.stream_arrow(params).map_err(|e| e.to_string())?);
    while let Some(a) = st.step().map_err(|e| e.to_string())? {
        f(&Chunk::new(&a)?)?;
    }
    Ok(())
}
