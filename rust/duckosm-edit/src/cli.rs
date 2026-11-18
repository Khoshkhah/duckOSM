//! The map commands of `duckosm` (group F of docs/design/all_in_rust.md), with the options of the Python CLI (src/duckosm/cli.py).

use crate::maps::{render_maps, Viz};
use crate::native::NativeDb;
use clap::{ArgAction, Parser, Subcommand};
use std::path::PathBuf;

/// The command names of this module: `duckosm <name> ...` goes to [`run`].
pub const COMMANDS: [&str; 2] = ["viz", "route-map"];

#[derive(Parser)]
#[command(name = "duckosm")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Draw the maps of a built network: one page with every mode (<name>_map.html) and one for each mode
    /// (<name>_<mode>_network.html), over the database's own base map. With -m, only the pages of those modes.
    Viz {
        db: PathBuf,
        /// A mode to draw a page of, again for more modes (default: every mode, and a page with all modes).
        #[arg(short, long = "mode")]
        modes: Vec<String>,
        /// The first background: the database's own layers (default), or voyager, positron, osm, satellite, ...
        #[arg(long)]
        basemap: Option<String>,
        /// The output folder.
        #[arg(long, default_value = "reports")]
        out_dir: PathBuf,
        /// No direction arrows on one-way roads.
        #[arg(long)]
        no_arrows: bool,
        /// Do not draw the area's boundary (main.boundary).
        #[arg(long)]
        no_boundary: bool,
        /// A Google 3D window linked to the map (default: on when GOOGLE_MAPS_KEY is set).
        #[arg(long = "google-3d", action = ArgAction::SetTrue, overrides_with = "no_google_3d")]
        google_3d: bool,
        #[arg(long = "no-google-3d", action = ArgAction::SetTrue)]
        no_google_3d: bool,
    },
    /// Write a route planner page for a built database: drag a start and an end, pick Drive, Walk, Cycle or Walk + drive
    /// (needs `duckosm multimodal` first), get the route and the turn-by-turn directions. It routes in the browser with this
    /// program's router (WebAssembly): the same routes as route-points, route-multimodal-points and directions.
    RouteMap {
        db: PathBuf,
        /// The first choice and the network in front (walk+drive: every mode; needs `duckosm multimodal`).
        #[arg(short, long, value_parser = ["driving", "walking", "cycling", "walk+drive"])]
        mode: Option<String>,
        /// The first background: the database's own layers (default), or voyager, positron, osm, ...
        #[arg(long)]
        basemap: Option<String>,
        /// A Google 3D window linked to the map (default: on when GOOGLE_MAPS_KEY is set).
        #[arg(long = "google-3d", action = ArgAction::SetTrue, overrides_with = "no_google_3d")]
        google_3d: bool,
        #[arg(long = "no-google-3d", action = ArgAction::SetTrue)]
        no_google_3d: bool,
        /// The output HTML (default: reports/<name>_route_map.html).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

/// Run one command; `args[0]` is its name.
pub fn run(args: &[String]) -> Result<(), String> {
    let cli = Cli::parse_from(std::iter::once("duckosm".to_string()).chain(args.iter().cloned()));
    match cli.cmd {
        Cmd::Viz { db, modes, basemap, out_dir, no_arrows, no_boundary, google_3d, no_google_3d } => {
            let all = modes.is_empty();
            let v = Viz { all_modes: all, modes: (!all).then_some(modes), basemap, arrows: !no_arrows, boundary: !no_boundary,
                          google_3d: if google_3d { Some(true) } else if no_google_3d { Some(false) } else { None }, ..Viz::new(out_dir) };
            for p in render_maps(&mut NativeDb::new(&db), &db, &v)? {
                println!("wrote {}", p.display());
            }
            Ok(())
        }
        Cmd::RouteMap { db, mode, basemap, google_3d, no_google_3d, out } => {
            let name = db.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let out = out.unwrap_or_else(|| PathBuf::from(format!("reports/{name}_route_map.html")));
            let g3d = if google_3d { Some(true) } else if no_google_3d { Some(false) } else { None };
            let p = crate::route_map::write_route_map(&db, &out, mode.as_deref(), basemap.as_deref(), g3d)?;
            println!("wrote {} — open it in a browser", p.display());
            Ok(())
        }
    }
}
