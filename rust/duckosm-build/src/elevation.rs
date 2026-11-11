//! The elevation step (duckOSM `duckosm elevation`): sample a DEM at every node and at every vertex of every
//! edge. One raster reader for the family: a reader that needs the heights along a way (SimReady's
//! `import-elevation`) reads `z_vertices` and never opens the raster.
//!
//! The DEM is a GeoTIFF in EPSG:4326 with one band of 32-bit floats, uncompressed or deflate (a crop of a
//! Copernicus GLO-30 tile, for example). A value is the pixel that holds the point, as rasterio's `sample`
//! takes it; a point outside the raster, a nodata value or a value that is not finite gets `nodata_fill`
//! (0.0, as duckOSM). The heights are kept to 2 decimals (cm), as duckOSM keeps them.
//!
//! The step also writes the pixels of the DEM over the network (`main.elevation_grid`): the ground between the
//! roads, for a reader that makes a terrain (SimReady's `gltf` adapter). The pixels are the raster's own, with
//! no resampling.

use duckosm_core::model::{Edge, Node};
use std::io::{Read, Seek};
use std::path::Path;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

pub struct Dem {
    width: usize,
    height: usize,
    /// The west and the north edge of the raster, in degrees.
    x0: f64,
    y0: f64,
    /// The pixel size in degrees.
    sx: f64,
    sy: f64,
    data: Vec<f32>,
    nodata: Option<f64>,
    pub resolution_m: f64,
}

impl Dem {
    pub fn open(path: &Path) -> Result<Dem, String> {
        let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Dem::from_reader(std::io::BufReader::new(f), &path.display().to_string())
    }

    /// The DEM from the bytes of a GeoTIFF (a file, or the bytes that a browser gives); `name` goes in the errors.
    pub fn from_reader<R: Read + Seek>(r: R, name: &str) -> Result<Dem, String> {
        let err = |e: tiff::TiffError| format!("{name}: {e}");
        let mut d = Decoder::new(r).map_err(err)?;
        let (w, h) = d.dimensions().map_err(err)?;
        let scale = d.get_tag_f64_vec(Tag::ModelPixelScaleTag).map_err(err)?;
        let tie = d.get_tag_f64_vec(Tag::ModelTiepointTag).map_err(err)?;
        let keys = d
            .find_tag_unsigned_vec::<u16>(Tag::GeoKeyDirectoryTag)
            .map_err(err)?
            .unwrap_or_default();
        let key = |k: u16| keys[4..].chunks(4).find(|e| e[0] == k).map(|e| e[3]);
        if keys.len() < 4 || key(2048) != Some(4326) {
            return Err(format!("{name}: the DEM must be in EPSG:4326"));
        }
        let nodata = match d.find_tag(Tag::GdalNodata).map_err(err)? {
            Some(_) => d
                .get_tag_ascii_string(Tag::GdalNodata)
                .map_err(err)?
                .trim_matches(char::from(0))
                .trim()
                .parse::<f64>()
                .ok(),
            None => None,
        };
        let data = match d.read_image().map_err(err)? {
            DecodingResult::F32(v) => v,
            _ => {
                return Err(format!("{name}: the DEM must have 32-bit float values"))
            }
        };
        let (sx, sy) = (scale[0], scale[1]);
        // the tie point is the corner of the first pixel; a raster of kind PixelIsPoint (GTRasterTypeGeoKey 2)
        // names its centre, so GDAL moves the corner by half a pixel, and so does this reader
        let (mut x0, mut y0) = (tie[3] - tie[0] * sx, tie[4] + tie[1] * sy);
        if key(1025) == Some(2) {
            x0 -= sx / 2.0;
            y0 += sy / 2.0;
        }
        Ok(Dem {
            width: w as usize,
            height: h as usize,
            x0,
            y0,
            sx,
            sy,
            data,
            nodata,
            // duckOSM's _dem_resolution_m: degrees to metres at 111,320 m a degree, 3 decimals
            resolution_m: (sx.abs().min(sy.abs()) * 111_320.0 * 1000.0).round() / 1000.0,
        })
    }

    /// The height at a point, or None outside the raster or on nodata.
    pub fn sample(&self, lon: f64, lat: f64) -> Option<f64> {
        let (right, bottom) = (
            self.x0 + self.width as f64 * self.sx,
            self.y0 - self.height as f64 * self.sy,
        );
        if !(lon >= self.x0 && lon <= right && lat >= bottom && lat <= self.y0) {
            return None;
        }
        let col = ((lon - self.x0) / self.sx).floor() as usize;
        let row = ((self.y0 - lat) / self.sy).floor() as usize;
        let z = f64::from(
            *self
                .data
                .get(row.min(self.height - 1) * self.width + col.min(self.width - 1))?,
        );
        (z.is_finite() && Some(z) != self.nodata).then_some(z)
    }
}

