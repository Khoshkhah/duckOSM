# HD map tables in DuckDB (`hd_<mode>`)

Status: design, waits for sign-off (2026-11-23). No code yet.

## The problem

Level 4 (HD map) has four outputs today. Each output reads the GMNS database and makes its own lane geometry:

| Output | Reads | Makes by itself |
|---|---|---|
| `to_lanelet2` (`lanelet2.py`) | `gmns_<mode>.lane.geom`, `link.free_speed` | left and right borders (`_offset_wkt` of `geom`, to the node), node snap on a 1 cm grid |
| `to_commonroad` (`hdmap.py`) | `lane.geom_cut`, `lane_connector` | borders (shapely `offset_curve` of `geom_cut`, in local metres), successors, left and right neighbours, integer ids `1..n` |
| `to_opendrive --junctions` (`opendrive.py`) | `link`, `movement` | one connecting road for each movement, with 1 lane, each lane 3.25 m |
| `lane_graph` (`lane_routing.py`) | `lane`, `movement` | lane changes from `lane_num`, turns from the lane ranges of `movement` |

The result:

- The three files do not agree. Lanelet2 borders end at the node, CommonRoad borders end at the edge of the junction,
  and OpenDRIVE ignores the lane width.
- Lanelet2 borders of two lanes side by side are two lines, not one shared line. The `lanelet2` router finds a next
  lanelet for only 732 of 3318 lanelets on Monaco, because a lane and its connector do not share end points.
- No output has stop lines, traffic lights, signs or road marks, but the GMNS database has the data for them
  (`node.ctrl_type`, `movement.ctrl_type`, `location`, `crossing`).
- CommonRoad ids are row numbers. They change when one lane is added.

## The proposal

One step, `duckosm hdmap <gmns_db>`, writes the schema `hd_<mode>` into the GMNS database. The schema holds the lane
map once: lanelets, shared borders, the relations between lanelets, junctions, markings and traffic rules. Each
exporter reads these tables and only writes its file format. The geometry, the ids and the rules then agree in each
file.

```
<area>.duckdb (Level 2) --duckosm gmns--> <area>_gmns.duckdb (Level 3)
                                              gmns_<mode>.*   lanes, connectors, movements, crossings
                                              hd_<mode>.*     --duckosm hdmap-->  (Level 4, this design)
                                                  |--> to_lanelet2   (.osm)
                                                  |--> to_commonroad (.xml)
                                                  |--> to_opendrive  (.xodr)
                                                  '--> lane_graph, route_lanes, surfaces
```

Only `driving` gets the schema by default (`--mode` for others), because the lane connectors come from SUMO's
netconvert for driving only.

## What the formats need

The data models of the formats are close. Each format has a lane piece with a left and a right border, links to the
next and to the side pieces, and rules that point to the lane pieces.

| Item | Lanelet2 | OpenDRIVE 1.7 / 1.8 | CommonRoad 2020a | This schema |
|---|---|---|---|---|
| lane piece | `lanelet` relation (left and right way) | `lane` in a `laneSection` of a `road`, by width from the reference line | `lanelet` (left, center, right vertices) | `lanelet` |
| border | `linestring` way, `type`, `subtype` (`line_thin`/`dashed`, `curbstone`, `virtual`) | `roadMark` of a lane | `lineMarking` of a border | `boundary` |
| shared border | the same way in two lanelets | implicit: lanes side by side | `adjacentLeft`/`adjacentRight`, same vertices | `boundary` row used by two lanelets |
| next piece | shared end points (the router finds them) | `link` `predecessor`/`successor`, junction `laneLink` | `predecessor`, `successor` | `lanelet_relation` |
| junction | none (only lanelets) | `junction` with `connection` and `laneLink` | `intersection` with `incoming` | `junction` |
| stop line | `stop_line` way, member `ref_line` | `signal` or object | `stopLine` of a lanelet | `marking` kind `stop_line` |
| crosswalk | lanelet `subtype=crosswalk` | object `crosswalk` | area (2022 and later) | `marking` kind `crosswalk` |
| traffic light, sign | `regulatory_element` (`traffic_light`, `traffic_sign`, `right_of_way`) with `refers`, `ref_line`, `yield` | `signal`, `signalReference` | `trafficLight`, `trafficSign` on lanelets | `regulatory`, `regulatory_member` |
| speed limit | lanelet tag `speed_limit` | `type` / `speed` of a road | traffic sign `274` | `lanelet.speed_limit` |
| height | `ele` tag on each node | `elevationProfile` of a road | none (2D) | `z_start`, `z_end` |
| id | integer (OSM id) | string road id, integer lane id | positive integer, unique in the file | `lanelet_id` (text) and `num_id` (BIGINT) |
| coordinates | latitude, longitude | metres, `geoReference` | metres, `location` | EPSG:4326, and `meta.utm_crs` |

