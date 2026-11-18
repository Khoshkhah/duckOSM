# Build by tiles

A build of a large area (for example France) by tiles keeps the memory of each step small: each tile builds alone, and the
union puts the tables of the tiles together.

```bash
duckosm-rust tiles france.osm.pbf --boundary france.geojson -o out --memory 2G -- --modes driving
```

## The steps

1. **Cut** (`tilecut::cut_to_tiles`, in its own process). The cut is the pbfcut library (`Khoshkhah/pbfcut`, pinned by a
   git `rev` in `rust/duckosm-core/Cargo.toml`); `duckosm_core::tilecut` only keeps the names and gives pbfcut the rule of
   the junctions (the `NodeUses` hook). One read of the region file gives one file for each standard map
   tile (`z_x_y.osm.pbf`) of the boundary. A tile file has the elements of the cut of the area to the tile: the nodes
   inside the area and in the tile, the ways with such a node (whole), and their relations. The list of tiles comes from
   one function, `tiles::tiles`. The cut completes the multipolygons of a tile with all their members (osmium's smart
   cut) only when the build reads them (`--raw`, the features, the building faces): the road network does not, and in
   France they were 60 of the 80 million nodes that the tiles took from outside.
2. **Junctions file** (`z_x_y.junctions`). For each node outside the tile, the cut writes whether it is a junction in the
   whole area, and whether the tile is its **home tile**:
   - A tile does not have all ways of a node outside the tile, so without the file it does not see the junctions there,
     and its edges are too long.
   - No tile holds a node outside the area. The cut gives each such node a home tile: the tile that holds the place of
     the node, when that tile has a file, else the first tile (in the order of the tiles) of the ways at the node. The cut
     adds all ways of the node to the home tile. (Before 2026-11-17 the home was always the first tile of the ways. The
     connector of the dangling paths runs only near its tile, so Tartu lost one connector edge: pbfcut PR 55.) These ways have a node
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

Removed on 2026-11-14, with the component filter of the whole build (Kaveh: a road that does not connect to the main
network is a real road). There is no step for it after the tile builds any more, and the tiles write no side files of
their connected parts. Before, France kept 25,546,974 of 25,919,914 edges; now all of them stay.

## The bus routes

A bus route crosses tiles: a tile has only some of its ways. So the walk of the routes (`bus::routes`, the function of
the whole build) runs once, after the union (`union::bus_routes`):

1. Each tile build writes `bus.osm.pbf`: its route relations (`route=bus`, `trolleybus`, `share_taxi`) with their way
   members only, and those member ways with their nodes and only the tags of their direction (`oneway`, `junction`).
2. The union writes the relations and the member ways of these files to two temporary files sorted by id, with one copy of
   each, and puts the routes into batches of about 20,000 member ways, in the order of their ids (the order of the whole
   build). Only the list of (member way, batch) stays in memory: 16 bytes for each.
3. The member ways go to a temporary file sorted by batch. One read of the union edges and bus lanes puts each edge of a
   member way, without its line, into one more temporary file, with a row for each batch that needs it, sorted by batch.
4. Each batch reads its relations, its member ways and its edges from the three files, walks its routes, and writes its
   rows of both tables to part files; the part files are joined as blocks.

The read of the edges decodes only the columns of the walk, not the lines. Kentucky: equal to the whole build (154 routes,
37,893 route edges). France: 25,985 routes, 7,501,173 route edges; the bus step takes 9.3 s with a heap of 171 MB (before:
14 s and 609 MB, with all relations and member ways in memory; all edges at once were 6.5 GB). Details:
`tail_steps_by_tiles.md`.

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

The numbers of an edge that later tiles read stay in memory only until the last of these tiles starts (each entry counts its
tiles; the `tags` rule keeps none): France, the tiles 187 -> 67 MB of heap (`tags`), 207 -> 94 MB (`solve`)
(`tail_steps_by_tiles.md`).

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

## The sidewalks

The sidewalk rule (`sidewalks.rs`) gives a road without a sidewalk tag `sidewalk=separate` when sidewalk lines (or the other
half of a dual carriageway) cover both its sides. It reads the lines within 12 m of the road.

- Each road has its own flat frame, from the mean latitude of its points (since 2026-11-14). Before, the frame came from
  the mean latitude of all lines of the input, so a tile, with other lines, gave other distances. This changes the
  whole build a little: Kentucky walking, 92 and 162 different edges of 2.6 million.
- A road crosses tiles, and the rule decides the whole road from its spots. So the sidewalk step runs between the cut and
  the tile builds (`--sidewalks-step`): each tile decides the roads that it owns (`Owner::way`) with the car roads and the
  sidewalk lines of its file and of the files of the tiles that the boxes of its roads touch (30 m more). The ids go
  into one sorted file, and each tile build sets the tag from it (`--sidewalks`) and does not run the rule.

