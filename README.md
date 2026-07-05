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
- **Intermodal (multimodal) routing**: stitch the per-mode networks into one layered graph
  (`mm.edges` + `mm.transfers`) so a single trip can **switch mode mid-route** — walk → drive →
  walk (park-and-ride). Build with `duckosm multimodal <db>`, route with `route_multimodal()`.
  See [`docs/multimodal.md`](docs/multimodal.md)
- **Base-map features**: optional `features.*` schema — every OSM base-map theme (roads+rail as
  `streets`, water, land, buildings, POIs, transit, places) mapped to the **Shortbread** vector-tile
  schema and extracted **once into the same db**, so a renderer like [duckmap](../duckmap) draws from
  it instead of re-ingesting OSM. Enable with `options.build_features`. See
  [`docs/features_schema.md`](docs/features_schema.md)
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
  (`write_graph` / `duckosm export-graph`); export a
  **SUMO** simulation net (`to_sumo` / `duckosm sumo`) whose edge ids *are* the duckOSM
  `edge_id` — so per-edge data maps onto the sim by identity, no conflation step; or write the
  geographic network to **GeoPackage / shapefile** for any GIS tool (`to_gis` /
  `duckosm export-gis`, `edge_id` preserved as an attribute — see [`docs/gis_export.md`](docs/gis_export.md)),
  then verify that export with `duckosm gis-debug` (reads the file back through GDAL → an HTML map +
  `edge_id`/round-trip QA audit); or extract a standalone **GMNS** network DuckDB — every GMNS table
  OSM supports incl. **lane detail** and turn **movements** (`to_gmns` / `duckosm gmns`, `link_id` =
  `edge_id` — see [`docs/gmns_export.md`](docs/gmns_export.md)), optionally with **mesoscopic**
  (`--meso`) and **microscopic** cell-based (`--micro`) lane-level networks, a single mode-tagged
  **combined** network (`--combined`), drive-side-aware lane geometry and smooth Bézier turn
  connectors (see [`docs/gmns_meso.md`](docs/gmns_meso.md), [`docs/gmns_micro.md`](docs/gmns_micro.md),
  [`docs/gmns_map_realism.md`](docs/gmns_map_realism.md)) — and an **interactive HTML viewer** to
  explore lanes / meso / micro with hover tooltips (`duckosm gmns-viz` —
  [`docs/gmns_viewer.md`](docs/gmns_viewer.md))
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

With the venv activated, the `duckosm` command has several subcommands (`build`, `extract`, `admin`,
`viz`, `sumo`, `export-graph`, `export-gis`, `gis-debug`, `gmns`, `gmns-viz`, `gmns-map`, `matsim`, `matsim-lanes`, `multimodal`). Run `duckosm --help` for the list, or
`duckosm <command> --help` for a command's options.

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

**`export-gis`** — export the geographic network to **GeoPackage** (multi-layer, primary) or
**shapefile** (`--format shp`) for QGIS / ArcGIS / any GIS tool, with `edge_id` preserved as an
attribute. Writes `edges_<mode>` / `nodes_<mode>` / `boundary` layers in EPSG:4326; GeoPackage
assembly needs `ogr2ogr` (GDAL). See [`docs/gis_export.md`](docs/gis_export.md):

```bash
duckosm export-gis data/db/sodermalm.duckdb                 # -> sodermalm.gpkg (all modes + boundary)
duckosm export-gis data/db/sodermalm.duckdb -m driving      # only the driving schema
duckosm export-gis data/db/sodermalm.duckdb --format shp -o gis/   # shapefile set into gis/
```

**`gis-debug`** — verify a GIS export by reading the file **back through GDAL** (the path QGIS /
ArcGIS / FME take) and writing a self-contained HTML debug page: a canvas map of every layer (edges
by highway class, nodes, boundary; per-mode toggles, pan/zoom, no tiles) plus a QA audit — feature
counts, geometry type, CRS, and `edge_id` integrity (a **float** dtype means a shapefile DBF rounded
the 64-bit id). With `--source-db` it runs a **round-trip diff** — the exported `edge_id` set vs the
db's, per mode — and reports an overall `PASS` / `CHECK` / `FAIL`. Verifies the file on disk, not the
db. Needs `geopandas` + `pyogrio`:

```bash
duckosm gis-debug sodermalm.gpkg --source-db data/db/sodermalm.duckdb   # -> sodermalm_gis_debug.html
duckosm gis-debug gis/ -o reports/gis_debug.html                        # a shapefile directory
```

**`gmns`** — extract a built network to a **standalone GMNS DuckDB** (the open network standard for
DTALite / Path4GMNS / the AMS ecosystem). Writes every GMNS table OSM supports — `config`, `node`,
`link`, `geometry`, **`lane`** (per-lane rows with turns/uses/width from OSM lane tags), **`movement`**
(turns from the line graph), `use_definition`/`use_group`, `signal_controller`, `curb_seg` — with
native geometry so it renders on its own, and `link_id = edge_id`. `--to-csv` also dumps the spec CSVs.
See [`docs/gmns_export.md`](docs/gmns_export.md):

