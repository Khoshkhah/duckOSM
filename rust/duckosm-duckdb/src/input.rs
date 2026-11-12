//! The input of the network step from `raw.*` in a `.duckdb` file (docs/design/network_stream.md, part A1): the
//! highway ways, the coordinates of their nodes and the turn restrictions, the same values as `pbf::read_ways`,
//! `pbf::read_coords` and `pbf::read_restrictions` give from the `.osm.pbf` file. `raw.*` comes from `read_osm`, which
//! gives the same rows as the file, so the network does not read the file a second time.

use duckdb::arrow::array::{Array, ArrayRef, AsArray, RecordBatch};
use duckdb::arrow::compute::cast;
use duckdb::arrow::datatypes::{DataType, Field, Float64Type, Int64Type};
use duckdb::Connection;
use duckosm_core::filter;
use duckosm_core::geom::Pt;
use duckosm_core::graph::{MemberType, Relation};
use duckosm_core::model::Way;
use duckosm_core::network::Coords;
use std::path::Path;
use std::sync::Arc;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("raw.*: {e}")
}

/// The ways that the reader of the file keeps (`filter::is_kept_by_reader`): a superset in SQL, the exact rule in Rust.
const WAYS: &str = "SELECT osm_id, map_keys(tags) AS k, map_values(tags) AS v, refs FROM raw.ways
  WHERE tags['highway'] IS NOT NULL OR tags['route'] = 'ferry'";

/// A column as an Arrow type that the code reads (DuckDB can give text as Utf8 or as Utf8View).
fn col(b: &RecordBatch, i: usize, t: &DataType) -> Result<ArrayRef, String> {
    cast(b.column(i), t).map_err(err)
}

fn text_list() -> DataType {
    DataType::List(Arc::new(Field::new("item", DataType::Utf8, true)))
}

fn id_list() -> DataType {
    DataType::List(Arc::new(Field::new("item", DataType::Int64, true)))
}

/// Run `sql` and give each Arrow batch to `f`.
fn batches(con: &Connection, sql: &str, mut f: impl FnMut(&RecordBatch) -> Result<(), String>) -> Result<(), String> {
    let mut stmt = con.prepare(sql).map_err(err)?;
    for b in stmt.query_arrow([]).map_err(err)? {
        f(&b)?;
    }
    Ok(())
}

/// Call `f` with the pairs of a list of keys and a list of values at `row` (a NULL map: no pairs).
fn with_pairs<R>(k: &ArrayRef, v: &ArrayRef, row: usize, f: impl FnOnce(&[(&str, &str)]) -> R) -> R {
    let (k, v) = (k.as_list::<i32>(), v.as_list::<i32>());
    if k.is_null(row) {
        return f(&[]);
    }
    let (kv, vv) = (k.value(row), v.value(row));
    let (ks, vs) = (kv.as_string::<i32>(), vv.as_string::<i32>());
    let pairs: Vec<(&str, &str)> = (0..ks.len()).map(|i| (ks.value(i), vs.value(i))).collect();
    f(&pairs)
}

/// The reader of the network input from the `raw.*` tables of a `.duckdb` file.
pub struct RawInput {
    con: std::cell::RefCell<Option<Connection>>,
}

impl RawInput {
    pub fn open(db: &Path) -> Result<Self, String> {
        Ok(RawInput {
            con: std::cell::RefCell::new(Some(Connection::open(db).map_err(err)?)),
        })
    }

    /// Close the connection, so that another connection can open the file (the build writes the network tables next).
    pub fn close(&self) {
        self.con.borrow_mut().take();
    }

    fn with<R>(&self, f: impl FnOnce(&Connection) -> Result<R, String>) -> Result<R, String> {
        match self.con.borrow().as_ref() {
            Some(c) => f(c),
            None => Err("raw.*: the input is closed".into()),
        }
    }

    /// The highway ways and the ferry lines, sorted by id, as `pbf::read_ways`.
    pub fn ways(&self) -> Result<Vec<Way>, String> {
        let mut ways = Vec::new();
        self.with(|con| batches(con, WAYS, |b| {
            let ids = col(b, 0, &DataType::Int64)?;
            let (k, v, refs) = (col(b, 1, &text_list())?, col(b, 2, &text_list())?, col(b, 3, &id_list())?);
            let ids = ids.as_primitive::<Int64Type>();
            let refs = refs.as_list::<i32>();
            for row in 0..b.num_rows() {
                let r = refs.value(row);
                let w = with_pairs(&k, &v, row, |t| {
                    Way::new(ids.value(row), r.as_primitive::<Int64Type>().values().to_vec(), t.iter().copied())
                });
                if filter::is_kept_by_reader(&w.tags) {
                    ways.push(w);
                }
            }
            Ok(())
        }))?;
        ways.sort_by_key(|w| w.id);
        ways.shrink_to_fit();
        Ok(ways)
    }

