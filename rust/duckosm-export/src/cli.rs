//! The export commands of `duckosm` (`duckosm matsim ...`), with the options of the Python CLI
//! (src/duckosm/cli.py).

use crate::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

/// The command names of this crate: `duckosm <name> ...` goes to [`run`].
pub const COMMANDS: [&str; 8] = ["matsim", "railml", "export-gis", "gis-debug", "opendrive", "matsim-lanes", "sumo", "lanelet2"];

#[derive(Parser)]
#[command(name = "duckosm")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Export a built network to GeoPackage, GeoParquet or shapefile, with edge_id as an attribute: the layers edges_<mode>,
    /// nodes_<mode> and boundary, in EPSG:4326. GeoPackage: one file (ogr2ogr of GDAL puts the layers together). Shapefile:
    /// one set of files for each layer. GeoParquet: one .parquet file for each layer and for each features.* table, and
    /// layers.json; the edges files also get the columns of visualization.edge_levels.
    #[command(name = "export-gis")]
    ExportGis {
        /// The duckOSM database.
        db: PathBuf,
        /// Mode schema(s) to export (default: each mode of the db).
        #[arg(short, long = "mode")]
        modes: Vec<String>,
        /// gpkg, shp or geoparquet.
        #[arg(short, long = "format", default_value = "gpkg", value_parser = ["gpkg", "shp", "geoparquet"])]
        fmt: String,
        /// gpkg: the .gpkg file; shp, geoparquet: the folder (default: <name>.gpkg, <name>_gis/, <name>_geoparquet/).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Do not export main.boundary as a boundary layer.
        #[arg(long)]
        no_boundary: bool,
        /// The base name of the layers and files (default: the file name of the db).
        #[arg(long)]
        name: Option<String>,
    },
    /// Check an export: read the GeoPackage or the shapefiles again through GDAL and write one HTML page: a map of each layer
    /// and the checks (the number of features, the CRS, edge_id exact, and the edge_id against --source-db).
    #[command(name = "gis-debug")]
    GisDebug {
        /// A .gpkg file or a folder of shapefiles.
        export_path: PathBuf,
        /// The built duckOSM db to compare with (each exported edge_id must be in it).
        #[arg(long)]
        source_db: Option<PathBuf>,
        /// The output HTML (default: <name>_gis_debug.html).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// The name on the page (default: the file name of the export).
        #[arg(long)]
        name: Option<String>,
        /// Also print the checks as JSON: the verdict, the summary, the checks of each layer.
        #[arg(long)]
        json: bool,
    },
    /// Export a MATSim network.xml from a built duckOSM db: each edge is one directed link (edge_id is the link id), with
    /// length, freespeed, capacity, permlanes and modes; the node coordinates are in a metric CRS.
    Matsim {
        /// The duckOSM database.
        db: PathBuf,
        /// Mode(s): one mode, all, or a comma list (for example driving,cycling): one network with the modes of each link.
        #[arg(short, long, default_value = "driving")]
        mode: String,
        /// The projected metric CRS of the node coordinates (default: the UTM zone of the data; for example EPSG:3006).
        #[arg(long)]
        crs: Option<String>,
        /// Do not gzip the output (gzip is the MATSim convention and the default).
        #[arg(long)]
        no_gzip: bool,
        /// The output file (default: <name>_network.xml[.gz]).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Export an ASAM OpenDRIVE .xodr from a built duckOSM db: roads with a reference line in a metric CRS and the driving
    /// lanes. Phase 1 (default): geometry and lanes. --junctions adds the junctions and their connecting roads (a GMNS db).
    Opendrive {
        /// The duckOSM database (with --junctions: a GMNS db).
        db: PathBuf,
        /// The mode schema to export.
        #[arg(short, long, default_value = "driving")]
        mode: String,
        /// The projected metric CRS of the reference line (default: the UTM zone of the data).
        #[arg(long)]
        crs: Option<String>,
        /// Phase 2: the junctions and the connecting roads of the turns (needs a GMNS db, not the core db).
        #[arg(long)]
        junctions: bool,
        /// The output .xodr (default: <name>.xodr).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Export MATSim lanes.xml (and signalSystems, signalGroups, signalControl.xml) from a GMNS db: the turn lanes from the
    /// movement table, the signals of the signal nodes with a default fixed-time plan. The link ids are the edge_id of
    /// duckosm matsim.
    #[command(name = "matsim-lanes")]
    MatsimLanes {
        /// The GMNS database.
        gmns_db: PathBuf,
        /// The GMNS mode schema.
        #[arg(short, long, default_value = "driving")]
        mode: String,
        /// Do not write the signal files.
        #[arg(long)]
        no_signals: bool,
        /// The default signal cycle time (seconds).
        #[arg(long, default_value_t = 90)]
        cycle: i64,
        /// The output folder.
        #[arg(short, long = "out", default_value = ".")]
        out_dir: PathBuf,
    },
    /// Export a built network to a SUMO net, with the duckOSM edge_id as the SUMO edge id. Writes <out-dir>/<name>.{nod,edg,con}.xml,
    /// a .netccfg and the .net.xml (not with --no-netconvert). The connections come from edge_graph, so the turn restrictions
    /// stay. netconvert comes with SUMO (`pip install duckosm[sumo]` or a system package).
    Sumo {
        /// The duckOSM database.
        db: PathBuf,
        /// The mode schema to export.
        #[arg(short, long, default_value = "driving")]
        mode: String,
        /// The output folder of <name>.{nod,edg,net}.xml.
        #[arg(long, default_value = "sumo")]
        out_dir: PathBuf,
        /// The base name of the files (default: the file name of the db).
        #[arg(long)]
        name: Option<String>,
        /// Do not write the connections of edge_graph (netconvert makes its own: the turn restrictions are lost).
        #[arg(long)]
        no_connections: bool,
        /// A netconvert config (.netccfg) to use in place of the built-in options.
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Do not run netconvert: write only the plain XML.
        #[arg(long)]
        no_netconvert: bool,
    },
    /// Export a Lanelet2 map (.osm) from a GMNS db: each lane is a lanelet, with left and right boundaries (the centerline
    /// offset by half the lane width) and the tags subtype, one_way and speed_limit. A lane map in the format of Autoware and
    /// the lanelet2 library, not a survey-grade HD map.
    Lanelet2 {
        /// The GMNS database.
        gmns_db: PathBuf,
        /// The GMNS mode schema.
        #[arg(short, long, default_value = "driving")]
        mode: String,
        /// The output .osm (default: <name>.lanelet2.osm).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Export a railML 2.4 rail infrastructure file from a built duckOSM db: the railway ways of the raw OSM, cut into
    /// tracks with their topology, switches, signals and stations (OCPs).
    Railml {
        /// The duckOSM database.
        db: PathBuf,
        /// The output file (default: <name>.railml.xml).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

/// A database file that is named like a duckOSM schema is refused, as `check_db_name` of the Python CLI.
fn check_db(db: &Path) -> Result<()> {
    if !db.exists() {
        return Err(format!("{}: no such file", db.display()));
    }
    let stem = db.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    if ["driving", "walking", "cycling", "mm", "raw", "features", "main"].contains(&stem.as_str()) {
        return Err(format!(
            "a database can't be called {}: DuckDB names a database after its file, and '{stem}' is also one of duckOSM's schemas",
            db.display()
        ));
    }
    Ok(())
}

fn stem(db: &Path) -> String {
    db.file_stem().and_then(|s| s.to_str()).unwrap_or("network").to_string()
}

/// Run one export command. `args[0]` is the command name.
pub fn run(args: &[String]) -> Result<()> {
    let cli = Cli::parse_from(std::iter::once("duckosm".to_string()).chain(args.iter().cloned()));
    match cli.cmd {
        Cmd::Matsim { db, mode, crs, no_gzip, out } => {
            check_db(&db)?;
            let out = out.unwrap_or_else(|| format!("{}_network.xml{}", stem(&db), if no_gzip { "" } else { ".gz" }).into());
            let s = crate::matsim::to_matsim(&db, &out, &mode, crs.as_deref(), !no_gzip)?;
            println!("wrote {} — {} nodes, {} links (CRS {})", out.display(), s.nodes, s.links, s.crs);
        }
        Cmd::ExportGis { db, modes, fmt, out, no_boundary, name } => {
            check_db(&db)?;
            let base = name.unwrap_or_else(|| stem(&db));
            let out = out.unwrap_or_else(|| {
                match fmt.as_str() {
                    "gpkg" => format!("{base}.gpkg"),
                    "shp" => format!("{base}_gis"),
                    _ => format!("{base}_geoparquet"),
                }
                .into()
            });
            let s = crate::gis::to_gis(&db, &out, &modes, &fmt, !no_boundary, Some(&base), "EPSG:4326")?;
            let layers = s.layers.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ");
            let dest = s.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "None".into());
            println!("wrote {dest} ({}: {} layers — {layers})", s.fmt, s.layers.len());
            if s.fmt == "gpkg" && s.path.is_none() {
                println!("  (ogr2ogr not found — wrote {} single-layer files instead)", s.files.len());
            }
        }
        Cmd::Opendrive { db, mode, crs, junctions, out } => {
            check_db(&db)?;
            let out = out.unwrap_or_else(|| format!("{}.xodr", stem(&db)).into());
            let s = crate::opendrive::to_opendrive(&db, &out, &mode, crs.as_deref(), junctions)?;
            let extra = if junctions { format!(" + {} connecting roads, {} junctions", s.connecting_roads, s.junctions) } else { String::new() };
            println!("wrote {} — {} roads{extra} (CRS {})", out.display(), s.roads, s.crs);
        }
        Cmd::MatsimLanes { gmns_db, mode, no_signals, cycle, out_dir } => {
            if !gmns_db.exists() {
                return Err(format!("{}: no such file", gmns_db.display()));
            }
            let s = crate::matsim_lanes::to_matsim_lanes(&gmns_db, &out_dir, &mode, !no_signals, cycle)?;
            let mut msg = format!("wrote lanes.xml ({} assignments)", s.lanes_links);
            if let Some(n) = s.signal_systems {
                msg.push_str(&format!(" + signals ({n} systems)"));
            }
            println!("{msg} -> {}", out_dir.display());
        }
        Cmd::Sumo { db, mode, out_dir, name, no_connections, config, no_netconvert } => {
            check_db(&db)?;
            if let Some(c) = config.as_ref().filter(|c| !c.exists()) {
                return Err(format!("{}: no such file", c.display()));
            }
            let name = name.unwrap_or_else(|| stem(&db));
            let s = crate::sumo::to_sumo(&db, &out_dir, &mode, &name, !no_connections, config.as_deref(), !no_netconvert)?;
            println!("wrote {} ({} edges) and {} ({} nodes)", s.edg.display(), s.n_edges, s.nod.display(), s.n_nodes);
            if let Some(c) = &s.con {
                println!("wrote {} ({} connections)", c.display(), s.n_connections);
            }
            if let Some(n) = &s.net {
                println!("wrote {}", n.display());
            }
        }
        Cmd::GisDebug { export_path, source_db, out, name, json } => {
            for p in std::iter::once(&export_path).chain(source_db.as_ref()) {
                if !p.exists() {
                    return Err(format!("{}: no such file", p.display()));
                }
            }
            let out = out.unwrap_or_else(|| {
                let s = export_path.file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.is_empty());
                format!("{}_gis_debug.html", s.unwrap_or_else(|| "export".into())).into()
            });
            let p = crate::gis_debug::write_debug(&export_path, source_db.as_deref(), &out, name.as_deref())?;
            if json {
                use crate::gis_debug::J;
                let pick = |k: &str| (k.to_string(), p.get(k).cloned().unwrap_or(J::Null));
                let j = J::Obj(vec![("out".into(), J::Str(out.display().to_string())), pick("verdict"), pick("summary"), pick("layers")]);
                println!("{}", j.dump(Some(1)));
            } else {
                println!("wrote {} — open in a browser", out.display());
            }
        }
        Cmd::Lanelet2 { gmns_db, mode, out } => {
            if !gmns_db.exists() {
                return Err(format!("{}: no such file", gmns_db.display()));
            }
            let out = out.unwrap_or_else(|| format!("{}.lanelet2.osm", stem(&gmns_db)).into());
            let s = crate::lanelet2::to_lanelet2(&gmns_db, &out, &mode, 0.01)?;
            println!("wrote {} — {} lanelets, {} boundary ways, {} nodes", out.display(), s.lanelets, s.ways, s.nodes);
        }
        Cmd::Railml { db, out } => {
            check_db(&db)?;
            let out = out.unwrap_or_else(|| format!("{}.railml.xml", stem(&db)).into());
            let s = crate::railml::to_railml(&db, &out)?;
            println!(
                "wrote {} — {} tracks, {} switches, {} signals, {} OCPs",
                out.display(),
                s.tracks,
                s.switches,
                s.signals,
                s.ocps
            );
        }
    }
    Ok(())
}
