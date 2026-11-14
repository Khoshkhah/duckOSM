//! Read back the tables of a build (the union of the tiles of a large build reads the tables of each tile).

use arrow_array::cast::AsArray;
use arrow_array::types::{Float32Type, Float64Type, Int32Type, Int64Type};
use arrow_array::{Array, RecordBatch};
use duckosm_core::geom::Pt;
use duckosm_core::graph::{GraphRow, TurnPermission, TurnRestriction};
use duckosm_core::lanes::LaneRow;
use duckosm_core::model::{Attrs, Edge};
use duckosm_core::util::FxHashSet;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::path::Path;
use std::sync::Arc;

fn batches(path: &Path) -> Result<impl Iterator<Item = Result<RecordBatch, String>>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let r = ParquetRecordBatchReaderBuilder::try_new(f).map_err(|e| e.to_string())?.build().map_err(|e| e.to_string())?;
    Ok(r.map(|b| b.map_err(|e| e.to_string())))
}

fn text(b: &RecordBatch, name: &str, i: usize) -> Option<String> {
    let c = b.column_by_name(name)?.as_string::<i32>();
    c.is_valid(i).then(|| c.value(i).to_string())
}

/// The points of a WKB LineString (as `line_wkb` writes it).
fn line(wkb: &[u8]) -> Vec<Pt> {
    let n = u32::from_le_bytes(wkb[5..9].try_into().unwrap()) as usize;
    let f = |k: usize| f64::from_le_bytes(wkb[k..k + 8].try_into().unwrap());
    (0..n).map(|i| (f(9 + 16 * i), f(17 + 16 * i))).collect()
}

/// The values of the functional types, the uses of a turn and the uses of a lane are few: each one is kept once for the run.
fn fixed(v: &str) -> &'static str {
    static SEEN: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap();
    match seen.iter().find(|s| **s == v) {
        Some(s) => s,
        None => {
            let s: &'static str = Box::leak(v.into());
            seen.push(s);
            s
        }
    }
}

/// Read the edges of an `edges` (or `private_edges`) file of the build whose `osm_id` passes `want`, one edge at a time. Edges with equal tags share one `Attrs` (`attrs`: the shared values).
pub fn read_edges(
    path: &Path,
    attrs: &mut FxHashSet<Arc<Attrs>>,
    want: impl Fn(i64) -> bool,
    mut f: impl FnMut(Edge) -> Result<(), String>,
) -> Result<(), String> {
    for b in batches(path)? {
        let b = b?;
        let i64s = |n: &str| b.column_by_name(n).map(|c| c.as_primitive::<Int64Type>().clone());
        let (edge_id, source, target, osm_id) = (i64s("edge_id").unwrap(), i64s("source").unwrap(), i64s("target").unwrap(), i64s("osm_id").unwrap());
        let edge_ref = b.column_by_name("edge_ref").unwrap().as_string::<i32>();
        let oneway = b.column_by_name("oneway").unwrap().as_boolean();
        let is_reverse = b.column_by_name("is_reverse").unwrap().as_boolean();
        let lanes = b.column_by_name("lanes").unwrap().as_primitive::<Int32Type>();
        let length = b.column_by_name("length_m").unwrap().as_primitive::<Float32Type>();
        let f32s = |n: &str| b.column_by_name(n).map(|c| c.as_primitive::<Float32Type>().clone());
        let (kmh, cost) = (f32s("maxspeed_kmh"), f32s("cost_s"));
        let dismount = b.column_by_name("dismount").map(|c| c.as_boolean().clone());
        let ftypes = b.column_by_name("walk_type").or(b.column_by_name("cycle_type")).map(|c| c.as_string::<i32>().clone());
        let refs = b.column_by_name("refs").unwrap().as_list::<i32>();
        let geom = b.column_by_name("geometry").unwrap().as_binary::<i32>();
        let opt_f32 = |c: &Option<arrow_array::Float32Array>, i: usize| c.as_ref().and_then(|c| c.is_valid(i).then(|| c.value(i)));
        for i in 0..b.num_rows() {
            if !want(osm_id.value(i)) {
                continue;
            }
            let geometry = line(geom.value(i));
            let a = Attrs {
                highway: text(&b, "highway", i).unwrap_or_default(),
                name: text(&b, "name", i),
                maxspeed: text(&b, "maxspeed", i),
                surface: text(&b, "surface", i),
                junction: text(&b, "junction", i),
                layer: text(&b, "layer", i),
                bridge: text(&b, "bridge", i),
                tunnel: text(&b, "tunnel", i),
                service: text(&b, "service", i),
            };
            let a = match attrs.get(&a) {
                Some(x) => x.clone(),
                None => {
                    let x = Arc::new(a);
                    attrs.insert(x.clone());
                    x
                }
            };
            // edge_ref: {osm_id}#{seq}{f|r}
            let seq = edge_ref.is_valid(i).then(|| {
                let r = edge_ref.value(i);
                let s = &r[r.find('#').map_or(0, |k| k + 1)..r.len() - 1];
                s.parse::<u32>().ok()
            });
            f(Edge {
                edge_id: edge_id.value(i),
                seq: seq.flatten(),
                source: source.value(i),
                target: target.value(i),
                osm_id: osm_id.value(i),
                attrs: a,
                oneway: oneway.value(i),
                lanes: lanes.is_valid(i).then(|| lanes.value(i)),
                access: text(&b, "access", i).map(Arc::from),
                refs: refs.value(i).as_primitive::<Int64Type>().values().to_vec(),
                geometry,
                is_reverse: is_reverse.value(i),
                length_m: length.value(i),
                dismount: dismount.as_ref().and_then(|c| c.is_valid(i).then(|| c.value(i))),
                maxspeed_kmh: opt_f32(&kmh, i),
                cost_s: opt_f32(&cost, i),
                ftype: ftypes.as_ref().and_then(|c| c.is_valid(i).then(|| fixed(c.value(i)))),
            })?;
        }
    }
    Ok(())
}

