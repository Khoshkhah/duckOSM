//! The shapefiles of `duckosm quack --geofabrik -f shp` (docs/concepts/geofabrik.md): one zip file with a shapefile for
//! each layer of a `Schema`, as Geofabrik's `<area>-latest-free.shp.zip`: `gis_osm_roads_free_1.shp`, `.shx`, `.dbf`
//! (the columns with Geofabrik's widths), `.prj` (WGS 84) and `.cpg` (UTF-8). Pure Rust (the crates `shapefile`, `dbase`
//! and `zip`), so the browser makes the same file.
//!
//! The files of a layer grow in temporary files (`duckosm_core::spill`); a layer goes into the zip file when it is
//! complete. A shapefile holds at most 2 GB: a larger layer continues in `_2`, `_3`, ... (Geofabrik's "spillover").

use duckosm_core::gdal::{Schema, Value};
use duckosm_core::quack::Feature;
use duckosm_core::spill::{Blob, Store};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

/// The most bytes of a `.shp` or a `.dbf` file.
const LIMIT: u64 = (1 << 31) - 1;

/// The `.prj` of Geofabrik's files.
const PRJ: &str = r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS_1984",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["Degree",0.017453292519943295]]"#;

/// A temporary file that the writers of `shapefile` and `dbase` write and this module reads after them.
#[derive(Clone)]
struct Shared(Arc<Mutex<Box<dyn Blob>>>);

impl Write for Shared {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("one writer").write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().expect("one writer").flush()
    }
}

impl Seek for Shared {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.0.lock().expect("one writer").seek(to)
    }
}

/// The open part of a layer. A buffer before each file: `dbase` writes each field and each space of its padding with a
/// write of its own (Kentucky took 580 s with no buffer).
struct Part {
    w: shapefile::Writer<BufWriter<Shared>>,
    files: [Shared; 3],
    shp: u64,
    dbf: u64,
}

/// The shapefiles of the layers of a `Schema` into one zip file.
pub struct ShpZip<W: Write> {
    schema: &'static Schema,
    zip: zip::ZipWriter<zip::write::StreamWriter<W>>,
    store: Box<dyn Store>,
    /// the open part of each layer (index: the layer) and the number of its parts
    open: Vec<Option<Part>>,
    parts: Vec<u32>,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("quack --geofabrik: the shapefiles: {e}")
}

/// The values of a row for `dbase`: text, or a number.
struct Record(Vec<(bool, Option<Value>)>);

impl dbase::WritableRecord for Record {
    fn write_using<W: Write>(&self, w: &mut dbase::FieldWriter<'_, W>) -> Result<(), dbase::FieldError> {
        for (int, v) in &self.0 {
            match (int, v) {
                (true, Some(Value::Int(n))) => w.write_next_field_value(&Some(*n as f64))?,
                (true, _) => w.write_next_field_value(&None::<f64>)?,
                (false, Some(Value::Text(s))) => w.write_next_field_value(&s.as_str())?,
                (false, _) => w.write_next_field_value(&"")?,
            }
        }
        Ok(())
    }
}

/// The points of WKB (little endian, 2D) from `b`.
fn points(b: &mut &[u8]) -> Vec<shapefile::Point> {
    let n = u32::from_le_bytes(b[..4].try_into().expect("4 bytes"));
    *b = &b[4..];
    (0..n)
        .map(|_| {
            let x = f64::from_le_bytes(b[..8].try_into().expect("8 bytes"));
            let y = f64::from_le_bytes(b[8..16].try_into().expect("8 bytes"));
            *b = &b[16..];
            shapefile::Point::new(x, y)
        })
        .collect()
}

/// A shape of the WKB of a point, a line or a multipolygon.
enum Shape {
    Point(shapefile::Point),
    Line(shapefile::Polyline),
    Area(shapefile::Polygon),
}

impl Shape {
    fn of(wkb: &[u8]) -> Shape {
        let kind = u32::from_le_bytes(wkb[1..5].try_into().expect("4 bytes"));
        let mut b = &wkb[5..];
        match kind {
            1 => {
                let x = f64::from_le_bytes(b[..8].try_into().expect("8 bytes"));
                let y = f64::from_le_bytes(b[8..16].try_into().expect("8 bytes"));
                Shape::Point(shapefile::Point::new(x, y))
            }
            2 => Shape::Line(shapefile::Polyline::new(points(&mut b))),
            _ => {
                let polys = u32::from_le_bytes(b[..4].try_into().expect("4 bytes"));
                b = &b[4..];
                let mut rings = Vec::new();
                for _ in 0..polys {
                    b = &b[5..];
                    let n = u32::from_le_bytes(b[..4].try_into().expect("4 bytes"));
                    b = &b[4..];
                    for i in 0..n {
                        let r = points(&mut b);
                        rings.push(if i == 0 { shapefile::PolygonRing::Outer(r) } else { shapefile::PolygonRing::Inner(r) });
                    }
                }
                Shape::Area(shapefile::Polygon::with_rings(rings))
            }
        }
    }

    /// The bytes of the record in the `.shp` file.
    fn size(&self) -> u64 {
        8 + match self {
            Shape::Point(_) => 20,
            Shape::Line(l) => 44 + 4 * l.parts().len() as u64 + 16 * l.parts().iter().map(|p| p.len() as u64).sum::<u64>(),
            Shape::Area(a) => 44 + 4 * a.rings().len() as u64 + 16 * a.rings().iter().map(|r| r.len() as u64).sum::<u64>(),
        }
    }
}

