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

## What is next

1. The build with a boundary writes `cut.osm.pbf` in step 0 and reads it as a whole file: offline, `raw.*` goes into
   DuckDB first (`read_osm`), and the network reads from DuckDB.
2. Online, Studio writes the area file to OPFS through a `FileSystemSyncAccessHandle`. A test of this writer once gave
   a wrong count of written bytes (the value −8 as an unsigned number, probably a `FILE_ERROR_NO_SPACE` of the
   browser). This must be understood before Studio uses it.
