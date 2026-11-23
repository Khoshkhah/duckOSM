# HD map database (`<area>_hdmap.duckdb`)

Status: design, waits for sign-off. No code yet.

| Date | Change |
|---|---|
| 2026-11-23 | first version: schema `hd_<mode>` in the GMNS database, tables `lanelet`, `boundary`, … |
| 2026-11-23 | a database of its own, not a schema in the GMNS database (Kaveh) |
| 2026-11-23 | the tables are the core of the SimReady road model, with a small number of changes, so that one database gives each HD map format (Kaveh: "the best version, more general, that can export every HD map") |

The full specification of each table and column: [`docs/reference/hdmap-database.md`](../reference/hdmap-database.md).
This page gives the reasons.

## The problem

Level 4 (HD map) has four outputs today. Each output reads the GMNS database and makes its own lane geometry:

| Output | Reads | Makes by itself |
|---|---|---|
| `to_lanelet2` (`lanelet2.py`) | `gmns_<mode>.lane.geom`, `link.free_speed` | left and right borders (`_offset_wkt` of `geom`, to the node), node snap on a 1 cm grid |
| `to_commonroad` (`hdmap.py`) | `lane.geom_cut`, `lane_connector` | borders (shapely `offset_curve` of `geom_cut`), successors, neighbours, ids `1..n` |
| `to_opendrive --junctions` (`opendrive.py`) | `link`, `movement` | one connecting road for each movement, with 1 lane; each lane 3.25 m |
| `lane_graph` (`lane_routing.py`) | `lane`, `movement` | lane changes from `lane_num`, turns from the lane ranges |

So the three files do not agree; the `lanelet2` router finds a next lanelet for only 732 of 3318 lanelets on Monaco;
no output has stop lines, signals, signs or road marks; and the CommonRoad ids change when one lane is added.

## The decision

One database for the lane-level road model of an area, `<area>_hdmap.duckdb`. Each exporter reads only this database
and writes only its format.

