# duckOSM

High-performance OSM-to-routing-network converter built on DuckDB.

## Features

- **Fast**: native `ST_READOSM` for ~100x faster PBF parsing
- **Memory efficient**: streaming SQL; graph simplification auto-batches so even
  country-scale extracts fit in RAM
- **Multi-modal**: separate `driving` / `walking` / `cycling` networks with
  mode-aware filtering, speeds and one-way handling (incl. cycling contraflow).
  Driving filtering is **access-aware** — it keeps drivable shared streets
  (`highway=pedestrian` etc. with `motor_vehicle=yes`, reclassed `living_street`) and
  drops drivable-class ways that forbid cars (`motor_vehicle=no` / `access=no`)
- **Stable ids**: `edge_id` is a deterministic content hash, so rebuilds don't renumber
  the graph and downstream consumers survive without a full re-match
- **Two source modes**: build from a PBF, **or clip an area out of an existing build**
  (`sodermalm ← sweden`) preserving edge_ids — see [`docs/pipeline.md`](docs/pipeline.md)
- **Clean clipped areas**: native in-pipeline `osmium` boundary clip + a
  connected-component clean-up that drops boundary stubs
- **Validation, report & viz**: build-time invariant checks, a `reports/<name>.{md,html}`
  build report, and an optional [roadstyle](../roadstyle) network map
- **Routing-ready**: degree-2 simplified graph, edge-adjacency line graph, turn
  restrictions, H3 spatial indexing, travel-time costs — plus `route()` / `Router`
  shortest-path helpers (`from duckosm import route`)
- **Exporters**: load the network as a `networkx` graph — the edge-based routing `DiGraph`
  (`to_networkx`) or the geographic node-based `MultiDiGraph` with full edge info
  (`to_networkx_nodes`, osmnx layout) — or write either to a GraphML / gpickle file
  (`write_graph` / `duckosm export-graph`); or export a
  **SUMO** simulation net (`to_sumo` / `duckosm sumo`) whose edge ids *are* the duckOSM
  `edge_id` — so per-edge data maps onto the sim by identity, no conflation step
- **Admin boundaries**: optional table of all OSM administrative levels with a
  derived parent hierarchy
- **Portable**: single `.duckdb` file, queryable anywhere

## Installation

```bash
cd duckOSM
python -m venv .venv          # create a virtual environment (once)
source .venv/bin/activate     # activate it  (Windows: .venv\Scripts\activate)
pip install -e .              # install duckosm (editable) + dependencies
```

> **Activate the venv in every new shell.** The `duckosm` command and the
> project's dependencies live inside `.venv`, so they're only on your `PATH`
> while it's active. Run `source .venv/bin/activate` first in each new terminal
> session. If you skip activation you'll get `command not found: duckosm` — fall
> back to `.venv/bin/duckosm ...`, or just activate the venv.

Optional extras: `pip install -e ".[viz]"` (notebook maps: folium/leafmap), `".[tz]"`
(timezones), `".[dev]"` (tests/linting). The `duckosm viz` roadstyle map additionally
needs `geopandas` + [`roadstyle`](../roadstyle).

## Usage

### CLI

With the venv activated, the `duckosm` command has four subcommands. Run
`duckosm --help` for the list, or `duckosm <command> --help` for a command's options.

**`build`** — build a network from a PBF, or clip one from a parent db:

```bash
# From a YAML config (copy config/template.yaml and edit it first)
duckosm build --config config/my_area.yaml

# From CLI arguments
duckosm build --pbf data/maps/input.osm.pbf \
    --output data/output/network.duckdb --modes driving walking
```

With no `--config`, `build` loads `config/default.yaml` if it exists.

**`extract`** — slice a sub-area out of an existing build into a new db, by admin
name / boundary `osm_id` / GeoJSON file (edge_ids preserved):

```bash
duckosm extract --source data/db/sweden.duckdb \
    --db data/db/sodermalm.duckdb --name "Sodermalm"
```

**`admin`** — add OSM administrative boundaries to a built db (needs `ogr2ogr`/GDAL):

```bash
duckosm admin --pbf data/maps/sweden-latest.osm.pbf --db data/db/sweden.duckdb
```

**`viz`** — render a roadstyle HTML map per mode into `reports/` (needs `geopandas` +
`roadstyle`). Roads are styled by class with **bridge/tunnel grade separation** (tunnels under,
bridges over) on by default:

```bash
duckosm viz data/db/sodermalm.duckdb            # add --arrows for zoom-gated one-way direction arrows
# standalone renderer with palette choice (highsat | carto | mono — grayscale):
python scripts/roadstyle_map.py --db data/db/sodermalm.duckdb --palette mono
```

