# The bus routes: `bus.routes` and `bus.route_edges`

Status: done on 2026-10-23 (the decisions and the results are at the end). Group 2 of [tables.md](tables.md).

## What duckOSM does

duckOSM (`src/duckosm/bus.py`, "Bus routes" in `docs/concepts/networks.md`) writes each OSM bus route relation
(`route` = `bus`, `trolleybus` or `share_taxi`) as the driving edges that its buses travel, in order and in their
direction. A build from a PBF with driving does it (`options.bus_routes`, on by default). The input is `raw.relations`,
`raw.ways` (after the OSM fixes), `driving.edges` and `driving.private_edges`.

1. **The members.** The walk uses only the members of type `way`. It skips the members with a role that starts
   with `platform` or `stop`. A role that is not `''`, `forward` or `backward` is the gap `role`.
2. **The runs.** A member way that is not in `raw.ways` is the gap `outside` and ends the run. A member way that
   has no node in common with the way before it is the gap `no_connection` (on the way before) and also ends the run.
3. **The walk of a run.** The route goes along each way from the node that it has in common with the way before
   (the entry) to the node that it has in common with the way after (the exit). If two ways have more than one node
   in common, `_link` selects one: an end of both ways first, then an end of one way, then a node that is not the
   entry, then the first node in the way's order.
   - The first way and the last way of a run have only one neighbor. The route goes to (or from) the far end of
     the way. If the neighbor joins the way in its middle, the role decides, then the `oneway` tag. If none of
     them can decide, the result is the gap `ambiguous`.
   - A closed way (a roundabout) goes from the entry round to the exit, in the direction of the role or `oneway`.
   - If the role and the neighbors do not agree, the walk follows the neighbors. The result is the gap
     `role_conflict`.
4. **The edges.** Each pair of nodes in the walk goes to the edge of that way (`osm_id`) that has the same pair, in
   the same order, in its `refs`. The edges come from `driving.edges` and from `driving.private_edges` (there,
   `bus_lane` is true). Thus a two-way street gives its `#nf` or its `#nr` edge.
   - No edge: the gap `no_edge`. A private edge with `access = 'private'`: the gap `private`. A private edge with
     another access that is not `bus` (for example a pedestrian street): the gap `no_edge`.
   - If one pair is on two edges in the same direction, duckOSM stops with an error.
   - The same edge two or more times in sequence gives one row. A gap of one kind on many pairs of one member gives
     one gap.
