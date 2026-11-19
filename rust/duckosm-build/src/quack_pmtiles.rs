//! `duckosm quack --pmtiles` (docs/exports/pmtiles.md): vector tiles of the Shortbread schema of a whole `.osm.pbf` file
//! in one PMTiles file, zoom 0 to 14.
//!
//! 1. The quack engine reads the file one time (or tile by tile, `--tile-zoom`) and gives its features.
//! 2. The Shortbread rules (`duckosm_core::shortbread`) give the layer features of each feature. Each layer feature is
//!    simplified for each zoom from its first zoom to 14, clipped to each tile that it touches (`duckosm_core::mvt`) and
//!    written as one row (tile id, layer, order, attributes, geometry in tile units) to a sort with a memory budget
//!    (`duckosm_core::spill::Sorter`, temporary files in the temporary store).
//! 3. The rows come back in the order of the tile ids. The rows of a tile become one MVT tile (with gzip) and go at once
//!    to the PMTiles writer (`duckosm_core::pmtiles`). No tile waits in memory: the memory is the sort budget, one batch
//!    of tiles and the directory entries.

use duckosm_core::mvt::{self, Shape, TileFeature, TileGeom, Value};
use duckosm_core::pmtiles;
use duckosm_core::quack::{self, Feature, TileSize};
use duckosm_core::shortbread::{self, LAYERS};
use duckosm_core::spill::{temp_store, Codec, Sorter};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};

/// The result of a run.
#[derive(Debug, Default)]
pub struct PmtilesCounts {
    /// The features that the quack engine gave.
    pub features: usize,
    /// The tiles (addressed) and the bytes of the file.
    pub tiles: u64,
    pub bytes: u64,
    /// The features of each layer at each zoom (after the join of equal features), by layer name.
    pub layer_zoom: BTreeMap<&'static str, [u64; 15]>,
}

/// The memory of the sort of the rows.
const SORT_BUDGET: usize = quack::BUDGET / 2;
/// The rows of the tiles that are encoded at once on all threads.
const TILE_BATCH: usize = 64 << 20;

