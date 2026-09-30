# duckOSM User Manual

## Installation

```bash
pip install duckosm                      # core
pip install "duckosm[routing]"           # + networkx, for route() / Router / to_networkx
```

Other extras: `[sumo]` (SUMO export), `[elevation]` (DEM sampling), `[tz]` (time zones), `[viz]`
(notebook maps). To work on duckOSM itself, install from source: see [Development](development.md).

Every relative path duckOSM uses (outputs, configs, reports) resolves against the folder you run
`duckosm` in.

## Build a network

```bash
# From a config file (recommended)
duckosm init-config my_area.yaml          # write the commented template, then edit it
duckosm build --config my_area.yaml

# Or from CLI options
duckosm build --pbf input.osm.pbf --output network.duckdb -m driving -m walking
```

With no `--config` and no `--pbf` / `--source-db`, `build` loads `config/default.yaml` if it exists.

### CLI options (`duckosm build`)

| Option | Description | Default |
|--------|-------------|---------|
| `-c`, `--config` | YAML config file | none |
| `-p`, `--pbf` | Input `.osm.pbf` (or give a config / `--source-db`) | none |
| `-o`, `--output` | Output DuckDB file | `<pbf name>.duckdb` in the current folder (`<boundary name>.duckdb` for a clip) |
| `-b`, `--boundary` | GeoJSON boundary to clip to | none |
| `--source-db` | Parent duckOSM db to clip from (instead of a PBF) | none |
| `-m`, `--modes` | `driving`, `walking` or `cycling`; repeat for several (`-m driving -m walking`) | `driving` |
| `--graph` / `--no-graph` | Build the `edge_graph` table | on |
| `--h3-index` / `--no-h3-index` | Add H3 cells to nodes and edges | on |
| `--h3-resolution` | H3 resolution (0-15) | `8` |
| `--log-file` | Also write the log to this file | none (console only) |

### Configuration file

`duckosm init-config my_area.yaml` writes the fully commented [template](https://github.com/Khoshkhah/duckOSM/blob/main/src/duckosm/templates/config.yaml); every field is
described in [Configuration](configuration.md). In a clone of the repo, `config/sample_monaco.yaml`
builds the bundled Monaco extract in seconds: `duckosm build --config config/sample_monaco.yaml`.
The core of the template:

```yaml
name: my_area                          # output file: <output_path>/<name>.duckdb
output_path: .                         # a directory, or a full *.duckdb path

source:
  type: pbf                            # 'pbf' (build from OSM) | 'duckdb' (clip a built db)
  pbf_path: data/maps/input.osm.pbf

boundary:                              # optional clip region
  path: null                           # GeoJSON polygon file

modes: [driving, walking, cycling]

options:
  h3_resolution: 8
  simplify: true                       # contract degree-2 nodes and merge same-road chains

validation:
  enabled: true                        # fail the build on a broken invariant
```

## Clip an area from a bigger build

Build a large region once, then cut areas out of it. No OSM is re-read, and every `edge_id` is the
parent's, so data keyed on the parent's ids works on the area as-is.

```bash
duckosm build --config sweden.yaml                              # slow, once

duckosm extract --source sweden.duckdb \
    --db sodermalm.duckdb --boundary sodermalm.geojson   # seconds
```

`extract` selects the area by `--boundary` (GeoJSON), or by `--name` / `--osm-id` when the parent
has [admin boundaries](admin_boundaries.md). Edges that intersect the area are kept whole. The same
clip also runs inside a build with `source.type: duckdb` and `source_db:` in the config; see
[Pipeline](pipeline.md).

## What a build contains

Each mode gets its own schema (`driving`, `walking`, `cycling`) with:

| Table | |
|---|---|
| `edges` | directed road segments: `edge_id`, `source` → `target`, geometry, `length_m`, `maxspeed_kmh`, `cost_s`, `lanes`, `oneway`, `highway`, `name`, … |
| `nodes` | junctions and end points (`node_id < 0` = virtual node) |
| `edge_graph` | legal edge → edge turns (`from_edge`, `to_edge`): the routing graph, built from every edge incl. `highway=service` |
| `turn_restrictions` | OSM turn restrictions (driving only) |

Every id column (`edge_id`, `source`, `target`, `from_edge`, …) is `BIGINT`; keep it `BIGINT` when
you join. H3 columns (`nodes.h3_cell`, `edges.from_cell` / `to_cell` / `lca_res`) exist only when
H3 indexing is on. Every column is described in the [data dictionary](data_dictionary.md).

## Python API

```python
from duckosm import DuckOSM, Config

# From YAML config
config = Config.from_yaml("my_area.yaml")
output_path = DuckOSM(config).run()

# Query results
import duckdb
con = duckdb.connect(str(output_path), read_only=True)
edges = con.sql("SELECT * FROM driving.edges LIMIT 10").fetchall()
```

## Example Queries

```sql
-- Fastest roads
SELECT name, highway, maxspeed_kmh, length_m
FROM driving.edges
ORDER BY maxspeed_kmh DESC LIMIT 10;

-- Edges by road class
SELECT highway, count(*) AS n
FROM driving.edges
GROUP BY highway ORDER BY n DESC;

-- Legal next edges after one edge (the line graph)
SELECT to_edge, cost
FROM driving.edge_graph
WHERE from_edge = 7968481847680619937;

-- Edges starting in one H3 cell (needs H3 indexing)
SELECT * FROM driving.edges
WHERE from_cell = 617700169958293503;
```

More in the [query cookbook](query_cookbook.md).

### Look up one OSM way by its `osm_id`

`duckosm way` shows all the data a build has for one OSM **way** (the OpenStreetMap object for a
street, or a stretch of one): first its raw OSM tags and node list, then every edge built from it,
in every mode, as one table.

```bash
duckosm way monaco.duckdb 24672722                  # Avenue Delphine; -m driving for one mode, --geom for WKT
duckosm way monaco.duckdb 24672722 -o way.csv       # write the table to a file (.csv / .parquet / .json)
```

```python
from duckosm.query import way_table
way_table(con, 24672722).show()                     # the same table in Python / a notebook
```

The table has one row per edge per mode, in travel order. A two-way street gives two edges, one per
direction, and the same stretch of road has the same `edge_id` in every mode. Use it when a
particular street looks wrong: how it was split, which edges came from it, what speed and cost each
got. A negative `osm_id` looks up the short connector edges duckOSM adds to join dangling paths.

## Visualization

```bash
duckosm viz monaco.duckdb
```

Renders a publication-quality roadstyle HTML map per mode
(`reports/<name>_<mode>_network.html`): edges styled by highway class, a
toggleable base-map switcher, a legend, and click-to-copy `edge_id`. Needs
`geopandas` + `roadstyle` installed. Use `--mode driving` to render a single
mode, or `--basemap` / `--out-dir` to tweak the output.

Add `--arrows` to overlay **one-way direction arrows** — a small gray chevron at
each one-way edge's midpoint pointing `source → target` (the legal travel
direction). They are zoom-gated (rendered only at zoom ≥ 18) so the zoomed-out
view stays clean; zoom in to a one-way street to see them. Two-way roads are not
arrowed (each is a forward+reverse edge pair, so arrowing all edges is too heavy).

