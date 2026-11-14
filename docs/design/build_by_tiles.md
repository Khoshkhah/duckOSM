# Build by tiles

A build of a large area (for example France) by tiles keeps the memory of each step small: each tile builds alone, and the
union puts the tables of the tiles together.

```bash
duckosm-rust tiles france.osm.pbf --boundary france.geojson -o out --memory 2G -- --modes driving
```

## The steps

1. **Cut** (`tilecut::cut_to_tiles`, in its own process). One read of the region file gives one file for each standard map
   tile (`z_x_y.osm.pbf`) of the boundary. A tile file has the elements of the cut of the area to the tile: the nodes
   inside the area and in the tile, the ways with such a node (whole), and their relations. The list of tiles comes from
   one function, `tiles::tiles`. The cut completes the multipolygons of a tile with all their members (osmium's smart
   cut) only when the build reads them (`--raw`, the features, the building faces): the road network does not, and in
   France they were 60 of the 80 million nodes that the tiles took from outside.
2. **Junctions file** (`z_x_y.junctions`). For each node outside the tile, the cut writes whether it is a junction in the
   whole area, and whether the tile is its **home tile**:
   - A tile does not have all ways of a node outside the tile, so without the file it does not see the junctions there,
     and its edges are too long.
   - No tile holds a node outside the area. The cut gives each such node a home tile: the first tile (in the order of
     the tiles) of the ways at the node, and the cut adds all ways of the node to the home tile. These ways have a node
     inside the area, so they are complete, and they bring no more work: their own outside nodes are in the file too.
3. **Tile builds** (each in its own process with `--tile-threads` threads; see "The scheduler"). `--tile z/x/y` keeps
   the part of the area in the tile and the home nodes (`Boundary::within_tile`, `Boundary::with_points`), and the tile
   file is the input as it is (it is already the cut: no second cut); `--outside-junctions` adds the junctions of the
   file. Each writer of the tile writes only its **own rows**
   (`union::Owner`; the network tables in `write_tables_owned`, the edge graph, `ways`, `lane_profile` and
   `global_junctions` at once, the small tables at the end in `union::own_rows`):
   - an edge and its lanes: the tile that holds its source node; a turn, a turn permission and a turn restriction: the
     tile that holds the target node of the from edge; a node: the tile that holds it;
   - a way of the mode (`ways`, `way_nodes`): the tile of its first node inside the area;
   - a raw node and a raw way: the tile of the node (of the first point of the way) inside the area;
   - a virtual node (the middle of a split loop or parallel road): the tile of its place.

   The tile that holds a node has all ways of the node, so its rows there are complete. The rows that no tile alone owns
   (outside the area: raw nodes, raw ways, virtual nodes; and the relations) go to a border file.
4. **Union** (`union::union_all`): for each table, the files of the tiles one after the other, copied as blocks of bytes
   (no decode, no new compression), and the border rows, merged with one copy of each row. Then the tables of the whole
   area (`boundary`, `visualization_metadata`, `transfers`) from the boundary and the nodes of the union: the nodes stay
   on disk (the point of the time zone from passes over the nodes file, the transfers from a sort on disk).

## The largest part

The filter of the largest part (on by default, as in the whole build; `--no-component-filter` turns it off) needs the whole
network, so it runs after the tile builds and before the union, with memory that does not grow with the area:

1. Each tile build finds the connected parts of all its edges (its own and the copies of other tiles; not the private
   edges): a union-find on their end nodes (`union::TileParts`). The tile that holds a node has all edges of the node, so
   the part of each node and edge that it owns is complete. It writes small side files: the parts (own edges, smallest
   node id), the part of each edge and node, and the links (the part at each node that another tile holds or shares).
2. The links of all tiles, sorted by node, join the parts of the tiles: a union-find on the parts, not on the nodes
   (`union::keep_largest_part`). The largest part has the most edges, then the smallest node id (the rule of the whole
   build).
3. Each tile (the tiles at the same time) drops the rows of the other parts: its edges, nodes and lanes, and the turns
   and turn rules that use a dropped edge. The private edges stay, as in the whole build.

Kentucky: equal to the whole build with the filter (2,224,823 of 2,261,507 edges stay). France: 25,546,974 of
25,919,914 edges stay.

## The bus routes

A bus route crosses tiles: a tile has only some of its ways. So the walk of the routes (`bus::routes`, the function of
the whole build) runs once, after the union (`union::bus_routes`):

1. Each tile build writes `bus.osm.pbf`: its route relations (`route=bus`, `trolleybus`, `share_taxi`) with their way
   members only, and those member ways with their nodes and only the tags of their direction (`oneway`, `junction`).
2. The union reads these files with one copy of each relation and way, and puts the routes into batches of about 20,000
   member ways, in the order of their ids (the order of the whole build).
3. One read of the union edges and bus lanes puts each edge of a member way, without its line, into one temporary file,
   with a row for each batch that needs it; the file is sorted by batch.
