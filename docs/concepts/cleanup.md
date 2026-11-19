# Network clean-up

OSM data has small gaps: a footway that ends a metre short of the road it meets, or a road that the border of the area
cuts. The build closes the first kind and keeps the second kind whole. Each road stays, also a road that does not connect
to the rest, for example on an island.

## Connect dangling paths

A **dangling end** is the end of a `cycleway`, `footway`, `path`, `steps`, `pedestrian` or `track` that touches no other
edge. In walking and cycling, the build adds a straight **connector edge** in both directions, to the nearest node within
10 m (`clip.connect_snap_m`). Without it, the path is a separate piece of the network.

| A connector edge has | Value |
|---|---|
| `osm_id` | negative: `-min(a, b)`, where `a` and `b` are the `osm_id` of the path and of the road. Select connectors with `osm_id < 0` |
| `highway` | the `highway` of the path |
| `oneway` | `FALSE` |
| `edge_id` | the [usual formula](edge-ids.md) |

| Limit | Why |
|---|---|
| A connector can be longer than the real gap | it joins the nearest node, not the nearest point of an edge |
| A path can join a road on a bridge above it | the distance does not look at levels |
| Driving is not connected | only walking and cycling |

`clip.connectivity_rescue: false` turns this off.

## Where the border cuts

| Build | At the border |
|---|---|
| from an OSM file | each way with a node inside the boundary is kept whole, so roads that cross the border go past it. `boundary.buffer_m` grows the boundary first |
| cut from a built database (`duckosm extract`) | each edge that touches the boundary is kept whole |

## Validation

The build can check each network and stop on a fault: [Check a build](../guides/check-build.md).
