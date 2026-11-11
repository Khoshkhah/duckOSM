//! The H3 columns of duckOSM (`options.h3_indexing`): `nodes.h3_cell`, and `edges.from_cell`, `to_cell`, `lca_res`.
//!
//! duckOSM (`processors/h3_indexer.py`, `utils/h3_calc.py`) gives each node the H3 cell of its point at
//! `options.h3_resolution` (default 8), each edge the cells of its two end nodes, and `lca_res`: the resolution of
//! the lowest common ancestor of the two cells (-1 when the base cells differ). The cell is stored as a signed 64-bit number.

use duckosm_core::model::{Edge, Node};
use h3o::{LatLng, Resolution};
use std::collections::HashMap;

/// The default of `options.h3_resolution`.
pub const DEFAULT_RESOLUTION: u8 = 8;

/// The cell of a point at a resolution, as duckOSM stores it. None for a point that H3 does not accept.
pub fn cell(lat: f64, lon: f64, res: Resolution) -> Option<i64> {
    let ll = LatLng::new(lat, lon).ok()?;
    Some(u64::from(ll.to_cell(res)) as i64)
}

/// The resolution of the lowest common ancestor of two cells (duckOSM `h3_resolution(h3_lca(a, b))`): the same
/// bit operations as `utils/h3_calc.py`. -1 when there is no common ancestor.
pub fn lca_res(a: i64, b: i64) -> i64 {
    let (a, b) = (a as u64, b as u64);
    if a == 0 || b == 0 {
        return -1;
    }
    let (ra, rb) = ((a >> 52) & 0xF, (b >> 52) & 0xF);
    let common = ra.min(rb);
    let mask = u64::MAX << (45 - common * 3);
    let res_bits = !(0xFu64 << 52);
    let diff = (a & mask & res_bits) ^ (b & mask & res_bits);
    if diff == 0 {
        // one cell is the ancestor of the other: the coarser one
        return ra.min(rb) as i64;
    }
    let msb = 64 - diff.leading_zeros() as u64; // bit_length
    if msb > 45 {
        return -1; // different base cells
    }
    ((45 - msb) / 3) as i64
}

/// `(node_id, h3_cell)` of each node, and `(edge_id, from_cell, to_cell, lca_res)` of each edge.
pub type NodeRows = Vec<(i64, Option<i64>)>;
pub type EdgeRows = Vec<(i64, Option<i64>, Option<i64>, Option<i64>)>;

pub fn index(nodes: &[Node], edges: &[Edge], res: u8) -> Result<(NodeRows, EdgeRows), String> {
    let r = Resolution::try_from(res).map_err(|e| format!("options.h3_resolution: {e}"))?;
    let n: NodeRows = nodes.iter().map(|n| (n.node_id, cell(n.lat, n.lon, r))).collect();
    let by_id: HashMap<i64, Option<i64>> = n.iter().copied().collect();
    let e = edges
        .iter()
        .map(|e| {
            let f = by_id.get(&e.source).copied().flatten();
            let t = by_id.get(&e.target).copied().flatten();
            let l = f.zip(t).map(|(f, t)| lca_res(f, t));
            (e.edge_id, f, t, l)
        })
        .collect();
    Ok((n, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lca_of_neighbours_and_parents() {
        let r8 = Resolution::Eight;
        let a = cell(43.7384, 7.4246, r8).unwrap();
        assert_eq!(lca_res(a, a), 8);
        // the parent at resolution 5 and the cell: the parent is the ancestor
        let c = h3o::CellIndex::try_from(a as u64).unwrap();
        let p = u64::from(c.parent(Resolution::Five).unwrap()) as i64;
        assert_eq!(lca_res(a, p), 5);
        // a point 1 degree away shares no cell below resolution 5
        let b = cell(44.7384, 7.4246, r8).unwrap();
        assert!(lca_res(a, b) < 5);
        assert_eq!(lca_res(a, 0), -1);
    }
}
