# HD map database (`<area>_hdmap.duckdb`)

Status: design, waits for sign-off. No code yet.

| Date | Change |
|---|---|
| 2026-11-23 | first version: schema `hd_<mode>` in the GMNS database, tables `lanelet`, `boundary`, … |
| 2026-11-23 | a database of its own, not a schema in the GMNS database (Kaveh) |
| 2026-11-23 | the tables are the core of the SimReady road model, so that one database gives each HD map format (Kaveh: "the best version, more general, that can export every HD map") |
| 2026-11-23 | bridge and tunnel heights by the SimReady chain rule (Kaveh) |
| 2026-11-24 | driving only; one carriageway for each chain of `driving.chains`, the same id as SUMO `--merge` (Kaveh); the fixes of the review (below) |

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

So the three files do not agree; `docs/exports/lanelet2.md` reports that the `lanelet2` router finds a next lanelet for
only 732 of 3318 lanelets on Monaco (an older build); no output has stop lines, signals, signs or road marks; and the
CommonRoad ids change when one lane is added.

## The decision

One database for the lane-level road model of the driving network of an area, `<area>_hdmap.duckdb`. Each exporter
reads only this database and writes only its format.

| Decision | Reason |
|---|---|
| a file of its own | Kaveh: the GMNS database must not become heavy and complicated. The file is complete alone |
| the tables of the SimReady road model, with changes | the most general model that I found (next section) |
| driving only | an HD map is for automated driving. The bike and bus lanes on the road, and the crosswalks, are in the driving data. The lane paths through the junctions exist only for driving |
| one carriageway for each chain of `driving.chains` | the edges are cut at each node that another network uses (a footway crossing). An HD map must not show these cuts as junctions. The chains are the rule that SUMO `--merge` already uses, so the two exports have the same roads and the same ids |
| two producers | `duckosm hdmap` from the GMNS database; SimReady from its own build, later |

```
<area>.duckdb (Level 2: driving.chains, heights) --+
                                                   +--duckosm hdmap--> <area>_hdmap.duckdb --> Lanelet2, OpenDRIVE, CommonRoad
<area>_gmns.duckdb (Level 3: lanes, paths) --------+                       ^                   later SUMO, glTF
                                                   SimReady build --adapter-'                   lane graph, surfaces
```

## Why the SimReady model

| Need of a format | First design (`lanelet`, `boundary`) | SimReady model |
|---|---|---|
| OpenDRIVE: reference line, lane sections, lane widths along `s` | no: only borders | yes: `carriageway.reference_line`, `lane_section`, `width_start_m`, `width_end_m`, taper |
| Lanelet2, CommonRoad: left and right borders | yes | yes: `boundary_left`, `boundary_right` |
| a lane that opens or ends (turn pocket, merge) | no | yes: `role`, sections, `predecessor_lane_id` |
| paths through a junction, with conflicts | partly | `movement` with `path`, `area`, `conflict_movement_ids`, `priority` |
| junction surface | no | `junction.shape`, `area`, `island` |
| stop lines | one rule | `stop_line` with the cut of each lane |
| signal timing | no | `signal_plan`, `signal_phase` |
| heights | two values on each row | `elevation_profile`, Z on the borders and paths |
| bridges, tunnels | `level` only | `structure` |
| where a value comes from | no | `ev_*` on each row |
| stable ids | hash of text keys | hash of content keys (SimReady D7), the `edge_id` formula |
| tested | no | the SimReady Lanelet2 map of Monaco loads in `lanelet2` 1.2.3 with 0 errors and 0 routing errors |

Other models that I looked at: OpenDRIVE as tables (no stored borders), Lanelet2 as tables (no reference line or
widths), NDS.Live (not open), Overture transportation (no lane geometry), ASAM OSI (an interface between simulators,
not a stored map).

## Changes to the SimReady model

The HD map database takes the tables that describe the road, not the tables of the SimReady build.

### Not taken