```bash
duckosm gmns data/db/sodermalm.duckdb                       # -> sodermalm_pbf_gmns.duckdb (all modes)
duckosm gmns data/db/sodermalm.duckdb -m driving --to-csv gmns/   # driving only, + spec CSVs
duckosm gmns data/db/sodermalm.duckdb --meso --micro        # + mesoscopic + microscopic (cell) networks
duckosm gmns data/db/sodermalm.duckdb --combined            # + a single mode-tagged gmns_all network
duckosm gmns data/db/sodermalm.duckdb --drive-side left     # left-hand traffic lane offset
```

Lane geometry is **drive-side aware** (two-way roads separate onto their travel sides) and turn
connectors are **smooth Béziers**, so junctions render like real roads. `--meso` adds a lane-level
mesoscopic network (section + turn-connector links); `--micro` adds a cell-based microscopic network
(lane cells + lane-change mesh + turn connectors) for microsimulation; `--combined` merges the modes
into one `gmns_all` network tagged by `allowed_uses`.

**`gmns-viz`** — write a **self-contained interactive HTML viewer** for a GMNS DuckDB: toggle between
individual **lanes** (offset by use), the **mesoscopic** section + turn-connector network, and the
**microscopic** cell mesh; **hover any line** for its id/attributes, scroll-zoom / drag-pan. Needs
only DuckDB (geometry drawn client-side). See [`docs/gmns_viewer.md`](docs/gmns_viewer.md):

```bash
duckosm gmns-viz sodermalm_pbf_gmns.duckdb                  # -> sodermalm_pbf_gmns_viewer.html
```

**`gmns-map`** — write a **pretty** self-contained HTML map of a GMNS DuckDB (presentation, vs
`gmns-viz`'s inspection): `--style road` draws one **carriageway ribbon per direction** coloured by
road class (two-way roads split in two); `--style lane` draws **every lane as a ribbon of its real
width**. Both overlay smooth turn connectors, on a dark canvas with pan/zoom. See
[`docs/gmns_map.md`](docs/gmns_map.md):

```bash
duckosm gmns-map sodermalm_pbf_gmns.duckdb                  # road-by-direction (-> ..._road.html)
duckosm gmns-map sodermalm_pbf_gmns.duckdb --style lane     # every lane by width
```

**`matsim`** — export a **MATSim `network.xml`** from a built db: each edge becomes one directed
`<link>` (stable `edge_id` preserved) with `length` / `freespeed` / `capacity` / `permlanes` / `modes`,
and node coordinates reprojected to a metric CRS (default `EPSG:3006` SWEREF99 TM). The directed
node+link substrate for **MATSim / BEAM / eqasim**. See [`docs/matsim_export.md`](docs/matsim_export.md):

```bash
duckosm matsim sodermalm_pbf.duckdb                        # -> sodermalm_pbf_network.xml.gz (driving, EPSG:3006)
duckosm matsim sodermalm_pbf.duckdb --mode all             # multimodal: one network, modes=car,bike,walk per link
duckosm matsim tartu_pbf.duckdb --crs EPSG:32635 --no-gzip # UTM 35N, plain XML
```

`--mode all` (or a comma-list like `driving,cycling`) merges the per-mode networks into one, keyed on
the mode-stable `edge_id`, so a segment shared by several modes becomes a single link tagged with all
its `modes` — the form MATSim/BEAM want for multimodal agents.

**`matsim-lanes`** — export MATSim **turn lanes** (`lanes.xml`) and **signals**
(`signalSystems`/`signalGroups`/`signalControl.xml`) from a **GMNS db** (its `movement` table is the
lane→turn model; `link_id` = `edge_id`, so the files pair with the `matsim` network). Every file is
validated against the official MATSim v2.0 XSD schemas. See
[`docs/matsim_lanes_signals.md`](docs/matsim_lanes_signals.md):

```bash
duckosm gmns         sodermalm_pbf.duckdb -o gmns.duckdb   # movements + signalised nodes (prereq)
duckosm matsim-lanes gmns.duckdb                           # -> lanes.xml + signalSystems/Groups/Control.xml
duckosm matsim-lanes gmns.duckdb --no-signals              # lanes.xml only
```

Turn *connectivity* (legal turns) and *which* junctions are signalised are real; per-lane turn
assignment needs `turn:lanes` tags, and the signal **timing** is a default fixed-time plan (OSM has no
signal plans) — a calibrate-me placeholder.

**`multimodal`** — stitch the per-mode networks of a built db into an intermodal `mm.*` graph
(`mm.edges` + `mm.transfers`) so a trip can switch mode mid-route (walk→drive→walk). Needs ≥2 modes
including `walking`; writes the `mm` schema in place. Route the result with
`duckosm.route_multimodal(con, src_node, dst_node)`:

```bash
duckosm multimodal data/db/sodermalm.duckdb --transfer-cost 60    # flat transfer penalty (seconds)
# or enable it during a build: modes: [driving, walking] + multimodal: {enabled: true} in the config
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
