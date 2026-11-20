# duckOSM for automated driving users

This page is for users of the CommonRoad Scenario Designer, Lanelet2, scenariogeneration and CARLA. Each table gives
the call of your tool and the duckOSM call that does the same work. The module is `duckosm.hdmap`
([reference](../reference/sdk/hdmap.md)). The notebook [9. HD maps](../notebooks/09_hdmap.md) runs each call on
Monaco.

```bash
pip install "duckosm[sim,hdmap]"     # netconvert (the lanes of GMNS) and commonroad-io
```

All lane maps come from the lanes of the GMNS step: `net.driving.to_gmns("out/gmns.duckdb")`. So the CommonRoad map,
the Lanelet2 map and the lane routes have the same lanes, and your lane fixes (`lanes`, `turn_lanes_*` in the fixes
file) are in each.

## CommonRoad

| CommonRoad Scenario Designer | duckOSM |
|---|---|
| OSM → lanelets (`crdesigner` OSM conversion) | `duckosm.hdmap.to_commonroad("out/gmns.duckdb", "monaco.xml")` |
| `CommonRoadFileReader("monaco.xml").open()` | the same call: commonroad-io reads the file |

| Part of the map | From |
|---|---|
| a lanelet for each lane | the GMNS `lane`: its center line to the edge of the junction (`geom_cut`) and its width |
| a lanelet for each turn through a junction | the GMNS `lane_connector` (from SUMO's netconvert) |
| predecessor, successor | lane → connector → lane |
| left and right neighbour | the lanes of one link, same direction |
| lanelet type | `bus_lane`, `bicycle_lane`, else `urban` |
| coordinates | metres east and north of the center of the area; `location` has its latitude and longitude |

The result has `ids`: the `lanelet_id`, the GMNS lane or connector id, and the `edge_id`. Join your data on the
`edge_id`.

## Lanelet2, OpenDRIVE and lane routes

| Your tool | duckOSM |
|---|---|
| Lanelet2 map from OSM (by hand in JOSM) | `duckosm.hdmap.to_lanelet2("out/gmns.duckdb", "monaco.osm")` |
| `lanelet2.routing` | `duckosm.hdmap.lane_graph(gmns_db)`, `duckosm.hdmap.route_lanes(gmns_db, a, b)` |
| scenariogeneration, CARLA's OSM import | `net.driving.to_opendrive("monaco.xodr", junctions=True)` (experimental) |
