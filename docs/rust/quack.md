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
- A run that is stopped (a signal) leaves its temporary files: 45 GB after some stopped runs. Open: delete them on a stop,
  or delete the files of dead runs at the start.

## The sort

QuackOSM sorts the rows by default with Sort-Tile-Recursive packing (`rq_geo_toolkit`, `_str_index_select_sql`), and
`duckosm_core::quack::str_order` uses the same rule:

1. The key of a row is the center of the bounding box of its geometry.
2. The rows sorted by x (then y) are cut into `ceil(sqrt(ceil(rows / 100 000)))` vertical strips (Kentucky: 5).
3. Each strip is sorted by y: up in the even strips, down in the odd strips.

Each row group of 100 000 rows then holds the rows of a small area. A reader with a bounding box can skip the other row
groups, and the file is smaller. On Kentucky, 2 391 567 of the 2 393 139 rows (99.93 %) are at the same position as in
QuackOSM's file, all rows are in the same row group, and no row is more than 11 positions away (rows with equal centers
are in another order). The sort needs all rows before it writes the first row, so the rows wait in memory.

## The results

The tables of Kentucky (all cases of both tools) and of France (unsorted GeoParquet), the row checks and the results that
are not in favor of duckOSM are on the page of the manual, `docs/concepts/quackosm.md`. The scripts that measured them ran
on the Mac mini: `/usr/bin/time -l` for the time, the CPU time and the macOS footprint, a sampler of the RSS of the
process tree and of the free disk every 2 s for the large runs, and `stat` for the file sizes.

## What QuackOSM has and duckOSM does not have

The filters: tags (`tags_filter`), OSM ids, custom SQL, and a geometry (`geometry_filter`, features that touch it). The
automatic download of an extract for a geometry, the geocoding, more than one PBF in one call, the tags as columns
(`explode_tags`), the Hilbert sort (`sort_algorithm="hilbert"`) and the WKT output. `duckosm-rust quack` makes the table of
QuackOSM's default options, with or without the sort.
