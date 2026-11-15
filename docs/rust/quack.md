# The features table of QuackOSM (`duckosm-rust quack`)

Status: done, 2026-11-13. Kaveh asked for a duckOSM output that is the same as the output of QuackOSM, so that the two tools
can be compared on the same rows. QuackOSM (kraina-ai) is a Python library that converts an `.osm.pbf` file to one table of
features in GeoParquet or DuckDB.

## The command

```bash
duckosm-rust quack kentucky.osm.pbf -o out --name kentucky -f duckdb,geoparquet
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

`duckosm-rust quack-tiles FILE --zoom 9 -o DIR` writes one table for each map tile (see "Tile tables"), and
`duckosm-rust quack-union DIR -o FILE.parquet` joins tile tables into one table.

The input must be sorted by type and id (nodes, then ways, then relations), as the files of Geofabrik are. To make the table of
an area, cut the region file first with `duckosm-rust cut`.

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
the relations in memory. `features_by_tiles` (`duckosm-rust quack --tiles N`, `duckosm_build::quack_tiles_to`) makes the
same rows tile by tile:

1. **The tiles.** A pass over the nodes counts the nodes of each map tile of zoom 14. A tile with more than `N` nodes is
   cut into its four tiles of the next zoom (a quadtree, down to zoom 14). So a city has small tiles and the country
   around it large ones: France at 4 million nodes has 324 tiles of zoom 3 to 11, Kentucky 16 tiles of zoom 6 to 9.
   With `--tile-zoom Z`, the tiles are the map tiles of zoom `Z` with a node (`TileSize::Zoom`).
2. **The cut.** The tile cut of the build by tiles (`tilecut::cut_to_store`) writes the file of each tile, with the ways
   whole (with their nodes from other tiles), and each multipolygon and boundary relation whole (all its member ways and
   their nodes) in each tile with a member, as the cut of the build: each tile file is a complete, standard `.osm.pbf`
   (Kaveh's rule). With one zoom for all of France (zoom 11, 5 883 tiles), the copies of the large boundaries filled 32 GB
   of temporary files; the quadtree has far fewer tiles. Natively the parts and the tile files are files in a temporary
   folder; in the browser, chunks of one temporary file of OPFS (`tilecut::Chunks`), because the browser has no folders.
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
12.2 GB: the browser compresses it with Snappy, the native run with zstd (9.7 GB). France sorted does not fit on the disk
of the Mac mini in the browser yet: there, the runs of the sort are not compressed (no zstd in WebAssembly), about 20 GB
more.

Two faults of the browser came out on the way. In the browser, pass 4 collected all relations of the file in memory
before it made them: now it makes them in batches of 4 million points. And the allocator of Studio's WebAssembly module
kept each freed small block with the thread that freed it, so memory grew from tile to tile up to 4 GiB; Studio's
allocator now gives the extra free blocks to a shared pool.

Two changes made the tile mode faster natively (France, one run measured again the same day: 104.3 s): the temporary
tile files have no zlib (the cut 88 s to 71 s, 30 % less CPU), and 4 tiles run at the same time, with their rows given in
the order of the tiles (the tiles 136 s to 100 s; the rows of 4 tiles wait in memory). The browser makes one tile at a
time, because a file in OPFS belongs to the thread that opened it.

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
   `duckosm-rust quack-union DIR -o FILE.parquet`, or in DuckDB
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

`duckosm-rust` raises its limit of open files to 10 240 at the start: 4 tiles at a time keep about 700 temporary files
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

## What QuackOSM has and duckOSM does not have

The filters: tags (`tags_filter`), OSM ids, custom SQL, and a geometry (`geometry_filter`, features that touch it). The
automatic download of an extract for a geometry, the geocoding, more than one PBF in one call, the tags as columns
(`explode_tags`), the Hilbert sort (`sort_algorithm="hilbert"`) and the WKT output. `duckosm-rust quack` makes the table of
QuackOSM's default options, with or without the sort.
