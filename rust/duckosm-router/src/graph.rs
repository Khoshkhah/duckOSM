//! A directed graph with the order of networkx's `DiGraph` (nodes and neighbours in the order they are added; an arc that is
//! added again keeps its place and takes the new weight), and networkx's `bidirectional_dijkstra`, step by step: the same
//! heap keys `(distance, counter)`, so a tie gives the same path as in duckOSM.

use crate::Result;
use std::collections::{BinaryHeap, HashMap};

/// The graph, compact: the arcs of each node in two flat arrays (successors and predecessors), with their offsets. A node is a
/// number 0..n in the order the nodes came; `ids` gives its id.
#[derive(Default, Clone)]
pub struct Graph {
    /// The node ids, in the order they were added.
    pub ids: Vec<i64>,
    index: HashMap<i64, u32>,
    succ: Arcs,
    pred: Arcs,
}

/// The arcs of all nodes, node by node: node `i` has the arcs `off[i]..off[i + 1]` of `to` and `w`.
#[derive(Default, Clone)]
struct Arcs {
    off: Vec<usize>,
    to: Vec<u32>,
    w: Vec<f64>,
}

/// The arcs of a graph as they come; [`GraphBuilder::finish`] makes the [`Graph`].
#[derive(Default)]
pub struct GraphBuilder {
    ids: Vec<i64>,
    index: HashMap<i64, u32>,
    rows: Vec<(u32, u32, f64)>,
}

impl GraphBuilder {
    fn node(&mut self, id: i64) -> u32 {
        if let Some(&i) = self.index.get(&id) {
            return i;
        }
        let i = self.ids.len() as u32;
        self.ids.push(id);
        self.index.insert(id, i);
        i
    }

    /// networkx `add_weighted_edges_from([(u, v, w)])`.
    pub fn add_arc(&mut self, u: i64, v: i64, w: f64) {
        let (a, b) = (self.node(u), self.node(v));
        self.rows.push((a, b, w));
    }

    /// The graph: for each node its arcs in the order they first came; an arc that came again keeps its place and takes the
    /// last weight (networkx's adjacency dicts).
    pub fn finish(self) -> Graph {
        let n = self.ids.len();
        let csr = |key: &dyn Fn(&(u32, u32, f64)) -> (u32, u32)| -> Arcs {
            let mut off = vec![0usize; n + 1];
            for r in &self.rows {
                off[key(r).0 as usize + 1] += 1;
            }
            for i in 0..n {
                off[i + 1] += off[i];
            }
            let mut fill = off.clone();
            let mut to = vec![0u32; self.rows.len()];
            let mut w = vec![0.0f64; self.rows.len()];
            for r in &self.rows {
                let (a, b) = key(r);
                let k = fill[a as usize];
                to[k] = b;
                w[k] = r.2;
                fill[a as usize] += 1;
            }
            drop(fill);
            // an arc that came again: its first place, the last weight (in place: the kept arcs move to the front)
            let mut out_off = vec![0usize; n + 1];
            let mut k = 0usize;
            for i in 0..n {
                let start = k;
                for j in off[i]..off[i + 1] {
                    let (b, x) = (to[j], w[j]);
                    match (start..k).find(|&q| to[q] == b) {
                        Some(q) => w[q] = x,
                        None => {
                            to[k] = b;
                            w[k] = x;
                            k += 1;
                        }
                    }
                }
                out_off[i + 1] = k;
            }
            to.truncate(k);
            w.truncate(k);
            to.shrink_to_fit();
            w.shrink_to_fit();
            Arcs { off: out_off, to, w }
        };
        let succ = csr(&|r| (r.0, r.1));
        let pred = csr(&|r| (r.1, r.0));
        Graph { ids: self.ids, index: self.index, succ, pred }
    }
}

impl Graph {
    /// The graph of these arcs (u, v, weight), in their order.
    pub fn from_arcs(arcs: impl IntoIterator<Item = (i64, i64, f64)>) -> Graph {
        let mut b = GraphBuilder::default();
        for (u, v, w) in arcs {
            b.add_arc(u, v, w);
        }
        b.finish()
    }

