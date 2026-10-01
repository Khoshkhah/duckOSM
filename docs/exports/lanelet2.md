# Lanelet2 (experimental)

```bash
duckosm gmns monaco.duckdb                  # the lanes first
duckosm lanelet2 monaco_gmns.duckdb         # -> monaco_gmns.lanelet2.osm: 3,182 lanelets
```

A [Lanelet2](https://github.com/fzi-forschungszentrum-informatik/Lanelet2) map, the lane-map format
of Autoware and the `lanelet2` library. It is OSM XML, so it also opens in OSM tools. Each lane of a
[GMNS database](gmns.md) becomes a lanelet: a left and a right border (the lane's centre line
± half its width), tagged `subtype` (`road`, `bus_lane`, `bicycle_lane`), `location=urban`,
`one_way=yes`, `speed_limit` (km/h) and `duckosm:edge_id`. It needs the lane centre lines that
`duckosm gmns` draws by default (not with `--no-lane-geometry`).

**It is a starting map, not a survey-grade HD map.** The borders come from OSM centre lines and a
3.25 m default width, accurate to about a metre, not centimetres. Lanelets touch where roads meet
but mostly don't share both end points (Monaco: 4 % of the legal turns do), so the `lanelet2`
router can't route across them yet. Not yet: routing between lanelets, lane-change neighbours,
traffic lights and stop lines. Checked for structure only; not yet loaded in Autoware.

```python
from duckosm import to_lanelet2
to_lanelet2("monaco_gmns.duckdb", "monaco.lanelet2.osm")     # -> {"lanelets", "ways", "nodes"}
```
