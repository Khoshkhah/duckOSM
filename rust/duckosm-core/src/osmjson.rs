//! Read OSM JSON, the output format of the Overpass API (`[out:json]`, `out body`): the input of the browser
//! page from a place name (docs/browser.md, step B2). The result is the same as the PBF reader gives for the
//! same data: the highway ways and ferry lines sorted by id, the coordinates of the nodes that they use, and
//! the `type=restriction` relations. With `raw`, also duckOSM's `raw.*` of the answer (all nodes, ways and
//! relations, with their tags in the order of the answer), cut to the boundary as osmium's `smart` cut does
//! (`pbf::read_area_raw`): the input of the base-map layers.

use crate::boundary::Boundary;
use crate::filter;
use crate::geom::Pt;
use crate::graph::{MemberType, Relation};
use crate::model::{Tags, Way};
use crate::network::Coords;
use crate::raw::{Raw, RawNode, RawTags, RawWay};
use crate::util::{FxHashMap, FxHashSet};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Doc {
    elements: Vec<Element>,
}

#[derive(Deserialize)]
struct Element {
    #[serde(rename = "type")]
    kind: String,
    id: i64,
    lat: Option<f64>,
    lon: Option<f64>,
    #[serde(default)]
    nodes: Vec<i64>,
    #[serde(default)]
    tags: Ordered,
    #[serde(default)]
    members: Vec<Member>,
}

#[derive(Deserialize)]
struct Member {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "ref")]
    id: i64,
    role: String,
}

/// The tags of an element in the order of the document (a `BTreeMap` sorts them).
#[derive(Default)]
struct Ordered(RawTags);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Ordered, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object of tags")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut m: A,
            ) -> Result<Ordered, A::Error> {
                let mut v = Vec::new();
                while let Some(kv) = m.next_entry()? {
                    v.push(kv);
                }
                Ok(Ordered(v))
            }
        }
        d.deserialize_map(V)
    }
}

/// The data of an OSM JSON document.
pub struct OsmJson {
    pub ways: Vec<Way>,
    pub relations: Vec<Relation>,
    nodes: FxHashMap<i64, Pt>,
    /// `raw.*` of the document, when [`OsmJson::read`] was asked for it; [`OsmJson::cut`] cuts it.
    pub raw: Option<Raw>,
}

/// A coordinate as the PBF reader computes it: `1e-9 * nanodegrees` (osmpbf). The JSON number has at most 7
/// decimals, so `round(v * 1e9)` is the exact number of nanodegrees.
fn degrees(v: f64) -> f64 {
    1e-9 * ((v * 1e9).round() as i64) as f64
}

impl OsmJson {
    /// Read the document; `raw`: also keep `raw.*`.
    pub fn read(bytes: &[u8], raw: bool) -> Result<OsmJson, String> {
        let doc: Doc = serde_json::from_slice(bytes).map_err(|e| format!("OSM JSON: {e}"))?;
        let (mut ways, mut relations, mut nodes) = (Vec::new(), Vec::new(), FxHashMap::default());
        let mut all = Raw::default();
        for el in doc.elements {
            let tags = el.tags.0;
            match el.kind.as_str() {
                "node" => {
                    if let (Some(lat), Some(lon)) = (el.lat, el.lon) {
                        let p = (degrees(lon), degrees(lat));
                        nodes.insert(el.id, p);
                        if raw {
                            all.nodes.push(RawNode {
                                id: el.id,
                                lon: p.0,
                                lat: p.1,
                                tags,
                            });
                        }
                    }
                }
                "way" => {
                    // the network reads the tags sorted, as before raw.* (a BTreeMap)
                    let sorted: BTreeMap<&str, &str> =
                        tags.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    let pairs = sorted.iter().map(|(k, v)| (*k, *v));
                    if filter::is_kept_by_reader(&Tags::from_pairs(pairs.clone())) {
                        ways.push(Way::new(el.id, el.nodes.clone(), pairs));
                    }
                    if raw {
                        all.ways.push(RawWay {
                            id: el.id,
                            tags,
                            refs: el.nodes,
                        });
                    }
                }
                "relation" => {
                    let restriction = tags.iter().any(|t| t.0 == "type" && t.1 == "restriction");
                    if !restriction && !raw {
                        continue;
                    }
                    let members: Vec<_> = el
                        .members
                        .into_iter()
                        .filter_map(|m| {
                            let t = match m.kind.as_str() {
                                "node" => MemberType::Node,
                                "way" => MemberType::Way,
                                "relation" => MemberType::Relation,
                                _ => return None,
                            };
                            Some((m.id, t, m.role))
                        })
                        .collect();
                    let r = Relation {
                        id: el.id,
                        tags,
                        members,
                    };
                    if raw {
                        all.relations.push(r.clone());
                    }
                    if restriction {
                        // the network reads the tags sorted, as before raw.* (a BTreeMap)
                        let mut r = r;
                        r.tags.sort();
                        relations.push(r);
                    }
                }
                _ => {}
            }
        }
        ways.sort_by_key(|w| w.id);
        relations.sort_by_key(|r| r.id);
        all.nodes.sort_by_key(|n| n.id);
        all.ways.sort_by_key(|w| w.id);
        all.relations.sort_by_key(|r| r.id);
        Ok(OsmJson {
            ways,
            relations,
            nodes,
            raw: raw.then_some(all),
        })
    }

