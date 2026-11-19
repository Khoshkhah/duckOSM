# Configuration

A build can read its settings from a YAML file. `duckosm init-config` writes a template with comments:

```bash
duckosm init-config my_area.yaml
duckosm --config my_area.yaml
```

| Rule | |
|---|---|
| A key that you leave out | takes the default of the tables below |
| An unknown key | is ignored, with a warning |
| A relative path | is read from the folder where you run `duckosm` |
| An option on the command line | wins over the file, for example `--config my_area.yaml -m walking` |
| A value outside its choices | stops the build before it starts |

## Area and output

| Key | Default | Option | Does |
|---|---|---|---|
| `name` | `default` | `--name` | the name of the area: the output file is `<name>.duckdb` |
| `output_path` | `output.duckdb` | `-o` | a `.duckdb` file, or a folder for `<name>.duckdb` |
| `modes` | `[driving]` | `-m` | `driving`, `walking`, `cycling` |
| `osm_overrides` | none | `--fixes` | your [OSM fixes file](../guides/edit-osm-data.md#the-fixes-file) |
| `source.pbf_path` (or `pbf_path`) | none | `PBF` | the `.osm.pbf` file |
| `source.type` | `pbf` | | `pbf`, or `duckdb` to cut an area out of the built database `source.source_db`, with its edge ids |

## `boundary`

Give one of these. With none, the whole file is built. [The boundary file](boundary.md).

| Key | Does |
|---|---|
| `boundary.path` (or `boundary_path`) | a GeoJSON file |
| `boundary.place` | a place name, for example `Monaco`, as [`duckosm boundary`](cli.md#boundary) finds it |
| `boundary.bbox` | a box, `[west, south, east, north]` in degrees |
| `boundary.h3_cell` (or `h3_cell`) | an H3 cell |
| `boundary.buffer_m` | grows the boundary by this many metres first (default `0`) |

## `options`

Each step is on by default, except where the table says off.

| Key | Default | Does |
|---|---|---|
| `options.build_graph` | on | the turn graph `edge_graph` |
| `options.extract_restrictions` | on | driving: the turn restrictions |
| `options.merge_segments` | on | joins a chain of edges that is one road ([merged edges](../concepts/edge-ids.md#merged-edges)) |
| `options.global_junctions` | on | cuts each mode at the same junctions, so a road has the same `edge_id` in each mode |
| `options.process_speeds`, `options.calculate_costs` | on | `maxspeed_kmh` and `cost_s` |
| `options.functional_types` | on | `walk_type` and `cycle_type` |
| `options.cycling_dismount` | on | footways in cycling, as [dismount edges](../concepts/networks.md#dismount-edges) |
| `options.infer_separate_sidewalks` | on | walking uses the sidewalk lines of a road, not the road |
| `options.h3_indexing`, `options.h3_resolution` | on, `8` | the H3 columns |
| `options.tile_indexing` | on | the zoom 28 [tile keys](database.md#tile-keys) |
| `options.build_features` | on | the [base-map layers](features.md) |
| `options.sea` | `overture` | `features.ocean` from Overture Maps; `false` for none |
| `options.levels` | on | the [drawing order](../guides/drawing-order.md) of the roads, with your edits kept |
| `options.building_faces` | off | the [walls of the buildings](../concepts/building-walls.md) |
| `options.bus_routes` | on | the [bus routes](../concepts/networks.md#bus-routes) on the driving edges |
| `options.boundary_cells`, `options.boundary_cell_resolutions` | off | the H3 cells that cover the boundary |
| `options.clip_strategy` | `smart` with the base-map layers, else `complete_ways` | how the file is cut to the boundary: `complete_ways`, `smart` or `simple` |
| `options.memory_limit`, `options.threads` | DuckDB's own | the memory and the threads of DuckDB |

## `clip`

| Key | Default | Does |
|---|---|---|
| `clip.connectivity_rescue` | on | walking and cycling: [connects dangling paths](../concepts/cleanup.md#connect-dangling-paths) |
| `clip.connect_snap_m` | `10.0` | the largest gap that it connects, in metres |
| `clip.predicate` | `intersects` | `source.type: duckdb` only: the edges that are copied: `intersects`, `within` or `centroid` |

## `elevation`

The DEM of the area: its [ground reference](../concepts/ground-reference.md). Give it one time for each area here.
`--dem` on the command line wins over this section.

```yaml
elevation:
  dem: data/dem/fabdem-monaco.tif     # a GeoTIFF in EPSG:4326, 32-bit floats
  product: DTM                        # DTM (bare earth) or DSM (surface)
  vertical_datum: EGM2008
  license: CC-BY-NC-SA-4.0
  reference: true                     # false: a test or a second surface; then the 3 keys above can be unknown
```

## `validation`, `multimodal`, `report`, `viz` {#validation}

| Key | Default | Does |
|---|---|---|
| `validation.enabled` | off | [checks the build](../guides/check-build.md) and stops on a fault (`validation.fail_on_error`) |
| `multimodal.enabled` | off | the [`mm` tables](../concepts/multimodal.md), for trips across modes. `multimodal.transfer_s` (`60`) is the cost of each change of mode |
| `report.enabled` | off | a report of the build, in `reports/` |
| `viz.enabled` | off | the maps of the build, as [`duckosm viz`](cli.md#viz) |

More keys: `validation.assert_*` (each check, [Check a build](../guides/check-build.md)), `multimodal.transfer_costs`
(a cost for each direction, for example `{"walking->driving": 60}`), `viz.basemap` (a web background under the maps) and
`options.simplify_batches` (a memory limit for one step). The template of `duckosm init-config` explains each one.