/// The shape type of a layer (`.shp` header) from its geometry.
fn shape_type(geometry: &str) -> i32 {
    match geometry {
        "POINT" => 1,
        "LINESTRING" => 3,
        _ => 5,
    }
}

impl<W: Write> ShpZip<W> {
    pub fn new(schema: &'static Schema, out: W) -> ShpZip<W> {
        let n = schema.layers as usize + 1;
        ShpZip {
            schema,
            zip: zip::ZipWriter::new_stream(out),
            store: duckosm_core::spill::temp_store(),
            open: (0..n).map(|_| None).collect(),
            parts: vec![0; n],
        }
    }

    fn new_part(&mut self, layer: u8) -> Result<Part, String> {
        let file = || -> Result<Shared, String> { Ok(Shared(Arc::new(Mutex::new(self.store.create().map_err(err)?)))) };
        let files = [file()?, file()?, file()?];
        let mut dbf = dbase::TableWriterBuilder::with_encoding(dbase::UnicodeLossy);
        for (c, int, width) in (self.schema.columns)(layer) {
            let name = dbase::FieldName::try_from(c).map_err(err)?;
            dbf = if int { dbf.add_numeric_field(name, width, 0) } else { dbf.add_character_field(name, width.max(1)) };
        }
        let buf = |f: &Shared| BufWriter::with_capacity(1 << 20, f.clone());
        let w = shapefile::Writer::new(shapefile::ShapeWriter::with_shx(buf(&files[0]), buf(&files[1])), dbf.build_with_dest(buf(&files[2])));
        self.parts[layer as usize] += 1;
        Ok(Part { w, files, shp: 100, dbf: 0 })
    }

    /// Write a row into the shapefile of its layer.
    pub fn write(&mut self, f: &Feature) -> Result<(), String> {
        let layer = f.layer;
        let shape = Shape::of(&f.geometry);
        let cols = (self.schema.columns)(layer);
        let record_bytes = 1 + cols.iter().map(|c| c.2.max(1) as u64).sum::<u64>();
        let full = self.open[layer as usize].as_ref().is_some_and(|p| p.shp + shape.size() > LIMIT || p.dbf + record_bytes > LIMIT);
        if full {
            let p = self.open[layer as usize].take().expect("an open part");
            self.add(layer, p)?;
        }
        if self.open[layer as usize].is_none() {
            let p = self.new_part(layer)?;
            self.open[layer as usize] = Some(p);
        }
        let p = self.open[layer as usize].as_mut().expect("an open part");
        let record = Record(cols.iter().map(|c| c.1).zip((self.schema.values)(f)).collect());
        match &shape {
            Shape::Point(s) => p.w.write_shape_and_record(s, &record),
            Shape::Line(s) => p.w.write_shape_and_record(s, &record),
            Shape::Area(s) => p.w.write_shape_and_record(s, &record),
        }
        .map_err(err)?;
        p.shp += shape.size();
        p.dbf += record_bytes;
        Ok(())
    }

    /// Put a complete part of a layer into the zip file.
    fn add(&mut self, layer: u8, p: Part) -> Result<(), String> {
        let Part { w, files, .. } = p;
        // the drop of the writers writes the headers, and the drop of the buffers the rest
        drop(w);
        let name = format!("{}_{}", (self.schema.table)(layer), self.parts[layer as usize]);
        let geometry = (self.schema.geometry)(layer);
        for (ext, file) in ["shp", "shx", "dbf"].iter().zip(files) {
            let mut blob = Arc::try_unwrap(file.0).map_err(|_| err("a file is still open"))?.into_inner().expect("one writer");
            let len = blob.seek(SeekFrom::End(0)).map_err(err)?;
            blob.seek(SeekFrom::Start(0)).map_err(err)?;
            self.start(&format!("{name}.{ext}"))?;
            if len == 100 && *ext != "dbf" {
                // an empty layer: `shapefile` writes the type of no shape (0); Geofabrik's files have the type of the layer
                let mut head = [0u8; 100];
                io::Read::read_exact(&mut blob, &mut head).map_err(err)?;
                head[32..36].copy_from_slice(&shape_type(geometry).to_le_bytes());
                self.zip.write_all(&head).map_err(err)?;
            } else {
                io::copy(&mut blob, &mut self.zip).map_err(err)?;
            }
        }
        self.start(&format!("{name}.prj"))?;
        self.zip.write_all(PRJ.as_bytes()).map_err(err)?;
        self.start(&format!("{name}.cpg"))?;
        self.zip.write_all(b"UTF-8\n").map_err(err)
    }

    fn start(&mut self, name: &str) -> Result<(), String> {
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        self.zip.start_file(name, o).map_err(err)
    }

    /// Put the open parts (and an empty shapefile for each layer with no rows) into the zip file, and close it.
    pub fn close(mut self) -> Result<(), String> {
        for layer in 1..=self.schema.layers {
            let p = match self.open[layer as usize].take() {
                Some(p) => p,
                None if self.parts[layer as usize] == 0 => self.new_part(layer)?,
                None => continue,
            };
            self.add(layer, p)?;
        }
        self.zip.finish().map_err(err)?.flush().map_err(err)
    }
}
