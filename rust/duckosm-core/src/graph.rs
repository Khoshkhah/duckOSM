//! The turn restrictions and the edge graph of duckOSM (`processors/restrictions.py`,
//! `processors/edge_graph.py`). See docs/edge_graph.md.
//!
//! - `turn_restrictions` (driving): the `type=restriction` relations with a via node, and the synthetic
//!   rules of the OSM fixes, mapped to (from edge, to edge).
//! - `turn_path_restrictions` (driving): the relations with a via way, as edge paths.
//! - `edge_graph`: the pairs of edges that meet (`e1.target = e2.source`), with the private edges of some
//!   access labels, without the turn-backs on bridges and without the banned turns.
//! - `turn_permission`: the rules that the graph cannot say (exceptions, vehicle classes, conditions).

use crate::filter::{Mode, ModeWay};
use crate::fixes::TurnFix;
use crate::model::Edge;
use crate::util::{FxHashMap, FxHashSet};

/// The element type of a relation member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberType {
    Node,
    Way,
    Relation,
}

/// A `type=restriction` relation as read: its tags and its members (id, type, role).
#[derive(Clone, Debug, Default)]
pub struct Relation {
    pub id: i64,
    pub tags: Vec<(String, String)>,
    pub members: Vec<(i64, MemberType, String)>,
}

/// A row of `edge_graph`. `via_edge` is `to_edge`.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphRow {
    pub from_edge: i64,
    pub to_edge: i64,
    /// `cost_s` of the from edge.
    pub cost: Option<f32>,
    /// Who may take the turn: `car`, `walk`, `bike`, or the access label of a private edge.
    pub uses: &'static str,
}

/// A row of `turn_restrictions`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TurnRestriction {
    pub restriction_id: i64,
    pub restriction_type: String,
    pub except_vehicles: Option<String>,
    pub applies_to: Option<String>,
    pub condition: Option<String>,
    pub via_node: i64,
    pub from_edge_id: i64,
    pub to_edge_id: i64,
}

/// A row of `turn_permission`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TurnPermission {
    pub from_edge: i64,
    pub to_edge: i64,
    pub allowed: bool,
    pub vehicles: Option<String>,
    pub except_vehicles: Option<String>,
    pub condition: Option<String>,
    pub restriction_id: i64,
}

/// A row of `turn_path_restrictions`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TurnPath {
    pub restriction_id: i64,
    pub restriction_type: String,
    pub except_vehicles: Option<String>,
    pub applies_to: Option<String>,
    pub condition: Option<String>,
    pub from_edge: i64,
    pub via_edges: Vec<i64>,
    pub to_edge: i64,
}

/// The tables of this module. A table that the options switch off is `None`.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub edge_graph: Option<Vec<GraphRow>>,
    pub turn_permission: Option<Vec<TurnPermission>>,
    pub turn_restrictions: Option<Vec<TurnRestriction>>,
    pub turn_path_restrictions: Option<Vec<TurnPath>>,
}

/// The access labels of the private edges that `edge_graph` has rows for (`RESTRICTED_USES`).
const RESTRICTED_USES: [&str; 4] = ["bus", "private", "pedestrian", "delivery"];
/// On a bridge, no move turns more than this (`BRIDGE_MAX_TURN_DEG`).
const BRIDGE_MAX_TURN_DEG: f64 = 90.0;

/// One rule of a relation (`restrictions_raw`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rule {
    id: i64,
    rtype: String,
    except: Option<String>,
    applies_to: Option<String>,
    condition: Option<String>,
}

/// A rule with its from way, via node and to way (`restrictions_pivoted`).
type NodeRule = (Rule, Option<i64>, Option<i64>, Option<i64>);

/// DuckDB `trim`: only the space character.
fn trim(s: &str) -> &str {
    s.trim_matches(' ')
}

/// SQL `x LIKE '<prefix>_%'`: the prefix and at least one more character.
fn like_prefix(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix) && s.chars().count() > prefix.chars().count()
}

