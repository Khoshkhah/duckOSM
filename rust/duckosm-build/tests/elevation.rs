//! The DEM reader of the elevation step against rasterio's `sample` (the reader of Python duckOSM) on a crop of
//! the Copernicus GLO-30 tile N43 E007 over Monaco. The values are rasterio's, rounded to the cm.

use duckosm_build::elevation::{round_cm, Dem};
use std::path::Path;

#[test]
fn dem_samples_as_rasterio() {
    let dem = Dem::open(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/monaco-copernicus-glo30.tif"
    )))
    .unwrap();
    for (lon, lat, z) in [
        (7.4206, 43.7311, 70.55),
        (7.428, 43.74, 48.48),
        (7.413, 43.727, 11.06),
    ] {
        assert_eq!(round_cm(dem.sample(lon, lat).unwrap()), z, "{lon} {lat}");
    }
    assert!(dem.sample(8.0, 43.73).is_none());
    assert_eq!(dem.resolution_m, 30.922);
}

/// The grid is the raster's own pixels: each cell's height is the sample at its centre, the rows run from the
/// north, and the margin adds whole pixels on each side.
#[test]
fn grid_is_the_pixels_of_the_box() {
    let dem = Dem::open(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/monaco-copernicus-glo30.tif"
    )))
    .unwrap();
    let (w, s, e, n) = (7.415, 43.728, 7.425, 43.735);
    let plain = dem.grid(w, s, e, n, 0);
    let wide = dem.grid(w, s, e, n, 3);
    let size = |g: &[duckosm_build::elevation::GridCell]| {
        (g.iter().map(|c| c.col).max().unwrap() + 1, g.iter().map(|c| c.row).max().unwrap() + 1)
    };
    let ((pc, pr), (wc, wr)) = (size(&plain), size(&wide));
    assert_eq!((wc, wr), (pc + 6, pr + 6));
    assert_eq!(plain.len() as i32, pc * pr);
    for c in &wide {
        assert_eq!(c.z, dem.sample(c.lon, c.lat).map(round_cm), "{} {}", c.col, c.row);
    }
    assert!(wide[0].lat > wide[wide.len() - 1].lat && wide[0].lon < wide[1].lon);
    assert!(dem.grid(8.0, 44.0, 8.1, 44.1, 3).is_empty());
}
