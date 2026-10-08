# Database schema

Every table and column a build writes. Types are as DuckDB reports them. Geometries are
longitude / latitude (EPSG:4326). How to query them: [Query the database](../guides/query.md).

| Schema | Holds | Written by |
|---|---|---|
| [`<mode>`](#mode-schema) | one network: `driving`, `walking` or `cycling` | `build` |
| [`main`](#main-schema) | the area: boundary, map centre, time zone, …; the `edge_id` macros | `build`, `elevation`, `admin` |
| [`raw`](#raw-schema) | the OSM data as read from the PBF | `build` from a PBF |
| [`mm`](#mm-schema) | routing across modes | `multimodal`, or `multimodal.enabled` |
| [`features`](#features-schema) | base-map layers | `options.build_features` |
| [`visualization`](#visualization-schema) | the drawing order of the roads | `levels` |
| [`bus`](#bus-schema) | the bus routes on the driving edges | `options.bus_routes`, or `bus` |

## `<mode>` schema {#mode-schema}

| Table | Present when |
|---|---|
| [`edges`](#edges) | always |
| [`private_edges`](#private_edges) | a new build; in a clip or extract, only if the parent has it |
| [`nodes`](#nodes) | always |
| [`edge_graph`](#edge_graph) | `options.build_graph` (on); in a clip or extract, only if the parent has it |
| [`turn_restrictions`](#turn_restrictions) | driving, with `options.extract_restrictions` (on); in a clip or extract, only if the parent has it |
| [`ways`](#ways), [`way_nodes`](#way_nodes) | a build from a PBF, and `duckosm extract` |
| [`edge_id_map`](#edge_id_map) | a build from a PBF with `options.merge_segments` (on) |
| [`virtual_nodes`](#virtual_nodes) | a build from a PBF; always empty |

A build cut from a parent database (`source.type: duckdb`) has only `edges`, `private_edges`,
`nodes`, `edge_graph` and `turn_restrictions`, and each of the last three only if the parent has it.

### `edges`

One row per direction of a road: a two-way road has two edges. What each mode keeps:
[What each network contains](../concepts/networks.md).

| Column | Type | Description |
|---|---|---|
| `edge_id` | BIGINT | the edge's id: `(hash(osm_id, source, target) >> 1)::BIGINT`. The same on every rebuild and in every mode. [Stable edge ids](../concepts/edge-ids.md) |
| `edge_ref` | VARCHAR | a readable id, `{osm_id}#{seq}` then `f` or `r` for the direction (e.g. `41790239#2f`); `seq` counts the way's edges from its start. Not stable across rebuilds: join on `edge_id` |
| `source` | BIGINT | the node the edge starts at |
| `target` | BIGINT | the node the edge ends at |
| `osm_id` | BIGINT | the OSM way. A merged edge has the way at its `source` end. Negative for a [connector edge](../concepts/cleanup.md#connect-dangling-paths) |
| `highway` | VARCHAR | the OSM `highway` value |
| `name` | VARCHAR | the OSM `name` |
| `oneway` | BOOLEAN | TRUE when the edge has no reverse twin of the same way. Never NULL. [One-way roads](../concepts/networks.md#one-way-roads) |
| `lanes` | INTEGER | lanes in this edge's direction. Never NULL, except on connector edges (`osm_id < 0`). [Lanes](../concepts/networks.md#lanes) |
| `surface` | VARCHAR | the OSM `surface` tag |
| `access` | VARCHAR | the mode's access: the value of its most specific access tag (driving: `motorcar`, `motor_vehicle`, `vehicle`, `access`; walking: `foot`, `access`; cycling: `bicycle`, `vehicle`, `access`). NULL when none is tagged. Never `private` or `bus`: those edges are in `private_edges`. [Access](../concepts/networks.md#access-private-and-forbidden-roads) |
| `junction` | VARCHAR | the OSM `junction` tag (`roundabout`, `circular`, …) |
| `layer` | VARCHAR | the OSM `layer` tag; NULL is ground level |
| `bridge` | VARCHAR | the OSM `bridge` tag (`yes`, `viaduct`, …) |
| `tunnel` | VARCHAR | the OSM `tunnel` tag (`yes`, `building_passage`, …) |
| `service` | VARCHAR | the OSM `service` tag of a `highway=service` road (`driveway`, `parking_aisle`, `alley`, …) |
| `refs` | BIGINT[] | the OSM node ids along the edge, from `source` to `target` |
| `geometry` | GEOMETRY | the line, from `source` to `target` |
| `is_reverse` | BOOLEAN | TRUE for the twin that runs against the way's drawing direction. A `oneway=-1` way is turned round first, so its edge is not reverse |
| `length_m` | FLOAT | length in metres |
| `dismount` | BOOLEAN | cycling only: TRUE on a [dismount edge](../concepts/networks.md#dismount-edges), where the bike is pushed |
| `maxspeed_kmh` | FLOAT | speed in km/h. Never NULL. [Speeds and travel time](../concepts/networks.md#speeds-and-travel-time) |
| `cost_s` | FLOAT | travel time in seconds: `length_m / (maxspeed_kmh / 3.6)` |
| `walk_type` | VARCHAR | walking only: `sidewalk`, `crossing`, `footpath`, `steps`, … [`walk_type` and `cycle_type`](../concepts/networks.md#walk_type-and-cycle_type) |
| `cycle_type` | VARCHAR | cycling only: `cycleway`, `cycle_track`, `cycle_lane`, … (same link) |
| `from_cell` | BIGINT | H3 cell of `source`, at `options.h3_resolution` |
| `to_cell` | BIGINT | H3 cell of `target` |
| `lca_res` | TINYINT | resolution of the smallest H3 cell that holds both `from_cell` and `to_cell` |
| `z_from`, `z_to` | DOUBLE | ground height in metres at `source` / `target`. Only after [`duckosm elevation`](../guides/elevation.md) |
| `z_from_<suffix>`, `z_to_<suffix>` | DOUBLE | the same on a second surface, from `duckosm elevation --suffix <suffix>` |

`maxspeed_kmh` and `cost_s` need `options.process_speeds` and `options.calculate_costs`;
`walk_type` and `cycle_type` need `options.functional_types`; the H3 columns need
`options.h3_indexing`. All are on by default.

### `private_edges`

The roads the mode may not use, built like `edges` and then moved out of it: private roads
(`access` is `private`) and, in driving, bus-only roads and bus lanes (`access` is `bus`; [bus
lanes](../concepts/networks.md#access-private-and-forbidden-roads)). They
are never routable: not in `turn_restrictions` or any export, nor in the routed rows of `edge_graph`
(the bus-only edges have rows of their own there, `uses = 'bus'`, which only the GMNS export reads), and `nodes` holds only
the ends of routable edges. The maps draw them, marked. [Access: private and forbidden roads](../concepts/networks.md#access-private-and-forbidden-roads).

Columns: those of `edges` from `edge_id` to `cycle_type`. The H3 columns and the elevation columns
are not added.

### `nodes`

The ends of the routable edges.

| Column | Type | Description |
|---|---|---|
| `node_id` | BIGINT | the OSM node id. Negative for a virtual node (below) |
| `geom` | GEOMETRY | the point |
| `h3_cell` | BIGINT | H3 cell at `options.h3_resolution` |
| `ele` | DOUBLE | ground height in metres. Only after [`duckosm elevation`](../guides/elevation.md) |
| `ele_<suffix>` | DOUBLE | the height on a second surface, from `duckosm elevation --suffix <suffix>`. `ele_dsm - ele` is the height above ground |

A **virtual node** splits a loop, or two edges between the same two nodes, at its middle point, so
that `(osm_id, source, target)` is unique and every edge gets its own `edge_id`. Its id is a hash too,
so it is the same on every rebuild: `-(hash(osm_id, source, refs::VARCHAR) >> 2)` for a loop,
`-(hash(osm_id, source, target, refs::VARCHAR) >> 2)` for a split edge.

### `edge_graph`

The graph of legal turns: one row for each pair of edges you can drive (or walk, or ride) one after
the other. Edge-based routing runs on it.

| Column | Type | Description |
|---|---|---|
| `from_edge` | BIGINT | the edge you come from |
| `to_edge` | BIGINT | the edge you go on to; its `source` is `from_edge`'s `target` |
| `via_edge` | BIGINT | the same as `to_edge` |
| `cost` | FLOAT | `cost_s` of `from_edge` |
| `uses` | VARCHAR | who may make the turn: the mode's own (`car` in driving, `walk`, `bike`) between two of its `edges`; `bus` in driving where one of the two is a bus-only edge of `private_edges` (`access = 'bus'`) |

Routing (`route()`, `to_networkx`, point routing, the route map, the SUMO connections) reads only the
mode's own rows; the `bus` rows are for the GMNS export's bus turns ([bus lanes](../design/bus_only_edges.md)).
A database built before `uses` holds only the mode's own rows.

A U-turn onto the reverse twin is included. Turns forbidden by `turn_restrictions` are removed: for a
`no_*` restriction that one turn, for an `only_*` restriction every other turn out of `from_edge`.

### `turn_restrictions`

Driving only. OSM turn restrictions, and the turn rules of a [fixes file](../guides/fix-osm-errors.md#turn-fixes-turn_restrictions),
mapped to edges. [Turn restrictions](../concepts/networks.md#turn-restrictions).

| Column | Type | Description |
|---|---|---|
| `restriction_id` | BIGINT | the OSM relation id; negative for a rule from the fixes file |
| `restriction_type` | VARCHAR | the OSM `restriction` value: `no_left_turn`, `only_straight_on`, … |
| `via_node` | BIGINT | the junction node |
| `from_edge_id` | BIGINT | the edge coming in |
| `to_edge_id` | BIGINT | the edge going out |

### `ways`

The OSM ways the mode kept, after the road filter and any fixes.

| Column | Type | Description |
|---|---|---|
| `osm_id` | BIGINT | the OSM way id |
| `highway` | VARCHAR | the `highway` value, as the mode uses it |
| `name` | VARCHAR | the OSM `name` |
| `maxspeed` | VARCHAR | the OSM `maxspeed` tag, as written |
| `oneway` | BOOLEAN | one-way for this mode |
| `surface`, `service`, `junction`, `layer`, `bridge`, `tunnel` | VARCHAR | the OSM tags |
| `access` | VARCHAR | the mode's access, as in `edges` |
| `lanes_fwd` | INTEGER | lanes in the drawing direction |
| `lanes_bwd` | INTEGER | lanes against it |
| `tags` | MAP(VARCHAR, VARCHAR) | every OSM tag of the way |
| `refs` | BIGINT[] | the way's OSM node ids, in order; reversed for `oneway=-1` |

### `way_nodes`

The nodes of `ways`, one row per node.

| Column | Type | Description |
|---|---|---|
| `way_id` | BIGINT | the OSM way id |
| `node_id` | BIGINT | the OSM node id |
| `seq` | BIGINT | position in the way, from 0 |

### `edge_id_map`

For each merged edge, the edges it was made from. It may be empty (driving in Monaco: no roads merge). Columns and use:
[`edge_id_map`](../concepts/edge-ids.md#edge_id_map).

| Column | Type |
|---|---|
| `old_edge_id` | BIGINT |
| `new_edge_id` | BIGINT |
| `seq` | INTEGER |
| `is_reverse` | BOOLEAN |
| `osm_id` | BIGINT |

### `virtual_nodes`

| Column | Type |
|---|---|
| `node_id` | BIGINT |
| `geom` | GEOMETRY |

Left over from the build and always empty: the virtual nodes are in `nodes`, with `node_id < 0`.

## `main` schema

| Table | Present when |
|---|---|
| [`visualization_metadata`](#visualization_metadata) | always |
| [`boundary`](#boundary) | a boundary is set |
| [`global_junctions`](#global_junctions) | a build from a PBF, with `options.global_junctions` (on) |
| [`boundary_cells`](#boundary_cells) | a build from a PBF, with `options.boundary_cells` (off) and a boundary |
| [`elevation_metadata`](#elevation_metadata) | after `duckosm elevation` |
| [`admin_boundaries`](#admin_boundaries) | after `duckosm admin`; in a `duckosm extract` result, only the sub-areas inside the area |

`main` also holds two macros: `edge_id_hash(osm_id, source, target)` gives the `edge_id` formula,
and `edge_id_hash_v1(osm_id, source, target, is_reverse)` the old one.
[Compute or match an id](../concepts/edge-ids.md#compute-or-match-an-id).

### `visualization_metadata`

One row: where to open a map, and the area's time zone.

| Column | Type | Description |
|---|---|---|
| `boundary_geojson` | JSON | the boundary; without one, the box around all OSM nodes |
| `center_lat` | DOUBLE | latitude of the centre of the boundary's bounding box (of all OSM nodes if there is no boundary; in a `duckosm extract` result, the centre of the area) |
| `center_lon` | DOUBLE | longitude of the same centre |
| `initial_zoom` | INTEGER | a map zoom level that shows the area, 1–14 (1–16 in a `duckosm extract` result) |
| `timezone` | VARCHAR | the area's IANA time zone, e.g. `Europe/Monaco`, found at the network node nearest the middle of all nodes. Always set: the build fails without it |
| `built_at` | TIMESTAMP | when the database was built (or extracted); NULL in a database built before duckOSM recorded it |
| `duckosm_version` | VARCHAR | the duckOSM version that built it; NULL like `built_at` |

### `boundary`

In a build from a PBF, the boundary file as read by DuckDB: one row per feature, a column per
GeoJSON property, and the shape in `geom` (GEOMETRY). A `duckosm extract` result has `geom` only. A file from [`duckosm boundary`](../guides/prepare-area.md) gives
`OGC_FID`, `name`, `osm_id`, `admin_level`, `area_km2`, `source`, `geom`. With `boundary.buffer_m`,
this is the grown boundary.

### `global_junctions`

The nodes where every mode cuts its ways, so a way has the same edges in every mode: nodes of a
road that another highway way in `raw.ways` also uses (a road, footway, path, cycleway, …; not
`proposed` / `construction` / `abandoned` / `razed` / `disused` ones), and nodes where a road ends.
A footway or path that passes through one is cut there too, even in a mode without the road.

| Column | Type | Description |
|---|---|---|
| `node_id` | BIGINT | the OSM node id |
| `is_road_junction` | BOOLEAN | always TRUE |

### `boundary_cells`

The H3 cells that cover the boundary, one row per cell and resolution
(`options.boundary_cell_resolutions`, or `options.h3_resolution`).

| Column | Type | Description |
|---|---|---|
| `h3_id` | VARCHAR | the H3 cell |
| `resolution` | INTEGER | its resolution |
| `geometry` | GEOMETRY | the hexagon |

### `elevation_metadata`

Where the heights came from: one row per height column (`ele`, `ele_dsm`, …). Running
`duckosm elevation` again replaces that column's row only. [Add elevation](../guides/elevation.md).

| Column | Type | Description |
|---|---|---|
| `ele_column` | VARCHAR | the column this row describes: `ele`, `ele_<suffix>` |
| `source` | VARCHAR | the model (`Copernicus GLO-30`), or the `--dem` file name |
| `source_type` | VARCHAR | `download` (from `--source`) or `file` (from `--dem`) |
| `uri` | VARCHAR | where it was read from |
| `resolution_m` | DOUBLE | the model's resolution in metres |
| `product` | VARCHAR | `DTM` (bare earth), `DSM` (surface) or `unknown` |
| `dem_crs` | VARCHAR | the model's CRS, e.g. `EPSG:4326` |
| `vertical_datum` | VARCHAR | e.g. `EGM2008`; `unknown` for a `--dem` file |
| `license` | VARCHAR | the model's licence; `unknown` for a `--dem` file |
| `nodata_fill` | DOUBLE | the height written where the model had no value |
| `n_nodes` | BIGINT | nodes sampled, over all modes |
| `n_nodata` | BIGINT | nodes that got `nodata_fill` |
| `sampled_at` | VARCHAR | when, as a UTC ISO time |
| `duckosm_version` | VARCHAR | the duckOSM version |

### `admin_boundaries`

Every OSM administrative boundary in the PBF. [Add administrative boundaries](../guides/admin-boundaries.md).

| Column | Type | Description |
|---|---|---|
| `osm_id` | BIGINT | the OSM relation id |
| `name` | VARCHAR | the OSM `name` |
| `name_en` | VARCHAR | the OSM `name:en`, when tagged |
| `admin_level` | INTEGER | the OSM `admin_level` |
| `geometry` | GEOMETRY | the (multi)polygon |
| `parent_osm_id` | BIGINT | the boundary it lies in, one level up; NULL at the top |

## `raw` schema

The OSM data as read from the PBF (after the cut to the boundary). Not in a build cut from a parent
database, nor in a `duckosm extract` result.

| Table | Column | Type |
|---|---|---|
| `raw.nodes` | `osm_id` | BIGINT |
| | `lat`, `lon` | DOUBLE |
| | `tags` | MAP(VARCHAR, VARCHAR) |
| `raw.ways` | `osm_id` | BIGINT |
| | `tags` | MAP(VARCHAR, VARCHAR) |
| | `refs` | BIGINT[] |
| `raw.relations` | `osm_id` | BIGINT |
| | `tags` | MAP(VARCHAR, VARCHAR) |
| | `refs` | BIGINT[] |
| | `ref_roles` | VARCHAR[] |
| | `ref_types` | ENUM('node', 'way', 'relation')[] |

`raw.ways` holds ways with at least two nodes; `raw.nodes` only nodes with a position. `refs`,
`ref_roles` and `ref_types` of a relation are its members, in order.

## `mm` schema

Routing across modes. [Routing across modes](../concepts/multimodal.md).

`mm.edges` is a **view**: every mode's `edges`, with a `mode` column. An `edge_id` can be in several
modes, so the key is `(mode, edge_id)`.

| Column | Type |
|---|---|
| `mode` | VARCHAR |
| `edge_id`, `source`, `target` | BIGINT |
| `cost_s`, `length_m` | FLOAT |
| `highway`, `name` | VARCHAR |
| `geometry` | GEOMETRY |

`mm.transfers` holds the changes of mode, at every node that walking shares with another mode. Each
change goes through walking.

| Column | Type | Description |
|---|---|---|
| `node_id` | BIGINT | the node where you change |
| `from_mode` | VARCHAR | the mode you leave |
| `to_mode` | VARCHAR | the mode you take |
| `cost_s` | DOUBLE | seconds the change costs: the flat cost (`--transfer-cost`, `multimodal.transfer_s`), unless `multimodal.transfer_costs` sets that pair |
| `kind` | VARCHAR | `park`, `retrieve` (driving), `bike_park`, `bike_unpark` (cycling) |

## `features` schema

The base-map layers: one table per layer, all with the columns `osm_id`, `osm_type`, `kind`,
`name`, `tags`, `geom`; `traffic` also has `bearing` (DOUBLE). Every layer: [Base-map layers](features.md).

## `visualization` schema

Layers computed for drawing. Written by [`duckosm levels`](cli.md#levels); not part of a build. Design: `docs/design/levels.md`.

### `edge_levels`

One row for every `edge_id` of the roads of all modes (`edges` and `private_edges` of each mode).

| Column | Type | Description |
|---|---|---|
| `edge_id` | BIGINT | the edge; the same as in the mode tables |
| `casing_start` | INTEGER | the casing number of the head at the start of the edge |
| `casing_level` | INTEGER | the casing number of the main part |
| `casing_end` | INTEGER | the casing number of the head at the end of the edge |
| `fill_level` | INTEGER | the fill number |
| `head_start_m`, `head_end_m` | DOUBLE | the length of each head as drawn, metres |
| `cap_start`, `cap_end` | VARCHAR | the end shape as drawn: `round`, `square` or `flat` |

The casings and fills are painted number by number, lowest first; at each number all casings before all fills. 0 is the ground.

### `edge_levels_meta`

One row: where the numbers came from.

| Column | Description |
|---|---|
| `source`, `area` | `area`, and the level area folder they were solved in |
| `n_edges`, `edge_hash` | the number of edges and a hash of their ids: the numbers belong to these edges only |
| `roadstyle_version`, `created` | the roadstyle version and when |

## `bus` schema

The OSM bus route relations (`route` = `bus`, `trolleybus`, `share_taxi`) as the driving edges they travel, in order and in their direction. No geometry: the
edges are in `driving.edges` and, for bus lanes and bus-only roads, `driving.private_edges`. Written by a build from a PBF with driving (`options.bus_routes`, on)
or by [`duckosm bus`](cli.md#bus). How: [Bus routes](../concepts/networks.md#bus-routes).

### `routes`

One row per route relation.

| Column | Type | Description |
|---|---|---|
| `osm_id` | BIGINT | the relation |
| `route` | VARCHAR | `bus`, `trolleybus` or `share_taxi` |
| `ref`, `name`, `operator`, `from`, `to`, `network` | VARCHAR | the relation's tags |
| `edges` | INTEGER | rows in `route_edges` |
| `outside` | INTEGER | member ways not in the file (outside the area) |
| `gaps` | VARCHAR[] | `'<way_id> <kind>'` for every other way the route could not be followed on; empty: matched fully |

### `route_edges`

One row per route and edge it travels.

| Column | Type | Description |
|---|---|---|
| `route_id` | BIGINT | the relation (`routes.osm_id`) |
| `ref` | VARCHAR | the route's `ref` (its line) |
| `seq` | INTEGER | 1, 2, … along the route |
| `edge_id`, `edge_ref` | BIGINT, VARCHAR | the driving edge |
| `bus_lane` | BOOLEAN | the edge is in `driving.private_edges` (`access = 'bus'`) |