4. Each batch walks its routes on its own edges only, and writes its rows to a part file; the part files are joined as
   blocks.

Kentucky: equal to the whole build (149 routes, 46,892 route edges). France: 25,985 routes, 7,479,730 route edges; the
bus step uses a heap of 586 MB (all edges at once were 6.5 GB).

## The levels

The drawing levels (`visualization.edge_levels`) run after the union, from its edge tables. Each pass reads only the
columns of the levels (`edge_id`, the line, `highway`, `tunnel`, `bridge`, `layer`, `junction`) from the Parquet files:
no temporary copy of the edges. One sort puts the edges of each tile together, in the order of the whole build (the
table, then `edge_id`):

- `--levels solve`: by tiles in their order, each with the numbers of the earlier tiles fixed (`levels::tiled`, the code
  of the whole build, with the union tables as its source): memory that does not grow with the area (France: 2.25 GB).
- `--levels tags` (the default): the rule of the whole build by tiles too (roadstyle `tags_tile`). The rule is local: the
  level of a road depends only on the edges at its end nodes. Each tile gives the rule its own edges and the edges at
  their ends, and keeps the rows of its own edges (France: 2.33 GB, before 11.7 GB with all level edges in memory).

Kentucky: the level rows are equal to the whole build with both methods (2,449,396 rows); the meta row differs only in
`created`. France: 27,789,657 level rows.

The speed of the solve: a tile starts when the tiles that it waits for are done (not when the tiles before it are
done), and the tiles run in the order of their depth in the waits. The rule reads the numbers of the earlier tiles in
their own order, so the result does not change. France on WSL (20 cores):

| Step of the levels | Before | Now |
|---|---|---|
| Read the edge tables and sort them by `edge_id` | 180 s | (not done) |
| The boxes of the tiles | 13 s | 9 s |
| The edges to their tiles | 59 s | 16 s |
| The sort by tile | 39 s | 25 s |
| The tiles | 102 s | 31 s |
| The rows | 10 s | 6 s |
| All the levels | 404 s | 86 s |

The union with the levels: 151 s (before 491 s); peak memory 1.99 GB. Before, the levels kept each tag set of the ways
(558 MB) and the list of all edge ids (220 MB); now the memory of the levels does not grow with the area.

## The scheduler

`--memory` (default 2 GB) is the memory of the tile builds at the same time. The tiles start largest file first, at most
`--jobs` at a time (default: the cores / `--tile-threads`), and a tile starts only when the estimates of the running
tiles and its own stay below the limit; a tile larger than the limit runs alone. The estimate of a tile is a line of its
file size: the command measures the peak of each finished tile process (`wait4`), fits base + factor × size to the
finished tiles (least squares), and adds the largest peak above the line. France (driving): 38 MB + 10.5 × size, 70 MB
above the line at most; Kentucky with `raw.*`: 73 MB + 22.7 × size. `<out>/tiles.log` has the file size and the peak of
each tile.

More memory does not make the build faster: the time is in the work of the tiles, not in waiting for memory.

The rows of a table are in the order of the tiles (each tile in its own order), not in one order for the whole file.

## Result

| Area | Cut | Tile builds | Union | Total |
|---|---|---|---|---|
| Kentucky, 142 tiles (driving, `--raw`) | 3.4 s | 6.6 s | 1.6 s | 11.7 s |
| France, 1,311 tiles (driving, 6 × 2) | 83.1 s | 81.1 s | 19.7 s | 183.9 s |

- Kentucky: the 19 tables are equal to the whole build, except `built_at`.
- France: the peak of the run is the cut (1.6 GB, in its own process); the largest tile (Paris) uses 0.78 GB. The whole
  build of France in one process took 894 s and 7.3 GB.

The zoom (France, 6 × 2, before the tile builds read their file without a second cut):

| Zoom | Tiles | Cut | Tile builds | Union | Total | Tile builds together | Largest tile |
|---|---|---|---|---|---|---|---|
| 10 | 1,311 | 107.3 s | 102.0 s | 19.2 s | 228.5 s | 1.5 GB | 0.96 GB |
| 9 | 376 | 96.5 s | 91.1 s | 13.1 s | 200.8 s | 2.8 GB | 1.52 GB |
| 8 | 104 | 92.1 s | 99.5 s | 23.8 s | 215.4 s | 5.5 GB | 1.83 GB |

The zoom changes the time a little (the work is the same, only cut into other pieces) and the memory much: zoom 10 stays
the default.

The tiles at the same time and the threads of each tile (France, 12 cores, before the writers of own rows):

| Tiles × threads | Total | Tile builds together |
|---|---|---|
| 4 × 3 | 262.5 s | 1.04 GB |
| 6 × 2 | 244.3 s | 1.06 GB |
| 12 × 1 | 251.8 s | 1.37 GB |

The default is the cores / 2 tiles at a time, with 2 threads each.

## Not yet in a build by tiles

- The `.duckdb` output: the build by tiles writes GeoParquet files.