/// The rules of a relation: one for each key `restriction`, `restriction:conditional` and
/// `restriction:<vehicle>` (not a key with two colons).
fn rules(rel: &Relation) -> Vec<Rule> {
    let get = |k: &str| rel.tags.iter().find(|t| t.0 == k).map(|t| t.1.clone());
    if get("type").as_deref() != Some("restriction") {
        return Vec::new();
    }
    let except = get("except");
    let mut out: Vec<Rule> = rel
        .tags
        .iter()
        .filter(|(k, _)| {
            k == "restriction" || (k.starts_with("restriction:") && k.matches(':').count() < 2)
        })
        .map(|(k, v)| {
            let cond = k == "restriction:conditional";
            let condition = cond.then(|| {
                // trim(regexp_replace(trim(split_part(v, '@', 2)), '^\(|\)$', '', 'g'))
                let c = trim(v.split('@').nth(1).unwrap_or(""));
                let c = c.strip_prefix('(').unwrap_or(c);
                trim(c.strip_suffix(')').unwrap_or(c)).to_string()
            });
            Rule {
                id: rel.id,
                rtype: if cond {
                    trim(v.split('@').next().unwrap_or("")).to_string()
                } else {
                    trim(v).to_string()
                },
                except: except.clone(),
                applies_to: (k != "restriction" && !cond)
                    .then(|| k["restriction:".len()..].to_string()),
                condition,
            }
        })
        .collect();
    // GROUP BY in restrictions_pivoted: the same rule two times is one rule
    out.sort();
    out.dedup();
    out
}

/// The largest member id of a role and a type (`MAX(CASE WHEN role = ... THEN ref_id END)`).
fn member(rel: &Relation, role: &str, t: MemberType) -> Option<i64> {
    rel.members
        .iter()
        .filter(|m| m.2 == role && m.1 == t)
        .map(|m| m.0)
        .max()
}

/// The ways of the mode that have the two nodes as neighbours (`way_nodes` with `abs(seq) = 1`).
struct WayEnds {
    pairs: FxHashMap<(i64, i64), Vec<i64>>,
}

impl WayEnds {
    fn new(mw: &[ModeWay]) -> WayEnds {
        let mut pairs: FxHashMap<(i64, i64), Vec<i64>> = FxHashMap::default();
        for w in mw {
            for p in w.refs.windows(2) {
                let k = (p[0].min(p[1]), p[0].max(p[1]));
                let v = pairs.entry(k).or_default();
                if !v.contains(&w.id) {
                    v.push(w.id);
                }
            }
        }
        WayEnds { pairs }
    }

    fn ways(&self, a: i64, b: i64) -> &[i64] {
        self.pairs
            .get(&(a.min(b), a.max(b)))
            .map_or(&[], |v| v.as_slice())
    }

    /// The ways at the source end of an edge (its first two nodes). A half of a split edge can have
    /// one node; then `refs[2]` is NULL in the SQL, and the end has no way.
    fn source(&self, e: &Edge) -> &[i64] {
        match e.refs[..] {
            [a, b, ..] => self.ways(a, b),
            _ => &[],
        }
    }

    /// The ways at the target end of an edge (its last two nodes).
    fn target(&self, e: &Edge) -> &[i64] {
        match e.refs[..] {
            [.., a, b] => self.ways(a, b),
            _ => &[],
        }
    }
}

/// `turn_restrictions` and `turn_path_restrictions` of the driving mode (`RestrictionProcessor`).
pub fn restrictions(
    relations: &[Relation],
    synthetic: &[TurnFix],
    mw: &[ModeWay],
    edges: &[Edge],
) -> (Vec<TurnRestriction>, Vec<TurnPath>) {
    let ends = WayEnds::new(mw);
    let mut by_target: FxHashMap<i64, Vec<usize>> = FxHashMap::default();
    let mut by_source: FxHashMap<i64, Vec<usize>> = FxHashMap::default();
    for (i, e) in edges.iter().enumerate() {
        by_target.entry(e.target).or_default().push(i);
        by_source.entry(e.source).or_default().push(i);
    }
    // the rules with a via node: (rule, from way, via node, to way)
    let mut node_rules: Vec<NodeRule> = Vec::new();
    for rel in relations {
        let (fw, vn, tw) = (
            member(rel, "from", MemberType::Way),
            member(rel, "via", MemberType::Node),
            member(rel, "to", MemberType::Way),
        );
        for r in rules(rel) {
            node_rules.push((r, fw, vn, tw));
        }
    }
    for s in synthetic {
        let rule = Rule {
            id: s.id,
            rtype: s.restriction.clone(),
            except: None,
            applies_to: None,
            condition: None,
        };
        node_rules.push((rule, Some(s.from_way), Some(s.via_node), Some(s.to_way)));
    }
    let mut out = Vec::new();
    for (r, fw, vn, tw) in &node_rules {
        let (Some(fw), Some(vn), Some(tw)) = (fw, vn, tw) else {
            continue;
        };
        let from = by_target.get(vn).map_or(&[][..], |v| v.as_slice());
        let to = by_source.get(vn).map_or(&[][..], |v| v.as_slice());
        for &fi in from
            .iter()
            .filter(|&&i| ends.target(&edges[i]).contains(fw))
        {
            for &ti in to.iter().filter(|&&i| ends.source(&edges[i]).contains(tw)) {
                out.push(TurnRestriction {
                    restriction_id: r.id,
                    restriction_type: r.rtype.clone(),
                    except_vehicles: r.except.clone(),
                    applies_to: r.applies_to.clone(),
                    condition: r.condition.clone(),
                    via_node: *vn,
                    from_edge_id: edges[fi].edge_id,
                    to_edge_id: edges[ti].edge_id,
                });
            }
        }
    }
    out.sort();
    out.dedup();
    (out, via_way_paths(relations, &ends, edges))
}

