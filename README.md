# duckOSM

High-performance OSM-to-routing-network converter built on DuckDB.

## Features

- **Fast**: Uses DuckDB's native `ST_READOSM` for 100x faster PBF parsing
- **Memory Efficient**: Streaming SQL processing, no Python object overhead
- **Portable**: Single `.duckdb` file output, queryable anywhere
- **Configurable**: YAML config or CLI arguments

## Installation

```bash
cd duckOSM
pip install -e .
```

## Usage

### CLI

```bash
# Using config file
python -m duckosm --config config/default.yaml

# Using CLI arguments
python -m duckosm \
    --pbf data/maps/input.osm.pbf \
    --output data/output/network.duckdb \
    --graph
```

### Python API

```python
from duckosm import DuckOSM

importer = DuckOSM(
    pbf_path="input.osm.pbf",
    output_path="output.duckdb"
)
importer.run()
```

## Output Tables

| Table | Description |
|-------|-------------|
| `nodes` | Road network nodes with coordinates |
| `edges` | Road segments with attributes |
| `turn_restrictions` | Turn restriction relations |
| `edge_graph` | Edge adjacency for routing |

## Configuration

```yaml
name: "my_import"
pbf_path: "data/maps/input.osm.pbf"
output_path: "data/output/network.duckdb"

options:
  build_graph: true
  h3_indexing: true
  h3_resolution: 8
  simplify: false  # Graph simplification (experimental)

## Tools

- **Visualizer**: Run `streamlit run scripts/visualize.py` to explore the network on an interactive map.
- **Comparison**: Use `scripts/compare_results.py` to validate output against other tools.
```

## License

MIT