| SimReady | Why not |
|---|---|
| tables `attribute`, `event`, `kerb_profile`, `kerb_line`, `street_face`, `design_profile`, `design_criterion`, `rule_attribute`, `rule_case`, `rule_design`, `check_result`, `loss_report`, `model_override`, `roundabout`, `structure_group` | the build: resolved tags, cut points, kerbs, rules, checks, edits, groups. The result is in the map tables |
| `carriageway.roundabout_id`, `island.roundabout_id`, `junction.roundabout_id` | the table `roundabout` is not taken |
| `lane_section.event_start_id`, `event_end_id`, `case_id`, `case_score`, `case_alternatives`, `change_reason`, `kerb_width_start_m`, `kerb_width_end_m` | build details |
| `junction.through_arms`, `through_kind`, `side_kinds`, `offset`, `inner_kind`, `fork_angle`, `case_id`, `case_score`, `case_alternatives`, `island_count` | the case of the junction, for the build |
| `island.case_id` | build detail |
| `stop_line.station_min_m`, `station_max_m` | in `lane_cuts` |
| `structure.structure_group_id` | the group table is not taken |
| `turn_rule.network_fix` | a SimReady fix for duckOSM (pass 0) |
| `project.urbanstyle_version`, `design_profile_id` | build details |

All other SimReady columns are taken with their names and values, for example `node.kind`, `junction.kind`,
`island.kind`, `signal_plan.controller_type`, `elevation_profile.method`, `earthwork_m`, the `source` columns,
`project_id = hash("{area_name}|{built_at}")` and `lane_order_convention = 'left_to_right_from_1'`. `lane.role`,
`lane.width_shape`, `movement.turn`, `movement.priority` and `project.driving_side` are in SimReady's `schema.sql`
and are taken as they are.

### Added

| Table | Addition | Why |
|---|---|---|
| `project` | `ground_id`, `ground_product` | the ground rule: each product names its ground |
| `project` | `producer` | `duckosm-gmns` or `simready`: the quality of the file |
| `carriageway` | `edge_start_m` | a value on one edge maps to the exact stations of the carriageway |
| `carriageway` | `lane_offset_m` | OpenDRIVE `laneOffset`. Without it, the lanes of a one-way road move half a road width (Fable: lane 1 is 0.05 m from the link line on one-way roads, 1.48 m on two-way roads) |
| `lane` | `mark_left`, `mark_right`, `mark_source` | road marks: Lanelet2 way subtypes, OpenDRIVE `roadMark`, CommonRoad `lineMarking` |
| `movement` | `boundary_left`, `boundary_right` | Lanelet2 and CommonRoad need the borders of a path. 342 connectors on Monaco have a width different from their lane in, so each exporter would have to make the same snap. One column, one rule |
| `movement` | `yield_to_movement_ids` | `priority` alone does not say who yields to whom (a left turn against the oncoming traffic, both `major`). SUMO `response` and Lanelet2 `right_of_way` need the pairs |
| `stop_line` | `painted`; `end = 'mid'`, `junction_id` empty | 29 of 31 traffic lights on Monaco are in the middle of a link (pedestrian lights). They need a stop line with no junction |
| new table | `crossing` | Lanelet2 crosswalk lanelets, OpenDRIVE crosswalk objects, SUMO crossings |
| new table | `signal` with `heading_deg`, `height_m` | the light or the sign as a thing at a position. A Lanelet2 `traffic_light` refers to a way, not a point: the exporter makes the way from the position, the heading and the height |

## Ids

| Rule | Why |
|---|---|
| `carriageway_id = chain_id` | the same id as the SUMO `--merge` edge. `chain_id` is the `edge_id` for a chain of one edge, else duckOSM's hash of the `edge_ids` (`duckosm-core/src/chains.rs`). It is read from `driving.chains`, not computed again |
| the other ids: `(hash(<text>) >> 1)::BIGINT` with the text forms of the SimReady pages | the rule of `edge_id` and SimReady D7. It holds while the `duckdb<2` pin holds |
| `crossing_id = hash(GMNS crossing_id)` | 10 OSM ids on Monaco have more than one crossing (`w1202361402#2`) |
| `signal_id` from `(kind, osm_id, carriageway_id)` | `(loc_type, osm_id, link_id)` is unique in GMNS `location`. No rank |
| `turn_rule_id` from one distinct pair of carriageways | 105 pairs on Monaco have more than one GMNS movement |
| driving only | 2770 of 2783 driving links are also cycling links: two modes would give the same ids |
| no row number, no sequence | a downstream join must not break when one lane is added |

