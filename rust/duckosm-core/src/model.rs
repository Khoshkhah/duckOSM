//! The data types: OSM ways as read, edges and nodes as written.

use crate::geom::Pt;
use std::sync::Arc;

/// The OSM tags that the network step reads. Other tags are not kept. All the values of a way are
/// in one string, so the tags of a way use two small allocations, not one field for each key.
#[derive(Default, Clone, Debug)]
pub struct Tags {
    /// The key of each value and the end of the value in `text`.
    keys: Box<[(Key, u32)]>,
    text: Box<str>,
}

impl Tags {
    /// Keep the tags that the network step reads. Like `map_extract(tags, k)[1]` in duckOSM, the
    /// first value of a key wins.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Tags {
        // Collect on the stack first, then allocate the exact size one time. Growing and
        // shrinking a Vec for each way leaves small holes in the heap (WebAssembly never returns them).
        let mut found = [(Key::highway, ""); N_KEYS];
        let mut n = 0;
        for (k, v) in pairs {
            if let Some(key) = Key::of(k) {
                if !found[..n].iter().any(|&(x, _)| x == key) {
                    found[n] = (key, v);
                    n += 1;
                }
            }
        }
        Tags::build(&found[..n])
    }

    /// Set the value of an OSM key, and replace an old value. A key that is not read is not kept.
    pub fn set(&mut self, k: &str, v: &str) {
        let Some(key) = Key::of(k) else {
            return;
        };
        let mut found: Vec<(Key, &str)> = self.pairs().filter(|p| p.0 != key).collect();
        found.push((key, v));
        let new = Tags::build(&found);
        *self = new;
    }

    /// Remove an OSM key, with all its values (a tag edit of the page, `extra.tag_edits`).
    pub fn remove(&mut self, k: &str) {
        let Some(key) = Key::of(k) else {
            return;
        };
        let found: Vec<(Key, &str)> = self.pairs().filter(|p| p.0 != key).collect();
        let new = Tags::build(&found);
        *self = new;
    }

    fn build(found: &[(Key, &str)]) -> Tags {
        let mut text = String::with_capacity(found.iter().map(|f| f.1.len()).sum());
        let keys = found
            .iter()
            .map(|&(key, v)| {
                text.push_str(v);
                (key, text.len() as u32)
            })
            .collect();
        Tags {
            keys,
            text: text.into_boxed_str(),
        }
    }

    fn pairs(&self) -> impl Iterator<Item = (Key, &str)> {
        let mut start = 0;
        self.keys.iter().map(move |&(k, end)| {
            let v = &self.text[start..end as usize];
            start = end as usize;
            (k, v)
        })
    }

    /// The kept tags as (OSM key, value).
    pub fn osm_pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.pairs().map(|(k, v)| (k.osm(), v))
    }

    fn get(&self, key: Key) -> Option<&str> {
        self.pairs().find(|p| p.0 == key).map(|p| p.1)
    }
}

/// The keys that the network step reads: one accessor (`tags.highway()`) for each key.
macro_rules! tag_keys {
    ($($name:ident = $osm:literal),* $(,)?) => {
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Key {
            $($name),*
        }

        const N_KEYS: usize = [$(Key::$name),*].len();

        impl Key {
            fn of(k: &str) -> Option<Key> {
                match k {
                    $($osm => Some(Key::$name),)*
                    _ => None,
                }
            }

            fn osm(self) -> &'static str {
                match self {
                    $(Key::$name => $osm,)*
                }
            }
        }

        impl Tags {
            $(pub fn $name(&self) -> Option<&str> {
                self.get(Key::$name)
            })*
        }
    };
}

tag_keys! {
    highway = "highway",
    name = "name",
    maxspeed = "maxspeed",
    oneway = "oneway",
    oneway_bus = "oneway:bus",
    oneway_psv = "oneway:psv",
    junction = "junction",
    motorcar = "motorcar",
    motor_vehicle = "motor_vehicle",
    vehicle = "vehicle",
    access = "access",
    bus = "bus",
    psv = "psv",
    surface = "surface",
    service = "service",
    layer = "layer",
    bridge = "bridge",
    tunnel = "tunnel",
    lanes = "lanes",
    lanes_forward = "lanes:forward",
    lanes_backward = "lanes:backward",
    lanes_reversible = "lanes:reversible",
    route = "route",
    footway = "footway",
    conveying = "conveying",
    area = "area",
    indoor = "indoor",
    cycleway_left = "cycleway:left",
    cycleway_right = "cycleway:right",
    cycleway_both = "cycleway:both",
    segregated = "segregated",
    foot = "foot",
    bicycle = "bicycle",
    cycleway = "cycleway",
    sidewalk = "sidewalk",
    sidewalk_both = "sidewalk:both",
    sidewalk_left = "sidewalk:left",
    sidewalk_right = "sidewalk:right",
    motorroad = "motorroad",
    oneway_foot = "oneway:foot",
    oneway_bicycle = "oneway:bicycle",
}

/// An OSM way with a highway tag, in the node order of the file.
#[derive(Clone, Debug)]
pub struct Way {
    pub id: i64,
    pub refs: Vec<i64>,
    pub tags: Tags,
    /// The tags that the network step does not read: only for the `ways` table, which has all tags.
    pub other: OtherTags,
}

