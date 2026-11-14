//! A boundary: a GeoJSON Polygon or MultiPolygon, also inside a Feature or a FeatureCollection (the file of
//! `duckosm boundary`). The input of a build of an area (docs/browser.md, step B4).

use serde_json::Value;

type Ring = Vec<(f64, f64)>;

pub struct Boundary {
    /// Each polygon: the outer ring and its holes, as (longitude, latitude).
    polygons: Vec<Vec<Ring>>,
    /// west, south, east, north
    bbox: (f64, f64, f64, f64),
    /// The edges by latitude band, for [`Boundary::contains`].
    bands: Bands,
    /// `properties.name` of the first Feature, if any.
    pub name: Option<String>,
    /// The build of one tile of a large build: only the part of the area in this tile ([`Boundary::within_tile`]).
    tile: Option<crate::tiles::Tile>,
}

/// An edge of a ring: the number of its polygon, and its two ends.
type Edge = (usize, (f64, f64), (f64, f64));

/// The edges of the polygons by latitude band. A point needs only the edges whose latitude range holds its own latitude: an
/// edge that does not reach it can neither cross the ray of the point nor have the point on it. Step 0 tests each node of the
/// region file in the box of the area (US South for Kentucky: tens of millions), and testing all edges of the border was
/// its largest cost.
struct Bands {
    south: f64,
    per_degree: f64,
    edges: Vec<Vec<Edge>>,
}

impl Bands {
    fn new(polygons: &[Vec<Ring>], south: f64, north: f64) -> Bands {
        let count = polygons.iter().flatten().map(Vec::len).sum::<usize>().clamp(1, 1 << 16);
        let per_degree = if north > south { count as f64 / (north - south) } else { 0.0 };
        let mut b = Bands { south, per_degree, edges: vec![Vec::new(); count] };
        for (p, rings) in polygons.iter().enumerate() {
            for ring in rings {
                for k in 0..ring.len() {
                    let (a, c) = (ring[k], ring[(k + 1) % ring.len()]);
                    for i in b.band(a.1.min(c.1))..=b.band(a.1.max(c.1)) {
                        b.edges[i].push((p, a, c));
                    }
                }
            }
        }
        b
    }

    /// The band of a latitude: the same function for the edges and the points, and it never gets smaller when the latitude
    /// gets larger, so an edge is in the band of each latitude of its range.
    fn band(&self, lat: f64) -> usize {
        (((lat - self.south) * self.per_degree) as usize).min(self.edges.len() - 1)
    }
}

