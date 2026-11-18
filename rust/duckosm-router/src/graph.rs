//! A directed graph with the order of networkx's `DiGraph` (nodes and neighbours in the order they are added; an arc that is
//! added again keeps its place and takes the new weight), and networkx's `bidirectional_dijkstra`, step by step: the same
//! heap keys `(distance, counter)`, so a tie gives the same path as in duckOSM.

use crate::Result;
use std::collections::{BinaryHeap, HashMap};

#[derive(Default, Clone)]
pub struct Graph {
    /// The node ids, in the order they were added.
    pub ids: Vec<i64>,
    index: HashMap<i64, usize>,
    succ: Vec<Vec<(usize, f64)>>,
    pred: Vec<Vec<(usize, f64)>>,
}

impl Graph {
    pub fn node(&mut self, id: i64) -> usize {
        if let Some(&i) = self.index.get(&id) {
            return i;
        }
        let i = self.ids.len();
        self.ids.push(id);
        self.index.insert(id, i);
        self.succ.push(Vec::new());
        self.pred.push(Vec::new());
        i
    }

    /// networkx `add_weighted_edges_from([(u, v, w)])`.
    pub fn add_arc(&mut self, u: i64, v: i64, w: f64) {
        let (a, b) = (self.node(u), self.node(v));
        match self.succ[a].iter_mut().find(|x| x.0 == b) {
            Some(x) => x.1 = w,
            None => self.succ[a].push((b, w)),
        }
        match self.pred[b].iter_mut().find(|x| x.0 == a) {
            Some(x) => x.1 = w,
            None => self.pred[b].push((a, w)),
        }
    }

    pub fn index_of(&self, id: i64) -> Option<usize> {
        self.index.get(&id).copied()
    }

    pub fn contains(&self, id: i64) -> bool {
        self.index.contains_key(&id)
    }

    pub fn n_nodes(&self) -> usize {
        self.ids.len()
    }

    pub fn n_arcs(&self) -> usize {
        self.succ.iter().map(Vec::len).sum()
    }

