//! The GeoParquet files of `duckosm quack --gdal -f geoparquet` and `duckosm quack --geofabrik -f geoparquet`: one file
//! for each layer of a `duckosm_core::gdal::Schema` (GDAL's or Geofabrik's), with the columns of the layer (text or
//! integer) and the geometry as WKB.

use super::{compression, err, geo_metadata, ParWriter, ROW_GROUP};
use arrow_array::builder::{BinaryBuilder, Int32Builder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use duckosm_core::gdal::{Schema as Layers, Value};
use duckosm_core::quack::Feature;
use parquet::file::properties::WriterProperties;
use std::io::Write;
use std::sync::Arc;

/// The geometry type of a layer in the `geo` metadata.
fn geometry_type(s: &Layers, layer: u8) -> &'static str {
    match (s.geometry)(layer) {
        "POINT" => "Point",
        "LINESTRING" => "LineString",
        "MULTILINESTRING" => "MultiLineString",
        "MULTIPOLYGON" => "MultiPolygon",
        _ => "GeometryCollection",
    }
}

fn schema(s: &Layers, layer: u8) -> SchemaRef {
    let mut f: Vec<Field> =
        (s.columns)(layer).iter().map(|(c, int, _)| Field::new(*c, if *int { DataType::Int32 } else { DataType::Utf8 }, true)).collect();
    f.push(Field::new("geometry", DataType::Binary, false));
    Arc::new(Schema::new(f))
}

/// The rows of one layer as one Arrow batch.
pub fn batch(s: &Layers, layer: u8, rows: &[Feature]) -> Result<RecordBatch, String> {
    let cols = (s.columns)(layer);
    let mut text: Vec<Option<StringBuilder>> = cols.iter().map(|c| (!c.1).then(StringBuilder::new)).collect();
    let mut ints: Vec<Option<Int32Builder>> = cols.iter().map(|c| c.1.then(Int32Builder::new)).collect();
    let mut geom = BinaryBuilder::new();
    for f in rows {
        for (i, v) in (s.values)(f).into_iter().enumerate() {
            match (&mut text[i], &mut ints[i], v) {
                (Some(b), _, Some(Value::Text(s))) => b.append_value(s),
                (_, Some(b), Some(Value::Int(n))) => b.append_value(n as i32),
                (Some(b), _, _) => b.append_null(),
                (_, Some(b), _) => b.append_null(),
                _ => {}
            }
        }
        geom.append_value(&f.geometry);
    }
    let mut out: Vec<ArrayRef> = Vec::new();
    for (t, i) in text.iter_mut().zip(ints.iter_mut()) {
        match (t, i) {
            (Some(b), _) => out.push(Arc::new(b.finish())),
            (_, Some(b)) => out.push(Arc::new(b.finish())),
            _ => unreachable!("a column is text or an integer"),
        }
    }
    out.push(Arc::new(geom.finish()));
    RecordBatch::try_new(schema(s, layer), out).map_err(err)
}

/// The writer of the file of one layer: [`GdalWriter::write`] the rows, then [`GdalWriter::close`].
pub struct GdalWriter<W: Write + Send> {
    schema: &'static Layers,
    layer: u8,
    w: ParWriter<W>,
    rows: Vec<Feature>,
    count: usize,
}

impl<W: Write + Send> GdalWriter<W> {
    pub fn new(schema: &'static Layers, layer: u8, out: W) -> Result<Self, String> {
        let props = WriterProperties::builder()
            .set_compression(compression())
            .set_max_row_group_row_count(Some(ROW_GROUP))
            .set_key_value_metadata(Some(vec![geo_metadata(geometry_type(schema, layer))]))
            .build();
        let w = ParWriter::try_new(out, self::schema(schema, layer), Some(props)).map_err(err)?;
        Ok(GdalWriter { schema, layer, w, rows: Vec::new(), count: 0 })
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
            let b = batch(self.schema, self.layer, &std::mem::take(&mut self.rows))?;
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
