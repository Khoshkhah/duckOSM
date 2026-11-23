# HD map database

!!! warning "Planned"
    This page is the specification of the HD map database. The code does not exist yet. The reasons for each decision
    are in [`docs/design/hdmap_duckdb.md`](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/hdmap_duckdb.md).

The HD map database (`<area>_hdmap.duckdb`) is the lane-level road model of the driving network of an area: the roads
with their reference lines, the lane sections, the lanes with their borders in 3D, the paths through each junction,
the stop lines, the crosswalks, the signals and the signs. Each value has its evidence: from the data, computed, or a
default.

One database gives each HD map format: Lanelet2, ASAM OpenDRIVE and CommonRoad, and later SUMO and glTF. Each exporter
reads only this database.

The tables are the core of the SimReady road model, with a small number of additions for the formats. Two programs can
write the database:

| Producer | Input | Quality |
|---|---|---|
| `duckosm hdmap` | the routing database (chains, heights) and its [GMNS database](../exports/gmns_tables.md) (lanes, junction paths) | OSM center lines offset by the lane widths, junction paths from SUMO's netconvert, heights from the ground |
| SimReady | its own build | measured kerbs, lane tapers, its own junction paths and areas, measured heights |

```bash
duckosm build --config config/sample_monaco.yaml              # Level 2: monaco.duckdb (with driving.chains)
duckosm gmns monaco.duckdb                                    # Level 3: monaco_gmns.duckdb (the lanes)
duckosm hdmap monaco_gmns.duckdb --source monaco.duckdb       # Level 4: monaco_hdmap.duckdb
duckosm lanelet2 monaco_hdmap.duckdb                          # a format from it
```

```python
from duckosm import hdmap
hdmap.build("monaco_gmns.duckdb", source="monaco.duckdb")     # -> monaco_hdmap.duckdb
hdmap.to_lanelet2("monaco_hdmap.duckdb", "monaco.osm")
```

## The command

`duckosm hdmap GMNS_DB --source DB [OPTIONS]`

| Option | Default | Effect |
|---|---|---|
| `--source PATH` | required | the routing database (Level 2) of the GMNS database: `driving.chains`, the edges, the heights and the ground (`main.elevation_metadata`) |
| `--out PATH` | `<area>_hdmap.duckdb`, next to the GMNS database | the HD map database. A run writes the whole file again |
| `--crs CRS` | the UTM zone of the center of the area | the metric CRS of all geometry |
| `--default-width M` | `3.25` | the width of a lane that has no width in the data |
| `--clearance M` | `5.0` | the free height between a bridge deck and the road under it, and between a tunnel road and the road over it |

The step reads the driving network only. It opens the two databases read-only and does not change them. The routing
database must have `driving.chains` (a build with this duckOSM version).

## Rules

| Rule | |
|---|---|
| File | one DuckDB file, complete without the other databases |
| Schema | all tables are in the schema `hdmap` |
| Mode | `driving` only |
| Roads | one carriageway for each chain of `driving.chains`: the edges joined where the road only continues. So a road is not cut at a crosswalk or at a node where only a footway meets it. A road is cut at a junction, at a traffic light, at a stop sign, at a turn rule and where its lanes change |
| Coordinates | metres, in `project.crs` (a UTM zone). The Lanelet2 exporter transforms the points to longitude and latitude |
| Heights | metres on `project.vertical_datum`, from the [ground of the area](../concepts/ground-reference.md) |
| Units in the names | `_m` metres, `_m2` square metres, `_s` seconds, `_kmh` km/h, `_deg` degrees. No suffix: no unit |
| Ids | `BIGINT`. Text keys go through `(hash(<text>) >> 1)::BIGINT`, the formula of the duckOSM `edge_id`; a key of several values is joined by `\|`, a list by `,`. Two ids come from duckOSM as they are: `node_id` (the OSM node id) and `carriageway_id` (the `chain_id`). No id comes from a sequence, a row number or a rank of computed numbers |
| Stable ids | the same input gives the same ids, on each machine. A SUMO `--merge` edge and a carriageway of the same chain have the same id. Ids from two producers are not the same: join them on `carriageway.edge_ids` |
| Stations | metres along the reference line of a carriageway, from its start |
| Left and right | in the direction of travel of the carriageway |
| Lane numbers | lane 1 is the leftmost lane; the numbers go up to the right with no gap. Bike and bus lanes on the road are lanes of the cross-section and have numbers |
| Lists, enums | DuckDB lists (`BIGINT[]`, `VARCHAR[]`). An enum is a `VARCHAR` with a `CHECK` of its values |
| Geometry | DuckDB `GEOMETRY`. The type (POINT, LINESTRING Z, POLYGON) is in the column description |

### Evidence

Each row of a table marked "ev" has five evidence columns, for the values that the producer computed for the row.