/// Makes the PMTiles file of `pbf` (`open` opens it) into `out`. `tiles`: tile by tile, as quack by tiles. `fixes`: the
/// OSM fixes of `duckosm build --fixes`.
pub fn quack_pmtiles_to<R: Read + Seek + Send, W: Write + Seek>(
    open: impl Fn() -> Result<R, String>,
    out: W,
    tiles: Option<TileSize>,
    fixes: Option<quack::QuackFixes>,
) -> Result<PmtilesCounts, String> {
    let io = |e: std::io::Error| format!("quack --pmtiles: {e}");
    let store = temp_store();
    let mut sorter = Sorter::new(row_key, SORT_BUDGET, &*store);
    let mut counts = PmtilesCounts::default();
    // the box of the data in world units
    let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut push = |rows: Vec<(Vec<Vec<u8>>, [f64; 4])>| -> Result<(), String> {
        for (rows, b) in rows {
            bbox = [bbox[0].min(b[0]), bbox[1].min(b[1]), bbox[2].max(b[2]), bbox[3].max(b[3])];
            for r in rows {
                sorter.push(&Bytes, &r).map_err(io)?;
            }
        }
        Ok(())
    };
    // the lines of the boundaries come from the member ways of the boundary relations, also the ways with no tags
    let lines = shortbread::boundary_lines(&open)?;
    push(lines.par_iter().map(|(way, o)| rows_of_out(o, fid("way", *way))).collect())?;
    duckosm_core::util::step("quack --pmtiles: the boundaries");
    let mut features = 0;
    let mut take = |batch: Vec<Feature>| -> Result<(), String> {
        features += batch.len();
        push(batch.par_iter().map(|f| rows_of(f)).collect())
    };
    match tiles {
        None => quack::features(&open, quack::BUDGET, fixes.as_ref(), &mut take)?,
        Some(size) => quack::features_by_tiles(&open, quack::BUDGET, size, fixes, &mut take)?,
    };
    counts.features = features;
    duckosm_core::util::step("quack --pmtiles: the rows of the tiles");

    let mut w = pmtiles::Writer::new(out).map_err(io)?;
    let mut fields: Vec<BTreeMap<String, &'static str>> = vec![BTreeMap::new(); LAYERS.len()];
    let mut zooms = vec![(u8::MAX, 0u8); LAYERS.len()];
    let mut layer_zoom = vec![[0u64; 15]; LAYERS.len()];
    let mut batch: Vec<(u64, Vec<Vec<u8>>)> = Vec::new();
    let mut batch_bytes = 0;
    let mut flush = |batch: &mut Vec<(u64, Vec<Vec<u8>>)>, w: &mut pmtiles::Writer<W>| -> Result<(), String> {
        let tiles: Vec<(u64, Vec<u8>, Vec<Made>)> = batch
            .par_drain(..)
            .map(|(id, rows)| {
                let (mvt, made) = tile_of(&rows);
                (id, pmtiles::gzip(&mvt), made)
            })
            .collect();
        for (id, gz, made) in tiles {
            let (z, ..) = pmtiles::tile_zxy(id);
            for m in made {
                let l = m.layer as usize;
                layer_zoom[l][z as usize] += m.features;
                zooms[l] = (zooms[l].0.min(z), zooms[l].1.max(z));
                for (k, t) in m.fields {
                    fields[l].entry(k).or_insert(t);
                }
            }
            w.add(id, &gz).map_err(io)?;
        }
        Ok(())
    };
    for row in sorter.sorted(&Bytes).map_err(io)? {
        let row = row.map_err(io)?;
        let id = u64::from_be_bytes(row[..8].try_into().unwrap());
        if batch.last().is_none_or(|t| t.0 != id) {
            if batch_bytes >= TILE_BATCH {
                flush(&mut batch, &mut w)?;
                batch_bytes = 0;
            }
            batch.push((id, Vec::new()));
        }
        batch_bytes += row.len();
        batch.last_mut().unwrap().1.push(row);
    }
    flush(&mut batch, &mut w)?;
    duckosm_core::util::step("quack --pmtiles: the tiles");

    // the header and the metadata
    let to_ll = duckosm_core::mvt::lon_lat;
    let ((w0, n0), (e0, s0)) = if bbox[0] <= bbox[2] { (to_ll([bbox[0], bbox[1]]), to_ll([bbox[2], bbox[3]])) } else { ((0.0, 0.0), (0.0, 0.0)) };
    let span = (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]).max(1e-9);
    // the first zoom at which the area is less than one tile: an exact compare, the same on each platform
    let center_zoom = (0..14u8).find(|&z| span * f64::from(1u32 << (z + 1)) > 1.0).unwrap_or(14);
    let mut layers = Vec::new();
    for (l, layer) in LAYERS.iter().enumerate() {
        if zooms[l].0 == u8::MAX {
            continue;
        }
        let f: serde_json::Map<String, serde_json::Value> = fields[l].iter().map(|(k, t)| (k.clone(), serde_json::Value::from(*t))).collect();
        layers.push(serde_json::json!({ "id": layer.name, "fields": f, "minzoom": zooms[l].0, "maxzoom": zooms[l].1 }));
        counts.layer_zoom.insert(layer.name, layer_zoom[l]);
    }
    let metadata = serde_json::json!({
        "name": "Shortbread",
        "description": "Shortbread 1.0 vector tiles (https://shortbread-tiles.org/schema/1.0/) made by duckosm quack --pmtiles",
        "version": "1.0",
        "type": "baselayer",
        "format": "pbf",
        "generator": concat!("duckOSM ", env!("CARGO_PKG_VERSION")),
        "attribution": "<a href=\"https://www.openstreetmap.org/copyright\">© OpenStreetMap contributors</a>",
        "vector_layers": layers,
    });
    let min_zoom = zooms.iter().map(|z| z.0).min().unwrap_or(0).min(14);
    let info = pmtiles::Info { min_zoom, max_zoom: 14, bounds: [w0, s0, e0, n0], center_zoom, metadata: metadata.to_string() };
    counts.tiles = w.addressed();
    let mut out = w.finish(&info).map_err(io)?;
    counts.bytes = out.seek(std::io::SeekFrom::End(0)).map_err(io)?;
    Ok(counts)
}

/// The rows of a sort are bytes.
struct Bytes;

impl Codec<Vec<u8>> for Bytes {
    fn put(&self, row: &Vec<u8>, out: &mut Vec<u8>) {
        out.extend_from_slice(row);
    }
    fn get(&self, b: &mut &[u8]) -> Vec<u8> {
        let v = b.to_vec();
        *b = &[];
        v
    }
}

/// The key of a row: the tile id, the layer, the order and the OSM element (its first 25 bytes; big endian, so the bytes
/// sort as the numbers). With the element, the order of the features in a tile is the same in each run.
fn row_key(b: &[u8]) -> [u8; 25] {
    b[..25].try_into().unwrap()
}

