# Build by tiles

A build of a large area (for example France) by tiles keeps the memory of each step small: each tile builds alone, and the
union joins the tables of the tiles.

```bash
duckosm-rust tiles france.osm.pbf --boundary france.geojson -o out --zoom 10 --jobs 4 -- --modes driving --raw
```

## The steps

1. **Cut** (`tilecut::cut_to_tiles`). One read of the region file gives one file for each standard map tile
   (`z_x_y.osm.pbf`) of the boundary. A tile file has the same elements as the cut of the area to the tile: the nodes
   inside the area and in the tile, the ways with such a node (whole), and their relations. The list of tiles comes from
   one function, `tiles::tiles`.
2. **Junctions file** (`z_x_y.junctions`). A tile does not have all ways of a node outside the tile. Without help, the tile
   does not see the junctions at such a node, and its edges are too long. The cut counts the uses of each node by all
   ways, and writes for each tile the nodes outside the tile that are junctions in the whole area (node id, road
   junction or not).
3. **Tile builds.** Each tile builds with the normal build, `--jobs` tiles at a time, each in its own process:
   `--tile z/x/y` keeps only the part of the area in the tile, and `--outside-junctions` adds the junctions of the file.
   The edges of a tile are then the same as the edges of the whole build.
4. **Union** (`union::union_all`):
   - edges, private edges and nodes: one copy of each row (the copies of an edge must be equal; the union counts the
     copies that are not);
   - the turns (`edge_graph`, `turn_permission`): from the tile that holds the node of the turn (the node is inside the
     area and in the tile). At a node outside the area, no tile has all roads, so the union makes the turns with the
     build's `EdgeGraph`;
   - `lane_profile`: from the tile that holds the source node of the edge;
   - the tables that the tiles share (`ways`, `way_nodes`, `raw.*`, `global_junctions`, ...): a merge of the sorted tile
     files that keeps one row for each key, at most 16 files at a time;
   - the tables of the whole area (`boundary`, `visualization_metadata`, `transfers`) from the boundary and the nodes of
     the union.

## Result

Kentucky, 142 tiles of zoom 10, against the whole build (driving, `--raw`, no filter of the largest part): the 19 tables
are equal, except `built_at`. Cut 3.3 s, tile builds 7.0 s, union 15.3 s, peak RSS 1.3 GB.

## Not yet in a build by tiles

- The steps that need the whole area: the bus routes, the drawing levels (in the order of the tiles), the filter of the
  largest part. The tile builds run without them.
- The `.duckdb` output: the build by tiles writes GeoParquet files.
