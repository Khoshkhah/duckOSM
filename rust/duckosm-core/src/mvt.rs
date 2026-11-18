//! Vector tiles: the shapes of a feature in Web Mercator, their simplification, their clip to the tiles of a zoom, and
//! the encoding of a tile as a Mapbox Vector Tile 2.1 (https://github.com/mapbox/vector-tile-spec/tree/master/2.1).
//! `duckosm quack --pmtiles` (docs/exports/pmtiles.md) uses it with the Shortbread rules (`crate::shortbread`) and the
//! PMTiles writer (`crate::pmtiles`). Pure Rust, so it also builds for WebAssembly.
//!
//! A tile has an extent of 4096 units. Each shape is clipped at the tile border plus a buffer of 64 units (4 pixels of a
//! tile of 256 pixels), so that a line or a polygon border at the border of a tile has no gap and no cap.

/// A point in Web Mercator world units: x from 0 (180° W) to 1 (180° E), y from 0 (north) to 1 (south).
pub type P = [f64; 2];

/// The extent of a tile.
pub const EXTENT: f64 = 4096.0;
/// The buffer around a tile, in units of the extent.
pub const BUFFER: f64 = 64.0;
/// The largest zoom.
pub const MAX_ZOOM: u8 = 14;

/// The MVT geometry types.
pub const POINT: u8 = 1;
pub const LINESTRING: u8 = 2;
pub const POLYGON: u8 = 3;

/// The shape of a feature in world units. A polygon is its rings, the shell first.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Points(Vec<P>),
    Lines(Vec<Vec<P>>),
    Polygons(Vec<Vec<Vec<P>>>),
}

/// Web Mercator of a longitude and a latitude (degrees), with the latitude limit of the projection (85.0511°).
pub fn mercator(lon: f64, lat: f64) -> P {
    let lat = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    let y = 0.5 - (lat.tan() + 1.0 / lat.cos()).ln() / (2.0 * std::f64::consts::PI);
    [(lon + 180.0) / 360.0, y]
}

/// The area of a ring (shoelace), in square world units; positive when the ring turns clockwise on the map.
pub fn ring_area(r: &[P]) -> f64 {
    let mut a = 0.0;
    for w in r.windows(2) {
        a += w[0][0] * w[1][1] - w[1][0] * w[0][1];
    }
    a / 2.0
}

impl Shape {
    /// The box (x0, y0, x1, y1) of the points.
    pub fn bbox(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut add = |p: &P| {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        };
        match self {
            Shape::Points(ps) => ps.iter().for_each(&mut add),
            Shape::Lines(ls) => ls.iter().flatten().for_each(&mut add),
            Shape::Polygons(ps) => ps.iter().flat_map(|p| p.first()).flatten().for_each(&mut add),
        }
        b
    }

    /// The shape with Douglas-Peucker simplification at `tol` world units (no change at 0). A line keeps its two ends; a
    /// ring that has less than 4 points after it is removed, and a polygon whose shell is removed is removed.
    pub fn simplify(&self, tol: f64) -> Option<Shape> {
        if tol <= 0.0 {
            return Some(self.clone());
        }
        match self {
            Shape::Points(_) => Some(self.clone()),
            Shape::Lines(ls) => {
                let ls: Vec<Vec<P>> = ls.iter().map(|l| douglas_peucker(l, tol)).filter(|l| l.len() >= 2).collect();
                (!ls.is_empty()).then_some(Shape::Lines(ls))
            }
            Shape::Polygons(ps) => {
                let ps: Vec<Vec<Vec<P>>> = ps
                    .iter()
                    .filter_map(|rings| {
                        let shell = douglas_peucker(rings.first()?, tol);
                        if shell.len() < 4 {
                            return None;
                        }
                        let mut out = vec![shell];
                        out.extend(rings[1..].iter().map(|r| douglas_peucker(r, tol)).filter(|r| r.len() >= 4));
                        Some(out)
                    })
                    .collect();
                (!ps.is_empty()).then_some(Shape::Polygons(ps))
            }
        }
    }