Kentucky walking: the step finds 2,948 roads, as the whole build; `ways_walking` and `way_nodes_walking` are equal. The step
takes 7.1 s.

A line of another tile is checked only against the boxes of the roads in its cells of a grid (about 1 km). Before
2026-11-17 each line was checked against all boxes of the tile: at zoom 8 the tile of Louisville has about 100,000 roads,
and the step of Kentucky (the three modes) took 23.0 s; now 3.3 s.

## The connectors

The connector of the dangling paths (`connect_paths`) joins a dangling path end to the nearest node within `snap_m` (10 m).
It was not local in three ways, and each one has a fix:

- **A false dangling end.** A node at the edge of what a tile has can look like a dangling end (the tile lacks its other
  ways). A tile build runs the connector only at the ends within `snap_m` of the tile (`Options::connect_near`).
- **A missing target.** The cut gives each tile also the ways with a node in the area within `border_m` = 2 × `snap_m` +
  5 m of the tile (`tilecut::near_tile`; 0 for driving only). At an end near the tile, the tile then has all the ways at the
  end and all the nodes within `snap_m` of it. The tile build keeps these ways; their rows belong to other tiles.
- **The number.** The number in `edge_ref` was the rank of the connector among all connectors with the same `osm_id`. Now
  it is always 1 (a connector has no way; `edge_id` is the key). This changes `edge_ref` of the connectors in the whole build
  (Kentucky walking: 1,682 edges).

Kentucky walking, Kentucky driving and cycling, and Monaco with Studio edits at zoom 14 in all three modes: all tables are
equal to the whole build.

## The building faces

The walls of the buildings (`main.building_faces`, with `--raw --building-faces`) run after the union, as the bus routes and the
levels. A wall reads the streets within 150 m and the other buildings within 60 m (the line of sight, the party walls), and these
can be in other tiles.

1. Each tile build writes the rings of the buildings that it owns, with the OSM fixes, to `building_rings.bin` (the tile of the
   first point of the building, `Owner::place`), the buildings outside the area to `building_rings.border.bin`, and the ids of the
   entrance nodes to `building_entrances.bin`. It makes no walls.
2. The union reads the rings of the tiles in their order, one copy of each building of the border parts, and all entrances. The
   street segments come from the edges of the union (driving, then walking), as the whole build reads them from the network.
3. The faces are the whole build's (`building_faces_of`, in bands: the memory does not grow with the area).

A segment that two edges have (an edge and its reverse edge, or two modes) takes the street of the first mode, and in one mode
the edge with the lowest `edge_id`. Before 2026-11-17 the rule was "the first copy in the order of the edges"; the edges of the
whole build are in the order of `edge_id`, so its walls do not change (Monaco, Tartu, Kentucky: the same file, byte for byte),
and the rule no longer depends on the order of the tiles.

`building_id` is the OSM id of the way or the relation of the building (with `building_type`). Monaco at zoom 14, Tartu and
Kentucky by tiles: the walls are equal to the whole build in every row (Kentucky: 6,887,305 walls).

## H3 and the heights

H3 (`--h3-resolution`) and the heights (`--dem`) are local: the row of a node or an edge needs only its own points.

- Each tile computes them from all its nodes and edges, before it keeps its own rows. It writes its own rows and the
  border rows, as for the other tables.
- A tile reads only the window of the DEM over its nodes and lines, 1 pixel more (`Dem::window`): the strips or the
  tiles of the GeoTIFF over the window. Before, each build read the whole raster. A pixel keeps the numbers of the
  whole raster, so the heights are the same.
- The union writes `elevation_metadata_main` (the counts of all tiles; each tile gives its nodes with no height) and
  `elevation_grid_main`. The grid is the pixels over the box of the nodes of every mode, 3 pixels more. The union writes
  it in strips of 256 rows of the raster, so its memory does not grow with the area.

Kentucky (driving, `--h3-resolution 8`) and Monaco with FABDEM N43E007 (driving and walking, zoom 12, `--dem`):
all tables are equal to the whole build. The whole build gives the same rows as before.

## The raw tables in the tiles

With `--raw`, each tile writes the raw rows of its file (`write_area_raw_split`): the rows of the tile go to `raw_*.parquet`, the
border rows to `raw_*.border.parquet`, and the rows of other tiles nowhere, as the tables are written (`union::raw_classes`: a
node by its place, a way by the place of its first node inside the area, a relation always a border row). Before, the tile
wrote all rows and then read the files again to split them (`own_rows`): about half of the cost of raw in the tiles.

Kentucky, driving, `--raw`, the old and the new build one after the other (two runs each): the tile builds 12.6 s -> 10.9 s,
the CPU time 214 s -> 197 s; without raw the tile builds take about 9.2 s, so raw costs about half as before. The raw tables
are equal to the whole build.

