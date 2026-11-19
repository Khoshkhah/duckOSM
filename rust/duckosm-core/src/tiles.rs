//! The tiles of a large build: the standard map tiles (z/x/y, Web Mercator). The code is in the pbfcut library, so the
//! tile cut and the build use one tile grid. [`tiles`] gives the list of tiles of an area.

pub use pbfcut::tiles::*;

use crate::model::{Edge, Node};
use std::collections::HashMap;

/// The zoom of the tile columns (`nodes.tile`, `edges.from_tile`, `to_tile`): 28, about 15 cm at the equator.
pub const KEY_ZOOM: u8 = 28;

/// The tile key of tileroute (a BIGINT, always >= 0): bits 61..6 hold the quadkey digits (digit = 2*ybit + xbit), coarsest
/// first and left-aligned; bits 5..0 hold the zoom. The key of a tile at zoom z has its 2z digit bits at the top of the field.
pub fn key(t: Tile) -> i64 {
    let mut digits = 0u64;
    for i in (0..t.z).rev() {
        digits = digits << 2 | (((t.y >> i) & 1) << 1 | ((t.x >> i) & 1)) as u64;
    }
    (digits << (6 + 2 * (KEY_ZOOM - t.z) as u32) | t.z as u64) as i64
}

/// The zoom of a key.
pub fn key_zoom(k: i64) -> u8 {
    (k & 63) as u8
}

/// The key of the ancestor at zoom `r` (r <= the zoom of `k`).
pub fn parent(k: i64, r: u8) -> i64 {
    k & !((1i64 << (6 + 2 * (KEY_ZOOM - r) as u32)) - 1) | r as i64
}

/// The zoom of the lowest common ancestor of two keys.
pub fn lca_zoom(a: i64, b: i64) -> u8 {
    let z = key_zoom(a).min(key_zoom(b));
    let d = ((a ^ b) as u64) >> 6;
    if d == 0 {
        return z;
    }
    z.min(((d.leading_zeros() - 8) / 2) as u8)
}

/// `(node_id, tile)` of each node, and `(edge_id, from_tile, to_tile, lca_zoom)` of each edge.
pub type KeyNodeRows = Vec<(i64, Option<i64>)>;
pub type KeyEdgeRows = Vec<(i64, Option<i64>, Option<i64>, Option<i64>)>;

/// The tile columns: the key of each node point at [`KEY_ZOOM`], and the keys of the two end nodes of each edge. An edge
/// takes its keys from its own end nodes, so a tile build and the whole build give the same keys.
pub fn index(nodes: &[Node], edges: &[Edge]) -> (KeyNodeRows, KeyEdgeRows) {
    let n: KeyNodeRows = nodes
        .iter()
        .map(|n| (n.node_id, (n.lat.is_finite() && n.lon.is_finite()).then(|| key(Tile::of(KEY_ZOOM, n.lon, n.lat)))))
        .collect();
    let by_id: HashMap<i64, Option<i64>> = n.iter().copied().collect();
    let e = edges
        .iter()
        .map(|e| {
            let f = by_id.get(&e.source).copied().flatten();
            let t = by_id.get(&e.target).copied().flatten();
            (e.edge_id, f, t, f.zip(t).map(|(f, t)| lca_zoom(f, t) as i64))
        })
        .collect();
    (n, e)
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn tileroute_vectors() {
        let t8 = Tile::of(8, 7.4246, 43.7384);
        assert_eq!((t8.x, t8.y), (133, 93));
        assert_eq!(key(t8), 1778007059137036296);
        let t28 = Tile::of(28, 7.4246, 43.7384);
        assert_eq!((t28.x, t28.y), (139753911, 97878998));
        let k = key(t28);
        assert_eq!(k, 1778020963338276700);
        assert_eq!(parent(k, 8), key(t8));
        assert_eq!(parent(k, 28), k);
        let near = key(Tile::of(28, 7.4250, 43.7390));
        assert_eq!(lca_zoom(k, near), 18);
        assert_eq!(lca_zoom(k, key(Tile::of(28, 2.35, 48.85))), 5);
        assert_eq!(lca_zoom(k, k), 28);
        assert_eq!(lca_zoom(k, key(t8)), 8);
        assert_eq!(key(Tile { z: 0, x: 0, y: 0 }), 0);
    }
}
