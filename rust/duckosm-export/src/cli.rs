//! The export commands of `duckosm` (`duckosm matsim ...`), with the options of the Python CLI
//! (src/duckosm/cli.py).

use crate::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

/// The command names of this crate: `duckosm <name> ...` goes to [`run`].
pub const COMMANDS: [&str; 10] = ["matsim", "railml", "export-gis", "gis-debug", "opendrive", "matsim-lanes", "sumo", "lanelet2", "gmns", "gmns-viz"];

#[derive(Parser)]
#[command(name = "duckosm")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Export a built network to GeoPackage, GeoParquet or shapefile, with edge_id as an attribute: the layers edges_<mode>,
    /// nodes_<mode> and boundary, in EPSG:4326. GeoPackage: one file with all layers. Shapefile:
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
    /// Write a self-contained interactive HTML viewer for a GMNS db: the lanes (offset by use), and the meso and micro
    /// networks where the db has them (`gmns --meso --micro`); hover a line for its type, id and attributes.
    #[command(name = "gmns-viz")]
    GmnsViz {
        /// The GMNS db (`duckosm gmns`).
        gmns_db: PathBuf,
        /// The mode schema to view.
        #[arg(short, long, default_value = "driving")]
        mode: String,
        /// The output HTML (default: <name>_viewer.html).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
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
    /// Extract a built network to a standalone GMNS DuckDB: every GMNS table OSM can support (config, node, link, geometry,
    /// lane, movement, use_definition/use_group, signal_controller, curb_seg), with native geometry and lane detail, keeping
    /// duckOSM edge_id as link_id. Units: length metres, free_speed km/h, coordinates EPSG:4326. Needs netconvert (SUMO) for
    /// the lanes of the movements.
    Gmns {
        /// The duckOSM database.
        db: PathBuf,
        /// Output GMNS .duckdb file (default: <name>_gmns.duckdb).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Mode schema(s) to extract (default: every mode present in the db).
        #[arg(short, long = "mode")]
        modes: Vec<String>,
        /// Also dump spec-standard GMNS CSVs into this directory.
        #[arg(long)]
        to_csv: Option<PathBuf>,
        /// No per-lane offset geometry.
        #[arg(long)]
        no_lane_geometry: bool,
        /// Also build a mesoscopic (lane-level) network: meso_<mode> schemas.
        #[arg(long)]
        meso: bool,
        /// Modes to build meso for (default: driving).
        #[arg(long = "meso-mode")]
        meso_modes: Vec<String>,
        /// Also write a single mode-tagged gmns_all network (links merged, allowed_uses unioned).
        #[arg(long)]
        combined: bool,
        /// Traffic side: two-way lanes offset to this side so the directions separate.
        #[arg(long, default_value = "right", value_parser = ["right", "left"])]
        drive_side: String,
        /// Do not place a one-way road mapped next to its opposite direction as one two-way road.
        #[arg(long)]
        no_pair_carriageways: bool,
        /// Move a mapped sidewalk off the carriageway: it takes its place from its road's lanes.
        #[arg(long)]
        walk_frame: bool,
        /// Gap in metres between the kerb and a footpath placed by --walk-frame.
        #[arg(long, default_value_t = 0.0)]
        walk_clearance: f64,
        /// Also build a microscopic (cell-based) network: micro_<mode> schemas.
        #[arg(long)]
        micro: bool,
        /// Modes to build micro for (default: driving).
        #[arg(long = "micro-mode")]
        micro_modes: Vec<String>,
        /// A GTFS feed (zip file or folder), repeatable: its stops become location rows with their gtfs_stop_id.
        #[arg(long)]
        gtfs: Vec<PathBuf>,
        /// How far from a link, in metres, a GTFS stop may be and still be put on it.
        #[arg(long, default_value_t = 30.0)]
        gtfs_max_m: f64,
        /// Keep duckOSM's extra columns in the CSVs as user-defined fields with a u_ prefix.
        #[arg(long)]
        csv_extensions: bool,
        /// Check the values of the result (ends on nodes, lengths, lane numbers, movements); exit 1 if a check fails.
        #[arg(long)]
        check: bool,
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

/// An integer with a comma between each group of three digits, as Python's `{:,}`.
pub(crate) fn thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
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
        Cmd::GmnsViz { gmns_db, mode, out } => {
            if !gmns_db.exists() {
                return Err(format!("{}: no such file", gmns_db.display()));
            }
            let out = out.unwrap_or_else(|| format!("{}_viewer.html", stem(&gmns_db)).into());
            let v = crate::gmns_viewer::write_viewer(&gmns_db, &out, &mode)?;
            println!("wrote {} — open in a browser", v.path.display());
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
        Cmd::Gmns {
            db,
            out,
            modes,
            to_csv,
            no_lane_geometry,
            meso,
            meso_modes,
            combined,
            drive_side,
            no_pair_carriageways,
            walk_frame,
            walk_clearance,
            micro,
            micro_modes,
            gtfs,
            gtfs_max_m,
            csv_extensions,
            check,
        } => {
            check_db(&db)?;
            if let Some(p) = gtfs.iter().find(|p| !p.exists()) {
                return Err(format!("--gtfs: {} does not exist", p.display()));
            }
            let out = out.unwrap_or_else(|| format!("{}_gmns.duckdb", stem(&db)).into());
            let o = crate::gmns::GmnsOptions {
                modes: modes.clone(),
                lane_geometry: !no_lane_geometry,
                pair_carriageways: !no_pair_carriageways,
                drive_side,
                combined,
                to_csv: to_csv.clone(),
                csv_extensions,
                gtfs,
                gtfs_max_m,
                walk_frame,
                walk_clearance_m: walk_clearance,
            };
            let res = crate::gmns::to_gmns_with(&db, &out, &o)?;
            println!("wrote {}", out.display());
            for m in &res.modes {
                let n = |t: &str| m.counts.iter().find(|c| c.0 == t).map(|c| c.1).unwrap_or(0);
                println!(
                    "  {}: {} links, {} lanes, {} movements, {} signals, {} curb segments",
                    m.mode,
                    n("link"),
                    n("lane"),
                    n("movement"),
                    n("signal_controller"),
                    n("curb_seg")
                );
            }
            if let Some((nodes, links)) = res.combined {
                println!("  combined: gmns_all — {links} links, {nodes} nodes (mode-tagged)");
            }
            if meso {
                for (mode, nodes, normal, movement) in crate::gmns_meso::to_meso(&out, &meso_modes, 6.0)? {
                    println!("  meso[{mode}]: {} meso links ({normal} section + {movement} connector), {nodes} nodes", normal + movement);
                }
            }
            if micro {
                for (mode, nodes, cell, change, turn) in crate::gmns_meso::to_micro(&out, &micro_modes, 7.0)? {
                    println!("  micro[{mode}]: {} micro links ({cell} cell + {change} lane-change + {turn} turn), {nodes} nodes", cell + change + turn);
                }
            }
            if let Some(d) = &to_csv {
                println!("  + GMNS CSVs -> {}", d.display());
            }
            if check {
                let rows = crate::gmns_check::check_gmns(&out, &modes)?;
                for g in crate::gmns_check::graph_report(&out, &modes)? {
                    println!(
                        "  graph {}: {} nodes, {} links; the largest strongly connected part has {} nodes ({:.0}%), {} parts in all; {} dead-end links",
                        g.schema,
                        thousands(g.nodes),
                        thousands(g.links as i64),
                        thousands(g.biggest as i64),
                        g.share * 100.0,
                        thousands(g.parts as i64),
                        thousands(g.dead_ends as i64)
                    );
                }
                let failed: Vec<_> = rows.iter().filter(|r| r.2 != 0).collect();
                for (schema, name, bad, total) in &failed {
                    eprintln!("  CHECK FAILED {schema}: {name}: {bad} of {total}");
                }
                println!("  checks: {} of {} passed", rows.len() - failed.len(), rows.len());
                if !failed.is_empty() {
                    std::process::exit(1);
                }
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
