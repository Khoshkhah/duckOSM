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
python main.py --config config/somerset.yaml

# Or with CLI options
python main.py --pbf input.osm.pbf --output network.duckdb
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
streamlit run scripts/visualize.py
```

Features:
- Toggle modes (driving/walking/cycling)
- View edge properties on hover
- Two-way roads shown as parallel lines

## Troubleshooting

| Issue | Solution |
|-------|----------|
| H3 extension not available | Falls back to Python H3 (slower) |
| Empty turn_restrictions | Normal if area has no restrictions |
| Lock error on database | Use `read_only=True` or close other connections |
