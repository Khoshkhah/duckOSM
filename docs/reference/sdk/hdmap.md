# duckosm.hdmap

Automated driving and HD maps. Lanelet2 and the lane graph read a GMNS database ([planning](planning.md)).

| Call | Returns | Today |
|---|---|---|
| `to_opendrive(net, out_path, mode=None, crs=None, junctions=False)` | the ASAM OpenDRIVE file of one mode (experimental) | yes |
| `to_lanelet2(gmns_db, out_path, mode="driving", snap_m=0.01)` | the Lanelet2 map (experimental) | yes |
| `lane_graph(gmns_db, mode="driving")` (`build_lane_graph`) | the counts of the lane graph | yes |
| `route_lanes(gmns_db, from_lane, to_lane, mode="driving")` | a route from lane to lane | yes |
| `to_commonroad(view, path)` | a CommonRoad map | later |

```python
net.driving.to_opendrive("out/monaco.xodr")          # = duckosm.hdmap.to_opendrive(net.driving, ...)
```

Pages: [OpenDRIVE](../../exports/opendrive.md), [Lanelet2](../../exports/lanelet2.md),
[Lane-level routing](../../exports/lane-routing.md).
