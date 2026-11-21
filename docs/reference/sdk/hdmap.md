# duckosm.hdmap

Automated driving and HD maps. Lanelet2 and the lane graph read a GMNS database ([planning](planning.md)).

| Call | Returns | Today |
|---|---|---|
| `to_opendrive(net, out_path, mode=None, crs=None, junctions=False)` | the ASAM OpenDRIVE file of one mode (experimental) | yes |
| `to_lanelet2(gmns_db, out_path, mode="driving", snap_m=0.01)` | the Lanelet2 map (experimental) | yes |
| `lane_graph(gmns_db, mode="driving")` (`build_lane_graph`) | the counts of the lane graph | yes |
| `route_lanes(gmns_db, from_lane, to_lane, mode="driving")` | a route from lane to lane | yes |
| `to_commonroad(gmns_db, path, mode="driving", country_id="ZAM", map_name=None)` | a CommonRoad scenario map (XML, commonroad-io): a lanelet for each lane (to the edge of the junction) and for each lane connector, with predecessors, successors and the lanes on each side; metres from the center of the area. Returns `{path, lanelets, lanes, connectors, ids}`; `ids`: `lanelet_id` → GMNS id and `edge_id` | yes |

```python
net.driving.to_opendrive("out/monaco.xodr")          # = duckosm.hdmap.to_opendrive(net.driving, ...)
net.driving.to_gmns("out/gmns.duckdb")
duckosm.hdmap.to_commonroad("out/gmns.duckdb", "out/monaco_commonroad.xml")
```

pip extra: `duckosm[hdmap]` (commonroad-io). `to_commonroad` works with commonroad-io 2022.1 to 2026.1: commonroad-io
2024 and later need protobuf 3.20, so pip takes an older version when another package needs a newer protobuf. Notebook: [14. HD maps](../../notebooks/14_hdmap.md). Guide:
[duckOSM for automated driving users](../../compare/for-hdmap-users.md).

Pages: [OpenDRIVE](../../exports/opendrive.md), [Lanelet2](../../exports/lanelet2.md),
[Lane-level routing](../../exports/lane-routing.md).