**`sumo`** — export a built network to a SUMO net, keeping `edge_id` as the SUMO edge id (needs
netconvert; `pip install duckosm[sumo]`):

```bash
duckosm sumo data/db/sodermalm.duckdb           # -> sumo/sodermalm.{nod,edg,net}.xml
duckosm sumo data/db/sodermalm.duckdb --out-dir net --name soder --no-netconvert  # plain-XML only
```

**`export-graph`** — write a built network to a `networkx` graph file (GraphML or gpickle; needs
`networkx`). Defaults to the geographic node-based `MultiDiGraph` with full edge info; `-g edge`
gives the edge-based routing `DiGraph`:

```bash
duckosm export-graph data/db/sodermalm.duckdb              # -> sodermalm_driving.graphml (node graph)
duckosm export-graph data/db/sodermalm.duckdb -o sm.gpickle   # gpickle (lossless, Python-only)
duckosm export-graph data/db/sodermalm.duckdb -g edge -o routing.graphml  # edge-based routing graph
```

Not activated? Use `.venv/bin/duckosm <command> ...`, `python -m duckosm <command> ...`,
or `python main.py <command> ...`.

### Python API

```python
from duckosm import DuckOSM, Config

config = Config.from_args(
    pbf_path="input.osm.pbf",
    output_path="output.duckdb",
    modes=["driving"],
)
DuckOSM(config).run()
# or: DuckOSM(Config.from_yaml("config/sweden.yaml")).run()
```

### networkx export

`to_networkx(con)` returns the network as a `networkx.DiGraph` — the **edge-based** graph: nodes are
`edge_id`s and arcs are legal turns (with a routing `weight`). Each node also carries its road
metadata (`name`, `highway`, `length_m`, `maxspeed_kmh`, `cost_s`, `geometry`), so you can analyse or
plot the network directly. Needs `pip install duckosm[routing]`.

```python
import duckdb
from duckosm import to_networkx

con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
G = to_networkx(con)                          # DiGraph; nodes = edge_id, each with road attributes
G.nodes[edge_id]["name"], G.nodes[edge_id]["length_m"]
to_networkx(con, weight="length")             # arc weight in metres (default: travel time, cost_s)
to_networkx(con, node_attrs=False)            # bare graph (routing weight only) — faster
```

For the **geographic** (node-based) view, `to_networkx_nodes(con)` returns a
`networkx.MultiDiGraph` in the osmnx layout: nodes are OSM junction `node_id`s (each with `x`/`y`
lon-lat), and every edge is a road segment from `<mode>.edges` keyed by `edge_id`, carrying its
**full attribute set** (`osm_id`, `highway`, `name`, `oneway`, `lanes`, `length_m`,
`maxspeed_kmh`, `cost_s`, `geometry`, …). Parallel ways between the same two junctions are kept, so
osmnx / momepy tooling consumes it directly.

```python
from duckosm import to_networkx_nodes

G = to_networkx_nodes(con, mode="driving")    # MultiDiGraph; nodes = junctions, edges = roads
G.nodes[node_id]["x"], G.nodes[node_id]["y"]  # lon, lat
G[u][v][edge_id]["highway"]                   # full edge attrs, keyed by edge_id
to_networkx_nodes(con, geometry="shapely")    # shapely LineStrings (needs shapely; else WKT strings)
to_networkx_nodes(con, geometry="none")       # drop geometry — lighter graph
```

To write either graph to a file, use `write_graph` / the `duckosm export-graph` CLI. GraphML is
portable (scalar-only — lists like `refs` and geometry are stringified, nulls dropped); gpickle
round-trips losslessly (incl. shapely objects) but is Python-only. The format is inferred from the
output extension (`.graphml` / `.gpickle`).

```bash
# node-based MultiDiGraph (default), GraphML -> <db-stem>_<mode>.graphml
duckosm export-graph data/db/sodermalm.duckdb

duckosm export-graph data/db/sodermalm.duckdb -o sodermalm.gpickle      # gpickle (lossless)
duckosm export-graph data/db/sodermalm.duckdb -g edge -o routing.graphml  # edge-based routing graph
```

```python
from duckosm import write_graph
write_graph(con, "sodermalm.graphml")                  # node graph, GraphML (format from extension)
write_graph(con, "routing.gpickle", graph="edge")      # edge-based routing graph, gpickle
```

### Routing (shortest path between two edges)

`edge_graph` is the edge-based routing graph (nodes = `edge_id`s; illegal turns already removed).
The `route()` / `Router` helpers wrap it with `networkx` (`pip install duckosm[routing]`):