## The scheduler

`--memory` (default 2 GB) is the memory of the tile builds at the same time. The tiles start largest file first, at most
`--jobs` at a time (default: the cores / `--tile-threads`), and a tile starts only when the estimates of the running
tiles and its own stay below the limit; a tile larger than the limit runs alone. The estimate of a tile is a line of its
file size: the command measures the peak of each finished tile process (`wait4`), fits base + factor × size to the
finished tiles (least squares), and adds the largest peak above the line. France (driving): 38 MB + 10.5 × size, 70 MB
above the line at most; Kentucky with `raw.*`: 73 MB + 22.7 × size. `<out>/tiles.log` has the file size and the peak of
each tile.

More memory does not make the build faster: the time is in the work of the tiles, not in waiting for memory.

On Linux, `duckosm-rust` sets the glibc memory thresholds to 128 KB (`mallopt`, `M_MMAP_THRESHOLD` and
`M_TRIM_THRESHOLD`): a freed block of 128 KB or more goes back to the system. Without this, each thread keeps the large
blocks that it freed, and a step uses much more memory than its live data. France (driving, `--raw`), same time:

| Step | glibc default | 128 KB |
|---|---|---|
| Cut | 1310 MB | 1081 MB |
| The largest tile process | 616 MB | 599 MB |
| Bus routes | 1032 MB | 870 MB |
| Levels | 1597 MB | 1071 MB |
| Total time | 615 s | 575 s |

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

The zoom changes the time a little (the work is the same, only cut into other pieces) and the memory much.

The zoom again, with the cut with two reads (PRs 196 and 197; France, driving, GeoParquet with the levels, 20 cores,
`--memory 2G`, 2026-11-15):

| Zoom | Tiles | Cut | Tile builds | The steps after the union | Total | Largest tile |
|---|---|---|---|---|---|---|
| 10 | 1,311 | 78.7 s, 1,285 MB | 133.2 s | 87.6 s | 299.5 s | 461 MB |
| 9 | 376 | 80.6 s, 1,233 MB | 145.4 s | 85.4 s | 311.5 s | 695 MB |
| 8 | 104 | 73.4 s, 1,110 MB | 208.7 s | 87.2 s | 369.3 s | 902 MB |

In the browser, quack by tiles is fastest at zoom 8 (Sweden in Chromium on the Mac, 12 threads: 37.0 s; zoom 10: 72.7 s).
**Zoom 8 is the default and the one tile zoom of all parts** (Kaveh, 2026-11-15): `duckosm-rust tiles`,
`duckosm-rust quack-tiles` and the quack page. At zoom 8 the build by tiles of France takes 23 % more time than at zoom
10, all in the tile builds: the 104 large tiles fit fewer at a time in the memory, and the largest finish last. The
largest tile (902 MB) stays in the budget of 2 GB.

The tiles at the same time and the threads of each tile (France, 12 cores, before the writers of own rows):

| Tiles × threads | Total | Tile builds together |
|---|---|---|
| 4 × 3 | 262.5 s | 1.04 GB |
| 6 × 2 | 244.3 s | 1.06 GB |
| 12 × 1 | 251.8 s | 1.37 GB |

The default is the cores / 2 tiles at a time, with 2 threads each.

## The `.duckdb` file

The tile builds write only GeoParquet (the command takes `--format` away from their options). After the union, `tiles` loads
the tables of the union into `{name}.duckdb`, as the whole build loads its files (`union::write_duckdb`,
`duckosm_duckdb::write_database`), with `raw.*` when the union has the raw files (`write_raw_from_parquet`). The default is both
formats, as in the whole build; with `--format duckdb` only, the Parquet files of the union go after the load. A whole build with
a boundary and `--raw` now also has `raw.*` in its `.duckdb` file.

The `.duckdb` file is written by a process of its own (`--only-duckdb`), so it starts with empty memory. DuckDB's
`memory_limit` is half the budget (`--memory`), with no row order and 4 threads: DuckDB takes about 1 GB more than its limit.
France, the tables of the union with `raw.*` (27 GB of files):

| DuckDB settings | Time | Peak memory |
|---|---|---|
| no limit | 81 s | 15.5 GB (the main process, with the union before) |
| limit 2 GB | 160 s | 5.2 GB |
| limit 2 GB, no row order | 78 s | 3.6 GB |
| limit 2 GB, no row order, 4 threads | 93 s | 3.0 GB |
| limit 1 GB, no row order, 8 threads | 105 s | 2.2 GB |
| **limit 1 GB, no row order, 4 threads** (the build, `--only-duckdb`) | **76 s** | **1.8 GB** |

