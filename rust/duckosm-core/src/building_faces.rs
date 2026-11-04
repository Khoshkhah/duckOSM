//! The walls of the buildings (`main.building_faces`, docs/osm-editing.md, M4): each segment of the outer ring of a building footprint is a face, and
//! each face has a type. This is not in duckOSM: it is made on request (`Params.building_faces`), from the raw cut and the street edges of the build.
//!
//! - `front`: the wall that faces a street, and any other wall that faces a street at a short distance (a corner building has two). A street counts only when
//!   it can be seen from the wall: the line from the wall to the street crosses no wall of a building (the building itself or another one).
//! - `back`: a wall that points away from the front wall and faces no street.
//! - `party`: a wall that two buildings share (the same two nodes in both rings).
//! - `side`: every other wall. A building with no street near it has no front and no back: all its walls are `side` (or `party`).
//!
//! A face has the key `(building id, node, node)`: it does not change when the ring has more nodes elsewhere. The rows do not change from run to run: the order
//! of the output is the order of the ways and of the ring, and a tie in the street match goes to the lower `edge_id`.

use crate::geom::Pt;
use crate::graph::{MemberType, Relation};
use crate::raw::{RawNode, RawWay};
use crate::util::FxHashMap;
use rstar::{RTree, RTreeObject, AABB};

/// A street farther than this from the middle of a wall is not the street of the wall (meters).
pub const MAX_STREET_M: f64 = 60.0;
/// A wall can be the front wall when the cosine of the angle between its outward normal and the way to the street is at least this (60 degrees).
pub const FRONT_ALIGN: f64 = 0.5;
/// Another wall is a front wall too (a corner) when it faces a street this much (about 50 degrees) and the street is not farther than `CORNER_DIST_M`.
pub const CORNER_ALIGN: f64 = 0.65;
pub const CORNER_DIST_M: f64 = 30.0;
/// A wall is a back wall when its normal and the front wall's normal have at most this cosine (about 114 degrees apart) and it faces no street (cosine at most `BACK_ALIGN`).
pub const BACK_DOT: f64 = -0.4;
pub const BACK_ALIGN: f64 = 0.1;
/// A building with no front (no entrance on a wall, no street that a wall can see) gets the wall that faces the nearest street within this distance from
/// the middle of the wall, with no test of the line of sight (a building behind other buildings). Its walls say `inferred` in `type_reason`.
pub const INFER_STREET_M: f64 = 150.0;
/// The score of a street that has the name of the `addr:street` tag of the building is multiplied by this.
pub const ADDRESS_BOOST: f64 = 1.8;
/// The values of `entrance` that are not a way in for people (a service door, a fire exit).
const NOT_ENTRANCES: [&str; 4] = ["no", "service", "emergency", "exit"];
/// A wall that is shorter than this is not a face (meters). Its place in the ring is kept in `face_index`.
pub const MIN_FACE_M: f64 = 0.1;
/// Meters in one degree of latitude, and of longitude at the equator (a sphere of radius 6,378,137 m).
const M_PER_DEG: f64 = 111_319.490_793;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceType {
    Front,
    Back,
    Side,
    Party,
    /// A wall of an inner ring (a courtyard) of a building that is a multipolygon relation.
    Court,
}

impl FaceType {
    pub fn name(self) -> &'static str {
        match self {
            FaceType::Front => "front",
            FaceType::Back => "back",
            FaceType::Side => "side",
            FaceType::Party => "party",
            FaceType::Court => "court",
        }
    }

    pub fn parse(s: &str) -> Option<FaceType> {
        [
            FaceType::Front,
            FaceType::Back,
            FaceType::Side,
            FaceType::Party,
            FaceType::Court,
        ]
        .into_iter()
        .find(|t| t.name() == s)
    }
}

/// A segment of a street edge, in (lon, lat).
#[derive(Clone, Debug)]
pub struct Street {
    pub a: Pt,
    pub b: Pt,
    pub edge_id: i64,
    pub osm_id: i64,
    pub name: Option<String>,
    /// The `highway` value of the edge: a street of a higher class (a wider street) wins over a smaller one at the same distance.
    pub highway: String,
}

/// The weight of a street class in the score of a street for a wall: a wide street is more of a front than a service road.
fn class_weight(highway: &str) -> f64 {
    match highway.trim_end_matches("_link") {
        "motorway" | "trunk" | "primary" => 1.3,
        "secondary" => 1.25,
        "tertiary" => 1.15,
        "service" => 0.7,
        _ => 1.0,
    }
}

/// A rule of `extra.building_faces`: the type of one wall, in place of the computed type, and the kind of door that the wall has (None: no door).
/// The two nodes can be in either order.
#[derive(Clone, Debug)]
pub struct FaceEdit {
    pub building: i64,
    pub from_node: i64,
    pub to_node: i64,
    pub face_type: FaceType,
    pub door: Option<String>,
}