```python
import duckdb
from duckosm import route, Router

con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
r = route(con, FROM_EDGE, TO_EDGE)          # fastest route (default); weight="length" for distance
r["edges"]                                   # ordered edge_ids
r["time_s"], r["length_m"]                   # door-to-door totals
r["path"]                                    # per-edge name/highway/length_m/cost_s/geometry

router = Router(con)                          # many routes: build the graph once, reuse it
router.route(FROM_EDGE, TO_EDGE)
```

Defaults are fastest-by-time; pass `weight="length"` to route by distance. Every road is in the
graph, incl. `highway=service`. See [`docs/query_cookbook.md`](docs/query_cookbook.md#shortest-path-between-two-edges).

### SUMO export (simulation network keyed on `edge_id`)

`to_sumo` writes SUMO *plain-XML* — `.nod.xml`, `.edg.xml` (ids = `edge_id`) and `.con.xml` (the legal
successors from `edge_graph`) — plus a standard netconvert config (`.netccfg`), then runs **netconvert**
to assemble the `.net.xml`. Because netconvert keeps the ids and the explicit connections:

- **every SUMO edge id equals the duckOSM `edge_id`** — anything keyed on `edge_id` (a per-edge
  flow/demand/regime table) maps onto the SUMO network by identity, no conflation step; and
- **turn restrictions are honoured** — junction movements are limited to the `edge_graph` successors,
  not guessed from geometry.

Needs `pip install duckosm[sumo]`.

```python
import duckdb
from duckosm import to_sumo

con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
out = to_sumo(con, "sumo/")                  # sumo/network.{nod,edg,con}.xml + .netccfg + .net.xml
out["net"], out["n_edges"], out["n_connections"]

to_sumo(con, "sumo/", run_netconvert=False)  # write only the plain-XML (assemble it yourself)
to_sumo(con, "sumo/", config={"junctions.join": "true"})   # override netconvert options
to_sumo(con, "sumo/", config="my.netccfg")   # or drive netconvert with your own config file
```

Edge `shape`, `numLanes`, `speed`, `priority`/`type` and true `length` are carried over; coordinates
are geographic and netconvert projects them. netconvert options come from a built-in default
(`DEFAULT_NETCFG`, written as a standard `.netccfg`); pass `config=` (a dict merged onto the default,
or a `.netccfg` path) to change them. CLI: `duckosm sumo data/db/sodermalm.duckdb [-c my.netccfg]`.

### Clip an area from a parent build

Build a large region once, then derive sub-areas cheaply — edge_ids are preserved, so a
sub-area is a stable view of its parent (no re-key downstream):

```bash
duckosm build --config config/sweden.yaml        # data/db/sweden.duckdb  (slow, once)
duckosm build --config config/sodermalm.yaml     # data/db/sodermalm.duckdb (fast clip)
# or ad-hoc:
duckosm build --source-db data/db/sweden.duckdb \
    --boundary data/boundaries/sodermalm.geojson \
    --output data/db/sodermalm.duckdb --modes driving
```

See [`config/template.yaml`](config/template.yaml) for the full schema (`source` /
`boundary` / `clip` / `validation` / `report` / `viz`) and
[`docs/pipeline.md`](docs/pipeline.md) for the stages. Tests: `pytest tests/`.

## Output

Each transport mode gets its own schema (`driving`, `walking`, `cycling`):

| Table | Description |
|-------|-------------|
| `edges` | Directed road segments: `edge_id` (**stable content hash** of `(osm_id, source, target, is_reverse)` — same id across rebuilds, see docs/data_dictionary.md), geometry, `length_m`, `maxspeed_kmh`, `cost_s`, `lanes` (int, per-direction), `oneway` (bool), `highway`, H3 cells, … |
| `nodes` | Junction / endpoint nodes |
| `edge_graph` | Edge adjacency (line graph) for routing — built from **all** edges, incl. `highway=service` |
| `turn_restrictions` | Turn-restriction relations (driving) |

Shared tables: `raw.*` (parsed OSM), `main.visualization_metadata`, and the optional
`main.admin_boundaries`.

See [`docs/data_dictionary.md`](docs/data_dictionary.md) for full column definitions
and [`docs/architecture.md`](docs/architecture.md) for the pipeline.

## Stable `edge_id` — reusing the hash elsewhere

`edge_id = (hash(osm_id, source, target, is_reverse) >> 1)::BIGINT` (DuckDB's `hash`). It is
deterministic — the same physical edge keeps the same id across rebuilds — so other projects can
recompute or match it. Three ways:

**1. DuckDB SQL — the persisted macro** (shipped in every output db, incl. sub-area extracts):

```sql
SELECT edge_id_hash(osm_id, source, target, is_reverse) AS edge_id FROM my_edges;
```

**2. Python helper:**

```python
from duckosm import edge_id_hash, edge_id_expr
edge_id_hash(832010768, 7767910376, 21761577, False)   # -> 6183985562678836213
# bulk: embed the SQL fragment in your own query / over a dataframe `df`
import duckdb
duckdb.sql(f"SELECT *, {edge_id_expr()} AS edge_id FROM df").df()
```

**3. Just need the id? Join on the natural key** — version-proof, never touches the hash:

```sql
... JOIN driving.edges USING (osm_id, source, target, is_reverse)
```

Notes: argument **order matters** (`osm_id, source, target, is_reverse`); integer types are
interchangeable (`INTEGER`≡`BIGINT`) and `BOOLEAN`≡`0/1`. `hash()` is a DuckDB-internal function —
reproducible only in DuckDB and **not guaranteed identical across major DuckDB versions**, so for
cross-project id matching pin the DuckDB version or use the join (option 3).

## Configuration

Copy [`config/template.yaml`](config/template.yaml) (fully commented) and edit. Full
field reference: [`docs/configuration.md`](docs/configuration.md). The schema groups
into `source` / `boundary` / `clip` / `modes` / `options` / `validation` / `report` /
`viz` blocks:

```yaml
name: my_import                        # output -> <output_path>/<name>.duckdb
output_path: data/db                   # a directory, or a full *.duckdb path

source:
  type: pbf                            # 'pbf' (build from OSM) | 'duckdb' (clip a built db)
  pbf_path: data/maps/input.osm.pbf    # type: pbf — local extract
  # country: sweden                    #   OR a Geofabrik code to auto-download
  # source_db: data/db/sweden.duckdb   # type: duckdb — parent build to clip from

boundary:                              # the clip region (set at most one)
  path: null                           # GeoJSON polygon file
  # place: "Södermalm, Stockholm"      #   OR a Nominatim place name
  # bbox: [min_lon, min_lat, max_lon, max_lat]

clip:                                  # clean-up after clipping (needs a boundary)
  predicate: intersects                # 'within' | 'intersects' | 'centroid'
  keep_largest_component: true         # drop disconnected boundary stubs -> one clean network

modes:
  - driving
  # - walking
  # - cycling

options:
  h3_resolution: 8                     # 0-15
  simplify: true                       # contract degree-2 nodes
  memory_limit: null                   # e.g. "16GB" for country-scale builds
  # build_graph / h3_indexing / process_speeds / extract_restrictions /
  # calculate_costs default to true; see config/template.yaml for the rest

validation:
  enabled: true                        # fail the build on a broken invariant

report:
  enabled: true                        # reports/<name>_<ts>.{md,html}
viz:
  enabled: false                       # roadstyle map -> reports/<name>_network.html
```

The legacy flat keys (`pbf_path`, `boundary_path`, `h3_cell` at the top level) are still
accepted as `source.type: pbf` shorthand.

## Administrative boundaries

Add all OSM administrative areas (country, county, municipality, district, …) to a
built database as an `admin_boundaries` table, with a derived `parent_osm_id` for
hierarchy traversal:

```bash
duckosm admin --pbf input.osm.pbf --db output.duckdb
```

Requires `ogr2ogr` (GDAL). The OSM `admin_level` meaning is country-specific — see
[`docs/admin_boundaries.md`](docs/admin_boundaries.md) for the levels, schema and
example queries.

## Notebooks

- [`notebooks/explore_network.ipynb`](notebooks/explore_network.ipynb) — load and map
  the network, plus admin-boundary **name search** (e.g. find the `osm_id` of
  "sodermalm"), hierarchy traversal, children, and point-in-region lookup.

## Sub-area extraction

Slice an area out of an existing database (no re-import) into a new self-contained
duckdb — by name (from `admin_boundaries`), boundary `osm_id`, or a GeoJSON file.
Edges intersecting the area are kept whole, with their nodes / edge_graph /
restrictions carried along.

```bash
duckosm extract --source data/db/sweden.duckdb \
    --db data/db/sodermalm.duckdb --name "Sodermalm"
# or: --osm-id 5691336   |   --boundary area.geojson
```

## Tools

- **Visualizer**: `duckosm viz data/db/sodermalm.duckdb` — render a roadstyle HTML map
  per mode into `reports/` (needs `geopandas` + `roadstyle`). Bridge/tunnel **grade
  separation** (tunnels under, bridges over) is on by default.
- **Standalone roadstyle map**: `scripts/roadstyle_map.py --db <db> [--palette highsat|carto|mono]
  [--color-by <col>]` — the same map with **palette selection** (incl. the
  grayscale `mono`) and data-driven colouring.
- **Comparison**: `scripts/compare_results.py` — validate output against other tools

## License

MIT
