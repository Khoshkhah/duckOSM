# Walking

The `walking` network holds every way that people can walk on, by the OSM access rules. Build it with `-m walking`.

```sql
SELECT walk_type, count(*) FROM walking.edges GROUP BY 1 ORDER BY 2 DESC;
```

## Which ways are walkable

The first row that matches decides:

| Way | Walkable |
|---|---|
| `highway` = `motorway`, `motorway_link` | no |
| `foot` = `yes` / `designated` / `permissive`, or `sidewalk` = `yes` / `both` / `left` / `right` | yes |
| `motorroad=yes`, or `foot=use_sidepath` | no |
| `sidewalk=separate`, `sidewalk:both=separate`, or `sidewalk:left` and `sidewalk:right` both `separate` | no: the sidewalk is a way of its own, and that way carries the walking |
| `highway` = `footway`, `path`, `pedestrian`, `steps`, `living_street`, `residential`, `service`, `platform`, `corridor`, `track`, `bridleway`, `road`, `unclassified`, `tertiary`, `secondary`, `primary`, `trunk` and their `_link` classes | yes |
| all other ways (`cycleway`, `busway`, `raceway`, ...) | no |

Then the most specific of `foot` and `access` decides, as in each mode: `no` removes the way, and `private` puts it in
`walking.private_edges` (drawn, not routed). Thus `access=no` with `foot=yes` is walkable
([Access](../concepts/networks.md#access-private-and-forbidden-roads)). To add or remove one road, use a
[fix](edit-osm-data.md#the-fixes-file).

## `walk_type`

The first row that matches decides:

| `walk_type` | From |
|---|---|
| `escalator` | `highway=steps` with `conveying` |
| `steps` | `highway=steps` |
| `sidewalk` | `footway=sidewalk`, or a road with a `sidewalk` tag (not `no` / `none` / `separate`) that is not `pedestrian`, `living_street`, `path`, `track`, `bridleway`, `corridor` or `platform` |
| `crossing` | `footway=crossing` |
| `footpath` | all other `highway=footway`, and all ways that no other row matches |
| `plaza` | `highway=pedestrian` with `area=yes` |
| `pedestrian_street` | `highway=pedestrian` |
| `shared_street` | `living_street` |
| `shared_road` | a road with no sidewalk tag (`residential`, `service`, `unclassified`, `road`, `tertiary` to `trunk` and their `_link` classes): people walk on the carriageway |
| `corridor` | `highway=corridor`, or `indoor=yes` |
| `platform` | `highway=platform` |
| `path` | `path`, `track`, `bridleway` |

## Direction, speed and ids

| | Walking |
|---|---|
| One-way | only with `oneway:foot` = `yes` / `1` / `true` / `-1`. Each other edge goes in the two directions |
| Speed | 5 km/h (`maxspeed_kmh`), so `cost_s` is the walking time |
| `edge_id` | the same as the same part of the road in driving and cycling ([Stable edge ids](../concepts/edge-ids.md)) |
| Connector edges | a footway that stops near a street gets a straight edge to the nearest node within `clip.connect_snap_m` (10 m), with a negative `osm_id` ([Connect dangling paths](../concepts/cleanup.md#connect-dangling-paths)) |

Example: way [143536279](https://www.openstreetmap.org/way/143536279) in Monaco is a `tertiary` street with `oneway=yes`
and no `sidewalk` tag. Driving has 2 edges, in one direction. Walking has 4 edges, in the two directions, with
`walk_type` = `shared_road`.

To route on foot, use `mode="walking"` ([Route](route.md)).