/// `_map_via_ways`: a rule with a via way bans or mandates a path. Each edge path from an edge of the
/// from way, along edges of the via ways (at most 6, no node two times), onto an edge of a to way.
fn via_way_paths(relations: &[Relation], ends: &WayEnds, edges: &[Edge]) -> Vec<TurnPath> {
    struct ViaRule {
        rule: Rule,
        from_way: Option<i64>,
        via: Vec<i64>,
        to: Vec<i64>,
    }
    let mut via_rules = Vec::new();
    for rel in relations {
        let of = |role: &str| -> Vec<i64> {
            rel.members
                .iter()
                .filter(|m| m.2 == role && m.1 == MemberType::Way)
                .map(|m| m.0)
                .collect()
        };
        let via = of("via");
        if via.is_empty() {
            continue;
        }
        // refs[list_position(ref_roles, 'from')]: the first member with the role from, of any type
        let from_way = rel.members.iter().find(|m| m.2 == "from").map(|m| m.0);
        for rule in rules(rel) {
            via_rules.push(ViaRule {
                rule,
                from_way,
                via: via.clone(),
                to: of("to"),
            });
        }
    }
    if via_rules.is_empty() {
        return Vec::new();
    }
    let ways: FxHashSet<i64> = via_rules
        .iter()
        .flat_map(|r| {
            r.from_way
                .into_iter()
                .chain(r.via.iter().copied())
                .chain(r.to.iter().copied())
        })
        .collect();
    // the way at each end of an edge that touches one of these ways; with two ways at one end, a way of
    // the rules wins (duckOSM keeps an arbitrary one of the SQL rows)
    let pick = |ws: &[i64]| ws.iter().find(|w| ways.contains(w)).or(ws.first()).copied();
    // edge index -> (source, target, way at the source end, way at the target end)
    let mut info: FxHashMap<usize, (i64, i64, Option<i64>, Option<i64>)> = FxHashMap::default();
    let mut out_of: FxHashMap<i64, Vec<usize>> = FxHashMap::default();
    for (i, e) in edges.iter().enumerate() {
        let (s, t) = (ends.source(e), ends.target(e));
        if s.iter().chain(t).any(|w| ways.contains(w)) {
            info.insert(i, (e.source, e.target, pick(s), pick(t)));
            out_of.entry(e.source).or_default().push(i);
        }
    }
    let mut out = Vec::new();
    for r in &via_rules {
        let vset: FxHashSet<i64> = r.via.iter().copied().collect();
        let tset: FxHashSet<i64> = r.to.iter().copied().collect();
        for (&fe, &(_, ftgt, _, ftw)) in &info {
            if ftw.is_none() || ftw != r.from_way {
                continue;
            }
            let mut todo: Vec<(i64, Vec<usize>, Vec<i64>)> = vec![(ftgt, Vec::new(), vec![ftgt])];
            while let Some((node, path, seen)) = todo.pop() {
                for &e in out_of.get(&node).map_or(&[][..], |v| v.as_slice()) {
                    let (_, etgt, esw, etw) = info[&e];
                    if !path.is_empty() && esw.is_some_and(|w| tset.contains(&w)) {
                        out.push(TurnPath {
                            restriction_id: r.rule.id,
                            restriction_type: r.rule.rtype.clone(),
                            except_vehicles: r.rule.except.clone(),
                            applies_to: r.rule.applies_to.clone(),
                            condition: r.rule.condition.clone(),
                            from_edge: edges[fe].edge_id,
                            via_edges: path.iter().map(|&p| edges[p].edge_id).collect(),
                            to_edge: edges[e].edge_id,
                        });
                    }
                    let on_via = esw.is_some_and(|w| vset.contains(&w))
                        || etw.is_some_and(|w| vset.contains(&w));
                    if on_via && !seen.contains(&etgt) && path.len() < 6 {
                        let mut p2 = path.clone();
                        p2.push(e);
                        let mut s2 = seen.clone();
                        s2.push(etgt);
                        todo.push((etgt, p2, s2));
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// The heading at the end of an edge and at its start, in degrees (`edge_graph._remove_bridge_turnbacks`):
/// `degrees(atan2(dy, dx * cos(radians(y))))`, with DuckDB's `degrees` and `radians`.
fn headings(e: &Edge) -> (f64, f64) {
    let deg = |x: f64| x * (180.0 / std::f64::consts::PI);
    let rad = |x: f64| x * (std::f64::consts::PI / 180.0);
    let g = &e.geometry;
    let (p, q) = (g[g.len() - 2], g[g.len() - 1]);
    let end = deg((q.1 - p.1).atan2((q.0 - p.0) * rad(q.1).cos()));
    let (a, b) = (g[0], g[1]);
    let start = deg((b.1 - a.1).atan2((b.0 - a.0) * rad(a.1).cos()));
    (end, start)
}

fn on_bridge(e: &Edge) -> bool {
    !matches!(e.attrs.bridge.as_deref().unwrap_or("no"), "no" | "")
}

/// `edge_graph` and `turn_permission` of one mode (`EdgeGraphBuilder`).
pub fn edge_graph(
    mode: Mode,
    edges: &[Edge],
    private_edges: &[Edge],
    restrictions: Option<&[TurnRestriction]>,
) -> (Vec<GraphRow>, Vec<TurnPermission>) {
    let own = match mode {
        Mode::Driving => "car",
        Mode::Walking => "walk",
        Mode::Cycling => "bike",
    };
    // the edges, and the private edges with a restricted access label: (edge, label)
    let label = |e: &Edge| {
        RESTRICTED_USES
            .iter()
            .copied()
            .find(|u| e.access.as_deref() == Some(*u))
    };
    let all: Vec<(&Edge, Option<&'static str>)> = edges
        .iter()
        .map(|e| (e, None))
        .chain(
            private_edges
                .iter()
                .filter_map(|e| label(e).map(|l| (e, Some(l)))),
        )
        .collect();
    let mut by_source: FxHashMap<i64, Vec<usize>> = FxHashMap::default();
    for (i, (e, _)) in all.iter().enumerate() {
        by_source.entry(e.source).or_default().push(i);
    }
    let heads: Vec<(f64, f64)> = all.iter().map(|(e, _)| headings(e)).collect();
    let mut rows = Vec::new();
    for (i, &(e1, l1)) in all.iter().enumerate() {
        for &j in by_source.get(&e1.target).map_or(&[][..], |v| v.as_slice()) {
            let (e2, l2) = all[j];
            if e1.edge_id == e2.edge_id {
                continue;
            }
            let uses = match (l1, l2) {
                (None, None) => {
                    // no turn back on a bridge (only between two edges of the mode)
                    if on_bridge(e1) && on_bridge(e2) {
                        let turn = ((heads[j].1 - heads[i].0 + 540.0) % 360.0 - 180.0).abs();
                        if turn > BRIDGE_MAX_TURN_DEG {
                            continue;
                        }
                    }
                    own
                }
                (Some(a), Some(b)) if a != b => continue, // no user takes two different restricted edges
                (Some(a), _) | (None, Some(a)) => a,
            };
            rows.push(GraphRow {
                from_edge: e1.edge_id,
                to_edge: e2.edge_id,
                cost: e1.cost_s,
                uses,
            });
        }
    }
    let mut permission = Vec::new();
    if let Some(trs) = restrictions {
        let mut by_from: FxHashMap<i64, Vec<&TurnRestriction>> = FxHashMap::default();
        for tr in trs {
            by_from.entry(tr.from_edge_id).or_default().push(tr);
        }
        let bans = |tr: &TurnRestriction, to: i64| {
            (like_prefix(&tr.restriction_type, "no") && tr.to_edge_id == to)
                || (like_prefix(&tr.restriction_type, "only") && tr.to_edge_id != to)
        };
        // what edge_graph will not say, recorded before it changes
        for g in &rows {
            for tr in by_from.get(&g.from_edge).map_or(&[][..], |v| v.as_slice()) {
                if !bans(tr, g.to_edge) {
                    continue;
                }
                if tr.applies_to.is_none() && tr.condition.is_none() && tr.except_vehicles.is_some()
                {
                    permission.push(TurnPermission {
                        from_edge: g.from_edge,
                        to_edge: g.to_edge,
                        allowed: true,
                        vehicles: tr.except_vehicles.clone(),
                        except_vehicles: None,
                        condition: None,
                        restriction_id: tr.restriction_id,
                    });
                } else if tr.applies_to.is_some() || tr.condition.is_some() {
                    permission.push(TurnPermission {
                        from_edge: g.from_edge,
                        to_edge: g.to_edge,
                        allowed: false,
                        vehicles: tr.applies_to.clone(),
                        except_vehicles: tr.except_vehicles.clone(),
                        condition: tr.condition.clone(),
                        restriction_id: tr.restriction_id,
                    });
                }
            }
        }
        // a rule for all vehicles, always, changes the graph
        rows.retain(|g| {
            !by_from.get(&g.from_edge).is_some_and(|trs| {
                trs.iter().any(|tr| {
                    tr.applies_to.is_none() && tr.condition.is_none() && bans(tr, g.to_edge)
                })
            })
        });
    }
    rows.sort_by_key(|g| (g.from_edge, g.to_edge));
    permission.sort();
    (rows, permission)
}

impl Graph {
    /// `ComponentFilter`: remove the rows that use an edge that the filter dropped.
    pub fn prune(&mut self, dropped: &FxHashSet<i64>) {
        let gone = |a: i64, b: i64| dropped.contains(&a) || dropped.contains(&b);
        if let Some(v) = &mut self.edge_graph {
            v.retain(|r| !gone(r.from_edge, r.to_edge));
        }
        if let Some(v) = &mut self.turn_restrictions {
            v.retain(|r| !gone(r.from_edge_id, r.to_edge_id));
        }
        if let Some(v) = &mut self.turn_permission {
            v.retain(|r| !gone(r.from_edge, r.to_edge));
        }
        if let Some(v) = &mut self.turn_path_restrictions {
            v.retain(|r| !gone(r.from_edge, r.to_edge));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tags: &[(&str, &str)]) -> Relation {
        Relation {
            id: 7,
            tags: tags
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            members: vec![],
        }
    }

    #[test]
    fn rules_of_a_relation() {
        let r = rules(&rel(&[
            ("type", "restriction"),
            ("restriction", " no_left_turn "),
            ("restriction:hgv", "no_right_turn"),
            ("restriction:conditional", "no_u_turn @ (Mo-Fr 07:00-09:00)"),
            ("restriction:hgv:conditional", "no_entry @ (wet)"),
            ("except", "bus"),
        ]));
        assert_eq!(r.len(), 3);
        let c = r.iter().find(|x| x.condition.is_some()).unwrap();
        assert_eq!(
            (c.rtype.as_str(), c.condition.as_deref()),
            ("no_u_turn", Some("Mo-Fr 07:00-09:00"))
        );
        let h = r.iter().find(|x| x.applies_to.is_some()).unwrap();
        assert_eq!(
            (h.rtype.as_str(), h.applies_to.as_deref()),
            ("no_right_turn", Some("hgv"))
        );
        assert!(r.iter().all(|x| x.except.as_deref() == Some("bus")));
        assert!(rules(&rel(&[
            ("type", "multipolygon"),
            ("restriction", "no_left_turn")
        ]))
        .is_empty());
        assert!(
            like_prefix("no_x", "no") && !like_prefix("no", "no") && like_prefix("only_x", "only")
        );
    }
}