5. **The tables.** `bus.routes`: one row for each relation, with its tags, `edges`, `outside` and `gaps`
   (`'<way_id> <kind>'`, sorted by the member's position). `bus.route_edges`: `route_id`, `ref`, `seq` (1, 2, ...),
   `edge_id`, `edge_ref`, `bus_lane`. The columns and types are in duckOSM's `docs/reference/database.md`.

Monaco (`/home/kaveh/projects/duckOSM/monaco.duckdb`): 56 routes, no gaps, 38 routes with ways outside the area,
5,609 rows in `route_edges` on 1,034 edges, 48 rows on bus lanes.

## The design in Rust

### Where the input comes from

The raw cut (`pbf::read_area_raw`, from the base-map work) has all that the walk needs, with the same contents as
duckOSM's `raw.*`:

- `Raw::relations`: the relations of the cut, with their tags and members (`graph::Relation`).
- `Raw::ways`: all the ways with a node inside, with all their nodes and tags, after `Raw::apply_fixes`. The walk
  needs these ways and not the network's ways, because a member way can be a road that the driving filter
  removes. The connection to the next way and the `outside` count also need all of its nodes.

Thus, when the build makes the bus routes, it does the raw cut. It writes the `raw_*.parquet` files only with
`--raw`, as now. The base-map work measured the cost: osmium's `smart` cut of Tartu takes 3.0 s, and the
`complete_ways` cut takes 2.0 s. I will measure the Rust cut too.

### The code

- `duckosm-core/src/bus.rs` (new): `routes(relations, ways, edges, private_edges) -> (Vec<Route>, Vec<RouteEdge>)`.
  It is a direct port of `bus.py`: `link`, `path`, `walk`, the match of the node pairs with a hash map from
  `(osm_id, u, v)` to the edge, and the merge of the gaps. It has no I/O. The unit tests use small hand-made ways: a
  two-way street in both directions, a roundabout, a role conflict, a gap, an `ambiguous` end.
- `duckosm-build`: a function that is called after all modes (as duckOSM's importer), with the raw tables and the
  driving network. The CLI build, the Python build and the area build of the page (`duckosm-wasm`) all use it.
- `duckosm-parquet`: `routes_bus.parquet` and `route_edges_bus.parquet`, with duckOSM's columns and types
  (`gaps` is `VARCHAR[]`).
- `duckosm-duckdb`: the two files go into schema `bus` of the `.duckdb` file.
- The page (`www/app/duck.js`): the two tables in SQL, in the zip, in `load.sql`, and a line for each in the table
  list.
- The option: `--no-bus-routes` (CLI), `bus_routes=False` (Python), as duckOSM's `options.bus_routes`. On by
  default.

### The edge cases

| Case | What the Rust code does |
|---|---|
| A member way outside the area | The gap `outside`, as duckOSM: not in `Raw::ways`. It ends the run. |
| A gap between two member ways | The gap `no_connection`. The walk starts a new run at the next way. |
| A way that the route travels in both directions (there and back) | Each pass gives its own pairs, so the `#nf` edge and later the `#nr` edge. |
| A two-way street, first or last way of a run, joined in its middle, no role | The gap `ambiguous`. |
| A one-way street against the route, with a contraflow bus lane | The bus lane edge from `private_edges` (`bus_lane` true). |
| One pair on two edges in the same direction | The gap `two_edges` (Kaveh's decision): the build goes on. duckOSM stops with an error. |
| No driving mode | No bus tables, as duckOSM. |
| OSM JSON input (Overpass) | No bus tables, as duckOSM (PBF only). |
| No boundary | See question 1. |

## The checks

1. **Monaco, `cargo test`**: a build of `tests/data/monaco_sample.osm.pbf` with `monaco.geojson`. Both tables are
   equal to duckOSM's golden (`monaco.duckdb`) on every row: 56 and 5,609 rows. The expected rows go into
   `tests/data/monaco_bus_routes.tsv` and `monaco_bus_route_edges.tsv`, from the golden.
2. **Monaco, `tools/compare_tables.py`**: the `.duckdb` file of the CLI against the golden, with `bus.*` in the list.
3. **Tartu**: the golden `data/db/tartu.duckdb` (2026-09-30) has no `bus` schema. Thus I make the reference: duckOSM's
   `duckosm bus` on a copy of the Rust Tartu `.duckdb` (built with `--raw`, so that it has `raw.*`). The Rust tables
   must be equal to it on every row. This check compares the walk only; the network check already compares the
   driving edges.
4. **The page** (`www/e2e.py`): Monaco, the two files equal to the native files; the tables are in the table list.

## Questions for Kaveh

1. **No boundary.** duckOSM then has the whole file in `raw.*`, but the Rust raw cut needs a boundary. I recommend:
   no bus tables without a boundary, and the CLI writes one line that says so. The other possibility is one more
   pass over the file for the bus relations and their member ways (about 60 lines). The page always has a boundary.
2. **The cost.** With the bus routes on by default, each build with a boundary does the raw cut. Is that correct,
   or do you want the bus routes off by default?
3. **One pair on two edges**: an error, as duckOSM, or a gap? An error stops the build in the page too.

## Decisions (Kaveh, 2026-10-23)

1. **No boundary: no bus tables.** The program prints "no bus tables: the build has no area outline (--boundary)".
   The page always has a boundary.
2. **On by default**, as in duckOSM: whenever a build has a boundary and the driving mode. `--no-bus-routes` (CLI),
   `bus_routes=False` (Python) or `options.bus_routes: false` (a duckOSM config) turns them off.
3. **One pair on two edges: a gap**, `two_edges`, not an error.

## Result (2026-10-23)

- `duckosm-core/src/bus.rs`: the port of `bus.py`, with unit tests (a street there and back, a roundabout, a role
  conflict, an ambiguous end, a way outside the area).
- `duckosm-build::write_bus`, after the driving mode, in the native build and in the page. When the bus routes are
  on, a build with a boundary does the raw cut; it writes the `raw_*.parquet` files only with `--raw`.
- The files `routes_bus.parquet` and `route_edges_bus.parquet`; the schema `bus` in the `.duckdb` file; the two
  tables in the page (SQL, the zip, `load.sql`, the table list).

Checks:
- Monaco, against duckOSM's `monaco.duckdb` (`tools/compare_tables.py`, a build with `config/sample_monaco.yaml`):
  `bus.routes` 56 rows and `bus.route_edges` 5,609 rows, equal on every row.
- Tartu from the Estonia file: duckOSM's `write_bus_routes` on a copy of the Rust `.duckdb` file (with the `raw.*`
  of `--raw`) gives 74 routes and 16,421 route edges, equal to the Rust tables on every row, with the 12 routes that
  have gaps.
- `cargo test -p duckosm-wasm`: the browser build writes the same 40 files as the native build, byte for byte.
- The page (`e2e.py`, Monaco): the two files equal to the native files, on every row.
- The cost on Tartu: 1.30 to 1.61 s with the bus routes, 1.30 to 1.45 s without (in the noise).

