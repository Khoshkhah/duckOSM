//! The routing commands of `duckosm-rust`, with the options of the Python CLI (src/duckosm/cli.py) where it has the command.

use crate::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

/// The command names of this crate: `duckosm-rust <name> ...` goes to [`run`].
pub const COMMANDS: [&str; 8] =
    ["route", "export-graph", "route-multimodal", "route-points", "route-multimodal-points", "directions", "lane-graph", "route-lanes"];

#[derive(Parser)]
#[command(name = "duckosm-rust")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// The shortest route between two edge ids on the edge graph of a mode (duckOSM `routing.route`): one JSON object with
    /// `edges`, `time_s`, `length_m` and `path` (each edge with its name, highway, length_m, cost_s and WKT), or null when
    /// they are not connected. With --pairs, a JSON list of the routes of many pairs (the graph is built once, as
    /// duckOSM's `Router` does).
    Route {
        db: PathBuf,
        #[arg(allow_hyphen_values = true, required_unless_present = "pairs")]
        from_edge: Option<i64>,
        #[arg(allow_hyphen_values = true, required_unless_present = "pairs")]
        to_edge: Option<i64>,
        /// A file with one pair of edge ids on each line ("FROM TO").
        #[arg(long, conflicts_with_all = ["from_edge", "to_edge"])]
        pairs: Option<PathBuf>,
        /// Mode schema.
        #[arg(short, long, default_value = "driving")]
        mode: String,
        /// The fastest route (time) or the shortest (length).
        #[arg(long, default_value = "time", value_parser = ["time", "length"])]
        weight: String,
    },
    /// The fastest trip across modes between two junction node_ids over the mm.* graph (duckOSM `routing.route_multimodal`):
    /// one JSON object (time_s, length_m, edges, legs, transfers, nodes) or null. Every part follows the edge graph of its
    /// mode. With --batch, a JSON list for the pairs of a file ("SRC DST" on each line), the graph read once.
    #[command(name = "route-multimodal")]
    RouteMultimodal {
        db: PathBuf,
        #[arg(allow_hyphen_values = true, required_unless_present = "batch")]
        src: Option<i64>,
        #[arg(allow_hyphen_values = true, required_unless_present = "batch")]
        dst: Option<i64>,
        #[arg(long, conflicts_with_all = ["src", "dst"])]
        batch: Option<PathBuf>,
        /// The multimodal schema.
        #[arg(long, default_value = "mm")]
        schema: String,
        #[arg(long, default_value = "walking")]
        start_mode: String,
        #[arg(long, default_value = "walking")]
        end_mode: String,
        /// Any change of mode that the transfers allow (default: walk* (drive|cycle)* walk*).
        #[arg(long)]
        no_enforce_sequence: bool,
        /// Only these modes, a comma list (for example walking,cycling: a car-free trip).
        #[arg(long)]
        modes: Option<String>,
    },
    /// The route in one mode between two points (lon, lat) (duckOSM `point_routing.route_points`): each point joins its roads
    /// within --radius, the walk to the road at --access-kmh. One JSON object or null. With --batch, a JSON list for the
    /// points of a file ("LON LAT LON LAT" on each line).
    #[command(name = "route-points")]
    RoutePoints {
        db: PathBuf,
        /// LON LAT LON LAT.
        #[arg(num_args = 4, allow_hyphen_values = true, required_unless_present = "batch")]
        coords: Vec<f64>,
        #[arg(long, conflicts_with = "coords")]
        batch: Option<PathBuf>,
        #[arg(short, long, default_value = "driving")]
        mode: String,
        #[arg(long, default_value = "time", value_parser = ["time", "length"])]
        weight: String,
        #[arg(long, default_value_t = 50.0)]
        radius: f64,
        #[arg(long, default_value_t = 4.5)]
        access_kmh: f64,
    },
    /// The trip across modes between two points (lon, lat), starting and ending walking (duckOSM
    /// `point_routing.route_multimodal_points`). One JSON object or null. With --batch, a JSON list for the points of a file.
    #[command(name = "route-multimodal-points")]
    RouteMultimodalPoints {
        db: PathBuf,
        /// LON LAT LON LAT.
        #[arg(num_args = 4, allow_hyphen_values = true, required_unless_present = "batch")]
        coords: Vec<f64>,
        #[arg(long, conflicts_with = "coords")]
        batch: Option<PathBuf>,
        #[arg(long, default_value_t = 50.0)]
        radius: f64,
        #[arg(long, default_value_t = 4.5)]
        access_kmh: f64,
        #[arg(long, default_value = "mm")]
        schema: String,
        /// Only these modes, a comma list (it must have walking).
        #[arg(long)]
        modes: Option<String>,
        #[arg(long)]
        no_enforce_sequence: bool,
    },
    /// Turn-by-turn steps for a route (duckOSM `directions.directions`): ROUTE is a JSON file with a route result, as the route
    /// commands print it (or a list of them; "-" reads it from the standard input). One JSON list of steps, or a list of lists.
    Directions {
        db: PathBuf,
        route: PathBuf,
        /// The mode of a route of one mode (route, route-points); a route across modes has its modes in its legs.
        #[arg(short, long, default_value = "driving")]
        mode: String,
    },
    /// Build a lane-level routing graph in a GMNS db: lane->lane turn edges (from movements) + lane-change edges (adjacent
    /// lanes), written as lane_<mode>.lane_edges.
    #[command(name = "lane-graph")]
    LaneGraph {
        gmns_db: PathBuf,
        /// GMNS mode schema.
        #[arg(short, long, default_value = "driving")]
        mode: String,
    },
    /// Plan a lane-level route between two lanes (each a lane_id or an edge_id -> its lane 1). Prints the lane count / cost /
    /// maneuvers; with -o writes the route geometry as GeoJSON.
    #[command(name = "route-lanes")]
    RouteLanes {
        gmns_db: PathBuf,
        #[arg(allow_hyphen_values = true)]
        from_lane: String,
        #[arg(allow_hyphen_values = true)]
        to_lane: String,
        /// GMNS mode schema.
        #[arg(short, long, default_value = "driving")]
        mode: String,
        /// Write the route as GeoJSON to this path.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Print the route as JSON: lanes, cost, maneuvers, geometry (WKT).
        #[arg(long = "json")]
        as_json: bool,
    },
    /// Export a built network as a graph file (GraphML). By default the geographic node-based MultiDiGraph (OSM junctions as
    /// nodes, road segments keyed by edge_id carrying their full attribute set); --graph edge for the edge-based routing
    /// DiGraph. GraphML is scalar-only: lists and geometry are text.
    #[command(name = "export-graph")]
    ExportGraph {
        db: PathBuf,
        /// Mode schema to export.
        #[arg(short, long, default_value = "driving")]
        mode: String,
        /// Output file (default: <name>_<mode>.<ext>). Extension picks the format.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// 'node': geographic MultiDiGraph (junctions + full edge info); 'edge': edge-based routing DiGraph.
        #[arg(short = 'g', long = "graph", default_value = "node", value_parser = ["node", "edge"])]
        graph_kind: String,
        /// Output format (default: inferred from --out extension, else graphml). gpickle is a Python pickle of a networkx
        /// graph: only duckOSM's Python package writes it.
        #[arg(short = 'f', long = "format", value_parser = ["graphml", "gpickle"])]
        fmt: Option<String>,
        /// Arc weight for --graph edge (ignored for node graphs).
        #[arg(long, default_value = "time", value_parser = ["time", "length"])]
        weight: String,
        /// Edge geometry for --graph node (shapely falls back to WKT for graphml).
        #[arg(long, default_value = "wkt", value_parser = ["wkt", "shapely", "none"])]
        geometry: String,
        /// More turns for --graph edge, a comma list of bus, private, pedestrian, delivery: the turns into and out of those
        /// restricted edges (default: the mode's own turns only).
        #[arg(long)]
        uses: Option<String>,
    },
}

