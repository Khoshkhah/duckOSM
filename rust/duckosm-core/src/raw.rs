//! duckOSM's `raw.*` tables of an area: all nodes, ways and relations of the cut, with all their tags, as osmium's
//! `smart` cut gives them (`pbf::read_area_raw`). duckOSM's base-map layers (`features.*`) are made from them with
//! duckOSM's own SQL (docs/features.md). The OSM fixes and the sidewalk rule change `raw.ways` as in duckOSM.

use crate::fixes::{Elem, NewNode, OsmFixes, TagEdit, NEW_WAY_ID_MIN};
use crate::graph::Relation;
use crate::model::Way;
use crate::util::FxHashMap;

/// The tags in the order of the PBF file. No tags: an empty list (NULL in `raw.*`).
pub type RawTags = Vec<(String, String)>;

pub struct RawNode {
    pub id: i64,
    pub lon: f64,
    pub lat: f64,
    pub tags: RawTags,
}

pub struct RawWay {
    pub id: i64,
    pub tags: RawTags,
    pub refs: Vec<i64>,
}

/// `raw.nodes`, `raw.ways` and `raw.relations`, each sorted by id.
#[derive(Default)]
pub struct Raw {
    pub nodes: Vec<RawNode>,
    pub ways: Vec<RawWay>,
    pub relations: Vec<Relation>,
    /// The replication time of the region file's header (seconds since 1970), for the header of the cut file.
    pub timestamp: Option<i64>,
}

/// DuckDB `map_concat(tags, new)`: a key that is there gets the new value in its place, a new key goes at the end.
pub fn set_tags<'a>(tags: &mut RawTags, new: impl IntoIterator<Item = (&'a str, &'a str)>) {
    for (k, v) in new {
        match tags.iter_mut().find(|t| t.0 == k) {
            Some(t) => t.1 = v.to_string(),
            None => tags.push((k.to_string(), v.to_string())),
        }
    }
}

