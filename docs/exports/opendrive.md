# OpenDRIVE (experimental)

```bash
duckosm opendrive monaco.duckdb                               # -> monaco.xodr: 1,940 roads
duckosm gmns monaco.duckdb                                    # for junctions: a GMNS db first
duckosm opendrive monaco_gmns.duckdb --junctions -o monaco.xodr   # + 1,720 connecting roads, 470 junctions
```

An ASAM OpenDRIVE 1.7 `.xodr`, for driving simulators (CARLA, esmini) and microsimulators (PTV
Vissim, Aimsun). Each directed edge becomes one road (`road id` = `edge_id`): a centre line in metres
(the UTM zone of the data, or `--crs`) and its driving lanes, 3.25 m wide unless OSM tags
`width:lanes`. With [elevation](../guides/elevation.md), roads get an elevation profile.

Without `--junctions`, roads don't connect through intersections: good to look at, not to drive.
With `--junctions` (reading a [GMNS database](gmns.md)), each legal turn becomes a connecting road
along a smooth curve, grouped into junctions, so vehicles can drive through.

**Limits:** checked for structure only (the ASAM schema isn't freely available), not yet loaded in
CARLA or esmini. The connecting roads start and end close to, not exactly at, the road ends, which a
strict simulator may report. No road markings, signals or banking.

```python
from duckosm import to_opendrive
to_opendrive("monaco.duckdb", "monaco.xodr")                          # -> {"roads", "crs"}
to_opendrive("monaco_gmns.duckdb", "monaco.xodr", junctions=True)
```
