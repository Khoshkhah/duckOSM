//! Which lane goes to which, without netconvert: SimReady's lane rules (docs/design/gmns_lane_rules.md).
//!
//! Ported from SimReady `rust/duckosm-sim/src/connectors.rs` (lane pairs: rules 2, 3, 4, 5, 20, 21, 24 of its page
//! `docs/3-modules/connectors.md`) and `turn_rules.rs` (rule 1, the turn kind; rule 3, the fork). The rule numbers are
//! SimReady's, so the two can be compared line by line. The input is GMNS's own: the links (= the unmerged edges, not the
//! chains), their lanes left to right, and the legal movements of `movement` (GMNS decides which turns are legal; these rules
//! decide only which lanes serve them). The output is the same as netconvert's: for each movement, its lane pairs.

use crate::gmns::Ctx;
use crate::gmns_sumo::{apply_rows, runs, Check};
use crate::shapely::{self as shp, P};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The angles of the turn kinds (SimReady's criteria `turn_angle_slight_deg`, `turn_angle_turn_deg`, `turn_angle_u_deg`).
const SLIGHT: f64 = 15.0;
const TURN: f64 = 45.0;
const U: f64 = 150.0;

/// Rule 1 of turn-rules: the kind of a turn of `angle` degrees (positive: to the left).
fn turn_kind(angle: f64) -> &'static str {
    let a = angle.abs();
    if a < SLIGHT {
        "straight"
    } else if a < TURN {
        if angle > 0.0 { "slight_left" } else { "slight_right" }
    } else if a < U {
        if angle > 0.0 { "left" } else { "right" }
    } else {
        "u"
    }
}

struct Lane {
    num: i64,
    bus: bool,
    bike: bool,
    turn: Option<String>,
}

/// The lanes of the network, and the lanes of each link left to right.
struct Net {
    lanes: Vec<Lane>,
    of: HashMap<i64, Vec<usize>>,
}

impl Net {
    /// The lanes of `link` that are (`bike`) or are not bike lanes, left to right.
    fn end_lanes(&self, link: i64, bike: bool) -> Vec<usize> {
        self.of.get(&link).map(|v| v.iter().copied().filter(|&i| self.lanes[i].bike == bike).collect()).unwrap_or_default()
    }
}

type Pair = (usize, usize);

/// The family of a turn kind (rule 2).
fn family_of(turn: &str) -> &'static str {
    match turn {
        "left" => "left",
        "right" => "right",
        "u" => "u",
        _ => "ahead",
    }
}

/// The family that a word of `turn:lanes` serves (rule 2); none for `none` and an unknown word.
fn arrow_family(w: &str) -> Option<&'static str> {
    Some(match w {
        "left" | "sharp_left" => "left",
        "right" | "sharp_right" => "right",
        "reverse" => "u",
        "through" | "slight_left" | "slight_right" | "merge_to_left" | "merge_to_right" | "slide_left" | "slide_right" => "ahead",
        _ => return None,
    })
}

fn words(turn: &Option<String>) -> Vec<String> {
    turn.as_deref().unwrap_or("").split([';', '|']).map(|w| w.trim().to_string()).collect()
}

fn families(net: &Net, li: usize) -> BTreeSet<&'static str> {
    words(&net.lanes[li].turn).iter().filter_map(|w| arrow_family(w)).collect()
}

fn has_arrows(net: &Net, lanes: &[usize]) -> bool {
    lanes.iter().any(|&l| !families(net, l).is_empty())
}

/// 'l' or 'r' when the words of a lane name a side and no word is `through`; else None (rule 4).
fn arrow_side(net: &Net, li: usize) -> Option<char> {
    let w = words(&net.lanes[li].turn);
    if w.iter().any(|x| x == "through") {
        return None;
    }
    let left = w.iter().any(|x| ["left", "sharp_left", "slight_left", "merge_to_left", "slide_left"].contains(&x.as_str()));
    let right = w.iter().any(|x| ["right", "sharp_right", "slight_right", "merge_to_right", "slide_right"].contains(&x.as_str()));
    if left && !right { Some('l') } else if right && !left { Some('r') } else { None }
}

/// Does the lane at index `i` of `n` driving lanes (left to right) serve the family by its position? (rule 2)
fn positional(fam: &str, i: usize, n: usize, rhd: bool) -> bool {
    match fam {
        "ahead" => true,
        "left" => i == 0,
        "right" => i == n - 1,
        _ => {
            if rhd { i == 0 } else { i == n - 1 }
        }
    }
}