/// A tag edit (`extra.tag_edits`): set the tags, then delete the keys.
fn edit_tags(tags: &mut RawTags, e: &TagEdit) {
    set_tags(tags, e.set.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    tags.retain(|t| !e.delete.contains(&t.0));
}

impl Raw {
    fn way_mut(&mut self, id: i64) -> Option<&mut RawWay> {
        let i = self.ways.binary_search_by_key(&id, |w| w.id).ok()?;
        Some(&mut self.ways[i])
    }

    /// duckOSM `_raw_way_fixes`: remove ways, keep a part of a way, set tags, and add the new ways with their new
    /// nodes. A new way with an OSM node that is not in `raw.nodes` is not added (the rules file is global).
    pub fn apply_fixes(&mut self, fixes: &OsmFixes) {
        let tags_from: FxHashMap<i64, RawTags> = fixes
            .new_ways
            .iter()
            .filter_map(|n| n.tags_from)
            .filter_map(|id| Some((id, self.way_mut(id)?.tags.clone())))
            .collect();
        self.ways
            .retain(|w| !fixes.ways.iter().any(|r| r.remove && r.osm_id == w.id));
        for r in fixes.ways.iter().filter(|r| !r.remove) {
            let Some(w) = self.way_mut(r.osm_id) else {
                continue;
            };
            if let Some((a, b)) = r.keep_nodes {
                let pa = w.refs.iter().position(|&n| n == a);
                let pb = w.refs.iter().position(|&n| n == b);
                if let (Some(pa), Some(pb)) = (pa, pb) {
                    if pa < pb {
                        w.refs = w.refs[pa..=pb].to_vec();
                    }
                }
            }
            set_tags(
                &mut w.tags,
                r.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            );
        }
        // the tag edits of the page (`extra.tag_edits`): ways, nodes and relations, so crossings, signals and multipolygon buildings can change
        for e in &fixes.tag_edits {
            match e.elem {
                Elem::Way => {
                    if let Some(w) = self.way_mut(e.id) {
                        edit_tags(&mut w.tags, e);
                    }
                }
                Elem::Node => {
                    if let Ok(i) = self.nodes.binary_search_by_key(&e.id, |n| n.id) {
                        edit_tags(&mut self.nodes[i].tags, e);
                    }
                }
                Elem::Relation => {
                    if let Some(r) = self.relations.iter_mut().find(|r| r.id == e.id) {
                        edit_tags(&mut r.tags, e);
                    }
                }
            }
        }
        for n in &fixes.new_ways {
            // apply_to_ways has checked the ids
            if n.id < NEW_WAY_ID_MIN {
                continue;
            }
            let has = |id: &i64| self.nodes.binary_search_by_key(id, |x| x.id).is_ok();
            if n.nodes
                .iter()
                .any(|x| matches!(x, NewNode::Osm(id) if !has(id)))
            {
                continue;
            }
            let mut refs = Vec::new();
            for (i, node) in n.nodes.iter().enumerate() {
                refs.push(match node {
                    NewNode::Osm(id) => *id,
                    NewNode::At(p) => {
                        let id = n.id * 1000 + i as i64 + 1;
                        self.nodes.push(RawNode {
                            id,
                            lon: p.0,
                            lat: p.1,
                            tags: Vec::new(),
                        });
                        id
                    }
                });
            }
            let mut tags = n
                .tags_from
                .and_then(|id| tags_from.get(&id).cloned())
                .unwrap_or_default();
            set_tags(
                &mut tags,
                n.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            );
            self.ways.push(RawWay {
                id: n.id,
                tags,
                refs,
            });
        }
        self.nodes.sort_by_key(|x| x.id);
        self.ways.sort_by_key(|w| w.id);
    }

    /// The tags that the network steps set on their ways after the fixes (the sidewalk rule's `sidewalk=separate` and
    /// `duckosm:sidewalk=inferred`, the arrows of the fixes for each mode) go into `raw.ways`, as duckOSM's `UPDATE raw.ways
    /// ... map_concat` does: a changed value in its place, a new key at the end.
    pub fn sync_tags(&mut self, ways: &[Way]) {
        for w in ways {
            if let Some(r) = self.way_mut(w.id) {
                let new: Vec<(&str, &str)> = w
                    .all_tags()
                    .filter(|(k, v)| !r.tags.iter().any(|t| t.0 == *k && t.1 == *v))
                    .collect();
                set_tags(&mut r.tags, new);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_edits_on_ways_nodes_and_relations() {
        let t = |v: &[(&str, &str)]| -> RawTags {
            v.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let mut raw = Raw {
            nodes: vec![RawNode {
                id: 5,
                lon: 0.0,
                lat: 0.0,
                tags: t(&[("highway", "crossing")]),
            }],
            ways: vec![RawWay {
                id: 1,
                tags: t(&[("building", "yes"), ("roof:colour", "red")]),
                refs: vec![],
            }],
            relations: vec![Relation {
                id: 9,
                tags: t(&[("type", "multipolygon"), ("building", "yes")]),
                members: vec![],
            }],
            timestamp: None,
        };
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
                edit(Elem::Way, 1, &[("building:levels", "5")], &["roof:colour"]),
                edit(Elem::Node, 5, &[("crossing", "marked")], &[]),
                edit(Elem::Relation, 9, &[], &["building"]),
                edit(Elem::Way, 404, &[("a", "b")], &[]), // not in this area: nothing happens
            ],
            ..Default::default()
        };
        raw.apply_fixes(&fixes);
        assert_eq!(
            raw.ways[0].tags,
            t(&[("building", "yes"), ("building:levels", "5")])
        );
        assert_eq!(
            raw.nodes[0].tags,
            t(&[("highway", "crossing"), ("crossing", "marked")])
        );
        assert_eq!(raw.relations[0].tags, t(&[("type", "multipolygon")]));
    }

    #[test]
    fn set_tags_is_map_concat() {
        // DuckDB: map_concat(MAP {'a':'1','sidewalk':'no','b':'2'}, MAP {'sidewalk':'separate','duckosm:sidewalk':'inferred'})
        let mut t: RawTags = [("a", "1"), ("sidewalk", "no"), ("b", "2")]
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .to_vec();
        set_tags(
            &mut t,
            [("sidewalk", "separate"), ("duckosm:sidewalk", "inferred")],
        );
        let keys: Vec<&str> = t.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(keys, ["a", "sidewalk", "b", "duckosm:sidewalk"]);
        assert_eq!(t[1].1, "separate");
    }
}
