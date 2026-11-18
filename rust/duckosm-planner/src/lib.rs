//! The route planner of `duckosm route-map` (docs/design/all_in_rust.md, group F): duckOSM's routes between two points over an area
//! held in memory, the data of the planner page (`RM`). The page runs this crate as WebAssembly (`wasm.rs`), so it routes as
//! `duckosm route-points`, `route-multimodal-points` and `directions` do: the same router (`duckosm-router`), the same data,
//! the same results (`tests/planner.rs` compares them on Monaco).

use duckosm_router::directions::{self, Road as DirRoad};
use duckosm_router::geom::Pt;
use duckosm_router::multimodal::{Network, Points, Prepared, Step, Weight, HUB};
use duckosm_router::points::{self, Cand, Road};
use serde_json::{json, Value as J};
use std::collections::HashMap;

#[cfg(target_arch = "wasm32")]
mod wasm;

pub type Result<T> = std::result::Result<T, String>;

/// The roads of one mode, in the order of `<mode>.edges` (the order of the database: the candidates and the roads that leave a
/// node keep it), with their cost and length (None: NULL), the legal turns (`edge_id -> [edge_id]`, in the order of the edge
/// graph) and the roads that leave each node.
struct Mode {
    k: Vec<usize>,
    cost: Vec<Option<f64>>,
    len: Vec<Option<f64>>,
    at: HashMap<i64, usize>,
    next: HashMap<i64, Vec<i64>>,
    by_time: HashMap<i64, f64>,
    by_len: HashMap<i64, f64>,
    out: HashMap<i64, Vec<usize>>,
}

/// An area: each road `k` (one for each edge_id of the page) with its edge_id, ends, name, highway, roundabout flag and line;
/// each mode's roads; walk + drive (`mm.*`) when the database has it.
pub struct Area {
    pub modes: Vec<String>,
    eid: Vec<i64>,
    src: Vec<i64>,
    tgt: Vec<i64>,
    name: Vec<Option<String>>,
    hw: Vec<Option<String>>,
    rb: Vec<bool>,
    line: Vec<Vec<Pt>>,
    bbox: Vec<[f64; 4]>,
    graphs: HashMap<String, Mode>,
    mm: Option<Prepared>,
}

fn arr<'a>(v: &'a J, k: &str) -> Result<&'a Vec<J>> {
    v[k].as_array().ok_or_else(|| format!("the planner data has no list {k}"))
}

fn id(v: &J) -> Result<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).ok_or_else(|| format!("{v} is not an id"))
}

fn text(v: &J) -> Option<String> {
    v.as_str().map(String::from)
}