/// The OSM element of a feature as one number: the kind (node, way, relation) in the top bits, then the id.
fn fid(kind: &str, id: i64) -> u64 {
    let k = match kind {
        "node" => 0u64,
        "way" => 1,
        _ => 2,
    };
    (k << 61) | ((id as u64) & ((1 << 61) - 1))
}

/// The rows of a feature of the quack engine, and the box of its layer features.
fn rows_of(f: &Feature) -> (Vec<Vec<u8>>, [f64; 4]) {
    let mut rows = Vec::new();
    let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let (kind, id) = f.feature_id.split_once('/').unwrap_or(("", "0"));
    let fid = fid(kind, id.parse().unwrap_or(0));
    for o in shortbread::features(&f.feature_id, &f.tags, &f.geometry) {
        let (r, b) = rows_of_out(&o, fid);
        rows.extend(r);
        bbox = [bbox[0].min(b[0]), bbox[1].min(b[1]), bbox[2].max(b[2]), bbox[3].max(b[3])];
    }
    (rows, bbox)
}

/// The rows of a layer feature (one for each tile of each zoom), and its box.
fn rows_of_out(o: &shortbread::Out, fid: u64) -> (Vec<Vec<u8>>, [f64; 4]) {
    let mut rows = Vec::new();
    let bbox = o.shape.bbox();
    {
        let mut attrs = Vec::new();
        let mut last_attrs = u8::MAX;
        for z in o.minzoom..=mvt::MAX_ZOOM {
            let tol = shortbread::tolerance(o.layer, z);
            let shape: std::borrow::Cow<Shape> = if tol == 0.0 {
                std::borrow::Cow::Borrowed(&o.shape)
            } else {
                match o.shape.simplify(tol) {
                    Some(s) => std::borrow::Cow::Owned(s),
                    None => continue,
                }
            };
            // the attributes of the zoom (an attribute can start at a later zoom)
            let n = o.attrs.iter().filter(|a| a.2 <= z).count() as u8;
            if n != last_attrs {
                last_attrs = n;
                attrs.clear();
                put_attrs(o.attrs.iter().filter(|a| a.2 <= z).map(|a| (a.0, &a.1)), &mut attrs);
            }
            shape.tiles(z, &mut |x, y, part| {
                if let Some(g) = mvt::quantize(part, z, x, y) {
                    let mut r = Vec::with_capacity(32 + attrs.len());
                    r.extend_from_slice(&pmtiles::tile_id(z, x, y).to_be_bytes());
                    r.push(o.layer);
                    r.extend_from_slice(&((o.sort as u64) ^ (1 << 63)).to_be_bytes());
                    r.extend_from_slice(&fid.to_be_bytes());
                    r.extend_from_slice(&(attrs.len() as u32).to_le_bytes());
                    r.extend_from_slice(&attrs);
                    put_geom(&g, &mut r);
                    rows.push(r);
                }
            });
        }
    }
    (rows, bbox)
}

fn put_attrs<'a>(attrs: impl Iterator<Item = (&'a str, &'a Value)>, out: &mut Vec<u8>) {
    for (k, v) in attrs {
        out.push(k.len() as u8);
        out.extend_from_slice(k.as_bytes());
        match v {
            Value::Str(s) => {
                out.push(0);
                mvt::varint(out, s.len() as u64);
                out.extend_from_slice(s.as_bytes());
            }
            Value::Int(i) => {
                out.push(1);
                out.extend_from_slice(&i.to_le_bytes());
            }
            Value::Float(f) => {
                out.push(2);
                out.extend_from_slice(&f.to_le_bytes());
            }
            Value::Bool(b) => out.extend_from_slice(&[3, *b as u8]),
        }
    }
}

fn get_attrs(mut b: &[u8]) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let n = b[0] as usize;
        let k = String::from_utf8_lossy(&b[1..1 + n]).into_owned();
        b = &b[1 + n..];
        let tag = b[0];
        b = &b[1..];
        let v = match tag {
            0 => {
                let len = varint(&mut b) as usize;
                let s = String::from_utf8_lossy(&b[..len]).into_owned();
                b = &b[len..];
                Value::Str(s)
            }
            1 | 2 => {
                let v = <[u8; 8]>::try_from(&b[..8]).unwrap();
                b = &b[8..];
                if tag == 1 {
                    Value::Int(i64::from_le_bytes(v))
                } else {
                    Value::Float(f64::from_le_bytes(v))
                }
            }
            _ => {
                let v = b[0] != 0;
                b = &b[1..];
                Value::Bool(v)
            }
        };
        out.push((k, v));
    }
    out
}

