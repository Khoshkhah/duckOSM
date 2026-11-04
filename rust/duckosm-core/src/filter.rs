//! The road filter of duckOSM (`RoadFilter`): which ways are in the network of a mode, and their
//! one-way direction, access and lanes. Modes: driving, walking, cycling.

use crate::model::{Attrs, Tags, Way};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Driving,
    Walking,
    Cycling,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Driving => "driving",
            Mode::Walking => "walking",
            Mode::Cycling => "cycling",
        }
    }

    /// Parse a list of mode names.
    pub fn parse_all<S: AsRef<str>>(names: &[S]) -> Result<Vec<Mode>, String> {
        names
            .iter()
            .map(|m| {
                Mode::parse(m.as_ref().trim()).ok_or_else(|| {
                    format!(
                        "unknown mode '{}' (use driving, walking or cycling)",
                        m.as_ref()
                    )
                })
            })
            .collect()
    }

    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "driving" => Some(Mode::Driving),
            "walking" => Some(Mode::Walking),
            "cycling" => Some(Mode::Cycling),
            _ => None,
        }
    }
}

pub const NON_ROAD: [&str; 7] = [
    "footway",
    "path",
    "cycleway",
    "steps",
    "pedestrian",
    "bridleway",
    "corridor",
];
const NOT_BUILT: [&str; 5] = ["proposed", "construction", "abandoned", "razed", "disused"];
const ROAD_CLASSES: [&str; 16] = [
    "motorway",
    "motorway_link",
    "trunk",
    "trunk_link",
    "primary",
    "primary_link",
    "secondary",
    "secondary_link",
    "tertiary",
    "tertiary_link",
    "unclassified",
    "residential",
    "service",
    "living_street",
    "road",
    "busway",
];
const WALK_CLASSES: [&str; 21] = [
    "footway",
    "path",
    "pedestrian",
    "steps",
    "living_street",
    "residential",
    "service",
    "platform",
    "corridor",
    "track",
    "bridleway",
    "road",
    "unclassified",
    "tertiary",
    "tertiary_link",
    "secondary",
    "secondary_link",
    "primary",
    "primary_link",
    "trunk",
    "trunk_link",
];
const CYCLE_CLASSES: [&str; 14] = [
    "cycleway",
    "path",
    "track",
    "bridleway",
    "living_street",
    "residential",
    "service",
    "unclassified",
    "tertiary",
    "tertiary_link",
    "secondary",
    "secondary_link",
    "primary",
    "primary_link",
];
const NON_MOTOR: [&str; 8] = [
    "footway",
    "path",
    "cycleway",
    "steps",
    "pedestrian",
    "bridleway",
    "corridor",
    "platform",
];
const YES: [&str; 3] = ["yes", "designated", "permissive"];

type Tri = Option<bool>; // SQL three-valued logic: None is NULL

fn is(o: Option<&str>, v: &str) -> bool {
    o == Some(v)
}
fn is_in(o: Option<&str>, vs: &[&str]) -> bool {
    o.is_some_and(|x| vs.contains(&x))
}
/// `x IN (...)`: NULL when x is NULL.
fn tin(o: Option<&str>, vs: &[&str]) -> Tri {
    o.map(|x| vs.contains(&x))
}
fn and(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}
fn or(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}
/// `COALESCE(x, '') <> v`
fn ne_or_null(o: Option<&str>, v: &str) -> Tri {
    Some(o.unwrap_or("") != v)
}
fn first<'a>(os: &[Option<&'a str>]) -> Option<&'a str> {
    os.iter().find_map(|o| *o)
}

/// A pedestrian street that admits cars is a road (`_ped_ok`).
pub fn ped_ok(t: &Tags) -> bool {
    if !is(t.highway(), "pedestrian") {
        return false;
    }
    let mv = first(&[t.motorcar(), t.motor_vehicle()]);
    mv.is_some_and(|x| YES.contains(&x))
        || is_in(t.access(), &["delivery", "destination", "agricultural"])
}

/// Does the way count for the global junctions? Every highway way that exists on the ground.
pub fn is_built_highway(t: &Tags) -> bool {
    t.highway().is_some() && !is_in(t.highway(), &NOT_BUILT)
}

/// A way that the reader keeps: a highway, or a ferry line (it has no highway tag).
pub fn is_kept_by_reader(t: &Tags) -> bool {
    t.highway().is_some() || is(t.route(), "ferry")
}

/// A road for the cut rule of the global junctions: not a path, or a pedestrian street cars may use.
pub fn is_road_raw(t: &Tags) -> bool {
    let hw = t.highway().unwrap_or("");
    !NON_ROAD.contains(&hw) || ped_ok(t)
}