impl Boundary {
    pub fn parse(geojson: &str) -> Result<Boundary, String> {
        let v: Value = serde_json::from_str(geojson).map_err(|e| format!("boundary: {e}"))?;
        let mut polygons = Vec::new();
        let mut name = None;
        collect(&v, &mut polygons, &mut name)?;
        if polygons.is_empty() {
            return Err("boundary: no Polygon or MultiPolygon in the GeoJSON".into());
        }
        let pts = polygons.iter().flatten().flatten();
        let bbox = pts.fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |b, &(x, y)| {
            (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y))
        });
        let bands = Bands::new(&polygons, bbox.1, bbox.3);
        Ok(Boundary {
            polygons,
            bbox,
            bands,
            name,
            tile: None,
        })
    }

    /// The part of the area in tile `t`: a point is inside when it is inside the area and its tile is `t` (`Tile::of`, the
    /// rule of the tile cut). The build of a tile takes the tile file with this boundary.
    pub fn within_tile(mut self, t: crate::tiles::Tile) -> Boundary {
        let (w, s, e, n) = t.bounds();
        let b = self.bbox;
        self.bbox = (b.0.max(w), b.1.max(s), b.2.min(e), b.3.min(n));
        self.tile = Some(t);
        self
    }

    /// Is the point inside? A hole of a polygon is outside (the even-odd rule over all rings of a polygon). A
    /// point on a ring counts as inside, as in osmium's cut: a border often runs through the nodes of a road.
    pub fn contains(&self, lon: f64, lat: f64) -> bool {
        let (w, s, e, n) = self.bbox;
        if lon < w || lon > e || lat < s || lat > n {
            return false;
        }
        if self.tile.is_some_and(|t| crate::tiles::Tile::of(t.z, lon, lat) != t) {
            return false;
        }
        if self.polygons.len() > 64 {
            return self.contains_all(lon, lat);
        }
        // the edges of the point's band only; the even-odd rule of each polygon is one bit
        let mut inside = 0u64;
        for &(p, a, b) in &self.bands.edges[self.bands.band(lat)] {
            if on_segment((lon, lat), a, b) {
                return true;
            }
            if (a.1 > lat) != (b.1 > lat) && lon < (b.0 - a.0) * (lat - a.1) / (b.1 - a.1) + a.0 {
                inside ^= 1 << p;
            }
        }
        inside != 0
    }

    /// [`Boundary::contains`] with all edges of all polygons (more than 64 polygons, and the test of the bands).
    fn contains_all(&self, lon: f64, lat: f64) -> bool {
        let (w, s, e, n) = self.bbox;
        if lon < w || lon > e || lat < s || lat > n {
            return false;
        }
        self.polygons.iter().any(|rings| {
            let mut inside = false;
            for ring in rings {
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    if on_segment((lon, lat), a, b) {
                        return true;
                    }
                    if (a.1 > lat) != (b.1 > lat)
                        && lon < (b.0 - a.0) * (lat - a.1) / (b.1 - a.1) + a.0
                    {
                        inside = !inside;
                    }
                }
            }
            inside
        })
    }

    /// Does the boundary touch the box from (`w`, `s`) to (`e`, `n`): a corner of the box inside, or an edge of a ring in the
    /// box or across it?
    pub fn touches_box(&self, w: f64, s: f64, e: f64, n: f64) -> bool {
        let (bw, bs, be, bn) = self.bbox;
        if be < w || bw > e || bn < s || bs > n {
            return false;
        }
        if [(w, s), (e, s), (e, n), (w, n)].iter().any(|&(x, y)| self.contains(x, y)) {
            return true;
        }
        // the segment a-b clipped to the box (Liang-Barsky): something is left
        let crosses = |a: (f64, f64), b: (f64, f64)| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let (mut t0, mut t1) = (0.0f64, 1.0f64);
            for (p, q) in [(-dx, a.0 - w), (dx, e - a.0), (-dy, a.1 - s), (dy, n - a.1)] {
                if p == 0.0 {
                    if q < 0.0 {
                        return false;
                    }
                } else {
                    let r = q / p;
                    if p < 0.0 {
                        t0 = t0.max(r);
                    } else {
                        t1 = t1.min(r);
                    }
                }
            }
            t0 <= t1
        };
        self.polygons.iter().flatten().any(|ring| (0..ring.len()).any(|k| crosses(ring[k], ring[(k + 1) % ring.len()])))
    }

    /// west, south, east, north
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        self.bbox
    }

    /// The outer rings, as (longitude, latitude): for the polygon filter of an Overpass query.
    pub fn outer_rings(&self) -> impl Iterator<Item = &Ring> {
        self.polygons.iter().filter_map(|p| p.first())
    }
}

/// Is p on the segment a-b? Exact: a vertex that is an OSM node has the same bits as the node (`degrees`).
fn on_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> bool {
    if p == a || p == b {
        return true;
    }
    let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    cross == 0.0
        && p.0 >= a.0.min(b.0)
        && p.0 <= a.0.max(b.0)
        && p.1 >= a.1.min(b.1)
        && p.1 <= a.1.max(b.1)
}

/// A coordinate as the PBF reader computes it (`1e-9 * nanodegrees`), so a vertex that is an OSM node has
/// exactly the bits of the node.
fn degrees(v: f64) -> f64 {
    1e-9 * ((v * 1e9).round() as i64) as f64
}