```bash
duckosm viz monaco.duckdb --arrows
```

The clip/area **boundary** (`main.boundary`) is overlaid as a dashed outline by default, so the
map shows the extent the network was clipped to; turn it off with `--no-boundary`.

**Grade separation.** Both `duckosm viz` and the standalone renderer carry the
`bridge` / `tunnel` / `layer` columns into roadstyle, which draws **tunnels underneath, bridges on
top with a solid casing**, and z-orders edges so over/underpasses don't look connected. Every road,
including `highway=service`, lives in `edges`, so the map shows the whole network.

### Standalone renderer & palettes

`scripts/roadstyle_map.py` is a standalone alternative to `duckosm viz` that adds **palette
selection**. It lives in the repo's `scripts/` folder (not in the pip package), so it needs a clone:

```bash
python scripts/roadstyle_map.py --db sodermalm.duckdb --palette mono
```

`--palette` picks the roadstyle palette — `highsat` (default, high-contrast), `carto`
(OSM-standard look), or `mono` (grayscale). It renders the same bridge/tunnel grade separation.
`--color-by <column>` colours edges by a numeric/categorical column instead of road class;
`--out` / `--basemap` / `--theme` tweak the output.

## Routing (shortest path)

`edge_graph` is the edge-based routing graph (nodes = `edge_id`s; illegal turns already
removed). The helpers wrap it with `networkx` — `pip install duckosm[routing]`:

```python
import duckdb
from duckosm import route, Router

con = duckdb.connect("monaco.duckdb", read_only=True)

# one-off: shortest route between two edge_ids (defaults: fastest by time)
r = route(con, FROM_EDGE, TO_EDGE)
r["edges"]        # ordered edge_ids
r["time_s"]       # total travel time (door-to-door)
r["length_m"]     # total length
r["path"]         # per-edge name/highway/length_m/cost_s/geometry, in order

# many routes: build the graph once, reuse it
router = Router(con)
router.route(FROM_EDGE, TO_EDGE)
```

Options: `weight="length"` routes by distance instead of time (every road, incl. `highway=service`,
is in the graph). `to_networkx(con, ...)` returns the weighted `DiGraph`
directly for your own networkx algorithms — by default each node (`edge_id`) carries its road
metadata (`name`, `highway`, `length_m`, `maxspeed_kmh`, `cost_s`, `geometry`); pass
`node_attrs=False` for a bare, faster graph. This loads the graph into memory — fine for a city;
for country scale prefer an on-disk A\* over `edge_graph`.

