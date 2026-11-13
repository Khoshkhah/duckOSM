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

/// The levels of a large area by tiles (`method = "solve"`, Kaveh 2026-11-13): no step holds the level edges, the roads or the
/// rules of the whole area. Each road belongs to the tile (`TILE_DEG`) of the smaller end of its line, so both ways of a road
/// are in one tile. A tile has its roads and the roads within `MARGIN_DEG` of them (a rule reaches 10 m: `band_dist`); the
/// roads that an earlier tile solved are fixed at their numbers (`roadstyle_levels::levels_tile`). The rows go to a temporary
/// file with their raw numbers; at the end one move to the ground for the whole area (`roadstyle_levels::Ground`), and the
/// rows are written in the order of `edge_id`. An area in one tile gives the rows of [`compute`].
pub mod tiled {
    use super::*;
    use duckosm_core::network::EdgeFiles;
    use duckosm_core::spill::{self, Codec, Writer};
    use roadstyle_levels::{Ground, LevelEdge, TileRoles};
    use std::collections::{BTreeSet, HashMap};

    const TILE_DEG: f64 = 0.1;
    const MARGIN_DEG: f64 = 0.001;
    const BUDGET: usize = 64 << 20;
    /// the tiles that can run at the same time: at most this many tiles and about this many edges in memory
    const WINDOW_TILES: usize = 64;
    const WINDOW_EDGES: usize = 1 << 18;

    type Tile = (i32, i32);

    fn tile_of(p: (f64, f64)) -> Tile {
        ((p.1 / TILE_DEG).floor() as i32, (p.0 / TILE_DEG).floor() as i32)
    }