fn need_db(db: &Path) -> Result<()> {
    if db.exists() { Ok(()) } else { Err(format!("database not found: {}", db.display())) }
}

fn mode_set(modes: Option<&str>) -> Option<std::collections::HashSet<String>> {
    modes.map(|m| m.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
}

/// One query (its words) as one JSON value, or each line of `batch` (`n` words) as a JSON list; an error of a line is
/// `{"error": text}` in its place.
fn print_one_or_batch(batch: Option<&Path>, n: usize, one: Option<Vec<String>>, f: impl Fn(&[String]) -> Result<serde_json::Value>) -> Result<()> {
    let v = match (batch, one) {
        (Some(file), _) => {
            let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let mut list = Vec::new();
            for l in text.lines().filter(|l| !l.trim().is_empty()) {
                let words: Vec<String> = l.split_whitespace().map(String::from).collect();
                if words.len() != n {
                    return Err(format!("{}: not {n} numbers: {l}", file.display()));
                }
                list.push(f(&words).unwrap_or_else(|e| serde_json::json!({"error": e})));
            }
            serde_json::Value::Array(list)
        }
        (None, Some(v)) => f(&v)?,
        _ => return Err("no query: give the numbers or --batch FILE".into()),
    };
    println!("{}", duckosm_tools::json::dumps(&v));
    Ok(())
}

fn nums<T: std::str::FromStr>(w: &[String]) -> Result<Vec<T>> {
    w.iter().map(|x| x.parse::<T>().map_err(|_| format!("not a number: {x}"))).collect()
}

/// Run one command; `args` starts with the command name.
pub fn run(args: &[String]) -> Result<()> {
    let cli = Cli::parse_from(std::iter::once("duckosm-rust".to_string()).chain(args.iter().cloned()));
    match cli.cmd {
        Cmd::Route { db, from_edge, to_edge, pairs, mode, weight } => {
            need_db(&db)?;
            let con = crate::route::open(&db)?;
            let g = crate::route::routing_graph(&con, &mode, &weight, &[])?;
            let one = |a: i64, b: i64| -> Result<serde_json::Value> {
                Ok(crate::route::route(&con, &g, a, b, &mode)?.unwrap_or(serde_json::Value::Null))
            };
            let v = match (from_edge, to_edge, pairs) {
                (Some(a), Some(b), None) => one(a, b)?,
                (_, _, Some(f)) => {
                    let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
                    let mut list = Vec::new();
                    for l in text.lines().filter(|l| !l.trim().is_empty()) {
                        let ids: Vec<i64> = l.split_whitespace().map(|x| x.parse().map_err(|_| format!("{}: not two edge ids: {l}", f.display()))).collect::<Result<_>>()?;
                        let [a, b] = ids[..] else { return Err(format!("{}: not two edge ids: {l}", f.display())) };
                        list.push(one(a, b)?);
                    }
                    serde_json::Value::Array(list)
                }
                _ => return Err("give FROM_EDGE TO_EDGE or --pairs FILE".into()),
            };
            println!("{}", duckosm_tools::json::dumps(&v));
        }
        Cmd::RouteMultimodal { db, src, dst, batch, schema, start_mode, end_mode, no_enforce_sequence, modes } => {
            need_db(&db)?;
            let con = crate::route::open(&db)?;
            let net = crate::points::network(&con, &schema)?;
            let allowed = mode_set(modes.as_deref());
            let one = |w: &[String]| {
                let v: Vec<i64> = nums(w)?;
                crate::points::route_multimodal(&con, &net, Some(v[0]), Some(v[1]), &schema, &start_mode, &end_mode, !no_enforce_sequence, allowed.as_ref(), None)
            };
            print_one_or_batch(batch.as_deref(), 2, src.zip(dst).map(|(a, b)| vec![a.to_string(), b.to_string()]), one)?;
        }
        Cmd::RoutePoints { db, coords, batch, mode, weight, radius, access_kmh } => {
            need_db(&db)?;
            let con = crate::route::open(&db)?;
            let one = |w: &[String]| {
                let v: Vec<f64> = nums(w)?;
                crate::points::route_points(&con, (v[0], v[1]), (v[2], v[3]), &mode, &weight, radius, access_kmh)
            };
            print_one_or_batch(batch.as_deref(), 4, (!coords.is_empty()).then(|| coords.iter().map(|x| x.to_string()).collect()), one)?;
        }
        Cmd::RouteMultimodalPoints { db, coords, batch, radius, access_kmh, schema, modes, no_enforce_sequence } => {
            need_db(&db)?;
            let con = crate::route::open(&db)?;
            let net = crate::points::network(&con, &schema)?;
            let allowed = mode_set(modes.as_deref());
            let one = |w: &[String]| {
                let v: Vec<f64> = nums(w)?;
                crate::points::route_multimodal_points(&con, &net, (v[0], v[1]), (v[2], v[3]), radius, access_kmh, &schema, allowed.as_ref(), !no_enforce_sequence)
            };
            print_one_or_batch(batch.as_deref(), 4, (!coords.is_empty()).then(|| coords.iter().map(|x| x.to_string()).collect()), one)?;
        }
        Cmd::Directions { db, route, mode } => {
            need_db(&db)?;
            let text = if route.as_os_str() == "-" {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).map_err(|e| e.to_string())?;
                s
            } else {
                std::fs::read_to_string(&route).map_err(|e| format!("{}: {e}", route.display()))?
            };
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", route.display()))?;
            let con = crate::route::open(&db)?;
            let out = match &v {
                serde_json::Value::Array(list) => serde_json::Value::Array(
                    list.iter().map(|r| crate::directions::directions(&con, r, &mode).unwrap_or_else(|e| serde_json::json!({"error": e}))).collect(),
                ),
                r => crate::directions::directions(&con, r, &mode)?,
            };
            println!("{}", duckosm_tools::json::dumps(&out));
        }
        Cmd::LaneGraph { gmns_db, mode } => {
            if !gmns_db.exists() {
                return Err(format!("Invalid value for 'GMNS_DB': Path '{}' does not exist.", gmns_db.display()));
            }
            let (lanes, lc, turn) = crate::lanes::build_lane_graph(&gmns_db, &mode)?;
            let t = duckosm_tools::info::thousands;
            eprintln!("lane graph[{mode}]: {} lanes, {} edges ({} turn + {} lane-change) -> lane_{mode}.lane_edges", t(lanes), t(lc + turn), t(turn), t(lc));
            println!("built lane graph — {lanes} lanes, {} edges ({turn} turn + {lc} lane-change)", lc + turn);
        }
        Cmd::RouteLanes { gmns_db, from_lane, to_lane, mode, out, as_json } => {
            if !gmns_db.exists() {
                return Err(format!("Invalid value for 'GMNS_DB': Path '{}' does not exist.", gmns_db.display()));
            }
            let res = crate::lanes::route_lanes(&gmns_db, &from_lane, &to_lane, &mode)?;
            let lanes = res["lanes"].as_array().map_or(0, Vec::len);
            if as_json {
                let mut o = serde_json::json!({"from": from_lane, "to": to_lane});
                for (k, v) in res.as_object().expect("an object") {
                    o[k] = v.clone();
                }
                println!("{}", duckosm_tools::json::dumps(&o));
            } else if lanes == 0 {
                println!("no lane route from {from_lane} to {to_lane}");
                return Ok(());
            } else {
                let m: Vec<&str> = res["maneuvers"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
                println!("route: {lanes} lanes, cost {:.0}, maneuvers: {}", res["cost"].as_f64().unwrap_or(0.0), if m.is_empty() { "(none)".to_string() } else { m.join(", ") });
            }
            if let (Some(path), Some(wkt)) = (out, res["geometry"].as_str()) {
                let body = &wkt[wkt.find('(').map_or(0, |i| i + 1)..wkt.rfind(')').unwrap_or(wkt.len())];
                let coords: Vec<serde_json::Value> = body
                    .split(',')
                    .map(|p| serde_json::Value::Array(p.split_whitespace().take(2).filter_map(|x| x.parse::<f64>().ok()).map(|x| serde_json::json!(x)).collect()))
                    .collect();
                let fc = serde_json::json!({"type": "FeatureCollection", "features": [{"type": "Feature",
                    "properties": {"lanes": lanes, "cost": res["cost"], "maneuvers": res["maneuvers"]},
                    "geometry": {"type": "LineString", "coordinates": coords}}]});
                std::fs::write(&path, duckosm_build::place::py_dumps(&fc)).map_err(|e| format!("{}: {e}", path.display()))?;
                if !as_json {
                    println!("wrote {}", path.display());
                }
            }
        }
        Cmd::ExportGraph { db, mode, out, graph_kind, fmt, weight, geometry, uses } => {
            need_db(&db)?;
            let out = out.unwrap_or_else(|| {
                let ext = if fmt.as_deref().unwrap_or("graphml") == "graphml" { "graphml" } else { "gpickle" };
                PathBuf::from(format!("{}_{mode}.{ext}", db.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()))
            });
            let ext = out.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let fmt = fmt.unwrap_or_else(|| match ext.as_str() {
                "gpickle" | "pkl" | "pickle" => "gpickle".into(),
                _ => "graphml".into(),
            });
            if fmt == "gpickle" {
                return Err("gpickle is a Python pickle of a networkx graph: use duckOSM's Python package for it, or GraphML".into());
            }
            let uses = crate::route::check_uses(uses.as_deref())?;
            if !uses.is_empty() && graph_kind != "edge" {
                return Err("uses is for the edge graph (graph='edge'): its turns carry the labels".into());
            }
            let con = crate::route::open(&db)?;
            let doc = if graph_kind == "node" {
                let geom = geometry != "none";
                let d = crate::graphml::node_graph(&con, &mode, geom)?;
                let geo = if geometry == "shapely" { "wkt" } else { geometry.as_str() };
                eprintln!(
                    "node graph[{mode}]: {} nodes, {} edges, geometry={geo}",
                    duckosm_tools::info::thousands(d.nodes.len() as i64),
                    duckosm_tools::info::thousands(d.edges.len() as i64)
                );
                d
            } else {
                let d = crate::graphml::edge_graph(&con, &mode, &weight, &uses)?;
                eprintln!(
                    "routing graph[{mode}]: {} edges, {} arcs, weight={weight}, +node attrs",
                    duckosm_tools::info::thousands(d.nodes.len() as i64),
                    duckosm_tools::info::thousands(d.edges.len() as i64)
                );
                d
            };
            if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&out, doc.write()).map_err(|e| format!("{}: {e}", out.display()))?;
            let (n, m) = (doc.nodes.len(), doc.edges.len());
            eprintln!(
                "wrote {graph_kind} graph[{mode}] -> {} ({fmt}: {} nodes, {} edges)",
                out.display(),
                duckosm_tools::info::thousands(n as i64),
                duckosm_tools::info::thousands(m as i64)
            );
            println!("wrote {} ({fmt}: {n} nodes, {m} edges)", out.display());
        }
    }
    Ok(())
}