| Column | Type | Holds |
|---|---|---|
| `ev_status` | VARCHAR | `observed`: in the source data (an OSM tag, a survey). `derived`: computed from observed values with no choice (a length from a line). `inferred`: chosen by a rule among options; `ev_confidence` has a value. `assumed`: a default for a value that is not in the data (a lane width of 3.25 m). `unknown`: no value. `conflict`: two sources do not agree |
| `ev_method` | VARCHAR | the rule or the step that made the value, for example `osm_tag`, `default_width`, `netconvert`, `offset` |
| `ev_confidence` | DOUBLE | 0 to 1, for `inferred` and `conflict` only |
| `ev_source_ids` | VARCHAR[] | `"{source_id}:{object}"`, for example `osm-2026-11-20:w6110234` |
| `ev_review` | VARCHAR | `none`, `needed`, `accepted`, `rejected` |

An exporter does not show an `assumed` value as an `observed` value. Where the format has a place for it, the exporter
writes the status (Lanelet2: a tag `ev_status`; OpenDRIVE: `userData`).

## The tables

The column "SimReady" says how the table differs from the SimReady model. The design lists each change.

| Table | One row is | ev | SimReady | `duckosm hdmap` fills it |
|---|---|---|---|---|
| [`project`](#project) | the database: area, CRS, ground, producer | | changed | yes |
| [`source`](#source) | a data source: an OSM extract, a DEM, a survey | | same | yes |
| [`node`](#node) | a network node at an end of a carriageway | ev | same | yes |
| [`carriageway`](#carriageway) | one chain: one direction of a road between two cuts, with its reference line | ev | changed | yes |
| [`lane_section`](#lane_section) | a part of a carriageway with a fixed set of lanes | ev | changed | yes |
| [`lane`](#lane) | one lane of a section, with its two borders | ev | changed | yes |
| [`junction`](#junction) | one junction: one node or a group of nodes | ev | changed | yes |
| [`turn_rule`](#turn_rule) | a turn from one carriageway to another at a junction | ev | changed | yes |
| [`movement`](#movement) | a path from one lane to another through a junction | ev | changed | yes |
| [`stop_line`](#stop_line) | a line across the lanes where they stop | ev | changed | yes |
| [`island`](#island) | a traffic island in a junction | ev | changed | no: OSM has no islands in the GMNS data |
| [`crossing`](#crossing) | a pedestrian crossing on the road | ev | **new** | yes |
| [`signal`](#signal) | a traffic light, or a traffic sign, at its position | ev | **new** | yes |
| [`signal_plan`](#signal_plan), [`signal_phase`](#signal_phase) | the timing of the traffic lights of a junction | ev | same | no: OSM has no signal timing |
| [`elevation_profile`](#elevation_profile) | a height at a station of a carriageway | ev | changed | yes, when the routing database has heights |
| [`structure`](#structure) | a bridge or a tunnel on a carriageway | ev | changed | yes |

```mermaid
erDiagram
    carriageway ||--o{ lane_section : "divided into"
    lane_section ||--o{ lane : holds
    lane ||--o| lane : succeeds
    carriageway ||--o{ elevation_profile : "heights"
    carriageway ||--o{ structure : carries
    carriageway ||--o{ stop_line : "stops at"
    junction ||--o{ stop_line : bounds
    junction ||--o{ turn_rule : decides
    turn_rule ||--o{ movement : allows
    movement }o--|| lane : from
    movement }o--|| lane : to
    junction ||--o{ island : holds
    junction ||--o| signal_plan : "controlled by"
    signal_plan ||--o{ signal_phase : has
    signal_phase }o--o{ movement : serves
    signal }o--o| stop_line : controls
    crossing }o--o{ lane : covers
    node }o--o| junction : "belongs to"
```

## project

One row.

| Column | Type | Holds |
|---|---|---|
| `project_id` | BIGINT | `hash("{area_name}\|{built_at}")`, as in SimReady |
| `area_name` | VARCHAR | for example `monaco` |
| `crs` | VARCHAR | the metric CRS, for example `EPSG:32632` |
| `vertical_datum` | VARCHAR | for example `EGM2008`. Empty when the area has no heights |
| `ground_id` | VARCHAR | **new.** The `sha256` of the ground (`main.elevation_metadata` of the routing database) |
| `ground_product` | VARCHAR | **new.** `DTM` or `DSM` |
| `producer` | VARCHAR | **new.** `duckosm-gmns` or `simready` |
| `boundary` | GEOMETRY | POLYGON. The area |
| `built_at` | TIMESTAMP | |
| `duckosm_db` | VARCHAR | the path of the routing database |
| `duckosm_version` | VARCHAR | |
| `simready_version` | VARCHAR | empty for `duckosm-gmns` |
| `jurisdiction` | VARCHAR | the country code of the traffic rules and signs, for example `MC`, `SE` |
| `driving_side` | VARCHAR | `right` or `left` |
| `osm_timestamp` | TIMESTAMP | the date of the OSM data |

## source

The columns of SimReady.

| Column | Type | Holds |
|---|---|---|
| `source_id` | VARCHAR | a short name, for example `osm-2026-11-20`, `fabdem-v1-2`, `netconvert` |
| `kind` | VARCHAR | `osm`, `city_gis`, `dem`, `lidar`, `imagery_aerial`, `imagery_street`, `survey`, `sensor`, `model`, `derived`, … |
| `title`, `provider` | VARCHAR | |
| `acquired_at` | DATE | |
| `imported_at` | TIMESTAMP | |
| `crs_native` | VARCHAR | |
| `coverage` | GEOMETRY | POLYGON |
| `licence` | VARCHAR | for example `ODbL-1.0` |
| `vertical_datum`, `geoid_grid` | VARCHAR | for a height source |
| `vertical_accuracy_m` | DOUBLE | |
| `reference`, `note` | VARCHAR | |

## node

One row for each node at an end of a carriageway.

| Column | Type | Holds |
|---|---|---|
| `node_id` | BIGINT | the OSM node id, as in duckOSM. Negative for a virtual node |
| `xy` | GEOMETRY | POINT |
| `kind` | VARCHAR | `priority`, `signal`, `stop`, `yield`, `roundabout`, `right_before_left`, `uncontrolled`, `dead_end`, `connector_end` |
| `degree_in`, `degree_out` | INTEGER | the carriageways in and out |
| `junction_id` | BIGINT | the junction of the node, or empty |
| `highway_tag` | VARCHAR | the OSM `highway` value of the node (`traffic_signals`, `stop`, …) |
| `level` | INTEGER | the vertical level: `layer`, else `bridge` 1, `tunnel` -1, else 0 |

## carriageway

One row for each chain of `driving.chains`.

| Column | Type | Holds |
|---|---|---|
| `carriageway_id` | BIGINT | the `chain_id` of the chain: the `edge_id` for a chain of one edge, else duckOSM's hash of its `edge_ids` |
| `edge_ids` | BIGINT[] | the duckOSM `edge_id`s, in travel order |
| `edge_start_m` | DOUBLE[] | **new.** The station where each edge of `edge_ids` starts. A value on one edge maps to the stations from `edge_start_m[i]` to `edge_start_m[i + 1]` |
| `mode` | VARCHAR | `driving` |
| `from_node_id`, `to_node_id` | BIGINT | the `source` and `target` of the chain |
| `shared_line` | BOOLEAN | true when the two directions of one OSM way share one reference line |
| `twin_carriageway_id` | BIGINT | the carriageway of the other direction of the same road, or empty |
| `lane_offset_m` | DOUBLE | **new.** The distance from the reference line to the left border of lane 1, to the right is positive. OpenDRIVE `laneOffset` |
| `highway` | VARCHAR | the OSM road class |
| `name` | VARCHAR | the street name |
| `oneway` | BOOLEAN | |
| `inside_junction_id` | BIGINT | set when the carriageway lies inside a junction |
| `posted_speed_kmh` | DOUBLE | the speed limit |
| `reference_line` | GEOMETRY | LINESTRING. The chain line (`chains.geometry`) in `project.crs`, in the direction of travel |
| `length_m` | DOUBLE | |
| `level` | INTEGER | as on `node` |
| `lane_order_convention` | VARCHAR | `left_to_right_from_1` |

## lane_section

A part of a carriageway where the set of lanes does not change. The sections of a carriageway cover it from station 0 to
`length_m` with no gap.

| Column | Type | Holds |
|---|---|---|
| `lane_section_id` | BIGINT | `hash("{carriageway_id}\|{rank}")`, `rank` the section number from 1, in station order |
| `carriageway_id` | BIGINT | |
| `station_start_m`, `station_end_m`, `length_m` | DOUBLE | |
| `lane_count_start`, `lane_count_end` | INTEGER | the driving lanes at the start and the end |
| `level` | INTEGER | |
| `structure_id` | BIGINT | the bridge or the tunnel of the section, or empty |

A chain has the same lanes on all its edges, so `duckosm hdmap` writes one section for each carriageway. SimReady can
write more.

## lane

| Column | Type | Holds |
|---|---|---|
| `lane_id` | BIGINT | `hash("{lane_section_id}\|{lane_num}")` |
| `lane_section_id`, `carriageway_id` | BIGINT | |
| `lane_num` | INTEGER | 1 is the leftmost lane |
| `role` | VARCHAR | `through`, `pocket` (a turn bay that opens), `merge` (a lane that ends), `parking`, `bike`, `bus`, `shoulder`, `median` |
| `use` | VARCHAR[] | who can drive on it: `auto`, `bus`, `bike`, `hgv`, `taxi`. Empty for `parking`, `shoulder`, `median` |
| `turn` | VARCHAR | the `turn:lanes` value at the end of the section (`left`, `through`, `left;through`, …) |
| `width_shape` | VARCHAR | `constant`, `linear`, `smoothstep` |
| `width_start_m`, `width_end_m` | DOUBLE | the width at the start and at the end of the section |
| `taper_start_m`, `taper_length_m` | DOUBLE | where the width starts to change, and over which length. Empty for `constant` |
| `boundary_left`, `boundary_right` | GEOMETRY | LINESTRING Z. The two borders, from the start (or `cut_start_m`) to the end (or `cut_end_m`) |
| `mark_left`, `mark_right` | VARCHAR | **new.** The road mark on the border: `solid`, `dashed`, `solid_solid`, `solid_dashed`, `dashed_solid`, `curb`, `none` |
| `mark_source` | VARCHAR | **new.** `osm` (from `change:lanes` or `lane_markings`) or `rule` (see [Road marks](#road-marks)) |
| `predecessor_lane_id`, `successor_lane_id` | BIGINT | the lane of the section before and after, on the same carriageway. Empty at the end of a carriageway: the next lane is then through a `movement` |
| `cut_start_m`, `cut_end_m` | DOUBLE | the station where the lane meets the junction. Two lanes of one section can have different cuts |

Views, not stored: `centerline` (the middle of the two borders, LINESTRING Z) and `area` (POLYGON Z, the lane between
its borders).

### Road marks

The first rule that applies, when OSM has no mark:

| Border | Mark |
|---|---|
| the right border of the rightmost lane | `curb` |
| the left border of lane 1 when the carriageway has a twin (the center line of a two-way road) | `solid` |
| the left border of lane 1 of a one-way road | `curb` |
| between two lanes with a different `use` | `solid` |
| between two lanes with the same `use` | `dashed` |

The lane on the left and the lane on the right of one border have the same mark on it.

## junction

| Column | Type | Holds |
|---|---|---|
| `junction_id` | BIGINT | `hash("{node_ids sorted, joined by ','}")` |
| `node_ids` | BIGINT[] | one node, or the nodes of a junction made of more than one node |
| `kind` | VARCHAR | `simple` (one node), `cluster` (more than one node), `roundabout_node` |
| `control` | VARCHAR | `signal`, `stop_all`, `stop`, `yield`, `priority`, `none` |
| `core` | GEOMETRY | POINT. The center |
| `arm_count` | INTEGER | the carriageways that touch it |
| `arm_order` | BIGINT[] | the carriageways, counter-clockwise |
| `shape` | GEOMETRY | POLYGON. The outline that the stop lines lie on |
| `area` | GEOMETRY | POLYGON. The road surface of the junction, without the islands |
| `area_m2` | DOUBLE | |
| `signal_plan_id` | BIGINT | or empty |
| `level` | INTEGER | |

## turn_rule

One row for each pair of a carriageway in and a carriageway out at a junction.

| Column | Type | Holds |
|---|---|---|
| `turn_rule_id` | BIGINT | `hash("{from_carriageway_id}\|{to_carriageway_id}")` |
| `junction_id`, `from_carriageway_id`, `to_carriageway_id` | BIGINT | |
| `turn` | VARCHAR | `left`, `straight`, `right`, `u`, `slight_left`, `slight_right` |
| `fork` | BOOLEAN | true for a merge or a diverge |
| `allowed` | BOOLEAN | false when a turn restriction stops the turn |
| `allowed_uses` | VARCHAR[] | |
| `method` | VARCHAR | `osm_restriction`, `turn_lanes`, `geometry`, `uturn_rule` |
| `source_ref` | VARCHAR | the OSM relation of a restriction |

## movement

A path from the end of one lane to the start of another, through a junction.

| Column | Type | Holds |
|---|---|---|
| `movement_id` | BIGINT | `hash("{from_lane_id}\|{to_lane_id}")` |
| `junction_id`, `turn_rule_id` | BIGINT | |
| `from_lane_id`, `to_lane_id` | BIGINT | |
| `from_carriageway_id`, `to_carriageway_id` | BIGINT | |
| `turn` | VARCHAR | as in `turn_rule` |
| `fork` | BOOLEAN | |
| `priority` | VARCHAR | `major`, `minor`, `yield`, `stop`, `signal` |
| `yield_to_movement_ids` | BIGINT[] | **new.** The movements that this movement must let go first. See [Right of way](#right-of-way) |
| `assignment_method` | VARCHAR | how the lane pair was chosen: `turn_lanes`, `connectivity_rule`, `netconvert` |
| `path` | GEOMETRY | LINESTRING Z. From the center of the lane in at its cut to the center of the lane out at its cut |
| `boundary_left`, `boundary_right` | GEOMETRY | **new.** LINESTRING Z. The two borders of the path. Each starts at the end point of the same border of the lane in and ends at the start point of the same border of the lane out |
| `length_m`, `width_m` | DOUBLE | |
| `area` | GEOMETRY | POLYGON. The area between the two borders |
| `curb_radius_m` | DOUBLE | the radius of a right turn |
| `allowed_uses` | VARCHAR[] | |
| `conflict_movement_ids` | BIGINT[] | the movements of the same junction whose area crosses this one |
| `signal_phase_ids` | BIGINT[] | the phases that give this movement green |

### Right of way

A movement yields to a movement of `conflict_movement_ids` when:

1. the other movement has a higher `priority` (`major` before `minor`, `minor` before `yield`, `yield` before `stop`), or
2. the two have the same `priority`, and this movement turns left (or makes a U-turn) across the other, which goes
   straight or turns right from the opposite arm.

Two `signal` movements of different phases do not yield to each other. When the phases are not known (no
`signal_plan`), the rule 2 applies to them too.

## stop_line

| Column | Type | Holds |
|---|---|---|
| `stop_line_id` | BIGINT | `hash("{carriageway_id}\|{junction_id or station_m to the cm}\|{end}")` |
| `carriageway_id` | BIGINT | |
| `junction_id` | BIGINT | the junction, or empty for a line in the middle of a carriageway (a pedestrian light) |
| `end` | VARCHAR | `in`: the carriageway goes into the junction here. `out`: it comes out of the junction here. `mid`: a line in the middle of the carriageway |
| `station_m` | DOUBLE | the mean of the cut stations of the lanes |
| `geometry` | GEOMETRY | LINESTRING. From the left border of lane 1 to the right border of the last lane, through the cut of each lane |
| `lane_cuts` | STRUCT(lane_id BIGINT, station_m DOUBLE, point GEOMETRY)[] | the cut of each lane, from left to right |
| `painted` | BOOLEAN | **new.** True when a line is painted on the road: an `in` or a `mid` line at a traffic light, a stop sign or a give-way sign. An `out` line is never painted |
| `method` | VARCHAR | `lane_border`, `crossing`, `stop_node`, `measured`, `setback`, `netconvert` |
| `source_ref` | VARCHAR | the object that placed the line |

## island

| Column | Type | Holds |
|---|---|---|
| `island_id` | BIGINT | `hash("{junction_id}\|{rank}")`, `rank` counter-clockwise from the core |
| `junction_id` | BIGINT | |
| `kind` | VARCHAR | `channelizing`, `refuge`, `splitter`, `central`, `median_end`, `remnant` |
| `level` | INTEGER | |
| `area` | GEOMETRY | POLYGON |
| `area_m2` | DOUBLE | |
| `source_ref` | VARCHAR | the OSM way of the island, when it is in OSM |

## crossing

**New.** A pedestrian crossing on the road.

| Column | Type | Holds |
|---|---|---|
| `crossing_id` | BIGINT | `hash("{GMNS crossing_id}")`, for example `hash("w1202361402#2")` |
| `osm_id` | BIGINT | the OSM way or node |
| `junction_id` | BIGINT | the junction that it is in or next to, or empty |
| `carriageway_ids` | BIGINT[] | the carriageways that it goes across |
| `lane_ids` | BIGINT[] | the lanes that it covers |
| `kind` | VARCHAR | the OSM `crossing` value: `zebra`, `traffic_signals`, `uncontrolled`, `unmarked`, … |
| `painted` | BOOLEAN | false for `unmarked` |
| `width_m`, `length_m` | DOUBLE | along the road, and across the road |
| `area` | GEOMETRY | POLYGON Z. The rectangle on the road |

## signal

**New.** A traffic light or a traffic sign as a thing at a position. The rules (who yields to whom) are in `movement`;
the timing is in `signal_plan`.

| Column | Type | Holds |
|---|---|---|
| `signal_id` | BIGINT | from OSM: `hash("{kind}\|{osm_id}\|{carriageway_id}")`. From a movement rule with no OSM node: `hash("{kind}\|{carriageway_id}\|end")` |
| `kind` | VARCHAR | `traffic_light`, `stop`, `yield`, `speed_limit`, `no_entry` |
| `code` | VARCHAR | the sign code of the `project.jurisdiction`, when it is known |
| `value` | DOUBLE | the number on the sign, for example 50 for `speed_limit` |
| `carriageway_id` | BIGINT | the carriageway that it applies to |
| `station_m` | DOUBLE | along the carriageway |
| `side` | VARCHAR | `right`, `left`, `above` |
| `heading_deg` | DOUBLE | **new.** The direction that the face of the light or the sign looks to, from north, clockwise: against the direction of travel |
| `height_m` | DOUBLE | **new.** The height of the light or the sign above the road: 3.0 m for a light, 2.0 m for a sign, when it is not known |
| `stop_line_id` | BIGINT | the stop line that it controls, or empty |
| `lane_ids` | BIGINT[] | the lanes that must obey it; empty: all lanes of the carriageway |
| `osm_id` | BIGINT | the OSM node, when it is in OSM |
| `position` | GEOMETRY | POINT Z. At the side of the road (`side`), at `height_m` above it |

## signal_plan

| Column | Type | Holds |
|---|---|---|
| `signal_plan_id` | BIGINT | `hash("{junction_id}\|{source_id}")` |
| `junction_id` | BIGINT | |
| `controller_type` | VARCHAR | `fixed_time`, `actuated`, `coordinated`, `unknown` |
| `cycle_s`, `offset_s` | DOUBLE | |
| `source_id` | VARCHAR | |
| `method` | VARCHAR | `measured`, `city`, `default` |
| `source_ref` | VARCHAR | |

## signal_phase

| Column | Type | Holds |
|---|---|---|
| `signal_phase_id` | BIGINT | `hash("{signal_plan_id}\|{phase_num}")` |
| `signal_plan_id` | BIGINT | |
| `phase_num` | INTEGER | |
| `green_s`, `yellow_s`, `red_clearance_s` | DOUBLE | |
| `movement_ids` | BIGINT[] | the movements with green in this phase |
| `protected` | BOOLEAN | |
| `ring` | INTEGER | |

## elevation_profile

| Column | Type | Holds |
|---|---|---|
| `elevation_profile_id` | BIGINT | `hash("{carriageway_id}\|{source_id}\|{method}\|{rank}")`, `rank` the row number of this source and method on the carriageway, in station order |
| `carriageway_id` | BIGINT | |
| `station_m` | DOUBLE | |
| `z_m` | DOUBLE | the height of the road on `project.vertical_datum` |
| `grade_pct` | DOUBLE | |
| `earthwork_m` | DOUBLE | the road height minus the ground height |
| `outlier` | BOOLEAN | |
| `source_id` | VARCHAR | the ground, or a bridge profile |
| `method` | VARCHAR | `dem_node`, `dem_sample`, `structure_line`, `bridge_profile`, `lidar_surface`, `survey_point`, `graded`, … |

## structure

| Column | Type | Holds |
|---|---|---|
| `structure_id` | BIGINT | `hash("{carriageway_id}\|{rank}")`, `rank` in station order |
| `carriageway_id` | BIGINT | |
| `station_start_m`, `station_end_m` | DOUBLE | |
| `kind` | VARCHAR | `bridge`, `viaduct`, `tunnel`, `building_passage`, `culvert`, `covered`, `embankment`, `cutting` |
| `level` | INTEGER | |
| `clearance_m`, `width_m` | DOUBLE | |
| `twin_structure_id` | BIGINT | the structure of the other direction |
| `source_ref` | VARCHAR | the OSM way |

## How `duckosm hdmap` fills the tables

| Table | From | ev_status |
|---|---|---|
| `carriageway` | one row for each row of `driving.chains`. `reference_line`: `chains.geometry` in `project.crs`. `edge_start_m`: the sum of the edge lengths before each edge. `twin_carriageway_id`, `shared_line`: the chain of the other direction of the same OSM way, when its `edge_ids` are the same edges in the other order. `lane_offset_m`: the distance from the reference line to the left border of lane 1 | `observed`; `lane_offset_m` `derived` |
| `node` | the `source` and `target` nodes of the chains, with `gmns_driving.node` (`ctrl_type` → `kind`) | `observed` |
| `lane_section` | one section for each carriageway, from 0 to `length_m` | `derived` |
| `lane` | the GMNS lanes of the links of the chain. A chain has the same lanes on each link, so lane `k` of each link joins to one lane. `lane_num`: 1 to n, left to right, in the order of the GMNS `lane_num` (the GMNS lane -1, a bike lane on the left, becomes lane 1). `role`: `bike` or `bus` from `allowed_uses`, else `through`. `width_shape`: `constant`. The center line: the GMNS `geom` of each link joined, then cut at the start of the `geom_cut` of the first link and at the end of the `geom_cut` of the last link. Borders: the center line offset by half the width (shapely `offset_curve`, one call for all lanes); two lanes side by side use one line for their shared border | width: `observed` (`width:lanes`) or `assumed`; borders `derived` |
| `junction` | one row for each node at an end of a carriageway with GMNS lane connectors. `kind`: `simple`. `control`: from `node.ctrl_type`. `area`: the union of the movement areas | `derived` |
| `turn_rule` | one row for each distinct pair of `ib_link_id`, `ob_link_id` in GMNS `movement` at a junction, as a pair of carriageways. `turn` from `movement.type`. All are `allowed` | `observed` (restrictions) |
| `movement` | one row for each GMNS `lane_connector` at a junction. The connectors at nodes inside a chain are not used. `path`: `lane_connector.geom`. Borders: the path offset by half its width, with the first and the last point set to the end points of the lane borders. `priority`: from `movement.ctrl_type` and the road class. `conflict_movement_ids`: the movements of the junction whose areas cross | `inferred`, method `netconvert` |
| `stop_line` | an `in` and an `out` row for each carriageway end at a junction, through the cuts of its lanes. A `mid` row at each traffic light in the middle of a carriageway, at its station | `derived` |
| `island`, `signal_plan`, `signal_phase` | none | |
| `crossing` | the GMNS `crossing` and `lane_crossing` tables | `observed` |
| `signal` | GMNS `location` with `loc_type` `traffic_signals`, `stop`, `give_way`; the `ctrl_type` of the movements with no `location` row | `observed` or `inferred` |
| `elevation_profile` | on the ground: one row at each vertex of each edge of the chain, from `edges.z_vertices` (else `z_from` and `z_to`), method `dem_sample`. On a bridge or in a tunnel: the rows of [Bridges and tunnels](#bridges-and-tunnels) | `derived`; `assumed` for `structure_line` |
| `structure` | one row for each run of edges of the chain with `bridge` or `tunnel`, or with a `layer` other than 0 | `observed` |

The Z of the borders and the paths: linear between the `elevation_profile` rows of the carriageway. A movement takes
the Z of the end of its lane in and of the start of its lane out, linear between them. When the routing database has no
heights (no `duckosm elevation`), Z is 0, `project.vertical_datum` is empty, and `elevation_profile` has no rows.

### Bridges and tunnels

The ground (DTM) is not the road on a bridge or in a tunnel. Under a bridge it is the valley, and over a tunnel it is
the hill. So `duckosm hdmap` does not use the ground there. It uses the rule of SimReady (`import-elevation`, steps 3b
and 4), without the crest and the vertical curves:

1. **Spans.** A span is a run of edges with `level <> 0`: a `bridge` or `tunnel` tag, or a `layer` other than 0. It
   gets a `structure` row.
2. **Chains of spans.** Spans that share a node are one chain of spans, bridges and tunnels together.
3. **Landings.** A landing is a node of the chain of spans that touches a carriageway part with `level = 0`. It takes
   the height of the ground at its node.
4. **Nodes inside the chain of spans.** Their heights come from one linear system: each node is the mean of its
   neighbours in the chain, with the weight `1 / length` of the span between them, and the landings are fixed. The
   step solves it directly (no loop). The result is a straight line along the chain between the landings.
5. **Clearance.** Where a bridge span goes over a carriageway of a lower level, the deck there is at least
   `z_road + clearance × (level difference)`, the lower roads first. Where a tunnel span goes under a carriageway of a
   higher level, the tunnel road there is at most `z_road − clearance × (level difference)`, the upper roads first.
   The span becomes the upper hull (a tunnel: the lower hull) of its ends and these points. A grade steeper than 8 %
   is not made: the span keeps the 8 % grade, and its rows get `ev_review = needed`.
6. **Rows.** Each height of steps 3 to 5 is an `elevation_profile` row with method `structure_line` and
   `ev_status = assumed`.

A chain of spans with no landing (its ends are all outside the area) keeps the ground heights, method `dem_sample`,
with `ev_review = needed`. A short span has no special rule.

A better height source (LiDAR, a survey, a measured bridge profile) is not in the GMNS data. The SimReady producer uses
one when it has one, and the same tables hold it.

## What each format reads

### Lanelet2

| Lanelet2 | From |
|---|---|
| node | each point of each border, in longitude and latitude, with `ele` from Z. Points within 1 cm are one node; no two points of a way are closer than 5 cm |
| way `line_thin` / `curbstone` / `virtual` | `lane.boundary_left/right` with `mark_*`: `solid`, `dashed` → `line_thin` with that `subtype`; `curb` → `curbstone`; `none` → `virtual`. The borders of a movement → `virtual` |
| relation `type=lanelet` | each `lane` (`subtype` from `role`, `speed_limit` from `posted_speed_kmh`) and each `movement` (`turn_direction` from `turn`). A lane or a movement with a border shorter than 5 cm gets no lanelet |
| way `stop_line` | each `stop_line` with `painted` |
| lanelet `subtype=crosswalk` | each `crossing` |
| `regulatory_element` `traffic_light` | each `signal` of kind `traffic_light`: `refers` a way of 0.5 m at its `position` and `height_m`, square to `heading_deg`; `ref_line` its stop line; members the lanelets of `lane_ids` |
| `regulatory_element` `right_of_way` | each junction: `right_of_way` the movements that others yield to, `yield` the movements with `yield_to_movement_ids` |
| `regulatory_element` `traffic_sign` | `signal` of kind `stop`, `yield`, `speed_limit` |
| map tag `duckosm:ground` | `project.ground_id` |

### ASAM OpenDRIVE 1.7

| OpenDRIVE | From |
|---|---|
| `road` | each `carriageway`; two twins with `shared_line` are one road with left and right lanes. `id`: `carriageway_id` |
| `planView` | `reference_line`, as `line` pieces |
| `laneOffset` | `lane_offset_m` |
| `elevationProfile` | `elevation_profile` |
| `laneSection` | each `lane_section`, at `station_start_m` |
| `lane`, `width` | each `lane`, `id` -1, -2, … from `lane_num`. `width`: a cubic from `width_start_m`, `width_end_m`, `taper_*` |
| `lane type` | from `role`: `driving`, `bus`, `biking`, `parking`, `shoulder`, `median` |
| `roadMark` | `mark_right` |
| `lane link` | `predecessor_lane_id`, `successor_lane_id` |
| connecting road, `junction`, `laneLink` | each `movement`: one road with one lane, in the `junction` of its `junction_id` |
| `signal`, `signalReference` | each `signal`: `s` = `station_m`, `country` from `jurisdiction`, `type` from `code` |
| `controller` | the traffic lights of one junction. OpenDRIVE has no timing |
| `object` crosswalk | each `crossing` |
| `object` bridge, tunnel | `structure` |
| `geoReference`, `userData` | `project.crs`, `project.ground_id` |

### CommonRoad

| CommonRoad | From |
|---|---|
| `lanelet` | each `lane` and each `movement`. Left and right vertices: the two borders, resampled to the same number of points |
| `lineMarking` | `mark_*`: `solid`, `dashed`; `curb`, `none` → `no_marking` |
| `predecessor`, `successor` | `predecessor_lane_id`, `successor_lane_id`, and lane → movement → lane |
| `adjacentLeft`, `adjacentRight` | lanes of one section with `lane_num` ± 1 |
| `laneletType`, `userOneWay` | `role`, `use` |
| `stopLine` | `stop_line` |
| `trafficSign`, `trafficLight` | `signal` |
| `intersection`, `incoming` | `junction`, `movement.turn` |

### Later: SUMO and glTF

| Format | From |
|---|---|
| SUMO | `carriageway` and `lane` (SUMO lane 0 is the rightmost: the exporter turns the order); `junction.area`; `movement` with `path`; `conflict_movement_ids`, `yield_to_movement_ids` → `foes`, `response`; `signal_plan` → `tlLogic`; `crossing` |
| glTF (3D) | the lane areas, the movement areas and `junction.area`, with Z; `island.area`; `mark_*` and `crossing` |

## Example queries

```sql
-- the lanes of the carriageway that holds one OSM edge, with their marks
SELECT l.lane_id, l.lane_num, l.role, l.width_start_m, l.mark_left, l.mark_right
FROM hdmap.carriageway c JOIN hdmap.lane l USING (carriageway_id)
WHERE list_contains(c.edge_ids, 117961209473327056)
ORDER BY l.lane_num;

-- the lanes that a vehicle can go to from lane 2 of that carriageway, through a junction
SELECT m.to_lane_id, m.turn, m.priority
FROM hdmap.movement m JOIN hdmap.lane l ON l.lane_id = m.from_lane_id
JOIN hdmap.carriageway c ON c.carriageway_id = l.carriageway_id
WHERE list_contains(c.edge_ids, 117961209473327056) AND l.lane_num = 2;

-- the values that are defaults, not data
SELECT ev_method, count(*) FROM hdmap.lane WHERE ev_status = 'assumed' GROUP BY ALL;

-- one table to GeoParquet (DuckDB writes the GeoParquet metadata when spatial is loaded)
COPY hdmap.lane TO 'monaco_lane.parquet' (FORMAT parquet);
```

## Checks

Two kinds. An **error** stops the build: the data is broken. A **review** writes `ev_review = needed` on the rows and a
count in the build report: the data is weak, and a person can look.

| Check | Kind | Passes when |
|---|---|---|
| ids | error | each id follows its rule; each primary key is unique |
| chains | error | each driving edge of the routing database is in exactly one carriageway |
| sections | error | the sections of each carriageway go from 0 to `length_m` with no gap and no overlap |
| lane numbers | error | the lanes of each section are 1 to n, with no gap |
| movements | error | `from_lane_id` ends at an `in` stop line of the junction; `to_lane_id` starts at an `out` stop line |
| lanes meet movements | error | the borders of each movement start and end on the border points of its two lanes (less than 1 cm) |
| marks | error | the two lanes on one border have the same mark |
| short lanelets | review | a lane or a movement with a border shorter than 5 cm |
| signals | review | a `traffic_light` with no `stop_line_id` |
| span clearance | review | a span closer than `clearance` to the road that it crosses, because the 8 % limit stopped the lift |
| no landing | review | a chain of spans with no landing |

## Limits of `duckosm hdmap`

| Limit | Why |
|---|---|
| Driving only | the lane paths through the junctions (netconvert) exist only for driving |
| Borders are correct to about 1 m | OSM center lines and lane widths, not a survey. SimReady uses measured kerbs |
| One lane section for each carriageway; no tapers, no pockets | a chain has one set of lanes, and the GMNS lanes have one width |
| A road is cut where its lanes change, and at each traffic light | the rules of `driving.chains`, the same as SUMO `--merge` |
| One junction for each node | close nodes of one junction are not joined (SimReady joins them) |
| The junction paths come from netconvert | duckOSM has no junction geometry of its own |
| No islands, no signal timing, few speed signs | not in OSM, or not in the GMNS data |
| Road marks are mostly from a rule | OSM has few road mark tags |
| Bridge and tunnel heights are a rule, not a measure | the straight line and the clearance of [Bridges and tunnels](#bridges-and-tunnels) |
