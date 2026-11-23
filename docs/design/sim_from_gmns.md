# SUMO and MATSim from the GMNS database

Status: implemented on branch `kaveh/sim-from-gmns` (2026-11-23).

## The problem

The docs put SUMO and MATSim in Level 3 (GMNS network), but both exports read the Level 2 database (`<area>.duckdb`).
`matsim-lanes` reads the GMNS database. So one MATSim network came from two databases:

- `network.xml` had no bus-only links, but `lanes.xml` (from GMNS) had lanes and turns on them;
- a lane fix in the GMNS tables did not go to SUMO or to `network.xml`.

## The decision

`duckosm sumo` and `duckosm matsim` accept two inputs. They look at the input file and select the source:

| Input | SUMO | MATSim `network.xml` |
|---|---|---|
| Level 2 db (`<mode>.edges` is there) | as before: `edges`, `edge_graph`, `lane_profile`, netconvert selects the lanes of each turn | as before |
| GMNS db (`gmns_<mode>.link` is there, `<mode>.edges` is not) | `node`, `link`, `lane` and `lane_connector`: each turn goes lane to lane, as GMNS has it | `node` and `link` |

The command and the Python call do not change: `duckosm sumo monaco_gmns.duckdb`, `to_sumo("monaco_gmns.duckdb", ...)`.

Why both inputs, and not GMNS only: GMNS is expensive. Kentucky takes 23 min and 5.0 GB for GMNS, and 11 s and 2.6 GB for
the SUMO export from Level 2. France needs about 150 GB for GMNS. The Level 2 input stays the fast input for large areas. The
GMNS input is the input when the lanes matter.

Rejected: GMNS for SUMO always. Large areas then become impossible.

Rejected: a new branch inside `sumo::to_sumo_with`. That function also does junction joins, turn arrows and chain merges.
GMNS already has the final lanes and lane connections, so the GMNS writer is a separate, small module (`sumo_gmns.rs`). It
uses the same `edge_xml`, netconvert config and `.tll.xml` output.

## No loop

`duckosm gmns` runs the SUMO export (from Level 2) and netconvert one time, and reads netconvert's lane connections into
`movement` and `lane_connector`. After that, GMNS is the source. The SUMO export from GMNS writes these lane connections back
as `<connection fromLane= toLane=>`. netconvert then builds the same lanes: it does not select them again.

## MATSim from GMNS

| MATSim | From `gmns_<mode>` |
|---|---|
| node `id`, `x`, `y`, `z` | `node.node_id`; `node.geom` in the CRS of the file (default: the UTM zone); `node.z_coord` |
| link `id` | `link.link_id` (= `edge_id`) |
| `from`, `to` | `link.from_node_id`, `link.to_node_id` |
| `length` | `link.length` |
| `freespeed` | `link.free_speed / 3.6` |
| `permlanes` | `link.lanes` (at least 1) |
| `capacity` | `link.capacity × permlanes`; empty `capacity` (no cars): the class value of `link.facility_type`, as before |
| `modes` | the mode of the schema (`car`, `bike`, `walk`); a bus-only link (`allowed_uses = 'bus'`): `bus` |

More than one mode (`-m all`): one link for each `link_id`, with the modes of all its rows, as before.

Differences from the Level 2 input: the bus-only links are in the network (Monaco: 18 links more); a link with no speed
has the GMNS speed (`free_speed`: the class value; walking 5 km/h, cycling 15 km/h), not `length / cost_s`; `length` is
the length of the GMNS line (Monaco: a median difference of 3 cm, at most 1.06 m); and the lanes are the GMNS lanes (Monaco:
one link has 2 lanes in GMNS, filled in from the road that it continues, and 1 in Level 2).

## SUMO from GMNS

| SUMO | From `gmns_<mode>` |
|---|---|
| `<node id x y>` | `node.node_id`, `x_coord`, `y_coord` (EPSG:4326; netconvert projects to UTM) |
| `<node type>` | `node.ctrl_type`: `signal` → `traffic_light`; `stop_4_way` → `allway_stop`; `stop`, `stop_2_way` → `priority_stop` |
| `<edge id from to>` | `link.link_id`, `from_node_id`, `to_node_id` |
| `priority`, `type`, `speed`, `length`, `name`, `shape` | `link.facility_type`, `free_speed / 3.6`, `length`, `name`, `geom` |
| `allow` of the edge | the mode (`pedestrian`, `bicycle`); a bus-only link: `bus` |
| `<lane index width allow>` | `lane` rows of the link, by `lane_num`, left to right. SUMO numbers from the right: `index = n - 1 - position` |
| `<connection from to fromLane toLane>` | `lane_connector` (`from_lane_id`, `to_lane_id`), each lane id made a SUMO index as above. Not for walking: a footway has one lane, and netconvert refuses a lane-to-lane connection onto its walking areas |
| `<connection from to>` | a `movement` with no `lane_connector` row (for example a U-turn that netconvert did not connect, or a walking movement) |
| `allow` of a connection | driving only, on a lane-to-lane connection: a bus-only or bike-only movement (netconvert takes `allow` only lane to lane) |
| `<connection from>` | a link with no movement out: netconvert then makes no turn there |

Not in the GMNS export:

- **Bus stops (`.add.xml`):** GMNS has no OSM bus stops. The Level 2 input writes them.
- **Signal groups:** the Level 2 input gives one traffic light to signals that are near each other (`tl`). GMNS has one
  `signal_controller` for each node, so each signal node is its own traffic light.
- **`--merge` and `--join-junctions`:** they change the network that GMNS describes. The export stops with an error.

## Test

On Monaco, with the GMNS db of `duckosm gmns`:

1. MATSim: each link of the Level 2 export is in the GMNS export with the same `from`, `to`, `length`, `capacity` and
   `permlanes`; the GMNS export has the bus-only links more.
2. SUMO: netconvert builds the network from the GMNS files with no error. Each lane-to-lane connection of
   `gmns_driving.lane_connector` is a connection of the `.net.xml` (the same lanes). The number of lanes of each edge equals
   the GMNS `lane` rows.

Result on Monaco (2026-11-23):

| Check | Result |
|---|---|
| MATSim: Level 2 links in the GMNS export | 2,840 of 2,840, with the same `from`, `to`, `freespeed` and `modes`; 18 bus-only links more (`bus`, `bike,bus`) |
| SUMO driving: lane pairs of `lane_connector` in the `.net.xml` | 4,123 of 4,123, with the same lanes; no other connections |
| SUMO driving: edges with the GMNS number of lanes | 2,858 of 2,858 |
| SUMO walking and cycling from GMNS | netconvert builds both networks with no error |

