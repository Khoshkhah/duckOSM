# Database schema

Every table and column a build writes. Types are as DuckDB reports them. Geometries are
longitude / latitude (EPSG:4326). How to query them: [Query the database](../guides/query.md).

| Schema | Holds | Written by |
|---|---|---|
| [`<mode>`](#mode-schema) | one network: `driving`, `walking` or `cycling` | `build` |
| [`main`](#main-schema) | the area: boundary, map center, time zone, …; the `edge_id` macros | `build`, `elevation`, `admin` |
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
| [`turn_path_restrictions`](#turn_path_restrictions) | driving, with `options.extract_restrictions` (on); in a clip or extract, only if the parent has it |
| [`turn_permission`](#turn_permission) | `options.build_graph` (on); in a clip or extract, only if the parent has it |
| [`ways`](#ways), [`way_nodes`](#way_nodes) | a build from a PBF, and `duckosm extract` |
| [`edge_id_map`](#edge_id_map) | a build from a PBF with `options.merge_segments` (on) |
| [`virtual_nodes`](#virtual_nodes) | a build from a PBF; always empty |
| [`lane_profile`](#lane_profile) | driving, a build from a PBF |

A build cut from a parent database (`source.type: duckdb`) has only `edges`, `private_edges`,
`nodes`, and the tables of turns (`edge_graph`, `turn_restrictions`, `turn_path_restrictions`, `turn_permission`)
that the parent has.

### `edges`

One row per direction of a road: a two-way road has two edges. What each mode keeps:
[What each network contains](../concepts/networks.md).

| Column | Type | Description |
|---|---|---|
| `edge_id` | BIGINT | the edge's id: `(hash(osm_id, source, target) >> 1)::BIGINT`. The same on every rebuild and in every mode. [Stable edge ids](../concepts/edge-ids.md) |
| `edge_ref` | VARCHAR | a readable id, `{osm_id}#{seq}` then `f` or `r` for the direction (for example `41790239#2f`); `seq` counts the way's edges from its start. Not stable across rebuilds: join on `edge_id` |
| `source` | BIGINT | the node the edge starts at |
| `target` | BIGINT | the node the edge ends at |
| `osm_id` | BIGINT | the OSM way. A merged edge has the way at its `source` end. Negative for a [connector edge](../concepts/cleanup.md#connect-dangling-paths) |
| `highway` | VARCHAR | the OSM `highway` value |
| `name` | VARCHAR | the OSM `name` |
| `maxspeed` | VARCHAR | the OSM `maxspeed` tag, as written. Only when `options.process_speeds` is off: then `maxspeed_kmh` replaces it |
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
| `length_m` | FLOAT | length in meters |
| `dismount` | BOOLEAN | cycling only: TRUE on a [dismount edge](../concepts/networks.md#dismount-edges), where the bike is pushed |
| `maxspeed_kmh` | FLOAT | speed in km/h. Never NULL. [Speeds and travel time](../concepts/networks.md#speeds-and-travel-time) |
| `cost_s` | FLOAT | travel time in seconds: `length_m / (maxspeed_kmh / 3.6)` |
| `walk_type` | VARCHAR | walking only: `sidewalk`, `crossing`, `footpath`, `steps`, … [`walk_type` and `cycle_type`](../concepts/networks.md#walk_type-and-cycle_type) |
| `cycle_type` | VARCHAR | cycling only: `cycleway`, `cycle_track`, `cycle_lane`, … (same link) |
| `from_cell` | BIGINT | H3 cell of `source`, at `options.h3_resolution` |
| `to_cell` | BIGINT | H3 cell of `target` |
| `lca_res` | TINYINT | resolution of the smallest H3 cell that holds both `from_cell` and `to_cell` |
| `from_tile` | BIGINT | [tile key](#tile-keys) of `source`, at zoom 28 |
| `to_tile` | BIGINT | tile key of `target`, at zoom 28 |
| `lca_zoom` | TINYINT | zoom of the smallest tile that holds both `from_tile` and `to_tile`, 0 to 28 |
| `z_from`, `z_to` | DOUBLE | ground height in meters at `source` / `target`. Only after [`duckosm elevation`](../guides/elevation.md) |
| `z_vertices` | DOUBLE[] | the ground height at each vertex of `geometry`, in its order. Only after `duckosm elevation` |
| `z_from_<suffix>`, `z_to_<suffix>`, `z_vertices_<suffix>` | DOUBLE, DOUBLE[] | the same on a second surface, from `duckosm elevation --suffix <suffix>` |

`maxspeed_kmh` and `cost_s` need `options.process_speeds` and `options.calculate_costs`;
`walk_type` and `cycle_type` need `options.functional_types`; the H3 columns need
`options.h3_indexing`; the tile columns need `options.tile_indexing` and the Rust build (`duckosm --config`,
`duckosm <PBF>`). All are on by default.

### `private_edges`

The roads that the mode cannot use. They are drawn on the maps, but never routed and never exported.

| `access` | Roads |
|---|---|
| `private` | private roads |
| `bus` | driving: bus-only roads and bus lanes |
| `pedestrian`, `delivery`, `private` with `highway = 'pedestrian'` | driving: pedestrian streets that cars cannot use |

Columns: those of `edges` from `edge_id` to `cycle_type`. The H3 columns, the tile columns and the
elevation columns are not added.

### `nodes`

The ends of the routable edges.

| Column | Type | Description |
|---|---|---|
| `node_id` | BIGINT | the OSM node id. Negative for a virtual node (below) |
| `geom` | GEOMETRY | the point |
| `h3_cell` | BIGINT | H3 cell at `options.h3_resolution` |
| `tile` | BIGINT | [tile key](#tile-keys) of the point, at zoom 28 |
| `ele` | DOUBLE | ground height in meters. Only after [`duckosm elevation`](../guides/elevation.md) |
| `ele_<suffix>` | DOUBLE | the height on a second surface, from `duckosm elevation --suffix <suffix>`. `ele_dsm - ele` is the height above ground |

A **virtual node** splits a loop, or two edges between the same two nodes, at its middle point, so
that `(osm_id, source, target)` is unique and every edge gets its own `edge_id`. Its id is a hash too,
so it is the same on every rebuild: `-(hash(osm_id, source, refs::VARCHAR) >> 2)` for a loop,
`-(hash(osm_id, source, target, refs::VARCHAR) >> 2)` for a split edge.

### Tile keys {#tile-keys}

The tile columns (`nodes.tile`, `edges.from_tile`, `to_tile`, `lca_zoom`) are for tileroute
(`https://github.com/Khoshkhah/tileroute`). Tileroute routes on a graph that is kept by map tiles
and reads only the tiles that a route needs. The tiles are the standard map tiles (z/x/y, Web Mercator),
the same tiles as the tile build. A tile at one zoom holds exactly 4 tiles of the next zoom.

A tile key is a BIGINT, always 0 or more:

| Bits | Value |
|---|---|
| 63–62 | 0 |
| 61–6 | the quadkey digits, the coarsest first. The digit at zoom level i is `2 * ybit + xbit`. A tile at zoom z fills the top `2z` bits; the other bits are 0 |
| 5–0 | the zoom z, 0 to 28 |

`key = (digits << (6 + 2 * (28 - z))) | z`. The parent at zoom r:
`(key & ~((1 << (6 + 2 * (28 - r))) - 1)) | r`. Thus the keys of a tile and of all its smaller tiles
are in one range, and `ORDER BY tile` puts near nodes near each other.

The build gives each node point the key of its tile at zoom 28 (about 15 cm at the equator). The tile of a
point is `x = floor((lon + 180) / 360 * 2^z)`, `y = floor((1 - asinh(tan(lat)) / pi) / 2 * 2^z)`, with
the latitude clamped to ±85.0511287798 degrees. An edge takes the keys of its two end nodes, so a tile
build and the whole build give the same keys. The value is NULL when the node has no point.

`lca_zoom` is the zoom of the smallest tile that holds both ends: the number of quadkey digits that
`from_tile` and `to_tile` share. An edge with `lca_zoom` 14 stays inside one zoom-14 tile.

### `edge_graph`

The graph of legal turns: one row for each pair of edges you can drive (or walk, or ride) one after
the other. Edge-based routing runs on it.

| Column | Type | Description |
|---|---|---|
| `from_edge` | BIGINT | the edge you come from |
| `to_edge` | BIGINT | the edge you go on to; its `source` is `from_edge`'s `target` |
| `via_edge` | BIGINT | the same as `to_edge` |
| `cost` | FLOAT | `cost_s` of `from_edge` |
| `uses` | VARCHAR | who can make the turn: the mode's own (`car` in driving, `walk`, `bike`) between two of its `edges`; where one of the two edges is in `private_edges`, its `access` label: `bus`, `private`, `pedestrian` or `delivery`. Two restricted edges with different labels have no row |

- Routing reads only the rows of the mode's own use (`car`, `walk`, `bike`). To read the restricted rows too, give
  `uses`: `to_networkx(con, uses="pedestrian,private")` or `duckosm export-graph -g edge --uses pedestrian,private`.
- A U-turn onto the reverse twin is in the table.
- The build removes the turns that a restriction forbids for all vehicles at all times. A restriction for one vehicle
  class, or with a condition, removes no turn: [`turn_permission`](#turn_permission) holds it.

### `turn_restrictions`

Driving only. OSM turn restrictions, and the turn rules of a [fixes file](../guides/edit-osm-data.md#the-fixes-file),
mapped to edges. [Turn restrictions](../concepts/networks.md#turn-restrictions).

| Column | Type | Description |
|---|---|---|
| `restriction_id` | BIGINT | the OSM relation id; negative for a rule from the fixes file |
| `restriction_type` | VARCHAR | the OSM `restriction` value: `no_left_turn`, `only_straight_on`, … |
| `except_vehicles` | VARCHAR | the vehicles that the rule does not apply to (OSM `except`, separated by `;`); NULL: none |
| `applies_to` | VARCHAR | the one vehicle class that the rule applies to (`restriction:<vehicle>`); NULL: all |
| `condition` | VARCHAR | when the rule applies (`restriction:conditional`); NULL: always |
| `via_node` | BIGINT | the junction node |
| `from_edge_id` | BIGINT | the edge coming in |
| `to_edge_id` | BIGINT | the edge going out |

### `turn_path_restrictions`

Driving only. The OSM turn restrictions with a via **way**: they forbid or make necessary a path of edges, not one turn.
One row for each edge path. `edge_graph` cannot hold them. The exports that can show a path use them.
[Design](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/turn_permissions.md).

| Column | Type | Description |
|---|---|---|
| `restriction_id` | BIGINT | the OSM relation id |
| `restriction_type` | VARCHAR | the OSM `restriction` value |
| `except_vehicles`, `applies_to`, `condition` | VARCHAR | as in [`turn_restrictions`](#turn_restrictions) |
| `from_edge` | BIGINT | the edge coming in |
| `via_edges` | BIGINT[] | the edges along the via way(s), in order |
| `to_edge` | BIGINT | the edge going out |

### `turn_permission`

The turns that `edge_graph` cannot show: the turns of a restriction with an exception, a vehicle class or a condition.
One row for each turn and restriction. Empty when the area has no such restriction.

| Column | Type | Description |
|---|---|---|
| `from_edge` | BIGINT | the edge you come from |
| `to_edge` | BIGINT | the edge you go on to |
| `allowed` | BOOLEAN | TRUE: the turn is removed from `edge_graph`, but `vehicles` can make it. FALSE: the turn is in `edge_graph`, but `vehicles` cannot make it (when `condition` applies) |
| `vehicles` | VARCHAR | the vehicle classes of the row: the exempt vehicles (`allowed` TRUE), or the class of the rule (`allowed` FALSE); NULL: all |
| `except_vehicles` | VARCHAR | with `allowed` FALSE: the vehicles that the rule does not apply to |
| `condition` | VARCHAR | when the rule applies; NULL: always |
| `restriction_id` | BIGINT | the restriction (`turn_restrictions.restriction_id`) |

### `ways`

The OSM ways the mode kept, after the road filter and any fixes.

| Column | Type | Description |
|---|---|---|
| `osm_id` | BIGINT | the OSM way id |
| `highway` | VARCHAR | the `highway` value, as the mode uses it |
| `name` | VARCHAR | the OSM `name` |
| `maxspeed` | VARCHAR | the OSM `maxspeed` tag, as written |
| `oneway` | BOOLEAN | one-way for this mode |
| `bus_back` | BOOLEAN | driving: a one-way way with a bus lane in the other direction. The build adds that reverse edge with `access` = `bus` |
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

For each merged edge, the edges that it was made from. It can be empty, when no edges merge. Columns and use:
[`edge_id_map`](../concepts/edge-ids.md#edge_id_map).

| Column | Type |
|---|---|
| `old_edge_id` | BIGINT |
| `new_edge_id` | BIGINT |
| `seq` | INTEGER |
| `is_reverse` | BOOLEAN |
| `osm_id` | BIGINT |

### `lane_profile` {#lane_profile}

Every driving edge's lanes, and every bus-only edge's (`private_edges` with `access = 'bus'`), decided once
([design](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/gmns_lane_profile.md)): one row per lane, left to right.

| Column | Type | Meaning |
|---|---|---|
| `edge_id` | BIGINT | the edge |
| `lane_num` | INTEGER | 1 = the left-most through lane, 1..n the motor lanes; -1 a bike lane left of them (on a one-way, `cycleway:left=lane`); n+1 a bike lane right of them |
| `use` | VARCHAR | `auto`, `bus`, `bike`, `bus,bike` |
| `width_m` | DOUBLE | the width of the lane, in meters |
| `turn` | VARCHAR | its `turn:lanes` entry, or NULL |
| `source` | VARCHAR | the tag that gives the lane and its use: `lanes`, `lanes:forward`, `lanes:backward`, `turn:lanes`, `override` (the fixes file), `inherited` (from the road that it continues), `default` (no tag), `bus:lanes`, `psv:lanes`, `lanes:bus`, `busway:*`, `cycleway:*`, `access=bus` |
| `width_source` | VARCHAR | `width:lanes`, `cycleway:*:width`, or `default` |

`lanes` does not count the bike lanes (as OSM, and the OSM import of SUMO). A count or a width that no tag gives has
`default`. To see how much of the network has no lane tags, count these rows (`WHERE source = 'default'`).

### `virtual_nodes`

| Column | Type |
|---|---|
| `node_id` | BIGINT |
| `geom` | GEOMETRY |

Always empty: the virtual nodes are in `nodes`, with `node_id < 0`.

## `main` schema

| Table | Present when |
|---|---|
| [`visualization_metadata`](#visualization_metadata) | always |
| [`boundary`](#boundary) | a boundary is set |
| [`global_junctions`](#global_junctions) | a build from a PBF, with `options.global_junctions` (on) |
| [`boundary_cells`](#boundary_cells) | a build from a PBF, with `options.boundary_cells` (off) and a boundary |
| [`elevation_metadata`](#elevation_metadata) | after `duckosm elevation` |
| [`elevation_grid`](#elevation_grid) | the build with `--dem`, and duckOSM Studio |
| [`admin_boundaries`](#admin_boundaries) | after `duckosm admin`; in a `duckosm extract` result, only the sub-areas inside the area |
| [`building_faces`](#building_faces) | a build from a PBF, with `options.building_faces` (off) and a boundary |
| [`building_entrances`](#building_entrances) | `duckosm build` from a PBF, with `options.building_faces` (off) and a boundary |

`main` also holds the macro `edge_id_hash(osm_id, source, target)`, the `edge_id` formula:
[Compute or match an id](../concepts/edge-ids.md#compute-or-match-an-id).

### `visualization_metadata`

One row: where to open a map, and the time zone of the area.

| Column | Type | Description |
|---|---|---|
| `boundary_geojson` | JSON | the boundary; without one, the box around all OSM nodes |
| `center_lat` | DOUBLE | latitude of the center of the bounding box of the boundary (of all OSM nodes if there is no boundary; in a `duckosm extract` result, the center of the area) |
| `center_lon` | DOUBLE | longitude of the same center |
| `initial_zoom` | INTEGER | a map zoom level that shows the area, 1–14 (1–16 in a `duckosm extract` result) |
| `timezone` | VARCHAR | the IANA time zone of the area, for example `Europe/Monaco`, at the network node nearest to the middle of all nodes. Always set: the build stops without it |
| `built_at` | TIMESTAMP | when the database was built (or extracted) |
| `duckosm_version` | VARCHAR | the duckOSM version that built it |
| `build_config` | VARCHAR | the config of the build as YAML (all keys, with the options of the command line). [`duckosm edit`](cli.md#edit) builds the database again from it. NULL in a database from `duckosm extract`, or from `duckosm` without `--config` |
| `build_dir` | VARCHAR | the folder in which the build ran: the relative paths of `build_config` are for this folder; NULL like `build_config` |

### `boundary`

In a build from a PBF, the boundary file as read by DuckDB: one row per feature, a column per
GeoJSON property, and the shape in `geom` (GEOMETRY). A `duckosm extract` result has `geom` only. A file from [`duckosm boundary`](../guides/prepare-area.md) gives
`OGC_FID`, `name`, `osm_id`, `admin_level`, `area_km2`, `source`, `geom`. With `boundary.buffer_m`,
this is the grown boundary.

### `global_junctions`

The nodes where every mode cuts its ways, so a way has the same edges in every mode. A node is in the table when two
or more highway ways in `raw.ways` use it, or when a road ends there. Ways with `highway` = `proposed`, `construction`,
`abandoned`, `razed` or `disused` do not count. A footway or path that goes through a road junction is cut there,
also in a mode without the road.

| Column | Type | Description |
|---|---|---|
| `node_id` | BIGINT | the OSM node id |
| `is_road_junction` | BOOLEAN | TRUE: a road uses the node. FALSE: only paths (footway, path, cycleway, steps, …) meet there, and only the paths are cut |

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
| `ele_column` | VARCHAR | the height column of the row: `ele`, `ele_<suffix>` |
| `source` | VARCHAR | the model (`Copernicus GLO-30`), or the `--dem` file name |
| `source_type` | VARCHAR | `download` (from `--source`) or `file` (from `--dem`) |
| `uri` | VARCHAR | where it was read from |
| `resolution_m` | DOUBLE | the resolution of the model, in meters |
| `product` | VARCHAR | `DTM` (bare earth), `DSM` (surface) or `unknown`. For a `--dem` file, the value of `--product` (`--dem-product` in `duckosm`); `unknown` like `vertical_datum` |
| `dem_crs` | VARCHAR | the CRS of the model, for example `EPSG:4326` |
| `vertical_datum` | VARCHAR | for example `EGM2008`. For a `--dem` file, the value of `--vertical-datum` (`--dem-vertical-datum` in `duckosm`); `unknown` only with `--not-reference` or `--suffix` |
| `license` | VARCHAR | the license of the model. For a `--dem` file, the value of `--license` (`--dem-license`); `unknown` like `vertical_datum` |
| `nodata_fill` | DOUBLE | the height written where the model had no value |
| `n_nodes` | BIGINT | nodes sampled, over all modes |
| `n_nodata` | BIGINT | nodes that got `nodata_fill` |
| `sampled_at` | VARCHAR | when, as a UTC ISO time |
| `duckosm_version` | VARCHAR | the program and its version, for example `duckosm 0.7.0` |
| `sha256` | VARCHAR | the **ground id**: the SHA-256 of the bytes of the DEM file, in lowercase hexadecimal, as `sha256sum` gives it. NULL when the step read no whole file: Copernicus tiles streamed from AWS, or a `--dem` URL or GDAL virtual path (`/vsi…`). EU-DTM is a downloaded file and has a hash ([The ground reference](../concepts/ground-reference.md)) |

### `elevation_grid`

The pixels of the DEM over the network: the ground between the roads, for a reader that makes a terrain (for example
the `gltf` adapter of SimReady). The rows are the pixels of the raster, with no resampling, over the bounding box of the
nodes of all modes, plus 3 pixels on each side. The build with `--dem` and duckOSM Studio write it.

| Column | Type | Description |
|---|---|---|
| `col` | INTEGER | the column of the pixel, 0 at the west edge |
| `row` | INTEGER | the row of the pixel, 0 at the north edge (the order of the raster) |
| `lon` | DOUBLE | the longitude of the pixel center (the CRS of the DEM, EPSG:4326) |
| `lat` | DOUBLE | the latitude of the pixel center |
| `z` | DOUBLE | the height of the pixel, to the centimeter, as `ele` and `z_vertices`; NULL on nodata |

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

### `building_faces`

One row for each wall of a building of the area: a segment of the outer ring of a building way, or of a ring of a multipolygon building
relation. The build finds the type of each wall from the street edges of the driving and walking networks, then applies the wall
rules of the [fixes file](../guides/edit-osm-data.md#the-fixes-file). A wall shorter than 0.1 m has no row. The method:
[The walls of the buildings](../concepts/building-walls.md).

| Column | Type | Description |
|---|---|---|
| `building_id` | BIGINT | the OSM id of the building way or relation |
| `building_type` | VARCHAR | `way` or `relation` |
| `face_index` | INTEGER | the place of the wall in the ring (it changes when the ring gets a node) |
| `from_node`, `to_node` | BIGINT | the OSM nodes at the ends of the wall: with `building_id`, the key of the wall |
| `face_type` | VARCHAR | `front`, `back`, `side`, `party` (two buildings share the wall) or `court` (a wall of a courtyard) |
| `computed_type` | VARCHAR | the type that the build found; `face_type` has the type of a wall rule |
| `edited` | BOOLEAN | a wall rule gives `face_type` |
| `type_reason` | VARCHAR | why the wall has its type: `street`, `corner`, `entrance`, `address`, `inferred`, `opposite`, `shared`, `other`, `inner` or `none` |
| `door` | VARCHAR | the kind of door from a wall rule, for example `main`; NULL for no door |
| `length_m` | FLOAT | the length of the wall |
| `azimuth_deg` | FLOAT | the direction of the wall |
| `street_osm_id`, `street_edge_id`, `street_name` | BIGINT, BIGINT, VARCHAR | the street that the wall faces; NULL for none |
| `distance_m` | FLOAT | the distance to that street |
| `geometry` | GEOMETRY | the wall as a LineString |

### `building_entrances`

The doors of the area: one row for each OSM node with an `entrance` or a `door` tag. `duckosm build` writes it with
`building_faces`.

| Column | Type | Description |
|---|---|---|
| `osm_id` | BIGINT | the OSM node id |
| `osm_type` | VARCHAR | always `node` |
| `kind` | VARCHAR | the `entrance` value (`main`, `yes`, …), or `door` when the node has only a `door` tag |
| `door`, `access`, `housenumber`, `street`, `level`, `wheelchair`, `name` | VARCHAR | the OSM tags `door`, `access`, `addr:housenumber`, `addr:street`, `level`, `wheelchair`, `name` |
| `building_id` | BIGINT | the building way that has the node in its outline (the smallest id when two buildings share it); NULL: none |
| `tags` | MAP(VARCHAR, VARCHAR) | all OSM tags of the node |
| `geom` | GEOMETRY | the point |

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

The drawing order of the roads: which road a map paints over which. The build (`options.levels`) and [`duckosm levels`](cli.md#levels) write it. Your edits of the order are in `level_edits`, `level_heads` and `level_caps`, and a new build keeps them. [Drawing order](../guides/drawing-order.md).

### `edge_levels`

One row for every `edge_id` of the roads of all modes (`edges` and `private_edges` of each mode).

| Column | Type | Description |
|---|---|---|
| `edge_id` | BIGINT | the edge; the same as in the mode tables |
| `casing_start` | INTEGER | the casing number of the head at the start of the edge |
| `casing_level` | INTEGER | the casing number of the main part |
| `casing_end` | INTEGER | the casing number of the head at the end of the edge |
| `fill_level` | INTEGER | the fill number |
| `head_start_m`, `head_end_m` | DOUBLE | the length of each head as drawn, in meters |
| `cap_start`, `cap_end` | VARCHAR | the end shape as drawn: `round`, `square` or `flat` |

The map paints the casings and the fills number by number, lowest first. At each number, it paints all casings before all fills. 0 is the ground.

### `edge_levels_meta`

One row: where the numbers came from.

| Column | Description |
|---|---|
| `source`, `area` | `area`, and the level area folder they were solved in |
| `n_edges`, `edge_hash` | the number of edges and a hash of their ids: the numbers belong to these edges only |
| `roadstyle_version`, `created` | the roadstyle version and when |

### `level_edits`, `level_heads`, `level_caps`

Your changes to the drawing order. The level editor writes them at each Apply, and a new build keeps them.

| Table | Columns |
|---|---|
| `level_edits` | `relation` (`order`, `stack`, `meet`), `a`, `b` (edges), `a_end`, `b_end`, `enabled` |
| `level_heads` | `road`, `start_m`, `end_m`: the head length at each end, in meters |
| `level_caps` | `road`, `start`, `end`: `round`, `square` or `flat` |

### `level_unmatched`

The rows of those tables that name a road that is not in this file (a new build changed it): `kind` (the table) and `row` (the row as JSON). The build does not
use them and does not delete them: the next build tries them again. The table is not there when there are no such rows.

## `bus` schema

The OSM bus route relations (`route` = `bus`, `trolleybus`, `share_taxi`) as the driving edges that they use, in order and in their direction. No geometry: the
edges are in `driving.edges` and, for bus lanes and bus-only roads, in `driving.private_edges`. Written by a build from a PBF with driving (`options.bus_routes`, on)
or by [`duckosm bus`](cli.md#bus). How: [Bus routes](../concepts/networks.md#bus-routes).

### `routes`

One row per route relation.

| Column | Type | Description |
|---|---|---|
| `osm_id` | BIGINT | the relation |
| `route` | VARCHAR | `bus`, `trolleybus` or `share_taxi` |
| `ref`, `name`, `operator`, `from`, `to`, `network` | VARCHAR | the relation's tags |
| `edges` | INTEGER | rows in `route_edges` |
| `outside` | INTEGER | the member ways that are not in the file (outside the area) |
| `gaps` | VARCHAR[] | `'<way_id> <kind>'` for each other way where the build cannot follow the route; empty: the full route matches |

### `route_edges`

One row for each route and edge.

| Column | Type | Description |
|---|---|---|
| `route_id` | BIGINT | the relation (`routes.osm_id`) |
| `ref` | VARCHAR | the route's `ref` (its line) |
| `seq` | INTEGER | 1, 2, … along the route |
| `edge_id`, `edge_ref` | BIGINT, VARCHAR | the driving edge |
| `bus_lane` | BOOLEAN | the edge is in `driving.private_edges` (`access = 'bus'`) |
