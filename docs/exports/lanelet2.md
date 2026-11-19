# Lanelet2 (experimental)

`duckosm lanelet2` writes a [Lanelet2](https://github.com/fzi-forschungszentrum-informatik/Lanelet2) map, the lane-map
format of Autoware and of the `lanelet2` library. The file is OSM XML, so OSM tools can also open it.

| Reference and format | |
|---|---|
| Format | [Lanelet2 map](https://github.com/fzi-forschungszentrum-informatik/Lanelet2/blob/master/lanelet2_core/doc/LaneletAndAreaTagging.md) ([primitives](https://github.com/fzi-forschungszentrum-informatik/Lanelet2/blob/master/lanelet2_core/doc/LaneletPrimitives.md)) in [OSM XML](https://wiki.openstreetmap.org/wiki/OSM_XML) |
| Reference tool | none: the map is not yet loaded in Autoware or in the `lanelet2` library |
| Check | none yet against a tool. The tests check the structure and the tags of the lanelets. [How each output is checked](../concepts/checks.md) |

```bash
duckosm gmns monaco.duckdb                  # first: the lanes
duckosm lanelet2 monaco_gmns.duckdb         # -> monaco_gmns.lanelet2.osm
```

Monaco test build: 3,321 lanelets.

## What it makes

The command reads a [GMNS database](gmns.md). Each lane becomes one lanelet:

| Lanelet2 item | From the GMNS lane |
|---|---|
| left and right border | the lane center line ± half the lane width (3.25 m if the width is empty) |
| `subtype` | `road`, `bus_lane`, `bicycle_lane` or `walkway`, from the lane use |
| `speed_limit` | the link speed, km/h |
| `location`, `one_way` | `urban`, `yes` |
| `duckosm:edge_id` | the `edge_id` of the road |

It needs the lane center lines that `duckosm gmns` makes by default. Do not use `--no-lane-geometry` for this.

## Options

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode` | the GMNS mode | `driving` |
| `-o`, `--out` | the output file | `<name>.lanelet2.osm` |

```python
from duckosm import to_lanelet2
to_lanelet2("monaco_gmns.duckdb", "monaco.lanelet2.osm")     # -> {"lanelets", "ways", "nodes"}
```

Design: [lanelet2_export](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/lanelet2_export.md).

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). Time: the wall-clock time. Memory: the peak memory footprint (`/usr/bin/time -l`). 1 MB = 10⁶ bytes.

| Area | Time | Memory | Output size |
|---|---|---|---|
| Monaco (0.7 MB) | 0.11 s | 63.3 MB | 10.5 MB |
| Kentucky (148 MB) | measuring |  |  |
| France (5.1 GB) | measuring |  |  |

The command: `duckosm lanelet2 area_gmns.duckdb`. The input is the GMNS database of `duckosm gmns`.

## Limits

**It is a start map, not a survey-grade HD map.** The borders come from OSM center lines and default widths. They are
correct to about one meter, not to centimeters.

- Lanelets touch where roads meet, but most do not share both end points. Thus the `lanelet2` router cannot route
  across them.
- Not yet: routes between lanelets, lane-change neighbors, traffic lights and stop lines.