The tables are the core of the SimReady road model (`/home/kaveh/projects/simready`, `docs/2-model/`), with the
changes in [Changes to the SimReady model](#changes-to-the-simready-model). Two programs can write the database:

| Producer | How |
|---|---|
| `duckosm hdmap` (this design) | from the GMNS database: OSM center lines offset by the lane widths, junction paths from SUMO's netconvert |
| SimReady | one more adapter that copies its own tables (`sim.*`) into the database: measured kerbs, tapers, its own junction paths and areas, heights along the road |

The duckOSM exporters (Lanelet2, OpenDRIVE, CommonRoad, later SUMO and glTF) read the database from either producer.
A better producer gives better files, with no change to an exporter.

```
<area>.duckdb (Level 2) --duckosm gmns--> <area>_gmns.duckdb (Level 3) --duckosm hdmap--> <area>_hdmap.duckdb (Level 4)
                                                                  SimReady build --adapter--> <area>_hdmap.duckdb
                                                                                                  |--> Lanelet2, OpenDRIVE,
                                                                                                  |    CommonRoad, SUMO, glTF
                                                                                                  '--> lane graph, surfaces
```

## Why the SimReady model

I compared three candidates against the needs of the formats.

| Need of a format | First design (`lanelet`, `boundary`) | SimReady model | Comment |
|---|---|---|---|
| OpenDRIVE: reference line, lane sections, lane widths along `s` | no: only borders | yes: `carriageway.reference_line`, `lane_section`, `width_start_m`, `width_end_m`, taper | to get widths back from borders is hard. To get borders from widths is easy |
| Lanelet2, CommonRoad: left and right borders | yes | yes: `boundary_left`, `boundary_right` | |
| a lane that opens or ends (turn pocket, merge) | no | yes: `role`, sections, `predecessor_lane_id` | OpenDRIVE and SUMO need it |
| paths through a junction, with conflicts | lanelet of kind `connector` | `movement` with `path`, `area`, `conflict_movement_ids`, `priority` | SUMO `foes`, Lanelet2 `right_of_way` |
| junction surface | no (a point) | `junction.shape`, `area`, `island` | SUMO junction shape, glTF, OpenDRIVE 1.8 boundary |
| stop lines | one rule | `stop_line` with the cut of each lane | all formats |
| signal timing | no | `signal_plan`, `signal_phase` | SUMO `tlLogic`, OpenDRIVE `controller` |
| heights | two values on each row | `elevation_profile`, Z on the borders and paths | OpenDRIVE `elevationProfile`, Lanelet2 `ele` |
| bridges, tunnels | `level` only | `structure` | OpenDRIVE objects, glTF |
| where a value comes from | no | `ev_*` on each row | an `assumed` width must not look `observed` |
| stable ids | hash of text keys | hash of content keys (SimReady D7), the `edge_id` formula | same rule |
| road marks | yes | no | added |
| crosswalks | yes | no | added |
| signs and lights as things at a position | partly | no (only the rules) | added |
| tested | no | yes: the SimReady Lanelet2 map of Monaco loads in `lanelet2` 1.2.3 with 0 errors and 0 routing errors; the adapters for SUMO, OpenDRIVE, GMNS, MATSim, Lanelet2 and glTF read it | |

Other models that I looked at:

| Model | Why not |
|---|---|
| ASAM OpenDRIVE as tables | one format's model: the borders are not stored, so Lanelet2 and CommonRoad must compute them |
| Lanelet2 as tables (points, ways, relations) | the shared nodes are a detail of the file format; no reference line or widths for OpenDRIVE |
| NDS.Live | the same idea (lane groups, borders), but the standard is not open |
| Overture transportation | segments and connectors only, no lane geometry |
| ASAM OSI | an interface between simulators while they run, not a stored map |

The SimReady model is the most general of these. It also has the same id rule and the same lane order (lane 1 is the
leftmost) as duckOSM.

## Changes to the SimReady model

The HD map database takes the tables that describe the road. It does not take the tables of the SimReady build (the
rules, the cases, the checks).

### Tables that the HD map database does not take

| SimReady table | Why not |
|---|---|
| `attribute`, `event` | build steps: the resolved OSM values and the cut points along a road. The result is in `lane`, `lane_section` |
| `kerb_profile`, `kerb_line`, `street_face` | the street space that SimReady measures the lanes in. The result is in the lane borders |
| `design_profile`, `design_criterion`, `rule_attribute`, `rule_case`, `rule_design` | the rules of the build |
| `check_result`, `loss_report` | build and export reports |
| `model_override` | the edits of a person, an input of the build |
| `roundabout`, `structure_group` | groups for the build. An exporter does not need them |

### Columns that the HD map database does not take

| Table | Columns | Why not |
|---|---|---|
| `carriageway` | `roundabout_id` | the table `roundabout` is not taken |
| `lane_section` | `event_start_id`, `event_end_id`, `case_id`, `case_score`, `case_alternatives`, `change_reason`, `kerb_width_start_m`, `kerb_width_end_m` | build details |
| `junction` | `through_arms`, `through_kind`, `side_kinds`, `offset`, `inner_kind`, `fork_angle`, `case_id`, `case_score`, `case_alternatives`, `island_count`, `roundabout_id` | the case of the junction, for the build |
| `stop_line` | `station_min_m`, `station_max_m` | in `lane_cuts` |
| `island` | `roundabout_id`, `case_id` | build details |
| `structure` | `structure_group_id` | the group table is not taken |
| `project` | `urbanstyle_version`, `design_profile_id` | build details |

### Additions

| Table | Addition | Why |
|---|---|---|
| `project` | `ground_id`, `ground_product` | the ground rule: each product names its ground |
| `project` | `producer` | `duckosm-gmns` or `simready`: the quality of the file |
| `project` | `driving_side` | in the SimReady pages, not in its `schema.sql` |
| `lane` | `mark_left`, `mark_right`, `mark_source` | road marks: Lanelet2 way subtypes, OpenDRIVE `roadMark`, CommonRoad `lineMarking`. SimReady's Lanelet2 adapter makes them with a rule; now the rule writes a column and each exporter uses it |
| `lane` | `role`, `width_shape` | on the SimReady page `lane.md`, not in its `schema.sql` |
| `movement` | `turn`, `priority` | on the SimReady page `movement.md`, not in its `schema.sql` |
| `stop_line` | `painted` | an `out` line is not painted, and a stop line at a junction with no control is not painted |
| new table | `crossing` | Lanelet2 crosswalk lanelets, OpenDRIVE crosswalk objects, SUMO crossings |
| new table | `signal` | the light or the sign as a thing at a position, with its code: OpenDRIVE `signal`, Lanelet2 `traffic_light` and `traffic_sign`, CommonRoad `trafficSign` |

SimReady's `schema.sql` (2026-11-17) and its pages (2026-11-07) do not agree on `lane.role`, `lane.width_shape`,
`movement.turn` and `movement.priority`. This design takes the columns of the pages. SimReady must correct its
`schema.sql` before its adapter can copy these columns.

## Ids

| Rule | Why |
|---|---|
| `(hash(<text>) >> 1)::BIGINT`, the text forms of the SimReady pages | the same rule as duckOSM's `edge_id` and SimReady D7. It holds while the `duckdb<2` pin holds |
| `carriageway_id = hash("{edge_ids joined by ','}")` | `duckosm-gmns` has one edge for each carriageway, so the id is the hash of the `edge_id` as text. It is not the `edge_id` itself, because a SimReady carriageway can have more than one edge |
| no row number, no sequence | a downstream join must not break when one lane is added |
| the two producers give different ids for one area | SimReady joins edges into carriageways and cuts sections at tapers. A user joins the two on `carriageway.edge_ids` |

## Where the database is

| Rule | Why |
|---|---|
| a file of its own, not a schema in the GMNS database | Kaveh: the GMNS database must not become heavy and complicated |
| complete alone | a user can copy or share the file without the GMNS database |
| one schema `hdmap`, with `carriageway.mode` | the SimReady model has one schema with a `mode` column. A copy from SimReady needs no change |
| metres in a UTM zone (`project.crs`) | the SimReady model, OpenDRIVE and CommonRoad use metres. Only Lanelet2 needs longitude and latitude, and its exporter transforms the points |

## Heights

The heights come from the area's one ground (`docs/concepts/ground-reference.md`), never from another source.

| Producer | Heights |
|---|---|
| `duckosm-gmns` | two `elevation_profile` rows for each carriageway: `edges.z_from` and `z_to` of the Level 2 database (`--source`), at stations 0 and `length_m`. The Z of the borders and the paths is linear between them. None on a bridge or in a tunnel |
| SimReady | its `elevation_profile`: the conformed road heights, and the heights of the bridge decks |

`project.ground_id` holds the `sha256` of the ground. Each exporter writes it into its file.

## How the exporters change

| Exporter | Change |
|---|---|
| `to_lanelet2` | reads `lane`, `movement`, `stop_line`, `crossing`, `signal`. It takes the rules of SimReady's Lanelet2 adapter (shared nodes within 1 cm, movement lanelets that start and end on the end nodes of their lanes, regulatory elements) |
| `to_opendrive` | with the HD map database: roads from `carriageway`, sections and widths from `lane_section` and `lane`, junctions from `movement`. With a Level 2 database: as today, roads only |
| `to_commonroad` | reads `lane`, `movement`, `stop_line`, `signal`. The ids are the `lane_id` and the `movement_id` |
| `lane_graph`, `route_lanes` | turns from `movement`, lane changes from lanes of one section with `lane_num` ± 1 and a `dashed` mark between them |
| `surfaces` | the lane areas, `movement.area`, `junction.area` |
| SUMO, glTF | later. SimReady has these adapters; duckOSM can take them when it needs them |

An exporter that gets a GMNS database first writes the HD map database to a temporary folder, as `to_opendrive` does
with the GMNS step today.

The Rust exporters (`rust/duckosm-export/src/lanelet2.rs`, `opendrive.rs`) can read the same tables later. This design
does not change them.

## Rejected alternatives

| Alternative | Why not |
|---|---|
| the first design (`lanelet`, `boundary`, `lanelet_relation`, `marking`, `regulatory`) | no reference line, no lane sections and no widths along the road, so no good OpenDRIVE; no junction area; no signal timing; no evidence |
| a schema `hd_<mode>` in the GMNS database | the GMNS database gets heavy. Kaveh's decision |
| the whole SimReady schema | the build tables (rules, cases, kerbs, checks) are not a map. They are of no use to an exporter |
| SimReady as the only producer | SimReady is a private test project. Public duckOSM users need an HD map from OSM alone |
| a model of our own, different from SimReady | two models of one road, two sets of exporters, and files that do not agree |
| `LINESTRING` without Z, heights in columns | the SimReady model has Z on the borders and the paths. Lanelet2 needs a height on each point |
| store a shared border once (a `boundary` table) | SimReady stores the two borders on each lane. The Lanelet2 exporter joins points within 1 cm, so the file shares them. One row for each lane is simpler to read |

## Tests (on `config/sample_monaco.yaml`)

| Test | Passes when |
|---|---|
| the checks of the reference page | each check passes on Monaco |
| stable ids | two builds give the same ids |
| ids from SimReady's text forms | an id made in Python equals the id of the same text in DuckDB |
| Lanelet2 | the `lanelet2` library loads the file; its routing graph finds a next lanelet for more lanelets than today (732 of 3318) |
| OpenDRIVE, CommonRoad | the files pass their current tests |

## Steps after sign-off

1. `src/duckosm/hdmap_build.py`: `build(gmns_db, out=None, mode="driving", source=None)`, CLI `duckosm hdmap`. The DDL
   in one SQL file, `src/duckosm/sql/hdmap_schema.sql`.
2. Tests: `tests/test_hdmap_build.py`.
3. The exporters read the database: Lanelet2 and CommonRoad first, then OpenDRIVE, then the lane graph and
   `surfaces`.
4. Docs: the reference page goes into the site menu; `docs/exports/lanelet2.md`, `opendrive.md`, `lane-routing.md`,
   `docs/reference/sdk/hdmap.md`, `docs/compare/for-hdmap-users.md`.
5. SimReady (its own session, its own repository): an adapter that writes the HD map database, and the fix of its
   `schema.sql`.

## Questions for Kaveh

1. Is the SimReady core, with the changes above, the HD map database?
2. Can the HD map tables be public in duckOSM? The SimReady repository is private. This design copies its model into
   duckOSM's public docs and code.
3. Schema name `hdmap` (not `sim`): OK?
