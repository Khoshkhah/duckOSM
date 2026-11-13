//! The drawing order of the roads, `visualization.edge_levels` and `edge_levels_meta` (docs/levels.md): roadstyle's
//! `compute_levels`, with `method="tags"` (a rule on the tags, the default) or `method="solve"` (its optimization,
//! with the defaults of duckOSM's `duckosm levels` and no edits). The rules and the solver are in roadstyle's crate
//! `roadstyle-levels` (roadstyle repository `rust/`); this module gives it duckosm's edges.

use duckosm_core::filter::Mode;
use duckosm_core::model::{Attrs, Edge};
use duckosm_core::util::{FxHashMap, FxHashSet};
use roadstyle_levels::LevelTags;
use std::sync::Arc;
pub use roadstyle_levels::{Levels, Method, Row, ROADSTYLE_VERSION};

/// The edges of all modes, `edges` and `private_edges` (duckOSM's `load_roads`). An edge in several tables has the
/// attributes of the first one in the sequence driving, walking, cycling, then their private edges.
#[derive(Default)]
pub struct LevelEdges {
    edges: roadstyle_levels::LevelEdges,
    /// One copy of each tag set: by the shared tags of the edges (`Attrs`, kept alive here), then by value
    of_attrs: FxHashMap<usize, (Arc<Attrs>, Arc<LevelTags>)>,
    tag_sets: FxHashSet<Arc<LevelTags>>,
}

impl LevelEdges {
    /// Makes space for `n` more edges: call it before the tables of a mode are added, so that the list does not double
    /// while it grows (in WebAssembly the memory never shrinks).
    pub fn reserve(&mut self, n: usize) {
        self.edges.reserve(n);
    }

    /// Takes the edges: their geometry moves into the level edges, with no copy (a copy of all geometries made the
    /// peak of the build while the network was still in memory). The edges with equal tags share one `LevelTags` (six
    /// strings of each edge were 2.45 million copies on Kentucky).
    pub fn add(&mut self, mode: Mode, private: bool, edges: impl IntoIterator<Item = Edge>) {
        let rank = match mode {
            Mode::Driving => 0,
            Mode::Walking => 1,
            Mode::Cycling => 2,
        } + if private { 3 } else { 0 };
        for e in edges {
            let key = Arc::as_ptr(&e.attrs) as usize;
            let tags = match self.of_attrs.get(&key) {
                Some((_, t)) => t.clone(),
                None => {
                    let a = &e.attrs;
                    let t = LevelTags {
                        highway: a.highway.clone(),
                        tunnel: a.tunnel.clone(),
                        bridge: a.bridge.clone(),
                        layer: a.layer.clone(),
                        junction: a.junction.clone(),
                        band: None,
                    };
                    let t = match self.tag_sets.get(&t) {
                        Some(t) => t.clone(),
                        None => {
                            let t = Arc::new(t);
                            self.tag_sets.insert(t.clone());
                            t
                        }
                    };
                    self.of_attrs.insert(key, (e.attrs.clone(), t.clone()));
                    t
                }
            };
            self.edges.add(
                rank,
                roadstyle_levels::LevelEdge { edge_id: e.edge_id, tags, geometry: e.geometry, ..Default::default() },
            );
        }
    }
}

/// roadstyle's levels of the edges; None for `Method::None`.
/// [`compute`] with the rows given to `sink` in parts, in the order of `edge_id`; the `Levels` has no rows.
pub fn compute_each(
    edges: &LevelEdges,
    method: Method,
    mut sink: impl FnMut(Vec<Row>) -> Result<(), String>,
) -> Result<Option<Levels>, String> {
    let o = roadstyle_levels::Options { method, ..Default::default() };
    let mut failed = None;
    let lv = roadstyle_levels::levels_each(&edges.edges, &o, &mut |rows| {
        if failed.is_none() {
            failed = sink(rows).err();
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    failed.map_or(Ok(lv), Err)
}

pub fn compute(edges: &LevelEdges, method: Method) -> Result<Option<Levels>, String> {
    let o = roadstyle_levels::Options { method, ..Default::default() };
    roadstyle_levels::levels(&edges.edges, &o).map_err(|e| e.to_string())
}
