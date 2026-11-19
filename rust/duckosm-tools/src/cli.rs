//! The database tools of `duckosm` (`duckosm info ...`), with the options of the Python CLI (src/duckosm/cli.py).

use crate::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// The command names of this crate: `duckosm <name> ...` goes to [`run`].
pub const COMMANDS: [&str; 9] = ["info", "way", "init-config", "boundary", "extract", "admin", "elevation", "bus", "multimodal"];

#[derive(Parser)]
#[command(name = "duckosm")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Match the bus routes of the db's OSM relations to its driving edges (schema bus). Writes bus.routes (one row per
    /// route relation, with its gaps) and bus.route_edges (the driving edges each route travels, in order and in its
    /// direction; bus lanes included). The build does it too (options.bus_routes); this command adds it to a file built
    /// before. Needs raw.* and the driving network.
    Bus {
        /// The .duckdb file of a build.
        db: PathBuf,
    },
    /// Build the multimodal graph (mm.edges and mm.transfers) into a built db: a route can then change mode at a junction
    /// (walk, drive, walk). Needs walking and one more mode. Route the result with `duckosm route-multimodal`. The db is
    /// changed in place.
    Multimodal {
        /// The .duckdb file of a build.
        db: PathBuf,
        /// The cost of a change of mode at a shared junction, in seconds.
        #[arg(long, default_value_t = 60.0)]
        transfer_cost: f64,
        /// Another cost for one direction, FROM->TO=SECONDS (for example walking->driving=120). Repeat for more.
        #[arg(long = "pair-cost", value_name = "FROM->TO=SECONDS")]
        pair_costs: Vec<String>,
        /// The schema of the mm.edges and mm.transfers tables.
        #[arg(long, default_value = "mm")]
        schema: String,
    },
    /// What a built database holds: per mode its edges, nodes, private edges, km, legal turns and turn restrictions; when and
    /// by which duckOSM version it was built; its time zone; and the other schemas (raw OSM data, base-map layers, across
    /// modes, boundary, elevation). Read-only.
    Info {
        db: PathBuf,
        /// Print one JSON object instead of a table.
        #[arg(long = "json")]
        as_json: bool,
    },
    /// Everything this db knows about one OSM_ID, across all modes, in one table: the raw way row (tags + node refs), then
    /// the union of the per-mode edges extracted from it, in traversal order. A negative OSM_ID inspects synthetic
    /// PathConnector connector edges.
    Way {
        db: PathBuf,
        #[arg(allow_hyphen_values = true)]
        osm_id: i64,
        /// Mode schema(s) to include (default: every mode present in the db).
        #[arg(short = 'm', long = "mode")]
        modes: Vec<String>,
        /// Include the edge geometry as WKT (wide!).
        #[arg(long, overrides_with = "no_geom")]
        geom: bool,
        #[arg(long, hide = true)]
        no_geom: bool,
        /// Write the table to a file instead of printing (format by extension: .csv, .parquet, or .json).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Print one JSON object: the raw way (tags, refs) and its edges.
        #[arg(long = "json")]
        as_json: bool,
    },
    /// Find the boundary of the area called NAME and write it as GeoJSON. Looks in --pbf's own borders first (offline), then
    /// asks Nominatim online.
    Boundary {
        name: Option<String>,
        /// Search the borders stored in this PBF first (offline; GDAL in DuckDB's spatial extension).
        #[arg(long)]
        pbf: Option<PathBuf>,
        /// Pick one exact OSM boundary relation (for ambiguous names).
        #[arg(long)]
        osm_id: Option<i64>,
        /// Never ask OpenStreetMap's Nominatim search online.
        #[arg(long)]
        offline: bool,
        /// Output GeoJSON (default: <name>.geojson).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Slice a sub-area out of an existing build into a new, self-contained db: an edge is kept whole if it intersects the
    /// area; main.boundary, a new main.visualization_metadata (with the time zone) and the admin boundaries inside the area
    /// are added; raw.* is not copied.
    Extract {
        /// Source duckOSM .duckdb.
        #[arg(long, required = true)]
        source: PathBuf,
        /// Output .duckdb (overwritten).
        #[arg(long, required = true)]
        db: PathBuf,
        /// Area name, matched in the source admin_boundaries.
        #[arg(long, group = "area")]
        name: Option<String>,
        /// Admin boundary osm_id to use as the area.
        #[arg(long, group = "area")]
        osm_id: Option<i64>,
        /// GeoJSON boundary file.
        #[arg(long, group = "area")]
        boundary: Option<PathBuf>,
    },
    /// Add an admin_boundaries table (all OSM administrative levels, with each one's parent) to a duckOSM db, from the
    /// boundary relations of a PBF.
    Admin {
        /// Source OSM PBF file.
        #[arg(long, required = true)]
        pbf: PathBuf,
        /// Target duckOSM .duckdb file.
        #[arg(long, required = true)]
        db: PathBuf,
        /// GeoPackage path; reused if it already exists, else written here (default: none).
        #[arg(long)]
        gpkg: Option<PathBuf>,
    },
    /// Sample a DEM at every node and add elevation to a built db, in place. Adds <mode>.nodes.ele and
    /// <mode>.edges.z_from/z_to/z_vertices, and records provenance in main.elevation_metadata. Either point --dem at a
    /// GeoTIFF in EPSG:4326 (32-bit floats), or let --source download a global DEM. A --dem file in `ele` is the area's
    /// reference ground: give --product, --vertical-datum and --license, or --not-reference.
    Elevation {
        db: PathBuf,
        /// A GeoTIFF in EPSG:4326 with 32-bit float values: a file or an HTTP address. Takes precedence over --source.
        #[arg(long)]
        dem: Option<String>,
        /// Global DEM to download if no --dem: 'auto' (best for the db's area), 'copernicus' (GLO-30, anonymous), or
        /// 'eudtm' (Europe bare-earth, needs OPENTOPOGRAPHY_API_KEY).
        #[arg(long, default_value = "auto")]
        source: String,
        /// Mode schema(s) to enrich (default: every mode present in the db).
        #[arg(short = 'm', long = "mode")]
        modes: Vec<String>,
        /// Elevation written where the DEM has a void / does not cover the node.
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        nodata_fill: f64,
        /// Store as a SECOND surface instead of overwriting: --suffix dsm writes ele_dsm / z_from_dsm / z_to_dsm.
        #[arg(long, default_value = "", value_name = "NAME")]
        suffix: String,
        /// The product of the --dem file: DTM (bare earth) or DSM (surface).
        #[arg(long, value_parser = ["DTM", "DSM"])]
        product: Option<String>,
        /// The vertical datum of the --dem file (for example EGM2008).
        #[arg(long, value_name = "DATUM")]
        vertical_datum: Option<String>,
        /// The license of the --dem file.
        #[arg(long, value_name = "TEXT")]
        license: Option<String>,
        /// The --dem file is not the area's reference ground (a test or a surface model).
        #[arg(long)]
        not_reference: bool,
    },
    /// Write the fully commented config template to PATH (default: duckosm.yaml). Edit it, then run
    /// `duckosm build --config PATH`.
    #[command(name = "init-config")]
    InitConfig {
        #[arg(default_value = "duckosm.yaml")]
        path: PathBuf,
        /// Overwrite PATH if it already exists.
        #[arg(long)]
        force: bool,
    },
}

