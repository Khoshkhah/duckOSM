//! duckOSM's `driving.lane_profile` (docs/lane_profile.md): one row for each lane of each driving edge and each
//! bus-only edge, from left to right. A port of duckOSM's `processors/lane_profile.py` and of `_inherit_lanes` in
//! `gmns.py`. A count, a use or a width that no tag gives has the source `default`.

use crate::filter::Mode;
use crate::fixes::OsmFixes;
use crate::model::{Attrs, Network, Way};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Metres: a motor lane without a width tag (duckOSM `LANE_W`).
const LANE_W: f64 = 3.25;
/// An on-road bike lane without a width tag.
const BIKE_W: f64 = 1.5;
/// The tags that make a way "tagged" for the inheritance of the lane count.
const LANE_TAGS: [&str; 5] = [
    "lanes",
    "lanes:forward",
    "lanes:backward",
    "turn:lanes",
    "width:lanes",
];

/// A row of `driving.lane_profile`.
#[derive(Clone, Debug, PartialEq)]
pub struct LaneRow {
    pub edge_id: i64,
    pub lane_num: i32,
    pub use_: &'static str,
    pub width_m: f64,
    pub turn: Option<String>,
    pub source: String,
    pub width_source: String,
}

/// The columns of an edge that the rules read.
#[derive(Clone, Debug)]
pub struct LaneEdge {
    pub edge_id: i64,
    pub osm_id: i64,
    pub is_reverse: bool,
    pub oneway: bool,
    pub attrs: Arc<Attrs>,
    pub lanes: Option<i32>,
    pub source: i64,
    pub target: i64,
}

/// The input of the lane profile, kept while the modes are built: the driving edges, the bus-only edges
/// (`private_edges` with `access = 'bus'`) and the edge ids of the cycling network.
#[derive(Default)]
pub struct LaneEdges {
    driving: Option<(Vec<LaneEdge>, Vec<LaneEdge>)>,
    cycling: HashSet<i64>,
}

impl LaneEdges {
    pub fn add(&mut self, mode: Mode, net: &Network) {
        let keep = |e: &crate::model::Edge| LaneEdge {
            edge_id: e.edge_id,
            osm_id: e.osm_id,
            is_reverse: e.is_reverse,
            oneway: e.oneway,
            attrs: e.attrs.clone(),
            lanes: e.lanes,
            source: e.source,
            target: e.target,
        };
        match mode {
            Mode::Driving => {
                let bus = net
                    .private_edges
                    .iter()
                    .filter(|e| e.access.as_deref() == Some("bus"))
                    .map(keep)
                    .collect();
                self.driving = Some((net.edges.iter().map(keep).collect(), bus));
            }
            Mode::Cycling => self.cycling = net.edges.iter().map(|e| e.edge_id).collect(),
            Mode::Walking => {}
        }
    }
}

/// The tags of a way, as duckOSM's `raw.ways.tags`.
struct WayTags<'a>(&'a [(&'a str, &'a str)]);

impl<'a> WayTags<'a> {
    fn get(&self, k: &str) -> Option<&'a str> {
        self.0.iter().find(|p| p.0 == k).map(|p| p.1)
    }

    /// Python's `tags.get(k)` in a test of truth: an empty value is no value.
    fn value(&self, k: &str) -> Option<&'a str> {
        self.get(k).filter(|v| !v.is_empty())
    }

    fn is(&self, k: &str, v: &str) -> bool {
        self.get(k) == Some(v)
    }

    /// A per-direction tag: `base:backward` on the reverse edge; `base:forward`, else `base`, on the forward one.
    fn pick(&self, base: &str, reverse: bool) -> Option<&'a str> {
        if reverse {
            self.value(&format!("{base}:backward"))
        } else {
            self.value(&format!("{base}:forward"))
                .or_else(|| self.value(base))
        }
    }
}

fn split(s: Option<&str>) -> Vec<&str> {
    s.map_or(Vec::new(), |s| s.split('|').collect())
}

