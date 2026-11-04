//! Geometry helpers. `Pt` is (longitude, latitude) in degrees.

pub type Pt = (f64, f64);

/// Haversine distance in metres, as in the SQL of duckOSM (`12742000 * asin(...)`).
pub fn haversine(a: Pt, b: Pt) -> f64 {
    let (lon1, lat1) = a;
    let (lon2, lat2) = b;
    let s1 = ((lat2 - lat1).to_radians() / 2.0).sin();
    let s2 = ((lon2 - lon1).to_radians() / 2.0).sin();
    12742000.0
        * (s1 * s1 + lat1.to_radians().cos() * lat2.to_radians().cos() * s2 * s2)
            .sqrt()
            .asin()
}

/// Length of a line as the sum of the haversine distances, rounded to a 32-bit float (the type of `length_m`).
pub fn line_length_m(pts: &[Pt]) -> f32 {
    let mut s = 0.0f64;
    for w in pts.windows(2) {
        s += haversine(w[0], w[1]);
    }
    s as f32
}

/// `ST_RemoveRepeatedPoints`: drop a point that equals the point before it.
pub fn remove_repeated(pts: &[Pt]) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(pts.len());
    for &p in pts {
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

fn seg_len(a: Pt, b: Pt) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// Cumulative planar length (in degrees) at each vertex.
fn cumulative(pts: &[Pt]) -> Vec<f64> {
    let mut c = Vec::with_capacity(pts.len());
    let mut s = 0.0;
    c.push(0.0);
    for w in pts.windows(2) {
        s += seg_len(w[0], w[1]);
        c.push(s);
    }
    c
}

/// The first segment `i` with `cum[i] <= d <= cum[i+1]`, and the point at distance `d` along the line.
fn locate(pts: &[Pt], cum: &[f64], d: f64) -> (usize, Pt) {
    let n = pts.len();
    let mut i = 0;
    while i + 2 < n && d > cum[i + 1] {
        i += 1;
    }
    let (a, b) = (pts[i], pts[i + 1]);
    let len = cum[i + 1] - cum[i];
    let p = if d >= cum[i + 1] {
        b
    } else if d <= cum[i] || len == 0.0 {
        a
    } else {
        let t = (d - cum[i]) / len;
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    };
    (i, p)
}

/// `ST_LineInterpolatePoint(line, frac)`: the point at a fraction of the planar length.
pub fn interpolate(pts: &[Pt], frac: f64) -> Pt {
    let cum = cumulative(pts);
    locate(pts, &cum, frac * cum[cum.len() - 1]).1
}

/// `ST_LineSubstring(line, from, to)`: the start point, the vertices between, the end point.
/// A cut that lands on a vertex keeps the vertex too, as DuckDB does.
pub fn substring(pts: &[Pt], from: f64, to: f64) -> Vec<Pt> {
    let cum = cumulative(pts);
    let total = cum[cum.len() - 1];
    let (s, ps) = locate(pts, &cum, from * total);
    let (e, pe) = locate(pts, &cum, to * total);
    let mut out = vec![ps];
    if e > s {
        out.extend_from_slice(&pts[s + 1..=e]);
    }
    out.push(pe);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Same results as DuckDB spatial: SELECT ST_LineSubstring('LINESTRING(0 0, 2 0, 2 2)', 0.5, 1)
    #[test]
    fn substring_on_a_vertex() {
        let l = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0)];
        assert_eq!(substring(&l, 0.0, 0.5), vec![(0.0, 0.0), (2.0, 0.0)]);
        assert_eq!(
            substring(&l, 0.5, 1.0),
            vec![(2.0, 0.0), (2.0, 0.0), (2.0, 2.0)]
        );
        assert_eq!(interpolate(&l, 0.5), (2.0, 0.0));
    }

    #[test]
    fn substring_between_vertices() {
        let l = [(0.0, 0.0), (3.0, 4.0), (3.0, 10.0), (8.0, 10.0)];
        assert_eq!(interpolate(&l, 0.5), (3.0, 7.0));
        assert_eq!(
            substring(&l, 0.0, 0.5),
            vec![(0.0, 0.0), (3.0, 4.0), (3.0, 7.0)]
        );
        assert_eq!(
            substring(&l, 0.5, 1.0),
            vec![(3.0, 7.0), (3.0, 10.0), (8.0, 10.0)]
        );
    }
}
