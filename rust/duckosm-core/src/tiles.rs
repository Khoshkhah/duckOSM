//! The tiles of a large build: the standard map tiles (z/x/y, Web Mercator), the same grid as the map tiles. One function,
//! [`tiles`], gives the list of tiles of an area, so the rule (the zoom, or later a split by content) changes in one place.

use crate::boundary::Boundary;

/// A standard map tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tile {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl Tile {
    /// The tile of zoom `z` that holds the point. A point on the line between two tiles is in the tile east or south of it.
    pub fn of(z: u8, lon: f64, lat: f64) -> Tile {
        let n = 1u64 << z;
        let x = ((lon + 180.0) / 360.0 * n as f64).floor();
        let lat = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians();
        let y = ((1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n as f64).floor();
        let max = (n - 1) as f64;
        Tile { z, x: x.clamp(0.0, max) as u32, y: y.clamp(0.0, max) as u32 }
    }

    /// West, south, east, north, in degrees.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let n = (1u64 << self.z) as f64;
        let lon = |x: f64| x / n * 360.0 - 180.0;
        let lat = |y: f64| (std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh().atan().to_degrees();
        (lon(self.x as f64), lat(self.y as f64 + 1.0), lon(self.x as f64 + 1.0), lat(self.y as f64))
    }

    /// The name of the tile, as in a map tile path.
    pub fn name(&self) -> String {
        format!("{}/{}/{}", self.z, self.x, self.y)
    }
}

/// The tiles of zoom `z` that the boundary touches, in the order of the build: south to north, then west to east.
pub fn tiles(boundary: &Boundary, z: u8) -> Vec<Tile> {
    let (w, s, e, n) = boundary.bbox();
    let (a, b) = (Tile::of(z, w, n), Tile::of(z, e, s));
    let mut out: Vec<Tile> = (a.y..=b.y)
        .rev()
        .flat_map(|y| (a.x..=b.x).map(move |x| Tile { z, x, y }))
        .filter(|t| {
            let (w, s, e, n) = t.bounds();
            boundary.touches_box(w, s, e, n)
        })
        .collect();
    out.sort_by_key(|t| (std::cmp::Reverse(t.y), t.x));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_is_in_its_tile() {
        // Louisville, zoom 10
        let t = Tile::of(10, -85.7585, 38.2527);
        assert_eq!((t.x, t.y), (268, 394));
        let (w, s, e, n) = t.bounds();
        assert!(w <= -85.7585 && -85.7585 < e && s < 38.2527 && 38.2527 <= n);
    }

    #[test]
    fn the_tiles_of_a_box() {
        let b = Boundary::parse(
            r#"{"type":"Polygon","coordinates":[[[-85.9,38.1],[-85.5,38.1],[-85.5,38.4],[-85.9,38.4],[-85.9,38.1]]]}"#,
        )
        .unwrap();
        let t = tiles(&b, 10);
        assert!(!t.is_empty());
        assert!(t.windows(2).all(|p| (std::cmp::Reverse(p[0].y), p[0].x) < (std::cmp::Reverse(p[1].y), p[1].x)));
        // every corner of the box is in a listed tile
        for (lon, lat) in [(-85.9, 38.1), (-85.5, 38.1), (-85.5, 38.4), (-85.9, 38.4)] {
            assert!(t.contains(&Tile::of(10, lon, lat)));
        }
    }
}
