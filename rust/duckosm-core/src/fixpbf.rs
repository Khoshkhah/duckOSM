//! `duckosm cut --fixes`: a `.osm.pbf` file with the OSM fixes in it (docs/exports/osm-file.md), so that other tools
//! (OSMnx, osm2pgsql, SUMO, QGIS) read the edited data. The changes are those of `quack --fixes` ([`QuackFixes`]):
//!
//! - `remove`, `keep_nodes`, `tags` and the tag edits of nodes, ways and relations;
//! - the network rules as OSM tags (`oneway`, `lanes`, `lanes:forward`, `lanes:backward`, `layer`);
//! - the new ways, with their new nodes (id `way id * 1000 + position`, as the build gives them);
//! - each rule of `turn_restrictions` as a relation `type=restriction` with the id of the build (-1, -2, ...).
//!
//! `exclude_modes` and the arrows of the lanes have no OSM tag, so the file does not have them.
//!
//! The input is sorted by type, then id, and so is the output: the new ids are above all OSM ids, so the new nodes go
//! after the last node and the new ways after the last way; the negative ids of the turn rules go before the first
//! relation. One block is in memory at a time.

use crate::cutfile::PbfWriter;
use crate::cutstream::{each_block, CutCounts};
use crate::fixes::{TurnFix, NEW_WAY_ID_MIN};
use crate::graph::{MemberType, Relation};
use crate::pbfraw::decode_block;
use crate::quack::QuackFixes;
use crate::raw::{RawNode, RawSink, RawWay};
use crate::util::FxHashSet;
use std::io::{Read, Write};

/// The elements of one block, with the fixes applied.
struct Fixed<'a> {
    fixes: &'a QuackFixes,
    nodes: Vec<RawNode>,
    ways: Vec<RawWay>,
    rels: Vec<Relation>,
}

impl RawSink for Fixed<'_> {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        let mut tags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.fixes.node_tags(id, &mut tags);
        self.nodes.push(RawNode { id, lon, lat, tags });
    }
    fn way<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        let (mut tags, mut refs) = (tags.map(|(k, v)| (k.to_string(), v.to_string())).collect(), refs.collect());
        if self.fixes.way(id, &mut tags, &mut refs) {
            self.ways.push(RawWay { id, tags, refs });
        }
    }
    fn relation<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        let mut tags = tags.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.fixes.relation_tags(id, &mut tags);
        self.rels.push(Relation { id, tags, members: members.map(|(m, t, r)| (m, t, r.to_string())).collect() });
    }
}

/// Copy the `.osm.pbf` file that `open` opens to `out`, with `fixes` and the turn rules `turns` in it. A turn rule
/// goes to the file only when its two ways are in it. Gives `out` back, and the numbers of elements written.
pub fn write_fixed<R: Read + Send, W: Write>(
    open: impl Fn() -> Result<R, String>,
    fixes: &QuackFixes,
    turns: &[TurnFix],
    out: W,
) -> Result<(W, CutCounts), String> {
    let timestamp = crate::cutstream::header(open()?)?.0;
    let mut w = PbfWriter::new(out, timestamp)?;
    let mut n = CutCounts::default();
    let new = fixes.new_ways();
    let turn_ways: FxHashSet<i64> = turns.iter().flat_map(|t| [t.from_way, t.to_way]).collect();
    let mut seen: FxHashSet<i64> = new.iter().map(|w| w.0).filter(|id| turn_ways.contains(id)).collect();
    // 0: nodes, 1: ways, 2: relations; `to` writes the new elements of the kinds before `kind`
    let mut at = 0;
    let mut to = |kind: usize, w: &mut PbfWriter<W>, n: &mut CutCounts, seen: &FxHashSet<i64>| -> Result<(), String> {
        if at == 0 && kind > 0 {
            let mut nodes: Vec<RawNode> = new
                .iter()
                .flat_map(|(_, _, refs, pts)| refs.iter().zip(pts.iter()))
                .filter(|(&id, _)| id >= NEW_WAY_ID_MIN * 1000)
                .map(|(&id, &(lon, lat))| RawNode { id, lon, lat, tags: Vec::new() })
                .collect();
            nodes.sort_by_key(|x| x.id);
            nodes.dedup_by_key(|x| x.id);
            n.nodes += nodes.len();
            nodes.into_iter().try_for_each(|x| w.node(x))?;
            at = 1;
        }
        if at == 1 && kind > 1 {
            for &(id, tags, refs, _) in &new {
                w.way(RawWay { id, tags: tags.clone(), refs: refs.to_vec() })?;
                n.ways += 1;
            }
            // -n first: the file is sorted by id
            for t in turns.iter().rev().filter(|t| seen.contains(&t.from_way) && seen.contains(&t.to_way)) {
                let tags = [("type", "restriction"), ("restriction", t.restriction.as_str())];
                w.relation(Relation {
                    id: t.id,
                    tags: tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                    members: vec![
                        (t.from_way, MemberType::Way, "from".into()),
                        (t.via_node, MemberType::Node, "via".into()),
                        (t.to_way, MemberType::Way, "to".into()),
                    ],
                })?;
                n.relations += 1;
            }
            at = 2;
        }
        Ok(())
    };
    each_block(
        &mut open()?,
        |b, buf| {
            let mut s = Fixed { fixes, nodes: Vec::new(), ways: Vec::new(), rels: Vec::new() };
            decode_block(b, [true, true, true], buf, &mut s)?;
            Ok((s.nodes, s.ways, s.rels))
        },
        |(nodes, ways, rels), _| {
            n.nodes += nodes.len();
            nodes.into_iter().try_for_each(|x| w.node(x))?;
            if !ways.is_empty() {
                to(1, &mut w, &mut n, &seen)?;
            }
            n.ways += ways.len();
            for x in ways {
                if turn_ways.contains(&x.id) {
                    seen.insert(x.id);
                }
                w.way(x)?;
            }
            if !rels.is_empty() {
                to(2, &mut w, &mut n, &seen)?;
            }
            n.relations += rels.len();
            rels.into_iter().try_for_each(|x| w.relation(x))?;
            Ok(true)
        },
    )?;
    to(2, &mut w, &mut n, &seen)?;
    Ok((w.finish()?, n))
}