/// The pairs of the entering lanes `s` with the leaving lanes `l` in order from the right (`right`) or the left (rule 3).
fn pair_up(s: &[usize], l: &[usize], right: bool, fan: bool) -> Vec<Pair> {
    if s.is_empty() || l.is_empty() {
        return vec![];
    }
    let (s, l): (Vec<usize>, Vec<usize>) = if right { (s.iter().rev().copied().collect(), l.iter().rev().copied().collect()) } else { (s.to_vec(), l.to_vec()) };
    let m = if fan { s.len().max(l.len()) } else { s.len() };
    (0..m).map(|i| (s[i.min(s.len() - 1)], l[i.min(l.len() - 1)])).collect()
}

/// The side from which the lanes of a family are paired (rule 3): true for `right`.
fn align_right(fam: &str, rhd: bool) -> bool {
    match fam {
        "left" => false,
        "right" => true,
        _ => !rhd,
    }
}

/// The lanes that are not `bus` lanes, or all lanes when there is no other.
fn not_bus(net: &Net, lanes: &[usize]) -> Vec<usize> {
    let o: Vec<usize> = lanes.iter().copied().filter(|&i| !net.lanes[i].bus).collect();
    if o.is_empty() { lanes.to_vec() } else { o }
}

/// Rules 2 and 3 (and 21: a U-turn with arrows but no `reverse` word takes the lane next to the median).
fn plain_pairs(net: &Net, turn: &str, e: &[usize], l: &[usize], rhd: bool) -> Vec<Pair> {
    let fam = family_of(turn);
    let side = match turn {
        "slight_left" => "left",
        "slight_right" => "right",
        _ => "",
    };
    if e.is_empty() || l.is_empty() {
        return vec![];
    }
    let arrows = has_arrows(net, e);
    let eo = not_bus(net, e);
    let lo = not_bus(net, l);
    let mut s = vec![];
    for (i, &x) in eo.iter().enumerate() {
        let f = if arrows { families(net, x) } else { BTreeSet::new() };
        if if !f.is_empty() { f.contains(fam) || f.contains(side) } else { positional(fam, i, eo.len(), rhd) } {
            s.push(x);
        }
    }
    let target: Vec<usize> = if fam == "u" { vec![if rhd { lo[0] } else { lo[lo.len() - 1] }] } else { lo.clone() };
    if fam == "u" && arrows && s.is_empty() {
        s = vec![if rhd { eo[0] } else { eo[eo.len() - 1] }];
    }
    let mut pairs = pair_up(&s, &target, align_right(fam, rhd), fam == "ahead");
    let has_other = e.iter().any(|&i| !net.lanes[i].bus);
    let eb: Vec<usize> = if has_other { e.iter().copied().filter(|&i| net.lanes[i].bus).collect() } else { vec![] };
    if !eb.is_empty() && (fam == "ahead" || turn == if rhd { "right" } else { "left" }) {
        let lb: Vec<usize> = l.iter().copied().filter(|&i| net.lanes[i].bus).collect();
        let tb = if lb.is_empty() { vec![if rhd { lo[lo.len() - 1] } else { lo[0] }] } else { lb };
        pairs.extend(pair_up(&eb, &tb, align_right(fam, rhd), fam == "ahead"));
    }
    let mut seen = HashSet::new();
    pairs.into_iter().filter(|p| seen.insert(*p)).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RingRole {
    Ring,
    Entry,
    Exit,
}

/// Rule 5: the lane pairs of a ring movement, an entry or an exit. Turn arrows on the entering lanes win.
fn ring_pairs(net: &Net, role: RingRole, turn: &str, e: &[usize], l: &[usize], rhd: bool) -> Vec<Pair> {
    if has_arrows(net, e) {
        return plain_pairs(net, turn, e, l, rhd);
    }
    if role == RingRole::Ring {
        return pair_up(e, l, !rhd, true);
    }
    let a: Vec<usize> = if rhd { e.iter().rev().copied().collect() } else { e.to_vec() }; // outermost lane first
    let b: Vec<usize> = if rhd { l.iter().rev().copied().collect() } else { l.to_vec() };
    if a.is_empty() || b.is_empty() {
        return vec![];
    }
    if role == RingRole::Entry {
        return (0..a.len()).map(|i| (a[i], b[i.min(b.len() - 1)])).collect();
    }
    (0..b.len()).map(|j| (a[j.min(a.len() - 1)], b[j])).collect()
}

/// One legal turn at a node, as the rules read it.
struct Turn {
    mid: String,
    node: i64,
    ib: i64,
    ob: i64,
    kind: &'static str,
    angle: f64,
    fork: bool,
}

/// Rule 4: the lane pairs of the turns of one fork group (a diverge: one link in; a merge: one link out).
fn fan_pairs(net: &Net, group: &[&Turn], diverge: bool, rhd: bool, rank_of: &dyn Fn(i64) -> i64) -> HashMap<String, Vec<Pair>> {
    let mut out = HashMap::new();
    if diverge {
        let eo = not_bus(net, &net.end_lanes(group[0].ib, false));
        if eo.is_empty() {
            return out;
        }
        // the arms from the left to the right
        let mut arms: Vec<&Turn> = group.to_vec();
        arms.sort_by(|a, b| b.angle.total_cmp(&a.angle).then(a.ob.cmp(&b.ob)));
        let lanes: Vec<Vec<usize>> = arms.iter().map(|a| not_bus(net, &net.end_lanes(a.ob, false))).collect();
        let (n_e, m) = (eo.len(), arms.len());
        let sums: Vec<usize> = (0..m).map(|i| lanes[..i].iter().map(|x| x.len()).sum()).collect();
        let total: usize = lanes.iter().map(|x| x.len()).sum();
        let arrows = has_arrows(net, &eo);
        for (i, a) in arms.iter().enumerate() {
            let ni = lanes[i].len();
            let (lo, hi) = if i == 0 {
                (0, ni.min(n_e))
            } else if i == m - 1 {
                (if total <= n_e { sums[i] } else { n_e.saturating_sub(ni) }, n_e)
            } else {
                let lo = sums[i].min(n_e - 1);
                (lo, (lo + 1).max((lo + ni).min(n_e)))
            };
            let mut win: Vec<usize> = if lo < hi { eo[lo..hi.min(n_e)].to_vec() } else { vec![] };
            if arrows {
                let mine: Option<char> = if i == 0 { Some('l') } else if i == m - 1 { Some('r') } else { None };
                let inside = win.clone();
                win = eo.iter().copied().filter(|&x| (inside.contains(&x) && (arrow_side(net, x).is_none() || arrow_side(net, x) == mine)) || (mine.is_some() && arrow_side(net, x) == mine)).collect();
                if win.is_empty() && mine.is_some() {
                    // rule 24: an arm that the arrows left with no lane takes the nearest lane that does not name the other side
                    let cand: Vec<usize> = eo.iter().copied().filter(|&x| arrow_side(net, x).is_none() || arrow_side(net, x) == mine).collect();
                    win = if mine == Some('r') { cand.last().copied().into_iter().collect() } else { cand.first().copied().into_iter().collect() };
                }
            }
            out.insert(a.mid.clone(), pair_up(&win, &lanes[i], !rhd, true));
        }
        return out;
    }
    if group.len() != 2 {
        return out; // a merge of more than two arms: each turn by rules 2 and 3
    }
    let l = not_bus(net, &net.end_lanes(group[0].ob, false));
    if l.is_empty() {
        return out;
    }
    // the arms from the left to the right, as seen along the leaving link
    let mut arms: Vec<&Turn> = group.to_vec();
    // the largest angle first: an arm that turns left to join comes from the left (SimReady: -angle, ascending)
    arms.sort_by(|a, b| b.angle.total_cmp(&a.angle).then(a.ib.cmp(&b.ib)));
    let es: Vec<Vec<usize>> = arms.iter().map(|a| not_bus(net, &net.end_lanes(a.ib, false))).collect();
    let (rl, rr) = (rank_of(arms[0].ib), rank_of(arms[1].ib));
    let main = if rl < rr { 0 } else if rr < rl { 1 } else if rhd { 0 } else { 1 };
    let (n, nl, nr) = (l.len(), es[0].len(), es[1].len());
    let (wl, wr): (Vec<usize>, Vec<usize>) = if n >= nl + nr {
        (l[..nl].to_vec(), l[nl..].to_vec())
    } else if main == 0 {
        (if nl >= n { l.clone() } else { l[..nl].to_vec() }, if nl < n { l[nl..].to_vec() } else { vec![l[n - 1]] })
    } else {
        (if nr < n { l[..n - nr].to_vec() } else { vec![l[0]] }, if nr >= n { l.clone() } else { l[n - nr..].to_vec() })
    };
    let toward_right = main == 1;
    out.insert(arms[0].mid.clone(), pair_up(&es[0], &wl, toward_right, true));
    out.insert(arms[1].mid.clone(), pair_up(&es[1], &wr, toward_right, true));
    out
}

/// The road class rank of a facility type (a lower rank is a more important road).
fn rank(hw: &str) -> i64 {
    match hw.trim_end_matches("_link") {
        "motorway" => 1,
        "trunk" => 2,
        "primary" => 3,
        "secondary" => 4,
        "tertiary" => 5,
        "unclassified" => 6,
        "residential" => 7,
        "living_street" => 8,
        "service" => 9,
        _ => 10,
    }
}

/// A line in local metres around its first point (x east, y north), and back.
fn local(pts: &[P], o: P) -> Vec<P> {
    let kx = o.1.to_radians().cos() * 111_320.0;
    pts.iter().map(|p| ((p.0 - o.0) * kx, (p.1 - o.1) * 111_320.0)).collect()
}

fn unit(v: P) -> P {
    let n = (v.0 * v.0 + v.1 * v.1).sqrt();
    if n == 0.0 { (0.0, 0.0) } else { (v.0 / n, v.1 / n) }
}

/// The direction of a line at its end (`end`) or at its start, over its last (first) 15 m.
fn tangent(l: &[P], end: bool) -> P {
    let n = shp::length(l);
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let (a, b) = if end { (shp::interpolate(l, (n - 15.0).max(0.0)), shp::interpolate(l, n)) } else { (shp::interpolate(l, 0.0), shp::interpolate(l, 15.0f64.min(n))) };
    unit((b.0 - a.0, b.1 - a.1))
}

/// The angle in degrees from direction `a` to direction `b` (positive: to the left).
fn signed_angle(a: P, b: P) -> f64 {
    (a.0 * b.1 - a.1 * b.0).atan2(a.0 * b.0 + a.1 * b.1).to_degrees()
}

/// The lane pairs (lane number in, lane number out) of each movement (`mvmt_id`) of the schema `sch`. `ring`: the links on a
/// roundabout. Movements with no pair are not in the result.
pub(crate) fn pairs(c: &Ctx, sch: &str, mode: &str, drive_side: &str, ring: &HashSet<i64>) -> Result<HashMap<String, Vec<(i64, i64)>>> {
    let rhd = drive_side == "right";
    let e = |x: duckdb::Error| x.to_string();
    // the links: ends, class and line in local metres (each line around its own first point: only directions are used)
    let mut links: HashMap<i64, (i64, i64, String, Vec<P>)> = HashMap::new();
    let mut st = c.con.prepare(&format!("SELECT link_id, from_node_id, to_node_id, coalesce(facility_type, ''), geometry FROM {sch}.link")).map_err(e)?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?))).map_err(e)? {
        let (id, f, t, hw, wkt) = r.map_err(e)?;
        let pts = wkt.map(|w| shp::wkt_points(&w)).unwrap_or_default();
        let l = if pts.is_empty() { vec![] } else { local(&pts, pts[0]) };
        links.insert(id, (f, t, hw, l));
    }
    // the lanes, left to right (lane_num: a bike lane on the left is -1)
    let mut net = Net { lanes: Vec::new(), of: HashMap::new() };
    let mut st = c.con.prepare(&format!("SELECT link_id, lane_num, coalesce(allowed_uses, ''), turn FROM {sch}.lane ORDER BY link_id, lane_num")).map_err(e)?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?))).map_err(e)? {
        let (link, num, uses, turn) = r.map_err(e)?;
        let has = |u: &str| uses.split(',').any(|x| x.trim() == u);
        // a bike lane is its own lane only in the driving network; in the cycling network each lane is a bike lane
        let bike = mode == "driving" && has("bike") && !has("auto") && !has("bus");
        let bus = has("bus") && !has("auto");
        net.of.entry(link).or_default().push(net.lanes.len());
        net.lanes.push(Lane { num, bus, bike, turn: turn.filter(|t| !t.is_empty()) });
    }
    // the legal turns, with their kind
    let mut turns: Vec<Turn> = Vec::new();
    let mut st = c.con.prepare(&format!("SELECT mvmt_id, node_id, ib_link_id, ob_link_id, type FROM {sch}.movement ORDER BY rowid")).map_err(e)?;
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<String>>(4)?))).map_err(e)? {
        let (mid, node, ib, ob, typ) = r.map_err(e)?;
        let (Some(a), Some(b)) = (links.get(&ib), links.get(&ob)) else { continue };
        let angle = signed_angle(tangent(&a.3, true), tangent(&b.3, false));
        // the turn type of GMNS wins (left and right from 30 degrees): the lanes follow the movement's own type; the angle
        // decides only between straight and slight for a through, merge or diverge movement
        let kind = match typ.as_deref() {
            Some("uturn") => "u",
            Some("left") => "left",
            Some("right") => "right",
            _ => match turn_kind(angle) {
                "left" => "slight_left",
                "right" => "slight_right",
                "u" => "straight",
                k => k,
            },
        };
        turns.push(Turn { mid, node, ib, ob, kind, angle, fork: false });
    }
    // rule 3 of turn-rules, the fork, node by node: the twins (the other direction of a two-way road) do not count
    let twin = |a: i64, b: i64| -> bool { links.get(&a).zip(links.get(&b)).is_some_and(|(x, y)| x.0 == y.1 && x.1 == y.0) };
    let mut at: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (i, t) in turns.iter().enumerate() {
        at.entry(t.node).or_default().push(i);
    }
    let mut groups: Vec<(Vec<usize>, bool)> = Vec::new();
    for idx in at.values() {
        let ins: BTreeSet<i64> = idx.iter().map(|&i| turns[i].ib).collect();
        let outs: BTreeSet<i64> = idx.iter().map(|&i| turns[i].ob).collect();
        let real_out: Vec<i64> = outs.iter().copied().filter(|&o| !ins.iter().any(|&i| twin(i, o))).collect();
        let real_in: Vec<i64> = ins.iter().copied().filter(|&i| !outs.iter().any(|&o| twin(i, o))).collect();
        let diverge = ins.len() == 1 && real_out.len() >= 2;
        let merge = outs.len() == 1 && real_in.len() >= 2;
        if !(diverge || merge) {
            continue;
        }
        let g: Vec<usize> = idx.iter().copied().filter(|&i| if diverge { real_out.contains(&turns[i].ob) } else { real_in.contains(&turns[i].ib) }).collect();
        for &i in &g {
            turns[i].fork = true;
        }
        groups.push((g, diverge));
    }
    let rank_of = |l: i64| links.get(&l).map(|x| rank(&x.2)).unwrap_or(10);
    let mut out: HashMap<String, Vec<(i64, i64)>> = HashMap::new();
    fn put(out: &mut HashMap<String, Vec<(i64, i64)>>, net: &Net, mid: &str, ps: Vec<Pair>) {
        if !ps.is_empty() {
            let v = out.entry(mid.to_string()).or_default();
            for (a, b) in ps {
                let p = (net.lanes[a].num, net.lanes[b].num);
                if !v.contains(&p) {
                    v.push(p);
                }
            }
        }
    }
    for (g, diverge) in &groups {
        let ts: Vec<&Turn> = g.iter().map(|&i| &turns[i]).collect();
        for (mid, ps) in fan_pairs(&net, &ts, *diverge, rhd, &rank_of) {
            put(&mut out, &net, &mid, ps);
        }
    }
    for t in &turns {
        let (e, l) = (net.end_lanes(t.ib, false), net.end_lanes(t.ob, false));
        let role = match (ring.contains(&t.ib), ring.contains(&t.ob)) {
            (true, true) => Some(RingRole::Ring),
            (false, true) => Some(RingRole::Entry),
            (true, false) => Some(RingRole::Exit),
            _ => None,
        };
        // a fork turn that the fan rule left with no pair falls back on rules 2 and 3, as a plain turn
        if !t.fork || !out.contains_key(&t.mid) {
            let ps = match role {
                Some(r) => ring_pairs(&net, r, t.kind, &e, &l, rhd),
                None => plain_pairs(&net, t.kind, &e, &l, rhd),
            };
            put(&mut out, &net, &t.mid, ps);
        }
        // rule 20: the bike lanes go on to the bike lanes of the leaving link, from the kerb side; not on a roundabout
        let (eb, lb) = (net.end_lanes(t.ib, true), net.end_lanes(t.ob, true));
        if !eb.is_empty() && !lb.is_empty() && role != Some(RingRole::Ring) && t.kind != "u" {
            put(&mut out, &net, &t.mid, pair_up(&eb, &lb, rhd, false));
        }
    }
    Ok(out)
}