/// A way of one network, after the filter.
#[derive(Clone, Debug)]
pub struct ModeWay {
    pub id: i64,
    /// Node ids; reversed for a one-way way against its drawing (`oneway=-1`).
    pub refs: Vec<i64>,
    pub attrs: Arc<Attrs>,
    pub oneway: bool,
    pub bus_back: bool,
    pub access: Option<Arc<str>>,
    pub lanes_fwd: Option<i32>,
    pub lanes_bwd: Option<i32>,
    pub dismount: Option<bool>,
    /// walk_type (walking), or cycle_type of the forward edge (cycling)
    pub type_fwd: Option<&'static str>,
    /// cycle_type of the reverse edge (cycling; the same as `type_fwd` in walking)
    pub type_rev: Option<&'static str>,
}

fn s(o: Option<&str>) -> Option<String> {
    o.map(str::to_string)
}

/// The first run of digits in a tag value as a positive INTEGER (`NULLIF(TRY_CAST(regexp_extract(v, '\d+') AS INTEGER), 0)`).
fn first_int(o: Option<&str>) -> Option<i32> {
    let v = o?;
    let b = v.as_bytes();
    let start = b.iter().position(|c| c.is_ascii_digit())?;
    let end = b[start..]
        .iter()
        .position(|c| !c.is_ascii_digit())
        .map_or(b.len(), |e| start + e);
    v[start..end].parse::<i32>().ok().filter(|&n| n != 0)
}

/// A footway or pedestrian way that does not admit bikes: cyclists push the bike (`dismount`).
fn pushes_bike(t: &Tags) -> bool {
    is_in(t.highway(), &["footway", "pedestrian"]) && !is_in(t.bicycle(), &YES)
}

/// The access value of the way for the mode (`_access_expression`).
fn access_of(t: &Tags, mode: Mode, dismount: bool) -> Option<String> {
    match mode {
        Mode::Driving => {
            let car = first(&[t.motorcar(), t.motor_vehicle(), t.vehicle(), t.access()]);
            if is(t.highway(), "pedestrian") && !ped_ok(t) {
                return Some(match car {
                    Some(c) if c == "delivery" || c == "private" => c.to_string(),
                    _ => "pedestrian".to_string(),
                });
            }
            let bus_ok =
                is_in(t.bus(), &["yes", "designated"]) || is_in(t.psv(), &["yes", "designated"]);
            if is(t.highway(), "busway")
                || car == Some("psv")
                || (matches!(car, Some("no") | Some("private")) && bus_ok)
            {
                return Some("bus".to_string());
            }
            car.map(str::to_string)
        }
        Mode::Walking => first(&[t.foot(), t.access()]).map(str::to_string),
        Mode::Cycling => {
            if dismount && pushes_bike(t) {
                first(&[t.foot(), t.access()]).map(str::to_string)
            } else {
                first(&[t.bicycle(), t.vehicle(), t.access()]).map(str::to_string)
            }
        }
    }
}

/// Travel direction: 1 one-way as drawn, -1 one-way against the drawing, 0 two-way (`_direction_expression`).
// the branches follow the CASE of duckOSM's SQL, so two of them give the same value
#[allow(clippy::if_same_then_else)]
fn direction(t: &Tags, mode: Mode, dismount: bool) -> i32 {
    match mode {
        Mode::Walking => {
            if is_in(t.oneway_foot(), &["yes", "1", "true"]) {
                1
            } else if is(t.oneway_foot(), "-1") {
                -1
            } else {
                0
            }
        }
        Mode::Cycling => {
            if dismount && pushes_bike(t) {
                return 0;
            }
            if is(t.oneway_bicycle(), "no") {
                0
            } else if is_in(t.oneway_bicycle(), &["yes", "1", "true"]) {
                1
            } else if is(t.oneway_bicycle(), "-1") {
                -1
            } else if is_in(t.junction(), &["roundabout", "circular"])
                || is_in(t.oneway(), &["yes", "1", "true"])
            {
                1
            } else if is(t.oneway(), "-1") {
                -1
            } else {
                0
            }
        }
        Mode::Driving => {
            if is_in(t.junction(), &["roundabout", "circular"]) {
                1
            } else if is_in(t.oneway(), &["yes", "1", "true"]) {
                1
            } else if is(t.oneway(), "-1") {
                -1
            } else if is_in(t.oneway(), &["no", "false", "0"]) {
                0
            } else if is_in(t.highway(), &["motorway", "motorway_link"]) {
                1
            } else {
                0
            }
        }
    }
}