impl Way {
    /// A way with all its tags: the ones that the network step reads in `tags`, the others in `other`.
    pub fn new<'a>(
        id: i64,
        refs: Vec<i64>,
        pairs: impl IntoIterator<Item = (&'a str, &'a str)> + Clone,
    ) -> Way {
        Way {
            id,
            refs,
            tags: Tags::from_pairs(pairs.clone()),
            other: OtherTags::of(pairs),
        }
    }

    /// Set a tag, and replace an old value (the OSM fixes).
    pub fn set_tag(&mut self, k: &str, v: &str) {
        if Key::of(k).is_some() {
            self.tags.set(k, v);
        } else {
            self.other.set(k, v);
        }
    }

    /// Remove a tag (a tag edit of the page, `extra.tag_edits`).
    pub fn remove_tag(&mut self, k: &str) {
        if Key::of(k).is_some() {
            self.tags.remove(k);
        } else {
            self.other.remove(k);
        }
    }

    /// All tags of the way: the `tags` column of duckOSM's `ways` table.
    pub fn all_tags(&self) -> impl Iterator<Item = (&str, &str)> {
        self.tags.osm_pairs().chain(self.other.pairs())
    }
}

/// The tags of a way that [`Tags`] does not keep, in one string: key, value, key, value, … separated by
/// NUL (an OSM tag has no NUL). The first value of a key wins, as in [`Tags`].
#[derive(Default, Clone, Debug)]
pub struct OtherTags(Box<str>);

impl OtherTags {
    pub fn of<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> OtherTags {
        let mut s = String::new();
        let mut seen: Vec<&str> = Vec::new();
        for (k, v) in pairs {
            if Key::of(k).is_none() && !seen.contains(&k) {
                seen.push(k);
                if !s.is_empty() {
                    s.push('\0');
                }
                s.push_str(k);
                s.push('\0');
                s.push_str(v);
            }
        }
        OtherTags(s.into_boxed_str())
    }

    pub fn pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        let mut it = self.0.split('\0').filter(|_| !self.0.is_empty());
        std::iter::from_fn(move || Some((it.next()?, it.next()?)))
    }

    pub fn set(&mut self, k: &str, v: &str) {
        let rest: Vec<(&str, &str)> = self.pairs().filter(|p| p.0 != k).collect();
        *self = OtherTags::of(rest.into_iter().chain([(k, v)]));
    }

    pub fn remove(&mut self, k: &str) {
        let rest: Vec<(&str, &str)> = self.pairs().filter(|p| p.0 != k).collect();
        *self = OtherTags::of(rest);
    }
}

/// The columns of an edge that come from its way and are the same on all its edges.
/// The edges of a way share one copy, so an edge does not keep its own strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attrs {
    pub highway: String,
    pub name: Option<String>,
    pub maxspeed: Option<String>,
    pub surface: Option<String>,
    pub junction: Option<String>,
    pub layer: Option<String>,
    pub bridge: Option<String>,
    pub tunnel: Option<String>,
    pub service: Option<String>,
}

/// A row of the `edges` table.
#[derive(Clone, Debug)]
pub struct Edge {
    pub edge_id: i64,
    /// The number in `edge_ref` (see [`Edge::edge_ref`]).
    pub seq: Option<u32>,
    pub source: i64,
    pub target: i64,
    pub osm_id: i64,
    pub attrs: Arc<Attrs>,
    pub oneway: bool,
    pub lanes: Option<i32>,
    /// Not in `attrs`: the reverse edge of a bus lane gets "bus".
    pub access: Option<Arc<str>>,
    pub refs: Vec<i64>,
    pub geometry: Vec<Pt>,
    pub is_reverse: bool,
    pub length_m: f32,
    /// Cycling only: the edge is walked, not ridden. None in the other modes.
    pub dismount: Option<bool>,
    /// With the speed step: the speed in km/h (the writer then leaves out the raw `maxspeed`) and the
    /// travel time.
    pub maxspeed_kmh: Option<f32>,
    pub cost_s: Option<f32>,
    /// Walking: `walk_type`. Cycling: `cycle_type`. None in the driving mode, or without the step.
    pub ftype: Option<&'static str>,
}

impl Edge {
    /// `edge_ref`: `{osm_id}#{seq}{f|r}`.
    pub fn edge_ref(&self) -> Option<String> {
        let dir = if self.is_reverse { 'r' } else { 'f' };
        self.seq.map(|q| format!("{}#{q}{dir}", self.osm_id))
    }
}

/// A row of the `nodes` table. A virtual node has `node_id < 0`.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub node_id: i64,
    pub lon: f64,
    pub lat: f64,
}

/// The result of the network step for one mode.
#[derive(Default, Debug)]
pub struct Network {
    pub edges: Vec<Edge>,
    pub private_edges: Vec<Edge>,
    pub nodes: Vec<Node>,
    /// One row for each piece of each merged edge (only with `merge_segments`).
    pub edge_id_map: Vec<MapRow>,
    /// The edge graph and the turn restrictions (`build_graph`, `extract_restrictions`).
    pub graph: crate::graph::Graph,
    pub stats: Stats,
    /// The ways of the mode after the road filter and the fixes: the tables `ways` and `way_nodes`
    /// (`Options::ways_tables`). A way that is one-way against its drawing has its nodes turned round.
    pub ways: Vec<crate::filter::ModeWay>,
}

/// A row of `edge_id_map`: the piece `old_edge_id` is part of the merged edge `new_edge_id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapRow {
    pub old_edge_id: i64,
    pub new_edge_id: i64,
    pub seq: i32,
    pub is_reverse: bool,
    pub osm_id: i64,
}

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub ways_read: usize,
    pub ways_kept: usize,
    pub forward_edges: usize,
    pub self_loops_split: usize,
    pub parallel_split: usize,
    pub antiparallel_split: usize,
    pub merged_chains: usize,
    pub connectors: usize,
    pub merged_pieces: usize,
    pub private_edges: usize,
    pub dropped_by_component_filter: usize,
    /// The per-mode rules of the OSM fixes that matched a way of this mode.
    pub osm_fixes: usize,
}