    /// The part of the shape in the box `b` (x0, y0, x1, y1): the points in the box, the lines cut at its border, and the
    /// polygons clipped with Sutherland-Hodgman (a polygon can get edges on the border of the box, as in other tile
    /// makers; the renderer does not draw them, as they are outside the tile).
    pub fn clip(&self, b: [f64; 4]) -> Option<Shape> {
        let inside = |p: &P| p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3];
        let bb = self.bbox();
        if bb[0] >= b[0] && bb[1] >= b[1] && bb[2] <= b[2] && bb[3] <= b[3] {
            return Some(self.clone());
        }
        if bb[2] < b[0] || bb[0] > b[2] || bb[3] < b[1] || bb[1] > b[3] {
            return None;
        }
        match self {
            Shape::Points(ps) => {
                let ps: Vec<P> = ps.iter().copied().filter(inside).collect();
                (!ps.is_empty()).then_some(Shape::Points(ps))
            }
            Shape::Lines(ls) => {
                let mut out = Vec::new();
                for l in ls {
                    clip_line(l, b, &mut out);
                }
                (!out.is_empty()).then_some(Shape::Lines(out))
            }
            Shape::Polygons(ps) => {
                let ps: Vec<Vec<Vec<P>>> = ps
                    .iter()
                    .filter_map(|rings| {
                        let shell = clip_ring(&rings[0], b);
                        if shell.len() < 4 {
                            return None;
                        }
                        let mut out = vec![shell];
                        out.extend(rings[1..].iter().map(|r| clip_ring(r, b)).filter(|r| r.len() >= 4));
                        Some(out)
                    })
                    .collect();
                (!ps.is_empty()).then_some(Shape::Polygons(ps))
            }
        }
    }

    /// Each tile (x, y) of zoom `z` that the shape touches, with the shape clipped to the tile and its buffer, in world
    /// units. The clip goes down the tile tree from the smallest tile that holds the whole shape, so a large shape is cut
    /// one time for each level, not one time for each tile.
    pub fn tiles(&self, z: u8, f: &mut impl FnMut(u32, u32, &Shape)) {
        let b = self.bbox();
        if b[0] > b[2] {
            return;
        }
        // the smallest tile (zoom zs <= z) that holds the box
        let n = (1u64 << z) as f64;
        let max = (1u32 << z) - 1;
        let cell = |v: f64| ((v * n).floor().max(0.0) as u32).min(max);
        let (x0, y0, x1, y1) = (cell(b[0]), cell(b[1]), cell(b[2]), cell(b[3]));
        let mut zs = z;
        let (mut x, mut y) = (x0, y0);
        while zs > 0 && (x0 >> (z - zs) != x1 >> (z - zs) || y0 >> (z - zs) != y1 >> (z - zs)) {
            zs -= 1;
        }
        x >>= z - zs;
        y >>= z - zs;
        descend(self, zs, x, y, z, f);
    }
}

/// The box of tile (z, x, y) with its buffer, in world units.
pub fn tile_box(z: u8, x: u32, y: u32) -> [f64; 4] {
    let s = 1.0 / (1u64 << z) as f64;
    let buf = s * BUFFER / EXTENT;
    [x as f64 * s - buf, y as f64 * s - buf, (x + 1) as f64 * s + buf, (y + 1) as f64 * s + buf]
}

fn descend(shape: &Shape, z: u8, x: u32, y: u32, target: u8, f: &mut impl FnMut(u32, u32, &Shape)) {
    let Some(part) = shape.clip(tile_box(z, x, y)) else { return };
    if z == target {
        f(x, y, &part);
        return;
    }
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        descend(&part, z + 1, x * 2 + dx, y * 2 + dy, target, f);
    }
}

