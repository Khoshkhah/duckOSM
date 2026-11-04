//! duckOSM's bus routes (`src/duckosm/bus.py`, docs/bus.md): each OSM bus route relation as the driving edges that its
//! buses travel, in order and in their direction. A relation lists its ways in travel order; each way is travelled from
//! the node that it shares with the way before to the node that it shares with the way after, so its direction comes
//! from its neighbors, and the role `forward` / `backward` (or `oneway`) decides where a neighbor cannot. The node
//! pairs of the route are then found in the driving edges and the bus lanes (`private_edges` with `access = 'bus'`).
//! What cannot be followed or found is a gap of the route, not an error.

use crate::graph::{MemberType, Relation};
use crate::model::Edge;
use crate::raw::RawWay;
use crate::util::FxHashMap;

const ROUTES: [&str; 3] = ["bus", "trolleybus", "share_taxi"];

/// A row of `bus.routes`.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub osm_id: i64,
    pub route: Option<String>,
    pub ref_: Option<String>,
    pub name: Option<String>,
    pub operator: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub network: Option<String>,
    /// The rows of the route in `bus.route_edges`.
    pub edges: i32,
    /// The member ways outside the area.
    pub outside: i32,
    /// `'<way_id> <kind>'` for each other way the route could not be followed on.
    pub gaps: Vec<String>,
}

/// A row of `bus.route_edges`.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteEdge {
    pub route_id: i64,
    pub ref_: Option<String>,
    pub seq: i32,
    pub edge_id: i64,
    pub edge_ref: Option<String>,
    /// The edge is in `driving.private_edges` (a bus lane or a bus-only road).
    pub bus_lane: bool,
}

fn tag<'a>(tags: &'a [(String, String)], k: &str) -> Option<&'a str> {
    tags.iter().find(|t| t.0 == k).map(|t| t.1.as_str())
}

/// +1 / -1 for a one-way way (along / against its drawing), else None.
fn oneway(tags: &[(String, String)]) -> Option<i8> {
    match tag(tags, "oneway") {
        Some("-1") => Some(-1),
        Some("yes" | "1" | "true") => Some(1),
        _ if matches!(tag(tags, "junction"), Some("roundabout" | "circular")) => Some(1),
        _ => None,
    }
}

fn index(nodes: &[i64], n: i64) -> usize {
    nodes
        .iter()
        .position(|&x| x == n)
        .expect("a node of the way")
}

/// The node where the route leaves way `a` for way `b`: a shared node, an end of both ways first, then an end of one,
/// then not `entry`, then the first in `a`'s order.
fn link(a: &[i64], b: &[i64], entry: Option<i64>) -> Option<i64> {
    let ends = |w: &[i64], n: i64| n == w[0] || n == w[w.len() - 1];
    a.iter()
        .copied()
        .filter(|n| b.contains(n))
        .min_by_key(|&n| {
            let (ea, eb) = (ends(a, n), ends(b, n));
            (!(ea && eb), !(ea || eb), Some(n) == entry, index(a, n))
        })
}

