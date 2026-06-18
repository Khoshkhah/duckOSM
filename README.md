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
- **Service roads separated**: `highway=service` (driveways/alleys) are moved into a
  `service_edges` table so the routing graph is real roads only; the full graph is kept as
  `edge_graph_with_service` for when you need them. Visualization still shows every road.
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
`roadstyle`):

```bash
duckosm viz data/db/sodermalm.duckdb            # add --arrows for zoom-gated one-way direction arrows
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

### Routing (shortest path between two edges)

`edge_graph` is the edge-based routing graph (nodes = `edge_id`s; illegal turns already removed).
The helpers wrap it with `networkx` (`pip install duckosm[routing]`):

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

Defaults are fastest-by-time and service-free; pass `weight="length"` or `with_service=True` to
change. See [`docs/query_cookbook.md`](docs/query_cookbook.md#shortest-path-between-two-edges).

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
| `edge_graph` | Edge adjacency (line graph) for routing — **service-free** by default |
| `edge_graph_with_service` | Full edge adjacency incl. `service` roads (present when any were split out) |
| `service_edges` | `highway=service` edges moved out of the routing graph (same columns as `edges`) |
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
  per mode into `reports/` (needs `geopandas` + `roadstyle`)
- **Comparison**: `scripts/compare_results.py` — validate output against other tools

## License

MIT