/// One face (wall) of a building, in the order of the ring (counter-clockwise).
#[derive(Clone, Debug)]
pub struct BuildingFace {
    pub building_id: i64,
    /// `way` or `relation`: the id of a way and the id of a relation are in two number spaces.
    pub building_type: &'static str,
    /// The place of the wall in the ring (from 0). A wall that is too short has a number but no row.
    pub face_index: i32,
    pub from_node: i64,
    pub to_node: i64,
    /// The type after the rules of `extra.building_faces`, and the type that the method computed.
    pub face_type: FaceType,
    pub computed_type: FaceType,
    pub edited: bool,
    /// Why the wall has its type: `entrance` (a door of the wall), `address` (it faces the street of the `addr:street` tag), `street` (it faces a street that it can see, also a second front wall of a corner),
    /// `inferred` (the nearest street, with no line of sight), `inner` (a wall of a courtyard), `opposite` (a back wall), `shared` (a party wall), `other` (a side wall
    /// of a building that has a front) or `none` (the building has no front).
    pub type_reason: &'static str,
    /// The kind of door of the wall (`main`, `service`, ...) from a rule; None when the wall has no door.
    pub door: Option<String>,
    pub length_m: f32,
    /// The compass direction of the outward normal (0 north, 90 east).
    pub azimuth_deg: f32,
    /// The street that the wall faces (None when no street is near it and aligned).
    pub street_osm_id: Option<i64>,
    pub street_edge_id: Option<i64>,
    pub street_name: Option<String>,
    pub distance_m: Option<f32>,
    /// The two ends, in (lon, lat).
    pub geometry: Vec<Pt>,
}

type P = (f64, f64);

struct Proj {
    lon0: f64,
    lat0: f64,
    kx: f64,
}

impl Proj {
    fn new(origin: Pt) -> Proj {
        Proj {
            lon0: origin.0,
            lat0: origin.1,
            kx: M_PER_DEG * origin.1.to_radians().cos(),
        }
    }
    fn pt(&self, p: Pt) -> P {
        ((p.0 - self.lon0) * self.kx, (p.1 - self.lat0) * M_PER_DEG)
    }
}

struct Seg {
    a: P,
    b: P,
    edge_id: i64,
    osm_id: i64,
    name: Option<String>,
    weight: f64,
}

impl RTreeObject for Seg {
    type Envelope = AABB<P>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            (self.a.0.min(self.b.0), self.a.1.min(self.b.1)),
            (self.a.0.max(self.b.0), self.a.1.max(self.b.1)),
        )
    }
}

/// A wall of any building, for the test of the line of sight.
struct WallSeg {
    a: P,
    b: P,
}

impl RTreeObject for WallSeg {
    type Envelope = AABB<P>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            (self.a.0.min(self.b.0), self.a.1.min(self.b.1)),
            (self.a.0.max(self.b.0), self.a.1.max(self.b.1)),
        )
    }
}

/// Do the segments pq and ab cross? (Touching at an end is not a crossing.)
fn crosses(p: P, q: P, a: P, b: P) -> bool {
    let turn = |u: P, v: P, w: P| (v.0 - u.0) * (w.1 - u.1) - (v.1 - u.1) * (w.0 - u.0);
    turn(a, b, p) * turn(a, b, q) < 0.0 && turn(p, q, a) * turn(p, q, b) < 0.0
}

/// The street that a wall faces: the best score of the segments within `MAX_STREET_M` that the wall points at and that it can see.
struct Match {
    q: P,
    edge_id: i64,
    osm_id: i64,
    name: Option<String>,
    dist: f64,
    align: f64,
    score: f64,
    /// The street has the name of the `addr:street` tag of the building.
    by_address: bool,
}

fn closest_on(a: P, b: P, p: P) -> (P, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 <= 1e-12 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
    };
    let q = (a.0 + dx * t, a.1 + dy * t);
    (q, ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt())
}

/// The nearest point to `p` on the street segment of the match (found again in the tree by its edge id and the match point).
fn closest_to_street(tree: &RTree<Seg>, m: &Match, p: P) -> P {
    let env = AABB::from_corners((m.q.0 - 1e-6, m.q.1 - 1e-6), (m.q.0 + 1e-6, m.q.1 + 1e-6));
    tree.locate_in_envelope_intersecting(&env)
        .filter(|s| s.edge_id == m.edge_id)
        .map(|s| closest_on(s.a, s.b, p).0)
        .next()
        .unwrap_or(m.q)
}

struct Wall {
    index: usize,
    from: i64,
    to: i64,
    p1: Pt,
    p2: Pt,
    normal: P,
    azimuth: f64,
    length: f64,
    street: Option<Match>,
    party: bool,
    /// The wall has an entrance node at one of its ends (an entrance in the middle of a straight wall splits the wall in two).
    entrance: bool,
}

/// A ring of a building: the outer ring of a way, or an outer or inner ring of a multipolygon relation.
struct Ring {
    id: i64,
    relation: bool,
    inner: bool,
    ids: Vec<i64>,
    pts: Vec<Pt>,
    addr: Option<String>,
}

/// The closed rings of the member ways of a role: a closed way is a ring, and open ways are chained by their end nodes (a way is turned round when it
/// has to be). A chain that does not close is dropped.
fn assemble(parts: Vec<Vec<i64>>) -> Vec<Vec<i64>> {
    let (mut rings, mut open): (Vec<_>, Vec<_>) =
        parts.into_iter().partition(|w| w.len() >= 4 && w.first() == w.last());
    open.retain(|w| w.len() >= 2);
    while let Some(mut cur) = open.pop() {
        while cur.first() != cur.last() {
            let end = *cur.last().unwrap();
            let Some(k) = open.iter().position(|o| o[0] == end || *o.last().unwrap() == end) else {
                break;
            };
            let mut next = open.remove(k);
            if next[0] != end {
                next.reverse();
            }
            cur.extend(next.into_iter().skip(1));
        }
        if cur.len() >= 4 && cur.first() == cur.last() {
            rings.push(cur);
        }
    }
    rings
}

