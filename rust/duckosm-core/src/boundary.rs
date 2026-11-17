//! A boundary: a GeoJSON Polygon or MultiPolygon, also inside a Feature or a FeatureCollection (the file of
//! `duckosm boundary`). The input of a build of an area (docs/browser.md, step B4). The code is in the pbfcut library, so
//! the tile cut and the build use one boundary type. pbfcut also reads an osmosis `.poly` file, WKT and a box `W,S,E,N`.

pub use pbfcut::boundary::*;
