//! The GeoParquet files of `duckosm quack --osm2pgsql` (docs/concepts/osm2pgsql.md): one file for each of osm2pgsql's
//! four tables, with its columns (`osm_id`, the columns of `default.style`, `tags` as a map, the geometry `way` as WKB in
//! EPSG:3857 or EPSG:4326).

use super::{compression, err, map_type, ParWriter, ROW_GROUP};
use arrow_array::builder::{BinaryBuilder, Float32Builder, Int32Builder, Int64Builder, MapBuilder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use duckosm_core::osm2pgsql::{self as o2p, Cell, Kind};
use duckosm_core::quack::Feature;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use std::io::Write;
use std::sync::Arc;

/// The PROJJSON of EPSG:3857 (pyproj), for the `geo` metadata.
const EPSG3857: &str = include_str!("epsg3857.json");

fn schema(table: u8) -> SchemaRef {
    let mut f = vec![Field::new("osm_id", DataType::Int64, false)];
    f.extend(o2p::columns(table).into_iter().map(|(c, k)| {
        let t = match k {
            Kind::Text => DataType::Utf8,
            Kind::Int => DataType::Int32,
            Kind::Real => DataType::Float32,
        };
        Field::new(c, t, true)
    }));
    f.push(Field::new("tags", map_type(), true));
    f.push(Field::new("way", DataType::Binary, false));
    Arc::new(Schema::new(f))
}

/// The rows of one table as one Arrow batch.
pub fn batch(table: u8, rows: &[Feature]) -> Result<RecordBatch, String> {
    let cols = o2p::columns(table);
    enum B {
        T(StringBuilder),
        I(Int32Builder),
        R(Float32Builder),
    }
    let mut b: Vec<B> = cols
        .iter()
        .map(|c| match c.1 {
            Kind::Text => B::T(StringBuilder::new()),
            Kind::Int => B::I(Int32Builder::new()),
            Kind::Real => B::R(Float32Builder::new()),
        })
        .collect();
    let mut id = Int64Builder::new();
    let mut tags = MapBuilder::new(None, StringBuilder::new(), StringBuilder::new());
    let mut way = BinaryBuilder::new();
    for f in rows {
        id.append_value(o2p::osm_id(f));
        let (vals, hstore) = o2p::values(f);
        for (b, v) in b.iter_mut().zip(vals) {
            match (b, v) {
                (B::T(b), Some(Cell::Text(s))) => b.append_value(s),
                (B::I(b), Some(Cell::Int(n))) => b.append_value(n),
                (B::R(b), Some(Cell::Real(n))) => b.append_value(n),
                (B::T(b), _) => b.append_null(),
                (B::I(b), _) => b.append_null(),
                (B::R(b), _) => b.append_null(),
            }
        }
        for (k, v) in &hstore {
            tags.keys().append_value(k);
            tags.values().append_value(v);
        }
        tags.append(true).map_err(err)?;
        way.append_value(&f.geometry);
    }
    let mut out: Vec<ArrayRef> = vec![Arc::new(id.finish())];
    for b in b.iter_mut() {
        out.push(match b {
            B::T(b) => Arc::new(b.finish()),
            B::I(b) => Arc::new(b.finish()),
            B::R(b) => Arc::new(b.finish()),
        });
    }
    out.push(Arc::new(tags.finish()));
    out.push(Arc::new(way.finish()));
    RecordBatch::try_new(schema(table), out).map_err(err)
}

/// The `geo` metadata: the column `way`, its type, and its CRS (EPSG:3857, or OGC:CRS84 with no `crs`).
fn geo_metadata(table: u8, merc: bool) -> KeyValue {
    let kind = match table {
        o2p::POINT => "\"Point\"",
        o2p::POLYGON => "\"Polygon\"",
        _ => "\"LineString\"",
    };
    let crs = if merc { format!(",\"crs\":{}", EPSG3857.trim()) } else { String::new() };
    let json = format!("{{\"version\":\"1.0.0\",\"primary_column\":\"way\",\"columns\":{{\"way\":{{\"encoding\":\"WKB\",\"geometry_types\":[{kind}]{crs}}}}}}}");
    KeyValue::new("geo".to_string(), json)
}

/// The writer of the file of one table: [`Osm2pgsqlWriter::write`] the rows, then [`Osm2pgsqlWriter::close`].
pub struct Osm2pgsqlWriter<W: Write + Send> {
    table: u8,
    w: ParWriter<W>,
    rows: Vec<Feature>,
    count: usize,
}

impl<W: Write + Send> Osm2pgsqlWriter<W> {
    pub fn new(table: u8, merc: bool, out: W) -> Result<Self, String> {
        let props = WriterProperties::builder()
            .set_compression(compression())
            .set_max_row_group_row_count(Some(ROW_GROUP))
            .set_key_value_metadata(Some(vec![geo_metadata(table, merc)]))
            .build();
        let w = ParWriter::try_new(out, schema(table), Some(props)).map_err(err)?;
        Ok(Osm2pgsqlWriter { table, w, rows: Vec::new(), count: 0 })
    }

    pub fn write(&mut self, f: Feature) -> Result<(), String> {
        self.rows.push(f);
        if self.rows.len() >= 20_000 {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), String> {
        if !self.rows.is_empty() {
            self.count += self.rows.len();
            let b = batch(self.table, &std::mem::take(&mut self.rows))?;
            self.w.write(&b).map_err(err)?;
        }
        Ok(())
    }

    /// Close the file; the number of rows.
    pub fn close(mut self) -> Result<usize, String> {
        self.flush()?;
        self.w.close().map_err(err)?;
        Ok(self.count)
    }
}