impl Area {
    /// The area of the page's data (`RM`, made by `duckosm route-map`): `eid` the edge_ids (as text), `nid` the node ids (as
    /// text) that `src` and `tgt` point to, `geom` each road's line as [x0, y0, x1, y1, ...], `graphs` each mode's roads
    /// (`k`, `cost`, `len`, `next`: the turns by road number), `mm` the roads and changes of mode of walk + drive.
    pub fn from_data(d: &J) -> Result<Area> {
        let modes: Vec<String> = arr(d, "modes")?.iter().filter_map(text).collect();
        let eid = arr(d, "eid")?.iter().map(id).collect::<Result<Vec<_>>>()?;
        let nid = arr(d, "nid")?.iter().map(id).collect::<Result<Vec<_>>>()?;
        let node = |v: &J| -> Result<i64> { v.as_u64().and_then(|i| nid.get(i as usize).copied()).ok_or_else(|| format!("{v} is not a node")) };
        let src = arr(d, "src")?.iter().map(node).collect::<Result<Vec<_>>>()?;
        let tgt = arr(d, "tgt")?.iter().map(node).collect::<Result<Vec<_>>>()?;
        let name = arr(d, "name")?.iter().map(text).collect();
        let hw = arr(d, "hw")?.iter().map(|v| text(v).filter(|s| !s.is_empty())).collect();
        let mut rb = vec![false; eid.len()];
        for k in arr(d, "rb")? {
            if let Some(r) = k.as_u64().and_then(|k| rb.get_mut(k as usize)) {
                *r = true;
            }
        }
        let line: Vec<Vec<Pt>> = arr(d, "geom")?.iter()
            .map(|g| g.as_array().map(|c| c.chunks(2).map(|p| (p[0].as_f64().unwrap_or(0.0), p.get(1).and_then(J::as_f64).unwrap_or(0.0))).collect()).unwrap_or_default())
            .collect();
        let bbox = line.iter().map(|l| l.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                                                      |b, p| [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)])).collect();
        let ix = |v: &J| -> Result<usize> { v.as_u64().map(|k| k as usize).filter(|&k| k < eid.len()).ok_or_else(|| format!("{v} is not a road")) };
        let opt = |v: &J| v.as_f64();
        let mut graphs = HashMap::new();
        for m in &modes {
            let g = &d["graphs"][m];
            let k = arr(g, "k")?.iter().map(ix).collect::<Result<Vec<_>>>()?;
            let cost: Vec<Option<f64>> = arr(g, "cost")?.iter().map(opt).collect();
            let len: Vec<Option<f64>> = arr(g, "len")?.iter().map(opt).collect();
            let mut next = HashMap::new();
            for (i, n) in arr(g, "next")?.iter().enumerate() {
                let to = n.as_array().into_iter().flatten().map(|t| ix(t).map(|t| eid[t])).collect::<Result<Vec<_>>>()?;
                if !to.is_empty() {
                    next.insert(eid[k[i]], to);
                }
            }
            let mut out: HashMap<i64, Vec<usize>> = HashMap::new();
            for &r in &k {
                out.entry(src[r]).or_default().push(r);
            }
            let at = k.iter().enumerate().map(|(i, &r)| (eid[r], i)).collect();
            let by_time = k.iter().zip(&cost).map(|(&r, c)| (eid[r], c.unwrap_or(0.0))).collect();
            let by_len = k.iter().zip(&len).map(|(&r, c)| (eid[r], c.unwrap_or(0.0))).collect();
            graphs.insert(m.clone(), Mode { k, cost, len, at, next, by_time, by_len, out });
        }
        let mm = match d.get("mm").filter(|m| !m.is_null()) {
            None => None,
            Some(mm) => {
                let mut net = Network { modes: modes.clone(), ..Network::default() };
                for e in arr(mm, "edges")? {
                    let r = ix(&e[3])?;
                    net.edges.push((e[0].as_u64().unwrap_or(0) as u8, node(&e[1])?, node(&e[2])?, eid[r], e[4].as_f64()));
                }
                for t in arr(mm, "transfers")? {
                    net.transfers.push((node(&t[0])?, t[1].as_u64().unwrap_or(0) as u8, t[2].as_u64().unwrap_or(0) as u8, t[3].as_f64(), text(&t[4])));
                }
                for (i, m) in modes.iter().enumerate() {
                    if let Some(g) = graphs.get(m) {
                        let turns = g.k.iter().flat_map(|&r| { let f = eid[r]; g.next.get(&f).into_iter().flatten().map(move |&t| (f, t)) }).collect();
                        net.turns.insert(i as u8, turns);
                        net.lengths.extend(g.k.iter().zip(&g.len).map(|(&r, l)| (eid[r], l.unwrap_or(0.0))));
                    }
                }
                let allowed: Vec<String> = ["walking", "driving"].iter().map(|m| m.to_string()).collect();
                Some(Prepared::new(net, Some(&allowed)))
            }
        };
        Ok(Area { modes, eid, src, tgt, name, hw, rb, line, bbox, graphs, mm })
    }

    fn mode(&self, m: &str) -> Result<&Mode> {
        self.graphs.get(m).ok_or_else(|| format!("mode {m} is not in the planner data"))
    }

    /// The road number of `e` (the page's index).
    pub fn road(&self, e: i64) -> Option<usize> {
        self.graphs.values().find_map(|g| g.at.get(&e).map(|&i| g.k[i])).or_else(|| self.eid.iter().position(|&x| x == e))
    }

    /// duckOSM `candidates` (duckosm-route's candidates on the database): the roads of `mode` near `point` and the blockers
    /// (the roads of `mode` and of `driving`). A road whose box meets the box of the radius stands for one that the
    /// database's ST_Intersects gives: the search's circle is in that box, so the results are the same.
    pub fn candidates(&self, mode: &str, point: Pt, radius_m: f64) -> Result<Vec<Cand>> {
        let (x0, y0, x1, y1) = points::bbox(point, radius_m);
        let near = |r: usize| { let b = self.bbox[r]; b[0] <= x1 && b[2] >= x0 && b[1] <= y1 && b[3] >= y0 };
        let g = self.mode(mode)?;
        let roads: Vec<Road> = g.k.iter().enumerate().filter(|(_, &r)| near(r))
            .map(|(i, &r)| Road { edge_id: self.eid[r], source: self.src[r], target: self.tgt[r], cost_s: g.cost[i], length_m: g.len[i], line: self.line[r].clone() })
            .collect();
        let mut blockers = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for sch in [mode, "driving"] {
            if seen.contains(&sch) {
                continue;
            }
            seen.push(sch);
            if let Some(b) = self.graphs.get(sch) {
                blockers.extend(b.k.iter().filter(|&&r| near(r)).map(|&r| self.line[r].clone()));
            }
        }
        Ok(points::candidates(point, radius_m, &roads, &blockers))
    }

    fn ends(&self, mode: &str, a: Pt, b: Pt, radius_m: f64) -> Result<(Vec<Cand>, Vec<Cand>)> {
        let (s, t) = (self.candidates(mode, a, radius_m)?, self.candidates(mode, b, radius_m)?);
        let r = py_g(radius_m);
        if s.is_empty() {
            return Err(format!("no road within {r} m of the start"));
        }
        if t.is_empty() {
            return Err(format!("no road within {r} m of the end"));
        }
        Ok((s, t))
    }

    /// duckOSM `route_points` (duckosm-route's `route_points`, the same JSON): the route between two points (lon, lat) in one
    /// mode, or null.
    pub fn route_points(&self, a: Pt, b: Pt, mode: &str, weight: &str, radius_m: f64, access_kmh: f64) -> Result<J> {
        if weight != "time" && weight != "length" {
            return Err("weight must be 'time' or 'length'".into());
        }
        let (s, t) = self.ends(mode, a, b, radius_m)?;
        let v = access_kmh / 3.6;
        let g = self.mode(mode)?;
        let w = if weight == "time" { &g.by_time } else { &g.by_len };
        let Some(r) = points::route_points(&s, &t, weight == "time", v, w, &g.next) else { return Ok(J::Null) };
        let mut rows = Vec::new();
        let (mut sum_cost, mut sum_len) = (0.0f64, 0.0f64);
        for (e, (f0, f1)) in r.path.iter().zip(&r.parts) {
            let i = g.at[e];
            let k = g.k[i];
            let share = f1 - f0;
            let (l, c) = (g.len[i].unwrap_or(0.0) * share, g.cost[i].unwrap_or(0.0) * share);
            sum_cost += c;
            sum_len += l;
            rows.push(json!({"edge_id": e, "name": self.name[k], "highway": self.hw[k], "from_fraction": f0, "to_fraction": f1, "length_m": l, "cost_s": c}));
        }
        let (start, end) = (end_info(a, &r.start, v), end_info(b, &r.end, v));
        Ok(json!({
            "time_s": start["access_s"].as_f64().unwrap_or(0.0) + sum_cost + end["access_s"].as_f64().unwrap_or(0.0),
            "length_m": r.start.access_m + sum_len + r.end.access_m,
            "edges": r.path,
            "start": start,
            "end": end,
            "path": rows,
        }))
    }

    /// duckOSM `route_multimodal_points` over walk + drive (duckosm-route's, with `allowed_modes` walking and driving; the same
    /// JSON without the WKT of each road): the trip between two points, starting and ending walking, or null. `weight`
    /// "length": the page's Shortest (`Weight::Length`), not in duckOSM.
    pub fn route_multimodal_points(&self, a: Pt, b: Pt, radius_m: f64, access_kmh: f64, weight: &str) -> Result<J> {
        let net = self.mm.as_ref().ok_or("walk + drive needs the multimodal tables: run `duckosm multimodal` on the database")?;
        let (s, t) = self.ends(HUB, a, b, radius_m)?;
        let v = access_kmh / 3.6;
        let pts = Points {
            starts: s.iter().map(|c| (c.edge_id, c.fraction, c.access_m / v)).collect(),
            ends: t.iter().map(|c| (c.edge_id, (c.fraction, c.access_m / v))).collect(),
        };
        let w = if weight == "length" { Weight::Length { v } } else { Weight::Time };
        let Some(steps) = net.route_by(0, 0, HUB, HUB, true, Some(&pts), w)? else { return Ok(J::Null) };
        let mut r = self.trip(net, &steps)?;
        let edges = r["edges"].as_array().cloned().unwrap_or_default();
        let (first, last) = (edges[0][1].as_i64().unwrap_or(0), edges[edges.len() - 1][1].as_i64().unwrap_or(0));
        let sc = s.iter().find(|c| c.edge_id == first).ok_or("the first road is not a start candidate")?;
        let tby: HashMap<i64, &Cand> = t.iter().map(|c| (c.edge_id, c)).collect();
        let tc = tby.get(&last).copied().ok_or("the last road is not an end candidate")?;
        r["start"] = end_info(a, sc, v);
        r["end"] = end_info(b, tc, v);
        let l = r["length_m"].as_f64().unwrap_or(0.0) + (sc.access_m + tc.access_m);
        r["length_m"] = json!(l);
        Ok(r)
    }

    /// duckosm-route's `route_multimodal` result of the steps of a trip between points (with `access_s`).
    fn trip(&self, net: &Prepared, steps: &[Step]) -> Result<J> {
        let mut edges_seq: Vec<(String, i64)> = Vec::new();
        let mut legs: Vec<(String, Vec<i64>, f64, Vec<f64>)> = Vec::new();
        let (mut transfers, mut transfer_costs) = (Vec::new(), Vec::new());
        let mut access_s = 0.0f64;
        let mut nodes: Vec<i64> = Vec::new();
        for step in steps {
            match step {
                Step::Edge(e, m, cost, frac, acc) => {
                    edges_seq.push((m.clone(), *e));
                    if legs.last().is_none_or(|l| &l.0 != m) {
                        legs.push((m.clone(), Vec::new(), 0.0, Vec::new()));
                    }
                    let l = legs.last_mut().expect("pushed");
                    l.1.push(*e);
                    l.3.push(*frac);
                    l.2 += cost;
                    access_s += acc;
                    let t = net.target(m, *e).ok_or(format!("edge {e} is not a {m} road"))?;
                    if nodes.last() != Some(&t) {
                        nodes.push(t);
                    }
                }
                Step::Transfer(n, fm, tm, cost, kind) => {
                    transfers.push(json!({"node_id": n, "from_mode": fm, "to_mode": tm, "cost_s": cost, "kind": kind}));
                    transfer_costs.push(*cost);
                }
            }
        }
        let mut length_m = 0.0f64;
        let (mut legs_out, mut leg_times) = (Vec::new(), Vec::new());
        for (mode, es, time, fr) in legs {
            let g = self.mode(&mode)?;
            let mut path = Vec::new();
            for (e, frac) in es.iter().zip(&fr) {
                let Some(&i) = g.at.get(e) else { continue };
                let k = g.k[i];
                let (len, cost) = (g.len[i], g.cost[i]);
                let mut row = json!({"edge_id": e, "name": self.name[k], "highway": self.hw[k], "length_m": len, "cost_s": cost});
                if *frac != 1.0 {
                    row["fraction"] = json!(frac);
                    row["length_m"] = json!(len.unwrap_or(0.0) * frac);
                    row["cost_s"] = json!(cost.unwrap_or(0.0) * frac);
                }
                path.push(row);
                if let Some(l) = len {
                    length_m += l * frac;
                }
            }
            leg_times.push(time);
            legs_out.push(json!({"mode": mode, "edges": es, "time_s": time, "path": path}));
        }
        let time_s = leg_times.iter().fold(0.0, |a, b| a + b) + transfer_costs.iter().fold(0.0, |a, b| a + b) + access_s;
        Ok(json!({
            "time_s": time_s,
            "length_m": length_m,
            "edges": edges_seq.iter().map(|(m, e)| json!([m, e])).collect::<Vec<_>>(),
            "legs": legs_out,
            "transfers": transfers,
            "nodes": nodes,
            "access_s": access_s,
        }))
    }

    /// duckOSM `directions` of a route result (duckosm-route's `directions`, the same JSON).
    pub fn directions(&self, route: &J, mode: &str) -> Result<J> {
        if route.is_null() || route.as_object().is_some_and(|o| o.is_empty()) {
            return Ok(json!([]));
        }
        let mut roads = Vec::new();
        for (m, ks, used) in legs(route, mode) {
            let g = self.mode(&m)?;
            for e in ks {
                let i = *g.at.get(&e).ok_or(format!("edge {e} is not in {m}.edges"))?;
                let k = g.k[i];
                roads.push(DirRoad {
                    mode: m.clone(),
                    name: self.name[k].clone(),
                    highway: self.hw[k].clone(),
                    length_m: used.get(&e).copied().or(g.len[i]).unwrap_or(f64::NAN),
                    roundabout: self.rb[k],
                    line: self.line[k].clone(),
                    out: g.out.get(&self.src[k]).into_iter().flatten().map(|&r| self.line[r].clone()).collect(),
                });
            }
        }
        Ok(J::Array(directions::directions(&roads).into_iter().map(|s| {
            let mut o = json!({"type": s.kind, "modifier": s.modifier, "name": s.name, "mode": s.mode, "at": [s.at.0, s.at.1], "length_m": s.length_m});
            if let Some(x) = s.exit {
                o["exit"] = json!(x);
            }
            o["text"] = json!(s.text);
            o
        }).collect()))
    }

    /// The page's answer to a request `{a, b, modes, weight, radius, kmh}`: `{res}` (the route in the page's shape: road numbers,
    /// the part of each road, the steps; `route` the result of duckOSM's function), `{res: null}` (no route), or `{error,
    /// empty: [start, end]}` (no road near a point).
    pub fn plan(&self, q: &J) -> Result<J> {
        let pt = |k: &str| -> Result<Pt> { Ok((q[k][0].as_f64().ok_or(format!("no point {k}"))?, q[k][1].as_f64().ok_or(format!("no point {k}"))?)) };
        let (a, b) = (pt("a")?, pt("b")?);
        let modes: Vec<String> = q["modes"].as_array().into_iter().flatten().filter_map(text).collect();
        let multi = modes.len() > 1;
        let (radius, kmh) = (q["radius"].as_f64().unwrap_or(50.0), q["kmh"].as_f64().unwrap_or(4.5));
        let weight = q["weight"].as_str().unwrap_or("time");
        let mode = if multi { HUB } else { modes.first().map(String::as_str).ok_or("no mode")? };
        let empty = [self.candidates(mode, a, radius)?.is_empty(), self.candidates(mode, b, radius)?.is_empty()];
        if empty[0] || empty[1] {
            return Ok(json!({"error": "no road", "empty": empty}));
        }
        let r = if multi { self.route_multimodal_points(a, b, radius, kmh, weight)? } else { self.route_points(a, b, mode, weight, radius, kmh)? };
        if r.is_null() {
            return Ok(json!({"res": null}));
        }
        let steps = self.directions(&r, mode)?;
        Ok(json!({"res": self.page_shape(&r, (!multi).then_some(mode), &steps)?}))
    }

    /// A route result in the page's shape: legs of road numbers with the part of each road ([from, to]), time and length;
    /// the two ends (`k`, `f`, `d`, `rp`); the steps (`mod`, `len`).
    fn page_shape(&self, r: &J, single: Option<&str>, steps: &J) -> Result<J> {
        let end = |e: &J| -> J { json!({"k": e["edge_id"].as_i64().and_then(|x| self.road(x)), "f": e["fraction"], "d": e["access_m"], "rp": e["road_point"]}) };
        let (start, stop) = (end(&r["start"]), end(&r["end"]));
        let mut legs = Vec::new();
        if let Some(mode) = single {
            let path = r["path"].as_array().cloned().unwrap_or_default();
            legs.push(json!({"mode": mode, "edges": r["edges"],
                             "time": path.iter().filter_map(|p| p["cost_s"].as_f64()).sum::<f64>(),
                             "len": path.iter().filter_map(|p| p["length_m"].as_f64()).sum::<f64>()}));
        } else {
            for l in r["legs"].as_array().into_iter().flatten() {
                let path = l["path"].as_array().cloned().unwrap_or_default();
                legs.push(json!({"mode": l["mode"], "edges": l["edges"], "time": l["time_s"],
                                 "len": path.iter().filter_map(|p| p["length_m"].as_f64()).sum::<f64>()}));
            }
        }
        // the part of each road: the first from the start's point, the last to the end's point
        let n: usize = legs.iter().map(|l| l["edges"].as_array().map_or(0, Vec::len)).sum();
        let mut i = 0;
        for l in &mut legs {
            let es: Vec<i64> = l["edges"].as_array().into_iter().flatten().filter_map(J::as_i64).collect();
            let mut parts = Vec::new();
            for _ in &es {
                let f0 = if i == 0 { start["f"].as_f64().unwrap_or(0.0) } else { 0.0 };
                let f1 = if i + 1 == n { stop["f"].as_f64().unwrap_or(1.0) } else { 1.0 };
                parts.push(json!([f0, f1]));
                i += 1;
            }
            l["parts"] = J::Array(parts);
            l["edges"] = J::Array(es.iter().map(|&e| json!(self.road(e))).collect());
        }
        let steps: Vec<J> = steps.as_array().into_iter().flatten().map(|s| json!({"type": s["type"], "mod": s["modifier"], "name": s["name"],
            "mode": s["mode"], "at": s["at"], "len": s["length_m"], "exit": s["exit"], "text": s["text"]})).collect();
        // the edge_ids as text too: JavaScript's numbers do not keep an id above 2**53 (the ids in `route` are rounded in a page)
        let ids: Vec<String> = r["edges"].as_array().into_iter().flatten().map(|e| e.as_array().and_then(|x| x.get(1)).unwrap_or(e).to_string()).collect();
        Ok(json!({"time": r["time_s"], "len": r["length_m"], "legs": legs, "edge_ids": ids, "transfers": r["transfers"].as_array().map_or(0, Vec::len),
                  "start": start, "end": stop, "steps": steps, "route": r}))
    }
}