For the **geographic** (node-based) view, `to_networkx_nodes(con, mode=...)` returns a
`networkx.MultiDiGraph` in the osmnx layout: nodes are OSM junction `node_id`s (each with `x`/`y`
lon-lat plus any extra node columns such as `h3_cell`), and every edge is a road segment from
`<mode>.edges` keyed by `edge_id`, carrying its **full attribute set** (`osm_id`, `highway`, `name`,
`oneway`, `lanes`, `length_m`, `maxspeed_kmh`, `cost_s`, `geometry`, …). Parallel ways between the
same two junctions are preserved, so osmnx / momepy tooling consumes it directly. Geometry is a WKT
string by default; pass `geometry="shapely"` for shapely objects or `geometry="none"` to drop it.

```python
from duckosm import to_networkx_nodes

G = to_networkx_nodes(con, mode="driving")    # MultiDiGraph; nodes = junctions, edges = roads
G.nodes[node_id]["x"], G.nodes[node_id]["y"]  # lon, lat
G[u][v][edge_id]["highway"]                   # full edge attrs, keyed by edge_id
```

### networkx file export (`export-graph`)

To persist either graph to disk, use `write_graph(con, path, ...)` or the `duckosm export-graph`
CLI. The format is inferred from the output extension: **GraphML** (`.graphml`) is portable but
scalar-only — list attributes like `refs` and the geometry are stringified and null values dropped;
**gpickle** (`.gpickle`) round-trips the graph losslessly (including shapely geometry) but is
Python-only. Defaults to the node-based graph; pass `graph="edge"` / `-g edge` for the edge-based
routing graph. Needs `networkx`.

```bash
duckosm export-graph monaco.duckdb              # -> monaco_driving.graphml (node graph)
duckosm export-graph monaco.duckdb -o sm.gpickle   # gpickle (lossless)
duckosm export-graph monaco.duckdb -g edge -o routing.graphml  # edge-based routing graph
```

```python
from duckosm import write_graph
write_graph(con, "monaco.graphml")                     # node graph, GraphML (format from extension)
write_graph(con, "routing.gpickle", graph="edge")         # edge-based routing graph, gpickle
```

## Exports

Every exporter keeps `edge_id` as the target format's own id.

| Format | Command | Details |
|---|---|---|
| SUMO | `duckosm sumo` | below |
| MATSim | `duckosm matsim`, `matsim-lanes` | [MATSim network](matsim_export.md), [lanes & signals](matsim_lanes_signals.md) |
| GMNS | `duckosm gmns` | [GMNS](gmns_export.md) |
| GeoPackage / shapefile | `duckosm export-gis` | [GeoPackage / shapefile](gis_export.md) |
| networkx | `duckosm export-graph` | above |

### SUMO export

Export the network to a [SUMO](https://eclipse.dev/sumo/) simulation net whose edge ids **are** the
duckOSM `edge_id`. `to_sumo` writes SUMO plain-XML — `.nod.xml`, `.edg.xml` (ids = `edge_id`) and
`.con.xml` (the legal successors from `edge_graph`) — plus a standard netconvert config (`.netccfg`),
then runs **netconvert** to assemble the `.net.xml`. netconvert keeps the ids and the explicit
connections, so per-edge data (flow / demand / a regime table) maps onto the simulation **by
identity** (no geometry conflation) **and turn restrictions are honoured** (movements limited to the
`edge_graph` successors, not guessed). Needs `pip install duckosm[sumo]`.

```python
import duckdb
from duckosm import to_sumo

con = duckdb.connect("monaco.duckdb", read_only=True)
out = to_sumo(con, "sumo/")                  # sumo/network.{nod,edg,con}.xml + .netccfg + .net.xml
out["net"], out["n_edges"], out["n_connections"]

to_sumo(con, "sumo/", run_netconvert=False)            # only the plain-XML (assemble it yourself)
to_sumo(con, "sumo/", config={"junctions.join": "true"})   # override default netconvert options
to_sumo(con, "sumo/", config="my.netccfg")             # or drive it with your own config file
```

Or from the CLI: `duckosm sumo monaco.duckdb` (`--no-netconvert` for plain-XML only,
`--no-connections` to let netconvert infer turns, `-c my.netccfg` for a custom config, `--out-dir` /
`--name`). Edge `shape`, `numLanes`, `speed`, `priority`/`type` and true `length` are carried over;
coordinates are geographic and netconvert projects them. The netconvert options come from a built-in
default (`DEFAULT_NETCFG`), written as a standard `.netccfg`; pass `config=` (a dict merged onto the
default, or a `.netccfg` path) to change them.

## Troubleshooting

| Issue | Solution |
|-------|----------|
| H3 extension not available | Falls back to Python H3 (slower) |
| Empty turn_restrictions | Normal if area has no restrictions |
| Lock error on database | Use `read_only=True` or close other connections |
