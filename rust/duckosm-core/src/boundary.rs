//! A boundary: a GeoJSON Polygon or MultiPolygon, also inside a Feature or a FeatureCollection (the file of
//! `duckosm boundary`). The input of a build of an area (docs/browser.md, step B4).

use serde_json::Value;

type Ring = Vec<(f64, f64)>;

pub struct Boundary {
    /// Each polygon: the outer ring and its holes, as (longitude, latitude).
    polygons: Vec<Vec<Ring>>,
    /// west, south, east, north
    bbox: (f64, f64, f64, f64),
    /// `properties.name` of the first Feature, if any.
    pub name: Option<String>,
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
        Ok(Boundary {
            polygons,
            bbox,
            name,
        })
    }

    /// Is the point inside? A hole of a polygon is outside (the even-odd rule over all rings of a polygon). A
    /// point on a ring counts as inside, as in osmium's cut: a border often runs through the nodes of a road.
    pub fn contains(&self, lon: f64, lat: f64) -> bool {
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
}
