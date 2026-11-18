//! duckOSM `routing.route_multimodal`: the fastest trip across modes (walk, drive, cycle) over the roads of `mm.edges` and the
//! changes of mode of `mm.transfers`. An edge-based Dijkstra: a road is followed by the roads of its mode's edge graph (the
//! turn restrictions), or, for a mode with no edge graph, by any road that leaves its end. With `enforce_sequence` the trip is
//! walk* (drive|cycle)* walk*. The same states, labels and heap keys as duckOSM, so a tie gives the same trip.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// The layer every change of mode goes through.
pub const HUB: &str = "walking";

/// The input, in the order the database gives it.
#[derive(Default, Clone, Debug)]
pub struct Network {
    /// `mm.edges`: (mode, source, target, edge_id, cost_s).
    pub edges: Vec<(String, i64, i64, i64, Option<f64>)>,
    /// `mm.transfers`: (node_id, from_mode, to_mode, cost_s, kind).
    pub transfers: Vec<(i64, String, String, Option<f64>, Option<String>)>,
    /// The legal turns (from_edge, to_edge) of each mode that has an edge graph, in its order.
    pub turns: HashMap<String, Vec<(i64, i64)>>,
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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum State {
    /// At the end of a road, its cost paid: (mode, edge_id, phase).
    E(usize, i64, u8),
    /// At a junction, just after a change of mode or at the start: (mode, node_id, phase).
    N(usize, i64, u8),
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

/// The steps of the fastest trip, or `Ok(None)` when the ends are not connected. `Err` names an end that is not in its layer.
#[allow(clippy::too_many_arguments)]
pub fn route(
    net: &Network,
    src: i64,
    dst: i64,
    start_mode: &str,
    end_mode: &str,
    enforce_sequence: bool,
    allowed: Option<&HashSet<String>>,
    points: Option<&Points>,
) -> Result<Option<Vec<Step>>, String> {
    let ok = |m: &str| allowed.is_none_or(|a| a.contains(m));
    let mut modes: Vec<String> = Vec::new();
    let mode_ix = |m: &str, modes: &mut Vec<String>| -> usize {
        match modes.iter().position(|x| x == m) {
            Some(i) => i,
            None => {
                modes.push(m.to_string());
                modes.len() - 1
            }
        }
    };
    let mut info: HashMap<(usize, i64), (i64, i64, f64)> = HashMap::new();
    let mut out_of: HashMap<(usize, i64), Vec<i64>> = HashMap::new();
    let mut nodes_by_mode: HashMap<usize, HashSet<i64>> = HashMap::new();
    for (m, s, t, e, c) in &net.edges {
        if !ok(m) {
            continue;
        }
        let mi = mode_ix(m, &mut modes);
        info.insert((mi, *e), (*s, *t, c.unwrap_or(0.0)));
        out_of.entry((mi, *s)).or_default().push(*e);
        let nm = nodes_by_mode.entry(mi).or_default();
        nm.insert(*s);
        nm.insert(*t);
    }
    let mut tadj: HashMap<(usize, i64), Vec<(usize, f64, Option<String>)>> = HashMap::new();
    for (n, fm, tm, c, k) in &net.transfers {
        if !ok(fm) || !ok(tm) {
            continue;
        }
        let (fi, ti) = (mode_ix(fm, &mut modes), mode_ix(tm, &mut modes));
        tadj.entry((fi, *n)).or_default().push((ti, c.unwrap_or(0.0), k.clone()));
    }
    let mut turns: HashMap<usize, HashMap<i64, Vec<i64>>> = HashMap::new();
    for (m, rows) in &net.turns {
        let Some(mi) = modes.iter().position(|x| x == m) else { continue };
        if !nodes_by_mode.contains_key(&mi) {
            continue;
        }
        let nxt = turns.entry(mi).or_default();
        for (f, t) in rows {
            if info.contains_key(&(mi, *t)) {
                nxt.entry(*f).or_default().push(*t);
            }
        }
    }
    let hub = modes.iter().position(|x| x == HUB);
    let ix = |m: &str| modes.iter().position(|x| x == m);
    let (smi, emi) = (ix(start_mode), ix(end_mode));
    if points.is_none() {
        if !smi.is_some_and(|i| nodes_by_mode.get(&i).is_some_and(|n| n.contains(&src))) {
            return Err(format!("src node {src} not in the '{start_mode}' layer of mm.edges"));
        }
        if !emi.is_some_and(|i| nodes_by_mode.get(&i).is_some_and(|n| n.contains(&dst))) {
            return Err(format!("dst node {dst} not in the '{end_mode}' layer of mm.edges"));
        }
    }
    let empty: Vec<i64> = Vec::new();
    let following = |mi: usize, e: i64| -> &Vec<i64> {
        match turns.get(&mi) {
            Some(t) => t.get(&e).unwrap_or(&empty),
            None => out_of.get(&(mi, info[&(mi, e)].1)).unwrap_or(&empty),
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
    let mut best_direct: Option<(f64, Vec<Step>)> = None;
    match points {
        None => push(0.0, State::N(smi.expect("checked"), src, init_phase), None, None, &mut dist, &mut pq),
        Some(p) => {
            let h = hub.ok_or("no walking roads in mm.edges")?;
            for &(e, frac, acc) in &p.starts {
                let c = info.get(&(h, e)).ok_or(format!("edge {e} is not a walking road of mm.edges"))?.2;
                push(acc + (1.0 - frac) * c, State::E(h, e, init_phase), None, Some(Step::Edge(e, HUB.into(), (1.0 - frac) * c, 1.0 - frac, acc)), &mut dist, &mut pq);
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
        let (mi, key, phase, is_edge) = match st {
            State::End => {
                goal = Some(st);
                break;
            }
            State::E(m, k, p) => (m, k, p, true),
            State::N(m, k, p) => (m, k, p, false),
        };
        let node = if is_edge { info[&(mi, key)].1 } else { key };
        if ends.is_none() && node == dst && Some(mi) == emi {
            goal = Some(st);
            break;
        }
        let roads: Vec<i64> = if is_edge { following(mi, key).clone() } else { out_of.get(&(mi, node)).cloned().unwrap_or_default() };
        for e in roads {
            // onto: take the road e; maybe stop part-way along it
            let c = info[&(mi, e)].2;
            push(d + c, State::E(mi, e, phase), Some(st), Some(Step::Edge(e, modes[mi].clone(), c, 1.0, 0.0)), &mut dist, &mut pq);
            if let Some(en) = ends {
                if Some(mi) == hub {
                    if let Some(&(g, acc_t)) = en.get(&e) {
                        push(d + g * c + acc_t, State::End, Some(st), Some(Step::Edge(e, modes[mi].clone(), g * c, g, acc_t)), &mut dist, &mut pq);
                    }
                }
            }
        }
        for (tm, cost, kind) in tadj.get(&(mi, node)).cloned().unwrap_or_default() {
            let mut nphase = phase;
            if enforce_sequence {
                let entering = Some(mi) == hub && Some(tm) != hub;
                let leaving = Some(mi) != hub && Some(tm) == hub;
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
            push(d + cost, State::N(tm, node, nphase), Some(st), Some(Step::Transfer(node, modes[mi].clone(), modes[tm].clone(), cost, kind)), &mut dist, &mut pq);
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
