# How a build works

A build turns an `.osm.pbf` file into one DuckDB file with the networks of an area. It runs in 6 steps. Each step
writes its result to storage, and the next step reads only what it needs.

```bash
duckosm monaco.osm.pbf --boundary monaco.geojson -f duckdb --features -o out   # the build, all in Rust
```

## The steps

| Step | Does | Writes |
|---|---|---|
| 0. Cut | cuts the OSM file to the boundary: each way with a node inside stays whole | the cut file |
| 1. Read | reads the ways, the node positions and the turn restrictions, and applies your [OSM fixes](../guides/edit-osm-data.md) | the ways and the nodes |
| 2. Junctions | finds the nodes where ways meet, for all modes together | `main.global_junctions` |
| 3. Network | for each mode: selects the ways ([what each network contains](networks.md)), cuts them into edges at the junctions, gives each edge its [`edge_id`](edge-ids.md), speed and cost, and makes the turn graph | `edges`, `nodes`, `edge_graph`, and for driving `turn_restrictions` |
| 4. Write | writes the tables of each mode into the DuckDB file | the schemas `driving`, `walking`, `cycling` |
| 5. More tables | the area tables, the drawing levels, the bus routes, the lane profile, the building walls, with `--raw` the raw OSM tables, and with `--features` the base-map layers | `main.*`, `visualization.*`, `bus.*`, `raw.*`, `features.*` |

For a country, use `duckosm tiles`: it builds each map tile in its own process, then joins the tiles. Its memory does not
grow with the area: [Browser and native builds](browser-native.md).

## Two ways in

| | Build from an OSM file | Cut from a built database |
|---|---|---|
| Input | an `.osm.pbf` file | a built `.duckdb` file and a boundary |
| Command | `duckosm FILE`, `duckosm build` | `duckosm extract` |
| `edge_id` | made from OSM | the same as in the larger database |
| Tables | all (`duckosm FILE` writes `raw.*` and `features.*` only with `--raw` and `--features`) | the network tables only, no `raw.*` and no base-map layers |

How to cut: [Prepare an area](../guides/prepare-area.md).

## What stays in the database

| Tables | Holds |
|---|---|
| `<mode>.edges`, `nodes`, `edge_graph`, and `driving.turn_restrictions` | the routable network of each mode |
| `<mode>.private_edges`, `ways`, `way_nodes`, `edge_id_map` | the restricted roads, the OSM ways, and the map from OSM ways to edges |
| `main.boundary`, `main.global_junctions` | the area and its junctions |
| `features.*`, `raw.*` | the base-map layers, and the raw OSM tables they are made from |

Every table and column: [Routable network tables](../reference/database.md).