    /// Keep the ways with a node inside the boundary (osmium's `complete_ways`, as `pbf::read_area`). Overpass
    /// cuts with the outer rings only, so this gives the same area as the cut of a region PBF. `raw.*` gets the
    /// `smart` cut of `pbf::read_area_raw`.
    pub fn cut(&mut self, boundary: &Boundary) {
        let nodes = &self.nodes;
        let inside = |n: &i64| nodes.get(n).is_some_and(|p| boundary.contains(p.0, p.1));
        self.ways.retain(|w| w.refs.iter().any(inside));
        if let Some(raw) = self.raw.as_mut() {
            smart_cut(raw, &inside);
        }
    }

    /// The coordinates of the nodes that `ways` use (pass 2 of the PBF reader).
    pub fn coords(&self, ways: &[Way]) -> Coords {
        let needed: FxHashSet<i64> = ways.iter().flat_map(|w| w.refs.iter().copied()).collect();
        Coords::new(
            needed
                .into_iter()
                .filter_map(|id| self.nodes.get(&id).map(|&p| (id, p)))
                .collect(),
        )
    }
}

/// osmium's `smart` cut of `raw` (as `pbf::read_area_raw`): the nodes inside, the ways with a node inside, the
/// relations with a member node inside or a member way kept and their parent relations, all member ways and nodes
/// of these multipolygons, and the nodes of the kept ways.
fn smart_cut(raw: &mut Raw, inside: &dyn Fn(&i64) -> bool) {
    let mut kept: FxHashSet<i64> = raw
        .ways
        .iter()
        .filter(|w| w.refs.iter().any(inside))
        .map(|w| w.id)
        .collect();
    let direct = |r: &Relation, kept: &FxHashSet<i64>| {
        r.members.iter().any(|m| match m.1 {
            MemberType::Node => inside(&m.0),
            MemberType::Way => kept.contains(&m.0),
            MemberType::Relation => false,
        })
    };
    let mut chosen: FxHashSet<i64> = raw
        .relations
        .iter()
        .filter(|r| direct(r, &kept))
        .map(|r| r.id)
        .collect();
    loop {
        let parents: Vec<i64> = raw
            .relations
            .iter()
            .filter(|r| !chosen.contains(&r.id))
            .filter(|r| {
                r.members
                    .iter()
                    .any(|m| m.1 == MemberType::Relation && chosen.contains(&m.0))
            })
            .map(|r| r.id)
            .collect();
        if parents.is_empty() {
            break;
        }
        chosen.extend(parents);
    }
    let mut more_nodes = FxHashSet::default();
    let mut more_ways = Vec::new();
    for r in raw.relations.iter().filter(|r| {
        r.tags
            .iter()
            .any(|t| t.0 == "type" && t.1 == "multipolygon")
            && direct(r, &kept)
    }) {
        for m in &r.members {
            match m.1 {
                MemberType::Way => more_ways.push(m.0),
                MemberType::Node => {
                    more_nodes.insert(m.0);
                }
                MemberType::Relation => {}
            }
        }
    }
    kept.extend(more_ways);
    raw.ways.retain(|w| kept.contains(&w.id));
    raw.relations.retain(|r| chosen.contains(&r.id));
    let refs: FxHashSet<i64> = raw
        .ways
        .iter()
        .flat_map(|w| w.refs.iter().copied())
        .collect();
    raw.nodes
        .retain(|n| inside(&n.id) || refs.contains(&n.id) || more_nodes.contains(&n.id));
}
