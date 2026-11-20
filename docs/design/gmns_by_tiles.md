# GMNS by tiles

Status: built, 2026-11-19. The section "Result" gives the numbers.

`duckosm gmns` kept the rows of the whole area in memory in some steps (the lanes: all edges with their OSM tags and
lines; the SUMO step: all lanes, then netconvert on the whole area). Kentucky (driving): 1085 s and 25.9 GB on the
Mac mini, more than 15 GB on WSL. Now an area with nodes in two or more map tiles is made one tile at a time.

## The steps of one tile

The tiles are the map tiles of `cut`, `quack --tile-zoom` and the build by tiles (`pbfcut::tiles::Tile`), zoom 9 by
default (`duckosm gmns --tile-zoom`, 0: the whole area at once).

| Step | What it does |
|---|---|
| 1. Tile source | A small duckOSM db with the rows of the source that the GMNS steps read: the edges whose box meets the tile and a band of 2 km around it (`HALO_M`), their nodes, `edge_graph`, the turn tables, `lane_profile`, `private_edges`, the OSM ways of the edges and the ways with a node in the band, with all their nodes. The rows keep the order of the source. |
| 2. GMNS | The GMNS steps of `gmns.rs`, with no change, on the tile source, into a GMNS db of the tile (with netconvert on the tile). |
| 3. Merge | The rows that the tile owns go into the GMNS db. |

Only one tile source and one tile GMNS db are on disk at a time (in `<out>.tiles/`). They go after each tile.

## Which tile owns a row

| Table | Owner |
|---|---|
| `node`, `signal_controller`, `movement` | the tile of the node (`x_coord`, `y_coord`) |
| `link` | the tile of its `from_node_id` |
| `lane`, `curb_seg`, `link_along`, `geometry`, `location` | the tile of the link |
| `lane_connector` | the tile of the movement |
| `crossing`, `lane_crossing` | the tile of the middle of the crossing |
| `lane_check` | the tile of the link or the movement in `id` |
| `config`, `use_definition`, `use_group`, `zone` | the same in each tile: from the first tile |

A table with no rule stops the export, so a new table cannot go wrong without a message.

## What crosses the edge of a tile

The band gives each owned row the roads around it. Two things are not in the band:

| Item | Effect |
|---|---|
| A run of lane pieces (one road through several edges, `chain_runs`) longer than the band | The lane lines near the end of the band can start at another piece. Only `lane.geom` (checked by its properties, not by equality). |
| The metre frame of the paired carriageways and of the crossings: the first row of the area | In a tile it is the first row of the tile. The frame is nearer to the road, so the measure is better. Only the geometry columns. |

The whole-area export does not give the same lines on two runs either: its lane rows come from a join, and the order of
a join on a large area changes from run to run. The ring runs and the frames follow that order. A fixed order
(`ORDER BY` the source row) changes the Monaco golden files, so it is not in this change (it needs Kaveh's yes).

## Result

Kentucky, driving, zoom 9 (49 tiles). WSL (20 cores, 23 GB). Memory: the program and netconvert together.

| Run | Time | Memory |
|---|---|---|
| Whole area, without netconvert | 187 s | 12.2 GB |
| By tiles, without netconvert | 346 s | 3.4 GB |
| Whole area, with netconvert | stopped past 15.8 GB (Mac mini: 1085 s, 25.9 GB) | |
| By tiles, with netconvert | 1893 s | 4.9 GB |

Without netconvert and without the crossings (the whole area stops in the crossings without netconvert, a GEOS
`TopologyException`), by tiles against the whole area: 123 columns, each with the same count and the same values, but
`lane.geom` (and the name of the file in `config` and `zone`).

| `lane.geom` differs (Hausdorff distance) | Rows | More than 1 cm | More than 1 m | Largest |
|---|---|---|---|---|
| Whole area against a second whole run | 17,801 | 968 | 755 | 225 m |
| By tiles against the whole area | 41,885 | 3,071 | 831 | 168 m |

The largest ones are ring roads, whose lanes start at another piece of the ring (the order of the rows), not the edges
of the tiles.

The time is larger, because each tile copies its source rows and the band is done two times or more. Not measured
yet: by tiles against the whole area with netconvert (the whole area needs about 26 GB).
