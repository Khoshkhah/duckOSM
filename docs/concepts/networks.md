# What each network contains

Each mode is its own schema with its own `edges`, `nodes` and `edge_graph`. Monaco:

| Mode | Edges | Nodes | Speed |
|---|---|---|---|
| `driving` | 2,133 | 1,246 | the speed limit, or a default per road class |
| `walking` | 9,232 | 3,602 | 5 km/h |
| `cycling` | 8,440 | 3,561 | 15 km/h; 5 km/h on dismount edges |

## Which OSM ways each mode keeps

| Mode | Keeps |
|---|---|
| `driving` | `highway` = `motorway`, `trunk`, `primary`, `secondary`, `tertiary` (and their `_link`s), `unclassified`, `residential`, `service`, `living_street`, `road`. Also `highway=pedestrian` when `motorcar` or `motor_vehicle` is `yes` / `designated` / `permissive`, or `access` is `delivery` / `destination` / `agricultural`: these become `living_street` |
| `walking` | `highway` = `footway`, `path`, `pedestrian`, `steps`, `living_street`, `residential`, `service`, `platform`, `corridor`; and any way tagged `sidewalk` = `yes` / `both` / `left` / `right`, or `foot` = `yes` / `designated` |
| `cycling` | `highway` = `cycleway`, `path`, `track`, `bridleway`, `living_street`, `residential`, `service`, `unclassified`, `tertiary`, `secondary`, `primary` (and their `_link`s); any way tagged `bicycle` = `yes` / `designated` / `permissive`, or with a `cycleway` tag other than `no` / `none` / `separate`. Then drops `bicycle=no`, and `access=private` / `no` unless bicycles are allowed. Plus [dismount edges](#dismount-edges) |

Access tags don't remove a road from driving or walking: a way tagged `access=private`,
`motor_vehicle=no` or `foot=no` stays in. Monaco: 108 driving edges and 242 walking edges are on such
ways. To remove a way, use a [fix](../guides/fix-osm-errors.md) with `exclude_modes`.

A way tagged `foot` or `bicycle` is kept even without a `highway` tag. In Monaco this brings the
ferry (`route=ferry`, `foot=yes`) into walking.

`highway=service` (driveways, parking aisles, alleys) is part of the network like any other road.

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

Monaco: 643 of the 2,133 driving edges have a `maxspeed` tag.

`cost_s`, the travel time in seconds, is `length_m / (maxspeed_kmh / 3.6)`. It uses the speed limit:
no traffic, no delay at junctions. Routing and the edge graph use it.

## Lanes

`lanes` is the number of lanes in the edge's own direction, never empty:

1. `lanes:forward` / `lanes:backward`, when tagged.
2. A one-way road: all of `lanes`.
3. A two-way road: `lanes` split in two; the drawing direction gets the larger half.
4. Nothing tagged: 2 for `motorway` and `trunk`, otherwise 1.

A `lanes:reversible` lane is added to both directions. Monaco: 884 of the 2,133 driving edges are on
a way with a lanes tag.

## Dismount edges

Cycling also gets footways and pedestrian streets, so that cycleways joined only by them stay
connected. On those where riding isn't allowed (`bicycle` isn't `yes` / `designated` /
`permissive`), `dismount` is `TRUE`: the bike is pushed, at 5 km/h, in both directions, and
`cycle_type` is `dismount`. They are left out when `access` is `private` / `no` or `foot=no`.
Monaco: 5,300 of the 8,440 cycling edges. For a ride-only network, use `WHERE NOT dismount`;
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
| `shared_road` | `residential`, `service`, `unclassified` |
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
| `mixed_traffic` | anything else |

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