/// A number of a tag; 0 is no number (Python's `_num(x) or ...`).
// ponytail: Python's float() also reads "1_000"; Rust's parse does not, add it if a tag needs it
fn num(s: Option<&str>) -> Option<f64> {
    s?.trim().parse::<f64>().ok().filter(|v| *v != 0.0)
}

fn class_width(cls: &str) -> f64 {
    match cls {
        "service" => 2.5,
        "residential" | "living_street" | "unclassified" => 3.0,
        _ => LANE_W,
    }
}

/// The road class: the `highway` value before `;`, without `_link`.
fn class(highway: &str) -> &str {
    let c = highway.split(';').next().unwrap_or("");
    c.strip_suffix("_link").unwrap_or(c)
}

/// One lane of [`lanes_of`] (a row without the edge id).
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub lane_num: i32,
    pub use_: &'static str,
    pub width_m: f64,
    pub turn: Option<String>,
    pub source: String,
    pub width_source: String,
}

/// What [`lanes_of`] needs to know about the edge, apart from its tags.
#[derive(Clone, Copy, Debug)]
pub struct EdgeLanes<'a> {
    pub reverse: bool,
    pub oneway: bool,
    pub highway: &'a str,
    /// The motor lane count and where it came from.
    pub n_motor: Option<i32>,
    pub n_source: &'a str,
    /// A bus-only edge: one lane.
    pub bus_only: bool,
    /// Bikes share the bus-only edge.
    pub shared_bike: bool,
    /// The edge ends where its way ends, in its direction: only its lanes get the arrows.
    pub way_end: bool,
}