fn end_info(point: Pt, c: &Cand, v: f64) -> J {
    json!({"point": [point.0, point.1], "road_point": [c.road_point.0, c.road_point.1], "edge_id": c.edge_id,
           "fraction": c.fraction, "access_m": c.access_m, "access_s": c.access_m / v})
}

/// Python's f"{x:g}" for a radius (a whole number has no ".0").
fn py_g(x: f64) -> String {
    let s = format!("{x}");
    s.strip_suffix(".0").map(String::from).unwrap_or(s)
}

/// duckOSM `_legs`: (mode, edge ids, metres used of each edge) of a route result.
fn legs(route: &J, mode: &str) -> Vec<(String, Vec<i64>, HashMap<i64, f64>)> {
    if let Some(l) = route["legs"].as_array().filter(|l| !l.is_empty()) {
        return l.iter().map(|x| (x["mode"].as_str().unwrap_or_default().to_string(), x["edges"].as_array().into_iter().flatten().filter_map(J::as_i64).collect(), HashMap::new())).collect();
    }
    let mut used = HashMap::new();
    for p in route["path"].as_array().into_iter().flatten() {
        if let (Some(e), Some(l)) = (p["edge_id"].as_i64(), p.get("length_m")) {
            used.insert(e, l.as_f64().unwrap_or(f64::NAN));
        }
    }
    vec![(mode.to_string(), route["edges"].as_array().into_iter().flatten().filter_map(J::as_i64).collect(), used)]
}
