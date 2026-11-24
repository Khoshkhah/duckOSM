# The HD map: duckOSM runs SimReady (`duckosm hdmap`)

Status: approved by Kaveh on 2026-11-24 (with both databases as required inputs, and the name `duckosm-sim build`).

| Date | Decision (Kaveh) |
|---|---|
| 2026-11-24 | SimReady is duckOSM's Level 4 (the HD map). One SimReady carriageway is one row of `gmns_driving.chains`, with its `chain_id` |
| 2026-11-24 | SimReady reads duckOSM's GMNS database for the network, and the raw OSM tables and the DEM grid of the same build for the street space. Kerbs, walls and bridge outlines do not go into GMNS |
| 2026-11-24 | duckOSM runs SimReady to make the HD map database |
| 2026-11-24 | the command takes both databases as required inputs: the routing database and the GMNS database. It does not make the GMNS database |

The duckOSM HD map database of draft PR 443 (`hdmap_duckdb.md`) is not built. This design replaces it.

## The idea

One duckOSM command makes the HD map of an area. It prepares SimReady's input from the duckOSM databases, runs
SimReady's program, and gives the user the HD map database and the format files.

```
<area>.duckdb (Level 2) ---duckosm gmns---> <area>_gmns.duckdb (Level 3)
        |                                           |
        +-------- duckosm hdmap: 1. the input folder (GMNS + raw OSM + DEM grid + input.json)
                                 2. duckosm-sim build (SimReady)
                                 3. <area>_hdmap.duckdb (Level 4) and the format files
```

## The command

`duckosm hdmap DB GMNS_DB [OPTIONS]`

| Option | Default | Effect |
|---|---|---|
| `DB` | required | the routing database (Level 2) of the area |
| `GMNS_DB` | required | the GMNS database of the same area, made by `duckosm gmns` with the lane rules (`--lane-source rules`) |
| `--out PATH` | `<area>_hdmap.duckdb` next to `DB` | the HD map database |
| `--formats LIST` | none | the format files that SimReady writes too: `lanelet2`, `opendrive`, `sumo`, `gmns`, `matsim`, `gltf` |
| `--simready PATH` | `$DUCKOSM_SIM`, else `duckosm-sim` on the `PATH` | SimReady's program |
| `--keep-input` | off | keep the input folder after the run |
| `--jurisdiction CODE` | none | the ISO 3166 code of the area, for SimReady's traffic rules and design profile |
| `--driving-side SIDE` | none | `right` or `left`; without it, SimReady takes it from the jurisdiction |

When SimReady's program is not found, the command stops with: "SimReady is not installed: give `--simready` or set
`DUCKOSM_SIM`". SimReady is a separate program, and it is private today.

## Step 1: the input folder

The command writes one folder, `<area>_hdmap_input/`. It is the contract between duckOSM and SimReady:

| File | Holds | From |
|---|---|---|
| `gmns.duckdb` | the network: links with `refs` and `z_vertices`, lanes, movements, chains, crossings, nodes, `config`, `main.elevation_metadata` | the GMNS database (a copy, or a link to it) |
| `raw_nodes.parquet`, `raw_ways.parquet`, `raw_relations.parquet` | the OSM tags, with the fixes of the build: kerbs, building walls, bridge relations, widths, `ele` | `raw.*` of the routing database |
| `elevation_grid.parquet` | the ground raster | `main.elevation_grid` of the routing database. Not written when the build has none |
| `fixes.yaml` | the fixes file of the build, as it was at the build | `main.fixes.yaml` of the routing database. Not written when the build had no fixes file |
| `input.json` | the manifest (below) | new |

`input.json`:

| Key | Value |
|---|---|
| `contract` | `1`. SimReady stops when it does not know the number |
| `area` | the area name |
| `duckosm_version` | the version that wrote the input |
| `ground_id` | the `sha256` of `main.elevation_metadata`, or empty. Its `license` column gives the licence of the DEM |
| `osm_timestamp` | the time of the OSM data (`main.osm_source.osm_timestamp` of the routing database, UTC, `YYYY-MM-DD HH:MM:SS`), or null |
| `jurisdiction` | the ISO 3166 code of the area (option `--jurisdiction`), or null: SimReady's project default |
| `driving_side` | `right` or `left` (option `--driving-side`), or null: from the jurisdiction |
| `files` | each file with its `sha256` (DuckDB `sha256(content)` of `read_blob`) |

