# Configuration

duckOSM is driven by a YAML config file (or equivalent CLI flags). A ready-to-edit
template is at [`config/template.yaml`](../config/template.yaml).

```bash
python -m duckosm --config config/my_import.yaml
```

CLI flags override the corresponding config values (see `python -m duckosm --help`).

## Top-level keys

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | `"default"` | Logical name; used as the output filename when `output_path` is a directory. |
| `pbf_path` | string | — (**required**) | Source OSM PBF file. |
| `output_path` | string | `"output.duckdb"` | If it ends in `.duckdb`, used as the file path; otherwise treated as a **directory** and the DB is written to `<output_path>/<name>.duckdb`. |
| `boundary_path` | string \| null | `null` | Optional GeoJSON polygon to clip the build to. Also enables `boundary_cells`. |
| `h3_cell` | string \| null | `null` | Optional single H3 cell id to clip to (alternative to `boundary_path`). |
| `modes` | list | `["driving"]` | One output schema per mode: any of `driving`, `walking`, `cycling`. |
| `options` | map | see below | Processing options. |

## `options`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `build_graph` | bool | `true` | Build the edge-adjacency `edge_graph` table for routing. |
| `h3_indexing` | bool | `true` | Add `from_cell` / `to_cell` / `lca_res` H3 columns to `edges`. |
| `h3_resolution` | int | `8` | H3 resolution (0–15) for edge indexing. |
| `simplify` | bool | `false` | Contract degree-2 nodes into single edges with full geometry. Recommended for routing. |
| `process_speeds` | bool | `true` | Compute `maxspeed_kmh` (and drop the raw `maxspeed` string). |
| `extract_restrictions` | bool | `true` | Build the `turn_restrictions` table (driving only). |
| `calculate_costs` | bool | `true` | Compute travel-time `cost_s` per edge. |
| `boundary_cells` | bool | `false` | Write `main.boundary_cells` (H3 grid over the boundary). Needs `boundary_path`. |
| `boundary_cell_resolutions` | list[int] \| null | `null` | Resolutions for `boundary_cells`; `null` = `[h3_resolution]`. |
| `timezone` | bool | `false` | Add an IANA `timezone` column to `visualization_metadata` (from the centroid). Needs the optional `timezonefinder` package (`pip install timezonefinder` or `pip install -e .[tz]`); skipped with a warning if absent. |
| `memory_limit` | string \| null | `null` | Cap DuckDB memory, e.g. `"16GB"`. `null` = DuckDB default (~80% of RAM). |
| `threads` | int \| null | `null` | Cap worker threads. `null` = DuckDB default. |
| `simplify_batches` | int | `0` | Way-id buckets for the simplifier. `0` = auto-size by node count (bounds peak RAM at country scale); `1` = single pass. |

## Notes

- **Spatial filter**: use at most one of `boundary_path` or `h3_cell`. With no filter the
  whole PBF is processed.
- **Large / country-scale builds**: the simplifier auto-batches (`simplify_batches: 0`),
  so even big extracts fit in RAM; set `memory_limit` to leave headroom for the spatial
  extension on memory-constrained machines.
- **`boundary_cells`** only runs when a `boundary_path` is supplied.

## Minimal example

```yaml
name: tartu
pbf_path: data/maps/estonia-latest.osm.pbf
output_path: data/output
boundary_path: data/boundaries/tartu.geojson
modes: [driving, walking, cycling]
options:
  simplify: true
```

See [`config/template.yaml`](../config/template.yaml) for the fully annotated version
and the existing files in [`config/`](../config) for more examples.