/// Read an `edge_graph` file of the build, one row at a time.
pub fn read_edge_graph(path: &Path, mut f: impl FnMut(GraphRow) -> Result<(), String>) -> Result<(), String> {
    for b in batches(path)? {
        let b = b?;
        let from = b.column_by_name("from_edge").unwrap().as_primitive::<Int64Type>();
        let to = b.column_by_name("to_edge").unwrap().as_primitive::<Int64Type>();
        let cost = b.column_by_name("cost").unwrap().as_primitive::<Float32Type>();
        let uses = b.column_by_name("uses").unwrap().as_string::<i32>();
        for i in 0..b.num_rows() {
            f(GraphRow {
                from_edge: from.value(i),
                to_edge: to.value(i),
                cost: cost.is_valid(i).then(|| cost.value(i)),
                uses: fixed(uses.value(i)),
            })?;
        }
    }
    Ok(())
}

/// Read a `lane_profile` file of the build, one row at a time.
pub fn read_lane_profile(path: &Path, mut f: impl FnMut(LaneRow) -> Result<(), String>) -> Result<(), String> {
    for b in batches(path)? {
        let b = b?;
        let id = b.column_by_name("edge_id").unwrap().as_primitive::<Int64Type>();
        let num = b.column_by_name("lane_num").unwrap().as_primitive::<Int32Type>();
        let width = b.column_by_name("width_m").unwrap().as_primitive::<Float64Type>();
        for i in 0..b.num_rows() {
            f(LaneRow {
                edge_id: id.value(i),
                lane_num: num.value(i),
                use_: fixed(&text(&b, "use", i).unwrap_or_default()),
                width_m: width.value(i),
                turn: text(&b, "turn", i),
                source: text(&b, "source", i).unwrap_or_default(),
                width_source: text(&b, "width_source", i).unwrap_or_default(),
            })?;
        }
    }
    Ok(())
}

