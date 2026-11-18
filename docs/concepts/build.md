# How a build works

A build (`duckosm build`, or `DuckOSM(config).run()` in Python) runs a list of steps chosen from the config. Each step is SQL
in the output DuckDB file. For the Rust build, see [Browser and native builds](browser-native.md).

## Two ways in

| | Build from a PBF | Clip from a parent build |
|---|---|---|
| Input | an `.osm.pbf` file | an existing `.duckdb` and a boundary |
| Config | `source.type: pbf` | `source.type: duckdb`, or `duckosm build --source-db` |
| `edge_id` | made from OSM | the parent's |
| Tables | all | `edges`, `private_edges`, `nodes`, `edge_graph`, `turn_restrictions` |

How to clip: [Several areas from one region](../guides/prepare-area.md#cut-an-area-from-a-built-region).

## The steps of a PBF build

| Stage | Steps |
|---|---|
| ① Once | cut the PBF to the boundary (`osmium`), parse OSM into `raw.*`, global junctions |
| ② Each mode | filter roads ([What each network contains](networks.md)), OSM fixes, build edges, simplify and merge, connect dangling paths, speeds, costs, walk / cycle type, private roads apart, turn restrictions, edge graph, H3, indexes, validate |
| ③ Once, at the end | intermodal graph `mm.*`, base-map layers `features.*`, bus routes `bus.*`, `edge_id_hash` macros, metadata and time zone, report and map |

A step in ③ that fails writes a warning, and the build continues. The time zone and the bus routes stop the build.

Each mode has its own schema (`driving`, `walking`, `cycling`). More about ②: [Stable edge ids](edge-ids.md),
[Network clean-up](cleanup.md).

## What stays in the database

- The routing tables: `edges`, `nodes`, `edge_graph`, `turn_restrictions`.
- For each mode: `private_edges`, `ways`, `way_nodes`, `edge_id_map`.
- `raw.*`, `main.global_junctions`, `main.boundary` and `features.*`.

Every table and column: [Database schema](../reference/database.md). Diagrams, the config key and condition of each step, and the
Monaco numbers: [build in detail](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/build_details.md).