/// The lanes of one directed edge, from left to right (duckOSM `lanes_of`).
pub fn lanes_of(tags: &[(&str, &str)], e: EdgeLanes) -> Vec<Lane> {
    let t = WayTags(tags);
    let reverse = e.reverse;
    let default_or = |w: Option<f64>, src: String| {
        if w.is_some() {
            src
        } else {
            "default".to_string()
        }
    };
    if e.bus_only {
        let w = num(split(t.pick("width:lanes", reverse)).first().copied());
        return vec![Lane {
            lane_num: 1,
            use_: if e.shared_bike { "bus,bike" } else { "bus" },
            width_m: w.unwrap_or(LANE_W),
            turn: None,
            source: "access=bus".into(),
            width_source: default_or(w, "width:lanes".into()),
        }];
    }
    let turns = split(t.pick("turn:lanes", reverse));
    let widths = split(t.pick("width:lanes", reverse));
    let n_motor = match e.n_motor {
        None | Some(0) => 1,
        Some(n) => n as i64,
    };
    let n = (turns.len() as i64).max(n_motor).max(1) as usize;
    let src = if turns.len() as i64 > n_motor {
        "turn:lanes"
    } else {
        e.n_source
    };
    let yes = |l: &[&str], i: usize| matches!(l.get(i), Some(&"designated") | Some(&"yes"));
    let bikes = split(t.pick("bicycle:lanes", reverse));
    let psvs = split(t.pick("psv:lanes", reverse));
    let buses = split(t.pick("bus:lanes", reverse));
    let cw = class_width(class(e.highway));
    let mut out: Vec<Lane> = (0..n)
        .map(|i| {
            let (use_, why) = if yes(&bikes, i) {
                ("bike", "bicycle:lanes")
            } else if yes(&psvs, i) {
                ("bus", "psv:lanes")
            } else if yes(&buses, i) {
                ("bus", "bus:lanes")
            } else {
                ("auto", src)
            };
            let w = num(widths.get(i).copied());
            let turn = turns
                .get(i)
                .filter(|s| e.way_end && !matches!(**s, "" | "none"));
            Lane {
                lane_num: i as i32 + 1,
                use_,
                width_m: w.unwrap_or(cw),
                turn: turn.map(|s| s.to_string()),
                source: why.into(),
                width_source: default_or(w, "width:lanes".into()),
            }
        })
        .collect();
    // this direction's right side, seen along the way
    let side = if reverse { "left" } else { "right" };
    // bus lanes counted (lanes:bus) or by side (busway:<side>=lane) when no per-lane tag says which: the right-most
    // motor lanes
    if !out.iter().any(|x| x.use_ == "bus") {
        let mut k = num(t.pick("lanes:bus", reverse))
            .or_else(|| num(t.pick("lanes:psv", reverse)))
            .unwrap_or(0.0) as i64;
        let mut why = "lanes:bus".to_string();
        for key in [
            format!("busway:{side}"),
            "busway:both".into(),
            "busway".into(),
        ] {
            if k == 0 && t.is(&key, "lane") {
                (k, why) = (1, key);
            }
        }
        // ponytail: a bus count >= the lanes leaves them all motor (duckOSM: no car lane left; listed by its checks later)
        if 0 < k && (k as usize) < out.len() {
            let from = out.len() - k as usize;
            for x in &mut out[from..] {
                x.use_ = "bus";
                x.source = why.clone();
            }
        }
    }
    // a bus lane that bikes share (cycleway:<this side>=share_busway): its right-most bus lane
    let cycleway_side = format!("cycleway:{side}");
    if [cycleway_side.as_str(), "cycleway:both", "cycleway"]
        .iter()
        .any(|k| t.is(k, "share_busway"))
    {
        if let Some(x) = out.iter_mut().rev().find(|x| x.use_ == "bus") {
            x.use_ = "bus,bike";
            x.source = format!("{} + cycleway share_busway", x.source);
        }
    }
    // bike lanes beside the motor lanes (not counted in lanes): right of travel; left of a one-way's travel (with
    // the traffic)
    if !bikes.contains(&"designated") {
        let bike = |key: &str, lane_num: i32| {
            let w = num(t.get(&format!("{key}:width"))).or_else(|| num(t.get("cycleway:width")));
            Lane {
                lane_num,
                use_: "bike",
                width_m: w.unwrap_or(BIKE_W),
                turn: None,
                source: key.into(),
                width_source: default_or(w, format!("{key}:width")),
            }
        };
        if let Some(key) = [cycleway_side.as_str(), "cycleway:both", "cycleway"]
            .into_iter()
            .find(|k| t.is(k, "lane"))
        {
            out.push(bike(key, n as i32 + 1));
        }
        if e.oneway && !reverse && !t.is("oneway:bicycle", "no") {
            if let Some(key) = ["cycleway:left", "cycleway:both"].into_iter().find(|k| {
                t.is(k, "lane") && !matches!(t.get(&format!("{k}:oneway")), Some("-1") | Some("no"))
            }) {
                out.insert(0, bike(key, -1));
            }
        }
    }
    out
}

/// The lane count of the untagged one-way edges that plainly continue a road (duckOSM `gmns._inherit_lanes`): the
/// node between them joins only the two edges, both one-way, of one class. The edge takes the count of the edge
/// that leads in, else of the edge that it leads into, when that has more lanes. The passes repeat, so a way cut
/// into pieces gets the count piece by piece. Returns the edges that change. duckOSM goes through the edges in
/// DuckDB's order and this function in the order of `edges`: a count only goes up, so the result is the same
/// when 6 passes are enough.
fn inherit_lanes(edges: &[LaneEdge], tagged: &[bool]) -> HashMap<i64, i32> {
    let mut lanes: Vec<Option<i32>> = edges.iter().map(|e| e.lanes).collect();
    let mut at: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        at.entry(e.source).or_default().push(i);
        if e.target != e.source {
            at.entry(e.target).or_default().push(i);
        }
    }
    let n_or_1 = |n: Option<i32>| match n {
        None | Some(0) => 1,
        Some(n) => n,
    };
    let mut changed = HashMap::new();
    for _ in 0..6 {
        let mut step = false;
        for (i, e) in edges.iter().enumerate() {
            if tagged[i] || !e.oneway {
                continue;
            }
            // what leads in, else what it leads into
            for (node, leads_in) in [(e.source, true), (e.target, false)] {
                let others: Vec<usize> = at[&node].iter().copied().filter(|&j| j != i).collect();
                let [j] = others[..] else {
                    continue;
                };
                let o = &edges[j];
                let end = if leads_in { o.target } else { o.source };
                if o.oneway
                    && class(&o.attrs.highway) == class(&e.attrs.highway)
                    && end == node
                    && n_or_1(lanes[j]) > n_or_1(lanes[i])
                {
                    lanes[i] = lanes[j];
                    // duckOSM keeps o["lanes"] as it is; a NULL there cannot be more than 1
                    changed.insert(e.edge_id, lanes[j].unwrap_or(1));
                    step = true;
                    break;
                }
            }
        }
        if !step {
            break;
        }
    }
    changed
}

