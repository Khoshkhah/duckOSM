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
- **Routing-ready**: degree-2 simplified graph, edge-adjacency table, turn
  restrictions, H3 spatial indexing, travel-time costs
- **Admin boundaries**: optional table of all OSM administrative levels with a
  derived parent hierarchy
- **Portable**: single `.duckdb` file, queryable anywhere

## Installation

```bash
cd duckOSM
pip install -e .
```

## Usage

### CLI

```bash
# From a YAML config
python -m duckosm --config config/default.yaml

# From CLI arguments
python -m duckosm \
    --pbf data/maps/input.osm.pbf \
    --output data/output/network.duckdb \
    --modes driving walking
```

### Python API

```python
from duckosm import DuckOSM, Config

config = Config.from_args(
    pbf_path="input.osm.pbf",
    output_path="output.duckdb",
    modes=["driving"],
)
DuckOSM(config).run()
# or: DuckOSM(Config.from_yaml("config/default.yaml")).run()
```

## Output

Each transport mode gets its own schema (`driving`, `walking`, `cycling`):

| Table | Description |
|-------|-------------|
| `edges` | Directed road segments: `edge_id` (**stable content hash** of `(osm_id, source, target, is_reverse)` — same id across rebuilds, see docs/data_dictionary.md), geometry, `length_m`, `maxspeed_kmh`, `cost_s`, `lanes` (int, per-direction), `oneway` (bool), `highway`, H3 cells, … |
| `nodes` | Junction / endpoint nodes |
| `edge_graph` | Edge adjacency for routing |
| `turn_restrictions` | Turn-restriction relations (driving) |

Shared tables: `raw.*` (parsed OSM), `main.visualization_metadata`, and the optional
`main.admin_boundaries`.

See [`docs/data_dictionary.md`](docs/data_dictionary.md) for full column definitions
and [`docs/architecture.md`](docs/architecture.md) for the pipeline.

## Configuration

Copy [`config/template.yaml`](config/template.yaml) and edit. Full field reference:
[`docs/configuration.md`](docs/configuration.md).

```yaml
name: "my_import"
pbf_path: "data/maps/input.osm.pbf"
output_path: "data/output/network.duckdb"   # or a directory -> <name>.duckdb
boundary_path: null                          # optional GeoJSON clip

options:
  build_graph: true
  h3_indexing: true
  h3_resolution: 8
  simplify: true
  process_speeds: true
  extract_restrictions: true
  calculate_costs: true
  # Boundary H3 grid (optional; needs boundary_path):
  boundary_cells: false             # write main.boundary_cells
  boundary_cell_resolutions: null   # e.g. [6, 7, 8]; null = [h3_resolution]
  timezone: false                   # add IANA timezone (needs timezonefinder)
  # Large-build tuning (optional):
  memory_limit: "16GB"     # cap DuckDB memory (null = ~80% of RAM)
  threads: null            # cap worker threads
  simplify_batches: 0      # 0 = auto-size by node count; 1 = single pass

modes:
  - driving
  - walking
  - cycling
```

## Administrative boundaries

Add all OSM administrative areas (country, county, municipality, district, …) to a
built database as an `admin_boundaries` table, with a derived `parent_osm_id` for
hierarchy traversal:

```bash
python scripts/add_admin_boundaries.py --pbf input.osm.pbf --db output.duckdb
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
python scripts/extract_area.py --source data/db/sweden.duckdb \
    --db data/db/sodermalm.duckdb --name "Sodermalm"
# or: --osm-id 5691336   |   --boundary area.geojson
```

## Tools

- **Visualizer**: `streamlit run scripts/visualize.py` — explore the network on a map
- **Comparison**: `scripts/compare_results.py` — validate output against other tools

## License

MIT
