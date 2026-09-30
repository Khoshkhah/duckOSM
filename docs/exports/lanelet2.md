# Lanelet2 (experimental)

```bash
duckosm gmns monaco.duckdb                  # the lanes first
duckosm lanelet2 monaco_gmns.duckdb         # -> monaco_gmns.lanelet2.osm: 2,469 lanelets
```

A [Lanelet2](https://github.com/fzi-forschungszentrum-informatik/Lanelet2) map, the lane-map format
of Autoware and the `lanelet2` library. It is OSM XML, so it also opens in OSM tools. Each lane of a
[GMNS database](gmns.md) becomes a lanelet: a left and a right border (the lane's centre line
± half its width), tagged `subtype` (road, bus lane, bike lane), `one_way=yes`, `speed_limit`, and
`duckosm:edge_id`. Lanelets that follow each other share their end points, so they connect.

**It is a starting map, not a survey-grade HD map.** The borders come from OSM centre lines and a
3.25 m default width, accurate to about a metre, not centimetres. Not yet: lane-change neighbours,
traffic lights and stop lines. Checked for structure only; not yet loaded in Autoware.

```python
from duckosm import to_lanelet2
to_lanelet2("monaco_gmns.duckdb", "monaco.lanelet2.osm")     # -> {"lanelets", "ways", "nodes"}
```
