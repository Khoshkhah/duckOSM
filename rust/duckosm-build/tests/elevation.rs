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

/// The browser gives the DEM as bytes: the build writes the same heights and the same grid as from the file.
#[test]
fn dem_bytes_build_as_dem_file() {
    use duckosm_build::{build, Params};
    use std::path::PathBuf;
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let tif = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/monaco-copernicus-glo30.tif");
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let p = Params {
        duckdb: false,
        built_at: Some(0),
        dem: Some(tif.clone()),
        dem_product: Some("DSM".into()),
        dem_vertical_datum: Some("EGM2008".into()),
        dem_license: Some("Copernicus".into()),
        ..Params::default()
    };
    let q = Params { dem: None, dem_bytes: Some(("glo30.tif".into(), std::fs::read(&tif).unwrap())), ..p.clone() };
    let (a, b) = (tmp.join("dem-file"), tmp.join("dem-bytes"));
    build(&data.join("monaco_sample.osm.pbf"), &a, &p).unwrap();
    build(&data.join("monaco_sample.osm.pbf"), &b, &q).unwrap();
    for f in ["elevation_grid_main.parquet", "elevation_nodes_driving.parquet", "elevation_edges_driving.parquet"] {
        let (x, y) = (std::fs::read(a.join(f)).unwrap(), std::fs::read(b.join(f)).unwrap());
        assert!(x.len() > 100 && x == y, "{f}");
    }
    // the ground id of both builds is the SHA-256 of the file; the datum and the license are written
    let sha = sha256_hex(&std::fs::read(&tif).unwrap());
    let con = duckdb::Connection::open_in_memory().unwrap();
    for dir in [&a, &b] {
        let f = dir.join("elevation_metadata_main.parquet");
        let row: (String, String, String, String) = con
            .query_row(
                &format!("SELECT sha256, vertical_datum, license, product FROM read_parquet('{}')", f.display()),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, (sha.clone(), "EGM2008".into(), "Copernicus".into(), "DSM".into()));
    }
}

fn sha256_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(b))
}

/// A DEM with no product, datum or license stops the build, unless it is marked as not the reference.
#[test]
fn reference_dem_needs_datum_and_license() {
    use duckosm_build::{build, Params};
    use std::path::PathBuf;
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let tif = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/monaco-copernicus-glo30.tif");
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let p = Params { duckdb: false, built_at: Some(0), dem: Some(tif), ..Params::default() };
    let e = build(&data.join("monaco_sample.osm.pbf"), &tmp.join("dem-no-datum"), &p).unwrap_err();
    assert!(e.contains("reference ground"), "{e}");
    let bad = Params { dem_product: Some("DEM".into()), dem_not_reference: true, ..p.clone() };
    let e = build(&data.join("monaco_sample.osm.pbf"), &tmp.join("dem-bad-product"), &bad).unwrap_err();
    assert!(e.contains("DTM (bare earth) or DSM"), "{e}");
    let q = Params { dem_not_reference: true, ..p };
    build(&data.join("monaco_sample.osm.pbf"), &tmp.join("dem-not-reference"), &q).unwrap();
}

/// A window of the DEM (the pixels of a box only, as a tile of a build by tiles reads it) gives the same values at the
/// points of the box and the same grid as the whole raster, also at the edges of the raster.
#[test]
fn a_window_equals_the_whole_raster() {
    let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/monaco-copernicus-glo30.tif"));
    let whole = Dem::open(path).unwrap();
    // inside, over the west edge of the raster, and outside
    for b in [[7.415, 43.728, 7.425, 43.735], [7.0, 43.70, 7.42, 43.75], [8.0, 44.0, 8.1, 44.1]] {
        let part = Dem::open_window(path, b, 3).unwrap();
        for i in 0..=20 {
            for j in 0..=20 {
                let (lon, lat) = (b[0] + (b[2] - b[0]) * i as f64 / 20.0, b[1] + (b[3] - b[1]) * j as f64 / 20.0);
                assert_eq!(part.sample(lon, lat), whole.sample(lon, lat), "{lon} {lat}");
            }
        }
        let cells = |d: &Dem| d.grid(b[0], b[1], b[2], b[3], 3).iter().map(|c| (c.col, c.row, c.lon, c.lat, c.z)).collect::<Vec<_>>();
        assert_eq!(cells(&part), cells(&whole));
    }
}