fn douglas_peucker(l: &[P], tol: f64) -> Vec<P> {
    if l.len() < 3 {
        return l.to_vec();
    }
    let mut keep = vec![false; l.len()];
    keep[0] = true;
    keep[l.len() - 1] = true;
    let mut stack = vec![(0, l.len() - 1)];
    let tol2 = tol * tol;
    while let Some((a, b)) = stack.pop() {
        let (mut best, mut at) = (0.0, 0);
        for i in a + 1..b {
            let d = seg_dist2(l[i], l[a], l[b]);
            if d > best {
                (best, at) = (d, i);
            }
        }
        if best > tol2 {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    l.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

fn seg_dist2(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
    qx * qx + qy * qy
}

/// The parts of line `l` in box `b` (Liang-Barsky for each segment), added to `out`.
fn clip_line(l: &[P], b: [f64; 4], out: &mut Vec<Vec<P>>) {
    let mut cur: Vec<P> = Vec::new();
    for w in l.windows(2) {
        let Some((p, q)) = clip_segment(w[0], w[1], b) else {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            continue;
        };
        if cur.last() != Some(&p) {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            cur.push(p);
        }
        cur.push(q);
        if q != w[1] {
            // the segment leaves the box
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
}

fn clip_segment(a: P, c: P, b: [f64; 4]) -> Option<(P, P)> {
    let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a[0] - b[0]), (dx, b[2] - a[0]), (-dy, a[1] - b[1]), (dy, b[3] - a[1])] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
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
    if t0 > t1 {
        return None;
    }
    let at = |t: f64| if t == 0.0 { a } else if t == 1.0 { c } else { [a[0] + t * dx, a[1] + t * dy] };
    Some((at(t0), at(t1)))
}

/// Ring `r` (closed: the last point is the first) clipped to box `b` (Sutherland-Hodgman); closed, or empty.
fn clip_ring(r: &[P], b: [f64; 4]) -> Vec<P> {
    let mut pts: Vec<P> = r[..r.len().saturating_sub(1)].to_vec();
    for edge in 0..4 {
        if pts.is_empty() {
            break;
        }
        let inside = |p: &P| match edge {
            0 => p[0] >= b[0],
            1 => p[0] <= b[2],
            2 => p[1] >= b[1],
            _ => p[1] <= b[3],
        };
        let cut = |p: P, q: P| -> P {
            let (v, i) = match edge {
                0 => (b[0], 0),
                1 => (b[2], 0),
                2 => (b[1], 1),
                _ => (b[3], 1),
            };
            let t = (v - p[i]) / (q[i] - p[i]);
            let mut o = [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])];
            o[i] = v;
            o
        };
        let mut next = Vec::with_capacity(pts.len() + 4);
        for i in 0..pts.len() {
            let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
            match (inside(&p), inside(&q)) {
                (true, true) => next.push(q),
                (true, false) => next.push(cut(p, q)),
                (false, true) => {
                    next.push(cut(p, q));
                    next.push(q);
                }
                (false, false) => {}
            }
        }
        pts = next;
    }
    if pts.len() < 3 {
        return Vec::new();
    }
    pts.push(pts[0]);
    pts
}

/// The geometry of a feature in a tile: its type and its parts in tile units (a point list, the lines, or the rings with
/// the MVT orientation: a shell has a positive area, a hole a negative area, with y down). Parts of several features
/// with the same attributes can be joined (the parts of one are added to the other).
#[derive(Debug, Clone, PartialEq)]
pub struct TileGeom {
    pub kind: u8,
    pub parts: Vec<Vec<[i32; 2]>>,
}

/// The shape (clipped to tile (z, x, y)) in tile units: rounded to the grid, with no repeated point; None when nothing
/// stays (a line of one point, a ring of no area).
pub fn quantize(shape: &Shape, z: u8, x: u32, y: u32) -> Option<TileGeom> {
    let n = (1u64 << z) as f64;
    let q = |p: &P| [((p[0] * n - x as f64) * EXTENT).round() as i32, ((p[1] * n - y as f64) * EXTENT).round() as i32];
    let line = |l: &[P]| {
        let mut o: Vec<[i32; 2]> = Vec::with_capacity(l.len());
        for p in l {
            let p = q(p);
            if o.last() != Some(&p) {
                o.push(p);
            }
        }
        o
    };
    let area = |r: &[[i32; 2]]| -> i64 { r.windows(2).map(|w| w[0][0] as i64 * w[1][1] as i64 - w[1][0] as i64 * w[0][1] as i64).sum() };
    let (kind, parts) = match shape {
        Shape::Points(ps) => (POINT, vec![ps.iter().map(q).collect::<Vec<_>>()]),
        Shape::Lines(ls) => (LINESTRING, ls.iter().map(|l| line(l)).filter(|l| l.len() >= 2).collect()),
        Shape::Polygons(ps) => {
            let mut parts = Vec::new();
            for rings in ps {
                for (i, r) in rings.iter().enumerate() {
                    let mut r = line(r);
                    if r.len() < 4 {
                        if i == 0 {
                            break;
                        }
                        continue;
                    }
                    let a = area(&r);
                    if a == 0 {
                        if i == 0 {
                            break;
                        }
                        continue;
                    }
                    // a shell turns clockwise on the screen (positive area with y down), a hole the other way
                    if (a > 0) != (i == 0) {
                        r.reverse();
                    }
                    parts.push(r);
                }
            }
            (POLYGON, parts)
        }
    };
    (!parts.is_empty() && !parts[0].is_empty()).then_some(TileGeom { kind, parts })
}

/// The command integers of the MVT geometry of `g`.
pub fn commands(g: &TileGeom) -> Vec<u32> {
    let cmd = |id: u32, count: usize| (id & 7) | ((count as u32) << 3);
    let zz = |v: i32| ((v << 1) ^ (v >> 31)) as u32;
    let mut out = Vec::new();
    let mut cur = [0i32; 2];
    let mut put = |out: &mut Vec<u32>, p: [i32; 2]| {
        out.push(zz(p[0] - cur[0]));
        out.push(zz(p[1] - cur[1]));
        cur = p;
    };
    match g.kind {
        POINT => {
            let ps: Vec<[i32; 2]> = g.parts.iter().flatten().copied().collect();
            out.push(cmd(1, ps.len()));
            ps.into_iter().for_each(|p| put(&mut out, p));
        }
        LINESTRING => {
            for l in &g.parts {
                out.push(cmd(1, 1));
                put(&mut out, l[0]);
                out.push(cmd(2, l.len() - 1));
                l[1..].iter().for_each(|&p| put(&mut out, p));
            }
        }
        _ => {
            for r in &g.parts {
                // the last point repeats the first: ClosePath in its place
                out.push(cmd(1, 1));
                put(&mut out, r[0]);
                out.push(cmd(2, r.len() - 2));
                r[1..r.len() - 1].iter().for_each(|&p| put(&mut out, p));
                out.push(cmd(7, 1));
            }
        }
    }
    out
}

/// A value of an attribute.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Value {
    /// The type name of the field in the `vector_layers` of the metadata.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Str(_) => "String",
            Value::Bool(_) => "Boolean",
            _ => "Number",
        }
    }

    /// The value as an MVT Value message.
    fn message(&self, out: &mut Vec<u8>) {
        match self {
            Value::Str(s) => {
                key(out, 1, 2);
                varint(out, s.len() as u64);
                out.extend_from_slice(s.as_bytes());
            }
            Value::Float(f) => {
                key(out, 3, 1);
                out.extend_from_slice(&f.to_le_bytes());
            }
            Value::Int(i) => {
                if *i >= 0 {
                    key(out, 5, 0);
                    varint(out, *i as u64);
                } else {
                    key(out, 6, 0);
                    varint(out, ((*i << 1) ^ (*i >> 63)) as u64);
                }
            }
            Value::Bool(b) => {
                key(out, 7, 0);
                varint(out, *b as u64);
            }
        }
    }
}

