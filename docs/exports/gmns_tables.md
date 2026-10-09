# GMNS tables, one by one

A reference for every table in the GMNS database that `duckosm gmns` writes: what a row is, how it is
made from OSM, and what each column holds. For the command and the options, see [GMNS](gmns.md). For
the standard itself, see the [GMNS specification](https://zephyr-data-specs.github.io/GMNS/spec/)
(release 0.97). Numbers are from Monaco, driving.

## How to read this page

**Where the tables are.** One schema per mode: `gmns_driving`, `gmns_walking`, `gmns_cycling`. With
`--combined` there is also `gmns_all`; with `--meso` / `--micro`, `meso_<mode>` and `micro_<mode>`.

```sql
SELECT count(*) FROM gmns_driving.link;      -- ATTACH the file, or open it directly
```

**Column marks.** The *Spec* column says where a column comes from (`--csv-extensions` keeps the ➕ columns in the CSVs too, as the spec's user-defined fields, with a `u_` prefix: `u_osm_id`, `u_bridge`, `u_turn`, …; the native `geom` columns never go to a CSV):

| Mark | Meaning | In the CSVs (`--to-csv`)? |
|---|---|---|
| ✅ | a column of the GMNS standard, filled from OSM | yes |
| ∅ | a column of the standard that OSM has no data for, so it stays empty | yes, empty |
| ➕ | a duckOSM extension: not in the standard | no (the CSV has only standard columns) |

**Conventions that hold in every table:**

- **Ids are integers.** `link_id` is duckOSM's `edge_id`, a content hash (`BIGINT`, up to 19 digits). It is the
  same on every rebuild and the same in every mode, so you can join GMNS to your own data on it.
  Keep it `BIGINT`: as a float or a JavaScript number it loses digits. `node_id` is the OSM node id.
- **Units:** lengths and widths in metres, speeds in km/h, coordinates in longitude / latitude
  (EPSG:4326). The `config` table says so.
- **Geometry twice.** Where a table has a shape, the standard column `geometry` holds it as WKT text, and the
  extension column `geom` holds the same shape as a native `GEOMETRY` you can query with the DuckDB `spatial`
  extension (`ST_*`) and draw. Only `geometry` goes to the CSVs.
- **One link per direction.** A two-way road is two links with swapped nodes and reversed geometry; a
  one-way road is one. Every link has `directed = true`.
- **Lane numbers** run from the left in the direction of travel: lane 1 is the leftmost lane, whichever side
  traffic drives. (With right-hand traffic, the default, the leftmost lane is next to the road's centre line;
  with `--drive-side left` the lanes lie left of it and the leftmost is the farthest.) OSM's `turn:lanes`
  and `bicycle:lanes` list lanes in the same order, so entry *n* is lane *n*.

## The tables at a glance

| Table | Standard? | One row is | Rows | Read it for |
|---|---|---|---|---|
| [`config`](#config) | yes | the dataset | 1 | units, CRS, spec version |
| [`use_definition`](#use_definition) | yes | a way of using a lane (auto, bus, …) | 3 | what `allowed_uses` values mean |
| [`use_group`](#use_group) | yes | a named group of uses | 1 | the mode's uses |
| [`node`](#node) | **required** | a junction or end of a link | 1,904 | where links meet |
| [`link`](#link) | **required** | one direction of a road | 3,092 | the routable network |
| [`geometry`](#geometry) | yes | the shape of a link | 3,092 | link shapes, keyed by id |
| [`lane`](#lane) | yes | one lane of a link | 3,520 | lane use, width, turns |
| [`movement`](#movement) | yes | one legal turn at a junction | 3,957 | which link may follow which, from which lanes |
| [`lane_connector`](#lane_connector) | ➕ extension | one lane-to-lane path across a junction (driving), and a footway's [join](#footway-joins) to a road lane (walking) | 1,813 + 542 | drawing lanes through junctions |
| [`lane_check`](#lane_check) | ➕ extension | something the lanes could not be given: a lane with no way in or out, a turn with no lane connection | — | what to check (in Street View) |
| [`crossing`](#crossing) | ➕ extension | one pedestrian crossing painted on the driving lanes | 741 | drawing zebra crossings |
| [`lane_crossing`](#lane_crossing) | ➕ extension | one lane a crossing covers, and which stretch of it | 909 | painting the stripes lane by lane |
| [`link_along`](#link_along) | ➕ extension | one road a footpath runs along (a footpath often runs along several) | — | matching a sidewalk to its whole street |
| [`curb_seg`](#curb_seg) | yes | a stretch of kerb with a rule | 2 | on-street parking |
| [`signal_controller`](#signal_controller) | yes | a traffic signal | 1 | where the signals are |
| [`location`](#location) | yes | a point along a link | 1,803 | crossings, signs, parking entrances |
| [`zone`](#zone) | yes | an area | 1 | the area's name and, when known, outline |
| [`gmns_all`](#gmns_all-all-modes-in-one) | | all modes in one `node` + `link` | | one network for every mode |
| [`meso_*`, `micro_*`](#meso-and-micro-networks) | osm2gmns layout | sections, connectors, cells | | simulators |

How they point at each other:

```mermaid
erDiagram
    node ||--o{ link : "from_node_id / to_node_id"
    link ||--|| geometry : "geometry_id"
    link ||--o{ lane : "link_id"
    link ||--o{ curb_seg : "link_id"
    link ||--o{ location : "link_id"
    node ||--o{ movement : "node_id"
    link ||--o{ movement : "ib_link_id / ob_link_id"
    movement ||--o{ lane_connector : "mvmt_id"
    lane ||--o{ lane_connector : "from_lane_id / to_lane_id"
    node ||--o| signal_controller : "node_id"
    use_definition ||--o{ lane : "allowed_uses"
```

## config

One row that says how to read the other tables. It is what a tool needs to know first.

| Column | Type | Spec | Value |
|---|---|---|---|
| `dataset_name` | VARCHAR | ✅ | the output file's name plus the mode, e.g. `monaco_gmns_driving` |
| `short_length` | VARCHAR | ✅ | `meter`: lane widths, linear references |
| `long_length` | VARCHAR | ✅ | `meter`: link lengths |
| `speed` | VARCHAR | ✅ | `kmh` |
| `crs` | VARCHAR | ✅ | `EPSG:4326` |
| `geometry_field_format` | VARCHAR | ✅ | `wkt` |
| `currency` | VARCHAR | ∅ | empty (OSM has no toll prices) |
| `version_number` | DOUBLE | ✅ | `0.97`, the GMNS release the output follows |
| `id_type` | VARCHAR | ✅ | `integer` |

## use_definition

What each use in `allowed_uses` means, so a tool can tell a bus lane from a car lane.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `use` | VARCHAR | ✅ | the name: `auto`, `bus`, `bike` (driving); `walk` (walking); `bike` (cycling) |
| `persons_per_vehicle` | DECIMAL | ✅ | `auto` 1, `bus` 25, `bike` 1, `walk` 1 |
| `pce` | DECIMAL | ✅ | passenger-car equivalents: `auto` 1.0, `bus` 2.0, `bike` 0.2, `walk` 0 |
| `special_conditions`, `description` | VARCHAR | ∅ | empty |

Every use that appears in `link.allowed_uses`, `lane.allowed_uses` or `movement.allowed_uses` is defined here,
as the standard requires.

## use_group

A name for the uses of a mode, to keep `allowed_uses` lists short.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `use_group` | VARCHAR | ✅ | the mode: `driving`, `walking`, `cycling` |
| `uses` | VARCHAR | ✅ | its use: `auto`, `walk`, `bike` |
| `description` | VARCHAR | ∅ | empty |

## node

A point where links start or end: a junction, a dead end, or a point where a road was cut. This is one of
the two required tables, with `link`.

A node is made wherever two OSM ways meet or a way ends (as in the source network's `nodes`; a bus-only road's own nodes,
which `nodes` lacks, are its links' end points, with no height). A road that
loops back on itself is cut in two at its midpoint; that point is a node with a **negative** id, because
it is not an OSM node.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `node_id` | BIGINT | ✅ key | the OSM node id; negative for the cut points above |
| `x_coord`, `y_coord` | DOUBLE | ✅ | longitude, latitude |
| `ctrl_type` | VARCHAR | ✅ | `signal` where the OSM node is a `highway=traffic_signals`; else from the stop and give-way signs on its approaches: `stop_4_way` (every approach stops, four or more), `stop` (every approach, fewer), `stop_2_way` (some approaches), `yield`; empty where there is none. See [`movement`](#movement) for which signs count |
| `z_coord` | DOUBLE | ✅ | metres: the node's height, when the source was given heights with [`duckosm elevation`](../guides/elevation.md); empty otherwise |
| `zone_id` | BIGINT | ✅ | `1`: every node is in the [`zone`](#zone). (Tools such as Path4GMNS refuse nodes with no zone.) |
| `node_type` | VARCHAR | ✅ | OSM names, as the spec's FAQ recommends: the OSM `highway` value for a `turning_circle`, `mini_roundabout` or `motorway_junction`; else `intersection` (three or more neighbouring nodes), `dead_end` (one); empty where a road is merely cut |
| `name`, `parent_node_id` | | ∅ | empty |
| `geom` | GEOMETRY | ➕ | the point |

## link

The routable network: **one row per direction of a road**. A footpath is one link too (two, if it can be
walked both ways). In driving, a direction only buses may use is a link too, of one lane (`allowed_uses` `bus`): a bus lane
against a one-way street and a bus-only road, the source's `driving.private_edges` with `access = 'bus'`
([design](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/bus_only_edges.md)). Their turns are the source
`edge_graph`'s rows with `uses = 'bus'`, so [movements](#movement) lead into and out of them, for buses only. Monaco: 18 (Boulevard Charles III's bus lane back,
`1449981121#1r`, is one).

| Column | Type | Spec | Holds |
|---|---|---|---|
| `link_id` | BIGINT | ✅ key | the `edge_id`: a hash of the OSM way and the two nodes; stable, same in every mode |
| `name` | VARCHAR | ✅ | the street name (`name` tag), often empty |
| `from_node_id`, `to_node_id` | BIGINT | ✅ | the two [`node`](#node)s, in the direction of travel |
| `directed` | BOOLEAN | ✅ | always `true` |
| `geometry_id` | BIGINT | ✅ | the same as `link_id`: the row of [`geometry`](#geometry) |
| `geometry` | VARCHAR | ✅ | the shape, WKT `LINESTRING`, from `from_node` to `to_node` |
| `dir_flag` | INTEGER | ✅ | always `1`: the shape runs from `from_node` to `to_node` |
| `length` | DOUBLE | ✅ | metres: the length of the shape on the ellipsoid, so it always agrees with `geometry` |
| `facility_type` | VARCHAR | ✅ | the OSM `highway` value as it is: `residential`, `primary_link`, `footway`, … |
| `capacity` | DOUBLE | ✅ | saturation capacity, passenger cars per hour **per lane**: a default per road class (motorway 2000, trunk 1800, primary 1600, secondary 1400, tertiary 1200, unclassified 1000, residential 800, living_street 300, service 300, anything else 800). Empty on links that cars cannot use, see below |
| `free_speed` | FLOAT | ✅ | km/h: the `maxspeed` tag, else a default per road class; walking 5, cycling 15 |
| `lanes` | INTEGER | ✅ | the number of permanent **motor-vehicle** lanes in this direction, from the `lanes` / `lanes:forward` / `lanes:backward` tags, **1 where untagged**; 1 on a bus-only link (a one-way street's `lanes` are its own direction's). Not the number of rows in `lane`: the standard excludes turn pockets, bike lanes, shoulders and parking lanes. Empty on links that cars cannot use |
| `bike_facility` | VARCHAR | ✅ | from OSM `cycleway`, in the standard's words: `unseparated bike lane`, `separated bike lane`, `shared lane`, `counter-flow bike lane`, `paved shoulder`, `none`, `other` |
| `ped_facility` | VARCHAR | ✅ | from OSM `sidewalk`: `sidewalk` (either side or both: which side is lost), `offstreet_path` (`separate`), `none`, `unknown` |
| `allowed_uses` | VARCHAR | ✅ | the mode's use: `auto`, `walk` or `bike`; on a bus-only link `bus`, or `bus,bike` where bikes may use it too (the way has `oneway:bicycle=no`, or `cycleway` / `cycleway:left` / `cycleway:right` = `share_busway`, or the edge is in cycling) |
| `grade` | DOUBLE | ✅ | percent, negative downhill: `100 × (height at the end − height at the start) / length`, when the source has heights (`duckosm elevation`). Empty on a bridge or in a tunnel (there the height is the ground below or above, not the road) and where the slope would pass 100 %, the spec's limit. A coarse elevation model makes short links noisy: see the [elevation guide](../guides/elevation.md#what-the-heights-mean) |
| `parent_link_id` | BIGINT | ✅ | for a sidewalk (an OSM way with `footway=sidewalk`) in the walking and cycling schemas, the road link it runs along: the nearest, roughly parallel one within 30 m, and of a two-way road the link the sidewalk is on the kerb side of (its right with right-hand traffic). Empty on roads and on a sidewalk with no road beside it |
| `parking`, `toll`, `jurisdiction`, `row_width` | | ∅ | empty (parking is in [`curb_seg`](#curb_seg)) |
| `osm_id` | BIGINT | ➕ | the OSM way the link comes from: one way gives several links (one per piece and direction). A way duckOSM had to split carries a *virtual* negative id in its edges (`edge_ref` `-1525349445#1f`); this column is always the real way (its absolute value), and the way's tags (`highway`, `footway`, `crossing`, lane tags) are read from it |
| `edge_ref` | VARCHAR | ➕ | duckOSM's readable id of the link, `<osm_id>#<n><f or r>` (`503633032#2f`): `n` counts a way's pieces along it, `f` / `r` is the direction. The two directions of one two-way piece share `<osm_id>#<n>` (`#2f` and `#2r`), so two links with the same way and number are twins; links of different ways (`503633032#2f`, `503633034#3f`) never are. Not a key (`link_id` is) |
| `footway` | VARCHAR | ➕ | the OSM `footway` tag (`sidewalk`, `crossing`, `link`, …), empty without raw tags: a `crossing` lies on the road it crosses (a reader draws it over the road) |
| `crossing`, `crossing_markings` | VARCHAR | ➕ | on a `footway=crossing` link, the OSM `crossing` (`marked`, `zebra`, `uncontrolled`, `unmarked`, `traffic_signals`, …) and `crossing:markings` (`yes`, `zebra`, `no`, …) tags; empty on every other link |
| `along_link_id`, `along_mode`, `along_gap_m`, `along_kind` | BIGINT, VARCHAR, DOUBLE, VARCHAR | ➕ | on a footpath link, the road it runs along: the nearest roughly parallel road of the footpath's own level (within 30 m, on its kerb side), even where that road is only in `gmns_driving` (`link_id` is the same hash in every schema); the network it is in (`walking` / `driving`); the metres between the two lines; and `along_kind`: `sidewalk` (a `footway=sidewalk` way) or `adjacent` (any other footway, path, pedestrian or cycleway link, not a link or steps, that lies within 5 m of the road's edge: a label of closeness, nothing is moved; a `footway=crossing` link only when it runs along the road for most of its length and is either 10 m or longer (a footpath OSM tagged by mistake) or shares a node with a matched footpath that runs along the same road (a sidewalk's crossing of a side street, which also runs along the main road)). The road is one cars can use: a road only people walk on is not one to run along. Empty on every other link. `parent_link_id` is the same road for a `sidewalk` only, and only when it is a link of this table |
| `bridge`, `tunnel`, `layer` | VARCHAR | ➕ | the OSM tags as they are (`yes`, `-1`, …): which links are on a bridge or in a tunnel, and the drawing order |
| `geom` | GEOMETRY | ➕ | the shape |

**Links that cars cannot use.** In the walking and cycling schemas, a link whose OSM `highway` is
`footway`, `path`, `cycleway`, `steps`, `pedestrian`, `bridleway`, `corridor` or `platform` has `lanes` and
`capacity` empty: both are motor-vehicle quantities, and the standard treats foot and bike links as
uncapacitated. A road that is walked or cycled on keeps its motor `lanes` and `capacity`.

```sql
-- the longest bridges
SELECT name, round(length) AS metres FROM gmns_driving.link WHERE bridge = 'yes' ORDER BY length DESC LIMIT 5;
```

## geometry

The shape of every link, in its own table keyed by `geometry_id`: for tools that keep shapes apart from
links. It repeats `link.geometry`, row for row.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `geometry_id` | BIGINT | ✅ key | the `link_id` |
| `geometry` | VARCHAR | ✅ | WKT `LINESTRING` |
| `geom` | GEOMETRY | ➕ | the same, native |

## lane

One row per lane of a link. Without OSM lane tags the lanes are just numbered; with them, each lane knows
its use, width and turns.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `lane_id` | VARCHAR | ✅ key | `<link_id>_<lane_num>`, e.g. `8511077704723192952_2` |
| `link_id` | BIGINT | ✅ | the [`link`](#link) |
| `lane_num` | BIGINT | ✅ | 1 = the leftmost lane in the direction of travel, up to the number of lanes. An on-road bike lane on the right (`cycleway:right=lane`) is the last; one on the LEFT of a one-way road (`cycleway:left=lane`) is -1, left of lane 1 (the spec numbers lanes left of the left-most through lane negative), so the motor lanes keep 1..n |
| `allowed_uses` | VARCHAR | ✅ | `auto`; `bus` where `psv:lanes` / `bus:lanes` says `designated`; `bike` where `bicycle:lanes` does, or for an on-road bike lane (see below); the one lane of a bus-only link as its link's, `bus` or `bus,bike` (driving). `walk` / `bike` in the walking / cycling schemas |
| `width` | DOUBLE | ✅ | metres, from `width:lanes`; else the class default (service 2.5, residential / living street / unclassified 3.0, others 3.25); a bus-only link's lane 3.25; a bike lane 1.5; **empty on a walk lane without a width** |
| `r_barrier`, `l_barrier` | VARCHAR | ∅ | empty |
| `turn` | VARCHAR | ➕ | the lane's `turn:lanes` value (`left`, `through;right`, …), empty where untagged |
| `geom` | GEOMETRY | ➕ | the lane's centre line, offset from the link line by the widths of the lanes before it. A two-way road's lanes sit on the traffic side; a road mapped as two one-way ways is placed as one road (no overlap). It runs to the link's nodes |
| `geom_cut` | GEOMETRY | ➕ | driving: the lane's line where SUMO netconvert ends it, at the edge of the junction; the `lane_connector` paths meet it ([design](../design/gmns_lane_profile.md)). `geom` is the line to the node |

How many rows: the lane count, plus a lane for each extra entry of `turn:lanes` (a turn pocket), plus one for an
on-road bike lane. That is why a link can have `lanes = 2` and three `lane` rows.

**On-road bike lanes.** Where OSM tags `cycleway=lane` (or `cycleway:right=lane` / `:both`) on the right of
the direction of travel (for the way's reverse direction: `cycleway:left` / `:both`), the link gets one more lane
row, `allowed_uses = 'bike'`, the *last* lane, to the right of the motor lanes; its `width` is the
`cycleway:right:width` tag, 1.5 m if untagged. The GMNS FAQ says on-road bike lanes belong in the lane
table. It is not a place to turn into: movements, forks and merges use the motor lanes only, and `link.lanes`
stays the motor lane count. Not done for left-hand traffic (`--drive-side left`: the lane would be on the left
and renumber the motor lanes), for separated tracks (they are their own links in the cycling schema) or where
`bicycle:lanes` already marks a lane.

```sql
-- the lanes of one street, with their uses
SELECT l.name, k.link_id, k.lane_num, k.allowed_uses
FROM gmns_driving.link l JOIN gmns_driving.lane k USING (link_id)
WHERE l.name = 'Boulevard Charles III' ORDER BY k.link_id, k.lane_num;
```

## movement

One row per legal turn: **from this link, onto that link, at this node**. It says which links may follow
which, and from which lanes.

The turns are the legal ones: OSM turn restrictions are applied. Turning back onto the road's other
direction (a U-turn) is kept only at a dead end, or where it is the only way into that road's other
direction.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `mvmt_id` | VARCHAR | ✅ key | `<ib_link_id>-<ob_link_id>` |
| `node_id` | BIGINT | ✅ | the junction where the turn is made: the end of the inbound link and the start of the outbound |
| `ib_link_id`, `ob_link_id` | BIGINT | ✅ | the inbound and outbound [`link`](#link) |
| `type` | VARCHAR | ✅ | `left`, `right`, `thru`, `uturn` (from the change of heading at the junction), `merge`, `diverge` (the road joins or splits within 45°) |
| `mvmt_code` | VARCHAR | ✅ | direction of travel and turn, e.g. `NBL` = northbound, left (`NB`/`EB`/`SB`/`WB` + `L`/`T`/`R`). **Empty for a U-turn**: the standard's code has no U; `type` says it |
| `start_ib_lane`, `end_ib_lane` | INTEGER | ✅ | the inbound lanes the turn starts from, `lane_num` from..to |
| `start_ob_lane`, `end_ob_lane` | INTEGER | ✅ | the outbound lanes it ends in. The two ranges are the same length and match in order: the first inbound lane goes to the first outbound lane. Where the lanes of one turn do not go on side by side (one lane into two), the turn has more rows: `<mvmt_id>-2`, ...; a bike lane's own, `<mvmt_id>-b<lane>` (`allowed_uses` bike) |
| `allowed_uses` | VARCHAR | ✅ | the mode's use; into or out of a bus-only link (driving), `bus`, or `bus,bike` where every bus-only link of the turn takes bikes |
| `ctrl_type` | VARCHAR | ✅ | `signal` at a signalised junction; otherwise `stop` or `yield` where an OSM `highway=stop` / `give_way` sign applies to the inbound link (cars and bikes, not walking). A sign lies on a way node on the approach, so it counts when it is within 50 m of the end of the inbound link, before the junction, and its `direction` (`forward`, `backward` or `both`, along the way) matches the link's. Signs with no `direction`, or exactly on the junction node, say nothing about which approach and are left out |
| `geometry` | VARCHAR | ✅ | a short curve from the end of the inbound link to the start of the outbound link, WKT |
| `name`, `penalty`, `capacity` | | ∅ | empty |

A bus's turn into or out of a bus-only link (the source `edge_graph`'s rows with `uses = 'bus'`, the same U-turn
rule) is a movement too. Monaco: 27 such turns.

Where the lane ranges come from ([design](../design/gmns_lane_profile.md)): driving and cycling, SUMO netconvert's lane
connections, given duckOSM's lanes and legal turns; walking (a footpath is one strip), the lanes in order, lane 1 into lane 1.
A turn netconvert gives no lanes keeps its row without lanes and is listed in [`lane_check`](#lane_check). A range is never half
empty: both ends are set, or none.

```sql
-- from which lanes can a vehicle turn left?
SELECT i.name, k.lane_num
FROM gmns_driving.movement m
JOIN gmns_driving.link i ON i.link_id = m.ib_link_id
JOIN gmns_driving.lane k ON k.link_id = m.ib_link_id
                        AND k.lane_num BETWEEN m.start_ib_lane AND m.end_ib_lane
WHERE m.type = 'left';
```

## lane_connector

duckOSM's own table (in `gmns_driving`; a footway has no turning path through a junction in OSM, its lane runs to its node, so `gmns_walking` has only the short [footway joins](#footway-joins)), for drawing and for lane-level routing: the **path from one lane to the next** across
a junction. A movement from lanes 1-2 into lanes 1-2 has two connectors.

| Column | Type | Holds |
|---|---|---|
| `connector_id` | VARCHAR | `<from_lane_id>><to_lane_id>` |
| `mvmt_id` | VARCHAR | the [`movement`](#movement) it belongs to |
| `from_lane_id`, `to_lane_id` | VARCHAR | the two [`lane`](#lane)s |
| `width` | DOUBLE | metres (the lane width, 3.25 m by default; a U-turn is as wide as its lanes, never more than the gap between them) |
| `geom` | GEOMETRY | the path through the junction: SUMO netconvert's internal lane, from the lane's `geom_cut` end to the next lane's `geom_cut` start |

Which lane goes to which lane, and the path between them, come from SUMO netconvert, given duckOSM's lanes (`driving.lane_profile`)
and legal turns ([design](../design/gmns_lane_profile.md)). Not in the standard, so never in the CSVs.

## lane_check

duckOSM's own table, in `gmns_driving`, `gmns_cycling` and `gmns_walking`: what the lanes could not be given, listed instead of filled
in by a guess. Empty when everything maps.

| Column | Type | Holds |
|---|---|---|
| `kind` | VARCHAR | `no way out` (no movement leaves the lane: it ends before a narrowing), `no way in`, `no lane connection` (netconvert built none for a legal turn), `lane count` (SUMO built a link with other lanes than duckOSM's), `against its arrow` (a turn from a lane whose `turn:lanes` arrow says otherwise: netconvert does not read the arrows) |
| `id` | VARCHAR | the `lane_id`, `mvmt_id` or `link_id` |
| `detail` | VARCHAR | what was found |

Most rows are OSM data to check (a `lanes=2` on a one-lane road leaves a lane with no way out); fix them with `osm_overrides.yaml`.

### Footway joins

When both `driving` and `walking` are exported, `gmns_walking.lane_connector` holds one straight connector per footway end that meets a road at a node
both links share, from the nearest road lane's end to the footway lane's end, when the two are 0.3-6 m apart (a road lane sits beside its link's line, a footway lane
on it). `from_lane_id` is the road lane (the same id in `gmns_driving` and `gmns_walking`), `to_lane_id` the footway lane, `width` the footway's, `mvmt_id` the walking
movement between the two links, or NULL. Nothing moves: the footway still runs to its OSM node. See the [design](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/gmns_walk_joins.md).

## crossing

duckOSM's own table (only in `gmns_driving`), for drawing: **one pedestrian crossing painted on the road**, from the OSM
`footway=crossing` ways and from the `highway=crossing` nodes that no crossing way has. The zebra is the **best rectangle for the links it crosses** (below), then projected on each lane. Rows exist only where the crossing lies
across a driving lane **of its own level** (a ground crossing over a road in a tunnel is no crossing of that road; level: the OSM `layer`, else bridge 1 / tunnel -1) (30 degrees or more from the lane, not winding); a crossing way and the nodes on it are one crossing, and two
on one place are one (the longer wins).

| Column | Type | Holds |
|---|---|---|
| `crossing_id` | VARCHAR | `w<osm_id>` (a crossing way; `w<osm_id>#2` for its second piece, across a dual carriageway) or `n<osm_id>` (a node) |
| `source` | VARCHAR | `way` (its line is the zebra's) or `node` (a crossing node with no way: the line is square across the road, through the node) |
| `crossing_type` | VARCHAR | the OSM `crossing` tag (`marked`, `zebra`, `uncontrolled`, `unmarked`, `traffic_signals`, …) |
| `markings` | VARCHAR | the OSM `crossing:markings` tag (`yes`, `zebra`, `no`, …) |
| `painted` | BOOLEAN | are stripes painted: false for `crossing=unmarked`, `crossing:markings=no`, signals alone and no tag at all |
| `width` | DOUBLE | the zebra's width along the road, metres: the OSM `width` if tagged, else estimated from the width of the road it crosses (`length`): 0.4 × that, between 2.5 and 4 m |
| `length` | DOUBLE | the rectangle's size across the road, metres, from the first covered lane's outer edge to the last |
| `osm_id` | BIGINT | the OSM way or node |
| `geom` | GEOMETRY | the zebra's **rectangle** (a polygon): `width` along the road, `length` across it |

## lane_crossing

duckOSM's own table (only in `gmns_driving`): **one row per crossing for each driving lane it covers**, so a zebra is painted lane
by lane and follows a curved lane. Only `auto` and `bus` (and `bus,bike`) lanes: a bike lane is no part of a zebra.

| Column | Type | Holds |
|---|---|---|
| `crossing_id` | VARCHAR | the [`crossing`](#crossing) |
| `lane_id` | VARCHAR | the [`lane`](#lane), or the `connector_id` of a [`lane_connector`](#lane_connector) (road surface too: a zebra over the joint of two lanes is on both lanes and the connector between them) |
| `link_id` | BIGINT | its [`link`](#link) (a connector's: that of the lane it leaves) |
| `start_lr`, `end_lr` | DOUBLE | the stretch of this lane the rectangle covers, metres from the lane's start: where the rectangle really overlaps the lane's own shape, so a lane that is short or ends inside the zebra still has its part (`end_lr - start_lr` is up to `width`) |
| `across_from`, `across_to` | DOUBLE | the extent of that overlap across the rectangle: metres from the rectangle's first side to the overlap's two sides. Stripes laid in that one frame stay continuous from lane to lane |
| `to_left` | BOOLEAN | which way `across` grows in this lane: true to the lane's left (in its direction of travel), false to its right |

To paint: cut the lane's geometry between `start_lr` and `end_lr`, and put in it the stripes (pitch `stripe + gap` from
`across = 0`) whose centre lies between `across_from` and `across_to`, each a strip along the lane (so parallel to it) at the lateral position that `across` and `to_left` give (a lane that is not square to the rectangle's axis: the part of it). Not in the standard, so never in the CSVs. See the
[design](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/gmns_crossings.md).

## link_along

duckOSM's own table (filled in `gmns_walking`, where the footpaths are; empty in the other networks): **one row per footpath link for each road it runs along**. A sidewalk
usually follows a street made of several edges, not one, so [`link`](#link)'s `along_link_id` (the road along the longest stretch) is only the first
of them. A road is in the route when the footpath runs along it for 4 m or more (its edge within 8 m of the footpath's and parallel to it, same level). Not in the
standard, so never in the CSVs.

| Column | Type | Holds |
|---|---|---|
| `link_id` | BIGINT | the footpath's [`link`](#link) |
| `along_link_id` | BIGINT | the road's link: of `gmns_walking` or, `along_mode` `driving`, of `gmns_driving` (the same hash in every schema) |
| `along_mode` | VARCHAR | the network the road is in: `walking` or `driving` |
| `covered_m` | DOUBLE | about how many metres of the footpath run along this road (sampled every 2 m) |
| `gap_m` | DOUBLE | the metres between the two lines at their nearest |

## curb_seg

A stretch of kerb with a rule. duckOSM writes on-street parking, where OSM tags `parking:left` /
`parking:right` (or `parking:lane:<side>`). Monaco has two.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `curb_seg_id` | VARCHAR | ✅ key | `<link_id>_<side>` |
| `link_id` | BIGINT | ✅ | the [`link`](#link) |
| `ref_node_id` | BIGINT | ✅ | the link's `from_node_id`: where distance 0 is |
| `start_lr`, `end_lr` | DOUBLE | ✅ | metres from `ref_node_id`: the whole link, `0` to its length |
| `regulation` | VARCHAR | ✅ | the OSM tag value, e.g. `parallel` |
| `width` | INTEGER | ✅ | empty |

## signal_controller

Where traffic signals are. OSM has no signal timing, so there are no phases or plans: only the
controllers.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `controller_id` | VARCHAR | ✅ key | `sig_<node_id>` |
| `node_id` | BIGINT | ➕ | the signalised [`node`](#node) |
| `control_type` | VARCHAR | ➕ | `signal` |

## location

A point **along a link**: a crossing, a stop or give-way sign, a signal. It is how the standard places
street furniture, and OSM has many of these (Monaco, driving: 1,521 crossings, 108 parking entrances,
100 give-way signs, 34 signals, 20 stop signs).

A row is made for each OSM node that lies on a link (within about 0.2 m of its shape) and carries one of these
tags: `highway` = `crossing`, `bus_stop`, `give_way`, `stop`, `traffic_signals`, `toll_gantry`,
`mini_roundabout`, `speed_camera`; `traffic_calming`; `amenity=parking_entrance`; `railway=level_crossing`.
A node that lies on several links (a junction) gets a row for each. A node beside the road, on no link, is
not written: that is why Monaco has no bus stops here, its stops are platform nodes next to the road. The
table needs the raw OSM tags, like lane details do: an area clipped from a bigger build has none.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `loc_id` | VARCHAR | ✅ key | `<osm_id>_<link_id>` |
| `link_id` | BIGINT | ✅ | the [`link`](#link) it lies on |
| `ref_node_id` | BIGINT | ✅ | the link's `from_node_id` |
| `lr` | DOUBLE | ✅ | metres from `ref_node_id` along the link's shape |
| `x_coord`, `y_coord` | DOUBLE | ✅ | longitude, latitude of the OSM node |
| `loc_type` | VARCHAR | ✅ | the OSM tag value: `crossing`, `bus_stop`, `give_way`, … (the standard recommends OSM names) |
| `z_coord` | DOUBLE | ✅ | metres, between the link's two end heights by distance along it (empty without heights, and on a bridge or in a tunnel) |
| `zone_id` | BIGINT | ✅ | `1`, the [`zone`](#zone) |
| `gtfs_stop_id` | VARCHAR | ✅ | the GTFS `stop_id`, on the rows made from a GTFS feed (`--gtfs`, below); empty on the OpenStreetMap rows |
| `osm_id` | BIGINT | ➕ | the OSM node id; empty on GTFS rows |
| `name` | VARCHAR | ➕ | the stop's name: OSM's `name` tag, or the GTFS `stop_name` |
| `geom` | GEOMETRY | ➕ | the point |

### Transit stops from a GTFS feed

The GMNS README recommends GTFS for transit and the spec has `location.gtfs_stop_id` to point at it. Give the feed to
`duckosm gmns`:

```bash
duckosm gmns monaco.duckdb --gtfs monaco_gtfs.zip              # a .zip or an unzipped folder; repeat for several feeds
duckosm gmns monaco.duckdb --gtfs a.zip --gtfs b.zip --gtfs-max-m 40
```

Only `stops.txt` is needed. With `stop_times.txt`, `trips.txt` and `routes.txt` the stops are told apart by the
service that calls there (`route_type`): `bus_stop`, `tram_stop`, `train_station`, `subway_station`,
`ferry_terminal`, … in OpenStreetMap's words, and `entrance` for a station entrance (`location_type` 2). Stations (1)
and boarding areas (4) only group stops and are left out. Each stop becomes a `location` row:

- **On the nearest link of each mode within 30 m** (`--gtfs-max-m`): in the driving schema the stops a **bus** calls
  at (a tram or train stop has no place on a road); in the walking and cycling schemas every stop, since people
  walk to a tram or train stop too. Of a two-way road's two links the stop is put on the one it is on the **kerb
  side** of (its right with right-hand traffic). A stop farther than that from any link of the mode is not written
  there (the log says how many).
- `lr` is the distance along the link; `x_coord`, `y_coord` are the stop's own coordinates; `gtfs_stop_id` and
  `name` come from the feed.
- An OpenStreetMap `bus_stop` node within 25 m of a GTFS bus stop is the same stop: its row is replaced by the GTFS
  one, which carries the id.

Example: the Monaco bus network's feed ([Compagnie des Autobus de Monaco, on transport.data.gouv.fr](https://transport.data.gouv.fr/datasets/gtfs-3),
Licence Ouverte 2.0) has 97 stops; 95 of the 96 that a bus serves go onto a road link of Monaco (OpenStreetMap alone
has none there: its bus stops are platform nodes beside the road).

## zone

An area. duckOSM writes **one** zone, and every node (and every [`location`](#location)) is in it. Its outline
is the area you built, when the database was built with a boundary (`--boundary`, an H3 cell, an admin area;
Tartu has one, Monaco does not); without one the zone has a name (the output file's) and no outline. A zone is
needed because GMNS tools such as [Path4GMNS](https://github.com/jiaweizhang/path4gmns) refuse a network whose
nodes are in no zone. [lanestyle](https://github.com/Khoshkhah/lanestyle) draws the outline.

| Column | Type | Spec | Holds |
|---|---|---|---|
| `zone_id` | BIGINT | ✅ key | `1` |
| `name` | VARCHAR | ✅ | the boundary's name if it has one, else the area (the output file's name without `_gmns`) |
| `boundary` | VARCHAR | ✅ | the outline as WKT `POLYGON` / `MULTIPOLYGON`; empty without a boundary |
| `super_zone` | VARCHAR | ∅ | empty |
| `geom` | GEOMETRY | ➕ | the same outline, native |

## gmns_all: all modes in one

`--combined` adds a schema `gmns_all` with `config`, `node`, `link`, `use_definition` and `use_group`
only. A road used by several modes is **one link** (they share `link_id`), with `allowed_uses` listing
them all, e.g. `auto,bike,walk`. There are no lanes or movements here: they differ by mode, so they stay in
the per-mode schemas.

## Meso and micro networks

`--meso` and `--micro` build networks for simulators. They are **modelled on osm2gmns' meso and micro
networks** (the same idea, and many of the same column names), but they are not its files and the standard
has no meso or micro tables. Their ids are built from the `link_id`, so they lead back to the road. Lengths
are in metres, measured on each link's own geometry, and **every link starts and ends exactly at its nodes**.

- **Meso:** each road becomes a *section*, shortened by about 6 m at both ends (at most 35 % of the road), and
  each legal turn a *connector*: a smooth curve from the end of the inbound section to the start of the
  outbound one.
- **Micro:** each lane is cut into cells of about 7 m. Cells in a row are linked; side-by-side cells of
  adjacent lanes are linked by a *lane change*; and each lane pair of a movement is a *turn* link from the end
  of the inbound lane to the start of the outbound lane, along the [`lane_connector`](#lane_connector) curve
  where there is one.

| Schema.table | Rows (Monaco) | One row is | Main columns |
|---|---|---|---|
| `meso_<mode>.meso_node` | 6,184 | a section end | `node_id` (`<link_id>u` start, `<link_id>d` end), `x_coord`, `y_coord`, `macro_node_id`, `macro_link_id`, `geom` |
| `meso_<mode>.meso_link` | 7,049 | a road **section** (`meso_type = 'normal'`) or a turn **connector** (`'movement'`) | `link_id` (`M<link_id>` / `X<ib>-<ob>`), `from_node_id`, `to_node_id`, `length`, `lanes`, `capacity` (empty: it follows from the lanes), `free_speed`, `facility_type`, `macro_link_id`, `movement_id`, `mvmt_txt_id`, `start_ib_lane`, `end_ib_lane`, `geometry`, `geom` |
| `micro_<mode>.micro_node` | 22,553 | the start of a cell of a lane | `node_id` (`<lane_id>@<k>`), `x_coord`, `y_coord`, `lane_id`, `cell_k`, `macro_link_id`, `lane_no`, `geom` |
| `micro_<mode>.micro_link` | 27,123 | a cell, a lane change, or a turn (`cell_type` = `normal`, `lane_change`, `movement`) | `link_id` (`C<lane_id>#<k>`, `H…`, `X…`), `from_node_id`, `to_node_id`, `length`, `width`, `lane_no`, `macro_link_id`, `meso_link_id`, `geometry`, `geom` |

**Compared with osm2gmns 0.7.6** (run on the same Monaco PBF): osm2gmns names the road class `link_type_name`
and codes it as a number `link_type`, and codes `cell_type` as 1 / 2; we write the OSM `highway` in
`facility_type` and text in `cell_type`. It has `macro_node_id`, `zone_id`, `activity_type`, `is_boundary`
and a micro `capacity` that we do not; we have `geom`, `meso_type`, the lane ranges, `width` and `cell_k` that it
does not. Its network is also coarser (Monaco: 1,280 links against our 3,092, since it merges junctions).

## What is not written, and why

The standard has 25 tables. duckOSM writes 12 of them (everything above except `lane_connector`, `crossing`, `lane_crossing` and `link_along`, which are
its own) and leaves out the other 13:

| Tables | Why |
|---|---|
| `segment`, `segment_lane` (a lane added or dropped part-way along a link) | not needed: an edge is cut at every node that two ways share or where a way ends, so wherever OSM's `lanes` changes, the edge is already cut |
| `link_tod`, `lane_tod`, `movement_tod`, `segment_tod`, `segment_lane_tod`, `time_set_definitions` (time-of-day changes) | OSM's `*:conditional` tags are on a fraction of one percent to 7 % of ways, and are mostly parking and loading rules, not travel lanes |
| `signal_timing_plan`, `signal_timing_phase`, `signal_phase_mvmt`, `signal_coordination`, `signal_detector` | OSM has no signal timing, phases or detectors; any value would be invented |

## Checking the output

The CSVs are validated against the standard's own schemas on every test run, and `duckosm gmns --check`
tests that the values agree with each other (a link starts and ends on its nodes, `length` is its shape's,
lane numbers run 1..n, a movement turns where its inbound link ends and uses lanes that exist). See
[Conformance](gmns.md#conformance-to-the-gmns-standard).