/// One pixel of the DEM: its column and row in the grid (0 at the west and the north edge), the lon and the lat of
/// its centre, and its height to the cm (None on nodata).
pub struct GridCell {
    pub col: i32,
    pub row: i32,
    pub lon: f64,
    pub lat: f64,
    pub z: Option<f64>,
}

impl Dem {
    /// The pixels over a box (degrees) plus `margin` pixels on each side, inside the raster, row by row from the
    /// north. Empty when the box is outside the raster.
    pub fn grid(&self, west: f64, south: f64, east: f64, north: f64, margin: usize) -> Vec<GridCell> {
        let col = |lon: f64| ((lon - self.x0) / self.sx).floor();
        let row = |lat: f64| ((self.y0 - lat) / self.sy).floor();
        let m = margin as f64;
        let (c0, c1) = ((col(west) - m).max(0.0), (col(east) + m).min(self.width as f64 - 1.0));
        let (r0, r1) = ((row(north) - m).max(0.0), (row(south) + m).min(self.height as f64 - 1.0));
        if !(c0 <= c1 && r0 <= r1) {
            return Vec::new();
        }
        let (c0, c1, r0, r1) = (c0 as usize, c1 as usize, r0 as usize, r1 as usize);
        let mut cells = Vec::with_capacity((c1 - c0 + 1) * (r1 - r0 + 1));
        for r in r0..=r1 {
            for c in c0..=c1 {
                let z = f64::from(self.data[r * self.width + c]);
                cells.push(GridCell {
                    col: (c - c0) as i32,
                    row: (r - r0) as i32,
                    lon: self.x0 + (c as f64 + 0.5) * self.sx,
                    lat: self.y0 - (r as f64 + 0.5) * self.sy,
                    z: (z.is_finite() && Some(z) != self.nodata).then(|| round_cm(z)),
                });
            }
        }
        cells
    }
}

/// Python's `round(x, 2)` for the values of a DEM: to the nearest cm, a tie to the even cm.
pub fn round_cm(x: f64) -> f64 {
    let y = x * 100.0;
    let r = y.round();
    let r = if (y - y.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        r - y.signum()
    } else {
        r
    };
    r / 100.0
}

/// A node's height: (node_id, ele).
pub struct NodeZ {
    pub node_id: i64,
    pub ele: f64,
}

/// An edge's heights: (edge_id, z_from, z_to, z_vertices).
pub struct EdgeZ {
    pub edge_id: i64,
    pub z_from: f64,
    pub z_to: f64,
    pub z_vertices: Vec<f64>,
}

/// The heights of one mode, and the number of nodes with no value (they get `fill`).
pub fn sample_mode(
    dem: &Dem,
    nodes: &[Node],
    edges: &[Edge],
    fill: f64,
) -> (Vec<NodeZ>, Vec<EdgeZ>, usize) {
    let mut nodata = 0;
    let at = |lon, lat| round_cm(dem.sample(lon, lat).unwrap_or(fill));
    let node_rows: Vec<NodeZ> = nodes
        .iter()
        .map(|n| {
            if dem.sample(n.lon, n.lat).is_none() {
                nodata += 1;
            }
            NodeZ {
                node_id: n.node_id,
                ele: at(n.lon, n.lat),
            }
        })
        .collect();
    let ele: std::collections::HashMap<i64, f64> =
        node_rows.iter().map(|n| (n.node_id, n.ele)).collect();
    let edge_rows = edges
        .iter()
        .map(|e| EdgeZ {
            edge_id: e.edge_id,
            z_from: ele.get(&e.source).copied().unwrap_or(fill),
            z_to: ele.get(&e.target).copied().unwrap_or(fill),
            z_vertices: e.geometry.iter().map(|&(x, y)| at(x, y)).collect(),
        })
        .collect();
    (node_rows, edge_rows, nodata)
}

#[cfg(test)]
mod tests {
    use super::{round_cm, write_area_dem, Dem};
    use std::path::Path;

