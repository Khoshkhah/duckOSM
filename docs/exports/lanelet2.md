# Lanelet2 (experimental)

`duckosm lanelet2` writes a [Lanelet2](https://github.com/fzi-forschungszentrum-informatik/Lanelet2) map, the lane map
of Autoware and of the `lanelet2` library, with your OSM fixes. The file is OSM XML, so OSM tools can also open it.

| Reference and format | |
|---|---|
| Format | [Lanelet2 map](https://github.com/fzi-forschungszentrum-informatik/Lanelet2/blob/master/lanelet2_core/doc/LaneletAndAreaTagging.md) in [OSM XML](https://wiki.openstreetmap.org/wiki/OSM_XML) |
| Reference tool | the [`lanelet2`](https://github.com/fzi-forschungszentrum-informatik/Lanelet2) library: chapter 14 of the notebook manual loads the Monaco map with it. It reads 3318 lanelets with 0 faults, but in its `RoutingGraph` only 732 lanelets have a next lanelet. No test loads the map in Autoware yet |
| Check | the tests check the structure and the tags of the lanelets. [How each output is checked](../concepts/checks.md) |

```bash
duckosm gmns monaco.duckdb                  # first: the lanes
duckosm lanelet2 monaco_gmns.duckdb         # -> monaco_gmns.lanelet2.osm
```

## The lanelets

Each lane of the [GMNS database](gmns.md) becomes one lanelet:

| Lanelet2 item | From the GMNS lane |
|---|---|
| left and right border | the center line of the lane ± half its width (3.25 m when the width is not known) |
| `subtype` | `road`, `bus_lane`, `bicycle_lane` or `walkway`, from the use of the lane |
| `speed_limit` | the speed of the link, in km/h |
| `location`, `one_way` | `urban`, `yes` |
| `duckosm:edge_id` | the `edge_id` of the road |

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB).

| Area (source .osm.pbf) | Time | Memory | Output |
|---|---|---|---|
| Monaco (0.7 MB) | 0.11 s | 63.6 MB | 10.6 MB |
| Kentucky (148 MB) ¹ | 1 min 24 s | 10.2 GB | 9.8 GB |
| France (5.1 GB) | not run | | |

¹ On the GMNS database made by tiles (`duckosm gmns`, zoom 10). lanelet2 reads the whole database, so its memory does not
go down with the tiles. France can need more than 16 GB.

## Limits

| Limit | Why |
|---|---|
| The borders are correct to about 1 m, not to cm | they come from OSM center lines and lane widths, not from a survey |
| The `lanelet2` router cannot route from one lanelet to the next | most lanelets touch where roads meet, but do not share both end points |
| No lane-change neighbours, traffic lights or stop lines | not made yet |