/// The faces of every building way in `ways`, with the street segments `streets` and the rules `edits`. `ways` and `nodes` are sorted by id, as
/// `Raw` has them. A way is a building when it has `building` with a value other than `no`, and a closed ring of three nodes or more whose nodes
/// are all in `nodes`.
pub fn building_faces(
    ways: &[RawWay],
    nodes: &[RawNode],
    relations: &[Relation],
    streets: &[Street],
    edits: &[FaceEdit],
) -> Vec<BuildingFace> {
    let coords = |refs: &[i64]| -> Option<Vec<Pt>> {
        refs.iter()
            .map(|id| nodes.binary_search_by_key(id, |n| n.id).ok().map(|i| (nodes[i].lon, nodes[i].lat)))
            .collect()
    };
    let addr_of = |tags: &[(String, String)]| {
        tags.iter()
            .find(|t| t.0 == "addr:street" && !t.1.trim().is_empty())
            .map(|t| t.1.trim().to_lowercase())
    };
    // 1. the rings, with their coordinates
    let mut rings: Vec<Ring> = Vec::new();
    for w in ways {
        if !w.tags.iter().any(|t| t.0 == "building" && t.1 != "no")
            || w.refs.len() < 4
            || w.refs.first() != w.refs.last()
        {
            continue;
        }
        let pts: Option<Vec<Pt>> = w
            .refs
            .iter()
            .map(|id| {
                nodes
                    .binary_search_by_key(id, |n| n.id)
                    .ok()
                    .map(|i| (nodes[i].lon, nodes[i].lat))
            })
            .collect();
        if let Some(pts) = pts {
            let addr = addr_of(&w.tags);
            rings.push(Ring { id: w.id, relation: false, inner: false, ids: w.refs.clone(), pts, addr });
        }
    }
    // the buildings that are multipolygon relations: the outer rings are walls like those of a way, and the inner rings (courtyards) have `court` walls
    for r in relations {
        let tag = |k: &str| r.tags.iter().find(|t| t.0 == k).map(|t| t.1.as_str());
        if tag("type") != Some("multipolygon") || !tag("building").is_some_and(|b| b != "no") {
            continue;
        }
        let addr = addr_of(&r.tags);
        for (role, inner) in [("outer", false), ("inner", true)] {
            let parts: Vec<Vec<i64>> = r
                .members
                .iter()
                .filter(|m| m.1 == MemberType::Way && (m.2 == role || (!inner && m.2.is_empty())))
                .filter_map(|m| ways.binary_search_by_key(&m.0, |w| w.id).ok().map(|i| ways[i].refs.clone()))
                .collect();
            for ids in assemble(parts) {
                if let Some(pts) = coords(&ids) {
                    rings.push(Ring { id: r.id, relation: true, inner, ids, pts, addr: addr.clone() });
                }
            }
        }
    }
    if rings.is_empty() {
        return Vec::new();
    }
    // 2. one local scale for the whole area: the middle of the buildings
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for p in rings.iter().flat_map(|r| r.pts.iter()) {
        lo = (lo.0.min(p.0), lo.1.min(p.1));
        hi = (hi.0.max(p.0), hi.1.max(p.1));
    }
    let proj = Proj::new(((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0));
    // 3. the streets, each segment once (an edge has one copy for each direction)
    let mut seen: FxHashMap<(i64, [u64; 4]), ()> = FxHashMap::default();
    let mut segs = Vec::new();
    for s in streets {
        let (a, b) = if s.a <= s.b { (s.a, s.b) } else { (s.b, s.a) };
        if seen
            .insert(
                (
                    s.osm_id,
                    [a.0.to_bits(), a.1.to_bits(), b.0.to_bits(), b.1.to_bits()],
                ),
                (),
            )
            .is_some()
        {
            continue;
        }
        segs.push(Seg {
            a: proj.pt(s.a),
            b: proj.pt(s.b),
            edge_id: s.edge_id,
            osm_id: s.osm_id,
            name: s.name.clone(),
            weight: class_weight(&s.highway),
        });
    }
    let tree = RTree::bulk_load(segs);
    // 4. a wall that two buildings have (the same two nodes) is a party wall
    let mut shared: FxHashMap<(i64, i64), u32> = FxHashMap::default();
    for r in &rings {
        for pair in r.ids.windows(2) {
            *shared
                .entry((pair[0].min(pair[1]), pair[0].max(pair[1])))
                .or_default() += 1;
        }
    }
    let rules: FxHashMap<(i64, i64, i64), (FaceType, Option<&str>)> = edits
        .iter()
        .map(|e| {
            (
                (
                    e.building,
                    e.from_node.min(e.to_node),
                    e.from_node.max(e.to_node),
                ),
                (e.face_type, e.door.as_deref()),
            )
        })
        .collect();

    // the entrances: the nodes with an `entrance` tag that is a way in for people
    let entrances: FxHashMap<i64, ()> = nodes
        .iter()
        .filter(|n| n.tags.iter().any(|t| t.0 == "entrance" && !NOT_ENTRANCES.contains(&t.1.as_str())))
        .map(|n| (n.id, ()))
        .collect();

    // 5. each ring counter-clockwise, so that the outward normal of a wall is (dy, -dx); and the tree of all walls, for the line of sight
    let mut oriented = Vec::new();
    let mut wall_segs = Vec::new();
    for Ring { id, relation, inner, mut ids, mut pts, addr } in rings {
        let mut local: Vec<P> = pts.iter().map(|&p| proj.pt(p)).collect();
        let n = local.len() - 1;
        let area: f64 = (0..n)
            .map(|i| local[i].0 * local[i + 1].1 - local[i + 1].0 * local[i].1)
            .sum();
        if area == 0.0 {
            continue; // no area: a line
        }
        // an outer ring counter-clockwise (the normal of a wall points out of the building); an inner ring clockwise (it points into the courtyard)
        if (area < 0.0) != inner {
            local.reverse();
            ids.reverse();
            pts.reverse();
        }
        wall_segs.extend(
            local
                .windows(2)
                .filter(|w| w[0] != w[1])
                .map(|w| WallSeg { a: w[0], b: w[1] }),
        );
        oriented.push((id, relation, inner, ids, pts, local, addr));
    }
    let wall_tree = RTree::bulk_load(wall_segs);
    // from a point on the wall to a point on the street: is there a wall in the way?
    let visible = |from: P, to: P| {
        let env = AABB::from_corners(
            (from.0.min(to.0), from.1.min(to.1)),
            (from.0.max(to.0), from.1.max(to.1)),
        );
        !wall_tree
            .locate_in_envelope_intersecting(&env)
            .any(|w| crosses(from, to, w.a, w.b))
    };

    let mut out = Vec::new();
    let mut next_index: FxHashMap<(i64, bool), i32> = FxHashMap::default();       // the place of a wall in the building goes on from the outer ring to the inner rings
    for (id, relation, inner, ids, pts, local, addr) in oriented {
        let n = local.len() - 1;
        let offset = *next_index.entry((id, relation)).or_default();
        next_index.insert((id, relation), offset + n as i32);
        let building_type = if relation { "relation" } else { "way" };
        if inner {
            for i in 0..n {
                let (v1, v2) = (local[i], local[i + 1]);
                let length = ((v2.0 - v1.0).powi(2) + (v2.1 - v1.1).powi(2)).sqrt();
                if length < MIN_FACE_M {
                    continue;
                }
                let (a, b) = (ids[i], ids[i + 1]);
                let normal = ((v2.1 - v1.1) / length, -(v2.0 - v1.0) / length);
                let (ruled, door) = match rules.get(&(id, a.min(b), a.max(b))) {
                    Some(&(t, d)) => (Some(t), d),
                    None => (None, None),
                };
                out.push(BuildingFace {
                    building_id: id,
                    building_type,
                    face_index: offset + i as i32,
                    from_node: a,
                    to_node: b,
                    face_type: ruled.unwrap_or(FaceType::Court),
                    computed_type: FaceType::Court,
                    edited: ruled.is_some_and(|r| r != FaceType::Court),
                    type_reason: "inner",
                    door: door.map(str::to_string),
                    length_m: length as f32,
                    azimuth_deg: (90.0 - normal.1.atan2(normal.0).to_degrees()).rem_euclid(360.0) as f32,
                    street_osm_id: None,
                    street_edge_id: None,
                    street_name: None,
                    distance_m: None,
                    geometry: vec![pts[i], pts[i + 1]],
                });
            }
            continue;
        }
        let mut walls: Vec<Wall> = Vec::new();
        for i in 0..n {
            let (v1, v2) = (local[i], local[i + 1]);
            let (dx, dy) = (v2.0 - v1.0, v2.1 - v1.1);
            let length = (dx * dx + dy * dy).sqrt();
            if length < MIN_FACE_M {
                continue;
            }
            let normal = (dy / length, -dx / length);
            let mid = ((v1.0 + v2.0) / 2.0, (v1.1 + v2.1) / 2.0);
            let env = AABB::from_corners(
                (mid.0 - MAX_STREET_M, mid.1 - MAX_STREET_M),
                (mid.0 + MAX_STREET_M, mid.1 + MAX_STREET_M),
            );
            let mut candidates: Vec<Match> = Vec::new();
            for s in tree.locate_in_envelope(&env) {
                let (q, dist) = closest_on(s.a, s.b, mid);
                if dist > MAX_STREET_M || dist < 1e-4 {
                    continue;
                }
                let align = normal.0 * (q.0 - mid.0) / dist + normal.1 * (q.1 - mid.1) / dist;
                if align <= 0.0 {
                    continue;
                }
                let by_address = addr.as_deref().is_some_and(|a| {
                    s.name.as_deref().is_some_and(|n| n.trim().to_lowercase() == a)
                });
                let score = align / (dist + 1.0).sqrt()
                    * (1.0 + (length + 1.0).ln())
                    * s.weight
                    * if by_address { ADDRESS_BOOST } else { 1.0 };
                candidates.push(Match {
                    q,
                    edge_id: s.edge_id,
                    osm_id: s.osm_id,
                    name: s.name.clone(),
                    dist,
                    align,
                    score,
                    by_address,
                });
            }
            // the best score that the wall can see; a tie goes to the lower edge id, so that the result does not depend on the order of the tree.
            // A street is seen when a line from one of three points of the wall (at 20, 50 and 80 percent) to the nearest point of the street crosses no wall:
            // a small building in the way of the middle point does not hide the street from the rest of the wall.
            candidates.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.edge_id.cmp(&b.edge_id)));
            let samples: Vec<P> = [0.5, 0.2, 0.8]
                .iter()
                .map(|t| {
                    (
                        v1.0 + dx * t + normal.0 * 0.05,
                        v1.1 + dy * t + normal.1 * 0.05,
                    )
                })
                .collect();
            let best = candidates.into_iter().find(|m| {
                samples.iter().any(|&p| {
                    let seg_q = if p == samples[0] { m.q } else { closest_to_street(&tree, m, p) };
                    visible(p, seg_q)
                })
            });
            let (a, b) = (ids[i], ids[i + 1]);
            walls.push(Wall {
                index: i,
                from: a,
                to: b,
                p1: pts[i],
                p2: pts[i + 1],
                normal,
                azimuth: (90.0 - normal.1.atan2(normal.0).to_degrees()).rem_euclid(360.0),
                length,
                street: best,
                party: shared.get(&(a.min(b), a.max(b))).is_some_and(|&c| c >= 2),
                entrance: entrances.contains_key(&a) || entrances.contains_key(&b),
            });
        }
        // the front wall: the best score among the walls that face a street and are not party walls (the first one in the ring on a tie)
        let mut front: Option<usize> = None;
        for (k, w) in walls.iter().enumerate() {
            let Some(s) = w
                .street
                .as_ref()
                .filter(|s| !w.party && s.align >= FRONT_ALIGN)
            else {
                continue;
            };
            if front.map_or(true, |f| {
                s.score > walls[f].street.as_ref().map_or(0.0, |x| x.score)
            }) {
                front = Some(k);
            }
        }
        let mut front_reason = "street";
        if let Some(f) = front {
            if walls[f].street.as_ref().is_some_and(|s| s.by_address) {
                front_reason = "address";
            }
        }
        // no street that a wall can see: the first wall with an entrance is the front wall
        if front.is_none() {
            front = walls.iter().position(|w| w.entrance && !w.party);
            front_reason = "entrance";
        }
        // still none (a building behind other buildings): the wall that faces the nearest street within `INFER_STREET_M`, with no test of the line of sight
        if front.is_none() {
            let mut best: Option<(usize, Match)> = None;
            for (k, w) in walls.iter().enumerate().filter(|(_, w)| !w.party) {
                let mid = (
                    (proj.pt(w.p1).0 + proj.pt(w.p2).0) / 2.0,
                    (proj.pt(w.p1).1 + proj.pt(w.p2).1) / 2.0,
                );
                let env = AABB::from_corners(
                    (mid.0 - INFER_STREET_M, mid.1 - INFER_STREET_M),
                    (mid.0 + INFER_STREET_M, mid.1 + INFER_STREET_M),
                );
                for s in tree.locate_in_envelope(&env) {
                    let (q, dist) = closest_on(s.a, s.b, mid);
                    if dist > INFER_STREET_M || dist < 1e-4 {
                        continue;
                    }
                    let align = w.normal.0 * (q.0 - mid.0) / dist + w.normal.1 * (q.1 - mid.1) / dist;
                    if align < FRONT_ALIGN {
                        continue;
                    }
                    let score = align / (dist + 1.0).sqrt() * (1.0 + (w.length + 1.0).ln()) * s.weight;
                    if best.as_ref().map_or(true, |(_, b)| {
                        score > b.score || (score == b.score && s.edge_id < b.edge_id)
                    }) {
                        best = Some((
                            k,
                            Match {
                                q,
                                edge_id: s.edge_id,
                                osm_id: s.osm_id,
                                name: s.name.clone(),
                                dist,
                                align,
                                score,
                                by_address: false,
                            },
                        ));
                    }
                }
            }
            if let Some((k, m)) = best {
                walls[k].street = Some(m);
                front = Some(k);
                front_reason = "inferred";
            }
        }
        let front_normal = front.map(|f| walls[f].normal);
        for (k, w) in walls.iter().enumerate() {
            let align = w.street.as_ref().map_or(0.0, |s| s.align);
            let dist = w.street.as_ref().map_or(f64::INFINITY, |s| s.dist);
            let (computed, reason) = if w.party {
                (FaceType::Party, "shared")
            } else if front == Some(k) {
                (FaceType::Front, front_reason)
            } else if w.entrance {
                (FaceType::Front, "entrance")
            } else if front.is_some() && align >= CORNER_ALIGN && dist <= CORNER_DIST_M {
                (FaceType::Front, "street")
            } else if front_normal.is_some_and(|f| w.normal.0 * f.0 + w.normal.1 * f.1 <= BACK_DOT)
                && align <= BACK_ALIGN
            {
                (FaceType::Back, "opposite")
            } else {
                (FaceType::Side, if front.is_some() { "other" } else { "none" })
            };
            let (ruled, door) = match rules.get(&(id, w.from.min(w.to), w.from.max(w.to))) {
                Some(&(t, d)) => (Some(t), d),
                None => (None, None),
            };
            // a street is named for a wall that faces it; the other walls have none
            let street = w
                .street
                .as_ref()
                .filter(|_| computed != FaceType::Back && computed != FaceType::Party);
            out.push(BuildingFace {
                building_id: id,
                building_type,
                face_index: offset + w.index as i32,
                from_node: w.from,
                to_node: w.to,
                face_type: ruled.unwrap_or(computed),
                computed_type: computed,
                edited: ruled.is_some_and(|r| r != computed),
                type_reason: reason,
                door: door.map(str::to_string),
                length_m: w.length as f32,
                azimuth_deg: w.azimuth as f32,
                street_osm_id: street.map(|s| s.osm_id),
                street_edge_id: street.map(|s| s.edge_id),
                street_name: street.and_then(|s| s.name.clone()),
                distance_m: street.map(|s| s.dist as f32),
                geometry: vec![w.p1, w.p2],
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: f64 = 1.0 / M_PER_DEG; // one meter in degrees (at the equator, where the tests are)

    /// the buildings of the tests are ways: no relations
    fn building_faces(ways: &[RawWay], nodes: &[RawNode], streets: &[Street], edits: &[FaceEdit]) -> Vec<BuildingFace> {
        super::building_faces(ways, nodes, &[], streets, edits)
    }

    fn node(id: i64, x: f64, y: f64) -> RawNode {
        RawNode {
            id,
            lon: x * M,
            lat: y * M,
            tags: vec![],
        }
    }
    fn building(id: i64, refs: &[i64]) -> RawWay {
        RawWay {
            id,
            tags: vec![("building".into(), "yes".into())],
            refs: refs.to_vec(),
        }
    }
    fn street(osm_id: i64, a: (f64, f64), b: (f64, f64)) -> Street {
        Street {
            a: (a.0 * M, a.1 * M),
            b: (b.0 * M, b.1 * M),
            edge_id: osm_id * 10,
            osm_id,
            name: Some(format!("street {osm_id}")),
            highway: "residential".into(),
        }
    }
    /// A building of 20 x 10 m: nodes 1 (0,0), 2 (20,0), 3 (20,10), 4 (0,10): the walls south, east, north, west (counter-clockwise).
    fn block() -> Vec<RawNode> {
        vec![
            node(1, 0.0, 0.0),
            node(2, 20.0, 0.0),
            node(3, 20.0, 10.0),
            node(4, 0.0, 10.0),
        ]
    }
    fn types(f: &[BuildingFace]) -> Vec<&'static str> {
        f.iter().map(|x| x.face_type.name()).collect()
    }

    #[test]
    fn a_street_in_the_south() {
        let ways = [building(100, &[1, 2, 3, 4, 1])];
        let f = building_faces(
            &ways,
            &block(),
            &[street(7, (-30.0, -8.0), (50.0, -8.0))],
            &[],
        );
        assert_eq!(
            types(&f),
            ["front", "side", "back", "side"],
            "south front, east side, north back, west side"
        );
        assert_eq!((f[0].from_node, f[0].to_node), (1, 2));
        assert!(
            (f[0].length_m - 20.0).abs() < 1e-3 && (f[0].azimuth_deg - 180.0).abs() < 1e-3,
            "{:?}",
            f[0]
        );
        assert!((f[0].distance_m.unwrap() - 8.0).abs() < 1e-3);
        assert_eq!(
            (f[0].street_osm_id, f[0].street_name.as_deref()),
            (Some(7), Some("street 7"))
        );
        assert_eq!(f[2].street_osm_id, None, "the back wall has no street");
        assert!(f
            .iter()
            .all(|x| !x.edited && x.face_type == x.computed_type));
    }

    #[test]
    fn the_same_for_a_clockwise_ring_and_a_street_in_the_north() {
        let ways = [building(100, &[1, 4, 3, 2, 1])]; // clockwise
        let f = building_faces(
            &ways,
            &block(),
            &[street(7, (-30.0, 18.0), (50.0, 18.0))],
            &[],
        );
        let north: Vec<_> = f
            .iter()
            .filter(|x| x.face_type == FaceType::Front)
            .collect();
        assert_eq!(north.len(), 1);
        assert_eq!(
            (
                north[0].from_node.min(north[0].to_node),
                north[0].from_node.max(north[0].to_node)
            ),
            (3, 4),
            "the north wall"
        );
        assert!(
            f.iter().any(|x| x.face_type == FaceType::Back),
            "a back wall in the south"
        );
    }

    #[test]
    fn no_street_no_front() {
        let ways = [building(100, &[1, 2, 3, 4, 1])];
        let f = building_faces(
            &ways,
            &block(),
            &[street(7, (-30.0, 500.0), (50.0, 500.0))],
            &[],
        );
        assert_eq!(types(&f), ["side", "side", "side", "side"]);
        assert!(f
            .iter()
            .all(|x| x.street_osm_id.is_none() && x.distance_m.is_none()));
        assert!(building_faces(&ways, &block(), &[], &[])
            .iter()
            .all(|x| x.face_type == FaceType::Side));
    }

    #[test]
    fn a_corner_building_has_two_fronts() {
        let ways = [building(100, &[1, 2, 3, 4, 1])];
        let streets = [
            street(7, (-30.0, -6.0), (50.0, -6.0)),
            street(8, (26.0, -30.0), (26.0, 40.0)),
        ];
        let f = building_faces(&ways, &block(), &streets, &[]);
        assert_eq!(
            types(&f),
            ["front", "front", "back", "side"],
            "south and east fronts, north back, west side"
        );
        assert_eq!((f[0].street_osm_id, f[1].street_osm_id), (Some(7), Some(8)));
    }

    #[test]
    fn a_shared_wall_is_a_party_wall() {
        let mut nodes = block();
        nodes.extend([node(5, 40.0, 0.0), node(6, 40.0, 10.0)]);
        let ways = [
            building(100, &[1, 2, 3, 4, 1]),
            building(101, &[2, 5, 6, 3, 2]),
        ];
        let f = building_faces(
            &ways,
            &nodes,
            &[street(7, (-30.0, -8.0), (60.0, -8.0))],
            &[],
        );
        let east: Vec<_> = f
            .iter()
            .filter(|x| {
                x.building_id == 100
                    && x.from_node.max(x.to_node) == 3
                    && x.from_node.min(x.to_node) == 2
            })
            .collect();
        let west: Vec<_> = f
            .iter()
            .filter(|x| {
                x.building_id == 101
                    && x.from_node.max(x.to_node) == 3
                    && x.from_node.min(x.to_node) == 2
            })
            .collect();
        assert_eq!(
            (east[0].face_type, west[0].face_type),
            (FaceType::Party, FaceType::Party)
        );
        assert_eq!(
            f.iter().filter(|x| x.face_type == FaceType::Party).count(),
            2
        );
        assert_eq!(
            f.iter().filter(|x| x.face_type == FaceType::Front).count(),
            2,
            "each building has its south front"
        );
    }

    #[test]
    fn a_rule_changes_the_type_and_keeps_the_computed_one() {
        let ways = [building(100, &[1, 2, 3, 4, 1])];
        let streets = [street(7, (-30.0, -8.0), (50.0, -8.0))];
        let edit = |a, b, t| FaceEdit {
            building: 100,
            from_node: a,
            to_node: b,
            face_type: t,
            door: None,
        };
        // the nodes of a rule can be in either order; a rule that gives the computed type is not an edit; a rule for another building does nothing
        let rules = [
            edit(3, 2, FaceType::Front),
            edit(1, 2, FaceType::Front),
            FaceEdit {
                building: 5,
                from_node: 3,
                to_node: 4,
                face_type: FaceType::Back,
                door: None,
            },
        ];
        let f = building_faces(&ways, &block(), &streets, &rules);
        assert_eq!(types(&f), ["front", "front", "back", "side"]);
        assert_eq!((f[1].computed_type, f[1].edited), (FaceType::Side, true));
        assert_eq!((f[0].computed_type, f[0].edited), (FaceType::Front, false));
        assert_eq!(f.iter().filter(|x| x.edited).count(), 1);
    }

    #[test]
    fn a_rule_gives_a_wall_a_door_with_no_change_of_type() {
        let ways = [building(100, &[1, 2, 3, 4, 1])];
        let streets = [street(7, (-30.0, -8.0), (50.0, -8.0))];
        let rules = [FaceEdit { building: 100, from_node: 2, to_node: 1, face_type: FaceType::Front, door: Some("main".into()) }];
        let f = building_faces(&ways, &block(), &streets, &rules);
        assert_eq!((f[0].face_type, f[0].edited, f[0].door.as_deref()), (FaceType::Front, false, Some("main")));
        assert_eq!(f.iter().filter(|x| x.door.is_some()).count(), 1);
    }

    #[test]
    fn rings_that_are_not_buildings_and_the_same_rows_each_time() {
        let mut ways = vec![
            building(100, &[1, 2, 3, 4, 1]),
            RawWay {
                id: 101,
                tags: vec![("building".into(), "no".into())],
                refs: vec![1, 2, 3, 4, 1],
            },
            RawWay {
                id: 102,
                tags: vec![("landuse".into(), "grass".into())],
                refs: vec![1, 2, 3, 4, 1],
            },
            building(103, &[1, 2, 3, 4]),    // not closed
            building(104, &[1, 2, 9, 4, 1]), // node 9 is not in the file
            building(105, &[1, 2, 1]),       // too short
        ];
        ways.sort_by_key(|w| w.id);
        let streets = [
            street(7, (-30.0, -8.0), (50.0, -8.0)),
            street(7, (50.0, -8.0), (-30.0, -8.0)),
        ]; // two directions of one segment
        let a = building_faces(&ways, &block(), &streets, &[]);
        assert!(a.iter().all(|x| x.building_id == 100) && a.len() == 4);
        let b = building_faces(&ways, &block(), &streets, &[]);
        let key = |f: &[BuildingFace]| {
            f.iter()
                .map(|x| {
                    (
                        x.building_id,
                        x.face_index,
                        x.face_type.name(),
                        x.street_edge_id,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(key(&a), key(&b));
    }

    #[test]
    fn a_building_behind_another_does_not_see_the_street() {
        // B (nodes 5 to 8) is 2 m north of A (nodes 1 to 4) and has the same width: the street in the south is blocked for B by A
        let mut nodes = block();
        nodes.extend([
            node(5, 0.0, 12.0),
            node(6, 20.0, 12.0),
            node(7, 20.0, 22.0),
            node(8, 0.0, 22.0),
        ]);
        let ways = [
            building(100, &[1, 2, 3, 4, 1]),
            building(101, &[5, 6, 7, 8, 5]),
        ];
        let f = building_faces(
            &ways,
            &nodes,
            &[street(7, (-30.0, -8.0), (50.0, -8.0))],
            &[],
        );
        let of = |id: i64| {
            f.iter()
                .filter(|x| x.building_id == id)
                .map(|x| x.face_type.name())
                .collect::<Vec<_>>()
        };
        assert_eq!(of(100), ["front", "side", "back", "side"]);
        assert_eq!(
            of(101),
            ["front", "side", "back", "side"],
            "B does not see the street (A is in the way), so its front is inferred from the nearest street"
        );
        let reasons: Vec<_> = f.iter().filter(|x| x.building_id == 101).map(|x| x.type_reason).collect();
        assert_eq!(reasons, ["inferred", "other", "opposite", "other"]);
        let a_reasons: Vec<_> = f.iter().filter(|x| x.building_id == 100).map(|x| x.type_reason).collect();
        assert_eq!(a_reasons[0], "street");
    }

    #[test]
    fn a_multipolygon_building_has_court_walls_for_its_inner_ring() {
        // an outer ring of 20 x 10 m (nodes 1 to 4) and a courtyard of 6 x 2 m (nodes 5 to 8) in the middle; the street is in the south
        let mut nodes = block();
        nodes.extend([node(5, 7.0, 4.0), node(6, 13.0, 4.0), node(7, 13.0, 6.0), node(8, 7.0, 6.0)]);
        let ways = [
            RawWay { id: 200, tags: vec![], refs: vec![1, 2, 3, 4, 1] },
            RawWay { id: 201, tags: vec![], refs: vec![5, 6, 7, 8, 5] },
        ];
        let rel = Relation {
            id: 900,
            tags: vec![("type".into(), "multipolygon".into()), ("building".into(), "yes".into())],
            members: vec![(200, MemberType::Way, "outer".into()), (201, MemberType::Way, "inner".into())],
        };
        let f = super::building_faces(&ways, &nodes, &[rel], &[street(7, (-30.0, -8.0), (50.0, -8.0))], &[]);
        assert!(f.iter().all(|x| x.building_id == 900 && x.building_type == "relation"));
        let outer: Vec<_> = f.iter().filter(|x| x.face_index < 4).map(|x| x.face_type.name()).collect();
        assert_eq!(outer, ["front", "side", "back", "side"], "the outer ring is a building like a way");
        let inner: Vec<_> = f.iter().filter(|x| x.face_index >= 4).collect();
        assert_eq!(inner.len(), 4);
        assert!(inner.iter().all(|x| x.face_type == FaceType::Court && x.type_reason == "inner"));
        // the normal of a court wall points into the courtyard: the southern wall of the courtyard (nodes 5 to 6) points north (azimuth 0)
        let south = inner.iter().find(|x| x.from_node.min(x.to_node) == 5 && x.from_node.max(x.to_node) == 6).unwrap();
        assert!(south.azimuth_deg < 1.0 || south.azimuth_deg > 359.0, "{}", south.azimuth_deg);
    }

    #[test]
    fn open_member_ways_are_chained_into_a_ring() {
        let rings = assemble(vec![vec![1, 2, 3], vec![1, 4, 3], vec![9, 10, 11, 9]]);
        assert!(rings.contains(&vec![9, 10, 11, 9]));
        assert!(rings.iter().any(|r| r.len() == 5 && r.first() == r.last()), "{rings:?}");
        assert!(assemble(vec![vec![1, 2, 3]]).is_empty(), "a chain that does not close is dropped");
    }

    #[test]
    fn an_entrance_makes_a_front_when_no_street_is_near() {
        // the entrance is the node 3 (the corner of the east and the north wall): both walls get it; the street is 500 m away
        let mut nodes = block();
        nodes[2].tags = vec![("entrance".into(), "main".into())];
        let ways = [building(100, &[1, 2, 3, 4, 1])];
        let streets = [street(7, (-30.0, 500.0), (50.0, 500.0))];
        let f = building_faces(&ways, &nodes, &streets, &[]);
        assert_eq!(types(&f), ["side", "front", "front", "back"]);   // the west wall is opposite the east front wall
        assert_eq!(f[1].type_reason, "entrance");
        // a service door is no way in
        nodes[2].tags = vec![("entrance".into(), "service".into())];
        assert!(building_faces(&ways, &nodes, &streets, &[]).iter().all(|x| x.face_type == FaceType::Side));
    }

    #[test]
    fn a_street_that_the_address_names_wins() {
        // two streets at the same distance, one south and one north of the building: the address names the north one
        let mut w = building(100, &[1, 2, 3, 4, 1]);
        w.tags.push(("addr:street".into(), "Street 3".into()));
        let streets = [
            street(7, (-30.0, -8.0), (50.0, -8.0)),
            street(3, (-30.0, 18.0), (50.0, 18.0)),
        ];
        let f = building_faces(&[w], &block(), &streets, &[]);
        assert_eq!(f[2].face_type, FaceType::Front, "the north wall");
        assert_eq!(f[2].type_reason, "address");
    }

    #[test]
    fn a_street_of_a_higher_class_wins() {
        // a service road and a primary road, both at 8 m, one in the south and one in the north
        let mut small = street(7, (-30.0, -8.0), (50.0, -8.0));
        small.highway = "service".into();
        let mut big = street(3, (-30.0, 18.0), (50.0, 18.0));
        big.highway = "primary".into();
        let f = building_faces(&[building(100, &[1, 2, 3, 4, 1])], &block(), &[small, big], &[]);
        assert_eq!(f[2].face_type, FaceType::Front, "the north wall faces the primary road");
        assert_eq!(f[2].type_reason, "street");
    }

    #[test]
    fn a_tie_goes_to_the_lower_edge_id() {
        // two streets at the same distance and alignment: the wall names the one with the lower edge id, in any order of the input
        let ways = [building(100, &[1, 2, 3, 4, 1])];
        let (s1, mut s2) = (
            street(7, (-30.0, -8.0), (50.0, -8.0)),
            street(3, (-30.0, -8.0), (50.0, -8.0)),
        );
        s2.a.1 += 1e-12; // a different segment, the same distance for the test
        s2.b.1 += 1e-12;
        let one = building_faces(&ways, &block(), &[s1.clone(), s2.clone()], &[]);
        let two = building_faces(&ways, &block(), &[s2, s1], &[]);
        assert_eq!(one[0].street_edge_id, two[0].street_edge_id);
    }
}
