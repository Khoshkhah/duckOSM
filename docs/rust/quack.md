# The features table of QuackOSM (`duckosm quack`)

Status: done, 2026-11-13. Kaveh asked for a duckOSM output that is the same as the output of QuackOSM, so that the two tools
can be compared on the same rows. QuackOSM (kraina-ai) is a Python library that converts an `.osm.pbf` file to one table of
features in GeoParquet or DuckDB.

## The command

```bash
duckosm quack kentucky.osm.pbf -o out --name kentucky -f duckdb,geoparquet
```

- `-f geoparquet` writes `out/kentucky_quack.parquet` (GeoParquet 1.0, WKB, zstd level 3, row groups of 100 000 rows, as
  QuackOSM writes it).
- `-f duckdb` writes the table `main.quackosm` (the table name of QuackOSM) into `out/kentucky_quack.duckdb`. The geometry
  column has DuckDB's `GEOMETRY` type.
- `-f duckdb,geoparquet` writes the two files in one pass.
- `--threads` sets the threads, as QuackOSM's `cpu_limit`: the pool of the Rust threads, the blocks that are decoded at
  the same time, and the threads of DuckDB. The default is all the cores.
- `--no-sort` keeps the rows in the order in which they are made (QuackOSM `sort_result=False`). By default the rows are
  sorted as QuackOSM sorts them (see "The sort").
- `--tiles N` makes the table tile by tile, with at most `N` nodes in a tile (for example `4000000`), so the memory follows
  the size of a tile, not of the file (see "By tiles"). It writes GeoParquet only.
- `--tile-zoom Z` makes the table tile by tile with the map tiles of zoom `Z` (0 to 14), in place of `--tiles`.

`duckosm quack-tiles FILE --zoom 9 -o DIR` writes one table for each map tile (see "Tile tables"), and
`duckosm quack-union DIR -o FILE.parquet` joins tile tables into one table.

The input must be sorted by type and id (nodes, then ways, then relations), as the files of Geofabrik are. To make the table of
an area, cut the region file first with `duckosm cut`.

The table has three columns, as in QuackOSM: `feature_id` (`node/<id>`, `way/<id>` or `relation/<id>`), `tags`
(`MAP(VARCHAR, VARCHAR)`) and `geometry`.

## The rules

The rules are the ones of QuackOSM 0.20.0 with its default options (no filters, `ignore_metadata_tags`, compact tags), from its
`pbf_file_reader.py`:

- A node with tags is a POINT.
- A way with tags, with 2 or more nodes and all its nodes in the file, is a LINESTRING. It is a POLYGON when it is closed, has
  4 or more points after the repeated points are removed, and its tags match the polygon rules of QuackOSM
  (`osm_way_polygon_features.json`: `area=yes`, 32 keys, 6 allow lists and 13 deny lists; `area=no` is always a line).
- A relation of the type `multipolygon` or `boundary` with all its member ways in the file is a POLYGON or a MULTIPOLYGON:
  1. The member lines of each role are joined as GEOS `LineMerge` joins them: only at the points where exactly two line ends
     meet.
  2. If one ring is not closed, the relation is not in the table.
  3. An empty role is `outer` (QuackOSM: `COALESCE(ref_role, 'outer')`; `ST_ReadOSM` gives NULL for an empty role). If no
     member has the role `outer`, all rings are outer rings.
  4. Each outer polygon loses the inner polygons that are in it. The union of the outer polygons is the geometry.
- The metadata tags (`area`, `created_by`, `converted_by`, `source`, `time`, `ele`, `note`, `todo`, `fixme`, `FIXME` and the keys
  that start with `openGeoDB:`) are not in the tags. An element with no other tag is not in the table. The polygon rules read
  the tags before this step.
- The coordinates are rounded to 7 decimals, as QuackOSM rounds them.

## How duckOSM makes it

All of it is Rust: `duckosm_core::quack` (the rows), `duckosm_parquet::quack` (the Arrow batches and the GeoParquet
file) and `duckosm_duckdb::quack` (the `.duckdb` table). There is no Python and no DuckDB spatial extension. The same
code builds for WebAssembly.

`features()` reads the file twice and puts each table into temporary files (`spill`, OPFS in the browser) as soon as a
pass makes it; a pass reads only what it needs. Each pass writes its time to the error output (`util::step`).

1. **Pass 1, the ways and the relations.** A way with tags gets a number; its header (id, tags, number of nodes) goes to
   a file, and its nodes go into buckets by node id (`spill::Buckets`, 2^27 ids each, France about 100) as requests
   (node, way, place). A way with no tags goes to a file of its own: only a member of a relation needs it, and the
   relations come after the ways. Each relation goes to a file of headers, and each member way to a file of (way,
   relation, place). After the pass, the member ways with no tags get numbers and requests too.
