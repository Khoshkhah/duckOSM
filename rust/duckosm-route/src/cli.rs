//! The routing commands of `duckosm-rust`, with the options of the Python CLI (src/duckosm/cli.py) where it has the command.

use crate::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

/// The command names of this crate: `duckosm-rust <name> ...` goes to [`run`].
pub const COMMANDS: [&str; 2] = ["route", "export-graph"];

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