    /// The coordinates of the nodes of `ways`, as `pbf::read_coords`: a node that is not in the file is not in the
    /// result.
    pub fn coords(&self, ways: &[Way]) -> Result<Coords, String> {
        let mut want: Vec<i64> = ways.iter().flat_map(|w| w.refs.iter().copied()).collect();
        want.sort_unstable();
        want.dedup();
        // the nodes of the kept raw ways; the fixes can change the ways, so the result keeps only the wanted nodes
        let sql = format!(
            "SELECT osm_id, lon, lat FROM raw.nodes WHERE osm_id IN (SELECT unnest(refs) FROM ({WAYS}))"
        );
        let mut nodes: Vec<(i64, Pt)> = Vec::with_capacity(want.len());
        self.with(|con| batches(con, &sql, |b| {
            let id = col(b, 0, &DataType::Int64)?;
            let (lon, lat) = (col(b, 1, &DataType::Float64)?, col(b, 2, &DataType::Float64)?);
            let (id, lon, lat) = (
                id.as_primitive::<Int64Type>(),
                lon.as_primitive::<Float64Type>(),
                lat.as_primitive::<Float64Type>(),
            );
            for i in 0..b.num_rows() {
                if want.binary_search(&id.value(i)).is_ok() {
                    nodes.push((id.value(i), (lon.value(i), lat.value(i))));
                }
            }
            Ok(())
        }))?;
        // a node that a fix added to a way (not in the raw ways)
        let mut have: Vec<i64> = nodes.iter().map(|n| n.0).collect();
        have.sort_unstable();
        let missing: Vec<String> = want
            .iter()
            .filter(|n| have.binary_search(n).is_err())
            .map(|n| n.to_string())
            .collect();
        for part in missing.chunks(10_000) {
            let sql = format!("SELECT osm_id, lon, lat FROM raw.nodes WHERE osm_id IN ({})", part.join(","));
            self.with(|con| batches(con, &sql, |b| {
                let id = col(b, 0, &DataType::Int64)?;
                let (lon, lat) = (col(b, 1, &DataType::Float64)?, col(b, 2, &DataType::Float64)?);
                for i in 0..b.num_rows() {
                    nodes.push((
                        id.as_primitive::<Int64Type>().value(i),
                        (lon.as_primitive::<Float64Type>().value(i), lat.as_primitive::<Float64Type>().value(i)),
                    ));
                }
                Ok(())
            }))?;
        }
        Ok(Coords::new(nodes))
    }

    /// The `type=restriction` relations, sorted by id, as `pbf::read_restrictions`.
    pub fn restrictions(&self) -> Result<Vec<Relation>, String> {
        let sql = "SELECT osm_id, map_keys(tags), map_values(tags), refs, CAST(ref_types AS VARCHAR[]), ref_roles
                   FROM raw.relations WHERE tags['type'] = 'restriction' ORDER BY osm_id";
        let mut rels = Vec::new();
        self.with(|con| batches(con, sql, |b| {
            let id = col(b, 0, &DataType::Int64)?;
            let (k, v) = (col(b, 1, &text_list())?, col(b, 2, &text_list())?);
            let (refs, types, roles) = (col(b, 3, &id_list())?, col(b, 4, &text_list())?, col(b, 5, &text_list())?);
            let (refs, types, roles) = (refs.as_list::<i32>(), types.as_list::<i32>(), roles.as_list::<i32>());
            for row in 0..b.num_rows() {
                let (m, t, r) = (refs.value(row), types.value(row), roles.value(row));
                let (m, t, r) = (m.as_primitive::<Int64Type>(), t.as_string::<i32>(), r.as_string::<i32>());
                let members = (0..m.len())
                    .map(|i| {
                        let ty = match t.value(i) {
                            "node" => MemberType::Node,
                            "way" => MemberType::Way,
                            _ => MemberType::Relation,
                        };
                        // no role: NULL in raw.relations, "" in the reader of the file
                        let role = if r.is_null(i) { "" } else { r.value(i) };
                        (m.value(i), ty, role.to_string())
                    })
                    .collect();
                let tags = with_pairs(&k, &v, row, |t| {
                    t.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
                });
                rels.push(Relation {
                    id: id.as_primitive::<Int64Type>().value(row),
                    tags,
                    members,
                });
            }
            Ok(())
        }))?;
        Ok(rels)
    }
}
