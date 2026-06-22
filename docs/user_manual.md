# duckOSM User Manual

## Installation

```bash
git clone https://github.com/your-org/duckOSM.git
cd duckOSM
python3 -m venv .venv
source .venv/bin/activate
pip install -e .
```

## Quick Start

```bash
# Using config file (recommended)
duckosm build --config config/sweden.yaml

# Or with CLI options
duckosm build --pbf input.osm.pbf --output network.duckdb
```

## CLI Options

| Option | Description | Default |
|--------|-------------|---------|
| `--pbf`, `-p` | Input PBF file path | Required |
| `--output`, `-o` | Output DuckDB file | `output.duckdb` |
| `--config`, `-c` | YAML config file | None |
| `--modes` | Transportation modes | `driving` |
| `--h3-resolution` | H3 resolution (0-15) | `8` |

## Configuration File

```yaml
# config/my_import.yaml
name: "my_region"
pbf_path: "data/maps/region.osm.pbf"
output_path: "data/output/region.duckdb"

options:
  simplify: true
  build_graph: true
  h3_indexing: true
  h3_resolution: 8
  process_speeds: true
  extract_restrictions: true
  calculate_costs: true

modes:
  - driving
  - walking
  - cycling
```

## Output Tables

Each mode has its own schema (e.g., `driving.edges`, `walking.edges`).

### `nodes`
| Column | Type | Description |
|--------|------|-------------|
| `node_id` | BIGINT | OSM node ID (negative = virtual node) |
| `geom` | GEOMETRY | Point geometry |
| `h3_cell` | UBIGINT | H3 spatial index |

### `edges`
| Column | Type | Description |
|--------|------|-------------|
| `edge_id` | INTEGER | Unique edge ID |
| `source` | BIGINT | Source node ID |
| `target` | BIGINT | Target node ID |
| `osm_id` | BIGINT | Original OSM way ID |
| `highway` | VARCHAR | Highway type |
| `name` | VARCHAR | Street name |
| `length_m` | FLOAT | Length in meters |
| `maxspeed_kmh` | FLOAT | Speed limit (km/h) |
| `cost_s` | FLOAT | Travel time (seconds) |
| `geometry` | GEOMETRY | LineString geometry |
| `is_reverse` | BOOLEAN | True if reverse direction |
| `from_cell` | UBIGINT | Source node H3 cell |
| `to_cell` | UBIGINT | Target node H3 cell |

### `edge_graph`
Edge-based routing graph (line graph), built from **all** edges — every road, incl.
`highway=service` (driveways/alleys), is routable.

| Column | Type | Description |
|--------|------|-------------|
| `from_edge` | INTEGER | Incoming edge |
| `to_edge` | INTEGER | Outgoing edge |
| `via_edge` | INTEGER | Same as to_edge |
| `cost` | FLOAT | Travel cost of from_edge |

### `turn_restrictions`
| Column | Type | Description |
|--------|------|-------------|
| `restriction_id` | BIGINT | OSM relation ID |
| `restriction_type` | VARCHAR | e.g., `no_left_turn` |
| `from_edge_id` | INTEGER | Restricted from edge |
| `to_edge_id` | INTEGER | Restricted to edge |

## Python API

```python
from duckosm import DuckOSM, Config

# From YAML config
config = Config.from_yaml("config/my_import.yaml")
importer = DuckOSM(config)
output_path = importer.run()

# Query results
import duckdb
con = duckdb.connect(str(output_path), read_only=True)
edges = con.sql("SELECT * FROM driving.edges LIMIT 10").fetchall()
```

## Example Queries

```sql
-- Find fastest roads
SELECT name, highway, maxspeed_kmh, length_m 
FROM driving.edges 
ORDER BY maxspeed_kmh DESC LIMIT 10;

-- Count edges by type
SELECT highway, count(*) 
FROM driving.edges 
GROUP BY highway ORDER BY count DESC;

-- Find connected edges (line graph)
SELECT to_edge, cost 
FROM driving.edge_graph 
WHERE from_edge = 123;

-- Edges in an H3 cell
SELECT * FROM driving.edges 
WHERE from_cell = 617700169958293503;
```

## Visualization

```bash
duckosm viz data/db/sodermalm.duckdb
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
duckosm viz data/db/sodermalm.duckdb --arrows
```

The clip/area **boundary** (`main.boundary`) is overlaid as a dashed outline by default, so the
map shows the extent the network was clipped to; turn it off with `--no-boundary`.

**Grade separation.** Both `duckosm viz` and the standalone renderer carry the
`bridge` / `tunnel` / `layer` columns into roadstyle, which draws **tunnels underneath, bridges on
top with a solid casing**, and z-orders edges so over/underpasses don't look connected. Every road,
including `highway=service`, lives in `edges`, so the map shows the whole network.

### Standalone renderer & palettes

`scripts/roadstyle_map.py` is a standalone alternative to `duckosm viz` that adds **palette
selection**:

```bash
python scripts/roadstyle_map.py --db data/db/sodermalm.duckdb --palette mono
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

con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)

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

## SUMO export

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

con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
out = to_sumo(con, "sumo/")                  # sumo/network.{nod,edg,con}.xml + .netccfg + .net.xml
out["net"], out["n_edges"], out["n_connections"]

to_sumo(con, "sumo/", run_netconvert=False)            # only the plain-XML (assemble it yourself)
to_sumo(con, "sumo/", config={"junctions.join": "true"})   # override default netconvert options
to_sumo(con, "sumo/", config="my.netccfg")             # or drive it with your own config file
```

Or from the CLI: `duckosm sumo data/db/sodermalm.duckdb` (`--no-netconvert` for plain-XML only,
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
