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
