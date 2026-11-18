//! duckOSM `routing.route_multimodal`: the fastest trip across modes (walk, drive, cycle) over the roads of `mm.edges` and the
//! changes of mode of `mm.transfers`. An edge-based Dijkstra: a road is followed by the roads of its mode's edge graph (the
//! turn restrictions), or, for a mode with no edge graph, by any road that leaves its end. With `enforce_sequence` the trip is
//! walk* (drive|cycle)* walk*. The same states, labels and heap keys as duckOSM, so a tie gives the same trip.
//!
//! duckOSM reads the tables and makes its indexes for each trip. Here [`Prepared::new`] makes them one time, in compact arrays
//! (a large area: France), and [`Prepared::route`] answers each trip; the trip is the same.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// The layer every change of mode goes through.
pub const HUB: &str = "walking";

/// The input, in the order the database gives it. A mode is a number: its place in `modes`.
#[derive(Default, Clone, Debug)]
pub struct Network {
    pub modes: Vec<String>,
    /// `mm.edges`: (mode, source, target, edge_id, cost_s).
    pub edges: Vec<(u8, i64, i64, i64, Option<f64>)>,
    /// `mm.transfers`: (node_id, from_mode, to_mode, cost_s, kind).
    pub transfers: Vec<(i64, u8, u8, Option<f64>, Option<String>)>,
    /// The legal turns (from_edge, to_edge) of each mode that has an edge graph, in its order.
    pub turns: HashMap<u8, Vec<(i64, i64)>>,
}

impl Network {
    /// The number of the mode `m` (a new mode gets the next number).
    pub fn mode(&mut self, m: &str) -> u8 {
        match self.modes.iter().position(|x| x == m) {
            Some(i) => i as u8,
            None => {
                self.modes.push(m.to_string());
                (self.modes.len() - 1) as u8
            }
        }
    }
}

/// One step of a trip: a road (or the part of it from or to a point), or a change of mode.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// edge_id, mode, cost, the fraction travelled, the access walk.
    Edge(i64, String, f64, f64, f64),
    /// node_id, from_mode, to_mode, cost, kind.
    Transfer(i64, String, String, f64, Option<String>),
}

/// The ends of a trip between points (duckOSM `route_multimodal_points`): `starts` (edge_id, fraction, access_s) and
/// `ends` (edge_id -> (fraction, access_s)), all on walking roads.
pub struct Points {
    pub starts: Vec<(i64, f64, f64)>,
    pub ends: HashMap<i64, (f64, f64)>,
}

/// The roads of one mode: by number (the order of `mm.edges`), their ids, ends and costs; the roads that leave each node;
/// the legal turns from each road.
#[derive(Default)]
struct Layer {
    eid: Vec<i64>,
    target: Vec<i64>,
    cost: Vec<f64>,
    index: HashMap<i64, u32>,
    /// node -> the range of `out` of the roads that leave it (empty for a node that is only an end)
    nodes: HashMap<i64, (u32, u32)>,
    out: Vec<u32>,
    /// from road -> the range of `next`, when the mode has an edge graph
    turns: Option<(Vec<usize>, Vec<u32>)>,
}