Checks before SimReady runs. Each failed check stops the command with a message:

| Check | Passes when |
|---|---|
| one area | the GMNS database was made from `DB` (`config.dataset_name` and the link ids are the edge ids of `DB`) |
| chains | `gmns_driving.chains` is there, and each driving link is in one chain |
| GMNS values | `link.refs`, `link.z_vertices` (can be empty) and `config.duckosm_version` are there (duckOSM main from PR 468) |
| raw OSM | `DB` has `raw.nodes`, `raw.ways` and `raw.relations`. A build cut from a parent database has none: the message says to build from the PBF |

## Step 2: run SimReady

The command runs `duckosm-sim build --input <folder> --out <out> [--formats ...]` and shows its log. SimReady's exit
code is the command's exit code.

SimReady must add this `build` command (its own work, in its repository):

- it reads only the input folder;
- it runs no duckOSM build of its own (no pass 0);
- it uses no netconvert (netconvert is only a comparison in SimReady, D34);
- it writes the HD map database and the format files.

## Step 3: the output

| Output | Holds |
|---|---|
| `<area>_hdmap.duckdb` | SimReady's road model (schema `sim`): carriageways, lane sections, lanes with their borders in 3D, junctions, movements, stop lines, islands, signal plans, heights, and the evidence of each value |
| the format files | one for each `--formats` value, from SimReady's adapters |
| `<area>_hdmap_fixes.yaml` | the OSM fixes that SimReady suggests (one-way, access, missing turns). The user reads them, adds the right ones to the fixes file and builds again. No automatic loop |

## What stays in duckOSM

| Part | Today | After this design |
|---|---|---|
| `duckosm lanelet2`, `opendrive --junctions`, `commonroad`, `lane-graph` (from the GMNS database) | experimental exporters | unchanged for now. A later decision: keep them as the "OSM only" version, or point their users to `duckosm hdmap` |
| HD map database of PR 443 | design | not built |

## Rules

| Rule | Why |
|---|---|
| no SimReady code in duckOSM | SimReady rule D35. duckOSM only writes files and runs a program |
| SimReady is optional | duckOSM installs and works without it. The tests of `duckosm hdmap` skip when `duckosm-sim` is not found |
| one contract number | a change of the input folder raises `contract`, so an old SimReady stops with a clear message |
| the GMNS lanes from the lane rules | SimReady never reads netconvert's lanes (D34). The user makes the GMNS database with `--lane-source rules`; the command reads `config` to check it when GMNS records the lane source |

## Rejected alternatives

| Alternative | Why not |
|---|---|
| duckOSM only prepares the input, the user runs SimReady | two commands for one result. Kaveh chose one duckOSM command |
| duckOSM builds the HD map itself (PR 443) | a second road model next to SimReady's, and files that do not agree |
| SimReady reads the routing database | Kaveh: SimReady reads GMNS |
| kerbs, walls and bridge outlines in the GMNS database | Kaveh: GMNS must not get heavy; they are street space, not network |
| SimReady as a Python package that duckOSM imports | SimReady's engine is Rust, and its code must stay out of duckOSM (D35) |

## Steps after sign-off

1. duckOSM, in Rust (the library is Rust; Python is a thin layer): `rust/duckosm-export/src/hdmap_run.rs` (the input
   folder, the checks, the run), the command `hdmap` in `duckosm-export/src/cli.rs`, and a pass-through `duckosm hdmap`
   in `cli.py`;
   tests with a fake `duckosm-sim` (a small script that checks the input folder and writes an empty output), and a
   real run on Monaco when SimReady is installed.
2. SimReady (its session): the `build` command on the input folder.
3. Docs: the Level 4 page, the command line page, the SDK page.

