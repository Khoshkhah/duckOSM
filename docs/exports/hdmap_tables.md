# HD map tables

The tables of the HD map database (level 4). [SimReady](https://github.com/Khoshkhah/simready) builds this database
from a duckOSM database: one carriageway for each chain of `driving.chains`, with the same `chain_id`
([Edges and chains](../concepts/edges-and-chains.md)). All tables are in the schema `sim`.

This page is made from SimReady's `schema.sql` (commit `df192a3`) by `tools/hdmap_tables_page.py`. The units are in the column names (`_m`, `_s`,
`_deg`, `_kmh`). The geometry is in the CRS of `project.crs` (metres, the UTM zone of the area). Each id is a 64-bit
hash of its content keys, with the hash function of duckOSM's [`edge_id`](../concepts/edge-ids.md).

## The tables

| Table | Holds |
|---|---|
| [`attribute`](#attribute) | One resolved value of one carriageway: the raw OSM value, the value that the model uses, and how the model got it. |
| [`carriageway`](#carriageway) | One directed roadway from junction to junction. Its reference line is the base of all the geometry along the road. |
| [`check_result`](#check_result) | One row per check per entity that the check looked at, or per area for a count. |
| [`design_criterion`](#design_criterion) | One design value of a design profile, with the clause of its source. Every number that a step needs is a row here. |
| [`design_profile`](#design_profile) | A named set of design values. A profile can fall back to a parent profile. |
| [`elevation_profile`](#elevation_profile) | The height of the road along a carriageway: `z` at stations of the reference line. |
| [`event`](#event) | A station on a carriageway where the cross-section changes. The events cut the carriageway into lane sections. |
| [`island`](#island) | A part of a junction area that no movement sweep covers and that is large enough to be a physical island: for example a channelizing island beside a slip lane, a refuge in a median, or a splitter island at a roundabout entry. |
| [`junction`](#junction) | One node, or a cluster of nodes that connectors join, with one area. The movements and the islands belong to it. |
| [`kerb_line`](#kerb_line) | An edge of the roadway as a line: a measured kerb, or a bound that the roadway cannot pass. |
| [`kerb_profile`](#kerb_profile) | The width of the roadway at stations of a carriageway, from the kerb lines or from an OSM width tag. |
| [`lane`](#lane) | One lane in one lane section: its number, its role, its width profile and its two boundaries. |
| [`lane_section`](#lane_section) | The part of a carriageway between two events. In it, the lane count and the lane uses do not change. |
| [`loss_report`](#loss_report) | What the format of a simulator could not hold. Each adapter writes one row for each value that it dropped, rounded or made weaker. |
| [`model_override`](#model_override) | One value of the model that a person changed by hand (the source `editor`). |
| [`movement`](#movement) | One legal way through a junction, from one lane to one lane. Its path is the connector and its area is the sweep. |
| [`node`](#node) | A junction point of the network, from the nodes of `duckOSM`. Carriageways start and end at nodes. |
| [`project`](#project) | One row per database. It names the area, the CRS and the versions of the inputs. |
| [`roundabout`](#roundabout) | A group, not a junction: a closed ring of one-way carriageways, a junction at each arm, and a central island. |
| [`rule_attribute`](#rule_attribute) | The rules that resolve the attributes of a carriageway. |
| [`rule_case`](#rule_case) | The case signatures that the case matcher compares with each junction and each lane change. |
| [`rule_design`](#rule_design) | The register of the design rules. |
| [`signal_phase`](#signal_phase) | One phase of a signal plan, and the movements that it serves. |
| [`signal_plan`](#signal_plan) | The controller of a signalled junction. |
| [`source`](#source) | A registered data source: an OSM extract, a city dataset, a LiDAR survey, a DEM, a model, and more. |
| [`stop_line`](#stop_line) | Where an arm ends and the junction starts: the line through the cut points of the arm's lanes. |
| [`street_face`](#street_face) | The street space of one group at one level: the area that the roadways, sidewalks and verges can fill, up to the bounds. |
| [`structure`](#structure) | A span: a bridge, a tunnel or another structure that carries one carriageway over a station range. |
| [`structure_group`](#structure_group) | One physical structure: the connected spans of a bridge system or a tunnel system. |
| [`turn_rule`](#turn_rule) | One turn of a junction, from an entering carriageway to a leaving one: allowed or not, and why. |

## Evidence

Most tables have five evidence columns. They say where each value of the row came from.

| Column | Type | Holds |
|---|---|---|
| `ev_status` | VARCHAR | `observed`: the value is in the source data (an OSM tag, a LiDAR width). `derived`: computed from observed values by a rule with no choice (a length from a line). `inferred`: chosen by a rule or a model among options (a case, a lane assignment); `ev_confidence` is set. `assumed`: a design-profile default stands in for a missing value. `unknown`: no value; the column is NULL. `conflict`: two sources disagree; the row keeps one and names both in `ev_source_ids`. |
| `ev_method` | VARCHAR | The name of the rule, algorithm or model that produced the value: `smoothstep-taper`, `case-matcher:TRN.pocket-swap`, `jev:transition_case`, `ring-inferred` (a one-way ring that OSM does not tag, by the circle fit; `inferred`, and always `ev_review = needed` until a person accepts the pass-0 suggestion. |
| `ev_confidence` | DOUBLE | 0 to 1. NULL unless `ev_status` is `inferred` or `conflict`. |
| `ev_source_ids` | VARCHAR[] | The sources of the value, each as `"{source_id}:{object}"` with `source_id` a row of [source.md](#model_override): `osm-2026-10-20:w6110234`, `lidar-vancouver-2022:tile-07`, `jev-latest:req-7f3a…`. |
| `ev_review` | VARCHAR | `none`: no review is needed. `needed`: a person must look (every `inferred` value below `design_criterion` `review_confidence_min`, every `conflict`). `accepted`, `rejected`: a person looked. |

## attribute

One resolved value of one carriageway: the raw OSM value, the value that the model uses, and how the model got it.

| Column | Type | Holds |
|---|---|---|
| `attribute_id` (key) | BIGINT | `hash("{carriageway_id}\|{name}\|{rank}")`, `rank` the row's number among the rows of this name on the carriageway, in station order from 1 |
| `carriageway_id` | BIGINT | The carriageway |
| `name` | VARCHAR | The attribute |
| `station_start_m` | DOUBLE | Where the value begins. 0 when the value holds for the whole carriageway |
| `station_end_m` | DOUBLE | Where it ends; `length_m` for the whole carriageway. A value that changes along the carriageway (a `turn:lanes` on the last edge only) is several rows |
| `raw_value` | VARCHAR | The OSM tag value as written, or NULL when the tag is missing |
| `value` | VARCHAR | The resolved value, in the model's form: a number as text (`3`, `50`, `10.5`), a list with `\|` (`left\|through\|right`), `yes` or `no`. NULL when nothing could resolve it (`ev_status = unknown`) |
| `unit` | VARCHAR | `count`, `m`, `kmh`, or empty |
| `method` | VARCHAR | `osm_tag` (the tag is there and passes the rules), `rule` (a plausibility rule filled or corrected it; `rule_name` says which), `override` (from `osm_overrides.yaml`), `model` (a decision model; `source_ref` is the request), `default` (the design profile's default) |
| `rule_name` | VARCHAR | The rule of the algorithm `attribute-rules` that produced the value: `lanes_from_turn_lanes`, `lanes_from_width`, `lanes_from_twin`, `lanes_forward_backward`, `speed_by_class`, `speed_unit`, `oneway_from_twin`, `layer_from_bridge`, … NULL for `osm_tag` and `override`. No rule invents a `width`: a missing road width stays `unknown`, because a default width is a lane rule, not a road width |
| `source_ref` | VARCHAR | The object the value came from, as `"{source_id}:{object}"`: the way, the override entry, the model request |
| `network_fix` | BOOLEAN | TRUE when the resolved value differs from the raw value and changes the network (one-way, access, a connection): the fix is written to the overrides file for pass 0 and takes effect only after the rebuild |

And the five [evidence](#evidence) columns.

## carriageway

One directed roadway from junction to junction. Its reference line is the base of all the geometry along the road.

| Column | Type | Holds |
|---|---|---|
| `carriageway_id` (key) | BIGINT | `hash("{edge_ids joined by ','}")`, the edge ids in travel order |
| `edge_ids` | BIGINT[] | The `duckOSM` `edge_id`s in travel order. Each edge is in exactly one carriageway |
| `mode` | VARCHAR | `driving`. Other modes come later; the schema does not change for them |
| `from_node_id` | BIGINT | The node at station 0. See [node.md](#node) |
| `to_node_id` | BIGINT | The node at `length_m` |
| `shared_line` | BOOLEAN | TRUE when the reference line is the twin's too (one OSM way with two directions); FALSE for a one-way street or a divided street. Decides the offset of lane 1 ([width-profile.md](#lane)) |
| `twin_carriageway_id` | BIGINT | The carriageway of the other direction of the same street, or NULL for a one-way street. For one OSM way with two directions the twin is known; for a divided street (two OSM ways) `reference-line` finds it by a rule, the algorithm `twin-pairing` at level 4: opposite direction, parallel within `design_criterion` `twin_max_distance_m`, the same name or one level, as duckOSM's `gmns.py` pairs them. Evidence `inferred` in that case |
| `highway` | VARCHAR | The OSM `highway` tag of the edges (all edges of a run share it) |
| `name` | VARCHAR | The street name, or NULL |
| `oneway` | BOOLEAN | TRUE when the street has no twin |
| `roundabout_id` | BIGINT | The roundabout this carriageway is a ring piece of, or NULL. See [roundabout.md](#roundabout) |
| `inside_junction_id` | BIGINT | The junction this carriageway lies wholly inside, when it is a piece between two nodes of one cluster ([junction.md](#junction)); NULL for an arm or a section. Written by `junction-cut` |
| `posted_speed_kmh` | DOUBLE | The resolved `maxspeed` attribute for most of the run's length, copied for the design criteria whose condition names a speed; NULL when `unknown` |
| `reference_line` | GEOMETRY (LINESTRING) | LINESTRING. The merged line of the edges, in metres. Station 0 is its first point. Where the twins share one OSM way, this is the way's line; where OSM has two ways, each carriageway has its own |
| `length_m` | DOUBLE | The length of `reference_line` |
| `level` | INTEGER | The level of the run: the level of its edges, the band of their tags in `driving.edges` (a non-zero `layer`, else 1 for a bridge, −1 for a tunnel that is not a `building_passage`, else 0; a level change ends a run, so one value holds). The sections on a span carry their own level |
| `lane_order_convention` | VARCHAR | `left_to_right_from_1`. Written on every row so an adapter can read it without knowing the decision |

And the five [evidence](#evidence) columns.

## check_result

One row per check per entity that the check looked at, or per area for a count.

| Column | Type | Holds |
|---|---|---|
| `check_result_id` (key) | BIGINT | `hash("{check_id}\|{entity_table}\|{entity_id}")` |
| `check_id` | VARCHAR | The check's name, as the level-6 page defines it: `lanes-no-overlap`, `width-sum`, `path-in-area`, `netconvert-path-distance`, `sections-unmatched-count` |
| `entity_table` | VARCHAR | The table of the entity checked: `lane`, `movement`, `junction`, or `project` for a count over the area |
| `entity_id` | BIGINT | The entity's id |
| `status` | VARCHAR | `pass`, `fail`, `info`. `info` is a measurement with no threshold (a count, a distance) |
| `value` | DOUBLE | The measured value, or NULL |
| `threshold` | DOUBLE | The threshold the check compared against, from the design profile, or NULL |
| `unit` | VARCHAR | The unit of `value` |
| `message` | VARCHAR | One sentence a person reads on the dashboard: `lane 3 overlaps lane 4 by 0.41 m² at station 163` |
| `geometry` | GEOMETRY (GEOMETRY) | Where to look: the overlap polygon, the point of the largest distance, or NULL |
| `built_at` | TIMESTAMP | The build |

## design_criterion

One design value of a design profile, with the clause of its source. Every number that a step needs is a row here.

| Column | Type | Holds |
|---|---|---|
| `design_profile_id` | VARCHAR | The profile |
| `name` | VARCHAR | The criterion's name. The names a module reads are listed on its module page; the full list is below |
| `condition` | VARCHAR | When the row applies, as a DuckDB boolean expression over the entity's columns: `highway = 'primary'`, `posted_speed_kmh >= 60 AND highway IN ('primary','secondary')`, or `TRUE` for the default row |
| `specificity` | INTEGER | The order among rows of one name: the highest that matches wins. The default row has 0 |
| `value` | DOUBLE | The value |
| `unit` | VARCHAR | `m`, `m2`, `s`, `deg`, `kmh`, `ratio`, `pct`, `count`, or empty |
| `clause` | VARCHAR | The cited place in the standard: `AASHTO GDHS 7th ed., §4.3.1, Table 4-2`. For a project profile: the decision or the measurement that set it |
| `note` | VARCHAR | Free text |

## design_profile

A named set of design values. A profile can fall back to a parent profile.

| Column | Type | Holds |
|---|---|---|
| `design_profile_id` (key) | VARCHAR | A short name, written by hand: `aashto-2018`, `tac-2017`, `nacto-2013`, `bc-supplement-2019`, `project-default`, `monaco-2026` |
| `title` | VARCHAR | The full title of the standard or of the project profile |
| `standard` | VARCHAR | The publishing body: `AASHTO`, `TAC`, `NACTO`, `MUTCD`, `project` |
| `edition` | VARCHAR | The year or edition |
| `jurisdiction` | VARCHAR | Where it applies: `US`, `CA`, `CA-BC`, `MC`, or NULL for a project profile |
| `parent_profile_id` | VARCHAR | The profile to read when this one has no row for a criterion, or NULL. A project profile's parent is a standard; a standard's parent is `project-default` for the engine's own thresholds |

## elevation_profile

The height of the road along a carriageway: `z` at stations of the reference line.

| Column | Type | Holds |
|---|---|---|
| `elevation_profile_id` (key) | BIGINT | `hash("{carriageway_id}\|{source_id}\|{method}\|{rank}")`, `rank` the row's number among the rows of this source and method on the carriageway, in station order from 1 |
| `carriageway_id` | BIGINT | The carriageway |
| `station_m` | DOUBLE | The station |
| `z_m` | DOUBLE | The height of the road surface at the reference line, in the vertical datum of `source` (`source.note` names it; `EGM2008` for Copernicus) |
| `grade_pct` | DOUBLE | The slope at this station, positive uphill in the travel direction, from the neighbouring rows. NULL on the first and last row |
| `earthwork_m` | DOUBLE | On a `graded` row: the graded height minus the terrain's height at this station (`dem_sample`, or `dem_node` interpolated when no sample exists). Positive is fill (an embankment), negative is cut. NULL on other rows and where no terrain source exists |
| `outlier` | BOOLEAN | TRUE on a `dem_node` or `dem_sample` row that the fit left out because it lies above the lower envelope of its neighbours (a roof, a tree; "From samples to a graded profile", step 2). FALSE on every other row. The row stays; only the fit ignores it |
| `source_id` | VARCHAR | The source, a row of [source.md](#model_override) |
| `method` | VARCHAR | `osm_ele` (an OSM `ele` tag on a node), `dem_node` (the DEM at the carriageway's nodes, bilinear between the pixels of duckOSM's `elevation_grid`, interpolated between the nodes), `dem_sample` (the DEM at every vertex of the carriageway, bilinear in the same way), `dsm_sample` (a surface model, on spans), `bridge_profile` (urbanstyle's deck heights), `lidar_surface` (the road surface in the point cloud), `survey_point`, `trace_mean` (GNSS traces), `structure_line` (assumed, on a span with no measured height: the straight line between the heights of the span's two ends), `graded` (the fitted vertical alignment the lanes use). The sources behind each are in "Sources" below |

And the five [evidence](#evidence) columns.

## event

A station on a carriageway where the cross-section changes. The events cut the carriageway into lane sections.

| Column | Type | Holds |
|---|---|---|
| `event_id` (key) | BIGINT | `hash("{carriageway_id}\|{kind}\|{rank}")`, `rank` the event's number among the events of this kind on the carriageway, in station order from 1 (schema.md, "Ids") |
| `carriageway_id` | BIGINT | The carriageway |
| `station_m` | DOUBLE | The station of the event |
| `kind` | VARCHAR | What changes |
| `value_before` | VARCHAR | The value upstream of the station, as text: `2` for a lane count, `left\|through` for a turn marking, `parallel` for parking. NULL when the kind has no value (a crossing) |
| `value_after` | VARCHAR | The value downstream |
| `point` | GEOMETRY (POINT) | POINT. The reference line at `station_m` |
| `source_ref` | VARCHAR | The object that places the event, as `"{source_id}:{object}"` ([source.md](#model_override)): an OSM node, the second of two ways with different tags, a city feature, an image. NULL for `start`, `end` and `stop_line` |
| `case_id` | VARCHAR | The transition case matched at this station (`TRN.*`, or NULL: all events at one station are one transition, and the id is written on each of them. Written by `case-matcher` with scope `transition` |
| `case_score` | DOUBLE | Its score, 0 to 1 |
| `case_alternatives` | STRUCT(case_id VARCHAR, score DOUBLE) | The other transition cases considered |

And the five [evidence](#evidence) columns.

## island

A part of a junction area that no movement sweep covers and that is large enough to be a physical island: for example a channelizing island beside a slip lane, a refuge in a median, or a splitter island at a roundabout entry.

| Column | Type | Holds |
|---|---|---|
| `island_id` (key) | BIGINT | `hash("{junction_id or roundabout_id}\|{rank}")`, `rank` the island's number among the islands of its junction (or roundabout), counter-clockwise by the bearing of its centroid from the core (or the roundabout's centre), from 1 |
| `junction_id` | BIGINT | The junction, or NULL for a `central` island |
| `roundabout_id` | BIGINT | The roundabout, for a `central` island; else NULL |
| `kind` | VARCHAR | `channelizing` (between a slip lane and the main movements), `refuge` (a crossing passes through it, wide enough to wait on), `splitter` (at a roundabout entry, between the entry and the exit), `central` (the island inside a roundabout's ring), `median_end` (the end of a physical median inside the junction), `remnant` (none of the above; the junction-area module could not name it) |
| `level` | INTEGER | The level of its junction or roundabout |
| `area` | GEOMETRY (POLYGON) | POLYGON |
| `area_m2` | DOUBLE | Its area |
| `case_id` | VARCHAR | The junction case that names this island (a slip lane case names its channelizing island), or NULL. |
| `source_ref` | VARCHAR | An object that maps the island, as `"{source_id}:{object}"`: an OSM `area:highway=traffic_island`, a `footway` across it, a `crossing:island=yes` node, a city kerb polygon, a LiDAR tile. NULL when the sweeps alone define it |

And the five [evidence](#evidence) columns.

## junction

One node, or a cluster of nodes that connectors join, with one area. The movements and the islands belong to it.

| Column | Type | Holds |
|---|---|---|
| `junction_id` (key) | BIGINT | `hash("{node_ids sorted, joined by ','}")` |
| `node_ids` | BIGINT[] | The nodes of the cluster, sorted |
| `kind` | VARCHAR | `simple` (one node), `cluster` (several nodes joined by connectors), `roundabout_node` (a node where an arm meets a roundabout's ring; the ring itself is a [roundabout](#roundabout), not a junction) |
| `control` | VARCHAR | The control of the junction as a whole: the `kind` of its main node (`signal`, `priority`, `stop`, `yield`, `roundabout`, `right_before_left`, `uncontrolled`) |
| `core` | GEOMETRY (POINT) | POINT. The centroid of the nodes |
| `arm_count` | INTEGER | The number of arms: a one-way carriageway is one arm, the two twins of a street are one arm. A four-way crossing of two-way streets has 4 arms and 8 stop lines |
| `arm_order` | BIGINT[] | The arms counter-clockwise by their bearing at `core`, as the id of the entering carriageway (or of the right-hand twin for a pair). Two arms that are consecutive in this list are neighbours; `junction-cut` intersects the outer lane borders of neighbours |
| `through_arms` | BIGINT[] | The through pair: the two arms, as ids of `arm_order`, that continue each other across the junction, or empty. Written by `junction-cut` step 6a |
| `through_kind` | VARCHAR | The kind of the through pair: `two_way`, `twins`, `oneway`, `divides` or `none` (../5-cases/index.md, scope `junction`) |
| `side_kinds` | VARCHAR[] | The kinds of the arms that are not in the through pair, sorted: `in`, `out`, `twins`, `two_way`. All arms when `through_kind` is `none` |
| `offset` | BOOLEAN | TRUE when the two through arms end at nodes with no node in common: an inner piece carries the through street (a dogleg, a staggered junction) |
| `inner_kind` | VARCHAR | The signature's `inner`: `slip`, `link`, `oneway`, `cross` or `none`, the first of them that one inner piece is (../5-cases/index.md, scope `junction`). Written by `junction-cut` step 6a |
| `fork_angle` | VARCHAR | For three one-way arms: `slight` or `turn`, by `turn_angle_slight_deg`. `none` for every other junction |
| `shape` | GEOMETRY (POLYGON) | POLYGON. The first junction shape: the area between the lane cuts of all arms, from the intersections of the outer lane borders of neighbouring arms, the kerb corners and the crossings. A cluster's shape is built per node and united with the inner pieces' roadway between the node shapes. Written by `junction-cut`. The checks compare it with `netconvert`'s node shape |
| `case_id` | VARCHAR | A derived junction case (level 5, or NULL. Written by `case-matcher` with scope `junction`, after `junction-cut` and before `connectors`, because the case sets the connector rule. |
| `case_score` | DOUBLE | Its score |
| `case_alternatives` | STRUCT(case_id VARCHAR, score DOUBLE) | The other cases considered |
| `area` | GEOMETRY (POLYGON) | POLYGON. The final junction area: `shape` with the movement sweeps and the arm ends added, clipped to the kerb lines ([kerb-line.md](#kerb_line)). Written by `junction-area` |
| `area_m2` | DOUBLE | Its area |
| `island_count` | INTEGER | The number of islands |
| `signal_plan_id` | BIGINT | The signal plan, or NULL. See [signal-plan.md](#signal_phase) |
| `roundabout_id` | BIGINT | The roundabout this junction is a ring node of, or NULL |
| `level` | INTEGER | The level of the junction's nodes. All nodes of a junction share it (rule 14 of schema.md) |

And the five [evidence](#evidence) columns.

## kerb_line

An edge of the roadway as a line: a measured kerb, or a bound that the roadway cannot pass.

| Column | Type | Holds |
|---|---|---|
| `kerb_line_id` (key) | BIGINT | `hash("{source_id}\|{object}\|{part}")`, `object` the object part of `source_ref`, `part` the index of the piece when a source object gives several lines; for a bound or a face end, the start point of the piece in decimetres (`{x}_{y}`), for a `sidewalk_way` line `sidewalk` and the index of the segment |
| `geometry` | GEOMETRY (LINESTRING) | LINESTRING, in metres |
| `source_id` | VARCHAR | The source, a row of [source.md](#model_override) |
| `source_ref` | VARCHAR | The object in the source, as `"{source_id}:{object}"`: the city feature, the LiDAR tile, the image; for a bound, the face (`<source_id>:<face_id>`) |
| `role` | VARCHAR | `kerb`: the measured edge of the roadway. `bound`: a line the roadway cannot pass; the sidewalk lies between a bound and the kerb. `face_end`: the cross-section line where a face ends (a landing line at a portal or an abutment, the end of a ring face), not an edge of the street; the kerb profile ignores it. `unused`: a kerb or a bound of an observed OSM method that lies farther than `kerb_ray_max_m` from every carriageway of its level (a planter kerb in a plaza): a kerb that no cross-section ray can reach, kept as evidence and used by nothing |
| `method` | VARCHAR | Role `kerb`: `city_kerb` (a kerb feature as delivered), `lidar_step` (a height step in the point cloud), `image_trace`, `osm_barrier` (`barrier=kerb` in OSM), `deck_profile` (the edge of a structure group's measured profile), `structure_width` (half of a span's `width_m` to each side of the reference line, when no source covers the span), `sidewalk_way` (a `footway=sidewalk` way shifted half the default sidewalk width toward the road). Role `bound`: `street_edge` (the ground face), `deck_edge` (the face of a bridge chain), `tunnel_edge` (the face of a tunnel chain), `ring_edge` (the face of a roundabout ring); the faces are built by `import-kerb` ([street-face.md](#street_face)), `osm_area`, `midline` (a side where another carriageway of the same level lies between the reference line and the street edge, [kerb-profile.md](#kerb_profile)) (an `area:highway` polygon's edge) |
| `height_m` | DOUBLE | The kerb height, when the source measures it (LiDAR, a survey), else NULL |
| `level` | INTEGER | The level of the roadway this kerb bounds: 0 for a street edge, the span's level for a parapet or a tunnel wall. A module reads only kerb lines of its own level |
| `structure_id` | BIGINT | The span whose parapet or wall this is, or NULL |
| `junction_id` | BIGINT | The junction whose area this line bounds, when the line lies within `design_criterion` `junction_kerb_search_m` of a junction's core, else NULL. Written by `junction-area` |

And the five [evidence](#evidence) columns.

## kerb_profile

The width of the roadway at stations of a carriageway, from the kerb lines or from an OSM width tag.

| Column | Type | Holds |
|---|---|---|
| `kerb_profile_id` (key) | BIGINT | `hash("{carriageway_id}\|{source_id}\|{method}\|{rank}")`, `rank` the row's number among the rows of this source and method on the carriageway, in station order from 1 |
| `carriageway_id` | BIGINT | The carriageway |
| `station_m` | DOUBLE | The station of the measurement |
| `offset_left_m` | DOUBLE | The distance from the reference line to the left kerb, negative to the left. NULL when only the total width is known |
| `offset_right_m` | DOUBLE | The distance to the right kerb, positive. NULL when only the total width is known |
| `width_m` | DOUBLE | The roadway width kerb to kerb. Equal to `offset_right_m − offset_left_m` when both exist |
| `bound` | BOOLEAN | TRUE when the row was sampled from a `bound` kerb line (a street edge, a building face): then `width_m` is the most the roadway can be, not what it is. The width-sum rule is an equality only for rows with `bound = FALSE` |
| `share` | DOUBLE | For a twin on a shared reference line: this carriageway's part of the measured street width, 0 to 1, as the ratio of the sum of its lanes' default widths to both twins' sum (the lanes do not exist yet when the row is written; the defaults come from the resolved `lanes` attributes). 1 for a carriageway on its own line |
| `source_id` | VARCHAR | The source, a row of [source.md](#model_override) |
| `method` | VARCHAR | How the width was read from the source: `osm_width` (the `width` tag of the way), `osm_width_lanes` (the sum of `width:lanes`), `street_edge`, `deck_edge`, `tunnel_edge`, `ring_edge` (the faces per group and level of `import-kerb`, `city_kerb`, `lidar_step`, `image_trace`, `osm_barrier`, `sidewalk_way`, `structure_width`, `deck_profile`, `osm_area` (each: the two points where a cross-section ray from the reference line meets the first kerb line of that method and of the carriageway's level on each side, within `kerb_ray_max_m`, [kerb-line.md](#kerb_line)), `survey_point`, `image_measure`, `midline` (a side where another carriageway of the same level lies between the reference line and the street edge, a twin, a frontage road, a ring piece: a bound at the midline between the two reference lines, because the face is one polygon for every carriageway inside it) |
| `point_left` | GEOMETRY (POINT) | POINT. The left kerb point, or NULL |
| `point_right` | GEOMETRY (POINT) | POINT. The right kerb point, or NULL |

And the five [evidence](#evidence) columns.

## lane

One lane in one lane section: its number, its role, its width profile and its two boundaries.

| Column | Type | Holds |
|---|---|---|
| `lane_id` (key) | BIGINT | `hash("{lane_section_id}\|{lane_num}")` |
| `lane_section_id` | BIGINT | The section |
| `carriageway_id` | BIGINT | The carriageway, copied for joins |
| `lane_num` | INTEGER | 1 is the left-most lane; numbers increase to the right. Parking and bike lanes count: they are lanes of the cross-section |
| `role` | VARCHAR | `through`, `pocket`, `merge`, `parking`, `bike`, `bus`, `shoulder`, `median` |
| `use` | VARCHAR[] | Who can drive on it: `auto`, `bus`, `bike`, `hgv`, `taxi`. Empty for `parking`, `shoulder` and `median` |
| `turn` | VARCHAR | The lane's entry in `turn:lanes` at the section's end (`left`, `through`, `right`, `left;through`, …), or NULL |
| `width_shape` | VARCHAR | `constant`, `linear`, `smoothstep` |
| `width_start_m` | DOUBLE | The width at `station_start_m` of the section |
| `width_end_m` | DOUBLE | The width at `station_end_m` |
| `taper_start_m` | DOUBLE | The station where the width starts to change; NULL for `constant` |
| `taper_length_m` | DOUBLE | The length over which it changes; NULL for `constant` |
| `boundary_left` | GEOMETRY | LINESTRING Z. The left edge of the lane, from the section's start (or `cut_start_m`) to its end (or `cut_end_m`). Stored |
| `boundary_right` | GEOMETRY | LINESTRING Z. The right edge. Stored |
| `predecessor_lane_id` | BIGINT | The lane of the previous section that this lane continues, or NULL when the lane begins here (a pocket opens) or the section is the first |
| `successor_lane_id` | BIGINT | The lane of the next section, or NULL when the lane ends here (a merge closes) or the section ends at a stop line or a dead end |
| `cut_start_m` | DOUBLE | The station where the lane's area begins, when the section begins at a junction: where the lane's borders meet the junction shape. NULL elsewhere. Written by `junction-cut` |
| `cut_end_m` | DOUBLE | The station where the lane's area ends, when the section ends at a junction. NULL elsewhere. Two lanes of one section can have different cuts |

And the five [evidence](#evidence) columns.

## lane_section

The part of a carriageway between two events. In it, the lane count and the lane uses do not change.

| Column | Type | Holds |
|---|---|---|
| `lane_section_id` (key) | BIGINT | `hash("{carriageway_id}\|{rank}")`, `rank` the section's number on the carriageway in station order from 1 |
| `carriageway_id` | BIGINT | The carriageway |
| `station_start_m` | DOUBLE | The start station: an event's station |
| `station_end_m` | DOUBLE | The end station: the next event's station |
| `length_m` | DOUBLE | `station_end_m − station_start_m` |
| `event_start_id` | BIGINT | The event at the start |
| `event_end_id` | BIGINT | The event at the end |
| `lane_count_start` | INTEGER | The number of driving lanes (roles `through`, `pocket`, `merge`, `bus`) at the start, from the resolved `lanes` attribute and the events, before any width exists. Parking and bike lanes are not counted |
| `lane_count_end` | INTEGER | The same at the end |
| `case_id` | VARCHAR | A derived section case (`SEC.*`, level 5, decisions D2 and D22): what the cross-section is between two events. The change at an event is the event's own `case_id` (`TRN.*`, [event.md](#event)). `SEC.uniform`, the fallback, when no rule reaches `case_score_min`. Written by `case-matcher` with scope `section` |
| `case_score` | DOUBLE | The score of `case_id`, 0 to 1 |
| `case_alternatives` | STRUCT(case_id VARCHAR, score DOUBLE) | The other cases the matcher considered, best first |
| `change_reason` | VARCHAR | Why this section is not part of its neighbour, in the matcher's words: `lane added from parking`, `uniform`, `pocket opens` |
| `kerb_width_start_m` | DOUBLE | The kerb profile's width at the start, or NULL |
| `kerb_width_end_m` | DOUBLE | The kerb profile's width at the end, or NULL |
| `level` | INTEGER | The level of the section: the level of the span it lies on ([structure.md](#structure)), else 0 or OSM `layer` |
| `structure_id` | BIGINT | The span the section lies on, or NULL |

And the five [evidence](#evidence) columns.

## loss_report

What the format of a simulator could not hold. Each adapter writes one row for each value that it dropped, rounded or made weaker.

| Column | Type | Holds |
|---|---|---|
| `loss_report_id` (key) | BIGINT | `hash("{adapter}\|{entity_table}\|{entity_id}\|{column}")` |
| `adapter` | VARCHAR | `sumo`, `opendrive`, `gmns`, `matsim`, `lanelet2`, `gltf`, `vissim`, `aimsun`, `synchro` |
| `entity_table` | VARCHAR | The table of the value |
| `entity_id` | BIGINT | The entity |
| `column` | VARCHAR | The column, or `*` for the whole entity |
| `reason` | VARCHAR | `no_field` (the format has no place for it), `precision` (rounded or sampled), `enum_missing` (the format's enum has no equal value; the nearest was written), `not_supported` (the whole entity has no counterpart), `status` (the value is `assumed` or `inferred` and the format cannot say so) |
| `value_lost` | VARCHAR | The value as text |
| `value_written` | VARCHAR | What the adapter wrote instead, or NULL |
| `built_at` | TIMESTAMP | The build |

And the five [evidence](#evidence) columns.

## model_override

One value of the model that a person changed by hand (the source `editor`).

| Column | Type | Holds |
|---|---|---|
| `override_id` (key) | BIGINT | `hash("{entity_table}\|{entity_id}\|{column}")` |
| `entity_table` | VARCHAR | The table of the edited value |
| `entity_id` | BIGINT | The entity |
| `column` | VARCHAR | The column |
| `value` | VARCHAR | The value as text, in the column's unit |
| `who` | VARCHAR | Who made the edit |
| `made_at` | TIMESTAMP | When |
| `applied` | BOOLEAN | TRUE when the last build found the entity and wrote the value; FALSE is the check `override-entity-missing` |

## movement

One legal way through a junction, from one lane to one lane. Its path is the connector and its area is the sweep.

| Column | Type | Holds |
|---|---|---|
| `movement_id` (key) | BIGINT | `hash("{from_lane_id}\|{to_lane_id}")` |
| `junction_id` | BIGINT | The junction |
| `turn_rule_id` | BIGINT | The turn rule that allows this movement ([turn-rule.md](#turn_rule)). The rule's evidence is the evidence of the movement's existence |
| `from_lane_id` | BIGINT | The lane whose section ends at a stop line of the junction (`end = in`) |
| `to_lane_id` | BIGINT | The lane whose section begins at a stop line of the junction (`end = out`) |
| `from_carriageway_id` | BIGINT | Copied for joins |
| `to_carriageway_id` | BIGINT | Copied for joins |
| `turn` | VARCHAR | `left`, `straight`, `right`, `u`, `slight_left`, `slight_right`. From the angle between the arms, with the thresholds in the design profile |
| `fork` | BOOLEAN | Copied from the turn rule: TRUE for a merge (more than one entering arm and one leaving arm) or a diverge (one entering arm and more than one leaving arm) where the lanes continue ([turn-rule.md](#turn_rule)). The lane pairs of a fork come from the fan rule of the connectors module |
| `assignment_method` | VARCHAR | How the lane pair was chosen: `turn_lanes` (OSM `turn:lanes` on the from-lane), `connectivity_rule` (the rule in the connectors module), `netconvert` (SUMO's assignment, read as a reference), `measured` (a city lane-line layer or an image), `model` |
| `path` | GEOMETRY (LINESTRING) | LINESTRING. The connector: a cubic Hermite curve from the from-lane's centerline at its cut (`cut_end_m`) to the to-lane's centerline at its cut (`cut_start_m`), with the tangents of the two reference lines there |
| `length_m` | DOUBLE | The length of `path` |
| `width_m` | DOUBLE | The width of the connector: the from-lane's width at the stop line, or the smaller of the two lanes' widths |
| `area` | GEOMETRY (POLYGON) | POLYGON. The sweep: `path` buffered by `width_m / 2`, with flat ends on the two stop lines |
| `curb_radius_m` | DOUBLE | For a `right` turn: the radius of the lane's centerline through the turn (the kerb corner's radius is this minus half the lane width); for a `u` turn: half the twin lanes' separation. NULL otherwise |
| `allowed_uses` | VARCHAR[] | The intersection of the two lanes' `use`, further limited by `duckOSM`'s turn permissions |
| `priority` | VARCHAR | `major`, `minor`, `yield`, `stop`, `signal`. From the junction's control and the arms' ranks |
| `conflict_movement_ids` | BIGINT[] | The movements of the same junction whose `area` crosses this one's, excluding those that share the from-lane or the to-lane |
| `signal_phase_ids` | BIGINT[] | The phases that serve this movement, or empty. Written by `import-signals`, after `connectors` |

And the five [evidence](#evidence) columns.

## node

A junction point of the network, from the nodes of `duckOSM`. Carriageways start and end at nodes.

| Column | Type | Holds |
|---|---|---|
| `node_id` (key) | BIGINT | The OSM node id, as in `duckOSM`. Not a hash: the one id that comes from outside |
| `xy` | GEOMETRY (POINT) | POINT, in metres |
| `kind` | VARCHAR | `priority`, `signal`, `stop`, `yield`, `roundabout`, `right_before_left`, `uncontrolled`, `dead_end`, `connector_end` |
| `degree_in` | INTEGER | The number of carriageways that end here |
| `degree_out` | INTEGER | The number that start here |
| `junction_id` | BIGINT | The junction the node belongs to. See [junction.md](#junction). NULL for `dead_end` |
| `highway_tag` | VARCHAR | The OSM `highway` tag of the node (`traffic_signals`, `stop`, `give_way`, `crossing`, `mini_roundabout`), or NULL |
| `level` | INTEGER | The level of the carriageways that meet here. A node joins carriageways of one level only; where a ramp changes level, the change is a `structure` event on the ramp, not at the node |

And the five [evidence](#evidence) columns.

## project

One row per database. It names the area, the CRS and the versions of the inputs.

| Column | Type | Holds |
|---|---|---|
| `project_id` (key) | BIGINT | `hash("{area_name}\|{built_at}")` |
| `area_name` | VARCHAR | The area, as `duckOSM` names it: `monaco`, `tartu`, `granville` |
| `crs` | VARCHAR | The EPSG code of every geometry column: `EPSG:32632` for Monaco. The UTM zone of the area's centre |
| `vertical_datum` | VARCHAR | The datum of every `z`: `EGM2008` by default, or the national one (`RH2000`, `CGVD2013`) when a finer source uses it. `import-elevation` converts every source to it ([elevation-profile.md](#elevation_profile), "Sources") |
| `boundary` | GEOMETRY (POLYGON) | POLYGON. The area's boundary, in `crs` |
| `built_at` | TIMESTAMP | When the build started, UTC |
| `duckosm_db` | VARCHAR | The path of the `duckOSM` database the build read |
| `duckosm_version` | VARCHAR | Its `duckOSM` or `duckosm-rust` version |
| `urbanstyle_version` | VARCHAR | The `urbanstyle` version whose faces the comparison read, or NULL when the build ran no comparison |
| `simready_version` | VARCHAR | The version of this model's code |
| `design_profile_id` | VARCHAR | The design profile of the build. See [design-profile.md](#design_criterion) |
| `jurisdiction` | VARCHAR | The country and region of the area, as ISO 3166-2 (`CA-BC`, `MC`, `EE`). The design criteria "by jurisdiction" (a speed default, the u-turn rule, the uncontrolled rule) read it |
| `driving_side` | VARCHAR | `right` or `left`. Decides on which side of a shared reference line a carriageway's lanes lie, and the sense of `lane_num` |
| `osm_timestamp` | TIMESTAMP | The timestamp of the OSM extract |

## roundabout

A group, not a junction: a closed ring of one-way carriageways, a junction at each arm, and a central island.

| Column | Type | Holds |
|---|---|---|
| `roundabout_id` (key) | BIGINT | `hash("{ring_carriageway_ids sorted, joined by ','}")` |
| `ring_carriageway_ids` | BIGINT[] | The ring pieces in travel order, starting at the smallest id |
| `junction_ids` | BIGINT[] | The ring nodes, in the same order; each a `junction` of kind `roundabout_node` |
| `arm_count` | INTEGER | The number of carriageways that enter or leave the ring |
| `centre` | GEOMETRY (POINT) | POINT. The centre of the circle fitted to the ring's reference lines |
| `icd_m` | DOUBLE | The inscribed circle diameter: twice the distance from `centre` to the outer kerb line, or to the outer lane border where no kerb line exists |
| `ring_lane_count` | INTEGER | The lanes of the ring pieces (the largest count when they differ) |
| `central_island_id` | BIGINT | The island of kind `central`, or NULL for a painted centre |
| `level` | INTEGER | The level of the ring (all ring pieces share it) |
| `case_id` | VARCHAR | A derived case of scope `roundabout`: `RAB.single` or `RAB.multi` by the ring lanes (../5-cases/index.md); signals, a bypass and turbo markings are parameters, and a mini-roundabout is a junction case. NULL when unmatched. Written by `case-matcher` |
| `case_score` | DOUBLE | As on [junction.md](#junction) |
| `case_alternatives` | STRUCT(case_id VARCHAR, score DOUBLE) | As on [junction.md](#junction) |

And the five [evidence](#evidence) columns.

## rule_attribute

The rules that resolve the attributes of a carriageway.

| Column | Type | Holds |
|---|---|---|
| `rule_name` (key) | VARCHAR | The rule, as [attribute.md](#attribute) names them: `lanes_from_turn_lanes`, `lanes_from_width`, `oneway_from_twin`, … The primary key |
| `name` | VARCHAR | The attribute the rule writes |
| `reads` | VARCHAR[] | The attributes and tables the rule reads (`turn:lanes`, `kerb_profile`, `twin`) |
| `order` | INTEGER | The order in which `attributes` tries the rules for one name; the first that fires wins |
| `pass` | INTEGER | 1 when the rule can run from the tags alone, 2 when it needs the kerb profile |
| `confidence` | DOUBLE | The `ev_confidence` a value from this rule gets, fixed per rule |
| `network_fix` | BOOLEAN | Whether a value from this rule changes the network |
| `clause` | VARCHAR | Why the rule holds: a standard's clause, a mapping convention, or a measured error rate on a golden area |
| `source` | VARCHAR | Where the knowledge comes from Values: `standard`, `osm-wiki`, `netconvert`, `urbanstyle`, `observation`, `review`. |
| `status` | VARCHAR | `adopted`, `candidate` or `rejected`, as in "Source and status" below |

## rule_case

The case signatures that the case matcher compares with each junction and each lane change.

| Column | Type | Holds |
|---|---|---|
| `case_id` (key) | VARCHAR | The derived case id (level 5). The primary key |
| `scope` | VARCHAR | `transition`, `section`, `junction` or `roundabout` |
| `family` | VARCHAR | The case family (level 5) |
| `kinds` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `lane_delta` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `width_change` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `opens` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `closes` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `kinds_start` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `kinds_end` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `node_count` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `arm_count` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `through` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `sides` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `offset` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `inner` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `ring` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `angle` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `ring_lanes` | VARCHAR | The signature columns. One column per parameter of the scope, as ../5-cases/index.md fixes them (`kinds`, `lane_delta`, `width_change`, `opens`, `closes` for a transition; `kinds_start`, `kinds_end`, `width_change` for a section; `node_count`, `arm_count`, `through`, `sides`, `offset`, `inner`, `ring`, `angle` for a junction, each computed by `junction-cut`; `ring_lanes`). Before level 5 the frame was: for a section, the event kinds at the start and the end, the lane count change, the width change (widens, stays, narrows, unknown), the strips that open or close; for a junction, the arm count, the control, the level, the islands; for a roundabout, the ring lanes, the diameter class, the arm count. A cell is a value, a range, or `*` for any |
| `w_kinds` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_lane_delta` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_width_change` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_opens` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_closes` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_kinds_start` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_kinds_end` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_node_count` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_arm_count` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_through` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_sides` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_offset` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_inner` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_ring` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_angle` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `w_ring_lanes` | VARCHAR | The weight columns, `w_` and the name of the signature column. One per signature column: the weight of a match on it, 0 to 1. Every row has a weight on every column of its scope except the gates. A `*` cell scores 0 with its weight kept in the sum; a cell that lists every value of its column (`any` on the case pages) scores its weight on every input. The weight columns of `node_count` and `ring` hold `gate`: these two columns have no weight, and a row whose gate cell holds a value matches only an input with that value and is otherwise out, with score 0 (../3-modules/case-matcher.md step 3) |
| `order` | INTEGER | The position of the row in the file; two rules with the same score are decided by it, the first wins |
| `fallback` | BOOLEAN | TRUE on the one rule of scope `section` and the one of scope `junction` that is taken when no rule reaches `case_score_min` (`SEC.uniform`, `JNC.cluster`). Scopes `transition` and `roundabout` have no fallback rule: with no rule at `case_score_min`, `case_id` is NULL |
| `writes` | VARCHAR | The parameters the case gives the later modules, as `key=value` pairs from this list: `open_role`, `close_role`, `open_side`, `close_side`, `grow_role`, `shrink_role` (the lanes that open, close, widen or narrow and their side), `taper` (`smoothstep`, `linear`), `taper_at` (`start`, `end`, `kerb`), `flare=scale`, `shift` (`kerb`), `connector` (`corner`, `slip_lane`, `ring`), `inner_piece`, `slip_piece` (the pieces the connectors need; the through pair is `junction.through_arms` from `junction-cut` step 6a). The cell can be empty (`SEC.uniform`). Before level 5 the frame was: the roles that open or close, the taper shape, the connector rule (`slip_lane`, `ring`, `corner`) |
| `example_ref` | VARCHAR | One real example as `"{source_id}:{object}"` from a golden area, O4) |
| `source` | VARCHAR | Where the knowledge comes from Values: `standard`, `osm-wiki`, `netconvert`, `urbanstyle`, `observation`, `review`. |
| `status` | VARCHAR | `adopted`, `candidate` or `rejected`, as in "Source and status" below |

## rule_design

The register of the design rules.

| Column | Type | Holds |
|---|---|---|
| `rule_id` (key) | VARCHAR | `GD.<slug>` (`GD.radius-min`, `GD.k-crest`). The primary key |
| `topic` | VARCHAR | `horizontal`, `vertical`, `cross_section`, `sight`, `intersection`, `roundabout`, `interchange`, `roadside`, `structure` |
| `statement` | VARCHAR | One sentence: what must hold |
| `equation` | VARCHAR | The equation where one exists, with its symbols named; NULL for a table rule |
| `criteria` | VARCHAR[] | The `design_criterion` names the rule reads |
| `clause` | VARCHAR | The standard's edition and section (`TAC 2017 3.2.3`, `AASHTO 2018 9.6`), never the HTML guide |
| `role` | VARCHAR | `check` (writes a check result), `fill` (gives an `assumed` value), `both` |
| `scope` | VARCHAR | What one application of the rule looks at: `vertex` (of a reference line), `connector`, `section`, `junction`, `roundabout`, `span`, `node` |
| `check_id` | VARCHAR | The level-6 check the rule writes, or NULL for `fill` |
| `reads` | VARCHAR[] | The modules or algorithms that apply it (`vertical-alignment`, `curb-radius-fit`, `width-profiles`, `checks`) |
| `example_ref` | VARCHAR | One place in a golden area where the rule fires or fills, as `"{source_id}:{object}"`, or NULL until the golden build |
| `source` | VARCHAR | Where the knowledge comes from Values: `standard`, `osm-wiki`, `netconvert`, `urbanstyle`, `observation`, `review`. |
| `status` | VARCHAR | `adopted`, `candidate` or `rejected`, as in "Source and status" below |

## signal_phase

One phase of a signal plan, and the movements that it serves.

| Column | Type | Holds |
|---|---|---|
| `signal_phase_id` (key) | BIGINT | `hash("{signal_plan_id}\|{phase_num}")` |
| `signal_plan_id` | BIGINT | The plan |
| `phase_num` | INTEGER | 1 to n, in cycle order |
| `green_s` | DOUBLE | The green time |
| `yellow_s` | DOUBLE | The yellow time |
| `red_clearance_s` | DOUBLE | The all-red time after it |
| `movement_ids` | BIGINT[] | The movements that have green in this phase |
| `protected` | BOOLEAN | TRUE when the phase's turns face no conflicting movement with green (a protected left) |
| `ring` | INTEGER | The NEMA ring, 1 or 2, or NULL when the controller is not dual-ring |

## signal_plan

The controller of a signalled junction.

| Column | Type | Holds |
|---|---|---|
| `signal_plan_id` (key) | BIGINT | `hash("{junction_id}\|{source_id}")` |
| `junction_id` | BIGINT | The junction |
| `controller_type` | VARCHAR | `fixed_time`, `actuated`, `coordinated`, `unknown` |
| `cycle_s` | DOUBLE | The cycle length, or NULL |
| `offset_s` | DOUBLE | The offset from the coordination reference, or NULL |
| `source_id` | VARCHAR | The source, a row of [source.md](#model_override): a city's timing data, the OSM extract, the design profile, a decision model |
| `method` | VARCHAR | How the plan was read: `timing_sheet`, `osm_tags` (the rare `traffic_signals:*` tags), `default_plan` (the design profile's plan for the junction's arm count and turn set), `model` |
| `source_ref` | VARCHAR | The object in the source: the sheet and row, the way id, the criterion name, the request id, as `"{source_id}:{object}"` |

And the five [evidence](#evidence) columns.

## source

A registered data source: an OSM extract, a city dataset, a LiDAR survey, a DEM, a model, and more.

| Column | Type | Holds |
|---|---|---|
| `source_id` (key) | VARCHAR | A short name written by hand, with no colon: `osm-2026-10-20`, `vancouver-gis-2025`, `lidar-vancouver-2022`, `mapillary-2026-q3`, `survey-granville-2026-04`, `jev-latest`, `profile-project-default` |
| `kind` | VARCHAR | `osm`, `city_gis`, `dem` (a height raster: a global DEM, a national DTM or DSM), `lidar`, `imagery_aerial`, `imagery_street`, `survey`, `sensor`, `model` (a decision model), `derived` (another program's output read as a source: urbanstyle's bridge profile; urbanstyle's faces for the comparison), `profile`, `editor` (the edits of `overrides/model_overrides.yaml`, and the OSM fixes file of pass 0, `osm-overrides`: both are corrections that a person wrote by hand; always first in `source_priority`) |
| `title` | VARCHAR | The dataset's own name |
| `provider` | VARCHAR | Who made it: `OpenStreetMap contributors`, `City of Vancouver`, `BC MoTI`, `Mapillary`, `TypeSafe AI` |
| `acquired_at` | DATE | When the data was captured or published, or NULL |
| `imported_at` | TIMESTAMP | When the build read it |
| `crs_native` | VARCHAR | The CRS of the data as delivered; the import module reprojects to `project.crs` |
| `coverage` | GEOMETRY (POLYGON) | POLYGON. Where the source has data, in `project.crs`, or NULL for the whole area |
| `licence` | VARCHAR | `ODbL`, `OGL-Vancouver`, `CC-BY-4.0`, a contract reference |
| `vertical_datum` | VARCHAR | For a source of heights: the datum of its heights as delivered (`EGM2008`, `EGM96`, `CGVD2013`, `RH2000`); `import-elevation` converts to `project.vertical_datum`. NULL for other sources |
| `geoid_grid` | VARCHAR | The two grid files the conversion needs when the datums differ, `from;to` in the area folder (the grid of the source's datum, then of the project's; for EGM2008 to CGVD2013, the cuts of `us_nga_egm08_25` and `ca_nrc_CGG2013an83`), or NULL |
| `vertical_accuracy_m` | DOUBLE | For a source of heights: its vertical error, one sigma (about 4 m for Copernicus GLO-30, about 2.5 m for FABDEM, 0.15 m for a LiDAR DTM, 0.02 m for a survey). The weight of its rows in the vertical-alignment fit ([elevation-profile.md](#elevation_profile)). NULL for other sources |
| `reference` | VARCHAR | A URL or a file path |
| `note` | VARCHAR | Free text |

## stop_line

Where an arm ends and the junction starts: the line through the cut points of the arm's lanes.

| Column | Type | Holds |
|---|---|---|
| `stop_line_id` (key) | BIGINT | `hash("{carriageway_id}\|{junction_id}\|{end}")` |
| `carriageway_id` | BIGINT | The arm |
| `junction_id` | BIGINT | The junction |
| `end` | VARCHAR | `in`: the carriageway enters the junction here (near its `length_m`). `out`: the carriageway leaves the junction here (near station 0) |
| `station_m` | DOUBLE | The mean of the lanes' cut stations. The `stop_line` event of the arm is at this station |
| `station_min_m` | DOUBLE | The smallest cut station of the arm's lanes |
| `station_max_m` | DOUBLE | The largest |
| `geometry` | GEOMETRY (LINESTRING) | LINESTRING. From the left border of lane 1 to the right border of the last lane, through the cut point of each lane border |
| `lane_cuts` | STRUCT(lane_id BIGINT, station_m DOUBLE, point GEOMETRY) | The cut of each lane, left to right. The same values as the lanes' cut columns, kept here for the connectors and the checks |
| `method` | VARCHAR | How the cuts were found. `lane_border`: the intersection of the arm's outer lane borders with the neighbouring arms' borders or the kerb corners (the normal case). `crossing`: pushed back to the far edge of a crossing within `design_criterion` `stop_line_crossing_search_m` of the core, plus `stop_line_crossing_setback_m`. `stop_node`: a `highway=stop` or `traffic_signals;stop_line` node on the arm. `measured`: a stop bar from city data, LiDAR or an image. `setback`: `stop_line_default_setback_m` past the arm's own node (the node at which the carriageway ends), measured along its reference line, square to the road, when the arm has no corner on either side (both arms of a continuation, the entering arm of a Y diverge) |
| `source_ref` | VARCHAR | The object that placed the line when `method` is `crossing`, `stop_node` or `measured`, as `"{source_id}:{object}"`. NULL otherwise |

And the five [evidence](#evidence) columns.

## street_face

The street space of one group at one level: the area that the roadways, sidewalks and verges can fill, up to the bounds.

| Column | Type | Holds |
|---|---|---|
| `street_face_id` (key) | BIGINT | `hash("simready\|{face_id}")` |
| `face_id` | VARCHAR | `g0` for the ground; `<letter><level>-<group_id>` for a group, the letter `b` for a bridge chain, `t` for a tunnel chain, `r` for a ring |
| `kind` | VARCHAR | `ground`, `bridge`, `tunnel`, `roundabout` |
| `level` | INTEGER | The level of the face |
| `group_id` | BIGINT | `structure_group_id` or `roundabout_id`, or NULL for the ground |
| `geometry` | GEOMETRY (POLYGON) | POLYGON or MULTIPOLYGON, in metres |
| `open_side_count` | INTEGER | The number of maximal runs of open stations, per side of a carriageway, in the face: the bound there is the default. The runs themselves are the bound pieces with `ev_method` `open-side` |

And the five [evidence](#evidence) columns.

## structure

A span: a bridge, a tunnel or another structure that carries one carriageway over a station range.

| Column | Type | Holds |
|---|---|---|
| `structure_id` (key) | BIGINT | `hash("{carriageway_id}\|{rank}")`, `rank` the span's number on the carriageway in station order from 1 |
| `carriageway_id` | BIGINT | The carriageway |
| `structure_group_id` | BIGINT | The physical structure this span is part of. See [structure-group.md](#structure_group) |
| `station_start_m` | DOUBLE | Where the span begins |
| `station_end_m` | DOUBLE | Where it ends |
| `kind` | VARCHAR | `bridge`, `viaduct`, `tunnel`, `building_passage`, `culvert`, `covered` (a road under a building or a cover), `embankment`, `cutting` |
| `level` | INTEGER | The level of the carriageway on the span: the resolved `layer` (a non-zero `layer`, else 1 for a bridge, −1 for a tunnel, else 0, the band of the edge's tags). 0 is the ground |
| `source_ref` | VARCHAR | The way, as `"{source_id}:{object}"` |
| `clearance_m` | DOUBLE | The vertical clearance under a bridge or in a tunnel (OSM `maxheight`, a survey), or NULL |
| `width_m` | DOUBLE | The deck or bore width at this span where a source gives it (OSM `width`, the group's measured profile, LiDAR), or NULL. `import-kerb` makes the kerb lines of the span from it when no kerb source covers the span |
| `twin_structure_id` | BIGINT | The same span under the twin carriageway (one deck, two directions), or NULL |

And the five [evidence](#evidence) columns.

## structure_group

One physical structure: the connected spans of a bridge system or a tunnel system.

| Column | Type | Holds |
|---|---|---|
| `structure_group_id` (key) | BIGINT | `hash("{structure_ids sorted, joined by ','}")` |
| `structure_ids` | BIGINT[] | The spans, sorted |
| `kind` | VARCHAR | `bridge` or `tunnel`: the kind of most of its spans' length. A group never mixes a bridge and a tunnel |
| `levels` | INTEGER[] | The distinct levels of the spans, sorted. One level for a plain bridge or tunnel; two or more for a double-deck bridge, a stacked interchange, a two-level tunnel (one tube over the other), an underground interchange, or a ramp that climbs or descends between decks or tubes. The level itself is on each span |
| `name` | VARCHAR | The structure's name from OSM or a city register, or NULL |
| `source_ref` | VARCHAR | The `type=bridge` or `type=tunnel` relation, as `"{source_id}:{object}"`, or NULL when the group was formed by connectivity alone |
| `length_m` | DOUBLE | The sum of the spans' lengths |
| `span_count` | INTEGER | The number of spans |
| `profile_source_id` | VARCHAR | The source of the measured deck or bore profile (urbanstyle's bridge profile from LiDAR, a city survey), or NULL |
| `height_source_id` | VARCHAR | The source of the road height on the group, or NULL when the elevation profile on it is `assumed` |

And the five [evidence](#evidence) columns.

## turn_rule

One turn of a junction, from an entering carriageway to a leaving one: allowed or not, and why.

| Column | Type | Holds |
|---|---|---|
| `turn_rule_id` (key) | BIGINT | `hash("{from_carriageway_id}\|{to_carriageway_id}")` |
| `junction_id` | BIGINT | The junction |
| `from_carriageway_id` | BIGINT | The arm that enters |
| `to_carriageway_id` | BIGINT | The arm that leaves |
| `turn` | VARCHAR | `left`, `straight`, `right`, `u`, `slight_left`, `slight_right`, from the angle between the arms with the design profile's thresholds. A merge or a diverge keeps its angle kind here and sets `fork` |
| `fork` | BOOLEAN | TRUE when the junction has more than one entering arm and one leaving arm (a merge), or one entering arm and more than one leaving arm (a diverge), counted without the twin of a u-turn pair, and the lanes continue from the entering arms into the leaving ones. FALSE otherwise. `turn-rules` maps the arrows of a fork pair by its side (../3-modules/turn-rules.md step 3), and `connectors` gives its lane pairs by the fan rule (../3-modules/connectors.md step 1) |
| `allowed` | BOOLEAN | Whether vehicles can make this turn |
| `allowed_uses` | VARCHAR[] | For which uses, when a restriction names one (`except=bus`): the uses the turn is open to; empty list when `allowed` is FALSE for all |
| `method` | VARCHAR | What decided: `osm_restriction` (a `type=restriction` relation, through duckOSM's `edge_graph`), `turn_lanes` (the markings of the entering arm: no arrow for this turn on any lane), `oneway` (the leaving arm is one-way against the turn), `geometry` (the turn is sharper than `turn_angle_max_deg` into a lower road class), `uturn_rule` (the jurisdiction's u-turn rule), `uturn_no_exit` (the u-turn into the entering road's own twin, allowed because the road has no other allowed turn: ../3-modules/turn-rules.md, rule 9), `roundabout` (a pair of a roundabout node whose two carriageways are both not on the ring), `access` (the arms have no lane use in common), `override` (`osm_overrides.yaml`), `default` (nothing forbids it, so it is allowed) |
| `source_ref` | VARCHAR | The relation, the way, the override entry or the model request, as `"{source_id}:{object}"` |
| `network_fix` | BOOLEAN | TRUE when `allowed` differs from what duckOSM's `edge_graph` has: the rule is written to the overrides file for pass 0, because the graph must change |

And the five [evidence](#evidence) columns.
