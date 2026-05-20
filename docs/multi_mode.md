# Multi-Mode Routing Support

duckOSM allows you to generate multiple routing networks (Driving, Walking, Cycling) in a single import run. Each mode is isolated within its own DuckDB schema, allowing for mode-specific filtering, speed profiles, and topological rules.

## Available Modes

| Mode | Target Users | Included Highway Tags | Speed Profile |
| :--- | :--- | :--- | :--- |
| `driving` | Cars, Motorcycles, Trucks | Motorways, Primary, Residential, etc. | `maxspeed` tags or defaults |
| `walking` | Pedestrians, Hikers | Footways, Paths, Pedestrian areas, Sidewalks | Fixed 5 km/h |
| `cycling` | Bicycles | Cycleways, Tracks, Cycle-friendly roads | Fixed 15 km/h |

## Configuration

To enable multiple modes, add the `modes` list to your configuration file:

```yaml
modes:
  - driving
  - walking
  - cycling
```

If omitted, the importer defaults to `["driving"]`.

## Database Schema Structure

When multiple modes are enabled, the database is organized as follows:

- `raw`: Contains the original OSM data (nodes, ways, relations).
- `driving`: Tables for the driving network (`edges`, `nodes`, `edge_graph`).
- `walking`: Tables for the walking network.
- `cycling`: Tables for the cycling network.

### Example Query

To query the walking network for edges with a specific H3 cell:

```sql
SELECT * FROM walking.edges WHERE source_h3 = '8f1234567890abc';
```

## Mode-Specific Logic

### Driving
- **Filtering**: Excludes non-routable paths like `footway`, `cycleway`, `path`.
- **Restrictions**: Supports extraction of `restriction` relations (No Left Turn, etc.).
- **Speeds**: Converts `maxspeed` strings (including MPH) to km/h.

### Walking
- **Filtering**: Includes all paths allow foot traffic. It also includes roads that have explicit `sidewalk` or `foot` tags.
- **Speeds**: Uses a constant walking speed of 5 km/h. Turn restrictions are ignored.

### Cycling
- **Filtering**: Includes cycleways and any roads where bicycles are designated or permitted.
- **Speeds**: Uses a constant cycling speed of 15 km/h.

## Troubleshooting Schema Visibility

If you open the resulting DuckDB file and don't see any tables, it's likely because your client is looking at the (empty) `main` schema by default.

### 1. In DuckDB CLI
Use the `SHOW ALL TABLES` command to see tables across all schemas:
```sql
SHOW ALL TABLES;
```

Or switch to a specific mode:
```sql
USE walking;
SHOW TABLES;
```

### 2. In DBeaver / GUIs
Ensure you refresh the connection and look under the "Schemas" node in the database explorer. You should see `driving`, `walking`, `cycling`, and `raw` in addition to `main`.

### 3. Verification Query
Run this query to list all processed tables:
```sql
SELECT table_schema, table_name 
FROM information_schema.tables 
WHERE table_schema NOT IN ('information_schema', 'pg_catalog', 'main')
ORDER BY table_schema;
```