/// Read a `turn_restrictions` file of the build.
pub fn read_turn_restrictions(path: &Path) -> Result<Vec<TurnRestriction>, String> {
    let mut out = Vec::new();
    for b in batches(path)? {
        let b = b?;
        let i64s = |n: &str| b.column_by_name(n).unwrap().as_primitive::<Int64Type>().clone();
        let (id, via, from, to) = (i64s("restriction_id"), i64s("via_node"), i64s("from_edge_id"), i64s("to_edge_id"));
        for i in 0..b.num_rows() {
            out.push(TurnRestriction {
                restriction_id: id.value(i),
                restriction_type: text(&b, "restriction_type", i).unwrap_or_default(),
                except_vehicles: text(&b, "except_vehicles", i),
                applies_to: text(&b, "applies_to", i),
                condition: text(&b, "condition", i),
                via_node: via.value(i),
                from_edge_id: from.value(i),
                to_edge_id: to.value(i),
            });
        }
    }
    Ok(out)
}

/// Read a `turn_permission` file of the build.
pub fn read_turn_permission(path: &Path) -> Result<Vec<TurnPermission>, String> {
    let mut out = Vec::new();
    for b in batches(path)? {
        let b = b?;
        let i64s = |n: &str| b.column_by_name(n).unwrap().as_primitive::<Int64Type>().clone();
        let (from, to, id) = (i64s("from_edge"), i64s("to_edge"), i64s("restriction_id"));
        let allowed = b.column_by_name("allowed").unwrap().as_boolean();
        for i in 0..b.num_rows() {
            out.push(TurnPermission {
                from_edge: from.value(i),
                to_edge: to.value(i),
                allowed: allowed.value(i),
                vehicles: text(&b, "vehicles", i),
                except_vehicles: text(&b, "except_vehicles", i),
                condition: text(&b, "condition", i),
                restriction_id: id.value(i),
            });
        }
    }
    Ok(out)
}

/// Read a `nodes` file of the build, one node at a time.
pub fn each_node(path: &Path, mut f: impl FnMut(duckosm_core::model::Node)) -> Result<(), String> {
    for b in batches(path)? {
        let b = b?;
        let id = b.column_by_name("node_id").unwrap().as_primitive::<Int64Type>();
        let g = b.column_by_name("geometry").unwrap().as_binary::<i32>();
        for i in 0..b.num_rows() {
            // a WKB Point: the byte order, the type, x, y
            let w = g.value(i);
            let x = |k: usize| f64::from_le_bytes(w[k..k + 8].try_into().unwrap());
            f(duckosm_core::model::Node { node_id: id.value(i), lon: x(5), lat: x(13) });
        }
    }
    Ok(())
}

/// The id, the source and target nodes, and the two ends of the line of each edge of an `edges` file (only these columns are
/// read).
pub fn read_edge_ends(path: &Path, mut f: impl FnMut(i64, i64, i64, Pt, Pt)) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let b = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| e.to_string())?;
    let cols: Vec<usize> = ["edge_id", "source", "target", "geometry"].iter().filter_map(|n| b.schema().index_of(n).ok()).collect();
    let mask = parquet::arrow::ProjectionMask::roots(b.parquet_schema(), cols);
    for batch in b.with_projection(mask).build().map_err(|e| e.to_string())? {
        let batch = batch.map_err(|e| e.to_string())?;
        let i64s = |n: &str| batch.column_by_name(n).unwrap().as_primitive::<Int64Type>().clone();
        let (id, src, tgt) = (i64s("edge_id"), i64s("source"), i64s("target"));
        let g = batch.column_by_name("geometry").unwrap().as_binary::<i32>();
        for i in 0..batch.num_rows() {
            let w = g.value(i);
            let n = u32::from_le_bytes(w[5..9].try_into().unwrap()) as usize;
            let pt = |k: usize| (f64::from_le_bytes(w[9 + 16 * k..17 + 16 * k].try_into().unwrap()), f64::from_le_bytes(w[17 + 16 * k..25 + 16 * k].try_into().unwrap()));
            f(id.value(i), src.value(i), tgt.value(i), pt(0), pt(n - 1));
        }
    }
    Ok(())
}