    /// The arcs that leave node `i` (forward) or reach it (backward): (other node, weight).
    fn arcs_of(&self, i: usize, backward: bool) -> impl Iterator<Item = (usize, f64)> + '_ {
        let a = if backward { &self.pred } else { &self.succ };
        (a.off[i]..a.off[i + 1]).map(move |k| (a.to[k] as usize, a.w[k]))
    }

    pub fn index_of(&self, id: i64) -> Option<usize> {
        self.index.get(&id).map(|&i| i as usize)
    }

    pub fn contains(&self, id: i64) -> bool {
        self.index.contains_key(&id)
    }

    pub fn n_nodes(&self) -> usize {
        self.ids.len()
    }

    pub fn n_arcs(&self) -> usize {
        self.succ.to.len()
    }

    /// The arcs `(u, v, w)` in networkx's order of `G.edges(data=True)`: by node, then by neighbour.
    pub fn arcs(&self) -> impl Iterator<Item = (i64, i64, f64)> + '_ {
        (0..self.ids.len()).flat_map(move |u| self.arcs_of(u, false).map(move |(v, w)| (self.ids[u], self.ids[v], w)))
    }

    /// networkx `bidirectional_dijkstra(G, source, target)`: the length and the path (node ids), None when there is no path.
    pub fn bidirectional_dijkstra(&self, source: i64, target: i64) -> Result<Option<(f64, Vec<i64>)>> {
        let s = self.index_of(source).ok_or(format!("Source {source} is not in G"))?;
        let t = self.index_of(target).ok_or(format!("Target {target} is not in G"))?;
        if s == t {
            return Ok(Some((0.0, vec![source])));
        }
        // [forward, backward]: the label of each node (seen; NaN: not seen), whether it is final, and its parent
        let n = self.ids.len();
        const NONE: u32 = u32::MAX;
        let mut done: [Vec<bool>; 2] = [vec![false; n], vec![false; n]];
        let mut seen: [Vec<f64>; 2] = [vec![f64::NAN; n], vec![f64::NAN; n]];
        let mut parent: [Vec<u32>; 2] = [vec![NONE; n], vec![NONE; n]];
        let mut fringe: [BinaryHeap<Key>; 2] = [BinaryHeap::new(), BinaryHeap::new()];
        let mut c: u64 = 0;
        seen[0][s] = 0.0;
        seen[1][t] = 0.0;
        fringe[0].push(Key(0.0, c, s as u32));
        c += 1;
        fringe[1].push(Key(0.0, c, t as u32));
        c += 1;
        let path_of = |parent: &Vec<u32>, w: usize| -> Vec<usize> {
            let mut p = vec![w];
            let mut x = w;
            while parent[x] != NONE {
                x = parent[x] as usize;
                p.push(x);
            }
            p.reverse();
            p
        };
        let mut finaldist = f64::INFINITY;
        let mut finalpath: Vec<usize> = Vec::new();
        let mut dir = 1usize;
        while !fringe[0].is_empty() && !fringe[1].is_empty() {
            dir = 1 - dir;
            let Key(dist, _, v) = fringe[dir].pop().expect("not empty");
            let v = v as usize;
            if done[dir][v] {
                continue;
            }
            // dists[dir][v] = dist (equal to seen[dir][v])
            done[dir][v] = true;
            if done[1 - dir][v] {
                return Ok(Some((finaldist, finalpath.iter().map(|&i| self.ids[i]).collect())));
            }
            for (w, cost) in self.arcs_of(v, dir == 1) {
                // a NULL weight (NaN here): networkx skips the arc
                if cost.is_nan() {
                    continue;
                }
                let vw = dist + cost;
                if done[dir][w] {
                    if vw < seen[dir][w] {
                        return Err("Contradictory paths found: negative weights?".into());
                    }
                } else if seen[dir][w].is_nan() || vw < seen[dir][w] {
                    seen[dir][w] = vw;
                    fringe[dir].push(Key(vw, c, w as u32));
                    c += 1;
                    parent[dir][w] = v as u32;
                    if !seen[0][w].is_nan() && !seen[1][w].is_nan() {
                        let total = seen[0][w] + seen[1][w];
                        if finalpath.is_empty() || finaldist > total {
                            finaldist = total;
                            // paths[0][w] + reversed(paths[1][w])[1:], as they are now
                            let mut p = path_of(&parent[0], w);
                            let back = path_of(&parent[1], w);
                            p.extend(back.iter().rev().skip(1));
                            finalpath = p;
                        }
                    }
                }
            }
        }
        Ok(None)
    }
}

