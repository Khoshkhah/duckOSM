//! duckOSM's `raw.*` tables of an area: all nodes, ways and relations of the cut, with all their tags, as osmium's
//! `smart` cut gives them (`pbf::read_area_raw`). duckOSM's base-map layers (`features.*`) are made from them with
//! duckOSM's own SQL (docs/features.md). The OSM fixes and the sidewalk rule change `raw.ways` as in duckOSM.

use crate::fixes::{Elem, NewNode, OsmFixes, TagEdit, NEW_WAY_ID_MIN};
use crate::graph::{MemberType, Relation};
use crate::model::Way;
use crate::util::FxHashMap;

/// The tags in the order of the PBF file. No tags: an empty list (NULL in `raw.*`).
pub type RawTags = Vec<(String, String)>;

/// A receiver of the elements of duckOSM's `raw.*` tables: all nodes, ways and relations of a file, with all their
/// tags, in the order of the file (the load step, docs/design/streaming_load.md). The tags come in the order of the
/// file; no tags is an empty iterator; a member without a role has the role "".
pub trait RawSink {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>);
    fn way<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        refs: &mut dyn Iterator<Item = i64>,
    );
    fn relation<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    );
}

#[derive(Clone)]
pub struct RawNode {
    pub id: i64,
    pub lon: f64,
    pub lat: f64,
    pub tags: RawTags,
}

#[derive(Clone)]
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
    /// Keep only the elements whose ids are in `read` (sorted, as [`RawPatch::ids`] gives them) and the new elements of the fixes.
    pub fn retain_touched(&mut self, read: &[Vec<i64>; 3]) {
        let keep = |k: usize, id: i64| id >= NEW_WAY_ID_MIN || read[k].binary_search(&id).is_ok();
        self.nodes.retain(|n| keep(0, n.id));
        self.ways.retain(|w| keep(1, w.id));
        self.relations.retain(|r| keep(2, r.id));
        self.nodes.shrink_to_fit();
        self.ways.shrink_to_fit();
        self.relations.shrink_to_fit();
    }

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
                sync_way(&mut r.tags, w);
            }
        }
    }
}

/// [`Raw::sync_tags`] for one way: the tags of the network's way `w` go into the raw tags `tags`.
pub fn sync_way(tags: &mut RawTags, w: &Way) {
    let new: Vec<(&str, &str)> = w
        .all_tags()
        .filter(|(k, v)| !tags.iter().any(|t| t.0 == *k && t.1 == *v))
        .collect();
    set_tags(tags, new);
}

/// What [`read_part`] keeps of an area file.
#[cfg(feature = "pbf")]
pub struct Want<'a> {
    /// The relations to keep, by id and tags. Their member ways are also kept.
    pub relation: &'a (dyn Fn(i64, &[(&str, &str)]) -> bool + Sync),
    /// The ways to keep, by id and tags.
    pub way: &'a (dyn Fn(i64, &[(&str, &str)]) -> bool + Sync),
    /// The nodes to keep, by id.
    pub node: &'a (dyn Fn(i64) -> bool + Sync),
    /// Also keep the nodes of the kept ways.
    pub way_nodes: bool,
    /// The tags to keep of a kept node (0) or way (1), by id and key. A member way of a kept relation keeps all its tags.
    pub tag: &'a (dyn Fn(usize, i64, &str) -> bool + Sync),
}

/// A part of the `raw.*` of an area file (the output of step 0, `cutstream`): the elements that `want` keeps, each
/// list sorted by id. Three passes read the file: the relations, the ways, the nodes. Only the calling thread reads,
/// all threads decode (an OPFS file in the browser belongs to one thread).
#[cfg(feature = "pbf")]
pub fn read_part<R: std::io::Read>(
    open: impl Fn() -> Result<R, String>,
    want: &Want,
) -> Result<Raw, String> {
    use crate::cutstream::each_block;
    use crate::pbfraw::decode_block_tags;
    use crate::util::FxHashSet;
    struct Part<'w> {
        keep: Option<&'w FxHashSet<i64>>,
        want: Option<&'w Want<'w>>,
        raw: Raw,
    }
    impl RawSink for Part<'_> {
        fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
            let want = self.want.unwrap();
            if (want.node)(id) || self.keep.is_some_and(|k| k.contains(&id)) {
                let tags = tags.filter(|t| (want.tag)(0, id, t.0)).map(|(k, v)| (k.to_string(), v.to_string())).collect();
                self.raw.nodes.push(RawNode { id, lon, lat, tags });
            }
        }
        fn way<'a>(
            &mut self,
            id: i64,
            tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            refs: &mut dyn Iterator<Item = i64>,
        ) {
            let tags: Vec<(&str, &str)> = tags.collect();
            let want = self.want.unwrap();
            let member = self.keep.is_some_and(|k| k.contains(&id));
            if member || (want.way)(id, &tags) {
                let tags = tags
                    .iter()
                    .filter(|t| member || (want.tag)(1, id, t.0))
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                self.raw.ways.push(RawWay { id, tags, refs: refs.collect() });
            }
        }
        fn relation<'a>(
            &mut self,
            id: i64,
            tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
        ) {
            let tags: Vec<(&str, &str)> = tags.collect();
            if (self.want.unwrap().relation)(id, &tags) {
                self.raw.relations.push(Relation {
                    id,
                    tags: tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                    members: members.map(|(m, t, r)| (m, t, r.to_string())).collect(),
                });
            }
        }
    }
    let pass = |kind: usize, keep: Option<&FxHashSet<i64>>| -> Result<Raw, String> {
        let mut types = [false; 3];
        types[kind] = true;
        let mut all = Raw::default();
        each_block(
            &mut open()?,
            |b, buf| {
                let mut p = Part { keep, want: Some(want), raw: Raw::default() };
                decode_block_tags(b, types, types, buf, &mut p)?;
                Ok(p.raw)
            },
            |mut r, _| {
                all.nodes.append(&mut r.nodes);
                all.ways.append(&mut r.ways);
                all.relations.append(&mut r.relations);
                Ok(true)
            },
        )?;
        Ok(all)
    };
    let mut raw = pass(2, None)?;
    let members: FxHashSet<i64> = raw
        .relations
        .iter()
        .flat_map(|r| r.members.iter().filter(|m| m.1 == MemberType::Way).map(|m| m.0))
        .collect();
    raw.ways = pass(1, Some(&members))?.ways;
    drop(members);
    let refs: FxHashSet<i64> = if want.way_nodes {
        raw.ways.iter().flat_map(|w| w.refs.iter().copied()).collect()
    } else {
        FxHashSet::default()
    };
    raw.nodes = pass(0, Some(&refs))?.nodes;
    Ok(raw)
}