    /// The Monaco crop of Copernicus GLO-30, cut to a smaller box and written again: the same heights at the same points, and the box is inside the file.
    #[test]
    fn area_dem_keeps_the_heights() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/monaco-copernicus-glo30.tif");
        let out = std::env::temp_dir().join(format!("duckosm-area-dem-{}.tif", std::process::id()));
        let bbox = [7.41, 43.725, 7.43, 43.745];
        let (w, h) = write_area_dem(&[src.as_path()], bbox, 2, &out).unwrap();
        assert!(w > 70 && w < 80 && h > 70 && h < 80, "{w} × {h}");
        let (a, b) = (Dem::open(&src).unwrap(), Dem::open(&out).unwrap());
        for (lon, lat) in [(7.415, 43.73), (7.4201, 43.7377), (7.428, 43.744)] {
            assert_eq!(a.sample(lon, lat), b.sample(lon, lat), "{lon} {lat}");
        }
        assert!(b.sample(bbox[0], bbox[1]).is_some() && b.sample(bbox[2], bbox[3]).is_some());
        std::fs::remove_file(out).ok();
    }

    #[test]
    fn round_cm_ties_to_even() {
        assert_eq!(round_cm(52.125), 52.12);
        assert_eq!(round_cm(52.375), 52.38);
        assert_eq!(round_cm(1.005), 1.0);
        assert_eq!(round_cm(-3.456), -3.46);
        assert_eq!(round_cm(12.3), 12.3);
    }
}

/// Join the DEM files `inputs` (tiles of one model, for example the 1° tiles of Copernicus GLO-30) over the box `bbox` (degrees) plus `margin`
/// pixels, and write them as one GeoTIFF `out`: EPSG:4326, 32-bit floats, deflate, nodata -9999. The pixel size is the smallest of the inputs; a
/// pixel takes the value of the first input that has the point. Gives the width and the height. `duckosm studio` uses it for a downloaded model.
pub fn write_area_dem(inputs: &[&Path], bbox: [f64; 4], margin: usize, out: &Path) -> Result<(usize, usize), String> {
    use tiff::encoder::{colortype::Gray32Float, Compression, DeflateLevel, TiffEncoder};
    const NODATA: f32 = -9999.0;
    let dems = inputs.iter().map(|p| Dem::open(p)).collect::<Result<Vec<_>, _>>()?;
    let first = dems.first().ok_or("no DEM file to join")?;
    let (sx, sy) = dems.iter().fold((first.sx.abs(), first.sy.abs()), |(a, b), d| (a.min(d.sx.abs()), b.min(d.sy.abs())));
    let m = margin as f64;
    // the grid of the first file, so that its pixels are not moved: the west and the north edge are on its pixel lines
    let x0 = first.x0 + ((bbox[0] - first.x0) / sx).floor() * sx - m * sx;
    let y0 = first.y0 - ((first.y0 - bbox[3]) / sy).floor() * sy + m * sy;
    let width = (((bbox[2] - x0) / sx).ceil() + m) as usize;
    let height = (((y0 - bbox[1]) / sy).ceil() + m) as usize;
    if width == 0 || height == 0 || width * height > 400_000_000 {
        return Err(format!("the area gives a DEM of {width} × {height} pixels: too large"));
    }
    let mut data = vec![NODATA; width * height];
    for row in 0..height {
        let lat = y0 - (row as f64 + 0.5) * sy;
        for col in 0..width {
            let lon = x0 + (col as f64 + 0.5) * sx;
            if let Some(z) = dems.iter().find_map(|d| d.sample(lon, lat)) {
                data[row * width + col] = z as f32;
            }
        }
    }
    let err = |e: tiff::TiffError| format!("{}: {e}", out.display());
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut enc = TiffEncoder::new(std::io::BufWriter::new(file)).map_err(err)?.with_compression(Compression::Deflate(DeflateLevel::default()));
    let mut img = enc.new_image::<Gray32Float>(width as u32, height as u32).map_err(err)?;
    let dir = img.encoder();
    dir.write_tag(Tag::ModelPixelScaleTag, &[sx, sy, 0.0][..]).map_err(err)?;
    dir.write_tag(Tag::ModelTiepointTag, &[0.0, 0.0, 0.0, x0, y0, 0.0][..]).map_err(err)?;
    // GeoKeys: version 1.1.0, 3 keys; GTModelType geographic, GTRasterType PixelIsArea, GeographicType EPSG:4326
    dir.write_tag(Tag::GeoKeyDirectoryTag, &[1u16, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326][..]).map_err(err)?;
    dir.write_tag(Tag::GdalNodata, "-9999").map_err(err)?;
    img.write_data(&data).map_err(err)?;
    Ok((width, height))
}

/// Write the pixels of the DEM file `dem` over the box (degrees) plus `margin` pixels as `elevation_grid_main.parquet` rows into `out` (the same rows
/// that a build with a DEM writes). Gives the number of pixels. Python's `duckosm studio` uses it: its build has no DEM step of the engine.
pub fn write_grid_file(dem: &Path, out: &Path, bbox: [f64; 4], margin: usize) -> Result<usize, String> {
    let cells = Dem::open(dem)?.grid(bbox[0], bbox[1], bbox[2], bbox[3], margin);
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    duckosm_parquet::write_elevation_grid_to(file, &cells.iter().map(|c| (c.col, c.row, c.lon, c.lat, c.z)).collect::<Vec<_>>())?;
    Ok(cells.len())
}