    /// The arcs `(u, v, w)` in networkx's order of `G.edges(data=True)`: by node, then by neighbour.
    pub fn arcs(&self) -> impl Iterator<Item = (i64, i64, f64)> + '_ {
        self.succ.iter().enumerate().flat_map(move |(u, s)| s.iter().map(move |&(v, w)| (self.ids[u], self.ids[v], w)))
    }

    /// networkx `bidirectional_dijkstra(G, source, target)`: the length and the path (node ids), None when there is no path.
    pub fn bidirectional_dijkstra(&self, source: i64, target: i64) -> Result<Option<(f64, Vec<i64>)>> {
        let s = self.index_of(source).ok_or(format!("Source {source} is not in G"))?;
        let t = self.index_of(target).ok_or(format!("Target {target} is not in G"))?;
        if s == t {
            return Ok(Some((0.0, vec![source])));
        }
        let n = self.ids.len();
        // [forward, backward]
        let mut dists: [Vec<Option<f64>>; 2] = [vec![None; n], vec![None; n]];
        let mut seen: [Vec<Option<f64>>; 2] = [vec![None; n], vec![None; n]];
        let mut parent: [Vec<usize>; 2] = [vec![usize::MAX; n], vec![usize::MAX; n]];
        let mut fringe: [BinaryHeap<Key>; 2] = [BinaryHeap::new(), BinaryHeap::new()];
        let mut c: u64 = 0;
        seen[0][s] = Some(0.0);
        seen[1][t] = Some(0.0);
        fringe[0].push(Key(0.0, c, s));
        c += 1;
        fringe[1].push(Key(0.0, c, t));
        c += 1;
        let neighs = [&self.succ, &self.pred];
        let path_of = |parent: &Vec<usize>, w: usize| -> Vec<usize> {
            let mut p = vec![w];
            let mut x = w;
            while parent[x] != usize::MAX {
                x = parent[x];
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
            if dists[dir][v].is_some() {
                continue;
            }
            dists[dir][v] = Some(dist);
            if dists[1 - dir][v].is_some() {
                return Ok(Some((finaldist, finalpath.iter().map(|&i| self.ids[i]).collect())));
            }
            for &(w, cost) in &neighs[dir][v] {
                // a NULL weight (NaN here): networkx skips the arc
                if cost.is_nan() {
                    continue;
                }
                let vw = dist + cost;
                if let Some(dw) = dists[dir][w] {
                    if vw < dw {
                        return Err("Contradictory paths found: negative weights?".into());
                    }
                } else if seen[dir][w].is_none_or(|sw| vw < sw) {
                    seen[dir][w] = Some(vw);
                    fringe[dir].push(Key(vw, c, w));
                    c += 1;
                    parent[dir][w] = v;
                    if let (Some(a), Some(b)) = (seen[0][w], seen[1][w]) {
                        let total = a + b;
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
        let mut dist: Vec<Option<f64>> = vec![None; n];
        let mut seen: Vec<Option<f64>> = vec![None; n];
        let mut parent: Vec<usize> = vec![usize::MAX; n];
        let mut fringe = BinaryHeap::new();
        let mut c: u64 = 0;
        seen[s] = Some(0.0);
        fringe.push(Key(0.0, c, s));
        c += 1;
        while let Some(Key(d, _, v)) = fringe.pop() {
            if dist[v].is_some() {
                continue;
            }
            dist[v] = Some(d);
            if v == t {
                break;
            }
            for &(u, cost) in &self.succ[v] {
                if cost.is_nan() {
                    continue;
                }
                let vu = d + cost;
                if let Some(du) = dist[u] {
                    if vu < du {
                        return Err("Contradictory paths found: negative weights?".into());
                    }
                } else if seen[u].is_none_or(|su| vu < su) {
                    seen[u] = Some(vu);
                    fringe.push(Key(vu, c, u));
                    c += 1;
                    parent[u] = v;
                }
            }
        }
        let Some(d) = dist[t] else { return Ok(None) };
        let mut path = vec![t];
        while path.last() != Some(&s) {
            path.push(parent[*path.last().expect("not empty")]);
        }
        path.reverse();
        Ok(Some((d, path.into_iter().map(|i| self.ids[i]).collect())))
    }

    /// networkx `has_path`: is `target` reached from `source` (any weights)?
    pub fn has_path(&self, source: i64, target: i64) -> bool {
        let (Some(s), Some(t)) = (self.index_of(source), self.index_of(target)) else { return false };
        let mut seen = vec![false; self.ids.len()];
        let mut stack = vec![s];
        seen[s] = true;
        while let Some(v) = stack.pop() {
            if v == t {
                return true;
            }
            for &(u, _) in &self.succ[v] {
                if !seen[u] {
                    seen[u] = true;
                    stack.push(u);
                }
            }
        }
        false
    }
}

/// A heap entry `(distance, counter, node)`, the smallest first (Python's `heapq` with tuples).
#[derive(PartialEq)]
struct Key(f64, u64, usize);

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
        let mut g = Graph::default();
        for (u, v, w) in [(1, 2, 1.0), (1, 3, 1.0), (2, 4, 1.0), (3, 4, 1.0), (4, 5, 1.0)] {
            g.add_arc(u, v, w);
        }
        assert_eq!(g.bidirectional_dijkstra(1, 4).unwrap(), Some((2.0, vec![1, 2, 4])));
        assert_eq!(g.bidirectional_dijkstra(5, 1).unwrap(), None);
        // networkx: nx.dijkstra_path(G, 1, 4) -> [1, 2, 4]; nx.has_path(G, 5, 1) -> False
        assert_eq!(g.dijkstra(1, 4).unwrap(), Some((2.0, vec![1, 2, 4])));
        assert!(!g.has_path(5, 1) && g.has_path(1, 5));
    }
}