/// The indexes of `mm.*` for the modes of one trip set (duckOSM `route_multimodal` with `allowed_modes`).
pub struct Prepared {
    modes: Vec<String>,
    layers: Vec<Layer>,
    /// (from_mode, node) -> (to_mode, cost, kind)
    tadj: HashMap<(u8, i64), Vec<(u8, f64, Option<u16>)>>,
    kinds: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum State {
    /// At the end of a road, its cost paid: (mode, road number, phase).
    E(u8, u32, u8),
    /// At a junction, just after a change of mode or at the start: (mode, node_id, phase).
    N(u8, i64, u8),
    /// The end of a trip that stops part-way along a road.
    End,
}

#[derive(PartialEq)]
struct Key(f64, u64, State);
impl Eq for Key {}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Prepared {
    /// The indexes of `net` (only its modes in `allowed`, when given; `net` is used up).
    pub fn new(net: Network, allowed: Option<&[String]>) -> Prepared {
        let ok = |m: u8| allowed.is_none_or(|a| a.iter().any(|x| x == &net.modes[m as usize]));
        let mut layers: Vec<Layer> = (0..net.modes.len()).map(|_| Layer::default()).collect();
        let mut sources: Vec<Vec<i64>> = vec![Vec::new(); net.modes.len()];
        for &(m, s, t, e, c) in &net.edges {
            if !ok(m) {
                continue;
            }
            let l = &mut layers[m as usize];
            match l.index.get(&e) {
                // the same road again: duckOSM's dict keeps the last values (and the road leaves its node a second time)
                Some(&i) => {
                    l.target[i as usize] = t;
                    l.cost[i as usize] = c.unwrap_or(0.0);
                }
                None => {
                    l.index.insert(e, l.eid.len() as u32);
                    l.eid.push(e);
                    l.target.push(t);
                    l.cost.push(c.unwrap_or(0.0));
                }
            }
            sources[m as usize].push(s);
            let i = l.index[&e];
            l.out.push(i);
        }
        for (m, l) in layers.iter_mut().enumerate() {
            // the roads that leave each node, in the order of mm.edges: a stable sort of `out` by its source
            let src = std::mem::take(&mut sources[m]);
            let mut order: Vec<usize> = (0..l.out.len()).collect();
            order.sort_by_key(|&k| src[k]);
            let out: Vec<u32> = order.iter().map(|&k| l.out[k]).collect();
            let mut k = 0;
            while k < order.len() {
                let s = src[order[k]];
                let start = k;
                while k < order.len() && src[order[k]] == s {
                    k += 1;
                }
                l.nodes.insert(s, (start as u32, k as u32));
            }
            for &t in &l.target {
                l.nodes.entry(t).or_insert((0, 0));
            }
            l.out = out;
        }
        for (m, rows) in &net.turns {
            let l = &mut layers[*m as usize];
            if !ok(*m) || l.eid.is_empty() {
                continue;
            }
            // from road -> the roads after it that are in the layer, in the order of the edge graph
            let mut pairs: Vec<(u32, u32)> = Vec::new();
            for (f, t) in rows {
                if let (Some(&a), Some(&b)) = (l.index.get(f), l.index.get(t)) {
                    pairs.push((a, b));
                }
            }
            let n = l.eid.len();
            let mut off = vec![0usize; n + 1];
            for p in &pairs {
                off[p.0 as usize + 1] += 1;
            }
            for i in 0..n {
                off[i + 1] += off[i];
            }
            let mut fill = off.clone();
            let mut next = vec![0u32; pairs.len()];
            for (a, b) in pairs {
                next[fill[a as usize]] = b;
                fill[a as usize] += 1;
            }
            l.turns = Some((off, next));
        }
        let mut kinds: Vec<String> = Vec::new();
        let mut tadj: HashMap<(u8, i64), Vec<(u8, f64, Option<u16>)>> = HashMap::new();
        for (n, fm, tm, c, k) in &net.transfers {
            if !ok(*fm) || !ok(*tm) {
                continue;
            }
            let k = k.as_ref().map(|k| match kinds.iter().position(|x| x == k) {
                Some(i) => i as u16,
                None => {
                    kinds.push(k.clone());
                    (kinds.len() - 1) as u16
                }
            });
            tadj.entry((*fm, *n)).or_default().push((*tm, c.unwrap_or(0.0), k));
        }
        Prepared { modes: net.modes, layers, tadj, kinds }
    }

    fn mode_ix(&self, m: &str) -> Option<u8> {
        self.modes.iter().position(|x| x == m).map(|i| i as u8)
    }

    /// The end node of road `e` of mode `m` (None when the mode has no such road).
    pub fn target(&self, m: &str, e: i64) -> Option<i64> {
        let l = &self.layers[self.mode_ix(m)? as usize];
        l.index.get(&e).map(|&i| l.target[i as usize])
    }

    fn has_node(&self, m: Option<u8>, n: i64) -> bool {
        m.is_some_and(|m| self.layers[m as usize].nodes.contains_key(&n))
    }

