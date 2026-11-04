//! The OSM fixes of duckOSM (the config key `osm_overrides`): local corrections of known OSM errors,
//! one rule for each way. The rules act at two points, as in duckOSM:
//! - before any step, on the ways as read (`remove`, `keep_nodes`, `tags`, `new_ways`;
//!   duckOSM `DuckOSM._raw_way_fixes`);
//! - for each mode, after the road filter (`oneway`, the lanes, `layer`, `exclude_modes`;
//!   duckOSM `processors/osm_overrides.py`).
//!
//! The rules for the arrows of the lanes (`turn_lanes_*`) and `turn_restrictions` are not part of the
//! network step. The reader keeps only highway ways and ferry lines, so a `tags` rule on another way
//! has no effect here.

use crate::building_faces::FaceEdit;
use crate::filter::{Mode, ModeWay};
use crate::geom::Pt;
use crate::model::Way;
use crate::network::Coords;
use crate::util::FxHashMap;
use std::sync::Arc;

/// The lowest id of a new way (duckOSM `NEW_WAY_ID_MIN`): far above every OSM way id.
pub const NEW_WAY_ID_MIN: i64 = 9_000_000_000_000;

/// One rule of the `overrides` list.
#[derive(Clone, Debug, Default)]
pub struct WayFix {
    pub osm_id: i64,
    /// Remove the way from the build.
    pub remove: bool,
    /// Keep only the part of the way from this node to that node.
    pub keep_nodes: Option<(i64, i64)>,
    /// OSM tags to set on the way.
    pub tags: Vec<(String, String)>,
    pub oneway: Option<bool>,
    /// The lanes in each direction (both directions).
    pub lanes: Option<i32>,
    pub lanes_forward: Option<i32>,
    pub lanes_backward: Option<i32>,
    pub layer: Option<i32>,
    /// Remove the way from the networks of these modes.
    pub exclude_modes: Vec<Mode>,
    /// The painted arrows (`turn_lanes_forward` / `turn_lanes_backward`) as the raw tags
    /// `turn:lanes:forward` / `turn:lanes:backward` (see [`OsmFixes::set_arrows`]).
    pub arrows: Vec<(String, String)>,
}

/// A node of a new way: an OSM node, or a new node at (longitude, latitude).
#[derive(Clone, Debug)]
pub enum NewNode {
    Osm(i64),
    At(Pt),
}

/// One way of the `new_ways` list: a way that OSM does not have.
#[derive(Clone, Debug)]
pub struct NewWay {
    pub id: i64,
    pub nodes: Vec<NewNode>,
    pub tags: Vec<(String, String)>,
    /// Copy the OSM tags of this way (before any fix changes them). `tags` has priority.
    pub tags_from: Option<i64>,
}

/// One rule of the `turn_restrictions` list: a turn restriction that OSM does not have.
#[derive(Clone, Debug)]
pub struct TurnFix {
    /// `-(position in the list)`, from -1: the ids of duckOSM, clear of the OSM relation ids.
    pub id: i64,
    pub restriction: String,
    pub from_way: i64,
    pub via_node: i64,
    pub to_way: i64,
}

/// What a tag edit changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elem {
    Way,
    Node,
    Relation,
}

/// One rule of `extra.tag_edits`: set and delete tags of a way, a node or a relation. duckOSM reads only `overrides`, so it
/// ignores the key `extra`: this is an extension of duckosm-rust. The rule acts on the network ways and on `raw.*`, so the base-map
/// layers (`features.*`) see the edited tags too. The tags are set first, then deleted: a key in both is deleted.
#[derive(Clone, Debug)]
pub struct TagEdit {
    pub elem: Elem,
    pub id: i64,
    pub set: Vec<(String, String)>,
    pub delete: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct OsmFixes {
    pub ways: Vec<WayFix>,
    pub new_ways: Vec<NewWay>,
    pub turn_restrictions: Vec<TurnFix>,
    pub tag_edits: Vec<TagEdit>,
    /// `extra.building_faces`: the type of a wall of a building (`building_faces.rs`).
    pub face_edits: Vec<FaceEdit>,
}

impl OsmFixes {
    pub fn is_empty(&self) -> bool {
        self.ways.is_empty() && self.new_ways.is_empty() && self.tag_edits.is_empty()
    }

