//! The elevation step (duckOSM `duckosm elevation`): sample a DEM at every node and at every vertex of every
//! edge. One raster reader for the family: a reader that needs the heights along a way (SimReady's
//! `import-elevation`) reads `z_vertices` and never opens the raster.
//!
//! The DEM is a GeoTIFF in EPSG:4326 with one band of 32-bit floats, uncompressed or deflate (a crop of a
//! Copernicus GLO-30 tile, for example). A value is the pixel that holds the point, as rasterio's `sample`
//! takes it; a point outside the raster, a nodata value or a value that is not finite gets `nodata_fill`
//! (0.0, as duckOSM). The heights are kept to 2 decimals (cm), as duckOSM keeps them.

use duckosm_core::model::{Edge, Node};
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
        let err = |e: tiff::TiffError| format!("{}: {e}", path.display());
        let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut d = Decoder::new(std::io::BufReader::new(f)).map_err(err)?;
        let (w, h) = d.dimensions().map_err(err)?;
        let scale = d.get_tag_f64_vec(Tag::ModelPixelScaleTag).map_err(err)?;
        let tie = d.get_tag_f64_vec(Tag::ModelTiepointTag).map_err(err)?;
        let keys = d
            .find_tag_unsigned_vec::<u16>(Tag::GeoKeyDirectoryTag)
            .map_err(err)?
            .unwrap_or_default();
        let key = |k: u16| keys[4..].chunks(4).find(|e| e[0] == k).map(|e| e[3]);
        if keys.len() < 4 || key(2048) != Some(4326) {
            return Err(format!("{}: the DEM must be in EPSG:4326", path.display()));
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
                return Err(format!(
                    "{}: the DEM must have 32-bit float values",
                    path.display()
                ))
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
    use super::round_cm;

    #[test]
    fn round_cm_ties_to_even() {
        assert_eq!(round_cm(52.125), 52.12);
        assert_eq!(round_cm(52.375), 52.38);
        assert_eq!(round_cm(1.005), 1.0);
        assert_eq!(round_cm(-3.456), -3.46);
        assert_eq!(round_cm(12.3), 12.3);
    }
}
