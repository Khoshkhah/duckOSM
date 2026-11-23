//! The chains of a mode (`<mode>.chains`, docs/reference/database.md): the edges joined at the nodes where the road only
//! continues. The network keeps every cut (the multimodal routing needs them). An export that asks for it (SUMO `merge`)
//! writes one edge for each chain.

use std::collections::{HashMap, HashSet};

/// The traffic lights and stop signs of OSM on the network nodes, as `NODE_TYPES_SQL` of sumo.py (the same text). The SUMO export makes
/// these nodes its traffic lights and stop signs, so a chain does not go through them: node id, type
/// (3 traffic light, 2 all-way stop, 1 stop), the shared program id (`tl`) of the lights within 20 m of each other.
pub const NODE_TYPES_SQL: &str = r#"WITH s AS (SELECT osm_id, lon, lat, CASE WHEN tags['highway'] = 'traffic_signals' THEN 3 WHEN tags['stop'] = 'all' THEN 2 ELSE 1 END AS t
    FROM raw.nodes WHERE tags['highway'] IN ('traffic_signals', 'stop')),
n AS (SELECT node_id, ST_X(geom) AS x, ST_Y(geom) AS y FROM {mode}.nodes WHERE geom IS NOT NULL),
c AS (SELECT DISTINCT s.osm_id, s.t, n.node_id, 111320 * sqrt(pow(n.y - s.lat, 2) + pow((n.x - s.lon) * cos(radians(s.lat)), 2)) AS d
    FROM s JOIN {mode}.way_nodes a ON a.node_id = s.osm_id JOIN {mode}.way_nodes b ON b.way_id = a.way_id JOIN n ON n.node_id = b.node_id),
k AS (SELECT * FROM c QUALIFY row_number() OVER (PARTITION BY osm_id ORDER BY d, node_id) = 1),
t AS (SELECT k.node_id, max(k.t) AS t, any_value(n.x) AS x, any_value(n.y) AS y FROM k JOIN n USING (node_id) WHERE k.d <= 25 GROUP BY k.node_id),
g AS (SELECT a.node_id, min(b.node_id) AS tl, count(*) AS m FROM t a JOIN t b ON a.t = 3 AND b.t = 3
    AND 111320 * sqrt(pow(b.y - a.y, 2) + pow((b.x - a.x) * cos(radians(a.y)), 2)) <= 20 GROUP BY a.node_id)
SELECT t.node_id, t.t, CASE WHEN g.m > 1 THEN g.tl END FROM t LEFT JOIN g USING (node_id) ORDER BY t.node_id"#;

/// One edge for the chains: its ends and `key`, the values that must be the same on the two sides of a join (the road's
/// class, name, lanes, speed, one-way, layer, bridge, tunnel, junction, access, service and lane profile, as text).
pub struct ChainEdge {
    pub id: i64,
    pub source: i64,
    pub target: i64,
    pub key: String,
}

/// `chain_id` of the edges `ids` in their order: the `edge_id` for a chain of one edge, else a hash of the ids.
pub fn chain_id(ids: &[i64]) -> i64 {
    if ids.len() == 1 {
        return ids[0];
    }
    let h = ids.iter().fold(0x9e37_79b9_7f4a_7c15u64, |h, &i| {
        crate::hash::mix(h ^ crate::hash::mix(i as u64))
    });
    (h >> 1) as i64
}

