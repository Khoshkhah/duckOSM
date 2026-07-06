# Configuration

duckOSM is driven by a YAML config file (or equivalent CLI flags). A ready-to-edit
template is at [`config/template.yaml`](../config/template.yaml).

```bash
duckosm build --config config/my_import.yaml
```

CLI flags override the corresponding config values (see `duckosm build --help`).

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
| `osm_overrides` | string \| null | `"osm_overrides/osm_overrides.yaml"` | Path to a global rules file of local corrections for known OSM errors, applied per mode between road-filtering and edge-building. Silent no-op if the file is absent. See [`osm_overrides`](#osm_overrides--local-corrections-for-known-osm-errors). |

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

## `source` — where the network comes from

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `source.type` | `pbf` \| `duckdb` | `pbf` | Build from OSM, or clip an existing duckOSM db. |
| `source.pbf_path` | string | — | (pbf) source PBF. Also accepted as the flat top-level `pbf_path`. |
| `source.country` / `country_url` | string \| null | `null` | (pbf) reserved — Geofabrik auto-download. |
| `source.source_db` | string | — | (duckdb) parent db to clip from, e.g. `data/db/sweden.duckdb`. |
| `source.source_modes` | list \| null | `null` | (duckdb) schemas to clip; `null` = same as `modes`. |
| `source.preserve_edge_ids` | bool | `true` | (duckdb) keep the parent's `edge_id`s (recommended). |

## `boundary` — the clip region

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `boundary.path` | string \| null | `null` | GeoJSON polygon. Also accepted as the flat `boundary_path`. |
| `boundary.place` / `bbox` / `h3_cell` | — | `null` | Alternative region specs (`place`/`bbox` reserved). |
| `boundary.buffer_m` | float | `0` | Outward margin (metres) added to the boundary before clipping — buffered in an auto-selected UTM zone (from the boundary centroid), so the clip region grows uniformly on the ground. Use it to keep roadside sensors just outside an admin boundary on the network. `0` = clip to the exact boundary. |

A boundary now **actually clips the graph** (PBF mode pre-clips with `osmium`; duckdb mode
spatially selects). Set at most one of `path` / `place` / `bbox` / `h3_cell`.

## `clip` — clean-up (runs when a boundary is set)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `clip.predicate` | `within` \| `intersects` \| `centroid` | `intersects` | Edge-vs-boundary test. |
| `clip.keep_largest_component` | bool | `true` | Drop disconnected boundary stubs; keep the largest component. |
| `clip.min_component_edges` | int | `1` | Drop weak components smaller than this. |
| `clip.connectivity_rescue` | bool | `true` | Re-admit a connector iff it reconnects a component (reserved). |
| `clip.strongly_connected` | bool | `false` | Require routable round-trip (reserved). |

## `osm_overrides` — local corrections for known OSM errors

Some OSM source data is wrong or ambiguous — a missing `oneway` tag, an undercounted `lanes` value.
When fixing it upstream in OpenStreetMap isn't practical, duckOSM can patch the affected ways locally
from a single global rules file so every rebuild reproduces the correction. The catalogue of issues
these rules address lives in [`known_osm_issues.md`](../osm_overrides/known_osm_issues.md).

The top-level `osm_overrides` key points at the file (default `osm_overrides/osm_overrides.yaml`); set it to
`null` to disable. The rules are applied to the `ways` table **after `RoadFilter` and before
`GraphBuilder`, once per mode** — placement matters: `oneway` is *topological* (it decides whether a
reverse-twin edge is created), so it cannot be patched on the finished DB.

Each rule is keyed by OSM `osm_id` and is a **silent no-op in any area that doesn't contain that way**,
so one global file is safe to apply to every build.

```yaml
# osm_overrides/osm_overrides.yaml
overrides:
  - osm_id: 4392632          # Hökens Gata, Södermalm — missing oneway (known_osm_issues #1)
    oneway: true
    note: "adjacent segment 676781760 is oneway=yes"
  - osm_id: 507979055        # Trans-Canada WB, Vancouver — lane undercount (known_osm_issues #3)
    lanes: 3
    note: "OSM lanes=2 but BC MoTI measured 3 WB lanes"
```

| Field | Type | Description |
|-------|------|-------------|
| `osm_id` | int | **required** — the OSM way id to patch. |
| `oneway` | bool | Force one-way (`true`) or two-way (`false`). Changes topology, so the affected edge(s) can be re-simplified and get a **new `edge_id`**. |
| `lanes` | int | Per-direction lane count; sets **both** directions. Attribute-only — `edge_id` stays stable. |
| `lanes_forward` / `lanes_backward` | int | Per-direction lane counts for asymmetric roads. |
| `note` | string | Free-text provenance (shown in the build log). |

Only enable a rule once the correction is **verified** — an unverified `oneway` flip silently drops a
direction of travel. Keep unverified issues commented out. Existing `.duckdb` builds must be **rebuilt**
to pick up any change. The log reports how many rules matched (`applied N/M OSM override(s)`).

## `validation` / `report` / `viz`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `validation.enabled` | bool | `false` | Run invariant checks per mode after the build. |
| `validation.fail_on_error` | bool | `true` | Abort the build on a failed check. |
| `validation.assert_single_component` | bool | `true` | One dominant weakly-connected component. |
| `validation.assert_no_stranded_named` | bool | `true` | No named edge outside the largest component. |
| `report.enabled` | bool | `false` | Write `reports/<name>_<ts>.{md,html}`. |
| `viz.enabled` | bool | `false` | Write a roadstyle network map (needs `geopandas` + `roadstyle`). |
| `viz.basemap` | string | `voyager` | `voyager` \| `positron` \| `esri_gray` \| `osm`. |

See [`docs/pipeline.md`](pipeline.md) for the full stage order and the two source modes.

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
