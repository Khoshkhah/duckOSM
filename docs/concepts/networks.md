# What each network contains

Each mode is its own schema with its own `edges`, `nodes` and `edge_graph`. Monaco:

| Mode | Edges | Nodes | Speed |
|---|---|---|---|
| `driving` | 2,765 | 1,719 | the speed limit, or a default per road class |
| `walking` | 10,952 | 4,116 | 5 km/h |
| `cycling` | 10,274 | 4,151 | 15 km/h; 5 km/h on dismount edges |

## Which OSM ways each mode keeps

| Mode | Keeps |
|---|---|
| `driving` | `highway` = `motorway`, `trunk`, `primary`, `secondary`, `tertiary` (and their `_link`s), `unclassified`, `residential`, `service`, `living_street`, `road`. Also `highway=pedestrian` when `motorcar` or `motor_vehicle` is `yes` / `designated` / `permissive`, or `access` is `delivery` / `destination`: these become `living_street` |
| `walking` | every way people may walk on: `highway` = `footway`, `path`, `pedestrian`, `steps`, `living_street`, `residential`, `service`, `platform`, `corridor`, `track`, `bridleway`, `road`, `unclassified`, `tertiary`, `secondary`, `primary`, `trunk` (and their `_link`s). Not a road tagged `motorroad=yes`, `foot=use_sidepath`, or `sidewalk` (or `sidewalk:both`, or both `sidewalk:left` and `sidewalk:right`) = `separate`: people walk on its separately mapped sidewalk. Any way tagged `foot` = `yes` / `designated` / `permissive` or `sidewalk` = `yes` / `both` / `left` / `right` is kept. Never `motorway` or `motorway_link`. [Walking](../guides/walking.md) |
| `cycling` | `highway` = `cycleway`, `path`, `track`, `bridleway`, `living_street`, `residential`, `service`, `unclassified`, `tertiary`, `secondary`, `primary` (and their `_link`s); any way tagged `bicycle` = `yes` / `designated` / `permissive`, or with a `cycleway` tag other than `no` / `none` / `separate`. Plus [dismount edges](#dismount-edges) |

Then the [access tags](#access-private-and-forbidden-roads) decide, per mode.

A way tagged `foot` or `bicycle` is kept even without a `highway` tag. In Monaco this brings the
ferry (`route=ferry`, `foot=yes`) into walking, where it is walked at 5 km/h like a footpath, and the
outline of a pedestrian area tagged `bicycle=yes` into cycling, as a ring of edges.

`highway=service` (driveways, parking aisles, alleys) is part of the network like any other road.

## Access: private and forbidden roads

For each mode, the most specific access tag decides:

| Mode | Tags, most specific first |
|---|---|
| `driving` | `motorcar`, `motor_vehicle`, `vehicle`, `access` |
| `walking` | `foot`, `access` |
| `cycling` | `bicycle`, `vehicle`, `access`; on a [dismount edge](#dismount-edges): `foot`, `access` |

| Its value | The road in that mode |
|---|---|
| `no` (driving also `agricultural`, `forestry`, `emergency`) | **not there.** It stays in the other modes that may use it: a road tagged `motor_vehicle=no` is still walkable |
| `private` | **visible, never routable**: in `<mode>.private_edges`, not in `edges` |
| driving only: for buses only (below) | **visible, never routable**, like a private road: in `driving.private_edges` with `access = 'bus'` |
| anything else (`yes`, `destination`, `delivery`, …) or none | a normal road |

So `access=private` + `motor_vehicle=yes` is a normal road for driving, and `motor_vehicle=no` doesn't
affect walking or cycling. The mode's value is kept in `edges.access`.

**Private roads** (a driveway, a gated street, a company road) are built like every other road,
with the same `edge_id` formula, then moved from `edges` to `private_edges`, which has the same
columns except the H3 ones. The component filter doesn't prune `private_edges`. So routing, the graph of legal turns and every export use only roads you may use, and a
road you can reach only through a private one is dropped with the
[component filter](cleanup.md#component-filter). The maps still draw private roads, marked
([Draw a map](../guides/draw-map.md)).

**Bus lanes and bus-only roads** are cars' private roads too: in driving they are built, then moved to
`private_edges` with `access = 'bus'`. A bus-only road is one with `highway=busway`, or whose driving
access is `psv`, or `no` / `private` while `bus` or `psv` is `yes` / `designated` (`psv` means buses
and taxis; both count as `bus`). A **bus lane against a one-way street** (`oneway=yes` with
`oneway:bus=no` or `oneway:psv=no`) adds the reverse edge, as a bus lane: its `edge_id` is the same as
that direction in cycling and walking. Monaco: 18 bus edges, among them the contraflow bus lane of
Boulevard des Moulins.

Monaco:

| Mode | Routable edges | In `private_edges` |
|---|---|---|
| `driving` | 2,765 | 214 (196 private, 18 for buses) |
| `walking` | 10,952 | 154 |
| `cycling` | 10,274 | 133 |

A way a mode leaves out is still in `raw.ways`, with all its tags. To remove or keep one road
yourself, use a [fix](../guides/fix-osm-errors.md).

## One-way roads

A one-way edge has no reverse twin, so the road can only be used from `source` to `target`.

| Mode | One-way when |
|---|---|
| `driving` | `oneway` = `yes` / `1` / `true` / `-1`; `junction` = `roundabout` / `circular`; `motorway` and `motorway_link` unless `oneway=no` |
| `cycling` | as driving (without the motorway rule), but `oneway:bicycle=no` makes it two-way and `oneway:bicycle=yes` one-way. Dismount edges are always two-way |
| `walking` | only `oneway:foot` = `yes` / `1` / `true` / `-1` |

`oneway=-1` (one-way against the way's drawing direction) is turned round, so its edge runs the
legal way.

## Speeds and travel time

`maxspeed_kmh` is never empty. For driving it is the `maxspeed` tag when it is a number, converted
when it ends in `mph`, and otherwise (untagged, or a value like `RU:urban` or `walk`) a default:

| Road class | km/h |
|---|---|
| `motorway`, `motorway_link` | 110 |
| `trunk`, `trunk_link` | 90 |
| `primary`, `primary_link` | 70 |
| `secondary`, `secondary_link` | 60 |
| `tertiary`, `tertiary_link` | 50 |
| `unclassified`, `road` | 40 |
| `residential` | 30 |
| `living_street`, `service` | 20 |
| any other class | 50 |

Monaco: 899 of the 2,765 driving edges have a `maxspeed` tag.

`cost_s`, the travel time in seconds, is `length_m / (maxspeed_kmh / 3.6)`. It uses the speed limit:
no traffic, no delay at junctions. Routing and the edge graph use it.

## Lanes

`lanes` is the number of lanes in the edge's own direction, never empty, except on
[connector edges](cleanup.md#connect-dangling-paths) (walking, cycling), where it is NULL:

1. `lanes:forward` / `lanes:backward`, when tagged.
2. A one-way road: all of `lanes`.
3. A two-way road: `lanes` split in two, at least 1 each way; the drawing direction gets the larger half.
4. Nothing tagged: 2 for `motorway` and `trunk`, otherwise 1.

A `lanes:reversible` lane is added to both directions. Monaco: 1,165 of the 2,765 driving edges are on
a way with a lanes tag.

## Dismount edges

Cycling also gets footways and pedestrian streets, so that cycleways joined only by them stay
connected. On those where riding isn't allowed (`bicycle` isn't `yes` / `designated` /
`permissive`), `dismount` is `TRUE`: the bike is pushed, at 5 km/h, in both directions, and
`cycle_type` is `dismount`. Their access is the walking one (`foot`, then `access`): `no` leaves
them out, `private` moves them to `private_edges`. A [connector edge](cleanup.md#connect-dangling-paths)
that extends a dismount path is dismount too. Monaco: 7,240 of the 10,274 cycling edges. For a ride-only network, use `WHERE NOT dismount`;
`options.cycling_dismount: false` leaves them out of the build.

## `walk_type` and `cycle_type`

Walking and cycling edges get a type from OSM sub-tags. The first rule that matches wins.

| `walk_type` | From |
|---|---|
| `escalator` / `steps` | `highway=steps`, with / without `conveying` |
| `sidewalk` / `crossing` / `footpath` | `highway=footway` with `footway=sidewalk` / `footway=crossing` / anything else |
| `plaza` / `pedestrian_street` | `highway=pedestrian`, with / without `area=yes` |
| `shared_street` | `living_street` |
| `sidewalk` | any other road with a `sidewalk` tag (not `no` / `none` / `separate`): you walk its sidewalk |
| `shared_road` | `residential`, `service`, `unclassified`, `road`, `tertiary`, `secondary`, `primary`, `trunk` (and their `_link`s): walked on the carriageway |
| `corridor` | `highway=corridor`, or `indoor=yes` |
| `platform` | `highway=platform` |
| `path` | `path`, `track`, `bridleway` |
| `footpath` | anything else |

| `cycle_type` | From |
|---|---|
| `dismount` | a [dismount edge](#dismount-edges) |
| `cycleway` | `highway=cycleway` |
| `cycle_track` / `cycle_lane` / `shared_lane` / `bus_cycle_lane` | the cycleway tag on the edge's side: `track` / `lane` / `shared_lane` / `share_busway` |
| `segregated_path` / `shared_path` | `path`, `track`, `bridleway`, with / without `segregated=yes` |
| `mixed_traffic` | anything else, including a footway or pedestrian street you may ride (`bicycle=yes`) |

The side is `cycleway:right` for an edge in the drawing direction and `cycleway:left` for its
reverse, then `cycleway:both`, then `cycleway`.

## Turn restrictions

Only the driving network has them. They come from OSM relations `type=restriction` with a `from`
way, a `via` node and a `to` way, and from the turn rules of a [fixes file](../guides/fix-osm-errors.md).
A `no_*` restriction removes that one turn from `edge_graph`; an `only_*` restriction removes every
other turn from the `from` edge. Monaco: 38 of the 43 restriction relations map onto edges.

Not used: restrictions through a via way, the `except` tag, and tags like `restriction:hgv` or
`restriction:conditional`. Any turn not removed is allowed, including a U-turn onto the reverse
twin of a two-way road.