The two producers give different ids for one area when SimReady joins more than one chain into one carriageway. A user
joins them on `carriageway.edge_ids`. SimReady is asked to build its carriageways from whole chains, so that a
carriageway of one chain has the same id in both (SimReady session `simready-chains`, 2026-11-24).

## Chains

A chain of `driving.chains` joins edges only where no other driving road meets the node, the two edges have the same
road values and lanes, the node has no traffic light or stop sign, and no turn rule names the two edges.

| Effect in the HD map | |
|---|---|
| no cut at a crosswalk or a footway | Monaco: 2840 edges become 1775 SUMO `--merge` edges. Fable found 926 of 1678 GMNS nodes with lane paths that only pass traffic through; most of them go into a chain |
| a cut at a traffic light and where the lanes change | as SUMO. Later, the HD map can join across these points as a new lane section, as SimReady does |
| one lane section for each carriageway | a chain has one set of lanes |
| the GMNS lanes of a chain join end to end | lane `k` of each link of the chain is one lane |
| the GMNS lane paths at nodes inside a chain are not used | they only go straight on |

The HD map uses the merged result only. Two copies of each road (merged and not merged) would put roads on top of each
other in Lanelet2 and OpenDRIVE. `edge_ids` and `edge_start_m` keep the link to each edge.

## Heights

The heights come from the area's one ground (`docs/concepts/ground-reference.md`), never from another source.

| Producer | Heights |
|---|---|
| `duckosm-gmns` | on the ground: `edges.z_vertices` (a height at each vertex), else `z_from` and `z_to`, from the routing database. On a bridge or in a tunnel: the chain rule below |
| SimReady | its `elevation_profile`: the graded road heights, and measured deck heights |

### Bridges and tunnels

I compared the two rules that exist:

| | roadstyle `bridge_profile` | SimReady `import-elevation` |
|---|---|---|
| For | drawing a 3D page | a road model |
| Heights | above the ground (a ground node is 0) | on the vertical datum |
| Nodes inside a chain | default: one height from the length of the group (Granville: 22 m everywhere); rule `design`: the highest that the grade allows | the mean of the neighbours by `1 / length` |
| Crest | in the rule `design` | `bridge_approach_grade_pct`, off by default |
| Clearance | a fixed estimate (5 m plus the deck depth) | over each road that the span crosses, by `level` |
| Measured data | heights that a person gives | LiDAR, DSM, survey, bridge profile first |
| Tunnels | no | yes |