pub fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn key(out: &mut Vec<u8>, field: u32, wire: u32) {
    varint(out, ((field << 3) | wire) as u64);
}

fn bytes_field(out: &mut Vec<u8>, field: u32, b: &[u8]) {
    key(out, field, 2);
    varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

/// A feature of a layer of a tile.
pub struct TileFeature {
    pub geom: TileGeom,
    pub attrs: Vec<(String, Value)>,
}

/// Adds the MVT Layer message `name` with `features` to the tile `out` (a Tile message is its layers, field 3).
pub fn write_layer(out: &mut Vec<u8>, name: &str, features: &[TileFeature]) {
    let mut keys: Vec<&str> = Vec::new();
    let mut values: Vec<Vec<u8>> = Vec::new();
    let mut key_at = crate::util::FxHashMap::<&str, u32>::default();
    let mut value_at = crate::util::FxHashMap::<Vec<u8>, u32>::default();
    let mut layer = Vec::new();
    key(&mut layer, 15, 0);
    varint(&mut layer, 2);
    bytes_field(&mut layer, 1, name.as_bytes());
    let mut f = Vec::new();
    let mut packed = Vec::new();
    let mut v = Vec::new();
    for feature in features {
        f.clear();
        packed.clear();
        for (k, val) in &feature.attrs {
            let ki = *key_at.entry(k.as_str()).or_insert_with(|| {
                keys.push(k.as_str());
                keys.len() as u32 - 1
            });
            v.clear();
            val.message(&mut v);
            let vi = match value_at.get(&v) {
                Some(&i) => i,
                None => {
                    values.push(v.clone());
                    value_at.insert(v.clone(), values.len() as u32 - 1);
                    values.len() as u32 - 1
                }
            };
            varint(&mut packed, ki as u64);
            varint(&mut packed, vi as u64);
        }
        if !packed.is_empty() {
            bytes_field(&mut f, 2, &packed);
        }
        key(&mut f, 3, 0);
        varint(&mut f, feature.geom.kind as u64);
        packed.clear();
        commands(&feature.geom).into_iter().for_each(|c| varint(&mut packed, c as u64));
        bytes_field(&mut f, 4, &packed);
        bytes_field(&mut layer, 2, &f);
    }
    for k in keys {
        bytes_field(&mut layer, 3, k.as_bytes());
    }
    for v in &values {
        bytes_field(&mut layer, 4, v);
    }
    key(&mut layer, 5, 0);
    varint(&mut layer, EXTENT as u64);
    bytes_field(out, 3, &layer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_of_the_spec() {
        // the examples of the MVT 2.1 spec, 4.3.5
        let pt = TileGeom { kind: POINT, parts: vec![vec![[25, 17]]] };
        assert_eq!(commands(&pt), vec![9, 50, 34]);
        let line = TileGeom { kind: LINESTRING, parts: vec![vec![[2, 2], [2, 10], [10, 10]]] };
        assert_eq!(commands(&line), vec![9, 4, 4, 18, 0, 16, 16, 0]);
        let poly = TileGeom { kind: POLYGON, parts: vec![vec![[3, 6], [8, 12], [20, 34], [3, 6]]] };
        assert_eq!(commands(&poly), vec![9, 6, 12, 18, 10, 12, 24, 44, 15]);
    }

    #[test]
    fn clip_and_quantize() {
        // a line across tile (1, 0, 0): cut at the border with the buffer
        let l = Shape::Lines(vec![vec![[0.25, 0.25], [0.75, 0.25]]]);
        let mut got = Vec::new();
        l.tiles(1, &mut |x, y, s| got.push((x, y, quantize(s, 1, x, y).unwrap())));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].2.parts, vec![vec![[2048, 2048], [4096 + 64, 2048]]]);
        assert_eq!(got[1].2.parts, vec![vec![[-64, 2048], [2048, 2048]]]);
        // a square around the tile is the tile with its buffer, as a shell with a positive area
        let sq = Shape::Polygons(vec![vec![vec![[-1.0, -1.0], [2.0, -1.0], [2.0, 2.0], [-1.0, 2.0], [-1.0, -1.0]]]]);
        let mut n = 0;
        sq.tiles(2, &mut |x, y, s| {
            let g = quantize(s, 2, x, y).unwrap();
            let r = &g.parts[0];
            let a: i64 = r.windows(2).map(|w| w[0][0] as i64 * w[1][1] as i64 - w[1][0] as i64 * w[0][1] as i64).sum();
            assert!(a > 0);
            assert_eq!(r.len(), 5);
            n += 1;
        });
        assert_eq!(n, 16);
    }
}
