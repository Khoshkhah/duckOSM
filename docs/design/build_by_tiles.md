# Build by tiles

A build of a large area (for example France) by tiles keeps the memory of each step small: each tile builds alone, and the
union puts the tables of the tiles together.

```bash
duckosm-rust tiles france.osm.pbf --boundary france.geojson -o out --zoom 10 --jobs 6 --tile-threads 2 -- --modes driving
```

## The steps

1. **Cut** (`tilecut::cut_to_tiles`, in its own process). One read of the region file gives one file for each standard map
   tile (`z_x_y.osm.pbf`) of the boundary. A tile file has the elements of the cut of the area to the tile: the nodes
   inside the area and in the tile, the ways with such a node (whole), and their relations. The list of tiles comes from
   one function, `tiles::tiles`.
2. **Junctions file** (`z_x_y.junctions`). For each node outside the tile, the cut writes whether it is a junction in the
   whole area, and whether the tile is its **home tile**:
   - A tile does not have all ways of a node outside the tile, so without the file it does not see the junctions there,
     and its edges are too long.
   - No tile holds a node outside the area. The cut gives each such node a home tile: the first tile (in the order of
     the tiles) of the ways at the node, and the cut adds all ways of the node to the home tile. These ways have a node
     inside the area, so they are complete, and they bring no more work: their own outside nodes are in the file too.
3. **Tile builds** (`--jobs` at a time, each in its own process with `--tile-threads` threads). `--tile z/x/y` keeps the
   part of the area in the tile and the home nodes (`Boundary::within_tile`, `Boundary::with_points`);
   `--outside-junctions` adds the junctions of the file. Each writer of the tile writes only its **own rows**
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
   area (`boundary`, `visualization_metadata`, `transfers`) from the boundary and the nodes of the union.

The rows of a table are in the order of the tiles (each tile in its own order), not in one order for the whole file.

## Result

| Area | Cut | Tile builds | Union | Total |
|---|---|---|---|---|
| Kentucky, 142 tiles (driving, `--raw`) | 3.4 s | 7.4 s | 0.7 s | 11.6 s |
| France, 1,311 tiles (driving, 6 × 2) | 106.0 s | 102.6 s | 14.3 s | 222.9 s |

- Kentucky: the 19 tables are equal to the whole build, except `built_at`.
- France: the peak of the run is the cut (1.6 GB, in its own process); the tile builds at the same time use 1.2 GB,
  the largest tile (Paris) 0.97 GB. The whole build of France in one process took 894 s and 7.3 GB.

The tiles at the same time and the threads of each tile (France, 12 cores, before the writers of own rows):

| Tiles × threads | Total | Tile builds together |
|---|---|---|
| 4 × 3 | 262.5 s | 1.04 GB |
| 6 × 2 | 244.3 s | 1.06 GB |
| 12 × 1 | 251.8 s | 1.37 GB |

The default is the cores / 2 tiles at a time, with 2 threads each.

## Not yet in a build by tiles

- The steps that need the whole area: the bus routes, the drawing levels (in the order of the tiles), the filter of the
  largest part. The tile builds run without them.
- The `.duckdb` output: the build by tiles writes GeoParquet files.