fn ring(v: &Value) -> Result<Ring, String> {
    v.as_array()
        .ok_or("boundary: a ring must be a list")?
        .iter()
        .map(|p| match p.as_array().map(|a| a.as_slice()) {
            Some([x, y, ..]) => match (x.as_f64(), y.as_f64()) {
                (Some(x), Some(y)) => Ok((degrees(x), degrees(y))),
                _ => Err("boundary: a position must be numbers".to_string()),
            },
            _ => Err("boundary: a position must be [longitude, latitude]".to_string()),
        })
        .collect()
}

fn polygon(v: &Value) -> Result<Vec<Ring>, String> {
    v.as_array()
        .ok_or("boundary: a Polygon must be a list of rings")?
        .iter()
        .map(ring)
        .collect()
}

fn collect(v: &Value, out: &mut Vec<Vec<Ring>>, name: &mut Option<String>) -> Result<(), String> {
    match v.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => {
            for f in v
                .get("features")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                collect(f, out, name)?;
            }
        }
        Some("Feature") => {
            if name.is_none() {
                *name = v
                    .pointer("/properties/name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            if let Some(g) = v.get("geometry") {
                collect(g, out, name)?;
            }
        }
        Some("Polygon") => out.push(polygon(&v["coordinates"])?),
        Some("MultiPolygon") => {
            for p in v["coordinates"]
                .as_array()
                .ok_or("boundary: bad MultiPolygon")?
            {
                out.push(polygon(p)?);
            }
        }
        Some(t) => return Err(format!("boundary: {t} is not a Polygon or a MultiPolygon")),
        None => return Err("boundary: GeoJSON without a type".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_with_a_hole() {
        let b = Boundary::parse(
            r#"{"type":"Feature","properties":{"name":"A"},"geometry":{"type":"Polygon","coordinates":[
            [[0,0],[10,0],[10,10],[0,10],[0,0]],[[4,4],[6,4],[6,6],[4,6],[4,4]]]}}"#,
        )
        .unwrap();
        assert_eq!(b.name.as_deref(), Some("A"));
        assert!(b.contains(1.0, 1.0) && b.contains(9.0, 5.0));
        assert!(!b.contains(5.0, 5.0), "in the hole");
        assert!(!b.contains(11.0, 5.0) && !b.contains(5.0, -1.0));
        assert!(Boundary::parse(r#"{"type":"Point","coordinates":[1,2]}"#).is_err());
    }

    /// The test with the bands gives the answer of the test with all edges: on Monaco's boundary and on a multipolygon with
    /// a hole, for random points in the box, the vertices, the middle of each edge and points just beside them.
    #[test]
    fn bands_give_the_answer_of_all_edges() {
        let monaco = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data/monaco.geojson")).unwrap();
        let multi = r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[10,0],[10,10],[0,10],[0,0]],[[4,4],[6,4],[6,6],[4,6],[4,4]]],
            [[[12,1],[15,1],[13.5,9],[12,1]]]]}"#;
        let mut seed = 12345u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for text in [monaco.as_str(), multi] {
            let b = Boundary::parse(text).unwrap();
            let (w, s, e, n) = b.bbox;
            let mut pts: Vec<(f64, f64)> = (0..20_000).map(|_| (w + (e - w) * rnd(), s + (n - s) * rnd())).collect();
            for ring in b.polygons.iter().flatten() {
                for k in 0..ring.len() {
                    let (a, c) = (ring[k], ring[(k + 1) % ring.len()]);
                    pts.extend([a, ((a.0 + c.0) / 2.0, (a.1 + c.1) / 2.0), (a.0 + 1e-9, a.1), (a.0, a.1 - 1e-9)]);
                }
            }
            let inside = pts.iter().filter(|p| b.contains(p.0, p.1)).count();
            assert!(inside > 100 && inside < pts.len(), "the points are on both sides");
            for &(x, y) in &pts {
                assert_eq!(b.contains(x, y), b.contains_all(x, y), "({x}, {y})");
            }
        }
    }
}