fn need_db(db: &std::path::Path) -> Result<()> {
    if db.exists() { Ok(()) } else { Err(format!("database not found: {}", db.display())) }
}

/// Run one command; `args` starts with the command name.
pub fn run(args: &[String]) -> Result<()> {
    let cli = Cli::parse_from(std::iter::once("duckosm".to_string()).chain(args.iter().cloned()));
    match cli.cmd {
        Cmd::Info { db, as_json } => {
            need_db(&db)?;
            let d = crate::info::db_info(&db)?;
            if as_json {
                println!("{}", crate::json::dumps(&d));
            } else {
                print!("{}", crate::info::text(&db, &d));
            }
        }
        Cmd::Way { db, osm_id, modes, geom, no_geom: _, out, as_json } => {
            need_db(&db)?;
            crate::way::way(&db, osm_id, &modes, geom, out.as_deref(), as_json, &mut std::io::stdout())?;
        }
        Cmd::Boundary { name, pbf, osm_id, offline, out } => {
            use duckosm_build::place::{find_boundary, write_boundary, Wanted};
            if let Some(p) = pbf.as_ref().filter(|p| !p.exists()) {
                return Err(format!("Invalid value for '--pbf': Path '{}' does not exist.", p.display()));
            }
            let want = match (&name, osm_id) {
                (_, Some(id)) => Wanted::OsmId(id),
                (Some(n), None) => Wanted::Name(n),
                (None, None) => return Err("give a NAME or --osm-id".into()),
            };
            let (geom, info) = find_boundary(want, pbf.as_deref(), offline)?;
            let out = out.unwrap_or_else(|| {
                let n = name.clone().unwrap_or_else(|| info["name"].as_str().unwrap_or_default().to_string());
                PathBuf::from(format!("{}.geojson", n.split(',').next().unwrap_or_default().trim().to_lowercase().replace(' ', "_")))
            });
            write_boundary(geom, info.clone(), &out)?;
            let describe = |i: &serde_json::Value| {
                let level = match &i["admin_level"] {
                    serde_json::Value::Null => String::new(),
                    l => format!("admin level {l}, "),
                };
                let text = |v: &serde_json::Value| v.as_str().map_or_else(|| if v.is_null() { "None".to_string() } else { v.to_string() }, str::to_string);
                let area = i["area_km2"].as_f64().map_or("None".to_string(), duckosm_build::place::py_float);
                format!("{} ({level}OSM relation {}, {area} km²)", text(&i["name"]), text(&i["osm_id"]))
            };
            println!("wrote {}: {}, from {}", out.display(), describe(&info), info["source"].as_str().unwrap_or_default());
            let others = info["others"].as_array().cloned().unwrap_or_default();
            if !others.is_empty() {
                println!("also matched (pick one with --osm-id):");
                for o in &others {
                    println!("  {}", describe(o));
                }
            }
        }
        Cmd::Extract { source, db, name, osm_id, boundary } => {
            let area = match (&name, osm_id, &boundary) {
                (Some(n), None, None) => crate::extract::Area::Name(n),
                (None, Some(id), None) => crate::extract::Area::OsmId(id),
                (None, None, Some(b)) => crate::extract::Area::Boundary(b),
                _ => return Err("one of the arguments --name --osm-id --boundary is required".into()),
            };
            crate::extract::extract(&source, &db, area)?;
        }
        Cmd::Elevation { db, dem, source, modes, nodata_fill, suffix, product, vertical_datum, license, not_reference } => {
            need_db(&db)?;
            let o = crate::elevation::Options { dem, source, modes, nodata_fill, suffix, product, vertical_datum, license, reference: !not_reference };
            let r = crate::elevation::to_elevation(&db, &o)?;
            println!(
                "elevation added from {} — {} nodes ({} nodata/fill) across {} mode(s) -> {}",
                r.source,
                r.n_nodes,
                r.n_nodata,
                r.modes.len(),
                r.column
            );
        }
        Cmd::Admin { pbf, db, gpkg } => crate::admin::admin(&pbf, &db, gpkg.as_deref())?,
        Cmd::Bus { db } => {
            need_db(&db)?;
            let c = duckosm_duckdb::steps::bus_routes(&db)?;
            println!("{} bus routes, {} route edges, {} with gaps; wrote bus.routes and bus.route_edges", c.routes, crate::info::thousands(c.edges as i64), c.with_gaps);
        }
        Cmd::Multimodal { db, transfer_cost, pair_costs, schema } => {
            need_db(&db)?;
            let pairs = pair_costs
                .iter()
                .map(|p| {
                    p.rsplit_once('=')
                        .and_then(|(k, v)| Some((k.to_string(), v.parse::<f64>().ok()?)))
                        .ok_or_else(|| format!("--pair-cost {p}: give FROM->TO=SECONDS"))
                })
                .collect::<Result<Vec<_>>>()?;
            let (e, t) = duckosm_duckdb::steps::multimodal(&db, transfer_cost, &pairs, &schema)?;
            println!("wrote {schema}.edges ({e} edges) and {schema}.transfers ({t} arcs)");
        }
        Cmd::InitConfig { path, force } => {
            if path.exists() && !force {
                return Err(format!("{} already exists (use --force to overwrite)", path.display()));
            }
            std::fs::write(&path, crate::CONFIG_TEMPLATE).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("Wrote {}. Edit it, then: duckosm build --config {}", path.display(), path.display());
        }
    }
    Ok(())
}