    /// The steps of the fastest trip, or `Ok(None)` when the ends are not connected. `Err` names an end that is not in its
    /// layer.
    pub fn route(&self, src: i64, dst: i64, start_mode: &str, end_mode: &str, enforce_sequence: bool, points: Option<&Points>) -> Result<Option<Vec<Step>>, String> {
        let hub = self.mode_ix(HUB);
        let (smi, emi) = (self.mode_ix(start_mode), self.mode_ix(end_mode));
        if points.is_none() {
            if !self.has_node(smi, src) {
                return Err(format!("src node {src} not in the '{start_mode}' layer of mm.edges"));
            }
            if !self.has_node(emi, dst) {
                return Err(format!("dst node {dst} not in the '{end_mode}' layer of mm.edges"));
            }
        }
        let following = |m: u8, i: u32| -> &[u32] {
            let l = &self.layers[m as usize];
            match &l.turns {
                Some((off, next)) => &next[off[i as usize]..off[i as usize + 1]],
                None => match l.nodes.get(&l.target[i as usize]) {
                    Some(&(a, b)) => &l.out[a as usize..b as usize],
                    None => &[],
                },
            }
        };
        let leaving = |m: u8, n: i64| -> &[u32] {
            let l = &self.layers[m as usize];
            match l.nodes.get(&n) {
                Some(&(a, b)) => &l.out[a as usize..b as usize],
                None => &[],
            }
        };
        let init_phase: u8 = if enforce_sequence && start_mode != HUB { 1 } else { 0 };
        let mut dist: HashMap<State, f64> = HashMap::new();
        let mut prev: HashMap<State, (Option<State>, Option<Step>)> = HashMap::new();
        let mut pq = BinaryHeap::new();
        let mut n_push: u64 = 0;
        let mut push = |d: f64, st: State, before: Option<State>, step: Option<Step>, dist: &mut HashMap<State, f64>, pq: &mut BinaryHeap<Key>| {
            if dist.get(&st).is_none_or(|&x| d < x) {
                dist.insert(st, d);
                prev.insert(st, (before, step));
                n_push += 1;
                pq.push(Key(d, n_push, st));
            }
        };
        let name = |m: u8| self.modes[m as usize].clone();
        let mut best_direct: Option<(f64, Vec<Step>)> = None;
        match points {
            None => push(0.0, State::N(smi.expect("checked"), src, init_phase), None, None, &mut dist, &mut pq),
            Some(p) => {
                let h = hub.ok_or("no walking roads in mm.edges")?;
                let l = &self.layers[h as usize];
                for &(e, frac, acc) in &p.starts {
                    let i = *l.index.get(&e).ok_or(format!("edge {e} is not a walking road of mm.edges"))?;
                    let c = l.cost[i as usize];
                    push(acc + (1.0 - frac) * c, State::E(h, i, init_phase), None, Some(Step::Edge(e, HUB.into(), (1.0 - frac) * c, 1.0 - frac, acc)), &mut dist, &mut pq);
                    if let Some(&(g, acc_t)) = p.ends.get(&e) {
                        if frac <= g {
                            let total = acc + (g - frac) * c + acc_t;
                            if best_direct.as_ref().is_none_or(|b| total < b.0) {
                                best_direct = Some((total, vec![Step::Edge(e, HUB.into(), (g - frac) * c, g - frac, acc + acc_t)]));
                            }
                        }
                    }
                }
            }
        }
        let ends = points.map(|p| &p.ends).filter(|e| !e.is_empty());
        let mut goal: Option<State> = None;
        while let Some(Key(d, _, st)) = pq.pop() {
            if dist.get(&st).is_some_and(|&x| d > x) {
                continue;
            }
            let (m, key, phase, is_edge) = match st {
                State::End => {
                    goal = Some(st);
                    break;
                }
                State::E(m, k, p) => (m, k as i64, p, true),
                State::N(m, k, p) => (m, k, p, false),
            };
            let l = &self.layers[m as usize];
            let node = if is_edge { l.target[key as usize] } else { key };
            if ends.is_none() && node == dst && Some(m) == emi {
                goal = Some(st);
                break;
            }
            let roads = if is_edge { following(m, key as u32) } else { leaving(m, node) };
            for &i in roads {
                // onto: take the road i; maybe stop part-way along it
                let (e, c) = (l.eid[i as usize], l.cost[i as usize]);
                push(d + c, State::E(m, i, phase), Some(st), Some(Step::Edge(e, name(m), c, 1.0, 0.0)), &mut dist, &mut pq);
                if let Some(en) = ends {
                    if Some(m) == hub {
                        if let Some(&(g, acc_t)) = en.get(&e) {
                            push(d + g * c + acc_t, State::End, Some(st), Some(Step::Edge(e, name(m), g * c, g, acc_t)), &mut dist, &mut pq);
                        }
                    }
                }
            }
            for &(tm, cost, kind) in self.tadj.get(&(m, node)).map(Vec::as_slice).unwrap_or(&[]) {
                let mut nphase = phase;
                if enforce_sequence {
                    let entering = Some(m) == hub && Some(tm) != hub;
                    let leaving = Some(m) != hub && Some(tm) == hub;
                    if entering {
                        if phase != 0 {
                            continue;
                        }
                        nphase = 1;
                    } else if leaving {
                        if phase != 1 {
                            continue;
                        }
                        nphase = 2;
                    }
                }
                let kind = kind.map(|k| self.kinds[k as usize].clone());
                push(d + cost, State::N(tm, node, nphase), Some(st), Some(Step::Transfer(node, name(m), name(tm), cost, kind)), &mut dist, &mut pq);
            }
        }
        if let Some((total, steps)) = &best_direct {
            if goal.is_none_or(|g| *total <= dist[&g]) {
                return Ok(Some(steps.clone()));
            }
        }
        let Some(mut s) = goal else { return Ok(None) };
        let mut steps = Vec::new();
        loop {
            let (before, step) = prev[&s].clone();
            if let Some(step) = step {
                steps.push(step);
            }
            match before {
                Some(b) => s = b,
                None => break,
            }
        }
        steps.reverse();
        Ok(Some(steps))
    }
}
