# Walking

The `walking` network holds every way that people can walk on, by OSM's access rules. Build it with
`-m walking`. The Monaco sample builds it with driving and cycling:

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
| `highway` = `motorway`, `motorway_link` | no | OSM does not let people walk there |
| `foot` = `yes` / `designated` / `permissive`, or `sidewalk` = `yes` / `both` / `left` / `right` | yes | a permission or a sidewalk wins over the rows below |
| `motorroad=yes` | no | people on foot cannot use a motorroad (expressway) |
| `foot=use_sidepath` | no | people must use the path next to it |
| `sidewalk=separate`, `sidewalk:both=separate`, or `sidewalk:left` and `sidewalk:right` both `separate` | no | the sidewalk is a `highway=footway` way of its own. That way carries the walking, so the network does not hold the street two times |
| `highway` = `footway`, `path`, `pedestrian`, `steps`, `living_street`, `residential`, `service`, `platform`, `corridor`, `track`, `bridleway`, `road`, `unclassified`, `tertiary`, `secondary`, `primary`, `trunk` and their `_link` classes | yes | OSM's default access lets people walk there, with or without a sidewalk tag |
| all other ways (`cycleway`, `busway`, `raceway`, …) | no | not for walking, unless `foot` permits it |

A ferry line (`route=ferry`) has no `highway` tag. It is in the network when the rows above keep it.

Then the access tags decide, as in every mode ([Access](../concepts/networks.md#access-private-and-forbidden-roads)).
The most specific of `foot` and `access` decides:

| Value | Result |
|---|---|
| `no` | the way is not in the network |
| `private` | the way goes to `walking.private_edges`: drawn, never routed |

Thus `access=no` + `foot=yes` is walkable.

A way that the network does not hold is still in `raw.ways`. To add or remove one road, use a
[fix](fix-osm-errors.md).

## `walk_type`

Each walking edge has a `walk_type`, from the OSM tags. The first rule that matches decides.

| `walk_type` | What it is | From |
|---|---|---|
| `escalator` | moving stairs | `highway=steps` with `conveying` |
| `steps` | stairs | `highway=steps` |
| `sidewalk` | a footway along a road | `highway=footway` + `footway=sidewalk`; or a road with a `sidewalk` tag (not `no` / `none` / `separate`) |
| `crossing` | a crosswalk over a road | `highway=footway` + `footway=crossing` |
| `footpath` | a footway of its own | all other `highway=footway`, and all ways that no other rule matches |
| `plaza` | a pedestrian square, drawn as its outline | `highway=pedestrian` + `area=yes` |
| `pedestrian_street` | a street for people only | `highway=pedestrian` |
| `shared_street` | a street that people share with slow cars | `living_street` |
| `shared_road` | a road with no sidewalk tag: people walk on the carriageway | `residential`, `service`, `unclassified`, `road`, `tertiary`, `secondary`, `primary`, `trunk` and their `_link` classes |
| `corridor` | indoors | `highway=corridor`, or `indoor=yes` |
| `platform` | a public-transport platform | `highway=platform` |
| `path` | an unpaved or shared way | `path`, `track`, `bridleway` |

The full rule: [`walk_type` and `cycle_type`](../concepts/networks.md#walk_type-and-cycle_type).

## Connector footways

In OSM, a footway often stops some meters before the street that it goes to. Such a footway is a separate part
of the network. The build joins each open path end to the nearest node within `clip.connect_snap_m` (default 10 m),
with a straight two-way **connector** edge.

| Column | Value on a connector |
|---|---|
| `osm_id` | negative |
| `highway` | the `highway` of the path |
| `name` | NULL |
| `edge_ref` | `{osm_id}#1f` or `{osm_id}#1r` |

Use `edge_id` as the key.

```sql
SELECT count(*) FROM walking.edges WHERE osm_id < 0;
```

Details: [Connect dangling paths](../concepts/cleanup.md#connect-dangling-paths).

## Direction, speed, routing

| | Walking |
|---|---|
| One-way | never from `oneway` or a roundabout: people walk in the two directions. Only `oneway:foot` = `yes` / `1` / `true` / `-1` makes an edge one-way |
| Speed | 5 km/h on each edge (`maxspeed_kmh`), so `cost_s` is the walking time |
| `edge_id` | the same as the same part of the road in driving and cycling ([Stable edge ids](../concepts/edge-ids.md)) |

To route on it, use `route(con, a, b, mode="walking")` or `Router(con, mode="walking")` ([Route](route.md)).
`duckosm multimodal` joins walking to driving and cycling at each node that they share. Thus a trip can walk to
the car, drive, and walk on ([Routing across modes](../concepts/multimodal.md)).

## Example: a one-way street

Way [143536279](https://www.openstreetmap.org/way/143536279) in Monaco is a short `tertiary` street, tagged
`oneway=yes`, with no `sidewalk` tag.

| Network | Edges |
|---|---|
| driving | 2 edges, one-way |
| walking | 4 edges, in the two directions, `walk_type` = `shared_road` |

Its two walking edges in the drawing direction have the same `edge_id` values as the driving edges.

Avenue de la Porte Neuve (way 159170452, `residential`, `sidewalk=separate`) is only in driving. Its sidewalks are
ways of their own, and they carry the walking.