/// The chains of `edges`, each one its edges in driving order. Every edge is in one chain. Edge `a` joins edge `b` (`b`
/// comes after `a`) only at a node where:
/// - the mode's edges go only to `a`'s source and `b`'s target (one in and one out, or two in and two out on a two-way road:
///   then each of the two other ends has one edge in and one edge out, so a road beside it between the same nodes is not
///   taken for its other direction);
/// - `a` and `b` have the same `key`;
/// - the node is not in `stop_nodes` (a traffic light or a stop sign);
/// - the turn `a` → `b` is in `turns` (the mode's legal turns), and not in `keep_turns` (a turn with a rule);
/// - neither edge is in `keep_edges` (an edge of a via-way rule).
///
/// A closed ring of joins is cut before its smallest `edge_id`. A chain that ends at its own start is cut in two.
pub fn chains(
    edges: &[ChainEdge],
    turns: &HashSet<(i64, i64)>,
    stop_nodes: &HashSet<i64>,
    keep_turns: &HashSet<(i64, i64)>,
    keep_edges: &HashSet<i64>,
) -> Vec<Vec<i64>> {
    let mut ins: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut outs: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        if e.source != e.target {
            outs.entry(e.source).or_default().push(i);
            ins.entry(e.target).or_default().push(i);
        }
    }
    let mut next: HashMap<i64, i64> = HashMap::new();
    let mut prev: HashMap<i64, i64> = HashMap::new();
    for (n, i_n) in &ins {
        let Some(o_n) = outs.get(n) else { continue };
        if stop_nodes.contains(n) || i_n.len() != o_n.len() || i_n.len() > 2 {
            continue;
        }
        let ends: HashSet<i64> = i_n
            .iter()
            .map(|&i| edges[i].source)
            .chain(o_n.iter().map(|&o| edges[o].target))
            .collect();
        // one in and one out, or two in and two out: always the same two other ends
        if ends.len() != 2 || ends.contains(n) {
            continue;
        }
        // two in and two out: a two-way road, each other end once in and once out. Two edges out to one node (and two in from
        // two nodes) is not one road: it is a road and another road beside it, between the same two nodes
        if i_n.len() == 2 {
            let mut from: Vec<i64> = i_n.iter().map(|&i| edges[i].source).collect();
            let mut to: Vec<i64> = o_n.iter().map(|&o| edges[o].target).collect();
            from.sort_unstable();
            to.sort_unstable();
            if from != to || from[0] == from[1] {
                continue;
            }
        }
        for &i in i_n {
            let a = &edges[i];
            // the edge out that does not go back to where `a` comes from
            let Some(&o) = o_n.iter().find(|&&o| edges[o].target != a.source) else {
                continue;
            };
            let b = &edges[o];
            if a.key == b.key
                && turns.contains(&(a.id, b.id))
                && !keep_turns.contains(&(a.id, b.id))
                && !keep_edges.contains(&a.id)
                && !keep_edges.contains(&b.id)
            {
                next.insert(a.id, b.id);
                prev.insert(b.id, a.id);
            }
        }
    }
    let mut done: HashSet<i64> = HashSet::new();
    let mut out: Vec<Vec<i64>> = Vec::new();
    let walk = |start: i64, done: &mut HashSet<i64>, out: &mut Vec<Vec<i64>>| {
        let mut c = vec![start];
        done.insert(start);
        let mut e = start;
        while let Some(&b) = next.get(&e) {
            if !done.insert(b) {
                break;
            }
            c.push(b);
            e = b;
        }
        out.push(c);
    };
    for e in edges {
        if !prev.contains_key(&e.id) && !done.contains(&e.id) {
            walk(e.id, &mut done, &mut out);
        }
    }
    // the rings: each edge left has an edge before it
    let mut left: Vec<i64> = edges
        .iter()
        .map(|e| e.id)
        .filter(|i| !done.contains(i))
        .collect();
    left.sort_unstable();
    for e in left {
        if !done.contains(&e) {
            walk(e, &mut done, &mut out);
        }
    }
    // a chain that ends where it starts (a loop): two chains, because an export edge needs two different ends
    let ends: HashMap<i64, (i64, i64)> =
        edges.iter().map(|e| (e.id, (e.source, e.target))).collect();
    let mut split = Vec::with_capacity(out.len());
    for c in out {
        if c.len() > 1 && ends[&c[0]].0 == ends[&c[c.len() - 1]].1 {
            let (a, b) = c.split_at(c.len() / 2);
            split.push(a.to_vec());
            split.push(b.to_vec());
        } else {
            split.push(c);
        }
    }
    split
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: i64, s: i64, t: i64, k: &str) -> ChainEdge {
        ChainEdge {
            id,
            source: s,
            target: t,
            key: k.into(),
        }
    }

    fn all_turns(es: &[ChainEdge]) -> HashSet<(i64, i64)> {
        es.iter()
            .flat_map(|a| {
                es.iter()
                    .filter(move |b| a.target == b.source)
                    .map(move |b| (a.id, b.id))
            })
            .collect()
    }

    #[test]
    fn parallel_road_is_not_the_other_direction() {
        // Monaco, node 6482583607 = 2: 10 (from 1) and 20 (from 3) come in; 11 and 21 both go out to 3 (a road and a service
        // road beside it). Not a two-way road: 10 must not join 11, so the turn 20 -> 11 is not inside a chain.
        let es = vec![e(10, 1, 2, "k"), e(20, 3, 2, "s"), e(11, 2, 3, "k"), e(21, 2, 3, "s")];
        let c = chains(&es, &all_turns(&es), &HashSet::new(), &HashSet::new(), &HashSet::new());
        assert!(c.iter().all(|c| c.len() == 1), "{c:?}");
    }

    #[test]
    fn two_way_street_with_two_crossings() {
        // A=1 -> x=2 -> y=3 -> B=4, both ways; a side road at B
        let es = vec![
            e(10, 1, 2, "k"),
            e(11, 2, 3, "k"),
            e(12, 3, 4, "k"),
            e(20, 2, 1, "k"),
            e(21, 3, 2, "k"),
            e(22, 4, 3, "k"),
            e(30, 4, 5, "k"),
        ];
        let c = chains(
            &es,
            &all_turns(&es),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(c.contains(&vec![10, 11, 12]));
        assert!(c.contains(&vec![22, 21, 20]));
        assert!(c.contains(&vec![30]));
        assert_eq!(c.iter().map(Vec::len).sum::<usize>(), es.len());
        assert_eq!(chain_id(&[30]), 30);
    }

    #[test]
    fn stops_at_a_change_a_signal_and_a_rule() {
        let es = vec![
            e(1, 1, 2, "a"),
            e(2, 2, 3, "b"),
            e(3, 3, 4, "b"),
            e(4, 4, 5, "b"),
            e(5, 5, 6, "b"),
        ];
        let t = all_turns(&es);
        let c = chains(
            &es,
            &t,
            &HashSet::from([4]),
            &HashSet::from([(4, 5)]),
            &HashSet::new(),
        );
        assert_eq!(c, vec![vec![1], vec![2, 3], vec![4], vec![5]]);
        // no legal turn: no join
        let c = chains(
            &es,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::from([1]),
        );
        assert_eq!(c.len(), 5);
    }

    #[test]
    fn ring_is_cut_at_its_smallest_edge() {
        let es = vec![e(7, 1, 2, "k"), e(5, 2, 3, "k"), e(9, 3, 1, "k")];
        let c = chains(
            &es,
            &all_turns(&es),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(c, vec![vec![5], vec![9, 7]]);
    }
}