Kentucky, driving, `--format duckdb --raw`: the 23 tables of the two `.duckdb` files are equal (but the two with the time of the
build), `raw.*` too (22.4 million rows). The load takes 10.7 s.

## In the browser (duckOSM Studio, 2026-11-17)

Studio runs the same functions in the browser: `union::cut_tiles`, `build` with `--tile` for each tile (a Web Worker of its
own), and `union::union_all`. The browser has no folders, so their files go through `duckosm_core::fs`: natively they are
the files of `std::fs` (no change), and a page sets its own files for the calling thread (`fs::set`). Three parts for the
browser:

- `fs::Blocks`: the temporary files of a step, named or not, are blocks of 1 MiB in one open file of the page. A file of
  OPFS opens only asynchronously, and the steps are synchronous, so the page opens its files before a step; with the
  blocks, the number of temporary files has no limit (France: 402 part files of the bus step). Only the files of the
  result are files of their own.
- `merge::Concat`: the join of Parquet files with the offsets in a u64. The writer of the parquet crate counts the bytes
  of a file in a usize, 32 bits in WebAssembly: France's `edges_driving` (more than 4 GiB with snap) got a wrong footer.
  The bus step adds the parts of each batch to its two tables at once.
- `union::cut_tiles` in WebAssembly gives the tile files to a sink (pbfcut `cut_to_sink_build`) with no zlib.

France in the browser (Mac mini, driving): 296 s, 5.7 GB of WebAssembly memory in all, 16.2 GB of browser storage at most.
The tile build against one build (the three modes, `--levels solve`, zoom 8, Mac mini; the tables are equal):

| Area | Native one build | Native by tiles |
|---|---|---|
| Monaco | 0.18 s | 0.27 s |
| Tartu | 0.91 s | 1.41 s |
| Kentucky | 35.7 s, 3.2 GB | 31.7 s, 1.3 GB |

## After the switch to pbfcut (2026-11-17)

The cut is the pbfcut library since duckOSM PRs 202 to 204 (pbfcut `8e565be`). Checks, natively on WSL (20 cores):

- **Kentucky by tiles:** the tables before the switch (duckOSM `7e9b423`) and after it (`e6425bc`) are equal row by row,
  in 18 of 20 files; in the other two, only the time of the build differs (`created`, `built_at`).
- **France at zoom 8 by tiles (driving):**

| | Before the switch | After the switch |
|---|---|---|
| The cut | 73.2 s, peak 900 MB | 80.7 s, peak 968 MB |
| 104 tile builds | 130.0 s | 131.6 s |
| The largest tile process | 698 MB | 677 MB |
| Total | 302.0 s | 301.5 s |

- **The cut alone** (`tile_cut` example, France, zoom 8, with the junctions files): 75.4 to 76.5 s before the switch,
  76.4 to 80.8 s after it, and **72.2 s** with pbfcut `8e565be`. Step 3d was the difference: pbfcut kept every way of the
  blocks that it read for the member ways (France: 9,193 blocks, about 73 million ways, for about one million); now a
  block gives only the needed ways, with no tags (pbfcut PR 46). Step 3d: 4.3 s, now 2.0 s.
- The times of one France run vary by about 5 % from run to run: compare two runs of each, one after the other.
- `pbfcut --steps` counts each allocation and makes the threads slower: its step times are not the times of the cut.

**Quack by tiles in the browser (Studio's quack page, zoom 8, not sorted, France `france-261113.osm.pbf`):**

| | Headless Chromium on WSL | Edge on a Windows laptop (8 cores) |
|---|---|---|
| 1. Count the nodes, choose the tiles | 15.1 s | 14.8 s |
| 2. The cut | 129.7 s | 163.6 s (45.3 s of it waiting for the file or the storage) |
| 3. The rows of each tile (8 workers) | 118.0 s | 165.0 s (7.9 of 8 workers busy) |
| 4. The join | 47.5 s | 35.2 s |
| Total | 310.4 s | 378.6 s |
| WebAssembly memory | 1,130 MiB + 4,025 MiB (workers) | 1,132 MiB + 4,034 MiB (workers) |
| Browser storage, at most | 34,020 MB | 34,143 MB |

The output is 12.3 GB of GeoParquet. Each tile worker is a WebAssembly module with its own 4 GiB; the largest France tile
(Paris, 8/129/88) used 587 MiB, so the limit is the memory of the whole tab, not of one module. At zoom 6 the same run
stopped with "Array buffer allocation failed" at about 5 GB in all modules together. One tile with 2 threads is 1.55 times
faster than with 1 thread (Paris, natively: 19.8 s, 12.7 s with 2, 10.3 s with 4, 9.6 s with 8) with the same memory.

Not yet by tiles: the bus routes and the drawing levels (France: about 30 s and 50 s, peak about 0.7 and 0.9 GB) run on
the whole area after the union, so their memory grows with the area.