Formats that I looked at and do not copy:

| Format | Why not the base |
|---|---|
| NDS.Live | its lane model (lane groups, borders) is the same idea, but the standard is not open. No open tool reads it |
| Overture transportation | segments and connectors only, with lane rules as properties. It has no lane geometry |
| ASAM OSI | an interface between simulators while they run, not a store of a map |

## Rules of the schema

| Rule | |
|---|---|
| Place | schema `hd_<mode>` in the GMNS database, the same file as `gmns_<mode>` and `lane_<mode>` |
| Ids | text keys from the GMNS keys: `lane_id` (`<edge_id>_<lane_num>`) and `connector_id` (`<from_lane_id>><to_lane_id>`). No sequence |
| Integer ids | `num_id = (hash(<text key>) >> 1)::BIGINT`, the formula of `edge_id` (`edge_id.py`). It holds while the `duckdb<2` pin holds. A test makes sure that each `num_id` is unique |
| Geometry | DuckDB `GEOMETRY`, EPSG:4326 (longitude, latitude), as in the GMNS tables. The exporters project to `meta.utm_crs` |
| Lane numbers | lane 1 is the leftmost lane in the direction of travel, as in GMNS |
| Logic | in SQL, as in `gmns.py`. One exception: DuckDB `spatial` has no offset curve, so the borders come from one vectorized shapely `offset_curve` call on all lanes (the method of `to_commonroad`), in `meta.utm_crs` |

## The tables

### meta

One row.

| Column | Type | Holds |
|---|---|---|
| `mode` | VARCHAR | `driving` |
| `utm_crs` | VARCHAR | the metric CRS of the area, for example `EPSG:32632`. Each exporter uses it, so each file has the same projection |
| `ground_id` | VARCHAR | the `sha256` of the ground in `main.elevation_metadata` of the Level 2 database. Empty when the area has no ground |
| `vertical_datum`, `ground_product` | VARCHAR | from the same row, for example `EGM2008`, `DTM` |
| `z_source` | VARCHAR | `edges` (the heights at the two nodes of each edge), or empty |
| `default_width` | DOUBLE | 3.25: the width of a lane that has no width |
| `duckosm_version`, `built_at` | VARCHAR | |

### lanelet

One row for each GMNS lane and each lane connector.

| Column | Type | Key | From | Holds |
|---|---|---|---|---|
| `lanelet_id` | VARCHAR | key | `lane.lane_id`, `lane_connector.connector_id` | |
| `num_id` | BIGINT | unique | new | the hash of `lanelet_id` |
| `kind` | VARCHAR | | new | `lane` or `connector` |
| `edge_id` | BIGINT | | `lane.link_id`; the link of `from_lane_id` | the duckOSM `edge_id` |
| `to_edge_id` | BIGINT | | the link of `to_lane_id` | connectors only |
| `lane_num` | INTEGER | | `lane.lane_num` | lanes only |
| `mvmt_id` | VARCHAR | | `lane_connector.mvmt_id` | connectors only |
| `junction_id` | BIGINT | | `movement.node_id` | connectors only |
| `turn` | VARCHAR | | `lane.turn`; `movement.type` | `left`, `thru`, `right`, … |
| `subtype` | VARCHAR | | `lane.allowed_uses` | `road`, `bus_lane`, `bicycle_lane`, `walkway` (the Lanelet2 words) |
| `allowed_uses` | VARCHAR | | `lane.allowed_uses` | |
| `width` | DOUBLE | | `lane.width`, `lane_connector.width` | metres, `meta.default_width` when empty |
| `speed_limit` | DOUBLE | | `link.free_speed` | km/h |
| `left_bound`, `right_bound` | VARCHAR | → `boundary` | new | |
| `length` | DOUBLE | | new | metres, of `geom` |
| `z_start`, `z_end` | DOUBLE | | new | metres on the ground datum. Empty on bridges and in tunnels |
| `geom` | GEOMETRY | | `lane.geom_cut` (else `geom`); `lane_connector.geom` | the center line |

