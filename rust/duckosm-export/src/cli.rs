//! The export commands of `duckosm-rust` (`duckosm-rust matsim ...`), with the options of the Python CLI
//! (src/duckosm/cli.py).

use crate::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

/// The command names of this crate: `duckosm-rust <name> ...` goes to [`run`].
pub const COMMANDS: [&str; 2] = ["matsim", "railml"];

#[derive(Parser)]
#[command(name = "duckosm-rust")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
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
    let cli = Cli::parse_from(std::iter::once("duckosm-rust".to_string()).chain(args.iter().cloned()));
    match cli.cmd {
        Cmd::Matsim { db, mode, crs, no_gzip, out } => {
            check_db(&db)?;
            let out = out.unwrap_or_else(|| format!("{}_network.xml{}", stem(&db), if no_gzip { "" } else { ".gz" }).into());
            let s = crate::matsim::to_matsim(&db, &out, &mode, crs.as_deref(), !no_gzip)?;
            println!("wrote {} — {} nodes, {} links (CRS {})", out.display(), s.nodes, s.links, s.crs);
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