/// The WHERE clause of `RoadFilter` for the mode (NULL counts as false).
fn in_network(t: &Tags, mode: Mode, dismount: bool) -> bool {
    let hw = t.highway();
    let class_ok: Tri = match mode {
        Mode::Driving => or(
            tin(t.highway(), &ROAD_CLASSES),
            Some(hw == Some("pedestrian")),
        ),
        Mode::Walking => {
            let not_motorway = Some(!matches!(hw.unwrap_or(""), "motorway" | "motorway_link"));
            let sidewalk_lr: Tri =
                Some(is(t.sidewalk_left(), "separate") && is(t.sidewalk_right(), "separate"));
            let by_class = and(
                and(
                    and(
                        tin(t.highway(), &WALK_CLASSES),
                        ne_or_null(t.motorroad(), "yes"),
                    ),
                    and(
                        ne_or_null(t.foot(), "use_sidepath"),
                        ne_or_null(t.sidewalk(), "separate"),
                    ),
                ),
                and(
                    ne_or_null(t.sidewalk_both(), "separate"),
                    sidewalk_lr.map(|b| !b),
                ),
            );
            and(
                not_motorway,
                or(
                    or(
                        tin(t.foot(), &YES),
                        tin(t.sidewalk(), &["yes", "both", "left", "right"]),
                    ),
                    by_class,
                ),
            )
        }
        Mode::Cycling => {
            let cycleway_infra: Tri = t
                .cycleway()
                .map(|c| !["no", "none", "separate"].contains(&c));
            let base = or(
                or(tin(t.highway(), &CYCLE_CLASSES), tin(t.bicycle(), &YES)),
                cycleway_infra,
            );
            if dismount {
                or(base, tin(t.highway(), &["footway", "pedestrian"]))
            } else {
                base
            }
        }
    };
    class_ok == Some(true)
}

const SHARED_ROAD: [&str; 12] = [
    "residential",
    "service",
    "unclassified",
    "road",
    "tertiary",
    "tertiary_link",
    "secondary",
    "secondary_link",
    "primary",
    "primary_link",
    "trunk",
    "trunk_link",
];

/// `walk_type` of a way (`FunctionalType._walk_type`); `t` is None for a connector (no way).
pub fn walk_type(hw: &str, t: Option<&Tags>) -> &'static str {
    if let Some(t) = t {
        if hw == "steps" {
            return if t.conveying().is_some() {
                "escalator"
            } else {
                "steps"
            };
        }
        if hw == "footway" {
            return match t.footway() {
                Some("sidewalk") => "sidewalk",
                Some("crossing") => "crossing",
                _ => "footpath",
            };
        }
        if hw == "pedestrian" {
            return if is(t.area(), "yes") {
                "plaza"
            } else {
                "pedestrian_street"
            };
        }
        if hw == "living_street" {
            return "shared_street";
        }
        if !matches!(t.sidewalk().unwrap_or("no"), "no" | "none" | "separate")
            && !matches!(hw, "path" | "track" | "bridleway" | "corridor" | "platform")
        {
            return "sidewalk";
        }
        if SHARED_ROAD.contains(&hw) {
            return "shared_road";
        }
        if hw == "corridor" || is(t.indoor(), "yes") {
            return "corridor";
        }
        if hw == "platform" {
            return "platform";
        }
        if matches!(hw, "path" | "track" | "bridleway") {
            return "path";
        }
        return "footpath";
    }
    match hw {
        "steps" => "steps",
        "pedestrian" => "pedestrian_street",
        "living_street" => "shared_street",
        h if SHARED_ROAD.contains(&h) => "shared_road",
        "corridor" => "corridor",
        "platform" => "platform",
        "path" | "track" | "bridleway" => "path",
        _ => "footpath",
    }
}

/// `cycle_type` of a way for the forward (`reverse` false: `cycleway:right`) or reverse edge (`cycleway:left`).
/// `t` is None for a connector. The `dismount` case is applied on the edge.
pub fn cycle_type(hw: &str, t: Option<&Tags>, reverse: bool) -> &'static str {
    if hw == "cycleway" {
        return "cycleway";
    }
    let path = matches!(hw, "path" | "track" | "bridleway");
    if let Some(t) = t {
        let side = first(&[
            if reverse {
                t.cycleway_left()
            } else {
                t.cycleway_right()
            },
            t.cycleway_both(),
            t.cycleway(),
        ]);
        match side {
            Some("track") => return "cycle_track",
            Some("lane") => return "cycle_lane",
            Some("shared_lane") => return "shared_lane",
            Some("share_busway") => return "bus_cycle_lane",
            _ => {}
        }
        if path && is(t.segregated(), "yes") {
            return "segregated_path";
        }
    }
    if path {
        "shared_path"
    } else {
        "mixed_traffic"
    }
}