The HD map takes the SimReady rule without the crest and the vertical curves (the reference page, "Bridges and
tunnels"). The review changed four points:

| Point | Change | Why |
|---|---|---|
| chains of spans | bridges and tunnels together; a span is any `level <> 0` | Monaco has 3 nodes where a bridge meets a tunnel with no level-0 road, and 11 links with a `layer` but no `bridge` or `tunnel` tag |
| landings | each node that touches a part with `level = 0` | the same nodes |
| inner nodes | a direct solve of the linear system | a loop of 1000 sweeps leaves 21 cm of error on a line of 100 nodes |
| crest | removed (no `--bridge-grade`) | SimReady has it off by default; a bridge with one landing rose with no limit |

One risk is open. The roadstyle 3D page and the HD map database can show different heights for one bridge. A better
place for the chain rule is `duckosm elevation` (deck heights for each edge in the routing database), so that every
reader has one set of heights. That move is a design of its own.

## Checks: errors and reviews

A check that fails on data that is only weak does not stop the build: it writes `ev_review = needed` and a count. On
Monaco, these would stop the build if they were errors: span clearance (SimReady itself leaves 8 bridge spans and 23
tunnel spans failed), chains of spans with no landing (1 bridge chain, 2 tunnel chains), and pedestrian lights with no
stop line. An error is kept for broken data: ids, chains, sections, lane numbers, lanes that do not meet their paths.

## How the exporters change

| Exporter | Change |
|---|---|
| `to_lanelet2` | reads `lane`, `movement`, `stop_line`, `crossing`, `signal`. The rules of SimReady's Lanelet2 adapter: shared nodes within 1 cm, no lanelet for a border shorter than 5 cm, regulatory elements |
| `to_opendrive` | with the HD map database: roads from `carriageway`, `laneOffset`, sections and widths from `lane_section` and `lane`, junctions from `movement`. With a routing database: as today, roads only |
| `to_commonroad` | reads `lane`, `movement`, `stop_line`, `signal`. The ids are `lane_id` and `movement_id` |
| `lane_graph`, `route_lanes` | turns from `movement`; lane changes between lanes of one section with `lane_num` ± 1 and a `dashed` mark between them |
| `surfaces` | the lane areas, `movement.area`, `junction.area` |
| SUMO, glTF | later |

The Rust exporters (`rust/duckosm-export/src/lanelet2.rs`, `opendrive.rs`) can read the same tables later.

## The first version

| In | Out, for later |
|---|---|
| the full DDL of the reference page | islands, signal plans and phases (no data) |
| all tables that an exporter reads, filled from the GMNS and routing databases | joins across a lane change or a traffic light (lane sections) |
| Lanelet2, CommonRoad, then OpenDRIVE read the database | close nodes of one junction joined (`junction.kind = cluster`) |
| the bridge and tunnel rule | SUMO and glTF from the database |

The DDL is one file, `src/duckosm/sql/hdmap_schema.sql`, written from the reference page. A test compares its tables
and columns with SimReady's `schema.sql` when that file is on the machine (it is private), and skips otherwise.

## Rejected alternatives

| Alternative | Why not |
|---|---|
| the first design (`lanelet`, `boundary`, `lanelet_relation`, `marking`, `regulatory`) | no reference line, no lane sections and no widths along the road, so no good OpenDRIVE; no junction area; no evidence |
| a schema `hd_<mode>` in the GMNS database | the GMNS database gets heavy. Kaveh's decision |
| one carriageway for each edge | hundreds of fake junctions and paths shorter than 1 cm at the nodes where a road only continues |
| both the merged and the not merged roads | each road twice, on top of each other |
| all modes | the ids collide; only driving has lane paths; an HD map is for driving |
| the whole SimReady schema | the build tables are not a map |
| SimReady as the only producer | SimReady is a private test project. Public duckOSM users need an HD map from OSM alone |
| a model of our own, different from SimReady | two models of one road, and files that do not agree |
| a DDL made by a script from SimReady's `schema.sql` | duckOSM is public and SimReady is private: a public build cannot read a private file. The test above finds a difference |

## Tests (on `config/sample_monaco.yaml`)

| Test | Passes when |
|---|---|
| the checks of the reference page | each error check passes on Monaco; the review counts are in the report |
| stable ids | two builds give the same ids; each carriageway id is a `chain_id`, and a SUMO `--merge` edge has the same id |
| chains | each driving edge is in exactly one carriageway |
| ids from text | an id made in Python equals the id of the same text in DuckDB |
| Lanelet2 | the `lanelet2` library loads the file, and its routing graph finds a next lanelet for each lanelet that has one in the database (when `lanelet2` is installed; skip otherwise) |
| OpenDRIVE, CommonRoad | the files pass their current tests |

The local Monaco databases are older than `driving.chains`: the tests build Monaco first.

## Steps after sign-off

1. `src/duckosm/sql/hdmap_schema.sql` and `src/duckosm/hdmap_build.py`: `build(gmns_db, source, out=None)`, CLI
   `duckosm hdmap`.
2. Tests: `tests/test_hdmap_build.py`.
3. The exporters read the database: Lanelet2 and CommonRoad first, then OpenDRIVE, then the lane graph and
   `surfaces`.
4. Docs: remove "planned" from the pages; `docs/exports/lanelet2.md`, `opendrive.md`, `lane-routing.md`,
   `docs/reference/sdk/hdmap.md`, `docs/compare/for-hdmap-users.md` describe what works.

## Questions for Kaveh

1. Can the HD map tables be public in duckOSM? The SimReady repository is private, and this design copies its model
   into duckOSM's public docs and code.
2. Schema name `hdmap` (not `sim`): OK?