/// The radius of each node's junction, in metres: half of the widest road at the node (both directions), plus 1 m, at least
/// 2 m; 0 where the road only continues (two other ends or fewer). A first value: SimReady's Level 4 makes the real shape.
fn junction_radius(c: &Ctx, sch: &str) -> Result<HashMap<i64, f64>> {
    let sql = format!(
        "WITH w AS (SELECT l.link_id, l.from_node_id AS a, l.to_node_id AS b, sum(coalesce(n.width, 3.25)) AS w
                    FROM {sch}.link l JOIN {sch}.lane n USING (link_id) GROUP BY ALL),
              road AS (SELECT least(a, b) AS a, greatest(a, b) AS b, sum(w) AS w FROM w GROUP BY ALL),
              ends AS (SELECT a AS node, b AS other, w FROM road UNION ALL SELECT b, a, w FROM road)
         SELECT node, count(DISTINCT other), max(w) FROM ends GROUP BY node"
    );
    let mut st = c.con.prepare(&sql).map_err(|e| format!("{e}\n{sql}"))?;
    let mut out = HashMap::new();
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?))).map_err(|e| e.to_string())? {
        let (n, k, w) = r.map_err(|e| e.to_string())?;
        out.insert(n, if k <= 2 { 0.0 } else { (0.5 * w + 1.0).max(2.0) });
    }
    Ok(out)
}

