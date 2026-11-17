//! The database tools of `duckosm-rust` (`duckosm-rust info ...`), with the options of the Python CLI (src/duckosm/cli.py).

use crate::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// The command names of this crate: `duckosm-rust <name> ...` goes to [`run`].
pub const COMMANDS: [&str; 3] = ["info", "way", "init-config"];

#[derive(Parser)]
#[command(name = "duckosm-rust")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
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
    let cli = Cli::parse_from(std::iter::once("duckosm-rust".to_string()).chain(args.iter().cloned()));
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