    /// Before pass 2 (the coordinates): remove ways, keep a part of a way, set tags, and add the new
    /// ways. A new node of a new way gets the id `way id * 1000 + position` (position from 1).
    /// `ways` stays sorted by id.
    pub fn apply_to_ways(&self, ways: &mut Vec<Way>) -> Result<(), String> {
        if self.is_empty() {
            return Ok(());
        }
        // tags_from copies the tags before any fix changes them
        let tags_from: FxHashMap<i64, Way> = self
            .new_ways
            .iter()
            .filter_map(|n| n.tags_from)
            .filter_map(|id| find(ways, id).map(|w| (id, w.clone())))
            .collect();
        let removed: Vec<i64> = self
            .ways
            .iter()
            .filter(|r| r.remove)
            .map(|r| r.osm_id)
            .collect();
        ways.retain(|w| !removed.contains(&w.id));
        for r in self.ways.iter().filter(|r| !r.remove) {
            let Some(w) = find_mut(ways, r.osm_id) else {
                continue; // one global file: most rules are for other areas
            };
            if let Some((a, b)) = r.keep_nodes {
                // DuckDB refs[list_position(a) : list_position(b)]: the first match, both ends kept
                let pa = w.refs.iter().position(|&n| n == a);
                let pb = w.refs.iter().position(|&n| n == b);
                if let (Some(pa), Some(pb)) = (pa, pb) {
                    if pa < pb {
                        w.refs = w.refs[pa..=pb].to_vec();
                    }
                }
            }
            for (k, v) in &r.tags {
                w.set_tag(k, v);
            }
        }
        for e in self.tag_edits.iter().filter(|e| e.elem == Elem::Way) {
            let Some(w) = find_mut(ways, e.id) else {
                continue;
            };
            for (k, v) in &e.set {
                w.set_tag(k, v);
            }
            for k in &e.delete {
                w.remove_tag(k);
            }
        }
        let mut seen = Vec::new();
        for n in &self.new_ways {
            if n.id < NEW_WAY_ID_MIN || seen.contains(&n.id) {
                return Err(format!(
                    "osm_overrides new_ways: id {} is below {NEW_WAY_ID_MIN} or used two times",
                    n.id
                ));
            }
            seen.push(n.id);
            let src = n.tags_from.and_then(|id| tags_from.get(&id));
            let own = n.tags.iter().map(|(k, v)| (k.as_str(), v.as_str()));
            let pairs: Vec<(&str, &str)> = own
                .chain(src.into_iter().flat_map(|w| w.all_tags()))
                .collect();
            let refs = n
                .nodes
                .iter()
                .enumerate()
                .map(|(i, node)| match node {
                    NewNode::Osm(id) => *id,
                    NewNode::At(_) => n.id * 1000 + i as i64 + 1,
                })
                .collect();
            ways.push(Way::new(n.id, refs, pairs.iter().copied()));
        }
        ways.sort_by_key(|w| w.id);
        Ok(())
    }

    /// After pass 2: add the coordinates of the new nodes. A new way with an OSM node that is not in
    /// the file is removed: the rules file is global, so the way is in another area.
    pub fn add_new_nodes(&self, ways: &mut Vec<Way>, coords: &mut Coords) {
        let mut absent = Vec::new();
        let mut new_nodes = Vec::new();
        for n in &self.new_ways {
            let osm_nodes = n.nodes.iter().filter_map(|x| match x {
                NewNode::Osm(id) => Some(id),
                NewNode::At(_) => None,
            });
            if osm_nodes.clone().any(|id| coords.get(id).is_none()) {
                absent.push(n.id);
                continue;
            }
            for (i, node) in n.nodes.iter().enumerate() {
                if let NewNode::At(p) = node {
                    new_nodes.push((n.id * 1000 + i as i64 + 1, *p));
                }
            }
        }
        ways.retain(|w| !absent.contains(&w.id));
        coords.extend(new_nodes);
    }

    /// After a mode is built: the arrows of a rule become raw tags of its way, when this mode kept the way
    /// (duckOSM `OSMOverrideProcessor`: `UPDATE raw.ways` in each mode, after that mode made its `ways`).
    /// So the `ways` tables of the modes after it show them, as in duckOSM; the lane steps read them there.
    pub fn set_arrows(&self, ways: &mut [Way], kept: &[ModeWay], mode: Mode) {
        for r in self
            .ways
            .iter()
            .filter(|r| !r.arrows.is_empty() && !r.exclude_modes.contains(&mode))
        {
            if kept.iter().any(|w| w.id == r.osm_id) {
                if let Ok(i) = ways.binary_search_by_key(&r.osm_id, |w| w.id) {
                    for (k, v) in &r.arrows {
                        ways[i].set_tag(k, v);
                    }
                }
            }
        }
    }