type Path = (Option<Vec<i64>>, Option<&'static str>);

/// The nodes of the way that the route travels, in its order, and a gap kind.
fn path(
    nodes: &[i64],
    entry: Option<i64>,
    exit: Option<i64>,
    role: Option<i8>,
    oneway: Option<i8>,
) -> Path {
    let want = role.or(oneway);
    let last = nodes.len() - 1;
    if nodes[0] == nodes[last] && nodes.len() > 2 {
        // a closed way (a roundabout): from the entry round to the exit
        let (Some(en), Some(ex), Some(w)) = (entry, exit, want) else {
            return (None, Some("ambiguous"));
        };
        let ring = &nodes[..last];
        let (i, j, k) = (
            index(ring, en) as i64,
            index(ring, ex) as i64,
            ring.len() as i64,
        );
        let w = w as i64;
        let steps = ((j - i) * w).rem_euclid(k);
        return (
            Some(
                (0..=steps)
                    .map(|s| ring[(i + w * s).rem_euclid(k) as usize])
                    .collect(),
            ),
            None,
        );
    }
    let pick = |w: Option<i8>, a: usize, b: usize| w.map(|w| if w == 1 { a } else { b });
    let (i, j) = match (entry, exit) {
        (Some(en), Some(ex)) => (Some(index(nodes, en)), Some(index(nodes, ex))),
        // the last way of a run: on to an end
        (Some(en), None) => {
            let i = index(nodes, en);
            (
                Some(i),
                if i == 0 {
                    Some(last)
                } else if i == last {
                    Some(0)
                } else {
                    pick(want, last, 0)
                },
            )
        }
        // the first way of a run: from an end
        (None, Some(ex)) => {
            let j = index(nodes, ex);
            (
                if j == last {
                    Some(0)
                } else if j == 0 {
                    Some(last)
                } else {
                    pick(want, 0, last)
                },
                Some(j),
            )
        }
        // a run of one way
        (None, None) => (pick(want, 0, last), pick(want, last, 0)),
    };
    let (Some(i), Some(j)) = (i, j) else {
        return (None, Some("ambiguous"));
    };
    let p: Vec<i64> = if i <= j {
        nodes[i..=j].to_vec()
    } else {
        nodes[j..=i].iter().rev().copied().collect()
    };
    let conflict = role.is_some_and(|r| i != j && r != if j > i { 1 } else { -1 });
    (Some(p), conflict.then_some("role_conflict"))
}

/// The walk of one relation: its member ways `(member, way, role)` -> steps `(member, way, u, v)` and gaps
/// `(member, way, kind)`.
#[allow(clippy::type_complexity)]
fn walk(
    members: &[(usize, i64, Option<i8>)],
    ways: &FxHashMap<i64, (&[i64], Option<i8>)>,
) -> (Vec<(usize, i64, i64, i64)>, Vec<(usize, i64, &'static str)>) {
    let (mut steps, mut gaps, mut run) = (
        Vec::new(),
        Vec::new(),
        Vec::<(usize, i64, Option<i8>)>::new(),
    );
    let flush = |run: &mut Vec<(usize, i64, Option<i8>)>, steps: &mut Vec<_>, gaps: &mut Vec<_>| {
        let mut entry = None;
        for k in 0..run.len() {
            let (m, w, role) = run[k];
            let (nodes, ow) = ways[&w];
            let exit = run
                .get(k + 1)
                .and_then(|next| link(nodes, ways[&next.1].0, entry));
            let (p, kind) = path(nodes, entry, exit, role, ow);
            if let Some(kind) = kind {
                gaps.push((m, w, kind));
            }
            if let Some(p) = p {
                steps.extend(p.windows(2).map(|uv| (m, w, uv[0], uv[1])));
            }
            entry = exit;
        }
        run.clear();
    };
    for &(m, w, role) in members {
        if !ways.contains_key(&w) {
            flush(&mut run, &mut steps, &mut gaps);
            gaps.push((m, w, "outside"));
            continue;
        }
        if let Some(&(pm, pw, _)) = run.last() {
            if link(ways[&pw].0, ways[&w].0, None).is_none() {
                gaps.push((pm, pw, "no_connection"));
                flush(&mut run, &mut steps, &mut gaps);
            }
        }
        run.push((m, w, role));
    }
    flush(&mut run, &mut steps, &mut gaps);
    (steps, gaps)
}

/// `bus.routes` and `bus.route_edges` (duckOSM `write_bus_routes`): the route relations of `relations` (raw.relations),
/// walked on `ways` (raw.ways, sorted by id), on the driving `edges` and `private_edges`. One difference from duckOSM:
/// a node pair on two driving edges in the same direction is the gap `two_edges`, where duckOSM stops with an error.
pub fn routes(
    relations: &[Relation],
    ways: &[RawWay],
    edges: &[Edge],
    private_edges: &[Edge],
) -> (Vec<Route>, Vec<RouteEdge>) {
    let rels: Vec<&Relation> = relations
        .iter()
        .filter(|r| tag(&r.tags, "route").is_some_and(|v| ROUTES.contains(&v)))
        .collect();
    let mut way_of: FxHashMap<i64, (&[i64], Option<i8>)> = FxHashMap::default();
    for r in &rels {
        for (id, t, _) in &r.members {
            if *t == MemberType::Way {
                if let Ok(i) = ways.binary_search_by_key(id, |w| w.id) {
                    way_of.insert(*id, (&ways[i].refs[..], oneway(&ways[i].tags)));
                }
            }
        }
    }
    // (osm_id, u, v) -> the edges with that node pair, in that order
    type Pairs<'a> = FxHashMap<(i64, i64, i64), Vec<(&'a Edge, bool)>>;
    let mut pairs: Pairs = FxHashMap::default();
    for (list, lane) in [(edges, false), (private_edges, true)] {
        for e in list {
            for uv in e.refs.windows(2) {
                pairs
                    .entry((e.osm_id, uv[0], uv[1]))
                    .or_default()
                    .push((e, lane));
            }
        }
    }
    // the gaps in the order duckOSM makes them: the roles, the walks, then the pairs without an edge
    let mut gaps: Vec<(i64, usize, i64, &str)> = Vec::new();
    let mut steps: Vec<(i64, usize, i64, i64, i64)> = Vec::new();
    for r in &rels {
        let mut members = Vec::new();
        for (m, (id, t, role)) in r.members.iter().enumerate() {
            let m = m + 1;
            if *t != MemberType::Way || role.starts_with("platform") || role.starts_with("stop") {
                continue; // a stop or a platform, not the road
            }
            let dir = match role.as_str() {
                "" => None,
                "forward" => Some(1),
                "backward" => Some(-1),
                _ => {
                    gaps.push((r.id, m, *id, "role"));
                    continue;
                }
            };
            members.push((m, *id, dir));
        }
        let (s, g) = walk(&members, &way_of);
        steps.extend(s.into_iter().map(|(m, w, u, v)| (r.id, m, w, u, v)));
        gaps.extend(g.into_iter().map(|(m, w, k)| (r.id, m, w, k)));
    }
    let mut out: Vec<RouteEdge> = Vec::new();
    let mut last_ok: Option<(i64, i64)> = None;
    let mut last_miss: Option<(i64, usize, &str)> = None;
    let mut seq: FxHashMap<i64, i32> = FxHashMap::default();
    let refs: FxHashMap<i64, Option<String>> = rels
        .iter()
        .map(|r| (r.id, tag(&r.tags, "ref").map(str::to_string)))
        .collect();
    for (rid, m, w, u, v) in steps {
        let found = pairs.get(&(w, u, v)).map_or(&[][..], |x| &x[..]);
        let miss = match found {
            [] => Some("no_edge"),
            [(e, lane)] if *lane && e.access.as_deref() != Some("bus") => {
                Some(if e.access.as_deref() == Some("private") {
                    "private"
                } else {
                    "no_edge"
                })
            }
            [_] => None,
            _ => Some("two_edges"),
        };
        match miss {
            Some(kind) => {
                // one gap for a run of pairs of one member
                if last_miss != Some((rid, m, kind)) {
                    gaps.push((rid, m, w, kind));
                }
                last_miss = Some((rid, m, kind));
            }
            None => {
                let (e, lane) = found[0];
                // an edge once for each stretch that the route travels on it
                if last_ok != Some((rid, e.edge_id)) {
                    let s = seq.entry(rid).or_insert(0);
                    *s += 1;
                    out.push(RouteEdge {
                        route_id: rid,
                        ref_: refs[&rid].clone(),
                        seq: *s,
                        edge_id: e.edge_id,
                        edge_ref: e.edge_ref(),
                        bus_lane: lane,
                    });
                }
                last_ok = Some((rid, e.edge_id));
            }
        }
    }
    gaps.sort_by_key(|g| (g.0, g.1)); // stable: the order of the kinds of one member stays
    let routes = rels
        .iter()
        .map(|r| {
            let own = gaps.iter().filter(|g| g.0 == r.id);
            let s = |k: &str| tag(&r.tags, k).map(str::to_string);
            Route {
                osm_id: r.id,
                route: s("route"),
                ref_: s("ref"),
                name: s("name"),
                operator: s("operator"),
                from: s("from"),
                to: s("to"),
                network: s("network"),
                edges: seq.get(&r.id).copied().unwrap_or(0),
                outside: own.clone().filter(|g| g.3 == "outside").count() as i32,
                gaps: own
                    .filter(|g| g.3 != "outside")
                    .map(|g| format!("{} {}", g.2, g.3))
                    .collect(),
            }
        })
        .collect();
    (routes, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(id: i64, refs: &[i64]) -> (i64, Vec<i64>) {
        (id, refs.to_vec())
    }

    fn walk_of(
        ws: &[(i64, Vec<i64>, Option<i8>)],
        members: &[(i64, Option<i8>)],
    ) -> (Vec<(i64, i64)>, Vec<&'static str>) {
        let ways: FxHashMap<i64, (&[i64], Option<i8>)> =
            ws.iter().map(|(id, r, o)| (*id, (&r[..], *o))).collect();
        let m: Vec<(usize, i64, Option<i8>)> = members
            .iter()
            .enumerate()
            .map(|(i, (id, r))| (i + 1, *id, *r))
            .collect();
        let (s, g) = walk(&m, &ways);
        (
            s.iter().map(|x| (x.2, x.3)).collect(),
            g.iter().map(|x| x.2).collect(),
        )
    }

    #[test]
    fn a_street_there_and_back() {
        // way 1: 1-2-3, way 2: 3-4; the route goes 1..4 and back on way 2 then way 1
        let (a, b) = (w(1, &[1, 2, 3]), w(2, &[3, 4]));
        let ws = [(a.0, a.1, None), (b.0, b.1, None)];
        let (s, g) = walk_of(&ws, &[(1, None), (2, None), (2, None), (1, None)]);
        assert!(g.is_empty(), "{g:?}");
        assert_eq!(s, vec![(1, 2), (2, 3), (3, 4), (4, 3), (3, 2), (2, 1)]);
    }

    #[test]
    fn roundabout_and_gaps() {
        // a roundabout 10-11-12-13-10 (one-way), entered at 10 from way 1 (1-10), left at 12 to way 3 (12-20)
        let ws = [
            (1, vec![1, 10], None),
            (2, vec![10, 11, 12, 13, 10], Some(1)),
            (3, vec![12, 20], None),
        ];
        let (s, g) = walk_of(&ws, &[(1, None), (2, None), (3, None)]);
        assert!(g.is_empty());
        assert_eq!(s, vec![(1, 10), (10, 11), (11, 12), (12, 20)]);
        // a role against the direction of the neighbors: the neighbors win, with a gap
        let (_, g) = walk_of(
            &[(1, vec![1, 2], None), (2, vec![2, 3], None)],
            &[(1, Some(-1)), (2, None)],
        );
        assert_eq!(g, vec!["role_conflict"]);
        // a run of one two-way way without a role: ambiguous; a way that is not there: outside
        let (s, g) = walk_of(&[(1, vec![1, 2], None)], &[(1, None), (9, None)]);
        assert!(s.is_empty());
        assert_eq!(g, vec!["ambiguous", "outside"]);
    }
}