fn varint(b: &mut &[u8]) -> u64 {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let byte = b[0];
        *b = &b[1..];
        v |= ((byte & 0x7f) as u64) << shift;
        if byte < 0x80 {
            return v;
        }
        shift += 7;
    }
}

/// The geometry of a row: the type, the parts, and the points of each part as zigzag deltas.
fn put_geom(g: &TileGeom, out: &mut Vec<u8>) {
    let zz = |v: i32| ((v << 1) ^ (v >> 31)) as u32 as u64;
    out.push(g.kind);
    mvt::varint(out, g.parts.len() as u64);
    let mut cur = [0i32; 2];
    for p in &g.parts {
        mvt::varint(out, p.len() as u64);
        for q in p {
            mvt::varint(out, zz(q[0] - cur[0]));
            mvt::varint(out, zz(q[1] - cur[1]));
            cur = *q;
        }
    }
}

fn get_geom(mut b: &[u8]) -> TileGeom {
    let unzz = |v: u64| ((v >> 1) as i32) ^ -((v & 1) as i32);
    let kind = b[0];
    b = &b[1..];
    let n = varint(&mut b) as usize;
    let mut cur = [0i32; 2];
    let mut parts = Vec::with_capacity(n);
    for _ in 0..n {
        let len = varint(&mut b) as usize;
        let mut p = Vec::with_capacity(len);
        for _ in 0..len {
            cur = [cur[0] + unzz(varint(&mut b)), cur[1] + unzz(varint(&mut b))];
            p.push(cur);
        }
        parts.push(p);
    }
    TileGeom { kind, parts }
}

/// What a tile has of a layer: its features and the fields (name and type) of their attributes.
struct Made {
    layer: u8,
    features: u64,
    fields: Vec<(String, &'static str)>,
}

/// The MVT tile of the rows of one tile (in the order of the sort: by layer, then by order).
fn tile_of(rows: &[Vec<u8>]) -> (Vec<u8>, Vec<Made>) {
    let mut tile = Vec::new();
    let mut made = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let layer = rows[i][8];
        let mut j = i;
        while j < rows.len() && rows[j][8] == layer {
            j += 1;
        }
        // the features of the layer; with `combine`, the parts of features with the same order, attributes and type
        // join the first of them
        let mut features: Vec<((&[u8], &[u8]), TileGeom)> = Vec::new();
        let mut first = duckosm_core::util::FxHashMap::<((&[u8], &[u8]), u8), usize>::default();
        for r in &rows[i..j] {
            // the order and the attributes (not the element: the features of other elements join)
            let n = u32::from_le_bytes(r[25..29].try_into().unwrap()) as usize;
            let key = (&r[9..17], &r[29..29 + n]);
            let g = get_geom(&r[29 + n..]);
            if LAYERS[layer as usize].combine && g.kind != mvt::POINT {
                if let Some(&at) = first.get(&(key, g.kind)) {
                    features[at].1.parts.extend(g.parts);
                    continue;
                }
                first.insert((key, g.kind), features.len());
            }
            features.push((key, g));
        }
        let features: Vec<TileFeature> = features.into_iter().map(|(key, geom)| TileFeature { geom, attrs: get_attrs(key.1) }).collect();
        let mut fields: Vec<(String, &'static str)> = Vec::new();
        for f in &features {
            for (k, v) in &f.attrs {
                if !fields.iter().any(|(fk, _)| fk == k) {
                    fields.push((k.clone(), v.type_name()));
                }
            }
        }
        mvt::write_layer(&mut tile, LAYERS[layer as usize].name, &features);
        made.push(Made { layer, features: features.len() as u64, fields });
        i = j;
    }
    (tile, made)
}

/// [`quack_pmtiles_to`] of the file `pbf` into the file `out`.
pub fn quack_pmtiles(
    pbf: &std::path::Path,
    out: &std::path::Path,
    tiles: Option<TileSize>,
    fixes: Option<&duckosm_core::fixes::OsmFixes>,
) -> Result<PmtilesCounts, String> {
    let open = || {
        let f = duckosm_core::fs::File::open(pbf).map_err(|e| format!("{}: {e}", pbf.display()))?;
        Ok(std::io::BufReader::with_capacity(1 << 20, f))
    };
    let fixes = fixes.map(|f| quack::QuackFixes::new(f, open)).transpose()?;
    let f = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    quack_pmtiles_to(open, std::io::BufWriter::with_capacity(1 << 20, f), tiles, fixes)
}
