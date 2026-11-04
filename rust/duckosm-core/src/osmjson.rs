//! Read OSM JSON, the output format of the Overpass API (`[out:json]`, `out body`): the input of the browser
//! page from a place name (docs/browser.md, step B2). The result is the same as the PBF reader gives for the
//! same data: the highway ways and ferry lines sorted by id, the coordinates of the nodes that they use, and
//! the `type=restriction` relations.

use crate::boundary::Boundary;
use crate::filter;
use crate::geom::Pt;
use crate::graph::{MemberType, Relation};
use crate::model::{Tags, Way};
use crate::network::Coords;
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
    tags: BTreeMap<String, String>,
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

/// The data of an OSM JSON document.
pub struct OsmJson {
    pub ways: Vec<Way>,
    pub relations: Vec<Relation>,
    nodes: FxHashMap<i64, Pt>,
}

/// A coordinate as the PBF reader computes it: `1e-9 * nanodegrees` (osmpbf). The JSON number has at most 7
/// decimals, so `round(v * 1e9)` is the exact number of nanodegrees.
fn degrees(v: f64) -> f64 {
    1e-9 * ((v * 1e9).round() as i64) as f64
}

impl OsmJson {
    pub fn read(bytes: &[u8]) -> Result<OsmJson, String> {
        let doc: Doc = serde_json::from_slice(bytes).map_err(|e| format!("OSM JSON: {e}"))?;
        let (mut ways, mut relations, mut nodes) = (Vec::new(), Vec::new(), FxHashMap::default());
        for el in doc.elements {
            match el.kind.as_str() {
                "node" => {
                    if let (Some(lat), Some(lon)) = (el.lat, el.lon) {
                        nodes.insert(el.id, (degrees(lon), degrees(lat)));
                    }
                }
                "way" => {
                    let pairs = el.tags.iter().map(|(k, v)| (k.as_str(), v.as_str()));
                    if filter::is_kept_by_reader(&Tags::from_pairs(pairs.clone())) {
                        ways.push(Way::new(el.id, el.nodes, pairs));
                    }
                }
                "relation" if el.tags.get("type").map(String::as_str) == Some("restriction") => {
                    let members = el
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
                    relations.push(Relation {
                        id: el.id,
                        tags: el.tags.into_iter().collect(),
                        members,
                    });
                }
                _ => {}
            }
        }
        ways.sort_by_key(|w| w.id);
        relations.sort_by_key(|r| r.id);
        Ok(OsmJson {
            ways,
            relations,
            nodes,
        })
    }

    /// Keep the ways with a node inside the boundary (osmium's `complete_ways`, as `pbf::read_area`). Overpass
    /// cuts with the outer rings only, so this gives the same area as the cut of a region PBF.
    pub fn cut(&mut self, boundary: &Boundary) {
        let nodes = &self.nodes;
        self.ways.retain(|w| {
            w.refs
                .iter()
                .any(|n| nodes.get(n).is_some_and(|p| boundary.contains(p.0, p.1)))
        });
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
