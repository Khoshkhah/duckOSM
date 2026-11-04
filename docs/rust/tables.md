# The tables that duckOSM makes and duckosm-rust does not make yet

Status: group 1 done on 2026-10-22 (the result is at the end). Kaveh: "there are many tables is missed yet. we
should first try to create them".

The list comes from duckOSM's Monaco database (`/home/kaveh/projects/duckOSM/monaco.duckdb`, a build with the
default options) and from duckOSM's reference `docs/reference/database.md`. duckosm-rust makes today, for each
mode: `edges`, `nodes`, `private_edges`, `edge_graph`, `edge_id_map`, `turn_permission`, and for driving
`turn_restrictions` and `turn_path_restrictions`.

## The missing tables

| Group | Table | What it holds (duckOSM) | Monaco rows | Work |
|---|---|---|---|---|
| 1 | `<mode>.ways` | The OSM ways that the mode kept, after the road filter and the fixes. | 1,196 / 3,188 / 2,986 | Small: the build has these ways. |
| 1 | `<mode>.way_nodes` | The nodes of `ways`, one row per node, in order. | 8,407 / 16,841 / 17,135 | Small. |
| 1 | `<mode>.virtual_nodes` | Always empty in duckOSM (the virtual nodes are in `nodes`). | 0 | Very small: an empty table with its columns. |
| 1 | `main.boundary` | The boundary file: one row per feature, its properties, and `geom`. | 1 | Small: the page and the CLI have the boundary. |
| 1 | `main.global_junctions` | The nodes where every mode cuts its ways. | 4,318 | Small: the build computes them. |
| 3 | `main.building_info` | One row for each building of `features.buildings`: floors (`levels`), `roof_levels`, `height_m`, `roof_shape`, `roof_color` and `facade_color` (as `#rrggbb`), `material` and `roof_material`, read from its tags. Not in duckOSM: the page makes it (`app/duck.js`) when the layers are on. A value that does not parse is NULL. | as many as `features.buildings` | Small. The text of the tags stays in `features.buildings.tags`. |
| 3 | `main.building_entrances` | One row for each door of the area: a node with an `entrance` or a `door` tag (`kind`: the `entrance` value, or `door` when only `door` is set; `door`, `access`, `housenumber`, `street`, `level`, `wheelchair`, `name`), `building_id` (the building way that has the node in its outline, the smallest id when two share it; NULL when none) and the point `geom`. Not in duckOSM: the page makes it (`app/duck.js`) from `raw.nodes` and `raw.ways` when the layers are on. Monaco sample: 110 doors, 83 on a building outline. | as many as the nodes with `entrance` or `door` | Small. |
| 3 | `main.building_faces` | One row for each wall (a segment of the outer ring) of a building of the raw cut: `building_id`, `face_index`, `from_node`, `to_node`, `face_type` (`front`, `back`, `side`, `party`), `computed_type`, `edited`, `type_reason` (why the wall has its type), `building_type` (`way` or `relation`), `door` (the kind of door of the wall from a rule, such as `main`; NULL for no door), `length_m`, `azimuth_deg`, the street that the wall faces (`street_osm_id`, `street_edge_id`, `street_name`, `distance_m`) and the wall as a LineString. Not in duckOSM: made with the layers in the page, and with `--building-faces --boundary` in the CLI. Method: [osm-editing.md](osm-editing.md). | 10,909 on Monaco (1,262 buildings: 1,231 ways and 31 relations) | Small: 0.2 s on Monaco. The same file on each run. |
| 1 | `main.visualization_metadata` | One row: the boundary, the center, the zoom, the time zone, the build time and version. | 1 | Small, except the time zone (see below). |
| 1 | `mm.edges` (a view) and `mm.transfers` | Routing across modes: all edges with their mode, and the changes of mode at the nodes that walking shares with another mode. | view / 11,620 | Small to medium: SQL on the tables of the build. |
| 2 | `visualization.edge_levels`, `edge_levels_meta` | The drawing order of the roads (bridges over roads). | 13,298 / 1 | Done: [levels.md](levels.md). The tags rule (the default) and roadstyle's solver (`--levels solve`). |
| 2 | `driving.lane_profile` | Each lane of each driving edge and bus-only edge, left to right. | 3,176 | Done: [lane_profile.md](lane_profile.md). Equal to duckOSM's on Monaco and Tartu. |
| 2 | `bus.routes`, `bus.route_edges` | The bus routes (OSM relations) as the driving edges they travel, in order. | 56 / 5,609 | Medium: the relations, and a walk along the edges (as the turn restrictions through a way). |
| 3 | `raw.nodes`, `raw.ways`, `raw.relations` | The OSM data of the area as read from the PBF. | 34,326 / 5,715 / 336 | In the session "Base map features" (its approach A writes `raw.*`). |
| 3 | `features.*` (12 tables) | The base map: buildings, land, water, POIs, place labels, … | 1 to 3,496 each | In the session "Base map features" (`docs/features.md`). |

Not in this list: `visualization.level_edits`, `level_heads`, `level_caps` and `level_unmatched` (the manual
edits of the drawing levels in duckOSM's level editor), `main.boundary_cells` and the H3 columns (duckOSM option
H3), `main.elevation_metadata` (elevation), `main.admin_boundaries` (the boundaries for `duckosm boundary`).
They come from separate duckOSM tools or options.