/// A cubic Hermite curve from `p0` (direction `t0`) to `p1` (direction `t1`), the tangents as long as the chord, 12 pieces.
fn hermite(p0: P, t0: P, p1: P, t1: P) -> Vec<P> {
    let d = ((p1.0 - p0.0).powi(2) + (p1.1 - p0.1).powi(2)).sqrt();
    if d < 0.05 {
        return vec![p0, p1];
    }
    let (m0, m1) = ((t0.0 * d, t0.1 * d), (t1.0 * d, t1.1 * d));
    (0..=12)
        .map(|k| {
            let s = k as f64 / 12.0;
            let (h00, h10, h01, h11) = (2.0 * s.powi(3) - 3.0 * s * s + 1.0, s.powi(3) - 2.0 * s * s + s, -2.0 * s.powi(3) + 3.0 * s * s, s.powi(3) - s * s);
            (h00 * p0.0 + h10 * m0.0 + h01 * p1.0 + h11 * m1.0, h00 * p0.1 + h10 * m0.1 + h01 * p1.1 + h11 * m1.1)
        })
        .collect()
}

fn wkt(pts: &[P]) -> String {
    format!("LINESTRING ({})", pts.iter().map(|p| format!("{:.8} {:.8}", p.0, p.1)).collect::<Vec<_>>().join(", "))
}