### boundary

One row for each border line. Two lanes side by side share one row.

| Column | Type | Key | Holds |
|---|---|---|---|
| `bound_id` | VARCHAR | key | `<edge_id>_b<k>`: `b0` is the left border of the leftmost lane, `bk` the border between lane `k` and the lane on its right. A connector: `<connector_id>_l`, `<connector_id>_r` |
| `num_id` | BIGINT | unique | the hash of `bound_id` |
| `edge_id` | BIGINT | | the link, or the link in of a connector |
| `line_type` | VARCHAR | | `line_thin`, `curbstone`, `virtual` (the Lanelet2 words) |
| `line_subtype` | VARCHAR | | `solid` or `dashed` |
| `color` | VARCHAR | | `white` |
| `lane_change` | BOOLEAN | | true when a vehicle can cross the line (`dashed`) |
| `z_start`, `z_end` | DOUBLE | | as in `lanelet` |
| `geom` | GEOMETRY | | the line, in the direction of travel |

How the borders are made:

- The border between lane `k` and lane `k + 1` is the center line of lane `k`, offset to the right by half its width.
  Lane `k + 1` uses the same row as its left border. Where the two lanes do not have the same length (a turn pocket),
  the shared row is cut to the shorter lane.
- A connector border starts at the end point of the border of its lane in, and ends at the start point of the border
  of its lane out. Its first and last points are set to these points. Then the Lanelet2 router finds each next
  lanelet, and CommonRoad and OpenDRIVE have no gap.
- Line types, the first rule that applies:

| Border | `line_type` | `line_subtype` |
|---|---|---|
| a connector border | `virtual` | `solid` |
| the right border of the rightmost lane | `curbstone` | `solid` |
| the left border of lane 1 of a two-way road (the center line) | `line_thin` | `solid` |
| between two lanes with different `allowed_uses` | `line_thin` | `solid` |
| between two lanes with the same `allowed_uses` | `line_thin` | `dashed` |

OSM has few tags for road marks (`lane_markings`, `change:lanes`). The first version uses only the table above. A read
of `change:lanes` can come later.

### lanelet_relation

| Column | Type | Key | Holds |
|---|---|---|---|
| `from_id` | VARCHAR | key with the next two | a `lanelet_id` |
| `to_id` | VARCHAR | key | a `lanelet_id` |
| `relation` | VARCHAR | key | `successor`, `left`, `right` |
| `lane_change` | BOOLEAN | | `left` and `right`: the `lane_change` of the shared border |

`successor` rows: lane → connector → lane, from `lane_connector`. `left` and `right` rows: the lanes of one link that
share a border. This table replaces the lane changes and the turns that `lane_graph` and `to_commonroad` make today.
`lane_graph` then reads it and only adds the costs.

### junction

| Column | Type | Key | From | Holds |
|---|---|---|---|---|
| `junction_id` | BIGINT | key | `node.node_id` | a node with one or more connectors |
| `num_id` | BIGINT | | new | the hash of `junction_id` as text |
| `ctrl_type` | VARCHAR | | `node.ctrl_type` | `signal`, `stop`, `yield`, … |
| `z` | DOUBLE | | `node.z_coord` | |
| `geom` | GEOMETRY | | `node.geom` | the point. The surface of the junction is `surfaces()`: it is not stored |

