# Step 0: the cut to an area file

Step 0 of a build with a boundary cuts the region file to the area. This note describes the cut that writes the
area to a `.osm.pbf` file while it reads the region file (`duckosm_core::cutstream::cut_to_pbf`, the command
`duckosm-rust cut`).

## The problem

The cut in memory (`pbf::read_area_raw`) keeps all elements of the area in memory to the end of the build: all
nodes with their coordinates and tags, all ways with their node lists, and the relations. On Kentucky (cut from the
Geofabrik file of the US South, 4.16 GB) that is 3.7 GB after step 0, with a peak of 4.4 GB. Online, in the browser,
a WebAssembly module has at most 4 GB, so this cut cannot cut Kentucky.

## The design

Step 0 writes the area as a file, and the later steps read that file as a whole file. Then a build with a boundary
has one step more than a build of a whole file, and every improvement of the whole-file path is also for areas.

The cut keeps only lists of ids. Four passes read the region file:

| Pass | It reads | It keeps in memory | It writes |
|---|---|---|---|
| 1 | all blocks of nodes | the ids of the nodes inside; the lowest and highest node id of each block | — |
| 2 | all blocks of ways and relations | the ids of the kept ways and of their nodes outside; the members of the relations; the id range of each block | — |
| member ways | only the blocks with a member way of a multipolygon | the ids of these ways and of their nodes outside | — |
| 3 | only the blocks of nodes with a kept node | — | the kept nodes |
| 4 | only the blocks of ways and relations with a kept element | — | the kept ways, then the kept relations |