    /// For one mode, after the road filter. Returns the number of rules that matched a way.
    pub fn apply_to_mode(&self, mw: &mut Vec<ModeWay>, mode: Mode) -> usize {
        let mut applied = 0;
        for r in &self.ways {
            if r.exclude_modes.contains(&mode) {
                let before = mw.len();
                mw.retain(|w| w.id != r.osm_id);
                applied += (mw.len() < before) as usize;
                continue;
            }
            let sets = r.oneway.is_some()
                || r.lanes.is_some()
                || r.lanes_forward.is_some()
                || r.lanes_backward.is_some()
                || r.layer.is_some();
            let mut matched = false;
            for w in mw.iter_mut().filter(|w| sets && w.id == r.osm_id) {
                matched = true;
                if let Some(b) = r.oneway {
                    w.oneway = b;
                }
                if let Some(n) = r.lanes {
                    // a one-way way keeps no lanes against it, unless this rule makes it two-way
                    w.lanes_bwd = if r.oneway == Some(false) || w.lanes_bwd.is_some() {
                        Some(n)
                    } else {
                        None
                    };
                    w.lanes_fwd = Some(n);
                }
                if let Some(n) = r.lanes_forward {
                    w.lanes_fwd = Some(n);
                }
                if let Some(n) = r.lanes_backward {
                    w.lanes_bwd = Some(n);
                }
                if let Some(l) = r.layer {
                    Arc::make_mut(&mut w.attrs).layer = Some(l.to_string());
                }
            }
            applied += matched as usize;
        }
        applied
    }
}

fn find(ways: &[Way], id: i64) -> Option<&Way> {
    ways.binary_search_by_key(&id, |w| w.id)
        .ok()
        .map(|i| &ways[i])
}

fn find_mut(ways: &mut [Way], id: i64) -> Option<&mut Way> {
    ways.binary_search_by_key(&id, |w| w.id)
        .ok()
        .map(|i| &mut ways[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::mode_way;

    fn way(id: i64, refs: &[i64], tags: &[(&str, &str)]) -> Way {
        Way::new(id, refs.to_vec(), tags.iter().copied())
    }

    #[test]
    fn raw_fixes_and_new_ways() {
        let mut ways = vec![
            way(1, &[10, 11, 12, 13], &[("highway", "pedestrian")]),
            way(2, &[13, 14], &[("highway", "residential"), ("name", "A")]),
            way(3, &[14, 15], &[("highway", "service")]),
        ];
        let fixes = OsmFixes {
            ways: vec![
                WayFix {
                    osm_id: 1,
                    keep_nodes: Some((11, 13)),
                    tags: vec![("highway".into(), "unclassified".into())],
                    ..Default::default()
                },
                WayFix {
                    osm_id: 3,
                    remove: true,
                    ..Default::default()
                },
                WayFix {
                    osm_id: 99, // not in this area
                    remove: true,
                    ..Default::default()
                },
            ],
            new_ways: vec![
                NewWay {
                    id: NEW_WAY_ID_MIN,
                    nodes: vec![NewNode::Osm(12), NewNode::At((7.0, 43.0))],
                    tags: vec![("name".into(), "B".into())],
                    tags_from: Some(2),
                },
                NewWay {
                    id: NEW_WAY_ID_MIN + 1, // node 777 is not in the file
                    nodes: vec![NewNode::Osm(777), NewNode::Osm(12)],
                    tags: vec![],
                    tags_from: None,
                },
            ],
            turn_restrictions: vec![],
            tag_edits: vec![],
            face_edits: vec![],
        };
        fixes.apply_to_ways(&mut ways).unwrap();
        assert_eq!(
            ways.iter().map(|w| w.id).collect::<Vec<_>>(),
            [1, 2, NEW_WAY_ID_MIN, NEW_WAY_ID_MIN + 1]
        );
        assert_eq!(ways[0].refs, [11, 12, 13]);
        assert_eq!(ways[0].tags.highway(), Some("unclassified"));
        let new = &ways[2];
        assert_eq!(new.refs, [12, NEW_WAY_ID_MIN * 1000 + 2]);
        assert_eq!(
            (new.tags.highway(), new.tags.name()),
            (Some("residential"), Some("B"))
        );

        let mut coords = Coords::new((10..15).map(|n| (n, (n as f64, 0.0))).collect());
        fixes.add_new_nodes(&mut ways, &mut coords);
        assert_eq!(ways.len(), 3);
        assert_eq!(coords.get(&(NEW_WAY_ID_MIN * 1000 + 2)), Some(&(7.0, 43.0)));

        let bad = OsmFixes {
            new_ways: vec![NewWay {
                id: 5,
                nodes: vec![],
                tags: vec![],
                tags_from: None,
            }],
            ..Default::default()
        };
        assert!(bad.apply_to_ways(&mut ways).is_err());
    }

    #[test]
    fn tag_edits_on_ways() {
        let mut ways = vec![
            way(
                1,
                &[10, 11],
                &[
                    ("highway", "residential"),
                    ("sidewalk", "no"),
                    ("roof:shape", "flat"),
                ],
            ),
            way(2, &[11, 12], &[("highway", "service")]),
        ];
        let edit = |elem, id, set: &[(&str, &str)], delete: &[&str]| TagEdit {
            elem,
            id,
            set: set
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            delete: delete.iter().map(|k| k.to_string()).collect(),
        };
        let fixes = OsmFixes {
            tag_edits: vec![
                edit(
                    Elem::Way,
                    1,
                    &[("sidewalk", "both"), ("roof:shape", "gabled")],
                    &["roof:shape", "absent"],
                ),
                edit(Elem::Way, 404, &[("a", "b")], &[]), // not in this area
                edit(Elem::Node, 1, &[("a", "b")], &[]),  // a node: the network ways do not change
            ],
            ..Default::default()
        };
        assert!(!fixes.is_empty());
        fixes.apply_to_ways(&mut ways).unwrap();
        let tags: Vec<_> = ways[0].all_tags().collect();
        assert!(tags.contains(&("sidewalk", "both")), "{tags:?}");
        assert!(
            !tags.iter().any(|t| t.0 == "roof:shape"),
            "set, then deleted: {tags:?}"
        );
        assert_eq!(ways[0].tags.highway(), Some("residential"));
        assert_eq!(ways[1].all_tags().count(), 1);
    }

    #[test]
    fn mode_fixes() {
        let ways = [
            way(
                1,
                &[1, 2],
                &[("highway", "primary"), ("oneway", "yes"), ("lanes", "2")],
            ),
            way(2, &[2, 3], &[("highway", "primary"), ("lanes", "4")]),
            way(3, &[3, 4], &[("highway", "residential"), ("layer", "-1")]),
        ];
        let mut mw: Vec<ModeWay> = ways
            .iter()
            .filter_map(|w| mode_way(w, Mode::Driving, false))
            .collect();
        assert_eq!((mw[0].lanes_fwd, mw[0].lanes_bwd), (Some(2), None));
        let fixes = OsmFixes {
            ways: vec![
                WayFix {
                    osm_id: 1,
                    lanes: Some(1),
                    ..Default::default()
                },
                WayFix {
                    osm_id: 2,
                    oneway: Some(true),
                    lanes_backward: Some(3),
                    ..Default::default()
                },
                WayFix {
                    osm_id: 3,
                    layer: Some(0),
                    ..Default::default()
                },
                WayFix {
                    osm_id: 3,
                    exclude_modes: vec![Mode::Walking],
                    ..Default::default()
                },
            ],
            new_ways: vec![],
            turn_restrictions: vec![],
            tag_edits: vec![],
            face_edits: vec![],
        };
        assert_eq!(fixes.apply_to_mode(&mut mw, Mode::Driving), 3);
        assert_eq!((mw[0].lanes_fwd, mw[0].lanes_bwd), (Some(1), None));
        assert!(mw[1].oneway);
        assert_eq!(mw[1].lanes_bwd, Some(3));
        assert_eq!(mw[2].attrs.layer.as_deref(), Some("0"));
        let mut walk: Vec<ModeWay> = ways
            .iter()
            .filter_map(|w| mode_way(w, Mode::Walking, false))
            .collect();
        let n = walk.len();
        fixes.apply_to_mode(&mut walk, Mode::Walking);
        assert_eq!(walk.len(), n - 1);
    }
}