/// Apply the filter of a mode to one way. None if the way is not in that network.
pub fn mode_way(w: &Way, mode: Mode, cycling_dismount: bool) -> Option<ModeWay> {
    let t = &w.tags;
    let dismount = mode == Mode::Cycling && cycling_dismount;
    let forbidden: &[&str] = match mode {
        Mode::Driving => &["no", "agricultural", "forestry", "emergency"],
        _ => &["no"],
    };
    if !(t.highway().is_some() || is(t.route(), "ferry")) || !in_network(t, mode, dismount) {
        return None;
    }
    let access = access_of(t, mode, dismount);
    if forbidden.contains(&access.as_deref().unwrap_or("")) {
        return None;
    }
    // a ferry line has no highway tag: it gets its own class
    let raw_hw = t.highway().unwrap_or("ferry");
    let highway = if mode == Mode::Driving && ped_ok(t) {
        "living_street".to_string()
    } else {
        raw_hw.to_string()
    };
    let dir = direction(t, mode, dismount);
    let car_dir = direction(t, Mode::Driving, false); // the lanes are the cars' in every mode
    let contra = is(t.oneway_bus(), "no") || is(t.oneway_psv(), "no");
    let n_total = first_int(t.lanes());
    let n_fwd = first_int(t.lanes_forward());
    let n_bwd = first_int(t.lanes_backward());
    let n_rev = first_int(t.lanes_reversible()).unwrap_or(0);
    let default = if highway == "motorway" || highway == "trunk" {
        2
    } else {
        1
    };
    let non_motor = NON_MOTOR.contains(&highway.split(';').next().unwrap_or(""));

    let car_fwd = if non_motor {
        None
    } else {
        let base = if car_dir == -1 {
            // GREATEST ignores a NULL, so an untagged one-way road has 1 lane here, not the class default
            n_bwd.unwrap_or_else(|| {
                n_total.map_or(1, |n| {
                    (n - if contra { n_fwd.unwrap_or(1) } else { 0 }).max(1)
                })
            })
        } else if let Some(n) = n_fwd {
            n
        } else if car_dir != 0 {
            n_total.map_or(1, |n| {
                (n - if contra { n_bwd.unwrap_or(1) } else { 0 }).max(1)
            })
        } else if let (Some(n), Some(b)) = (n_total, n_bwd) {
            (n - b).max(1)
        } else if let Some(n) = n_total {
            ((n + 1) / 2).max(1)
        } else {
            default
        };
        Some(base + n_rev)
    };
    let car_bwd = if non_motor || (car_dir != 0 && !contra) {
        None
    } else {
        let base = if let Some(b) = n_bwd {
            b
        } else if let (Some(n), Some(f)) = (n_total, n_fwd) {
            (n - f).max(1)
        } else if let Some(n) = n_total {
            (n / 2).max(1)
        } else {
            default
        };
        Some(base + n_rev)
    };
    // this mode's forward edge runs along `dir`; the cars' forward lanes run along `car_dir`
    let swap = (dir == -1) != (car_dir == -1);
    let (lanes_fwd, lanes_bwd) = if swap {
        (car_bwd, car_fwd)
    } else {
        (car_fwd, car_bwd)
    };

    let refs = if dir == -1 {
        w.refs.iter().rev().copied().collect()
    } else {
        w.refs.clone()
    };
    let (type_fwd, type_rev) = match mode {
        Mode::Walking => (
            Some(walk_type(&highway, Some(t))),
            Some(walk_type(&highway, Some(t))),
        ),
        Mode::Cycling => (
            Some(cycle_type(&highway, Some(t), false)),
            Some(cycle_type(&highway, Some(t), true)),
        ),
        Mode::Driving => (None, None),
    };
    Some(ModeWay {
        id: w.id,
        refs,
        attrs: Arc::new(Attrs {
            highway,
            name: s(t.name()),
            maxspeed: s(t.maxspeed()),
            surface: s(t.surface()),
            junction: s(t.junction()),
            layer: s(t.layer()),
            bridge: s(t.bridge()),
            tunnel: s(t.tunnel()),
            service: s(t.service()),
        }),
        oneway: dir != 0,
        bus_back: mode == Mode::Driving && dir != 0 && contra,
        access: access.map(Arc::from),
        lanes_fwd,
        lanes_bwd,
        dismount: if dismount { Some(pushes_bike(t)) } else { None },
        type_fwd,
        type_rev,
    })
}

/// The driving filter (kept for the first spike).
pub fn driving_way(w: &Way) -> Option<ModeWay> {
    mode_way(w, Mode::Driving, false)
}