The rules are those of the cut in memory (osmium's `smart` cut): the nodes inside; each way with one node inside,
whole, with its nodes outside; the relations with a member node inside or a member way kept, and their parent
relations; the member ways and nodes of the multipolygons among them.

Other parts of the design:

- **One block writer.** `cutfile::PbfWriter` writes the blocks of both cuts.
- **Each thread makes, compresses and frees its own blocks.** In passes 3 and 4 the main thread only writes the
  ready blocks in the order of the region file. An element is freed on the thread that made it. The allocator of the
  browser build keeps a freed small block on the thread that frees it; when the main thread freed the elements, that
  allocator kept about 580 MB on Kentucky. The compression also runs on all threads.
- **The decoder reads only the tags that a pass needs** (`pbfraw::decode_block_tags`). Passes 1 and the pass of the
  member ways read no tags; pass 2 reads only the tags of the relations (`type=multipolygon`). With no tags, the
  string table of a block is not read.
- **The region file must be sorted by type and id** (`Sort.Type_then_ID`, as the files of Geofabrik). Then the
  elements arrive in the order of the area file.

The area file has the same elements as the file of the cut in memory, with the same tags and coordinates. It does
not have the same bytes: it has one block of the area file for each block of the region file.

## The measurements

Kentucky (Geofabrik's border `kentucky.poly`) cut from `us-south-latest.osm.pbf` (4.16 GB), on a Mac mini (Apple M6,
12 cores, 16 GB), three runs each. The area file has 20,517,720 nodes, 1,845,848 ways and 15,270 relations.

| | Offline (native, 12 threads) | Online (Chromium, 8 threads, the file to OPFS) |
|---|---|---|
| Cut in memory | 22.7 s, peak 4,429 MiB | stops at the 4 GB limit |
| This cut | **10.8 s, peak 353 MiB** | **20.2 s, 441 MiB of WebAssembly memory** |

The time of each pass of this cut:

| Pass | Offline | Online |
|---|---|---|
| 1. The nodes inside | 5.5 s | 8.7 s |
| 2. The ways and relations | 1.6 s | 3.9 s |
| The relations, the member ways | 0.1 s | 0.3 s |
| 3. Write the nodes | 1.7 s | 3.7 s |
| 4. Write the ways and relations | 1.9 s | 3.6 s |

Passes 1 and 2 read all of the region file. Passes 3 and 4 read only the blocks with a kept element.

Online is slower than offline for two reasons, both measured:

1. **Fewer threads:** 8 against 12. With 8 threads the native cut takes 17.1 s instead of 14.0 s (measured before
   the skip of blocks).
2. **Each thread is about 1.6 times slower.** A test that reads all of the US South file on one thread gives, natively
   and online:
   - reading the file: 0.9 s and 1.3 s;
   - unpacking (zlib): 12.3 s and 18.7 s;
   - decoding the elements: 8.4 s and 19.1 s.

   So the decoding and the unpacking in WebAssembly are the causes, not the reading of the file through the browser.
   The allocator is not a cause: with one thread, the allocator of the browser build and the standard allocator give
   the same time.

## Step 1 online: readers that the browser can use

In the browser, only the thread that opened a file in OPFS can read or write it. The readers of `osmpbf`
(`par_map_reduce`) read the blocks on the threads of the pool, so in the browser they wait for ever, and `load_raw`
writes its shards from the threads of the pool. The new functions read (and write) on the calling thread and decode
on all threads:

- `pbf::read_ways_from`, `pbf::read_coords_from`, `pbf::read_restrictions_from`: the network input from any reader,
  the same as `read_ways`, `read_coords` and `read_restrictions` (a test on Monaco).
- `duckosm_parquet::load_raw_here`: `raw.*` as Parquet, written on the calling thread (a test on Monaco; on Kentucky
  the files are equal, row by row, to the tables of `read_osm`).

Step 1 on Kentucky's area file (the same code offline and online, three runs each):

| Step 1 | Offline | Online (8 threads) |
|---|---|---|
| Highway ways (690,243) | 0.33 s | 0.51 s |
| Coordinates (8,616,177) | 0.40 s | 0.80 s |
| Turn restrictions (3,062) | 0.18 s | 0.38 s |

**Online, the raw data stays in the area file.** `raw.*` as Parquet is 816 MB uncompressed in OPFS (325 MB with
gzip; zstd does not build for WebAssembly), against 149 MB for the area file, and writing it takes 4.4 s online.
Reading is not the reason: with one thread each, the Rust engine reads all of the area file into Arrow columns in
2.0 s, and DuckDB-Wasm opens and reads the three Parquet files in 2.0 s. So online, the steps in Rust read their part
of the area file, and `raw.*` as Parquet is made only for the base-map layers (their SQL runs in DuckDB-Wasm) and
deleted after them.

When the storage quota of the site is full, Chromium gives the error −8 (`FILE_ERROR_NO_SPACE`) as the count of
written bytes of a `FileSystemSyncAccessHandle` write. A test browser had a quota under 1 GB.

## The build reads the area file

A build with a boundary (`duckosm_build::build`) runs step 0 first and writes the area file: `cut.osm.pbf` in the
output folder when the user asks for it, else a temporary file. Then `run` reads the area file (`Input::Area`):

- **The network input** (step 1): the readers of a whole file (`read_ways_from`, `read_coords_from`,
  `read_restrictions_from`). The network keeps only the ways with a node inside the boundary, as osmium's
  `complete_ways`; the area file also has the member ways of its multipolygons.
- **The raw part** (`raw::read_part`): only the elements that the build needs in memory: the elements that the OSM
  fixes touch, the route relations of the bus routes with their member ways, and the buildings with their nodes (for
  the building faces). The fixes change the part (`Raw::apply_fixes`).
- **`raw.*`** (`duckosm_parquet::write_area_raw`): written while the area file is read, in two passes (the nodes,
  then the ways and the relations). Each element gets its version of the raw part, and the tags of the network's
  ways (`RawPatch`). In memory: the coordinates of all nodes, for the column `line` of the ways.

A test on Monaco (`raw_of_the_area_file_equals_the_cut_in_memory`) shows that `raw.*` has the rows of the cut in
memory, with the fixes of both fixture files and the tags of the sidewalk rule.

Kentucky from the US South file, the driving mode, `raw.*` and the building faces (the Mac mini above, the Rust heap
of the cargo feature `heap-probe`):

| | Before (the cut in memory) | After (the area file) |
|---|---|---|
| Heap after the input is read | 3,663 MiB | 1,317 MiB |
| Peak heap (in the building faces) | 17,206 MiB | 14,860 MiB |
| Time | 54.2 s | 44.5–46.9 s |

All files of the network, the bus routes and the building faces have the same bytes. Most of the memory is now in the
building faces: the step itself, and the street segments that it reads (1.9 GB on Kentucky).

## The building faces in parts

The building faces step (`building_faces::building_faces_each`) gives the faces in parts of 32,768 rings, in the order
of the result, and `BuildingFacesWriter` writes each part. All threads compute the walls of the rings of a part. The
segments of the streets share the name and the `highway` value of their edge (`Arc<str>`), not one copy for each
segment. Kentucky, the same run as above:

| Building faces | Before | After |
|---|---|---|
| Peak heap | 14,860 MiB | 6,636 MiB |
| Time | 15.0 s | 7.4 s |

The file has the same rows in the same order and the same row groups (the values of each column are equal); the pages
in the file are not the same bytes.

The bus routes (`bus::routes`) look up the node pairs of the member ways of the routes only, so their map of node pairs
has only the edges of these ways, not all edges. Kentucky: the peak heap of the step goes from 7,218 MiB to 3,991 MiB
(no more than the memory before it), and its time from 1.85 s to 0.28 s. The files have the same bytes.

After these changes, the peak heap of the whole Kentucky build (driving, `raw.*`, building faces) is 6,636 MiB, in the
building faces, and the build takes 36.8 s (before: 17,206 MiB and 54.2 s).
