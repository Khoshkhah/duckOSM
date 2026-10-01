# OpenDRIVE (experimental)

```bash
duckosm opendrive monaco.duckdb                               # -> monaco.xodr: 2,765 roads
duckosm gmns monaco.duckdb                                    # for junctions: a GMNS db first
duckosm opendrive monaco_gmns.duckdb --junctions                  # -> monaco_gmns.xodr: + 1,744 connecting roads, 482 junctions
```

An ASAM OpenDRIVE 1.7 `.xodr`, for driving simulators (CARLA, esmini) and microsimulators (PTV
Vissim, Aimsun). Each directed edge becomes one road (`road id` = `edge_id`): a centre line in metres
(the UTM zone of the data, or `--crs`) and its driving lanes, each 3.25 m wide (OSM's `width:lanes`
isn't used yet). With [elevation](../guides/elevation.md), roads get an elevation profile (not yet
with `--junctions`).

Without `--junctions`, roads are not linked to each other at all: good to look at, not to drive.
With `--junctions` (reading a [GMNS database](gmns.md)), each legal turn at an intersection becomes
a connecting road along a smooth curve, grouped into junctions, so vehicles can drive through; where
a road simply continues, the two roads link directly.

**Limits:** checked for structure only, not against the ASAM schema, and not yet loaded in CARLA or
esmini. A connecting road has one lane, linked to the rightmost lane of the roads it joins. It starts
and ends near the road ends, usually about 2 m off but sometimes tens of metres (Monaco: 220 of
1,744 more than 5 m), which a strict simulator may report. No road type or speed limit, road
markings, signals or banking.

```python
from duckosm import to_opendrive
to_opendrive("monaco.duckdb", "monaco.xodr")                          # -> {"roads", "crs"}
to_opendrive("monaco_gmns.duckdb", "monaco.xodr", junctions=True)
```