## Decisions (Kaveh, 2026-10-22)

- **The time zone** (`visualization_metadata.timezone`): one value for each area, the zone at the node nearest
  the middle of the area's nodes (duckOSM's rule). Natively, the CLI and the Python package find it with the world
  map of the Rust crate `tzf-rs` (MIT; its data is ODbL), as duckOSM does with Python's `timezonefinder`. In the
  browser, that map would make the module 2.7 MB larger (with gzip). So the page has one table file
  (`www/timezones.json`, 447 KB with gzip, loaded at the first build, then cached; Kaveh's choice, 2026-10-23):
  for each Geofabrik region, its zone name, and the borders of the zones only where a region really has more
  than one zone (`tools/make_timezone_table.py`). The page gives the build the entry of the area's smallest
  region (`set_timezone_entry`). Two other ways are in [todo.md](todo.md).
- **The drawing levels**: the tags rule is the default, and a Rust port of roadstyle's solver (the real
  optimization, as in Python duckOSM) is an option. Done: [levels.md](levels.md). `edge_levels_meta.source` says
  which one made the table.
- **The check**: each new table is compared with duckOSM's table, row for row, on Monaco and Tartu, as the
  network tables are.

## The plan

1. Group 1, in Rust (`duckosm-core`, `duckosm-build`), so the CLI, the Python package and the page get the same
   tables. One test against duckOSM's Monaco golden for each table. The page adds them to SQL, the zip and
   `load.sql`.
2. Group 2: `lane_profile` and `bus`. The levels table ([levels.md](levels.md)) and the lane profile
   ([lane_profile.md](lane_profile.md)) are done.
3. Group 3 stays in the session "Base map features". Its `raw.*` tables are also the input that group 2 can use
   for the bus relations.

## Result of group 1 (2026-10-22)

Done, natively (CLI, Python package) and in the browser, with the same code (`duckosm-build`, `area.rs`):

| Table | File | Monaco, against duckOSM's `monaco.duckdb` |
|---|---|---|
| `<mode>.ways` | `ways_<mode>.parquet` | equal: 1,196 / 3,188 / 2,986 rows |
| `<mode>.way_nodes` | `way_nodes_<mode>.parquet` | equal: 8,407 / 16,841 / 17,135 rows |
| `<mode>.virtual_nodes` | `virtual_nodes_<mode>.parquet` | equal: empty |
| `main.global_junctions` | `global_junctions_main.parquet` | equal: 4,318 rows |
| `main.boundary` | `boundary_main.parquet` | equal: the properties and the shape |
| `main.visualization_metadata` | `visualization_metadata_main.parquet` | equal: the center, the zoom, the shape, `Europe/Monaco` |
| `mm.transfers` | `transfers_mm.parquet` | equal: 11,620 rows |
| `mm.edges` (a view) | in the `.duckdb` file and in `load.sql` | 4 of 25,641 rows differ: the known connector labels of `walking.edges` (item 3c of design.md) |

What it needed:
- The reader keeps the other OSM tags of each way too (`OtherTags`, one string for each way), because
  `ways.tags` has all tags. The fixes set them (`Way::set_tag`), and the sidewalk rule adds
  `duckosm:sidewalk=inferred`, as duckOSM.
- duckOSM writes the lane arrows of the fixes (`turn_lanes_forward` / `_backward`) into the raw tags after each
  mode, so the `ways` of the later modes show them and driving does not. The build does the same
  (`OsmFixes::set_arrows`).
- `built_at` is the build time (`Params::built_at` can fix it, for a build that is the same each time);
  `duckosm_version` is `duckosm-rust <version>`.
- `--no-ways-tables` (Python `ways_tables=False`) leaves out `ways` and `way_nodes` in a large area.

Checks:
- `python tools/compare_tables.py monaco.duckdb <out>/monaco.duckdb` after
  `duckosm-rust --config config/sample_monaco.yaml --allow-skipped -o <out>` in the duckOSM folder.
- `cargo test -p duckosm-wasm`: the browser build writes the same 33 files as the native build, byte for byte
  (Monaco, an area and a fixes file).
- The page (`e2e.py`): the same files as the native CLI, on every row (`built_at` apart). The zip has them all.
- Tartu from the Estonia file, natively: 1.11 s with these tables, 0.99 s without; the same memory.
- The time zone: first with the world map of `tzf-rs` in the WebAssembly module too (9.6 MB, 3.7 MB with gzip).
  Then, by Kaveh's choice, the page's table: the module is 5.9 MB (1.19 MB with gzip), and the page loads
  `timezones.json` (445 KB with gzip) at the first build. The table's check: 4,022 of 4,038 random land points
  (99.6 %) in the regions with more zones give `timezonefinder`'s zone; the others are within a few km of a border.
  The browser build with the table's Monaco entry writes the same files as the native build with `tzf-rs`, byte for
  byte (`cargo test -p duckosm-wasm`). The page: `Europe/Monaco` for Monaco, `Europe/Tallinn` for Tartu.