### marking

Road marks that are not lane borders.

| Column | Type | Key | From | Holds |
|---|---|---|---|---|
| `marking_id` | VARCHAR | key | new | `<edge_id>_stop`, or the `crossing_id` |
| `num_id` | BIGINT | | new | the hash of `marking_id` |
| `kind` | VARCHAR | | | `stop_line` or `crosswalk` |
| `edge_id` | BIGINT | | | the link |
| `painted` | BOOLEAN | | `crossing.painted` | true for a stop line |
| `geom` | GEOMETRY | | | a stop line: a line across all lanes of the link, at the end of `geom_cut`. A crosswalk: `crossing.geom` |

A link gets a stop line when a movement out of it has `ctrl_type` `signal` or `stop`.

### regulatory and regulatory_member

A traffic rule, and the lanelets and lines that it applies to. The model is the Lanelet2 regulatory element, because
the other two formats are a subset of it.

`regulatory`:

| Column | Type | Key | Holds |
|---|---|---|---|
| `reg_id` | VARCHAR | key | `<kind>_<edge_id>`, for example `traffic_light_123` |
| `num_id` | BIGINT | | the hash of `reg_id` |
| `kind` | VARCHAR | | `traffic_light`, `stop`, `yield`, `speed_limit` |
| `junction_id` | BIGINT | | |
| `osm_id` | BIGINT | | the OSM node of the light or the sign, from `location`, when it is known |
| `geom` | GEOMETRY | | the point of the light or the sign (`location.geom`), else the end of the link |

`regulatory_member`:

| Column | Type | Key | Holds |
|---|---|---|---|
| `reg_id` | VARCHAR | key with the next two | |
| `member_id` | VARCHAR | key | a `lanelet_id` or a `marking_id` |
| `role` | VARCHAR | key | `refers` (the lanelet that must obey), `ref_line` (the stop line), `yield`, `right_of_way` |

Source: the `ctrl_type` of each movement. A movement with `signal` gives a `traffic_light` row for its link in. With
`stop` or `yield` it gives a `stop` or a `yield` row. The connectors of the other movements at the same junction are
`right_of_way` members of a `yield` or `stop` row. The current GMNS data has no signal phases, so a traffic light has no
phase data.

## Height

The heights come from the area's one ground (`concepts/ground-reference.md`), never from another source.

| Item | Height |
|---|---|
| `lanelet`, `boundary` | `z_start` and `z_end`: the Level 2 `edges.z_from` and `z_to` of the edge, at the two ends of the line. A connector takes the end of its lane in and the start of its lane out |
| bridge and tunnel links | empty: the ground is not the road there. SimReady makes the road heights; the step can read them later |
| `meta` | `ground_id`, `vertical_datum`, `ground_product`. Each exporter writes the `ground_id` into its file (OpenDRIVE `<userData code="ground">`, Lanelet2 a tag on the map) |

The GMNS database does not hold `z_from` and `z_to` today, only `node.z_coord`. The step uses `node.z_coord` at the two
ends of each link. It reads `main.elevation_metadata` from the Level 2 database (`ATTACH`, read only) when the user
gives its path with `--source`, else `ground_id` is empty.

I keep the geometry 2D. DuckDB 1.5.3 can store `LINESTRING Z` and writes it to GeoParquet (tested), but duckOSM has
heights only at the nodes today. A `Z` on each point adds no data until SimReady gives the road heights. Then a new
column `geom_z` can come without a change to the other columns.

## How the exporters change

