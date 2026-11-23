# What each network contains

Each mode is a schema of its own, with its own `edges`, `nodes` and `edge_graph`. Your [OSM fixes](../guides/edit-osm-data.md)
can remove a way from a mode, or change its one-way, lanes and turns.

| Mode | Keeps | Speed |
|---|---|---|
| `driving` | roads from `motorway` to `service`, `living_street`, `road` | the speed limit, or a default for the road class |
| `walking` | footways, paths, steps and the roads that people can walk on; never `motorway` ([Walking](../guides/walking.md)) | 5 km/h |
| `cycling` | cycleways, paths, tracks and roads, and the dismount edges | 15 km/h; 5 km/h on dismount edges |

A way with no `highway` tag is in no network, except a ferry line. A way that a mode does not keep stays in `raw.ways`.

## Access: private and forbidden roads

The most specific access tag of the mode decides: driving `motorcar`, `motor_vehicle`, `vehicle`, `access`; walking `foot`,
`access`; cycling `bicycle`, `vehicle`, `access`.

| Value | The road |
|---|---|
| `no` (driving also `agricultural`, `forestry`, `emergency`) | not in this mode |
| `private` | in `<mode>.private_edges`: drawn, never routed |
| bus-only road or bus lane (driving) | in `driving.private_edges` with `access = 'bus'` |
| other or none | a normal road |

### Pedestrian streets in driving

A pedestrian street that cars can use is a `living_street` in `driving.edges`. All other pedestrian streets go to
`driving.private_edges` with `highway = 'pedestrian'`. Their `access` is `delivery`, `private` or `pedestrian`.

## One-way roads

| Mode | One-way when |
|---|---|
| `driving` | `oneway` = `yes` / `1` / `true` / `-1`; a roundabout; `motorway` unless `oneway=no` |
| `cycling` | as driving, but `oneway:bicycle` wins. Dismount edges are two-way |
| `walking` | only `oneway:foot` |

## Speeds and travel time

`maxspeed_kmh` is the `maxspeed` tag, or a default from 110 km/h (`motorway`) to 20 km/h (`living_street`, `service`).
`cost_s = length_m / (maxspeed_kmh / 3.6)`: no traffic, no delay at junctions.

## Lanes

`lanes` is the number of lanes in the edge's direction: from `lanes:forward` / `lanes:backward`, else from `lanes`, else 2 for
`motorway` and `trunk` and 1 for other roads. It is NULL on connector edges and on ways with no cars (footways, paths,
cycleways, steps).

## Dismount edges

Cycling also gets footways and pedestrian streets, so that the cycleways stay connected. Where riding is not allowed, `dismount`
is TRUE: you push the bike at 5 km/h. For a ride-only network, use `WHERE NOT dismount`. `options.cycling_dismount: false`
removes them.

## `walk_type` and `cycle_type`

Walking and cycling edges get a type from OSM sub-tags, for example `sidewalk`, `crossing`, `steps` (walking) and `cycleway`,
`cycle_lane`, `dismount`, `mixed_traffic` (cycling).

## Turn restrictions

Only driving has them, from OSM `type=restriction` relations (via a node) and from a [fixes file](../guides/edit-osm-data.md#the-fixes-file).
A `no_*` restriction removes one turn from `edge_graph`. An `only_*` restriction removes all other turns.

## Bus routes

OSM bus route relations are matched to the driving edges, in schema [`bus`](../reference/database.md#bus-schema)
(`bus.routes`, `bus.route_edges`). A PBF build with driving makes them; [`duckosm bus DB`](../reference/cli.md#bus) adds them later.
`bus.routes.gaps` lists each way that the route cannot follow.
