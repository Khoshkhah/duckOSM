# Configuration

A build reads its settings from a YAML file. `duckosm init-config` writes a commented template;
edit it, then build:

```bash
duckosm init-config my_area.yaml
duckosm build -c my_area.yaml
```

- A key you leave out takes the **default** in the tables below. An unknown key anywhere (at the top
  level, a section name, or inside a section) is ignored with a warning. That covers a typo and a
  key that no longer exists.
- Relative paths resolve against the folder you run `duckosm` in, not the config file's folder.
- With `-c`, a `build` option you type overrides the file (e.g. `-c monaco.yaml -m walking
  --no-features`); options you don't type leave the file's values. The **Flag** column shows the
  option for each field ([Command line](cli.md#build)).
- The template sets a few keys to a value other than the default; the **Template** column shows them.
  It turns on `validation` and `report`, and gives example values for `name`, `output_path` and
  `source.pbf_path`.

In a clone of the repo, `config/sample_monaco.yaml` is a short working example.

## Top level

| Key | Default | Template | Flag | Does |
|---|---|---|---|---|
| `name` | `default` | `my_import` | stem of `-o`, else of `-b`, else of `-p` or `--source-db` | the area's name: the output file is `<name>.duckdb` when `output_path` is a folder; also names the cached PBF cut (below) and the report files |
| `output_path` | `output.duckdb` | `.` | `-o` | a path ending in `.duckdb` is the output file; anything else is a folder, and the file is `<output_path>/<name>.duckdb` |
| `modes` | `[driving]` | `[driving]` | `-m` | the networks to build: any of `driving`, `walking`, `cycling`, one schema each |
| `osm_overrides` | none | none | `--fixes` | a rules file of [fixes for OSM errors](../guides/fix-osm-errors.md). Used only when named |
| `pbf_path` | none | | | short for `source.pbf_path` |
| `boundary_path` | none | | | short for `boundary.path` |
| `h3_cell` | none | | `--h3-cell` | short for `boundary.h3_cell` |

With a boundary, the PBF is first cut to it and the cut is kept as `pbf/<name>.<strategy>.<key>.osm.pbf`,
where `<key>` is a fingerprint of the boundary and the PBF. The next build reuses it while both are
unchanged; after a change it cuts again and removes the old cut of the same name and strategy.

## `source`

Where the network comes from. [How a build works](../concepts/build.md) describes both kinds.

| Key | Default | Flag | Does |
|---|---|---|---|
| `source.type` | `pbf` | `--source-db` sets `duckdb` | `pbf`: build from OSM data. `duckdb`: cut an area out of a built database, keeping its edge ids |
| `source.pbf_path` | none | `-p` | the `.osm.pbf` file; required for `pbf` |
| `source.source_db` | none | `--source-db` | the built database to cut from; required for `duckdb`, together with a boundary (`path`, `place`, `bbox` or `h3_cell`) |

## `boundary`

The area to build.

| Key | Default | Flag | Does |
|---|---|---|---|
| `boundary.path` | none | `-b` | a GeoJSON polygon. With none of the four boundary keys, the whole PBF is built and the component filter doesn't run |
| `boundary.buffer_m` | `0` | | grow the boundary outwards by this many metres first (measured in the local UTM zone) |
| `boundary.place` | none | | a place name, e.g. `Monaco`: looked up in the PBF's own borders, else on Nominatim, as [`duckosm boundary`](cli.md#boundary) does |
| `boundary.bbox` | none | | a box, `[west, south, east, north]` in degrees |
| `boundary.h3_cell` | none | `--h3-cell` | an H3 cell id: its outline is the area |

Give one of `path`, `place`, `bbox`, `h3_cell`; `path` wins. The other three are written to
`<name>.boundary.geojson` next to the output, and the build uses that file.

To make a boundary file: [Prepare an area](../guides/prepare-area.md).

## `clip`

The clean-up after the area is cut. Details: [Network clean-up](../concepts/cleanup.md).

| Key | Default | Does |
|---|---|---|
| `clip.predicate` | `intersects` | `duckdb` source only: which edges are copied. `intersects`: every edge that touches the area; `within`: only edges completely inside; `centroid`: edges whose middle point is inside. A `pbf` build always keeps whole ways |
| `clip.keep_largest_component` | `true` | keep only the largest connected piece of each network. Runs only with a boundary and, for a `pbf` build, with `options.build_graph` |
| `clip.min_component_edges` | `1` | with `keep_largest_component: false`, keep every piece with at least this many edges |
| `clip.connectivity_rescue` | `true` | walking and cycling, `pbf` build: [join dangling path ends](../concepts/cleanup.md#connect-dangling-paths) to the nearest node. Runs with or without a boundary |
| `clip.connect_snap_m` | `10.0` | the largest gap, in metres, that `connectivity_rescue` joins |

## `options`

Build options. They apply to a `pbf` build; a `duckdb` build uses only `memory_limit`, `threads` and
`build_graph` (for the indexes).

| Key | Default | Flag | Does |
|---|---|---|---|
| `options.build_graph` | `true` | `--graph` / `--no-graph` | build `edge_graph`, the graph of legal turns. The component filter needs it |
| `options.h3_indexing` | `true` | `--h3-index` / `--no-h3-index` | add `nodes.h3_cell` and `edges.from_cell`, `to_cell`, `lca_res` |
| `options.h3_resolution` | `8` | `--h3-resolution` | H3 resolution, 0–15, for those columns and for `boundary_cells` |
| `options.merge_segments` | `true` | | join a chain of edges that is one road into one edge; writes `<mode>.edge_id_map`. [Merged edges](../concepts/edge-ids.md#merged-edges) |
| `options.global_junctions` | `true` | | split every mode's roads at the same junctions, so a road has the same `edge_id` in every mode. [The same id in every mode](../concepts/edge-ids.md#the-same-id-in-every-mode) |
| `options.functional_types` | `true` | | add `walk_type` (walking) and `cycle_type` (cycling). [`walk_type` and `cycle_type`](../concepts/networks.md#walk_type-and-cycle_type) |
| `options.cycling_dismount` | `true` | | add footways and pedestrian streets to cycling as [dismount edges](../concepts/networks.md#dismount-edges) |
| `options.process_speeds` | `true` | | add `maxspeed_kmh` to edges. [Speeds and travel time](../concepts/networks.md#speeds-and-travel-time) |
| `options.calculate_costs` | `true` | | add `cost_s`, the travel time in seconds. Routing and `multimodal` need it |
| `options.extract_restrictions` | `true` | | driving: build `turn_restrictions` and remove the forbidden turns from `edge_graph`. [Turn restrictions](../concepts/networks.md#turn-restrictions) |
| `options.boundary_cells` | `false` | | write `main.boundary_cells`, the H3 cells that cover the boundary. Needs a boundary |
| `options.boundary_cell_resolutions` | none | | resolutions for `boundary_cells`, e.g. `[6, 7, 8]`; none means `[h3_resolution]` |
| `options.build_features` | `true` | | build the [base-map layers](features.md), `features.*` (`build --no-features` turns it off). Needs `raw.*`, so a `duckdb` build skips it |
| `options.clip_strategy` | none | | how `osmium` cuts the PBF: `complete_ways`, `smart` or `simple` ([`clip-pbf`](cli.md#clip-pbf)). None means `smart` with `build_features`, else `complete_ways` |
| `options.memory_limit` | none | | DuckDB memory limit, e.g. `"16GB"`; none is DuckDB's default |
| `options.threads` | none | | number of DuckDB threads; none is DuckDB's default |
| `options.simplify_batches` | `0` | | split the simplify step into this many batches to save memory; `0` sizes them from the node count, `1` is one pass |

## `validation`

Checks each mode at the end of the build. What each check means: [Check a build](../guides/check-build.md).

| Key | Default | Template | Does |
|---|---|---|---|
| `validation.enabled` | `false` | `true` | run the checks |
| `validation.fail_on_error` | `true` | `true` | stop the build when a check fails; `false` only warns |
| `validation.assert_single_component` | `true` | `true` | one connected network. Skipped when no component clean-up runs: no boundary, no edge graph, or `clip.keep_largest_component: false` with `clip.min_component_edges: 1` |
| `validation.assert_no_stranded_named` | `true` | `true` | no named road outside the main network. Skipped when no component clean-up runs, as above |
| `validation.assert_unique_node_id` | `true` | | no two nodes with the same `node_id` |
| `validation.assert_way_length_conserved` | `true` | | no part of an OSM way lost when it was split into edges. Skipped in a `duckdb` build |
| `validation.warn_layer_without_structure` | `true` | `true` | warn (never fail) about edges with a `layer` but no `bridge` or `tunnel` tag |

## `multimodal`

Builds `mm.edges` and `mm.transfers` at the end of the build, the same as
[`duckosm multimodal`](cli.md#multimodal). Needs walking and at least one other mode.
[Routing across modes](../concepts/multimodal.md).

| Key | Default | Does |
|---|---|---|
| `multimodal.enabled` | `false` | build the `mm` tables |
| `multimodal.transfer_s` | `60.0` | seconds added at each change of mode |
| `multimodal.transfer_costs` | none | seconds per direction, e.g. `{"walking->driving": 60, "driving->walking": 30}`; a pair not listed uses `transfer_s` |

## `report` and `viz`

| Key | Default | Template | Does |
|---|---|---|---|
| `report.enabled` | `false` | `true` | write `reports/<name>_<time>.md` and `.html`: counts per mode, what the road filter kept, network pieces, validation results |
| `viz.enabled` | `false` | `false` | write a map of each mode, `reports/<name>_<mode>_network.html`, like [`duckosm viz`](cli.md#viz). Needs `duckosm[viz]` |
| `viz.basemap` | `voyager` | | first base map, a roadstyle name: `voyager`, `positron`, `dark_matter`, `osm`, `satellite`, `blank`, … |

A failed report or map logs a warning; the build still succeeds.

The values of `modes`, `options.clip_strategy` and `clip.predicate` are not checked when the file is
loaded. An unknown `clip.predicate` is treated as `intersects`.