/// `driving.lane_profile`, or None when the driving mode was not built. `ways` are the ways of the build, sorted by
/// id, after all modes (the arrows of the fixes are then in their tags). The ways that a fixes rule gives a lane
/// count have the source `override`.
pub fn lane_profile(input: &LaneEdges, ways: &[Way], fixes: &OsmFixes) -> Option<Vec<LaneRow>> {
    let (driving, bus) = input.driving.as_ref()?;
    let overrides: HashSet<i64> = fixes
        .ways
        .iter()
        .filter(|r| r.lanes.is_some() || r.lanes_forward.is_some() || r.lanes_backward.is_some())
        .map(|r| r.osm_id)
        .collect();
    let way = |osm_id: i64| {
        ways.binary_search_by_key(&osm_id.abs(), |w| w.id)
            .ok()
            .map(|i| &ways[i])
    };
    fn tags_of(w: Option<&Way>) -> Vec<(&str, &str)> {
        w.map_or(Vec::new(), |w| w.all_tags().collect())
    }
    let tagged: Vec<bool> = driving
        .iter()
        .map(|e| {
            let tags = tags_of(way(e.osm_id));
            tags.iter().any(|(k, _)| LANE_TAGS.contains(k)) || overrides.contains(&e.osm_id.abs())
        })
        .collect();
    let inherited = inherit_lanes(driving, &tagged);
    let mut out = Vec::new();
    let mut push = |edge_id: i64, lanes: Vec<Lane>| {
        out.extend(lanes.into_iter().map(|l| LaneRow {
            edge_id,
            lane_num: l.lane_num,
            use_: l.use_,
            width_m: l.width_m,
            turn: l.turn,
            source: l.source,
            width_source: l.width_source,
        }))
    };
    for e in driving {
        let w = way(e.osm_id);
        let tags = tags_of(w);
        let t = WayTags(&tags);
        let dir = if e.is_reverse {
            "lanes:backward"
        } else {
            "lanes:forward"
        };
        let src = if overrides.contains(&e.osm_id.abs()) {
            "override"
        } else if inherited.contains_key(&e.edge_id) {
            "inherited"
        } else if t.value(dir).is_some() {
            dir
        } else if t.value("lanes").is_some() {
            "lanes"
        } else {
            "default"
        };
        let way_end = w.is_some_and(|w| {
            let end = if e.is_reverse {
                w.refs.first()
            } else {
                w.refs.last()
            };
            end == Some(&e.target)
        });
        let lanes = lanes_of(
            &tags,
            EdgeLanes {
                reverse: e.is_reverse,
                oneway: e.oneway,
                highway: &e.attrs.highway,
                n_motor: inherited.get(&e.edge_id).copied().or(e.lanes),
                n_source: src,
                bus_only: false,
                shared_bike: false,
                way_end,
            },
        );
        push(e.edge_id, lanes);
    }
    for e in bus {
        let tags = tags_of(way(e.osm_id));
        let t = WayTags(&tags);
        // bikes share a bus-only edge: oneway:bicycle=no, a share_busway, or the edge is in the cycling network
        let shared = t.is("oneway:bicycle", "no")
            || input.cycling.contains(&e.edge_id)
            || ["cycleway", "cycleway:left", "cycleway:right"]
                .iter()
                .any(|k| t.is(k, "share_busway"));
        let lanes = lanes_of(
            &tags,
            EdgeLanes {
                reverse: e.is_reverse,
                oneway: false,
                highway: &e.attrs.highway,
                n_motor: None,
                n_source: "default",
                bus_only: true,
                shared_bike: shared,
                way_end: true,
            },
        );
        push(e.edge_id, lanes);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(
        reverse: bool,
        oneway: bool,
        highway: &'static str,
        n: i32,
        src: &'static str,
    ) -> EdgeLanes<'static> {
        EdgeLanes {
            reverse,
            oneway,
            highway,
            n_motor: Some(n),
            n_source: src,
            bus_only: false,
            shared_bike: false,
            way_end: true,
        }
    }

    /// (lane_num, use, width, turn, source) of each lane.
    fn short(l: &[Lane]) -> Vec<(i32, &str, f64, Option<&str>, &str)> {
        l.iter()
            .map(|x| {
                (
                    x.lane_num,
                    x.use_,
                    x.width_m,
                    x.turn.as_deref(),
                    x.source.as_str(),
                )
            })
            .collect()
    }

    #[test]
    fn motor_lanes_and_class_width() {
        let l = lanes_of(
            &[("lanes", "2")],
            edge(false, true, "residential", 2, "lanes"),
        );
        assert_eq!(
            short(&l),
            [
                (1, "auto", 3.0, None, "lanes"),
                (2, "auto", 3.0, None, "lanes")
            ]
        );
        let l = lanes_of(&[], edge(false, false, "primary", 0, "default"));
        assert_eq!(short(&l), [(1, "auto", 3.25, None, "default")]);
        assert_eq!(l[0].width_source, "default");
    }

    #[test]
    fn turn_lanes_add_lanes_and_arrows_only_at_the_way_end() {
        let tags = [("turn:lanes", "left|through|none")];
        let l = lanes_of(&tags, edge(false, true, "primary", 2, "lanes"));
        assert_eq!(
            short(&l),
            [
                (1, "auto", 3.25, Some("left"), "turn:lanes"),
                (2, "auto", 3.25, Some("through"), "turn:lanes"),
                (3, "auto", 3.25, None, "turn:lanes"),
            ]
        );
        let mid = EdgeLanes {
            way_end: false,
            ..edge(false, true, "primary", 3, "lanes")
        };
        assert!(lanes_of(&tags, mid).iter().all(|x| x.turn.is_none()));
        // the reverse edge reads the :backward tag only
        let l = lanes_of(
            &[("turn:lanes:backward", "left|right")],
            edge(true, false, "primary", 1, "lanes"),
        );
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn width_lanes() {
        let l = lanes_of(
            &[("width:lanes", "3.5|0")],
            edge(false, true, "primary", 2, "lanes"),
        );
        assert_eq!(
            (l[0].width_m, l[0].width_source.as_str()),
            (3.5, "width:lanes")
        );
        assert_eq!(
            (l[1].width_m, l[1].width_source.as_str()),
            (3.25, "default")
        );
    }

    #[test]
    fn bus_lanes_per_lane_by_count_and_by_side() {
        let l = lanes_of(
            &[("bus:lanes", "no|designated")],
            edge(false, true, "primary", 2, "lanes"),
        );
        assert_eq!((l[1].use_, l[1].source.as_str()), ("bus", "bus:lanes"));
        let l = lanes_of(
            &[("psv:lanes", "designated|no")],
            edge(false, true, "primary", 2, "lanes"),
        );
        assert_eq!((l[0].use_, l[0].source.as_str()), ("bus", "psv:lanes"));
        let l = lanes_of(
            &[("lanes:bus", "1")],
            edge(false, true, "primary", 3, "lanes"),
        );
        assert_eq!(short(&l)[2], (3, "bus", 3.25, None, "lanes:bus"));
        // busway:left is the right side of the reverse edge
        let l = lanes_of(
            &[("busway:left", "lane")],
            edge(true, false, "primary", 2, "lanes"),
        );
        assert_eq!((l[1].use_, l[1].source.as_str()), ("bus", "busway:left"));
        // a count that is not less than the lanes changes no lane
        let l = lanes_of(
            &[("lanes:bus", "2")],
            edge(false, true, "primary", 2, "lanes"),
        );
        assert!(l.iter().all(|x| x.use_ == "auto"));
    }

    #[test]
    fn share_busway() {
        let tags = [
            ("bus:lanes", "designated|designated"),
            ("cycleway:right", "share_busway"),
        ];
        let l = lanes_of(&tags, edge(false, true, "primary", 2, "lanes"));
        assert_eq!(l[0].use_, "bus");
        assert_eq!(
            (l[1].use_, l[1].source.as_str()),
            ("bus,bike", "bus:lanes + cycleway share_busway")
        );
    }

    #[test]
    fn bike_lanes() {
        // right of travel, not counted in lanes
        let l = lanes_of(
            &[("cycleway:right", "lane")],
            edge(false, true, "primary", 2, "lanes"),
        );
        assert_eq!(short(&l)[2], (3, "bike", 1.5, None, "cycleway:right"));
        // a one-way's cycleway:left goes with the traffic: lane -1
        let tags = [("cycleway:left", "lane"), ("cycleway:left:width", "1.2")];
        let l = lanes_of(&tags, edge(false, true, "primary", 1, "lanes"));
        assert_eq!(short(&l)[0], (-1, "bike", 1.2, None, "cycleway:left"));
        assert_eq!(l[0].width_source, "cycleway:left:width");
        // ... but not against it
        let tags = [("cycleway:left", "lane"), ("oneway:bicycle", "no")];
        assert_eq!(
            lanes_of(&tags, edge(false, true, "primary", 1, "lanes")).len(),
            1
        );
        // a two-way's cycleway:left is the reverse edge's right
        let l = lanes_of(
            &[("cycleway:left", "lane")],
            edge(true, false, "primary", 1, "lanes"),
        );
        assert_eq!(short(&l)[1], (2, "bike", 1.5, None, "cycleway:left"));
        // bicycle:lanes with a designated lane: no lane added
        let tags = [
            ("bicycle:lanes", "no|designated"),
            ("cycleway:right", "lane"),
        ];
        let l = lanes_of(&tags, edge(false, true, "primary", 2, "lanes"));
        assert_eq!(
            l.iter().map(|x| x.use_).collect::<Vec<_>>(),
            ["auto", "bike"]
        );
    }

    #[test]
    fn bus_only_edge() {
        let e = EdgeLanes {
            bus_only: true,
            shared_bike: true,
            ..edge(false, false, "service", 1, "default")
        };
        let l = lanes_of(&[], e);
        assert_eq!(short(&l), [(1, "bus,bike", 3.25, None, "access=bus")]);
    }

    #[test]
    fn inherit_from_the_road_it_continues() {
        let attrs = Arc::new(Attrs {
            highway: "primary".into(),
            name: None,
            maxspeed: None,
            surface: None,
            junction: None,
            layer: None,
            bridge: None,
            tunnel: None,
            service: None,
        });
        let e = |id, s, t, lanes| LaneEdge {
            edge_id: id,
            osm_id: id,
            is_reverse: false,
            oneway: true,
            attrs: attrs.clone(),
            lanes: Some(lanes),
            source: s,
            target: t,
        };
        // 1 (2 lanes, tagged) -> 2 -> 3, then 3 -> 4 and 3 -> 5 (a fork: nothing passes it)
        let edges = [
            e(1, 10, 11, 2),
            e(2, 11, 12, 1),
            e(3, 12, 13, 1),
            e(4, 13, 14, 1),
            e(5, 13, 15, 1),
        ];
        let got = inherit_lanes(&edges, &[true, false, false, false, false]);
        assert_eq!(got, HashMap::from([(2, 2), (3, 2)]));
    }
}
