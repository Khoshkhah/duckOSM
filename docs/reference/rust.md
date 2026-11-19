# Rust SDK

The engine of duckOSM is a set of Rust crates in the folder
[`rust/`](https://github.com/Khoshkhah/duckOSM/tree/main/rust) of the repository. The program
`duckosm`, the Python package and the browser page ([duckOSM Studio](../guides/studio.md)) use the
same crates. A Rust program can use them too.

## Add a crate

The crates are not on crates.io. Add them as Git dependencies at one commit (`rev`), so that a
change on `main` does not change your build:

```toml
[dependencies]
duckosm-build = { git = "https://github.com/Khoshkhah/duckOSM.git", rev = "<commit>" }
duckosm-route = { git = "https://github.com/Khoshkhah/duckOSM.git", rev = "<commit>" }
```

All crates have the same version as the Python package. To use a new engine, change the `rev`.
duckOSM Studio does this in its `crates/duckosm-wasm/Cargo.toml`.

## The crates

| Crate | Contents | Uses DuckDB |
|---|---|---|
| `duckosm-core` | The network step: read the PBF file, filter the ways of each mode, cut the roads at the junctions, the `edge_id` hash, the OSM fixes, the QuackOSM and GDAL readers | no |
| `duckosm-parquet` | Write the tables as GeoParquet files | no |
| `duckosm-duckdb` | Write the GeoParquet files into a `.duckdb` file, and make the base-map layers | yes |
| `duckosm-build` | The whole build: `build`, `run`, the config file, `quack`, the levels | with the feature `duckdb` (on by default) |
| `duckosm-route` | Routing on a built database: `route`, `route_points`, `directions` | yes |
| `duckosm-router` | The routing algorithms on plain data (no DuckDB), for the program, Python and the browser | no |
| `duckosm-planner` | The route planner of `duckosm route-map`, also as WebAssembly | no |
| `duckosm-export` | The exports: SUMO, MATSim, GMNS, GIS, OpenDRIVE, Lanelet2, railML | yes |
| `duckosm-tools` | The database tools: `info`, `way`, `boundary`, `extract`, `admin`, `elevation`, `init-config` | yes |
| `duckosm-edit` | The local pages `duckosm edit`, `duckosm ask` and `duckosm studio` | yes |
| `duckosm-editor` | The files of the OSM fixes editor, for a host page | no |
| `duckosm-cli` | The program `duckosm` | yes |
| `duckosm-py` | The Python module `duckosm._rust` | yes |

The crates with "no" also compile to WebAssembly.

## Cargo features of `duckosm-build`

| Feature | Does | Default |
|---|---|---|
| `duckdb` | Write the `.duckdb` file | on |
| `zstd` | zstd compression in the GeoParquet files | on |
| `snap` | Snappy compression, in pure Rust (for WebAssembly) in place of `zstd` | off |
| `tzf` | The time zone of an area from the world map of `tzf-rs` | on |
| `gpkg` | The GeoPackage output of `quack --gdal` | on |

For WebAssembly, use `default-features = false, features = ["snap"]`.

## Main functions

| Function | Does | Command |
|---|---|---|
| `duckosm_build::build(pbf, out, &Params) -> Result<Summary, String>` | Builds the networks of an area. Writes GeoParquet files and `<out>/<name>.duckdb`. | `duckosm build` |
| `duckosm_build::config::read(path, pbf_is_cut, strict) -> Result<FromConfig, String>` | Reads a [config file](../guides/build.md#the-config-file) into a PBF path, an output folder and `Params` | `duckosm build -c` |
| `duckosm_build::run(input, &fixes, boundary, &Params, &mut log, file)` | The whole build without a file system: each output file goes to a writer that you give. The browser build uses it. | |
| `duckosm_build::quack(pbf, parquet, duckdb, threads, sort, tiles)` | The features table of QuackOSM | `duckosm quack` |
| `duckosm_build::quack_gdal::quack_gdal(pbf, gpkg, geoparquet, tiles)` | The five layers of GDAL's OSM driver | `duckosm quack --gdal` |
| `duckosm_build::config::read_osm_fixes(path) -> Result<OsmFixes, String>` | Reads an [OSM fixes file](../guides/edit-osm-data.md#the-fixes-file) | |
| `duckosm_core::hash::edge_id(osm_id, source, target) -> i64` | The [stable edge id](../concepts/edge-ids.md) | |
| `duckosm_route::route::routing_graph(&con, mode, weight, &uses)` | The routing graph of one mode | |
| `duckosm_route::route::route(&con, &graph, from_edge, to_edge, mode)` | The shortest route between two edges, as JSON | `duckosm route` |
| `duckosm_route::points::route_points(&con, a, b, mode, weight, radius_m, access_kmh)` | The shortest route between two `(lon, lat)` points | `duckosm route-points` |
| `duckosm_route::directions::directions(&con, &route, mode)` | The turn-by-turn steps of a route | `duckosm directions` |
| `duckosm_export::sumo::to_sumo(db, out_dir, mode, net_name, connections, config, netconvert)` | SUMO files | `duckosm sumo` |
| `duckosm_export::matsim::to_matsim(db, out, mode, crs, gz)` | A MATSim network | `duckosm matsim` |
| `duckosm_export::gmns::to_gmns(source, out, &modes)` | A GMNS database | `duckosm gmns` |
| `duckosm_export::gis::to_gis(db, out, &modes, fmt, boundary, name, srs)` | A GeoPackage, GeoParquet or shapefile | `duckosm export-gis` |
| `duckosm_tools::extract::extract(source, db, area)` | Cuts an area out of a built database. `area` is `Area::Name`, `Area::OsmId` or `Area::Boundary`. | `duckosm extract` |

## Example: build and route

```rust
use duckosm_build::{build, NetworkMode, Params};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // build the driving network of Monaco into out/monaco.duckdb
    let p = Params {
        name: "monaco".into(),
        modes: NetworkMode::parse_all(&["driving"])?,
        ..Params::default()
    };
    let sum = build(Path::new("monaco-latest.osm.pbf"), Path::new("out"), &p)?;
    println!("{} edges", sum.modes[0].edges);

    // the fastest route between two edges
    let con = duckosm_route::route::open(Path::new("out/monaco.duckdb"))?;
    let graph = duckosm_route::route::routing_graph(&con, "driving", "time", &[])?;
    let (from, to) = (/* edge_id */ 0, /* edge_id */ 0);
    if let Some(r) = duckosm_route::route::route(&con, &graph, from, to, "driving")? {
        println!("{} s, {} m", r["time_s"], r["length_m"]);
    }
    Ok(())
}
```

To build from a config file, read it first:

```rust
let c = duckosm_build::config::read(Path::new("duckosm.yaml"), false, false)?;
duckosm_build::build(&c.pbf, &c.out, &c.params)?;
```

The documents for the developers of the crates are in
[`docs/rust/`](https://github.com/Khoshkhah/duckOSM/tree/main/docs/rust) on GitHub.