impl Graph {
    /// networkx `dijkstra_path(G, source, target)` (`_dijkstra_multisource` with one source and a target): the length and
    /// the path (node ids), None when there is no path.
    pub fn dijkstra(&self, source: i64, target: i64) -> Result<Option<(f64, Vec<i64>)>> {
        let s = self.index_of(source).ok_or(format!("Source {source} is not in G"))?;
        let t = self.index_of(target).ok_or(format!("Target {target} is not in G"))?;
        let n = self.ids.len();
        const NONE: u32 = u32::MAX;
        let mut done = vec![false; n];
        let mut seen = vec![f64::NAN; n];
        let mut parent = vec![NONE; n];
        let mut fringe = BinaryHeap::new();
        let mut c: u64 = 0;
        seen[s] = 0.0;
        fringe.push(Key(0.0, c, s as u32));
        c += 1;
        while let Some(Key(d, _, v)) = fringe.pop() {
            let v = v as usize;
            if done[v] {
                continue;
            }
            done[v] = true;
            if v == t {
                break;
            }
            for (u, cost) in self.arcs_of(v, false) {
                if cost.is_nan() {
                    continue;
                }
                let vu = d + cost;
                if done[u] {
                    if vu < seen[u] {
                        return Err("Contradictory paths found: negative weights?".into());
                    }
                } else if seen[u].is_nan() || vu < seen[u] {
                    seen[u] = vu;
                    fringe.push(Key(vu, c, u as u32));
                    c += 1;
                    parent[u] = v as u32;
                }
            }
        }
        if !done[t] {
            return Ok(None);
        }
        let mut path = vec![t];
        while parent[*path.last().expect("not empty")] != NONE {
            path.push(parent[*path.last().expect("not empty")] as usize);
        }
        path.reverse();
        Ok(Some((seen[t], path.into_iter().map(|i| self.ids[i]).collect())))
    }

    /// networkx `has_path`: is `target` reached from `source` (any weights)?
    pub fn has_path(&self, source: i64, target: i64) -> bool {
        let (Some(s), Some(t)) = (self.index_of(source), self.index_of(target)) else { return false };
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![s];
        seen.insert(s);
        while let Some(v) = stack.pop() {
            if v == t {
                return true;
            }
            for (u, _) in self.arcs_of(v, false) {
                if seen.insert(u) {
                    stack.push(u);
                }
            }
        }
        false
    }
}

/// A heap entry `(distance, counter, node)`, the smallest first (Python's `heapq` with tuples).
#[derive(PartialEq)]
struct Key(f64, u64, u32);

impl Eq for Key {}

impl Ord for Key {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

#[cfg(test)]
mod tests {
    use super::Graph;

    #[test]
    fn a_tie_takes_the_path_of_networkx() {
        // networkx: nx.bidirectional_dijkstra(G, 1, 4) on these arcs gives (2.0, [1, 2, 4])
        let g = Graph::from_arcs([(1, 2, 1.0), (1, 3, 1.0), (2, 4, 1.0), (3, 4, 1.0), (4, 5, 1.0), (1, 2, 1.0)]);
        assert_eq!(g.bidirectional_dijkstra(1, 4).unwrap(), Some((2.0, vec![1, 2, 4])));
        assert_eq!(g.bidirectional_dijkstra(5, 1).unwrap(), None);
        // networkx: nx.dijkstra_path(G, 1, 4) -> [1, 2, 4]; nx.has_path(G, 5, 1) -> False
        assert_eq!(g.dijkstra(1, 4).unwrap(), Some((2.0, vec![1, 2, 4])));
        assert!(!g.has_path(5, 1) && g.has_path(1, 5));
    }
}
