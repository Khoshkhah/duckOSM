# duckosm.hdmap

Automated driving and HD maps. Lanelet2 and the lane graph read a GMNS database ([planning](planning.md)).

| Call | Returns | Today |
|---|---|---|
| `to_opendrive(net, out_path, mode=None, crs=None, junctions=False)` | the ASAM OpenDRIVE file of one mode (experimental). `net`: a network, a mode view or a GMNS database. `junctions=True` reads the junctions from a GMNS database: with a network, the function writes the GMNS database of the mode to a temporary folder first | yes |
| `to_lanelet2(gmns_db, out_path, mode="driving", snap_m=0.01)` | the Lanelet2 map (experimental) | yes |
| `lane_graph(gmns_db, mode="driving")` (`build_lane_graph`) | the counts of the lane graph | yes |
| `route_lanes(gmns_db, from_lane, to_lane, mode="driving")` | a route from lane to lane | yes |
| `to_commonroad(gmns_db, path, mode="driving", country_id="ZAM", map_name=None)` | a CommonRoad scenario map (XML, commonroad-io): a lanelet for each lane (to the edge of the junction) and for each lane connector, with predecessors, successors and the lanes on each side; metres from the center of the area. Returns `{path, lanelets, lanes, connectors, dropped, ids}`; `dropped`: the lanes and connectors that are not in the file, because a border is not one line (231 of 7450 on Monaco); `ids`: `lanelet_id` → GMNS id and `edge_id`, only for the lanelets in the file | yes |
| `surfaces(gmns_db, mode="driving")` | the road surfaces as two GeoDataFrames (EPSG:4326), `(lanes, junctions)`. `lanes`: one polygon for each lane, its line to the edge of the junction with the lane width, flat ends (`lane_id`, `edge_id`, `lane_num`, `width`, `allowed_uses`). `junctions`: one polygon for each junction node, the union of its lane connectors with their widths (`node_id`, `connectors`) | yes |

```python
net.driving.to_opendrive("out/monaco.xodr")          # = duckosm.hdmap.to_opendrive(net.driving, ...)
net.driving.to_opendrive("out/monaco_junctions.xodr", junctions=True)
net.driving.to_gmns("out/gmns.duckdb")
duckosm.hdmap.to_commonroad("out/gmns.duckdb", "out/monaco_commonroad.xml")
lanes, junctions = duckosm.hdmap.surfaces("out/gmns.duckdb")    # the road surfaces as polygons
```

pip extra: `duckosm[hdmap]` (commonroad-io). `to_commonroad` works with commonroad-io 2022.1 to 2026.1: commonroad-io
2024 and later need protobuf 3.20, so pip takes an older version when another package needs a newer protobuf. Notebook: [14. HD maps](../../notebooks/14_hdmap.md). Guide:
[duckOSM for automated driving users](../../compare/for-hdmap-users.md).

Pages: [OpenDRIVE](../../exports/opendrive.md), [Lanelet2](../../exports/lanelet2.md),
[Lane-level routing](../../exports/lane-routing.md).
