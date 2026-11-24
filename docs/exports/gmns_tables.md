# GMNS tables

Each table of the GMNS database of [`duckosm gmns`](gmns.md), and each of its columns. The standard: the
[GMNS specification](https://zephyr-data-specs.github.io/GMNS/spec/) 0.97.

| Rule | |
|---|---|
| Schemas | Every table belongs to a schema: `gmns_driving`, `gmns_walking`, `gmns_cycling`, or `gmns_all` (with `--combined`). There are no tables outside schemas. With `--meso`, `--micro` also `meso_<mode>`, `micro_<mode>`. See [network schemas](gmns.md#network-schemas). |
| Ids | `link_id` is the duckOSM `edge_id` (BIGINT), the same after each build and in each mode. `node_id` is the OSM node id |
| Units | metres, km/h, longitude and latitude (EPSG:4326) |
| Geometry | `geometry` is WKT text (in the CSV files). `geom` is the same shape as a DuckDB `GEOMETRY` (not in the CSV files) |
| Links | one link for each direction of a road. `directed` is always `true` |
| Lane numbers | lane 1 is the leftmost lane in the direction of travel |

The column "Spec" says where a column comes from:

| Mark | Meaning | In the CSV files |
|---|---|---|
| ✅ | a column of the standard, with a value from OSM | yes |
| ∅ | a column of the standard that OSM has no data for: empty | yes, empty |
| ➕ | a duckOSM column, not in the standard | only with `--csv-extensions`, with the prefix `u_` |

## The tables

In the GMNS database (`<area>_gmns.duckdb`), **every network table belongs to a network schema**. The default `main`
schema has one table: [`elevation_metadata`](#elevation_metadata), when the source db has heights.

### The network schemas

#### Macro schemas (`duckosm gmns`)
* **`gmns_driving` (18 tables, with netconvert):** Driving network for cars, buses, and trucks. Includes lanes, turns, signals, curb parking, crosswalks, intersection turn curves, and the road chains (`chains`).
* **`gmns_walking` (15 tables):** Pedestrian network for sidewalks, paths, and crosswalks. Tracks parallel roads via `link_along`, connects footways to road crossings, and lists unpaired lanes in `lane_check`.
* **`gmns_cycling` (13 tables):** Bicycle network for cycleways and bikeable roads. Tracks parallel roads via `link_along`.
* **`gmns_all` (5 tables):** Created with `--combined`. Merges shared streets across modes into one link with `allowed_uses = 'auto,bike,walk'`. Lanes and turns stay in the mode schemas.

#### Meso and micro schemas (`--meso`, `--micro`)
* **`meso_<mode>` (2 tables, via `--meso`):** Road sections and junction turn connectors for mesoscopic simulation (default: `meso_driving`).
* **`micro_<mode>` (2 tables, via `--micro`):** Discrete 7-meter lane cells, lateral lane changes, and turn movements for microscopic simulation (default: `micro_driving`).

### Table matrix: macro schemas

| Table | In standard | `gmns_all` | `gmns_driving` | `gmns_walking` | `gmns_cycling` | One row is |
|---|:---:|:---:|:---:|:---:|:---:|---|
| [`link`](#link) | required | ✅ *(merged)* | ✅ | ✅ | ✅ | one direction of a road or path: length, speed, lanes, capacity |
| [`node`](#node) | required | ✅ *(merged)* | ✅ | ✅ | ✅ | a junction or endpoint (`x_coord`, `y_coord`, `ctrl_type`) |
| [`config`](#config) | yes | ✅ | ✅ | ✅ | ✅ | dataset metadata: units, CRS, version |
| [`use_definition`](#use_definition) | yes | ✅ | ✅ | ✅ | ✅ | a use of a lane (`auto`, `bus`, `bike`, `walk`) |
| [`use_group`](#use_group) | yes | ✅ | ✅ | ✅ | ✅ | the uses of a mode |
| [`geometry`](#geometry) | yes | — *(in link)* | ✅ | ✅ | ✅ | the spatial shape of a link |
| [`lane`](#lane) | yes | — | ✅ | ✅ | ✅ | one lane: use, width, and turns |
| [`movement`](#movement) | yes | — | ✅ | ✅ | ✅ | one legal turn at a junction |
| [`location`](#location) | yes | — | ✅ | ✅ | ✅ | a point on a link: crossing, sign, stop |
| [`signal_controller`](#signal_controller) | yes | — | ✅ | ✅ | ✅ | traffic signal controller at an intersection |
| [`zone`](#zone) | yes | — | ✅ | ✅ | ✅ | study area boundary polygon |
| [`curb_seg`](#curb_seg) | yes | — | ✅ | ✅ | ✅ | on-street parking and curb segments |
| [`lane_connector`](#lane_connector) | ➕ | — | ✅ | ✅ | — | path through a junction (driving) or footway join (walking) |
| [`crossing`](#crossing) | ➕ | — | ✅ | — | — | pedestrian crossing painted on a roadway |
| [`lane_crossing`](#lane_crossing) | ➕ | — | ✅ | — | — | specific vehicular road lanes crossed by a crossing |
| [`link_along`](#link_along) | ➕ | — | ✅ *(ref)* | ✅ | ✅ | footpath or cycle track running parallel to a road |
| [`chains`](#chains) | ➕ | — | ✅ | — | — | the links of one road between two real junctions, for `sumo --merge` and `matsim --merge` |
| [`lane_check`](#lane_check) | ➕ | — | optional | ✅ | optional | diagnostic log of unmapped lane arrows or disconnected turns |

### Table matrix: meso and micro schemas

| Schema | Table | In standard | One row is | Key columns |
|---|---|:---:|---|---|
| `meso_<mode>` | `meso_node` | ➕ duckOSM | the end of a road section | `node_id`, `x_coord`, `y_coord`, `macro_node_id`, `macro_link_id`, `geom` |
| `meso_<mode>` | `meso_link` | ➕ duckOSM | a road section or junction connector | `link_id`, `meso_type`, `from_node_id`, `to_node_id`, `length`, `lanes`, `free_speed`, `macro_link_id`, `movement_id`, `geom` |
| `micro_<mode>` | `micro_node` | ➕ duckOSM | the boundary vertex of a lane cell | `node_id`, `x_coord`, `y_coord`, `lane_id`, `cell_k`, `macro_link_id`, `lane_no`, `geom` |
| `micro_<mode>` | `micro_link` | ➕ duckOSM | a 7 m cell, lane change, or turn arc | `link_id`, `cell_type`, `from_node_id`, `to_node_id`, `length`, `width`, `lane_no`, `macro_link_id`, `meso_link_id`, `geom` |

## config

| Column | Type | Spec | Value |
|---|---|---|---|
| `dataset_name` | VARCHAR | ✅ | the output name and the mode, for example `monaco_gmns_driving` |
| `short_length`, `long_length` | VARCHAR | ✅ | `meter` |
| `speed` | VARCHAR | ✅ | `kmh` |
| `crs` | VARCHAR | ✅ | `EPSG:4326` |
| `geometry_field_format` | VARCHAR | ✅ | `wkt` |
| `version_number` | DOUBLE | ✅ | `0.97` |
| `id_type` | VARCHAR | ✅ | `integer` |
| `duckosm_version` | VARCHAR | ➕ | the duckOSM version that wrote the database, for example `duckosm 0.8.0` |
| `currency` | VARCHAR | ∅ | empty |

## use_definition

| Column | Type | Spec | Holds |
|---|---|---|---|
| `use` | VARCHAR | ✅ | `auto`, `bus`, `bike`, `walk` |
| `persons_per_vehicle` | DECIMAL | ✅ | `auto` 1, `bus` 25, `bike` 1, `walk` 1 |
| `pce` | DECIMAL | ✅ | passenger-car equivalents: `auto` 1.0, `bus` 2.0, `bike` 0.2, `walk` 0 |
| `special_conditions`, `description` | VARCHAR | ∅ | empty |

## use_group

| Column | Type | Spec | Holds |
|---|---|---|---|
| `use_group` | VARCHAR | ✅ | the mode: `driving`, `walking`, `cycling` |
| `uses` | VARCHAR | ✅ | its use: `auto`, `walk`, `bike` |
| `description` | VARCHAR | ∅ | empty |

## node

| Column | Type | Spec | Holds |
|---|---|---|---|
| `node_id` | BIGINT | ✅ key | the OSM node id. Negative where a loop road is cut at its middle |
| `x_coord`, `y_coord` | DOUBLE | ✅ | longitude, latitude |
| `z_coord` | DOUBLE | ✅ | the height in metres, after [`duckosm elevation`](../guides/elevation.md) |
| `ctrl_type` | VARCHAR | ✅ | `signal` at traffic lights; else from the signs on the approaches: `stop_4_way`, `stop`, `stop_2_way`, `yield` |
| `node_type` | VARCHAR | ✅ | `turning_circle`, `mini_roundabout`, `motorway_junction` (the OSM value), `intersection` or `dead_end` |
| `zone_id` | BIGINT | ✅ | `1`, the [`zone`](#zone) |
| `name`, `parent_node_id` | | ∅ | empty |
| `geom` | GEOMETRY | ➕ | the point |

## link

| Column | Type | Spec | Holds |
|---|---|---|---|
| `link_id` | BIGINT | ✅ key | the `edge_id` |
| `name` | VARCHAR | ✅ | the street name |
| `from_node_id`, `to_node_id` | BIGINT | ✅ | the two [nodes](#node), in the direction of travel |
| `directed` | BOOLEAN | ✅ | `true` |
| `geometry_id` | BIGINT | ✅ | the `link_id`: the row of [`geometry`](#geometry) |
| `geometry` | VARCHAR | ✅ | the shape, WKT `LINESTRING` |
| `dir_flag` | INTEGER | ✅ | `1` |
| `length` | DOUBLE | ✅ | metres, the length of the shape |
| `facility_type` | VARCHAR | ✅ | the OSM `highway` value |
| `capacity` | DOUBLE | ✅ | passenger cars an hour for each lane, from the road class. Empty on links that cars cannot use |
| `free_speed` | FLOAT | ✅ | km/h: the `maxspeed` tag, else from the road class; walking 5, cycling 15 |
| `lanes` | INTEGER | ✅ | the motor-vehicle lanes in this direction, from the lane tags; 1 where OSM has no tag. Empty on links that cars cannot use |
| `bike_facility` | VARCHAR | ✅ | from OSM `cycleway`, in the words of the standard |
| `ped_facility` | VARCHAR | ✅ | from OSM `sidewalk`: `sidewalk`, `offstreet_path`, `none`, `unknown` |
| `allowed_uses` | VARCHAR | ✅ | the use of the mode; `bus` or `bus,bike` on a bus-only link |
| `grade` | DOUBLE | ✅ | the slope in percent, after `duckosm elevation`. Empty on bridges and in tunnels |
| `parent_link_id` | BIGINT | ✅ | on a sidewalk: the road link that it runs along |
| `parking`, `toll`, `jurisdiction`, `row_width` | | ∅ | empty |
| `osm_id` | BIGINT | ➕ | the OSM way |
| `edge_ref` | VARCHAR | ➕ | the readable id of the link, `<osm_id>#<n><f or r>` |
| `footway`, `crossing`, `crossing_markings` | VARCHAR | ➕ | the OSM tags of a footway or a crossing |
| `along_link_id`, `along_mode`, `along_gap_m`, `along_kind` | | ➕ | on a footpath: the road it runs along, its network, the distance, and `sidewalk` or `adjacent` |
| `bridge`, `tunnel`, `layer` | VARCHAR | ➕ | the OSM tags |
| `refs` | BIGINT[] | ➕ | the OSM node ids at each vertex of the shape, in its order. Copied from the source db's `edges.refs` |
| `z_vertices` | DOUBLE[] | ➕ | the ground height in metres at each vertex of the shape, in its order, after [`duckosm elevation`](../guides/elevation.md). Copied from `edges.z_vertices`. Empty on a bus-only link |
| `geom` | GEOMETRY | ➕ | the shape |

## geometry

| Column | Type | Spec | Holds |
|---|---|---|---|
| `geometry_id` | BIGINT | ✅ key | the `link_id` |
| `geometry` | VARCHAR | ✅ | WKT `LINESTRING` |
| `geom` | GEOMETRY | ➕ | the same shape |

## lane

A link has a lane row for each lane, for each turn pocket of `turn:lanes`, and for an on-road bike lane. So a link with
`lanes = 2` can have 3 lane rows.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `lane_id` | VARCHAR | ✅ key | `<link_id>_<lane_num>` |
| `link_id` | BIGINT | ✅ | the [`link`](#link) |
| `lane_num` | BIGINT | ✅ | 1 is the leftmost lane. A bike lane on the left of a one-way road is -1 |
| `allowed_uses` | VARCHAR | ✅ | `auto`; `bus` from `psv:lanes` or `bus:lanes`; `bike` from `bicycle:lanes` or an on-road bike lane |
| `width` | DOUBLE | ✅ | metres, from `width:lanes`; else from the road class; a bike lane 1.5 |
| `r_barrier`, `l_barrier` | VARCHAR | ∅ | empty |
| `turn` | VARCHAR | ➕ | the `turn:lanes` value of the lane |
| `geom` | GEOMETRY | ➕ | the center line of the lane, to the nodes of its link |
| `geom_cut` | GEOMETRY | ➕ | the center line of the lane to the edge of the junction, where the `lane_connector` paths start |

## movement

The legal turns, with the OSM turn restrictions. A U-turn is kept only at a dead end, or where it is the only way into
the other direction of the road.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `mvmt_id` | VARCHAR | ✅ key | `<ib_link_id>-<ob_link_id>` |
| `node_id` | BIGINT | ✅ | the junction of the turn |
| `ib_link_id`, `ob_link_id` | BIGINT | ✅ | the link in, the link out |
| `type` | VARCHAR | ✅ | `left`, `right`, `thru`, `uturn`, `merge`, `diverge` |
| `mvmt_code` | VARCHAR | ✅ | the direction and the turn, for example `NBL` (northbound, left). Empty on a U-turn |
| `start_ib_lane`, `end_ib_lane` | INTEGER | ✅ | the lanes in that the turn starts from |
| `start_ob_lane`, `end_ob_lane` | INTEGER | ✅ | the lanes out that the turn ends in |
| `allowed_uses` | VARCHAR | ✅ | the use of the mode; `bus` into or out of a bus-only link |
| `ctrl_type` | VARCHAR | ✅ | `signal`, `stop` or `yield`, from the signal or the sign on the link in |
| `geometry` | VARCHAR | ✅ | a short curve through the junction, WKT |
| `name`, `penalty`, `capacity` | | ∅ | empty |

The lanes of a turn come from SUMO's netconvert, for driving and cycling. A turn that gets no lanes is in
[`lane_check`](#lane_check).

## location

A row for each OSM node on a link with one of these tags: `highway` = `crossing`, `bus_stop`, `give_way`, `stop`,
`traffic_signals`, `toll_gantry`, `mini_roundabout`, `speed_camera`; `traffic_calming`; `amenity=parking_entrance`;
`railway=level_crossing`.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `loc_id` | VARCHAR | ✅ key | `<osm_id>_<link_id>` |
| `link_id` | BIGINT | ✅ | the link it is on |
| `ref_node_id` | BIGINT | ✅ | the `from_node_id` of the link |
| `lr` | DOUBLE | ✅ | metres from `ref_node_id` along the link |
| `x_coord`, `y_coord`, `z_coord` | DOUBLE | ✅ | the position, and the height after `duckosm elevation` |
| `loc_type` | VARCHAR | ✅ | the OSM value: `crossing`, `bus_stop`, `give_way`, … |
| `zone_id` | BIGINT | ✅ | `1` |
| `gtfs_stop_id` | VARCHAR | ✅ | the GTFS `stop_id`, on the rows of a GTFS feed |
| `osm_id`, `name` | | ➕ | the OSM node, the name |
| `geom` | GEOMETRY | ➕ | the point |

### Transit stops from a GTFS feed

`duckosm gmns monaco.duckdb --gtfs feed.zip` adds the stops of a GTFS feed as `location` rows. Each stop goes on the
nearest link of each mode within 30 m (`--gtfs-max-m`). In driving, only the stops that a bus calls at. An OSM
`bus_stop` within 25 m of a GTFS bus stop is the same stop, and the GTFS row replaces it.

## curb_seg

| Column | Type | Spec | Holds |
|---|---|---|---|
| `curb_seg_id` | VARCHAR | ✅ key | `<link_id>_<side>` |
| `link_id` | BIGINT | ✅ | the link |
| `ref_node_id` | BIGINT | ✅ | the `from_node_id` of the link |
| `start_lr`, `end_lr` | DOUBLE | ✅ | the whole link: 0 to its length |
| `regulation` | VARCHAR | ✅ | the OSM `parking:left` or `parking:right` value, for example `parallel` |
| `width` | INTEGER | ✅ | empty |

## signal_controller

OSM has no signal timing, so there are only the controllers.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `controller_id` | VARCHAR | ✅ key | `sig_<node_id>` |
| `node_id` | BIGINT | ➕ | the node with the signal |
| `control_type` | VARCHAR | ➕ | `signal` |

## zone

One zone, the area. Each node is in it, because tools such as Path4GMNS need a zone.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `zone_id` | BIGINT | ✅ key | `1` |
| `name` | VARCHAR | ✅ | the name of the boundary, else of the area |
| `boundary` | VARCHAR | ✅ | the outline, WKT. Empty when the database has no boundary |
| `super_zone` | VARCHAR | ∅ | empty |
| `geom` | GEOMETRY | ➕ | the outline |

## lane_connector

In `gmns_driving`: one row for each lane pair of a movement. A turn from lanes 1-2 into lanes 1-2 has 2 connectors.

| Column | Type | Holds |
|---|---|---|
| `connector_id` | VARCHAR | `<from_lane_id>><to_lane_id>` |
| `mvmt_id` | VARCHAR | the [movement](#movement) |
| `from_lane_id`, `to_lane_id` | VARCHAR | the two [lanes](#lane) |
| `width` | DOUBLE | metres |
| `geom` | GEOMETRY | the path through the junction, from SUMO's netconvert |

### Footway joins

When driving and walking are both exported, `gmns_walking.lane_connector` also has a short straight connector from the
end of each footway to the nearest road lane, where they meet at a node.

## chains

Driving only. One row for each chain: the links of one road between two real junctions, joined again over the footway
crossings. It comes from the Level 2 [`driving.chains`](../reference/database.md#chains), with three changes: a chain is cut
where the GMNS lanes change (a merged SUMO edge has the lanes of its last link); a link in no Level 2 chain (a bus-only
link) is a chain of one link; and a chain is cut at each node inside it where another chain ends, so the two directions of
a two-way road end at the same nodes. Each driving link is in one chain. A source db with no `driving.chains` (an old build, or
`--no-chains`) gives one chain for each link, so `--merge` changes nothing. `duckosm sumo --merge` and `duckosm matsim --merge` read it.

| Column | Type | Holds |
|---|---|---|
| `chain_id` | BIGINT | the `link_id` for a chain of one link, else a hash of `link_ids`. A chain that is not cut has its Level 2 `chain_id` |
| `from_node_id`, `to_node_id` | BIGINT | the first node and the last node |
| `link_ids` | BIGINT[] | the links, in driving order |
| `length` | DOUBLE | metres, the sum of the link lengths |
| `geom` | GEOMETRY | the link lines joined into one line |

## elevation_metadata

In the `main` schema, when the source db has heights: a copy of the source db's
[`main.elevation_metadata`](../reference/database.md#elevation_metadata). It names the
[ground of the area](../concepts/ground-reference.md) that `node.z_coord`, `link.grade` and `link.z_vertices` come
from: `sha256` is the ground id, with `product` and `vertical_datum`.

## lane_check

What the lanes cannot get, in place of a guess. In `gmns_driving` and `gmns_cycling`, the table exists only when netconvert ran. `gmns_walking` always has it. Most rows are OSM data to check, in
[Fix the OSM data](../guides/edit-osm-data.md).

| Column | Type | Holds |
|---|---|---|
| `kind` | VARCHAR | `no way out`, `no way in`, `no lane connection`, `lane count`, `against its arrow`, `arrows not matched` |
| `id` | VARCHAR | the `lane_id`, `mvmt_id` or `link_id` |
| `detail` | VARCHAR | what was found |

## crossing

In `gmns_driving`: one pedestrian crossing painted on the road, from the OSM `footway=crossing` ways and the
`highway=crossing` nodes.

| Column | Type | Holds |
|---|---|---|
| `crossing_id` | VARCHAR | `w<osm_id>` (a way) or `n<osm_id>` (a node) |
| `source` | VARCHAR | `way` or `node` |
| `crossing_type`, `markings` | VARCHAR | the OSM tags `crossing` and `crossing:markings` |
| `painted` | BOOLEAN | `false` for an unmarked crossing |
| `width`, `length` | DOUBLE | metres, along the road and across it |
| `osm_id` | BIGINT | the OSM way or node |
| `geom` | GEOMETRY | the rectangle of the crossing |

## lane_crossing

In `gmns_driving`: one row for each crossing and each driving lane that it covers, so a crossing can be painted lane by
lane.

| Column | Type | Holds |
|---|---|---|
| `crossing_id` | VARCHAR | the [crossing](#crossing) |
| `lane_id` | VARCHAR | the lane, or the `connector_id` of a lane connector |
| `link_id` | BIGINT | the link |
| `start_lr`, `end_lr` | DOUBLE | the part of the lane that the crossing covers, metres from its start |
| `across_from`, `across_to`, `to_left` | | the position of that part across the crossing, to paint the stripes |

## link_along

In `gmns_walking`: one row for each footpath and each road that it runs along for 4 m or more.

| Column | Type | Holds |
|---|---|---|
| `link_id` | BIGINT | the footpath link |
| `along_link_id`, `along_mode` | | the road link and its network |
| `covered_m` | DOUBLE | the metres of the footpath along this road |
| `gap_m` | DOUBLE | the distance between the two lines |

## What is not written, and why

The standard has 25 tables. duckOSM writes 12 of them.

| Tables | Why not |
|---|---|
| `segment`, `segment_lane` | an edge is cut where the lane count changes, so no segment is needed |
| the 6 time-of-day tables (`*_tod`, `time_set_definitions`) | few OSM ways have `*:conditional` tags, and most are parking rules |
| the 5 signal timing tables | OSM has no signal timing, phases or detectors |

## gmns_all: all modes in one

`--combined` adds the schema `gmns_all`, with `config`, `node`, `link`, `use_definition` and `use_group`. A road that
several modes use is one link, with all of them in `allowed_uses` (`auto,bike,walk`). The lanes and the movements
differ by mode, so they stay in the schema of each mode.

The tables `meso_*` and `micro_*`: [GMNS meso and micro](gmns.md#meso-and-micro-networks).
