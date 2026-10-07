# Walking

The `walking` network holds every way people may walk on, by OSM's access rules. Build it with
`-m walking` (the Monaco sample builds it with driving and cycling):

```bash
duckosm build --config config/sample_monaco.yaml
```

```sql
SELECT walk_type, count(*) FROM walking.edges GROUP BY 1 ORDER BY 2 DESC;
```

## Which ways are walkable

The first row that matches decides:

| Way | Walkable | Why |
|---|---|---|
| `highway` = `motorway`, `motorway_link` | no | OSM forbids walking there, whatever the other tags say |
| `foot` = `yes` / `designated` / `permissive`, or `sidewalk` = `yes` / `both` / `left` / `right` | yes | an explicit permission or a sidewalk on the road wins over the rows below; also on a way without a `highway` tag (the ferry) |
| `motorroad=yes` | no | a motorroad (expressway) is closed to people on foot |
| `foot=use_sidepath` | no | people must use the path beside it |
| `sidewalk=separate`, `sidewalk:both=separate`, or `sidewalk:left` and `sidewalk:right` both `separate` | no | the sidewalk is its own `highway=footway` way, which carries the walking: keeping the road too would draw and route each street twice |
| `highway` = `footway`, `path`, `pedestrian`, `steps`, `living_street`, `residential`, `service`, `platform`, `corridor`, `track`, `bridleway`, `road`, `unclassified`, `tertiary`, `secondary`, `primary`, `trunk` and their `_link`s | yes | OSM's default access lets people walk on them, with or without a sidewalk tag |
| anything else (`cycleway`, `busway`, `raceway`, …) | no | not for walking unless `foot` says so |

Then the access tags decide, as in every mode ([Access](../concepts/networks.md#access-private-and-forbidden-roads)):
the most specific of `foot`, `access`. `no` leaves the way out; `private` moves it to
`walking.private_edges` (drawn, never routed). So `access=no` + `foot=yes` is walkable.

A way the network leaves out is still in `raw.ways`. To add or remove one road, use a
[fix](fix-osm-errors.md).

## `walk_type`

Every walking edge has a `walk_type`, from OSM sub-tags. The first rule that matches wins.

| `walk_type` | What it is | From |
|---|---|---|
| `escalator` | moving stairs | `highway=steps` with `conveying` |
| `steps` | stairs | `highway=steps` |
| `sidewalk` | a footway along a road | `highway=footway` + `footway=sidewalk`; or a road with a `sidewalk` tag (not `no` / `none` / `separate`): you walk its sidewalk |
| `crossing` | a crosswalk over a road | `highway=footway` + `footway=crossing` |
| `footpath` | a footway on its own | any other `highway=footway`, and anything no rule below matches |
| `plaza` | a pedestrian square, drawn as its outline | `highway=pedestrian` + `area=yes` |
| `pedestrian_street` | a street for people only | `highway=pedestrian` |
| `shared_street` | a street people share with slow cars | `living_street` |
| `shared_road` | a minor street walked on the carriageway | `residential`, `service`, `unclassified` |
| `corridor` | indoors | `highway=corridor`, or `indoor=yes` |
| `platform` | a public-transport platform | `highway=platform` |
| `path` | an unpaved or shared way | `path`, `track`, `bridleway` |

The full rule: [`walk_type` and `cycle_type`](../concepts/networks.md#walk_type-and-cycle_type).

## Connector footways

A footway that stops a few metres short of the street it leads to (common in OSM) would be a separate
piece, and the [component filter](../concepts/cleanup.md#component-filter) would drop it. So the build
joins each dangling path end to the nearest node within `clip.connect_snap_m` (10 m) by a straight
two-way **connector** edge: it has a negative `osm_id`, the path's `highway`, and no name.

```sql
SELECT count(*) FROM walking.edges WHERE osm_id < 0;   -- Monaco: 496
```

Details: [Connect dangling paths](../concepts/cleanup.md#connect-dangling-paths).

## Direction, speed, routing

| | Walking |
|---|---|
| One-way | never from `oneway` or roundabouts: people walk both ways. Only `oneway:foot` = `yes` / `1` / `true` / `-1` makes an edge one-way |
| Speed | 5 km/h on every edge (`maxspeed_kmh`), so `cost_s` is the walking time |
| `edge_id` | the same as the same piece of road in driving and cycling ([Stable edge ids](../concepts/edge-ids.md)) |

Route on it with `route(con, a, b, mode="walking")` or `Router(con, mode="walking")`
([Route](route.md)). `duckosm multimodal` joins walking to driving and cycling at every node they share,
so a trip can walk to the car, drive and walk on ([Routing across modes](../concepts/multimodal.md)).

## Example: a one-way street

Way [143536279](https://www.openstreetmap.org/way/143536279) in Monaco is a short `tertiary` street
between Avenue d'Alsace and Pont Sainte-Dévote, tagged `oneway=yes` and no `sidewalk`:

| | Before 2026-10-07 | Now |
|---|---|---|
| driving | 2 edges, one-way | 2 edges, one-way |
| walking | not there (a `tertiary` needed a `sidewalk` or `foot` tag) | 4 edges, both ways, 97 m, `cost_s` 70 s |

Its two walking edges in the drawing direction have the same `edge_id`s as the driving ones. Avenue de
la Porte Neuve (way 159170452, `residential`, `sidewalk=separate`) went the other way: it was walkable
before and is now only in driving, as its separately mapped sidewalks carry the walking.

On Monaco the rule added 149 roads (61 `tertiary`, 43 `secondary`, 35 `primary`, 9 `_link`s,
1 `unclassified`) and removed 4 with separate sidewalks: 11,968 walking edges instead of 11,374.
