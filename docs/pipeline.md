# Pipeline

duckOSM builds a per-area routing network as a single `.duckdb` file with one schema per
mode (`driving` / `walking` / `cycling`), each holding `nodes`, `edges`, `edge_graph`
(and `turn_restrictions` for driving). There are **two source modes**.

## Source modes

### `source.type: pbf` — build from OSM
```
[fetch boundary] → [osmium clip] → load PBF → RoadFilter → GraphBuilder →
GraphSimplifier → speeds → costs → restrictions → EdgeGraphBuilder →
ComponentFilter → H3 → validate → report → viz
```

- **All roads in one graph**: `highway='service'` (driveways/alleys) stays in `edges` and is
  built into `edge_graph` like any other road — there is no separate `service_edges` table.
- **osmium clip** (new): when a boundary is set, the PBF is pre-clipped in-pipeline with
  `osmium extract --polygon` (cached under `pbf/<name>.osm.pbf`). This makes `boundary`
  actually constrain the graph — previously `boundary_path` only loaded a metadata table.
- **RoadFilter**: class-trusting, access-aware (see `road_filter.py`). Road classes
  (incl. `*_link` ramps, `service`, `living_street`) are kept; `highway=pedestrian` is
  rescued only with `motorcar/motor_vehicle ∈ {yes,designated,permissive}` or
  `access ∈ {delivery,destination,agricultural}` (reclassed `living_street`).

### `source.type: duckdb` — clip an area from a parent build
```
[fetch boundary] → DuckdbClipper(parent, boundary) → ComponentFilter → validate → report → viz
```
Instead of building from OSM, copy the rows of an existing parent build (e.g.
`sweden.duckdb`) that fall inside the boundary, for each mode. Because edges are copied
**verbatim** (same `osm_id` / `source` / `target` / `geometry`), their `edge_id` hash is
unchanged — **the clipped area is a stable view of its parent, with no re-key** for
downstream consumers.

Build the parent once, then clip sub-areas cheaply:
```bash
duckosm build --config sweden.yaml        # source.type: pbf     (slow, once)
duckosm build --config sodermalm.yaml     # source.type: duckdb  (fast clip of sweden.duckdb)
```

## ComponentFilter

After the graph is built (or clipped), `ComponentFilter` keeps the largest
weakly-connected component and drops disconnected boundary stubs / fragments (the
`clip.keep_largest_component` / `min_component_edges` knobs). It runs **only when a
boundary is set** — a whole-country build legitimately has separate components (islands).
`connectivity_rescue` and `strongly_connected` are accepted but not yet implemented.

## Validation

When `validation.enabled`, a `Validator` runs per mode after the build and **fails the
build** (when `fail_on_error`) on a broken invariant:
- `single_component` — one dominant weakly-connected component,
- `no_stranded_named` — no named edge outside the largest component,
- `edge_id_stable` — reserved (needs a fixture/parent baseline).

## Report & viz

- `report.enabled` → `reports/<name>_<ts>.{md,html}`: source/boundary provenance,
  per-mode network stats, highway breakdown, validation results.
- `viz.enabled` → a [roadstyle](https://github.com/) network map
  `reports/<name>_<mode>_network.html` (needs `geopandas` + `roadstyle`).

## Predicate (clip)

`clip.predicate` selects edges vs the boundary:
- `within` — whole edge inside (no stubs, but cuts boundary-crossing roads short),
- `intersects` — edge touches the boundary (keeps whole crossing roads; pair with
  `keep_largest_component` to drop the outside fragments) — **the recommended default**,
- `centroid` — edge midpoint inside.
