# HD map database

!!! warning "Planned"
    This page is the specification of the HD map database. The code does not exist yet. The reasons for each decision
    are in [`docs/design/hdmap_duckdb.md`](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/hdmap_duckdb.md).

The HD map database (`<area>_hdmap.duckdb`) is the lane-level road model of an area: the roads with their reference
lines, the lane sections, the lanes with their borders in 3D, the paths through each junction, the junction areas,
the stop lines, the crosswalks, the signals and the signs. Each value has its evidence: from the data, computed, or a
default.

One database gives each HD map format: Lanelet2, ASAM OpenDRIVE, CommonRoad, SUMO and glTF. Each exporter reads only
this database.

The model is the core of the SimReady road model (`sim` schema, decision D7 for the ids), with a small number of
additions for the formats. Two programs can write it:

| Producer | Input | Quality |
|---|---|---|
| `duckosm hdmap` | a [GMNS database](../exports/gmns_tables.md) | OSM center lines offset by the lane widths, junction paths from SUMO's netconvert, heights at the nodes |
| SimReady | its own build | measured kerbs, lane tapers, its own junction paths and areas, heights along the road |

```bash
duckosm build --config config/sample_monaco.yaml          # Level 2: monaco.duckdb
duckosm gmns monaco.duckdb                                # Level 3: monaco_gmns.duckdb (the lanes)
duckosm hdmap monaco_gmns.duckdb --source monaco.duckdb   # Level 4: monaco_hdmap.duckdb
duckosm lanelet2 monaco_hdmap.duckdb                      # a format from it
```

```python
from duckosm import hdmap
hdmap.build("monaco_gmns.duckdb", source="monaco.duckdb")     # -> monaco_hdmap.duckdb
hdmap.to_lanelet2("monaco_hdmap.duckdb", "monaco.osm")
```

## The command

`duckosm hdmap GMNS_DB [OPTIONS]`