| Exporter | Reads now | Writes |
|---|---|---|
| `to_lanelet2` | `boundary` → ways (the points of shared rows become shared nodes, no snap grid), `lanelet` → relations, `regulatory` → regulatory elements, `marking` → `stop_line` ways and crosswalk lanelets | ids: `num_id` |
| `to_commonroad` | `lanelet` with its two `boundary` rows, `lanelet_relation`, `junction` → `intersection`, `regulatory` → traffic lights and signs, `marking` → stop lines | ids: `num_id`. It keeps its resample of the two borders to the same number of points: that is a rule of the format |
| `to_opendrive --junctions` | `lanelet` (lanes) → roads, with the real widths and one lane for each lane; connectors → connecting roads with a `laneLink` for each connector; `boundary.line_subtype` → `roadMark`; `regulatory` → `signal`; `z_start`/`z_end` → `elevationProfile` | road ids: `edge_id`, connecting road ids: `num_id` |
| `lane_graph`, `route_lanes` | `lanelet_relation` | |
| `surfaces` | `lanelet` (width, `geom`) | |

Each exporter runs `duckosm hdmap` first when the GMNS database has no `hd_<mode>` schema, as `route_lanes` does with
the lane graph today. OpenDRIVE without `--junctions` keeps its read of the Level 2 database.

The Rust exporters (`rust/duckosm-export/src/lanelet2.rs`, `opendrive.rs`) can read the same tables later. This design
does not change them.

Parquet: the step adds no flag. `COPY hd_driving.lanelet TO 'lanelet.parquet'` writes GeoParquet 1.0 metadata when
`spatial` is loaded (tested on DuckDB 1.5.3).

## Rejected alternatives

| Alternative | Why not |
|---|---|
| One `hdmap` schema for all modes | the lanes and the movements differ by mode, and GMNS is one schema for each mode |
| The tables in the Level 2 database | the lanes need the GMNS step (netconvert). Level 4 reads Level 3, as today |
| Store the OpenDRIVE model (reference line, lane widths as polynomials in `s`) | `ST_*` cannot query it, and Lanelet2 and CommonRoad need the borders as lines. Widths from borders is easy; borders from polynomials is the hard way |
| Store the Lanelet2 model as OSM primitives (point, way, relation tables) | the shared nodes are a rule of the file format. A `GEOMETRY` line is the border; the exporter makes the nodes |
| Ids from sequences or row numbers | they change when one lane changes. Downstream data joins on the ids |
| `LINESTRING Z` on each point now | no height data between the nodes yet (see Height) |
| Store the lane and junction polygons | `surfaces()` makes them from `lanelet` in one query. A stored copy can disagree with the lines |
| A new step in `duckosm gmns` | the GMNS database is a standard of its own. The HD map is an extra layer, as the lane graph is |

## Tests (on `config/sample_monaco.yaml`)

| Test | Passes when |
|---|---|
| ids | each `num_id` is unique in its table; two builds give the same `lanelet_id`, `bound_id` and `num_id` |
| shared borders | the right border of lane `k` is the left border of lane `k + 1` (same `bound_id`) |
| connectors meet | the end point of each lane border equals the start point of its connector border |
| relations | each `successor` row joins two lanelets that touch; each lane connector gives 2 `successor` rows |
| rules | each movement with `ctrl_type = 'signal'` gives a `traffic_light` row with a `ref_line` |
| exporters | Lanelet2, CommonRoad and OpenDRIVE files from the tables pass their current tests. The `lanelet2` router finds a next lanelet for more lanelets than today (732 of 3318) |

## Steps after sign-off

1. `src/duckosm/hdmap_build.py`: `build_hdmap(gmns_db, mode="driving", source=None)`, CLI `duckosm hdmap`.
2. Tests: `tests/test_hdmap_tables.py`.
3. Exporters read the tables: Lanelet2 and CommonRoad first, then OpenDRIVE, then the lane graph. One PR each, or one
   PR if they stay small.
4. Docs: `docs/reference/database.md` (the `hd_<mode>` tables), `docs/exports/lanelet2.md`, `opendrive.md`,
   `lane-routing.md`, `docs/reference/sdk/hdmap.md`, `docs/compare/for-hdmap-users.md`.

## Questions for Kaveh

1. Schema name `hd_<mode>`, in the GMNS database: OK?
2. Stop lines, traffic lights and yield rules in the first PR, or lanelets, borders and relations first?
3. Line types from the table above (few OSM tags): OK as a default, or leave all borders `line_thin` `dashed` until the
   data is better?
