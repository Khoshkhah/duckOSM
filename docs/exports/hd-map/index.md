# 4. HD map

The HD map (high-definition map): each lane with its exact shape and its connections, for driving simulators and autonomous driving. These exports are experimental. These outputs read the
GMNS database of `duckosm gmns`, because it has the lanes and the turns of each lane.

The planned HD-map database (built with SimReady) has the chain as its unit: one carriageway for each chain of
`gmns_driving.chains`, which keeps the list of its links. The exports of this page work lane by lane on the GMNS links today. [Edges and chains](../../concepts/edges-and-chains.md).

```mermaid
flowchart LR
  gmns[("<area>_gmns.duckdb<br/>(3D geometry)")] --> ll2["Lanelet2 (.osm)"]
  gmns --> xodr["OpenDRIVE (.xodr)"]
  gmns --> lr["Lane-level routing"]
```

| Read by | Export | Command |
|---|---|---|
| Your own lane router | [Lane-level routing](../lane-routing.md): a lane graph | `duckosm lane-graph`, `route-lanes` |
| Autoware, lanelet2 | [Lanelet2](../lanelet2.md): `.osm` | `duckosm lanelet2` |
| CARLA, esmini, Vissim | [OpenDRIVE](../opendrive.md): `.xodr` | `duckosm opendrive` |

All three are **experimental**. Tests check their structure. They are not loaded in the target tools yet.

## The tables

Every table of the HD map database, with its columns: [HD map tables](../hdmap_tables.md).