2. **Pass 2, the nodes.** The nodes come in the order of their ids. The buckets of requests come in the same order
   (`spill::SortedParts`: each bucket read into one buffer of the caller and sorted in memory on all threads), and each
   request gets its point. The points go into buckets by way number (about 64). A node with tags is a row.
3. **Pass 3, the ways.** A reader thread reads the buckets of points, each sorted by way and place, with the headers;
   this thread makes the rows of each batch of 50 000 ways on all threads. The line of a member way goes to a file.
4. **Pass 4, the relations.** The member file sorted by way meets the member lines sorted by way; the lines, sorted by
   relation and place, meet the relation headers. The relations with all their lines go to all threads one at a time
   (`par_bridge`), and each row goes to the writer. A relation that takes more than 2 s is written to the error output.

The polygon operations use the pure-Rust crates `geo` 0.33 and `i_overlay` 4.5:

- **The ring check.** A ring is valid when no two of its segments meet other than at the point that they share. A sweep
  (`geo::algorithm::sweep::Intersections`) finds the pairs that meet. `geo`'s `Validation` compares each segment with
  each other segment (a boundary ring of 50 000 points took seconds).
- **The repair of an invalid ring.** i_overlay with the even-odd fill rule (the area of GEOS's `ST_MakeValid`).
- **The union and the difference.** i_overlay, called directly with `preserve_input_collinear` and
  `preserve_output_collinear` (GEOS keeps a point on a straight edge). The rings are oriented first, and the result goes
  back on the 7-decimal grid.
- **Do polygons meet?** `polygons_meet`: one sweep over the edges of all the polygons, and a point-in-polygon test for
  the pairs whose boxes meet. `geo`'s `Intersects` of two polygons compares each segment with each segment: one relation
  of France (16467322, 6 811 members) took 100 s. When no polygons meet, the union is the polygons as they are, and the
  holes go into the outer polygon with no overlay, so the coordinates stay exact.

## Large areas: what France showed

The first design held all node points in memory: about 18 GB for France, more than the Mac mini's 16 GB. The steps to
the design above, each measured on France (unsorted GeoParquet):

| Design | Time | Peak RSS | Peak disk use |
|---|---|---|---|
| Spill join of the network step (`join::Merge`, two external sorts) | 405 s | 5.6 GB | 35.0 GB |
| Each table to a file (member lines and relations too) | 501 s | 2.8 GB | 36.1 GB |
| Buckets in place of the external sorts | 233 s | 5.5 GB | 23.1 GB |
| The sweep in place of `geo`'s `Intersects` | 128 s | 6.2 GB | 23.2 GB |
| One reused buffer for the buckets; no copy of the Arrow pieces | 129 s | 3.8 GB | 23.2 GB |

- The two external sorts took 174 s of 300: `spill::sort_bytes` sorts each run on all threads, but it reads, writes and
  merges on one thread. The buckets write each row once and read it once, and sort each bucket in memory on all threads.
- The relations in batches waited for the slowest relation of each batch (203 s); `par_bridge` lets a slow relation delay
  only itself.
- A worker that waits on a full channel can block the writer, which uses the same thread pool for its Arrow batches
  (Ile-de-France stopped). A full channel now makes the worker run other waiting work (`rayon::yield_now`).
- The Rust heap never passed 2.3 GB (cargo feature `heap-probe`), but the macOS footprint was 11 GB: `vmmap` showed
  "Malloc Large (empty)", freed large blocks that the system allocator keeps for later use. A new vector for each bucket
  (each a new size) and a copy of each Arrow batch filled that cache. With `MallocLargeCache=0` the footprint was 1.4 GB.
  `SortedParts` now fills one buffer of the caller, and the Arrow pieces go to the writer with no copy: 3.5 GB.
- A run that is stopped (a signal) left its temporary files: 45 GB after some stopped runs. Now, on macOS and Linux, a
  temporary file has no name while it is open (`spill::DirStore`), so the system frees it also when the process stops.

## By tiles

The browser has at most 4 GiB of WebAssembly memory, and this memory never shrinks. In one run, Kentucky (141 MB) needs
1.9 GiB, and Île-de-France (338 MB) stops with no memory left in pass 3. Parts of one run grow with the area: the node
buckets cover a fixed range of ids, the sorts have a budget of 512 MB (128 MB in the browser), and the browser collects
the relations in memory. `features_by_tiles` (`duckosm quack --tiles N`, `duckosm_build::quack_tiles_to`) makes the
same rows tile by tile:

1. **The tiles.** A pass over the nodes counts the nodes of each map tile of zoom 14. A tile with more than `N` nodes is
   cut into its four tiles of the next zoom (a quadtree, down to zoom 14). So a city has small tiles and the country
   around it large ones: France at 4 million nodes has 324 tiles of zoom 3 to 11, Kentucky 16 tiles of zoom 6 to 9.
   With `--tile-zoom Z`, the tiles are the map tiles of zoom `Z` with a node (`TileSize::Zoom`).
2. **The cut.** The tile cut of the build by tiles (`tilecut::cut_to_store`) writes the file of each tile, with the ways
   whole (with their nodes from other tiles), and each multipolygon and boundary relation whole (all its member ways and
   their nodes) in each tile with a member, as the cut of the build: each tile file is a complete, standard `.osm.pbf`
   (Kaveh's rule). With one zoom for all of France (zoom 11, 5 883 tiles), the copies of the large boundaries filled 32 GB
   of temporary files; the quadtree has far fewer tiles. Natively the tile files are files in a temporary folder; in the
   browser, chunks of one temporary file of OPFS, because the browser has no folders. The cut is the pbfcut library
   (`docs/design/build_by_tiles.md`).
3. **The rows of each tile.** `features_owned` runs on the file of each tile and keeps only the rows that the tile owns
   (`quack::Owner`), so each row comes one time:
   - a node: the tile of its place;
   - a way: the tile of the place of its first node. The cut gives a way to the tile of each of its nodes, and the cut and
     quack decode the same coordinates, so that tile has the way;
   - a relation: the first tile that makes it. A set of the relation ids that the tiles made stays in memory (France:
     286 000 ids).

   A row that a tile makes is the row of the whole file: the ways are whole, and a relation is a row only when the tile
   has all its member ways.
4. **The sort**, as in one run (see "The sort"); its strip has a bounded memory too.

Tile by tile gives the same rows as one run: on Monaco (a test, tiles of at most 2 000 nodes), Kentucky (2 393 139 rows)
and France (99 412 535 rows), `quack_diff --large` finds the same ids, the same tags and the same normalized geometry for
each row. The runs of the table below were on the same day, one after the other.

| Area (unsorted GeoParquet, Mac mini) | Mode | Time | Peak RSS | Peak disk use |
|---|---|---|---|---|
| Kentucky (141 MB) | one run | 3.0 s | 1.0 GB | 0.3 GB |
| Kentucky | 16 tiles | 7.7 s | 0.7 GB | 0.4 GB |
| France (5.1 GB) | one run | 105.9 s | 4.6 GB | 23.1 GB |
| France | 324 tiles | 223.5 s | 2.0 GB | 19.9 GB |
| France | 324 tiles, no zlib in the tile files, 4 tiles at a time | 171.3 s | 3.7 GB | 24.9 GB |
| France | zoom 8 (241 tiles) | 200.4 s | 2.2 GB | 19.4 GB |
| France | zoom 9 (697 tiles) | 222.5 s | 1.9 GB | 19.7 GB |
| Île-de-France (338 MB) | zoom 8 (13 tiles) | 20.4 s | 1.1 GB | 2.0 GB |
| Île-de-France | zoom 9 (32 tiles) | 18.3 s | 1.0 GB | 1.7 GB |

In the browser (duckOSM Studio `quack.html`, Chromium on the Mac mini, 12 threads), Île-de-France (338 MB) in one run stops
with no memory left (more than 4 GiB). Tile by tile (28 tiles of zoom 1 to 11), it gives the same 8 833 620 rows as the
native run (`quack_diff --large`):

| Île-de-France in Chromium | Time | Peak WebAssembly memory |
|---|---|---|
| 28 tiles, not sorted | 41.4 s | 429 MiB |
| 28 tiles, sorted | 45.6 s | 586 MiB |
| zoom 8 (13 tiles), not sorted | 42.5 s | 474 MiB |
| zoom 8, sorted | 47.0 s | 746 MiB |
| zoom 9 (32 tiles), not sorted | 41.7 s | 467 MiB |
| zoom 9, sorted | 45.9 s | 695 MiB |

France (5.1 GB) in Chromium, 324 tiles of at most 4 million nodes, not sorted: 582.3 s (the cut 267.7 s, the tiles
306.6 s) and 1 004 MiB of WebAssembly memory, with the same numbers of rows as the native run. The GeoParquet file is
12.2 GB: the browser compresses it with Snappy, the native run with zstd (9.7 GB). France sorted: see "The sort tile by
tile" below.

Two faults of the browser came out on the way. In the browser, pass 4 collected all relations of the file in memory
before it made them: now it makes them in batches of 4 million points. And the allocator of Studio's WebAssembly module
kept each freed small block with the thread that freed it, so memory grew from tile to tile up to 4 GiB; Studio's
allocator now gives the extra free blocks to a shared pool.

Two changes made the tile mode faster natively (France, one run measured again the same day: 104.3 s): the temporary
tile files have no zlib (the cut 88 s to 71 s, 30 % less CPU), and 4 tiles run at the same time, with their rows given in
the order of the tiles (the tiles 136 s to 100 s; the rows of 4 tiles wait in memory).

In the browser, a file in OPFS belongs to the thread that opened it. Thus Studio does the tile mode in four steps, and
each step uses all the threads:

1. The count of the nodes chooses the tiles.
2. The cut (`tilecut::cut_to_sink`, the pbfcut library) writes one complete `.osm.pbf` file for each tile. The thread
   that owns the files only writes; the threads of the pool decode the blocks and pack the blocks of the tile files.
3. A Web Worker for each thread makes the rows of one tile at a time (`quack_tile`). Each worker has its own
   WebAssembly module, its own memory and its own files, so the tiles do not wait for each other. A tile file is
   removed when its rows are made. With the sort, each worker also sorts the rows of its tile (see "The sort tile by
   tile").
4. The join writes the parts into one GeoParquet file. Without the sort, it copies the row groups of the parts. With the
   sort, it merges the sorted parts.

Sweden (975 MB) in Chromium on the Mac mini, zoom 6 (21 tiles), 12 threads, not sorted: 60.9 s, with the same numbers of
rows as the native run:

| Step | Time | CPU |
|---|---|---|
| 1. Count the nodes and choose the tiles | 2.0 s | |
| 2. The cut | 29.2 s | 49.5 s before the jobs in a tile file; part 5 (the tile files) on 11 to 12 cores |
| 3. The rows of each tile | 25.0 s | 12 cores, then the largest tile alone for about 10 s |
| 4. The join | 4.7 s | |

The run used at most 7.0 GB of browser storage: the tile files and the parts of the rows, then the parts and the 2.7 GB
output file.

### The sort tile by tile

QuackOSM's sort (`str_order`) cuts the rows, in the order of x, into strips, and sorts each strip by y. Tile by
tile, the order of x of all rows is the merge of the order of x of each tile:

1. Each tile worker sorts its own rows by (x, y, row number) of the center (`PartSort`: in memory while the rows fit in
   the budget of the sort, else `StrSort` with runs on disk) and writes its part with row groups of 2 048 rows.
2. The join (`quack_join_to`) merges the parts with a heap, by (x, y, part). This is the order (x, y, row number) of all
   rows, because the row numbers of a part come after those of the parts before it. The merge holds one row group of each
   part; when a part needs more, it reads one row group for each thread and decodes them on all threads.
3. The strips of `StrSort` come from the merged rows, as before. The join writes no runs, and it sends its progress
   (rows merged of all rows) to the page.

Before, the join pushed all rows into `StrSort` (runs of 128 MB to OPFS, then a merge) on one worker, and the page showed
no progress. In Chromium on the Mac mini (zoom 8, 8 tile workers, join with 8 threads), sorted, before and after:

| Sorted, in Chromium | Tiles | The join | Join worker memory | Total |
|---|---|---|---|---|
| Kentucky (148 MB), before | 3.7 s | 2.9 s | 815 MiB | 9.1 s |
| Kentucky, after | 4.5 s | 1.9 s | 827 MiB | 8.9 s |
| Île-de-France (355 MB), before | 14.1 s | 8.9 s | 965 MiB | 29.2 s |
| Île-de-France, after | 17.7 s | 5.8 s | 626 MiB | 29.7 s |
| Sweden (975 MB), 12 workers, before | 13.6 s | 26.3 s | 2 240 MiB | 58.9 s |
| Sweden, after | 14.6 s | 12.2 s | 2 182 MiB | 45.8 s |
| France (5.1 GB), 12 workers, after | 145.5 s | 114.9 s | 1 168 MiB | 340.2 s |

The join is faster, and the work of the sort moves to the tile workers. Sweden has 206 tiles, and on average 11.8 of the
12 workers are busy: the run is 13.1 s shorter, and it used at most 9.4 GB of browser storage, not 13.2 GB (no runs of
the sort in the join). Île-de-France has one large tile (Paris, 8/129/88):
its worker sorts alone for about 3.5 s more, and the other workers have no tiles then, so the total stays the same. A
file with many tiles of similar size divides this work between all workers. The memory of all tile workers on
Île-de-France went from 1 471 MiB to 2 459 MiB (the rows of a tile wait in memory for the sort, up to 128 MB).

The disk holds the output and only a few other files at the same time. The join drops each part when it has read all its
rows, and the page then deletes the file of the part. The sort writes the pieces of each strip over the pieces of the
strip before. Before these two changes, Sweden used at most 9.4 GB of browser storage on the disk (the parts 2.7 GB, the
temporary file of the join 3.8 GB, the output 2.6 GB), and France stopped at 37.6 GB, so that the Mac kept 30 GB free.
After them, Sweden uses at most 3.3 GB, and France at most 17.9 GB for a GeoParquet file of 11.7 GB (241 tiles, 99 412 535
rows).

Natively (`examples/quack_join_bench.rs`, Île-de-France, zoom 8), the sorted join takes 2.6 s, against 6.9 s with all
rows into `StrSort`.

Natively, one run is faster: the tile mode reads the file four times (the count, two passes of the cut, and the blocks
of the extra elements), it writes and reads each element again, and the ways at the edge of a tile are made in each
tile. The memory of the cut (its index of the nodes: 205 MB of runs for France) and of the tiles stays small. Use the
tile mode where the memory has a limit: the browser.

## Tile tables

`quack --tiles` makes one table of one file. The tile tables are for a table of the world, made in parts: one table for
each map tile of one zoom, `DIR/<z>/<x>/<y>.parquet` (`duckosm_build::quack_tiles`).

1. **The cut** makes the file of each tile, as osmium's smart cut: the nodes in the tile, each way with a node in the
   tile (whole, with its nodes from other tiles), and each relation with a member in the tile; each multipolygon and
   boundary with all its members, in each tile with a member.
2. **Each tile alone:** quack on the file of the tile, and the table of the tile at once. Nothing waits for another tile,
   so the tiles can run in any order, at the same time, or on other computers. The table holds the nodes in the tile,
   the ways with a node in the tile and the relations that the file has whole (not the nodes and member ways that the
   cut added from other tiles). So the table of one tile is usable alone.
3. **The owner:** a way or a relation at the edge of a tile is in the table of each tile with a node or member of it. A
   column `owner` is true in one tile only, and each tile decides it alone: a node by its place, a way by the place of
   its first node, a relation by the place of the first node of its first member way. The cut and quack decode the same
   coordinates, so the owner tile always has the row.
4. **The union** keeps the rows with `owner`, so each row comes one time with no search for doubles:
   `duckosm quack-union DIR -o FILE.parquet`, or in DuckDB
   `SELECT feature_id, tags, geometry FROM read_parquet('DIR/9/*/*.parquet') WHERE owner`.

A polygon that covers a tile with no node or member in it (a country around the tile) is not in the table of that tile.
For the rows of an area, read the tiles that the area touches, keep the rows that intersect the area
(`ST_Intersects`), and remove the doubles with `DISTINCT ON (feature_id)`, not with `owner`: a road that comes into the
area can have its owner in a tile outside the list.

Each table has its tile, the input file and the time of the file in its metadata (`duckosm_tile`). The checks: on Monaco
(a test, zoom 14), the union equals the table of one run and each node is in the table of its tile only. On Kentucky
(zoom 9), the union equals one run (`quack_diff --large`), and against DuckDB's `ST_Intersects` with the box of each tile
(on QuackOSM's own table) no table has a row that does not touch its tile; 18 rows touch a tile with no node in it.

| Tile tables, zoom 9 (Mac mini) | Tables | Time | Peak RSS | Size | Rows in the tables | Union |
|---|---|---|---|---|---|---|
| Kentucky | 65 | 9.5 s | | 327 MB (one table: 321 MB) | 2 400 820 | 1.0 s |
| France | 697 | 232.6 s | 3.7 GB | 13 GB (one table: 9.7 GB) | 99 594 359 | 43.6 s |

France: the count 4.1 s, the cut 96.2 s, the tiles 132.3 s, and a peak disk use of 31.0 GB (the tile files of the cut have
no zlib, and each multipolygon and boundary is whole in each tile with a member). The union is byte for byte the table of
one run (fingerprints of all 99 412 535 rows).

`duckosm` raises its limit of open files to 10 240 at the start: 4 tiles at a time keep about 700 temporary files
open, and macOS gives a process 256.

## The sort

QuackOSM sorts the rows by default with Sort-Tile-Recursive packing (`rq_geo_toolkit`, `_str_index_select_sql`), and
`duckosm_core::quack::str_order` uses the same rule:

1. The key of a row is the center of the bounding box of its geometry.
2. The rows sorted by x (then y) are cut into `ceil(sqrt(ceil(rows / 100 000)))` vertical strips (Kentucky: 5).
3. Each strip is sorted by y: up in the even strips, down in the odd strips.

Each row group of 100 000 rows then holds the rows of a small area. A reader with a bounding box can skip the other row
groups, and the file is smaller. On Kentucky, 2 391 567 of the 2 393 139 rows (99.93 %) are at the same position as in
QuackOSM's file, all rows are in the same row group, and no row is more than 11 positions away (rows with equal centers
are in another order).

The sort needs all rows before it writes the first row. `duckosm_core::quack::StrSort` keeps them on disk:

1. The rows go as bytes into one buffer. When the buffer holds 512 MB, the rows are sorted by (x, y, row number) on all
   threads and written as one run: blocks of 2 048 rows, each compressed with zstd level 1 on all threads, at the end of
   one temporary file.
2. At the end, a merge of the runs gives the rows in the order of x. The merge keeps one block of each run in memory.
3. The rows go into the bytes of the current strip. When the strip is full, it is sorted by y, and its rows are decoded
   on all threads, one row group at a time. A strip larger than the budget goes to the end of the temporary file in
   sorted pieces of the budget, and a merge of the pieces by y gives its rows: so the memory of the sort does not grow
   with the area (the strip is `sqrt(rows * 100 000)` rows: 3.2 million on France).

The order is the order of `str_order` (a test compares them). On France, the 9.2 GB file needs 3.2 million rows (one
strip) in memory, and the disk holds the rows one time. The tries before this design, on France:

| Design | Time | Peak RSS | Peak disk use |
|---|---|---|---|
| All rows in memory as Arrow batches | not run: Kentucky needed 1.7 GB for a 0.3 GB file | | |
| One file for each 1/16 degree of x | not possible: more than 256 open files (the limit of macOS) | | |
| Runs with no compression | stopped at 138 s: less than 30 GB free | | 44.8 GB |
| Compressed runs in one file, a strip as decoded rows | 156.5 s | 6.4 GB | 23.5 GB |
| A strip as bytes, decoded a row group at a time | 150.2 s | 4.5 GB | 23.5 GB |

The writer (`ParWriter::set_groups_at_once`) encodes 4 row groups at the same time: the geometry column is most of a row
group, and one column is encoded on one thread.

## The results

The tables of Kentucky (all cases of both tools) and of France (GeoParquet sorted and not sorted, `.duckdb` not sorted), the row checks and the results that
are not in favor of duckOSM are on the page of the manual, `docs/concepts/quackosm.md`. The scripts that measured them ran
on the Mac mini: `/usr/bin/time -l` for the time, the CPU time and the macOS footprint, a sampler of the RSS of the
process tree and of the free disk every 2 s for the large runs, and `stat` for the file sizes.

## GDAL layers (`--gdal`)

Status: done, 2026-11-17. `duckosm quack --gdal` writes the five layers of GDAL's OSM driver, as `ogr2ogr` and QGIS
give them for an `.osm.pbf` file with GDAL's default `osmconf.ini` (GDAL 3.13): `points`, `lines`, `multilinestrings`,
`multipolygons` and `other_relations`.

```bash
duckosm quack kentucky.osm.pbf --gdal -o out --name kentucky                  # out/kentucky_gdal.gpkg
duckosm quack kentucky.osm.pbf --gdal -o out --name kentucky -f geoparquet    # out/kentucky_gdal_<layer>.parquet
duckosm quack france.osm.pbf --gdal -o out --name france --tile-zoom 8         # tile by tile
```

The engine is the engine of quack. It has three parts: read, decide and write. `--gdal` changes only "decide" and
"write":

- **Read** (unchanged): the four passes of "How duckOSM makes it". With `--gdal`, pass 1 also keeps each relation (not
  only the multipolygons) with its member nodes, and pass 2 also gives the points of the member nodes.
- **Decide** (`duckosm-core/src/gdal.rs`, the rules of GDAL's `ogrosmdatasource.cpp` and `ogrosmlayer.cpp`):
  - `points`: a node with a tag that is not `created_by`, `converted_by`, `source`, `time`, `ele` or `attribution`.
  - `lines`: a way with a tag that is not an area. The line has the nodes that the file has (2 or more).
  - An area: a closed way (its first node is its last node) with `area=yes`, or with a key or a key and value of
    `closed_ways_are_polygons`, and not `area=no`. Its polygon is a row of `multipolygons` with `osm_way_id`, except when
    it is an outer member of a complete multipolygon or boundary. These polygons wait in a temporary file until the
    relations are done.
  - A relation of the type `multipolygon` or `boundary` with all its member ways (not the role `subarea`): the closed
    member ways are rings, the other member ways are joined into rings (`OGRBuildPolygonFromEdges`), and the holes go into
    their polygons (`organizePolygons`, the fast test of one point). A relation with no tag other than `type` and
    `created_by` takes the tags of its first outer member area.
  - `multilinestrings`: a relation of the type `multilinestring` or `route`, its member ways.
    `other_relations`: each other relation, its member nodes as points and its member ways as lines (an area as a
    polygon).
  - The columns: each tag that is a column of the layer goes into the column. The other tags go into `other_tags`
    (`"key"=>"value"`, with `\` before `"` and `\`), except the tags that the layer ignores. `lines` has the `z_order` of
    `osmconf.ini`.
- **Write** (`duckosm-build/src/quack_gdal.rs`, on a thread of its own, so the engine does not wait): a GeoPackage with the
  tables of GDAL's GeoPackage driver (a table, an R-tree index and its triggers for each layer, `gpkg_ogr_contents`), or one
  GeoParquet file for each layer. The rows are encoded on all threads. The R-tree is written in one go at the end (the
  boxes in the order of a Hilbert curve, full nodes, straight into SQLite's shadow tables), as GDAL does: an insert into
  the R-tree for each row took 10 s for each million rows.

By tiles, the cut completes only the multipolygons and boundaries in each tile (as for quack). A relation of
`multilinestrings` or `other_relations` can have its members in many tiles: each tile gives the members that it owns
(`Feature::part`), and `gdal::Parts` joins the members of each relation, in their order, into one row.

In the browser (duckOSM Studio's quack page, output **GDAL layers**), each tile runs alone in a worker of its own, with no
set of the relations that other tiles made. `quack_gdal_tile_to` writes the rows that the tile owns (`gdal::put_row`): the
tile of the first node of the first member way owns a multipolygon, as in the tile tables. `quack_gdal_join_to` reads the
files of all tiles, joins the parts of the relations, and writes the five layers.

The check: `duckosm-build/tests/quack_gdal_monaco.rs` compares the output with `tests/data/monaco_gdal.jsonl.gz`, the
rows of `ogr2ogr -f GPKG out.gpkg tests/data/monaco.osm.pbf` (GDAL 3.13.3), made before the code: the same rows in each
layer, the same ids and column values, the same geometries (in 1e-7 degrees). The whole file and the file by tiles of
zoom 12 and each tile alone with the join (the browser's way) are equal. The GeoPackage passes GDAL's `validate_gpkg.py`. Kentucky (2 240 511 rows) is equal to `ogr2ogr` on
Linux x86-64; against GDAL 3.13.3 of Homebrew on the Mac mini (arm64), one multipolygon (relation 13011728) is different:
a ring that touches another ring at a vertex is a hole there and a polygon here. GDAL's test of a point on a ring
(`dx1 * dy2 - dx2 * dy1 == 0`) gives another result on that build; the x86-64 build agrees with duckOSM.

The memory (Kentucky, Mac mini, peak footprint): 1.1 GB at first, 0.25 GB now (`ogr2ogr`: 0.44 GB). Three causes:

- The rows that wait for the writer: a channel of 8 batches of up to 50 000 rows held 200 MB. Now one batch waits; the
  engine waits for the writer.
- The buffers of the temporary files of `spill::Buckets`: about 100 files of requests on Kentucky, with 1 MB each. Now
  256 KB each.
- macOS keeps the memory that a program frees: the heap was at most 0.37 GB, the footprint 0.89 GB. The memory of macOS
  is space-efficient when `MallocSpaceEfficient` is set at the start of the program, so `duckosm quack --gdal` starts
  again with it on macOS (0.25 GB; the time goes from 3.9 s to 4.3 s). Linux returns the memory with the `mallopt` of
  `main`.

By tiles, France had a footprint of 5.0 GB at first, and a later run stopped with no error. Two causes:

- Natively, the tiles ran in groups of four, and each tile kept all its rows until the group was done: the four dense tiles
  of Paris held 8.8 GB of rows. With `--gdal` (the order of the rows does not matter), each tile sends its rows at once
  through a small channel, and a pool of four threads takes the next tile when a tile is done.
- The writer encoded on the pool of the engine. A thread that waited for a job of the writer took a job of the relations;
  that job waited for the channel of the relations, which waited for the writer. Now the writer has a pool of its own (a
  quarter of the cores, at least 2). `duckosm-build/tests/quack_gdal_full.rs` runs Monaco with two threads, a channel of
  one row for the relations (`quack::REL_CHANNEL`) and tiles of zoom 14.

France by tiles now: 3 min 37 s and 0.9 GB (Mac mini).

Not done: GDAL's own `osmconf.ini` (only the default one), the open options of the driver, and GDAL's repair of a
self-touching ring with GEOS in `organizePolygons` (Monaco has none).

## Geofabrik layers (`--geofabrik`)

Status: done, 2026-11-18. `duckosm quack --geofabrik` writes the 20 layers of Geofabrik's free files
(`<area>-latest-free.shp.zip` and `-free.gpkg.zip`) from any `.osm.pbf` file. The user page is
[duckOSM and Geofabrik](../exports/geofabrik.md).

```bash
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco                         # out/monaco_geofabrik.gpkg
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco -f shp                  # out/monaco_geofabrik.shp.zip
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco -f gpkg,shp,geoparquet  # all three in one read
```

The engine is the engine of `--gdal` in the mode `quack::Mode::Geofabrik`. The mode has two differences from GDAL: a
multipolygon with no tags of its own does not take the tags of its outer member, and the polygon of an outer member way
is a row also when a complete multipolygon uses it. `duckosm_core::geofabrik::rows` then makes the Geofabrik rows of each
GDAL row:

| GDAL row | Geofabrik layers |
|---|---|
| a point (a node) | the point layers: `places`, `pois`, `pofw`, `natural`, `traffic`, `transport` |
| a line (a way) | the line layers `roads`, `railways`, `waterways`; a closed line with no `area=no` also the area layers |
| a polygon of a way | the area layers (`_a`); its ring also the line layers |
| a polygon of a relation (complete, all rings closed) | the area layers |

The rules are in `duckosm-core/src/geofabrik_rules.txt`, one line for each rule: `<class> <code> <fclass> <conditions>`.
Each key of the rules gives one row: a cafe with `shop=bakery` is a `bakery` row and a `cafe` row in `pois`, as in
Geofabrik's files. The file lists where Geofabrik's PDF and its files do not agree; the rules follow the files.

#### Where Geofabrik's PDF and its files do not agree

Geofabrik describes the layers in
[OpenStreetMap Data in Layered GIS Format](https://download.geofabrik.de/osm-data-in-gis-formats-free.pdf). Its files
are not always as the PDF says. duckOSM follows the files:

| The PDF | The files |
|---|---|
| parking: `parking_surface`, `parking_multistorey`, ..., not `access=private` | only `parking`, also with `access=private` |
| `crossing`, `market_place`, `art`, `taxi_rank` | `pedestrian_crossing`, `marketplace`, `artwork`, `taxi` |
| no `fitness_centre`, `sports_hall`, `track` in `pois` | these three are in `pois` |
| `tree`, `street_lamp`, `mobile_phone_shop`, `car_dealership` are not in the free files | they are in the free files |
| `scrub` is `landuse=scrub` | `natural=scrub` |
| guide posts and maps are `tourist_guidepost`, `tourist_map` | `tourist_info` |
| bus stops include `public_transport=stop_position` | only `highway=bus_stop` |
| one feature class for each object | one row for each key: a cafe with `shop=bakery` is a `bakery` row and a `cafe` row |
| `railways` has no `name` | it has `name` |
| no `places_a` layer | `places_a` is in the files |

A class that Monaco does not have follows Geofabrik's 0.8 standard, without the classes that the PDF calls "not in the
free files". Not checked: names such as "fixme" (the PDF says that they become empty), `maxspeed` in mph, and the oneway
of motorways.

The writers are those of `--gdal` with a `gdal::Schema` (the tables, the geometry types, the columns and their widths),
and one more: `duckosm-build/src/shp.rs` writes a shapefile for each layer (crates `shapefile` and `dbase`, pure Rust)
into one zip file (crate `zip`). The files of a layer grow in temporary files, and a complete layer goes into the zip file.
A layer of more than 2 GB continues in `_2`, `_3` (Geofabrik's spillover).

`duckosm-build/tests/quack_geofabrik_monaco.rs` compares the output with Geofabrik's GeoPackage of the same day
(`tests/data/monaco_geofabrik.jsonl.gz`, made by `tools/geofabrik_golden.py` from Geofabrik's file): the whole file, by
tiles of zoom 12, each tile alone with the join (the browser), and the shapefiles.

## What QuackOSM has and duckOSM does not have

The filters: tags (`tags_filter`), OSM ids, custom SQL, and a geometry (`geometry_filter`, features that touch it). The
automatic download of an extract for a geometry, the geocoding, more than one PBF in one call, the tags as columns
(`explode_tags`), the Hilbert sort (`sort_algorithm="hilbert"`) and the WKT output. `duckosm quack` makes the table of
QuackOSM's default options, with or without the sort.