    /// The tile of a road: of the smaller end of its line (the same for both ways of the road).
    fn owner(g: &[(f64, f64)]) -> Tile {
        let (a, b) = (g[0], g[g.len() - 1]);
        let less = a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).is_le();
        tile_of(if less { a } else { b })
    }

    fn bbox(g: &[(f64, f64)]) -> [f64; 4] {
        g.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |[w, s, e, n], p| [w.min(p.0), s.min(p.1), e.max(p.0), n.max(p.1)])
    }

    /// One level edge in a tile: (tile, owned, needed by a later tile, rank, edge id, tag set, line). The tile is first: the sort key.
    type TileEdge = (Tile, bool, bool, u8, i64, u32, Vec<(f64, f64)>);
    struct TileEdgeCodec;
    impl Codec<TileEdge> for TileEdgeCodec {
        fn put(&self, r: &TileEdge, out: &mut Vec<u8>) {
            out.extend_from_slice(&(r.0 .0 as i64 - i32::MIN as i64).to_be_bytes()[4..]);
            out.extend_from_slice(&(r.0 .1 as i64 - i32::MIN as i64).to_be_bytes()[4..]);
            out.extend_from_slice(&[r.1 as u8, r.2 as u8, r.3]);
            out.extend_from_slice(&r.4.to_le_bytes());
            out.extend_from_slice(&r.5.to_le_bytes());
            for p in &r.6 {
                out.extend_from_slice(&p.0.to_bits().to_le_bytes());
                out.extend_from_slice(&p.1.to_bits().to_le_bytes());
            }
        }
        fn get(&self, b: &mut &[u8]) -> TileEdge {
            let u = |b: &[u8]| (u32::from_be_bytes(b.try_into().expect("4 bytes")) as i64 + i32::MIN as i64) as i32;
            let tile = (u(&b[0..4]), u(&b[4..8]));
            let id = i64::from_le_bytes(b[11..19].try_into().expect("8 bytes"));
            let tags = u32::from_le_bytes(b[19..23].try_into().expect("4 bytes"));
            let pts = b[23..]
                .chunks_exact(16)
                .map(|c| {
                    let f = |c: &[u8]| f64::from_bits(u64::from_le_bytes(c.try_into().expect("8 bytes")));
                    (f(&c[..8]), f(&c[8..]))
                })
                .collect();
            (tile, b[8] != 0, b[9] != 0, b[10], id, tags, pts)
        }
    }

    /// A row with its raw numbers: (edge id, the four numbers, square caps (bit 0 the start, bit 1 the end), head length).
    type RawRow = (i64, [i64; 4], u8, Option<f64>);
    struct RawRowCodec;
    impl Codec<RawRow> for RawRowCodec {
        fn put(&self, r: &RawRow, out: &mut Vec<u8>) {
            out.extend_from_slice(&((r.0 as u64) ^ (1 << 63)).to_be_bytes()); // the order of the ids in the order of the bytes
            for v in r.1 {
                out.extend_from_slice(&(v as i16).to_le_bytes());
            }
            out.push(r.2 | if r.3.is_some() { 4 } else { 0 });
            out.extend_from_slice(&r.3.unwrap_or(0.0).to_bits().to_le_bytes());
        }
        fn get(&self, b: &mut &[u8]) -> RawRow {
            let id = (u64::from_be_bytes(b[0..8].try_into().expect("8 bytes")) ^ (1 << 63)) as i64;
            let v = |k: usize| i16::from_le_bytes(b[8 + 2 * k..10 + 2 * k].try_into().expect("2 bytes")) as i64;
            let head = f64::from_bits(u64::from_le_bytes(b[17..25].try_into().expect("8 bytes")));
            (id, [v(0), v(1), v(2), v(3)], b[16] & 3, (b[16] & 4 != 0).then_some(head))
        }
    }

    /// The tag sets of the level edges, one copy of each (by the shared `Attrs` of the edges, then by value).
    #[derive(Default)]
    struct TagTable {
        of_attrs: FxHashMap<usize, (Arc<Attrs>, u32)>,
        of_value: FxHashMap<Arc<LevelTags>, u32>,
        sets: Vec<Arc<LevelTags>>,
    }

    impl TagTable {
        fn id(&mut self, a: &Arc<Attrs>) -> u32 {
            let key = Arc::as_ptr(a) as usize;
            if let Some((_, i)) = self.of_attrs.get(&key) {
                return *i;
            }
            let t = Arc::new(LevelTags {
                highway: a.highway.clone(),
                tunnel: a.tunnel.clone(),
                bridge: a.bridge.clone(),
                layer: a.layer.clone(),
                junction: a.junction.clone(),
                band: None,
            });
            let i = match self.of_value.get(&t) {
                Some(&i) => i,
                None => {
                    self.sets.push(t.clone());
                    self.of_value.insert(t, (self.sets.len() - 1) as u32);
                    (self.sets.len() - 1) as u32
                }
            };
            self.of_attrs.insert(key, (a.clone(), i));
            i
        }
    }

    /// Each level edge of all modes (edges, then private edges), with its rank, from the spill files.
    fn each_level_edge(modes: &mut [(Mode, EdgeFiles)], mut f: impl FnMut(u8, Edge) -> Result<(), String>) -> Result<(), String> {
        let err = |e: std::io::Error| format!("temporary file of the network step: {e}");
        for (mode, files) in modes.iter_mut() {
            let base = match mode {
                Mode::Driving => 0,
                Mode::Walking => 1,
                Mode::Cycling => 2,
            };
            for e in files.edges().map_err(err)? {
                f(base, e.map_err(err)?)?;
            }
            for e in files.private_edges().map_err(err)? {
                f(base + 3, e.map_err(err)?)?;
            }
        }
        Ok(())
    }

    /// The levels by tiles: the files `edge_levels_visualization.parquet` and `edge_levels_meta_visualization.parquet`, and
    /// the line of the summary (as [`crate::write_levels`]).
    pub fn write_levels_tiled<W: std::io::Write + Send>(
        modes: &mut [(Mode, EdgeFiles)],
        built_at: i64,
        mut file: impl FnMut(&str) -> Result<W, String>,
    ) -> Result<Option<String>, String> {
        use duckosm_parquet as pq;
        let o = roadstyle_levels::Options { method: Method::Solve, ..Default::default() };
        let io = |e: std::io::Error| format!("a temporary file of the levels: {e}");
        let store = spill::temp_store();
        let store = &*store;
        // 1. the box of the roads of each tile
        let mut boxes: HashMap<Tile, [f64; 4]> = HashMap::new();
        each_level_edge(modes, |_, e| {
            if e.geometry.len() >= 2 {
                let b = bbox(&e.geometry);
                let t = boxes.entry(owner(&e.geometry)).or_insert([f64::MAX, f64::MAX, f64::MIN, f64::MIN]);
                *t = [t[0].min(b[0]), t[1].min(b[1]), t[2].max(b[2]), t[3].max(b[3])];
            }
            Ok(())
        })?;
        let order: HashMap<Tile, usize> = {
            let mut t: Vec<Tile> = boxes.keys().copied().collect();
            t.sort_unstable();
            t.into_iter().enumerate().map(|(i, t)| (t, i)).collect()
        };
        // the tiles near each cell of the grid: a tile's box with the margin, in the cells it covers
        let mut near: HashMap<Tile, Vec<Tile>> = HashMap::new();
        for (&t, b) in &boxes {
            let (lo, hi) = (tile_of((b[0] - MARGIN_DEG, b[1] - MARGIN_DEG)), tile_of((b[2] + MARGIN_DEG, b[3] + MARGIN_DEG)));
            for y in lo.0..=hi.0 {
                for x in lo.1..=hi.1 {
                    near.entry((y, x)).or_default().push(t);
                }
            }
        }
        // 2. each edge to its tile, and to each tile whose box (with the margin) it touches; sorted by tile. A tile waits for the
        // earlier tiles that own a road of it (their numbers are fixed in it)
        let mut waits: HashMap<Tile, FxHashSet<Tile>> = HashMap::new();
        let mut tags = TagTable::default();
        let mut rows = Writer::new(store).map_err(io)?;
        each_level_edge(modes, |rank, e| {
            if e.geometry.len() < 2 {
                return Ok(());
            }
            let own = owner(&e.geometry);
            let b = bbox(&e.geometry);
            let (lo, hi) = (tile_of((b[0], b[1])), tile_of((b[2], b[3])));
            let mut others: Vec<Tile> = Vec::new();
            for y in lo.0..=hi.0 {
                for x in lo.1..=hi.1 {
                    for &t in near.get(&(y, x)).map_or(&[][..], |v| v.as_slice()) {
                        let tb = boxes[&t];
                        let touches = b[0] <= tb[2] + MARGIN_DEG
                            && b[2] >= tb[0] - MARGIN_DEG
                            && b[1] <= tb[3] + MARGIN_DEG
                            && b[3] >= tb[1] - MARGIN_DEG;
                        if t != own && touches && !others.contains(&t) {
                            others.push(t);
                        }
                    }
                }
            }
            let later = others.iter().any(|t| order[t] > order[&own]);
            for t in others.iter().filter(|t| order[t] > order[&own]) {
                waits.entry(*t).or_default().insert(own);
            }
            let id = tags.id(&e.attrs);
            for t in &others {
                rows.push(&TileEdgeCodec, &(*t, false, false, rank, e.edge_id, id, e.geometry.clone())).map_err(io)?;
            }
            rows.push(&TileEdgeCodec, &(own, true, later, rank, e.edge_id, id, e.geometry)).map_err(io)?;
            Ok(())
        })?;
        drop((boxes, near));
        let rows = spill::sort_bytes(rows.finish().map_err(io)?, |b| u64::from_be_bytes(b[..8].try_into().expect("8 bytes")), BUDGET, store).map_err(io)?;
        // 3. the tiles: the numbers of an edge that a later tile needs are kept, with the order of its tile. A tile reads only the
        // numbers of earlier tiles, as when the tiles run one after the other; the tiles that wait for no tile still running run at
        // the same time (a window of the next tiles in their order: the first one never waits)
        let mut earlier: FxHashMap<i64, (usize, [i64; 4])> = FxHashMap::default();
        let mut raw = Writer::new(store).map_err(io)?;
        let (mut casing, mut values) = (HashMap::<i64, usize>::new(), BTreeSet::<i64>::new());
        let mut sums = roadstyle_levels::TileLevels::default();
        let solve = |tile: Vec<TileEdge>, earlier: &FxHashMap<i64, (usize, [i64; 4])>| {
            let me = order[&tile[0].0];
            let (tile_y, tile_x) = tile[0].0;
            let mut edges = roadstyle_levels::LevelEdges::default();
            edges.reserve(tile.len());
            let (mut owned, mut keep) = (FxHashSet::default(), FxHashSet::default());
            for (_, own, later, rank, id, t, g) in tile {
                if own {
                    owned.insert(id);
                    if later {
                        keep.insert(id);
                    }
                }
                edges.add(rank, LevelEdge { edge_id: id, tags: tags.sets[t as usize].clone(), geometry: g, ..Default::default() });
            }
            let roles = TileRoles {
                owned: &|id| owned.contains(&id),
                earlier: &|id| earlier.get(&id).filter(|(o, _)| *o < me).map(|(_, v)| *v),
            };
            let r = roadstyle_levels::levels_tile(&edges, &roles, &o)
                .map_err(|e| format!("{e} (the tile of {:.1}, {:.1})", tile_y as f64 * TILE_DEG, tile_x as f64 * TILE_DEG))?;
            Ok::<_, String>((me, r, keep))
        };
        let mut done: FxHashSet<Tile> = FxHashSet::default();
        let mut window: std::collections::VecDeque<Vec<TileEdge>> = std::collections::VecDeque::new();
        let (mut in_window, mut tile) = (0usize, Vec::<TileEdge>::new());
        let mut rows = rows.read(&TileEdgeCodec).map_err(io)?.peekable();
        loop {
            // the next tiles to the window, up to WINDOW_TILES tiles and WINDOW_EDGES edges
            while window.len() < WINDOW_TILES && (window.is_empty() || in_window < WINDOW_EDGES) {
                while let Some(r) = rows.next_if(|r| r.as_ref().map_or(true, |r| tile.is_empty() || r.0 == tile[0].0)) {
                    tile.push(r.map_err(io)?);
                }
                if tile.is_empty() {
                    break;
                }
                in_window += tile.len();
                window.push_back(std::mem::take(&mut tile));
            }
            if window.is_empty() {
                break;
            }
            // the tiles of the window that wait for no tile that is not done
            let ready = |t: &Tile| waits.get(t).is_none_or(|w| w.iter().all(|u| done.contains(u)));
            let (run, wait): (Vec<_>, Vec<_>) = window.drain(..).partition(|x| ready(&x[0].0));
            window.extend(wait);
            in_window -= run.iter().map(Vec::len).sum::<usize>();
            let tiles: Vec<Tile> = run.iter().map(|x| x[0].0).collect();
            let results: Vec<_> = {
                use rayon::prelude::*;
                run.into_par_iter().map(|t| solve(t, &earlier)).collect()
            };
            for (t, r) in tiles.into_iter().zip(results) {
                let (me, r, keep) = r?;
                for row in &r.rows {
                    let v = [row.casing_start, row.casing_level, row.casing_end, row.fill_level];
                    *casing.entry(v[1]).or_default() += 1;
                    values.extend(v);
                    let caps = u8::from(row.cap_start == Some("square")) | u8::from(row.cap_end == Some("square")) << 1;
                    raw.push(&RawRowCodec, &(row.edge_id, v, caps, row.head_start_m)).map_err(io)?;
                    if keep.contains(&row.edge_id) {
                        earlier.insert(row.edge_id, (me, v));
                    }
                }
                sums.roads += r.roads;
                sums.pairs += r.pairs;
                sums.order_pairs += r.order_pairs;
                sums.given_up += r.given_up;
                sums.orders_not_kept += r.orders_not_kept;
                sums.crossings += r.crossings;
                for (s, v) in sums.stages.iter_mut().zip(r.stages) {
                    *s += v;
                }
                done.insert(t);
            }
        }
        drop((earlier, tags, waits, done, order));
        // 4. one move to the ground for the whole area; the rows in the order of edge_id
        let ground = Ground::new(&casing, &values, o.max_level).map_err(|e| e.to_string())?;
        let span = match (values.first(), values.last()) {
            (Some(lo), Some(hi)) => hi - lo,
            _ => 0,
        };
        sums.stages[4] = roadstyle_levels::tiled_stage4(sums.stages[4], sums.roads, span, &o);
        let raw = spill::sort_bytes(raw.finish().map_err(io)?, |b| u64::from_be_bytes(b[..8].try_into().expect("8 bytes")), BUDGET, store).map_err(io)?;
        let mut w = pq::EdgeLevelsWriter::new(file("edge_levels_visualization.parquet")?)?;
        let (mut ids, mut part) = (Vec::new(), Vec::new());
        for r in raw.read(&RawRowCodec).map_err(io)? {
            let (id, v, caps, head) = r.map_err(io)?;
            let cap = |bit: u8| Some(if caps & bit != 0 { "square" } else { "round" });
            ids.push(id);
            part.push(Row {
                edge_id: id,
                casing_start: ground.level(v[0]),
                casing_level: ground.level(v[1]),
                casing_end: ground.level(v[2]),
                fill_level: ground.level(v[3]),
                head_start_m: head,
                head_end_m: head,
                cap_start: cap(1),
                cap_end: cap(2),
            });
            if part.len() == 1 << 16 {
                w.write(&part)?;
                part.clear();
            }
        }
        w.write(&part)?;
        w.close()?;
        let meta = pq::LevelsMeta {
            source: Method::Solve.name(),
            n_edges: ids.len() as i64,
            edge_hash: roadstyle_levels::edge_hash(&ids),
            roadstyle_version: ROADSTYLE_VERSION,
            created: crate::area::iso_time(built_at),
        };
        pq::write_edge_levels_meta_to(file("edge_levels_meta_visualization.parquet")?, &meta)?;
        Ok(Some(format!(
            "levels (solve): {} edges, {} roads, {} stack pairs given up, {} order wishes not kept (best values of the stages: {:?})",
            ids.len(),
            sums.roads,
            sums.given_up,
            sums.orders_not_kept,
            sums.stages
        )))
    }
}