/// The changes of a build to the `raw.*` of its area file: the OSM fixes, and the tags that the network steps set on
/// their ways ([`Raw::sync_tags`]). `fixed` is a part of the file ([`read_part`]) after [`Raw::apply_fixes`], with every
/// element that a fix touches, with all its tags; `read` has the ids of these elements that the part had before the fixes
/// (nodes, ways, relations). An element of the file that is in `read` gets its version in `fixed`, or is removed when
/// `fixed` does not have it. The other elements of the part can have only some of their tags: the file gives them. The
/// elements of `fixed` with an id from `NEW_WAY_ID_MIN` are new (the new ways of the fixes and their new nodes): they come
/// after the elements of the file, as their ids are larger than OSM ids.
pub struct RawPatch<'a> {
    pub fixed: &'a Raw,
    pub read: [Vec<i64>; 3],
    /// The network's ways, sorted by id.
    pub net: Vec<&'a Way>,
}

/// The version of an element of the file after the patch.
pub enum Patched<'a, T> {
    /// The element as the file has it.
    Same,
    /// The element is removed.
    Removed,
    /// The element is this one.
    Fixed(&'a T),
}

fn find<'a, T>(read: &[i64], list: &'a [T], id: i64, key: impl Fn(&T) -> i64) -> Patched<'a, T> {
    if read.binary_search(&id).is_err() {
        return Patched::Same;
    }
    match list.binary_search_by_key(&id, key) {
        Ok(i) => Patched::Fixed(&list[i]),
        Err(_) => Patched::Removed,
    }
}

impl<'a> RawPatch<'a> {
    /// The ids of the part before the fixes that are in `touched` (the elements that the fixes touch): `read`.
    pub fn ids(part: &Raw, touched: &[crate::util::FxHashSet<i64>; 3]) -> [Vec<i64>; 3] {
        let mut rels: Vec<i64> = part.relations.iter().map(|r| r.id).filter(|i| touched[2].contains(i)).collect();
        rels.sort_unstable();
        [
            part.nodes.iter().map(|n| n.id).filter(|i| touched[0].contains(i)).collect(),
            part.ways.iter().map(|w| w.id).filter(|i| touched[1].contains(i)).collect(),
            rels,
        ]
    }

    pub fn node(&self, id: i64) -> Patched<'a, RawNode> {
        find(&self.read[0], &self.fixed.nodes, id, |n| n.id)
    }

    pub fn way(&self, id: i64) -> Patched<'a, RawWay> {
        find(&self.read[1], &self.fixed.ways, id, |w| w.id)
    }

    /// The relations of `fixed` are in the order of the file, sorted by id.
    pub fn relation(&self, id: i64) -> Patched<'a, Relation> {
        find(&self.read[2], &self.fixed.relations, id, |r| r.id)
    }

    /// The network's way with this id.
    pub fn net_way(&self, id: i64) -> Option<&'a Way> {
        self.net.binary_search_by_key(&id, |w| w.id).ok().map(|i| self.net[i])
    }

    /// The new nodes and the new ways, in the order of their ids.
    pub fn new_nodes(&self) -> impl Iterator<Item = &'a RawNode> + '_ {
        self.fixed.nodes.iter().filter(|n| n.id >= NEW_WAY_ID_MIN)
    }

    pub fn new_ways(&self) -> impl Iterator<Item = &'a RawWay> + '_ {
        self.fixed.ways.iter().filter(|w| w.id >= NEW_WAY_ID_MIN)
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
