# duckOSM User Manual

## Installation

```bash
pip install duckosm                      # core
pip install "duckosm[routing]"           # + networkx, for route() / Router / to_networkx
```

Other extras: `[sumo]` (SUMO export), `[elevation]` (DEM sampling), `[viz]`
(`duckosm viz` maps). To work on duckOSM itself, install from source: see [Development](development.md).

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
pip install "duckosm[viz]"
duckosm viz monaco.duckdb        # -> reports/monaco_<mode>_network.html, one map per mode
```

Each map shows the roads coloured by class, with a legend, a base-map switcher, a hover tooltip and
a 2D/3D button. Click a road to copy its `edge_id`. Tunnels are drawn under the roads above them,
bridges on top.

| Option | Does |
|---|---|
| `-m driving` | one mode only (default: every mode) |
| `--arrows` | direction arrows on one-way roads (zoom in to see them) |
| `--basemap positron` | first base map: `voyager` (default), `positron`, `dark_matter`, `osm`, `satellite`, `blank` |
| `--no-boundary` | hide the dashed outline of the clip area |
| `--out-dir maps` | output folder (default `reports`) |

**Other palettes, or colour by a column** (needs a clone of the repo):

```bash
python scripts/roadstyle_map.py --db monaco.duckdb --palette mono           # highsat | carto | mono
python scripts/roadstyle_map.py --db monaco.duckdb --color-by maxspeed_kmh
```

## Routing

A route goes from one edge to another over `edge_graph`, so it only takes legal turns. Needs
`pip install "duckosm[routing]"`.

```python
import duckdb
from duckosm import route, Router

con = duckdb.connect("monaco.duckdb", read_only=True)

r = route(con, from_edge, to_edge)                  # fastest, by car
r["edges"], r["time_s"], r["length_m"]              # ordered edge_ids, seconds, metres
r["path"]                                           # per edge: name, highway, length_m, cost_s, geometry

route(con, from_edge, to_edge, weight="length")     # shortest instead of fastest
route(con, from_edge, to_edge, mode="walking")      # on the walking network (or "cycling")

router = Router(con, mode="driving")                # many routes: build the graph once
router.route(from_edge, to_edge)
```

The graph is held in memory: fine for a city, heavy for a country.

### Across modes (walk → drive → walk)

`duckosm multimodal` joins the walking, cycling and driving networks at the junctions they share (a
mode change costs 60 s; `--transfer-cost` changes it). `route_multimodal` then finds the fastest
trip that may change mode on the way. It goes from one junction to another (OSM `node_id`s), starts
and ends on foot, and returns `None` if the two points aren't connected. *Experimental.*

```bash
duckosm multimodal monaco.duckdb             # adds the mm.* tables to the db
```

```python
from duckosm import route_multimodal

r = route_multimodal(con, from_node, to_node)
r["time_s"], r["legs"]                       # e.g. 552 s: walk 7 s -> drive 264 s -> walk 161 s
```

More in [Intermodal routing](multimodal.md).

### As a networkx graph

| Function | Nodes | Edges |
|---|---|---|
| `to_networkx(con)` | edges (`edge_id`), with name, highway, length, speed, geometry | legal turns, weighted by time (`weight="length"`: metres) |
| `to_networkx_nodes(con)` | junctions (`x`, `y` = lon, lat) | roads, keyed by `edge_id`, with every column (osmnx layout) |

Both take `mode=` too.

To save either one to a file (the format comes from the extension):

```bash
duckosm export-graph monaco.duckdb                            # -> monaco_driving.graphml (junctions)
duckosm export-graph monaco.duckdb -g edge -o routing.gpickle # the routing graph
```

GraphML opens in any tool but stores lists and geometry as text; gpickle keeps everything but only
loads in Python. From Python: `write_graph(con, "monaco.graphml")`.

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