| Option | Default | Effect |
|---|---|---|
| `--out PATH` | `<area>_hdmap.duckdb`, next to the GMNS database | the HD map database. A run writes the whole file again |
| `--mode NAME` | `driving` | a mode of the GMNS database. Give it more than one time for more modes. Only `driving` has junction paths |
| `--source PATH` | none | the Level 2 database: the ground of the area (`main.elevation_metadata`) and the heights of the edges |
| `--crs CRS` | the UTM zone of the center of the area | the metric CRS of all geometry |
| `--default-width M` | `3.25` | the width of a lane that has no width in the data |
| `--bridge-grade G` | `0.05` | the grade that a bridge deck climbs from its landings ([Bridges and tunnels](#bridges-and-tunnels)) |
| `--clearance M` | `5.0` | the free height between a bridge deck and the road under it, and between a tunnel road and the road over it |

The step opens the GMNS database and the Level 2 database read-only. It does not change them.

## Rules

| Rule | |
|---|---|
| File | one DuckDB file, complete without the other databases |
| Schema | all tables are in the schema `hdmap`. A row of `carriageway` has its `mode` |
| Coordinates | metres, in `project.crs` (a UTM zone). The exporters to longitude and latitude (Lanelet2) transform the points |
| Heights | metres on `project.vertical_datum`, from the [ground of the area](../concepts/ground-reference.md) |
| Units in the names | `_m` metres, `_m2` square metres, `_s` seconds, `_kmh` km/h. No suffix: no unit |
| Ids | `BIGINT`, `(hash(<text>) >> 1)::BIGINT` of the content keys, the formula of the duckOSM `edge_id`. A key of several values is joined by `\|`; a list by `,`. No sequence and no row number. Exception: `node_id` is the OSM node id |
| Stable ids | the same input gives the same ids, on each machine. Ids from two producers are not the same: join them on `carriageway.edge_ids` |
| Stations | metres along the reference line of a carriageway, from its start |
| Left and right | in the direction of travel of the carriageway |
| Lane numbers | lane 1 is the leftmost lane; the numbers go up to the right. Parking and bike lanes are lanes of the cross-section and have numbers |
| Lists, enums | DuckDB lists (`BIGINT[]`, `VARCHAR[]`). An enum is a `VARCHAR` with a `CHECK` of its values |
| Geometry | DuckDB `GEOMETRY`. The type (POINT, LINESTRING Z, POLYGON) is in the column description |

### Evidence

Each row of a table marked "ev" has five evidence columns. They apply to the values that the producer computed for the
row.

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

The column "SimReady" says how the table differs from the SimReady model; the design lists each change.

| Table | One row is | ev | SimReady |
|---|---|---|---|
| [`project`](#project) | the database: area, CRS, ground, producer | | yes, changed |
| [`source`](#source) | a data source: an OSM extract, a DEM, a survey | | yes |
| [`node`](#node) | an OSM node where roads meet or end | ev | yes |
| [`carriageway`](#carriageway) | one direction of travel of a road, with its reference line | ev | yes, changed |
| [`lane_section`](#lane_section) | a part of a carriageway with a fixed set of lanes | ev | yes, changed |
| [`lane`](#lane) | one lane of a section, with its two borders | ev | yes, changed |
| [`junction`](#junction) | one junction: one node or a group of nodes | ev | yes, changed |
| [`turn_rule`](#turn_rule) | a turn from one carriageway to another at a junction | ev | yes |
| [`movement`](#movement) | a path from one lane to another through a junction | ev | yes, changed |
| [`stop_line`](#stop_line) | the line where the lanes of a carriageway end at a junction | ev | yes, changed |
| [`island`](#island) | a traffic island in a junction | ev | yes, changed |
| [`crossing`](#crossing) | a pedestrian crossing on the road | ev | **new** |
| [`signal`](#signal) | a traffic light, or a traffic sign, at its position | ev | **new** |
| [`signal_plan`](#signal_plan), [`signal_phase`](#signal_phase) | the timing of the traffic lights of a junction | ev | yes |
| [`elevation_profile`](#elevation_profile) | a height at a station of a carriageway | ev | yes |
| [`structure`](#structure) | a bridge or a tunnel on a carriageway | ev | yes, changed |

```mermaid
erDiagram
    carriageway ||--o{ lane_section : "divided into"
    lane_section ||--o{ lane : holds
    lane ||--o| lane : succeeds
    carriageway ||--o{ elevation_profile : "heights"
    carriageway ||--o{ structure : carries
    carriageway ||--o{ stop_line : "ends at"
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
    crossing }o--o| junction : "near"
    node }o--o| junction : "belongs to"
```

## project

One row.

| Column | Type | Holds |
|---|---|---|
| `project_id` | BIGINT | `hash("{area_name}")` |
| `area_name` | VARCHAR | for example `monaco` |
| `crs` | VARCHAR | the metric CRS, for example `EPSG:32632` |
| `vertical_datum` | VARCHAR | for example `EGM2008`. Empty when the area has no heights |
| `ground_id` | VARCHAR | **new.** The `sha256` of the ground (`main.elevation_metadata` of the Level 2 database) |
| `ground_product` | VARCHAR | **new.** `DTM` or `DSM` |
| `producer` | VARCHAR | **new.** `duckosm-gmns` or `simready` |
| `boundary` | GEOMETRY | POLYGON. The area |
| `built_at` | TIMESTAMP | |
| `duckosm_db` | VARCHAR | the path of the Level 2 database |
| `duckosm_version` | VARCHAR | |
| `simready_version` | VARCHAR | empty for `duckosm-gmns` |
| `jurisdiction` | VARCHAR | the country code of the traffic rules and signs, for example `MC`, `SE`. From the boundary |
| `driving_side` | VARCHAR | `right` or `left` |
| `osm_timestamp` | TIMESTAMP | the date of the OSM data |

## source

| Column | Type | Holds |
|---|---|---|
| `source_id` | VARCHAR | a short name, for example `osm-2026-11-20`, `fabdem-v1-2` |
| `kind` | VARCHAR | `osm`, `dem`, `netconvert`, `survey`, `lidar`, `city`, `editor` |
| `uri` | VARCHAR | the file or the address |
| `license` | VARCHAR | for example `ODbL-1.0` |
| `retrieved_at` | TIMESTAMP | |

## node

| Column | Type | Holds |
|---|---|---|
| `node_id` | BIGINT | the OSM node id, as in duckOSM. Negative for a virtual node |
| `xy` | GEOMETRY | POINT |
| `degree_in`, `degree_out` | INTEGER | the carriageways in and out |
| `junction_id` | BIGINT | the junction of the node, or empty |
| `highway_tag` | VARCHAR | the OSM `highway` value of the node (`traffic_signals`, `stop`, …) |
| `level` | INTEGER | the vertical level: `layer`, else `bridge` 1, `tunnel` -1, else 0 |

## carriageway

One direction of travel of a road between two junctions or ends.

| Column | Type | Holds |
|---|---|---|
| `carriageway_id` | BIGINT | `hash("{edge_ids joined by ','}")`, the edge ids in travel order |
| `edge_ids` | BIGINT[] | the duckOSM `edge_id`s. `duckosm-gmns`: one edge. SimReady: one or more |
| `mode` | VARCHAR | `driving`, `cycling`, `walking` |
| `from_node_id`, `to_node_id` | BIGINT | |
| `shared_line` | BOOLEAN | true when the two directions of one road share one reference line (OpenDRIVE: one road with left and right lanes) |
| `twin_carriageway_id` | BIGINT | the other direction of the same road, or empty |
| `highway` | VARCHAR | the OSM road class |
| `name` | VARCHAR | the street name |
| `oneway` | BOOLEAN | |
| `inside_junction_id` | BIGINT | set when the carriageway lies inside a junction |
| `posted_speed_kmh` | DOUBLE | the speed limit |
| `reference_line` | GEOMETRY | LINESTRING. The line that the stations and the lane offsets are measured on |
| `length_m` | DOUBLE | |
| `level` | INTEGER | as on `node` |
| `lane_order_convention` | VARCHAR | `left_to_right` (lane 1 is the leftmost) |

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
| `predecessor_lane_id`, `successor_lane_id` | BIGINT | the lane of the section before and after, on the same carriageway. Empty at a junction |
| `cut_start_m`, `cut_end_m` | DOUBLE | the station where the lane meets the junction area. Two lanes of one section can have different cuts |

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
| `assignment_method` | VARCHAR | how the lane pair was chosen: `turn_lanes`, `connectivity_rule`, `netconvert` |
| `path` | GEOMETRY | LINESTRING Z. From the center of the lane in at its cut to the center of the lane out at its cut |
| `length_m`, `width_m` | DOUBLE | |
| `area` | GEOMETRY | POLYGON. The path with its width, with flat ends on the two stop lines |
| `curb_radius_m` | DOUBLE | the radius of a right turn |
| `allowed_uses` | VARCHAR[] | |
| `conflict_movement_ids` | BIGINT[] | the movements of the same junction whose area crosses this one |
| `signal_phase_ids` | BIGINT[] | the phases that give this movement green |

## stop_line

| Column | Type | Holds |
|---|---|---|
| `stop_line_id` | BIGINT | `hash("{carriageway_id}\|{junction_id}\|{end}")` |
| `carriageway_id`, `junction_id` | BIGINT | |
| `end` | VARCHAR | `in`: the carriageway goes into the junction here. `out`: it comes out of the junction here |
| `station_m` | DOUBLE | the mean of the cut stations of the lanes |
| `geometry` | GEOMETRY | LINESTRING. From the left border of lane 1 to the right border of the last lane, through the cut of each lane |
| `lane_cuts` | STRUCT(lane_id BIGINT, station_m DOUBLE, point GEOMETRY)[] | the cut of each lane, from left to right |
| `painted` | BOOLEAN | **new.** True when a line is painted on the road (an `in` line at a `signal`, `stop` or `yield`). An `out` line is never painted |
| `method` | VARCHAR | `lane_border`, `crossing`, `stop_node`, `measured`, `setback`, `netconvert` |
| `source_ref` | VARCHAR | the object that placed the line |

## island

| Column | Type | Holds |
|---|---|---|
| `island_id` | BIGINT | `hash("{junction_id}\|{rank}")`, `rank` counter-clockwise from the core |
| `junction_id` | BIGINT | |
| `level` | INTEGER | |
| `area` | GEOMETRY | POLYGON |
| `area_m2` | DOUBLE | |
| `source_ref` | VARCHAR | the OSM way of the island, when it is in OSM |

## crossing

**New.** A pedestrian crossing on the road.

| Column | Type | Holds |
|---|---|---|
| `crossing_id` | BIGINT | `hash("{osm type}{osm_id}")`, for example `hash("w123")` |
| `osm_id` | BIGINT | the OSM way or node |
| `junction_id` | BIGINT | the junction that it is in or next to, or empty |
| `carriageway_ids` | BIGINT[] | the carriageways that it goes across |
| `lane_ids` | BIGINT[] | the lanes that it covers |
| `kind` | VARCHAR | the OSM `crossing` value: `zebra`, `traffic_signals`, `uncontrolled`, `unmarked`, … |
| `painted` | BOOLEAN | false for `unmarked` |
| `width_m`, `length_m` | DOUBLE | along the road, and across the road |
| `area` | GEOMETRY | POLYGON Z. The rectangle on the road |

## signal

**New.** A traffic light or a traffic sign as a thing at a position. The rules (who yields to whom) are in
`movement.priority`; the timing is in `signal_plan`.

| Column | Type | Holds |
|---|---|---|
| `signal_id` | BIGINT | `hash("{carriageway_id}\|{kind}\|{rank}")`, `rank` in station order |
| `kind` | VARCHAR | `traffic_light`, `stop`, `yield`, `speed_limit`, `no_entry`, `crossing` |
| `code` | VARCHAR | the sign code of the `project.jurisdiction` (for example the Vienna Convention code), when it is known |
| `value` | DOUBLE | the number on the sign, for example 50 for `speed_limit` |
| `carriageway_id` | BIGINT | the carriageway that it applies to |
| `station_m` | DOUBLE | along the carriageway |
| `side` | VARCHAR | `right`, `left`, `above` |
| `stop_line_id` | BIGINT | the stop line that it controls, or empty |
| `lane_ids` | BIGINT[] | the lanes that must obey it; empty: all lanes of the carriageway |
| `osm_id` | BIGINT | the OSM node, when it is in OSM |
| `position` | GEOMETRY | POINT Z |

## signal_plan

| Column | Type | Holds |
|---|---|---|
| `signal_plan_id` | BIGINT | `hash("{junction_id}\|{source_id}")` |
| `junction_id` | BIGINT | |
| `cycle_s`, `offset_s` | DOUBLE | |
| `source_id` | VARCHAR | |
| `method` | VARCHAR | `measured`, `city`, `default` |

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

OSM has no signal timing. `duckosm-gmns` writes no rows in these two tables.

## elevation_profile

| Column | Type | Holds |
|---|---|---|
| `elevation_profile_id` | BIGINT | `hash("{carriageway_id}\|{source_id}\|{method}\|{rank}")` |
| `carriageway_id` | BIGINT | |
| `station_m` | DOUBLE | |
| `z_m` | DOUBLE | the height of the road on `project.vertical_datum` |
| `grade_pct` | DOUBLE | |
| `source_id` | VARCHAR | the ground, or a bridge profile |
| `outlier` | BOOLEAN | |

## structure

| Column | Type | Holds |
|---|---|---|
| `structure_id` | BIGINT | `hash("{carriageway_id}\|{rank}")` |
| `carriageway_id` | BIGINT | |
| `station_start_m`, `station_end_m` | DOUBLE | |
| `kind` | VARCHAR | `bridge`, `viaduct`, `tunnel`, `building_passage`, `culvert`, … |
| `level` | INTEGER | |
| `clearance_m`, `width_m` | DOUBLE | |
| `twin_structure_id` | BIGINT | the structure of the other direction |
| `source_ref` | VARCHAR | the OSM way |

## How `duckosm hdmap` fills the tables

| Table | From the GMNS database | ev_status |
|---|---|---|
| `carriageway` | one row for each `link`. `edge_ids = [link_id]`. `reference_line`: `link.geom` in `project.crs`. `twin_carriageway_id`: the link with the same OSM way in the other direction. `shared_line`: false | `observed` |
| `lane_section` | one section for each link, from 0 to its length | `derived` |
| `lane` | one row for each `lane`. `role`: `bike`, `bus` from `allowed_uses`, `pocket` when the lane is shorter than its link, else `through`. `width_shape`: `constant`. Borders: the lane line (`geom_cut`, else `geom`) offset by half its width (shapely `offset_curve`, one call for all lanes). `cut_end_m`: the station of the end of `geom_cut` | width: `observed` (`width:lanes`) or `assumed`; borders: `derived` |
| `junction` | one row for each node with lane connectors. `control`: from `node.ctrl_type`. `area`: the union of the movement areas | `derived` |
| `turn_rule` | one row for each GMNS `movement` (all are allowed); `turn` from `movement.type` | `observed` (restrictions), `inferred` |
| `movement` | one row for each `lane_connector`. `path`: `lane_connector.geom`. `priority`: from `movement.ctrl_type` and the road class. `conflict_movement_ids`: the movements of the junction whose areas cross | `inferred`, method `netconvert` |
| `stop_line` | one `in` and one `out` row for each carriageway end at a junction, through the cuts of its lanes. `painted` from the `signal` rows | `derived`, method `netconvert` |
| `island` | none | |
| `crossing` | the GMNS `crossing` and `lane_crossing` tables | `observed` |
| `signal` | GMNS `location` with `loc_type` `traffic_signals`, `stop`, `give_way`, and the `ctrl_type` of the movements. `speed_limit` signs: none | `observed` or `inferred` |
| `elevation_profile` | on the ground: two rows for each carriageway, the `z_from` and `z_to` of its edge (Level 2), at stations 0 and `length_m`, method `dem_node`. On a bridge or in a tunnel: the rows of [Bridges and tunnels](#bridges-and-tunnels), method `structure_line` | `derived`; `assumed` for `structure_line` |
| `structure` | one row for each link with `bridge` or `tunnel`, over its whole length | `observed` |

The Z of the borders and the paths: linear between the `elevation_profile` rows of the carriageway. A movement takes
the Z of the end of its lane in and of the start of its lane out, linear between them. Without heights (no `--source`,
or no `duckosm elevation`), Z is 0, `project.vertical_datum` is empty, and `elevation_profile` has no rows.

### Bridges and tunnels

The ground (DTM) is not the road on a bridge or in a tunnel. Under a bridge it is the valley, and over a tunnel it is
the hill. So `duckosm hdmap` does not use the ground there. It uses the rule of SimReady (`import-elevation`, step 3b,
and the deck clearance of step 4), without the vertical curves:

1. **Spans.** A span is a carriageway with `level <> 0` and a `bridge` or `tunnel` tag. It gets a `structure` row.
2. **Chains.** Spans of one kind (bridge or tunnel) that share a node are one chain. A node where a span meets a
   carriageway of level 0 is a **landing** (the end of a bridge, or the entrance of a tunnel).
3. **Landings.** A landing takes the height of the ground at its node.
4. **Nodes inside a chain.** Each node takes the mean of the heights of its neighbour nodes in the chain, with the
   weight `1 / length` of the span between them. The step repeats this until no height moves by more than 1 µm, at
   most 1000 times. The result is a straight line along the chain between the landings.
5. **The crest of a bridge.** A bridge node takes the higher of its height from step 4 and the lowest of
   `z_landing + bridge_grade × d` over the landings, `d` the distance along the chain. Where the two rises meet inside
   a span, the span gets one more row at that station. A tunnel gets no crest.
6. **Clearance.** Where a bridge span goes over a carriageway of a lower level, the deck there is at least
   `z_road + clearance × (level difference)`. Where a tunnel span goes under a carriageway of a higher level, the
   tunnel road there is at most `z_road − clearance × (level difference)`. The lower roads are done first. The deck
   becomes the upper hull (a tunnel: the lower hull) of its ends and these points. A lift steeper than 8 % is not
   made, and the check `span-clearance` fails there.
7. **Rows.** Each height of steps 3 to 6 is an `elevation_profile` row with method `structure_line` and
   `ev_status = assumed`. The span's ground rows are not written.

A chain with no landing (a bridge whose ends are all outside the area) keeps the ground heights with
`ev_review = needed`.

A better height source (LiDAR, a survey, a measured bridge profile) is not in the GMNS data. The SimReady producer uses
one when it has one, and the same tables hold it.

## What each format reads

### Lanelet2

| Lanelet2 | From |
|---|---|
| node | each point of each border, in longitude and latitude, with `ele` from Z. Points within 1 cm are one node, so two lanes side by side share their border, and a lane and its movement share their end points |
| way `line_thin` / `curbstone` / `virtual` | `lane.boundary_left/right` with `mark_*`: `solid`, `dashed` → `line_thin` with that `subtype`; `curb` → `curbstone`; `none` → `virtual`. The border of a movement → `virtual` |
| relation `type=lanelet` | each `lane` (`subtype` from `role`, `speed_limit` from `posted_speed_kmh`) and each `movement` (`turn_direction` from `turn`) |
| way `stop_line` | each `stop_line` with `painted` |
| lanelet `subtype=crosswalk` | each `crossing` |
| `regulatory_element` `traffic_light` | each `signal` of kind `traffic_light`: `refers` its position, `ref_line` its stop line, members the lanelets of `lane_ids` |
| `regulatory_element` `right_of_way` | each junction with `yield` or `stop` movements: `right_of_way` the `major` movements, `yield` the others |
| `regulatory_element` `speed_limit`, `traffic_sign` | `signal` of kind `speed_limit`, `stop`, `yield` |
| map tag `duckosm:ground` | `project.ground_id` |

### ASAM OpenDRIVE 1.7

| OpenDRIVE | From |
|---|---|
| `road` | each `carriageway` (two twins with `shared_line` are one road). `id`: `carriageway_id` |
| `planView` | `reference_line`, as `line` pieces |
| `elevationProfile` | `elevation_profile` |
| `laneSection` | each `lane_section`, at `station_start_m` |
| `lane`, `width` | each `lane`, `id` -1, -2, … from `lane_num`. `width`: a cubic from `width_start_m`, `width_end_m`, `taper_*` |
| `lane type` | from `role`: `driving`, `bus`, `biking`, `parking`, `shoulder`, `median` |
| `roadMark` | `mark_right` |
| `lane link` | `predecessor_lane_id`, `successor_lane_id` |
| connecting road, `junction`, `laneLink` | each `movement`: one road with one lane, in the `junction` of its `junction_id` |
| `signal`, `signalReference` | each `signal`: `s` = `station_m`, `country` from `jurisdiction`, `type` from `code` |
| `controller` | `signal_plan` |
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

### SUMO

| SUMO | From |
|---|---|
| `edge`, `lane` | `carriageway` (or `lane_section`) and `lane`. SUMO lane 0 is the rightmost: the exporter turns the order |
| `junction`, `shape` | `junction.area` |
| `connection`, internal lane | `movement`, with `path` |
| `foes`, `response` | `conflict_movement_ids`, `priority` |
| `tlLogic` | `signal_plan`, `signal_phase` |
| crossing | `crossing` |

### glTF (3D)

The road surface: `lane` areas, `movement` areas and `junction.area`, with Z. Islands: `island.area`. Marks:
`mark_*` and `crossing`.

## Example queries

```sql
-- the lanes of one OSM edge, with their marks
SELECT l.lane_id, l.lane_num, l.role, l.width_start_m, l.mark_left, l.mark_right
FROM hdmap.carriageway c JOIN hdmap.lane l USING (carriageway_id)
WHERE list_contains(c.edge_ids, 117961209473327056)
ORDER BY l.lane_num;

-- the lanes that a vehicle can go to from one lane, through a junction
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

The step stops with an error when a check fails. The tests run them on Monaco.

| Check | Passes when |
|---|---|
| ids | each id is the hash of its content keys; each primary key is unique |
| sections | the sections of each carriageway go from 0 to `length_m` with no gap and no overlap |
| lane numbers | the lanes of each section are 1 to n, with no gap |
| movements | `from_lane_id` ends at an `in` stop line of the junction; `to_lane_id` starts at an `out` stop line |
| lanes meet movements | the end of the borders of the lane in, and the start of the borders of the movement, are less than 1 cm apart |
| marks | the two lanes on one border have the same mark |
| signals | each `traffic_light` has a `stop_line_id` |
| heights | each span row of `elevation_profile` has method `structure_line`; no span row comes from the ground |
| landings | each landing has one height, the ground at its node |
| span clearance | each bridge deck is at least `clearance` over the road under it, and each tunnel road at least `clearance` under the road over it (a fail only where the 8 % limit stops the lift) |

## Limits of `duckosm hdmap`

| Limit | Why |
|---|---|
| Borders are correct to about 1 m | OSM center lines and lane widths, not a survey. SimReady uses measured kerbs |
| One lane section for each edge; no tapers | the GMNS lanes have one width |
| The junction paths come from netconvert | duckOSM has no junction geometry of its own |
| No islands, no signal timing, few speed signs | not in OSM, or not in the GMNS data |
| Road marks are mostly from a rule | OSM has few road mark tags |
| Bridge and tunnel heights are a rule, not a measure | the straight line, the crest and the clearance of [Bridges and tunnels](#bridges-and-tunnels). No vertical curves |