/// `lanes_from_rules`: the lane pairs of SimReady's rules, written as `lanes_from_sumo` writes netconvert's: each movement one
/// row for each run of lanes side by side (a bike lane its own row `<mvmt_id>-b<lane>`), `lane_check`, and with `geometry`
/// `lane.geom_cut` and `lane_connector`. `source_ring`: the source db's roundabout edges of the mode.
pub(crate) fn lanes_from_rules(c: &Ctx, sch: &str, mode: &str, geometry: bool, drive_side: &str, ring: &HashSet<i64>) -> Result<()> {
    let e = |x: duckdb::Error| x.to_string();
    let got = pairs(c, sch, mode, drive_side, ring)?;
    c.exec(&format!("CREATE OR REPLACE TABLE {sch}.lane_check(kind VARCHAR, id VARCHAR, detail VARCHAR)"))?;
    let mut check: Vec<Check> = Vec::new();
    let mut use_of: BTreeMap<(i64, i64), String> = BTreeMap::new();
    let mut lid_of: HashMap<(i64, i64), String> = HashMap::new();
    let mut width_of: HashMap<String, f64> = HashMap::new();
    let mut st = c.con.prepare(&format!("SELECT link_id, lane_num, coalesce(allowed_uses, ''), lane_id, coalesce(width, 3.25)::DOUBLE FROM {sch}.lane")).map_err(e)?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, f64>(4)?))).map_err(e)? {
        let (lk, n, u, lid, w) = r.map_err(e)?;
        use_of.insert((lk, n), u);
        lid_of.insert((lk, n), lid.clone());
        width_of.insert(lid, w);
    }
    let mut st = c.con.prepare(&format!("SELECT mvmt_id, ib_link_id, ob_link_id, type FROM {sch}.movement ORDER BY rowid")).map_err(e)?;
    let mv: Vec<(String, i64, i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
    // the movement rows, as lanes_from_sumo makes them: (old id, new id, lanes, bike row, kept as it was)
    type Row = (String, String, Option<(i64, i64, i64, i64)>, bool, bool);
    let mut rows: Vec<Row> = Vec::new();
    let mut row_ends: Vec<(i64, i64, Option<(i64, i64, i64, i64)>, bool)> = Vec::new();
    let mut lane_pairs: Vec<(String, i64, i64, i64, i64)> = Vec::new(); // (row id, ib, lane in, ob, lane out)
    for (mid, ib, ob, typ) in &mv {
        let ps = got.get(mid).cloned().unwrap_or_default();
        if ps.is_empty() {
            rows.push((mid.clone(), mid.clone(), None, false, true));
            row_ends.push((*ib, *ob, None, false));
            check.push(("no lane connection".into(), mid.clone(), format!("{typ} {ib} -> {ob}: the lane rules give no lane pair")));
            continue;
        }
        let is_bike = |a: i64, b: i64| mode == "driving" && (use_of.get(&(*ib, a)).is_some_and(|u| u == "bike") || use_of.get(&(*ob, b)).is_some_and(|u| u == "bike"));
        let bike: Vec<(i64, i64)> = ps.iter().copied().filter(|p| is_bike(p.0, p.1)).collect();
        let motor: Vec<(i64, i64)> = ps.iter().copied().filter(|p| !bike.contains(p)).collect();
        for (k, run) in runs(&motor).iter().enumerate() {
            let m = if k == 0 { mid.clone() } else { format!("{mid}-{}", k + 1) };
            let lanes = (run[0].0, run[run.len() - 1].0, run[0].1, run[run.len() - 1].1);
            rows.push((mid.clone(), m.clone(), Some(lanes), false, false));
            row_ends.push((*ib, *ob, Some(lanes), false));
            lane_pairs.extend(run.iter().map(|p| (m.clone(), *ib, p.0, *ob, p.1)));
        }
        if motor.is_empty() {
            rows.push((mid.clone(), mid.clone(), None, false, false));
            row_ends.push((*ib, *ob, None, false));
        }
        for &(a, b) in &bike {
            let m = format!("{mid}-b{a}");
            rows.push((mid.clone(), m.clone(), Some((a, a, b, b)), true, false));
            row_ends.push((*ib, *ob, Some((a, a, b, b)), true));
            lane_pairs.push((m, *ib, a, *ob, b));
        }
    }
    apply_rows(c, sch, &rows)?;
    // every lane of a link with movements has a way out and a way in (driving: the car lanes)
    let (mut outs, mut ins): (HashMap<i64, HashSet<i64>>, HashMap<i64, HashSet<i64>>) = (HashMap::new(), HashMap::new());
    for (ib, ob, lanes, _) in &row_ends {
        if let Some((a0, a1, b0, b1)) = lanes {
            outs.entry(*ib).or_default().extend(*a0..=*a1);
            ins.entry(*ob).or_default().extend(*b0..=*b1);
        }
    }
    let has_out: HashSet<i64> = row_ends.iter().map(|r| r.0).collect();
    let has_in: HashSet<i64> = row_ends.iter().map(|r| r.1).collect();
    for ((lk, n), u) in &use_of {
        if mode == "driving" && u == "bike" {
            continue;
        }
        if has_out.contains(lk) && !outs.get(lk).is_some_and(|s| s.contains(n)) {
            check.push(("no way out".into(), lid_of[&(*lk, *n)].clone(), "no movement leaves this lane".into()));
        }
        if has_in.contains(lk) && !ins.get(lk).is_some_and(|s| s.contains(n)) {
            check.push(("no way in".into(), lid_of[&(*lk, *n)].clone(), "no movement enters this lane".into()));
        }
    }
    let has_geom = c.count(&format!("SELECT count(*) FROM duckdb_columns() WHERE schema_name = '{sch}' AND table_name = 'lane' AND column_name = 'geom'"))? > 0;
    if geometry && has_geom {
        // each lane cut at the junction radius of its two ends; the path of each lane pair a Hermite curve between the cuts
        let radius = junction_radius(c, sch)?;
        let mut st = c.con.prepare(&format!(
            "SELECT n.lane_id, l.from_node_id, l.to_node_id, ST_AsText(n.geom) FROM {sch}.lane n JOIN {sch}.link l USING (link_id) WHERE n.geom IS NOT NULL"
        )).map_err(e)?;
        let mut cut: HashMap<String, Vec<P>> = HashMap::new();
        for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))).map_err(e)? {
            let (lid, a, b, w) = r.map_err(e)?;
            let pts = shp::wkt_points(&w);
            if pts.len() < 2 {
                continue;
            }
            let o = pts[0];
            let (kx, ky) = (o.1.to_radians().cos() * 111_320.0, 111_320.0);
            let loc: Vec<P> = pts.iter().map(|p| ((p.0 - o.0) * kx, (p.1 - o.1) * ky)).collect();
            let n = shp::length(&loc);
            let (ra, rb) = (radius.get(&a).copied().unwrap_or(0.0), radius.get(&b).copied().unwrap_or(0.0));
            // a lane shorter than its two cuts keeps its middle 10 %
            let (s0, s1) = if ra + rb < 0.9 * n { (ra, n - rb) } else { (0.45 * n, 0.55 * n) };
            let piece = shp::substring(&loc, s0, s1);
            cut.insert(lid, piece.iter().map(|p| (o.0 + p.0 / kx, o.1 + p.1 / ky)).collect());
        }
        let mut cons: Vec<(String, String, String, String, f64, String)> = Vec::new();
        for (m, ib, a, ob, b) in &lane_pairs {
            let (Some(la), Some(lb)) = (lid_of.get(&(*ib, *a)), lid_of.get(&(*ob, *b))) else { continue };
            let (Some(ca), Some(cb)) = (cut.get(la), cut.get(lb)) else { continue };
            let o = ca[ca.len() - 1];
            let (kx, ky) = (o.1.to_radians().cos() * 111_320.0, 111_320.0);
            let to_l = |p: &P| ((p.0 - o.0) * kx, (p.1 - o.1) * ky);
            let (pa, pb): (Vec<P>, Vec<P>) = (ca.iter().map(to_l).collect(), cb.iter().map(to_l).collect());
            let path = hermite(pa[pa.len() - 1], tangent(&pa, true), pb[0], tangent(&pb, false));
            let ll: Vec<P> = path.iter().map(|p| (o.0 + p.0 / kx, o.1 + p.1 / ky)).collect();
            let w = width_of.get(la).copied().unwrap_or(3.25).min(width_of.get(lb).copied().unwrap_or(3.25));
            cons.push((format!("{la}>{lb}"), m.clone(), la.clone(), lb.clone(), w, wkt(&ll)));
        }
        c.exec(&format!(
            "ALTER TABLE {sch}.lane ADD COLUMN IF NOT EXISTS geom_cut GEOMETRY;
             CREATE OR REPLACE TABLE {sch}.lane_connector(connector_id VARCHAR, mvmt_id VARCHAR, from_lane_id VARCHAR, to_lane_id VARCHAR, width DOUBLE, geom GEOMETRY);
             CREATE OR REPLACE TEMP TABLE _cut(lane_id VARCHAR, wkt VARCHAR);
             CREATE OR REPLACE TEMP TABLE _con(connector_id VARCHAR, mvmt_id VARCHAR, from_lane_id VARCHAR, to_lane_id VARCHAR, width DOUBLE, wkt VARCHAR)"
        ))?;
        {
            let mut app = c.con.appender("_cut").map_err(e)?;
            for (lid, pts) in &cut {
                if pts.len() >= 2 {
                    app.append_row(duckdb::params![lid, wkt(pts)]).map_err(e)?;
                }
            }
            app.flush().map_err(e)?;
            let mut app = c.con.appender("_con").map_err(e)?;
            for (id, m, a, b, w, g) in &cons {
                app.append_row(duckdb::params![id, m, a, b, w, g]).map_err(e)?;
            }
            app.flush().map_err(e)?;
        }
        c.exec(&format!(
            "UPDATE {sch}.lane l SET geom_cut = ST_GeomFromText(c.wkt) FROM _cut c WHERE c.lane_id = l.lane_id;
             INSERT INTO {sch}.lane_connector SELECT connector_id, mvmt_id, from_lane_id, to_lane_id, width, ST_GeomFromText(wkt) FROM _con ORDER BY connector_id;
             DROP TABLE _cut; DROP TABLE _con"
        ))?;
    }
    if !check.is_empty() {
        let mut app = c.con.appender_to_db("lane_check", sch).map_err(e)?;
        for (k, i, d) in &check {
            app.append_row(duckdb::params![k, i, d]).map_err(e)?;
        }
        app.flush().map_err(e)?;
    }
    eprintln!("GMNS[{mode}] lanes from the lane rules: {} movements -> {} rows, {} lane pairs, {} checks", mv.len(), rows.len(), lane_pairs.len(), check.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(lanes: &[(i64, i64, &str, Option<&str>)]) -> Net {
        let mut n = Net { lanes: Vec::new(), of: HashMap::new() };
        for &(link, num, uses, turn) in lanes {
            n.of.entry(link).or_default().push(n.lanes.len());
            n.lanes.push(Lane { num, bus: uses == "bus", bike: uses == "bike", turn: turn.map(String::from) });
        }
        n
    }

    #[test]
    fn kinds() {
        assert_eq!(turn_kind(5.0), "straight");
        assert_eq!(turn_kind(30.0), "slight_left");
        assert_eq!(turn_kind(-90.0), "right");
        assert_eq!(turn_kind(170.0), "u");
    }

    #[test]
    fn positions_without_arrows() {
        // two lanes in (link 1), two out (link 2): straight ahead pairs side by side; a right turn takes the right lane
        let n = net(&[(1, 1, "auto", None), (1, 2, "auto", None), (2, 1, "auto", None), (2, 2, "auto", None)]);
        let (e, l) = (n.end_lanes(1, false), n.end_lanes(2, false));
        let straight: Vec<(i64, i64)> = plain_pairs(&n, "straight", &e, &l, true).iter().map(|p| (n.lanes[p.0].num, n.lanes[p.1].num)).collect();
        assert_eq!(straight.iter().copied().collect::<BTreeSet<_>>(), BTreeSet::from([(1, 1), (2, 2)]));
        let right: Vec<(i64, i64)> = plain_pairs(&n, "right", &e, &l, true).iter().map(|p| (n.lanes[p.0].num, n.lanes[p.1].num)).collect();
        assert_eq!(right, vec![(2, 2)]);
    }

    #[test]
    fn arrows_win() {
        // turn:lanes left|through: only lane 1 turns left, only lane 2 goes ahead
        let n = net(&[(1, 1, "auto", Some("left")), (1, 2, "auto", Some("through")), (2, 1, "auto", None), (3, 1, "auto", None)]);
        let e = n.end_lanes(1, false);
        let left: Vec<i64> = plain_pairs(&n, "left", &e, &n.end_lanes(2, false), true).iter().map(|p| n.lanes[p.0].num).collect();
        let ahead: Vec<i64> = plain_pairs(&n, "straight", &e, &n.end_lanes(3, false), true).iter().map(|p| n.lanes[p.0].num).collect();
        assert_eq!((left, ahead), (vec![1], vec![2]));
    }

    #[test]
    fn hermite_ends() {
        let p = hermite((0.0, 0.0), (1.0, 0.0), (10.0, 10.0), (0.0, 1.0));
        assert_eq!((p[0], p[p.len() - 1]), ((0.0, 0.0), (10.0, 10.0)));
    }
}
