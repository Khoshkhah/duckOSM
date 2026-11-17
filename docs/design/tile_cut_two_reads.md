# Tile cut with two reads (proposal)

Status: built, 2026-11-15. On 2026-11-16 the cut moved to the pbfcut library (`Khoshkhah/pbfcut`, its `tilecut`
module). duckOSM depends on pbfcut by one pinned git `rev` (`rust/duckosm-core/Cargo.toml`), and `duckosm_core::tilecut`
is a thin module: it keeps the names that duckOSM and Studio call and gives pbfcut duckOSM's rule of the junctions
(`NodeUses`). The tile cut exists only in pbfcut. This document records the design and the measurements of the cut before
the move. The section "Moved to pbfcut" at the end gives the check of the move.

The tile cut (`duckosm_core::tilecut`) makes one `.osm.pbf` file for each map tile of one zoom. Each tile file is
complete: all nodes of its ways, a border, and each multipolygon with all its members. The build by tiles
(`duckosm-rust tiles`), quack by tiles and the browser use this cut.

This document shows the work that the cut does more than one time, and proposes a cut with two reads of the input
file and one write of each element.

## The work today

| Part | What it does | France, native, 20 cores |
|---|---|---|
| 1 | Reads all node blocks. Packs the nodes inside each tile into the node part of the tile. | 40.5 s |
| 2 | Reads all way blocks. Packs each way into the way part of its tiles. Lists the outside nodes that each tile needs. | 28.4 s |
| 3b | Reads the node blocks and the way blocks again: which relation members are in the file. | 6.8 s |
| 3 | Reads the relations. Lists the member ways and nodes of the multipolygons. | 7.7 s |
| 4b | Reads the node blocks again: packs the outside nodes into the extra-node part of each tile. | 11.3 s |
| 4 | Reads way blocks again: packs the member ways of the multipolygons into the extra-way part of each tile. | 2.5 s |
| 5 | Unpacks the parts of each tile, merges them in id order, and packs the tile file. | 18.7 s |
| | **The cut** | **128.6 s** |

So:

- The node blocks are read and unpacked three times (parts 1, 3b and 4b). The way blocks are read three or four times
  (parts 2, 3b, 3 and 4).
- Each element is packed into PBF with zlib two times: into a part file, then into the tile file. Part 5 unpacks it
  between the two. Without zlib in the parts and the tile files, the cut takes 104.5 s (19 % less), and the tile files
  need 7.8 GB, not 5.3 GB.
- The part files hold a copy of all the data until part 5.

The browser has the same parts. There, one thread reads the input file and the storage (OPFS), so each extra read
costs more: Kaveh's laptop cut France at zoom 8 in 559 s, and part 5 took 230 s of it.

## Proposal

### Read 1: the plan (no element is written)

1. **Nodes.** The book of the nodes: the tile of each node inside the area (as today). Also the ids of all nodes of
   the file, in a temporary file: the ids as delta varints, about 1.5 bytes for each node (France: about 0.9 GB).
2. **Ways.** The book of the ways: the tiles of each way (as today). For each tile, the nodes outside it that its ways
   use: rows (node id, tile) in a temporary file, sorted later (as today). The ids of all ways of the file, in a
   temporary file (France: about 0.1 GB).
3. **Relations.** The members of each relation. Which members are in the file: a merge of the sorted member ids with
   the two id files of steps 1 and 2. This merge reads about 1 GB of ids, with no unpack, in place of the two reads of
   part 3b. Then the tiles of each relation (as today), and for the multipolygons the member nodes and member ways for
   each tile: rows (id, tile).
4. The rows of steps 2 and 3, sorted by id: the extra nodes and the extra ways of each tile.

### Read 2: the write

Each tile has one open writer with a small block buffer (about 64 KB; France at zoom 10: 1,311 writers, about 85 MB).

1. **Nodes, in id order.** Each node goes into its own tile, and into each tile that the sorted extra-node rows give
   for it (a merge of the node stream with the rows).
2. **Ways, in id order.** Each way goes into its tiles, and into each tile that the extra-way rows give for it.
3. **Relations, in id order.** Each relation goes into its tiles.

Each tile file is then complete and in the order of the file (nodes, ways, relations, each by id), with no part files
and no part 5.

### What changes

| | Today | Proposal |
|---|---|---|
| Reads of the node blocks | 3 | 2 |
| Reads of the way blocks | 3 to 4 | 2 |
| Packs of each element | 2 (with an unpack between) | 1 |
| Part files (a copy of the data) | yes | no |
| Part 5 | yes | no |
| Temporary files | parts, sort rows | id files (about 1 GB for France), sort rows |

### Questions to answer before the build

1. **Memory of the open writers.** A small block (64 KB) for each tile keeps the memory low. A tile file then has more
   and smaller blocks. The tile builds read them as before.
2. **zlib in the tile files.** The tile files are temporary for the build by tiles. Without zlib they need about 47 %
   more disk (France: 7.8 GB, not 5.3 GB), and the cut and the tile builds are faster. Kaveh decides the storage.
3. **The browser.** Read 2 writes into many tile files from the one thread that owns the storage. The packing of the
   blocks runs on the pool, in the order of the input, as `cutstream::pipeline` does today.
4. **Equal output.** The tile files have other block borders than today. The check is the elements of each tile
   (`tile_cut compare`) and the tables of the build by tiles against the whole build (Kentucky), not the bytes.

## Plan

1. Build read 1 and read 2 as a new function next to `cut`, for the build by tiles first (native).
2. Kentucky: the elements of each tile equal to today; the build by tiles equal to the whole build. France: time,
   memory and disk against today.
3. The browser (Studio's `cut.html`, quack by tiles): Sweden and France at fixed zooms.
4. When all paths are equal or better: the new cut replaces `cut`, and the part files, part 4b and part 5 go.

## Result

Built as one cut: parts 1 to 3 of `cut` only plan (no part files; part 1 also finds the tiles with a node inside, the
tiles with a file), and read 2 (`tilecut/write.rs`) puts each element into its tile files: the nodes (their own tile
from the book of the nodes, the extra tiles from the sorted rows), the junctions (as before), the ways and the
relations. Parts 4b, 4 and 5 and the part files of the nodes, the ways and the relations are gone.

How read 2 works:

- This thread reads the blocks and the extra rows of each block. The pool decodes each block and gives each element
  to its tiles. The elements of a tile from one block go into one buffer of bytes (a chunk), not into an object with
  strings and lists for each element. This thread collects the chunks of each tile until they hold a full block of
  8,000 elements. The pool packs the full blocks while it decodes the next batch, and this thread writes them in the
  order of the file (in the browser the storage belongs to it).
- The collected chunks of all tiles hold 128 MB at most. Above it, the tiles that hold the most write their blocks
  before they are full. Thus the memory does not grow with the number of tiles. (A buffer of element objects for each
  tile, with no limit, took 5.4 GB on France.)
- Before, each block of the input gave each tile its own block: France at zoom 8 had 4 million blocks of 1.3 KB on
  average, and an element for more than 8 tiles had a block of its own (160,000 blocks of 4 nodes in one tile). The
  files were 11.7% larger.
- The lists of read 1 go as soon as read 2 is past them, and the allocator gives memory back between the passes.

| | Old cut | Two reads |
|---|---|---|
| France, native (`duckosm-rust tiles`, the cut) | 107.4 s, 1,489 MB | 92.9 to 94.1 s; peak 1,308 against 1,506 MB in the same probe |
| Sweden in Chromium, zoom 10, 8 threads | 66.0 s, 1,077 MiB | 52.1 s, 1,038 MiB |
| Sweden in Chromium, zoom 6, 8 threads | 45.5 s, 851 MiB | 38.9 s, 856 MiB |
| Kentucky, the cut in the build by tiles | 4.9 s, 434 MB | 4.4 s, 356 MB |
| France, the tile files on disk | 5.3 GB | 6.4 GB (smaller blocks compress less) |

Full blocks for each tile (France, zoom 8, `tile_cut` example, native, two runs of each):

| | Block for each input block | Full blocks |
|---|---|---|
| The cut | 85.8 s, 104.8 s; 1.25 to 1.30 GB | 77.4 s, 77.5 s; 1.25 to 1.32 GB |
| The tile files | 5,133 MB, 4,023,977 blocks | 4,532 MB, 76,780 blocks |

The tile files have the same elements (France 104 tiles, Kentucky 17 tiles).

Checks: the tile files have the same elements as the old cut (Kentucky 142 tiles, France 1,311 tiles); the junctions
files are equal byte for byte; the build by tiles of Kentucky equals the whole build (41 tables, only the build times
differ).

Part 3 reads the relations three times. Each pass decodes only the ids, the member ids, the member types and whether
the type is in the list of complete relations, into flat lists: no strings for the roles and the tags. With strings
(France: 12 million roles in each pass, on all threads), part 3 in the browser took 128 s on a Windows laptop against
15 s in headless Chromium on Linux (in WebAssembly one allocator serves all threads), and the WebAssembly memory grew
to 2.6 GiB. With flat lists: France in headless Chromium, part 3 11.9 s, the cut 211.7 s against 233.7 s, the memory
984 MiB against 2,659 MiB. The tile files and the junctions files do not change.

The steps of the cut, in the order that they run (the names that the cut prints): 1 the nodes inside, 2 the ways, 3a
to 3d the relations, 4 the nodes from outside (read 1); 5a the nodes to the tiles, 5b and 5c the junctions files (the
build by tiles only), 5d the ways, 5e the relations and the tile files (read 2).

The id files of step 3 are done (PR 197). The size of the tile files is done: full blocks for each tile.

## Moved to pbfcut

On 2026-11-16 the cut moved to the pbfcut library. The pin is `918960d` (pbfcut PR 38: every cut takes `compress`).

| Check | Result |
|---|---|
| The tile files before the move: duckOSM's cut against pbfcut (`--complete multipolygon`), zoom 8 | Kentucky 20 files, France 104 files: the same elements (`pbfcut compare`) |
| The junctions files of France, zoom 8: the old cut against the new | 104 files, equal byte for byte |
| The Kentucky build by tiles against the whole build (three modes, `--raw`, zoom 8) | 41 tables equal; the meta row of the levels differs only in its build time |
| The cut of France, zoom 8, native (`tile_cut` example, 4 runs each, a shared computer) | new 81.9 to 104.3 s, old 79.2 to 97.4 s; peak 1.15 to 1.18 GB for both |

`duckosm_core::tilecut` keeps the names that Studio calls (`cut_to_tiles`, `cut_to_store`, `cut_to_store_build`,
`cut_to_sink`, `read_junctions`, `node_positions`, `border_m`, `near_tile`, `JUNCTION`, `ROAD`, `HOME`, `TileFiles`,
`Sink`). As before the move, `cut_to_sink` and `cut_to_store` write the tile files with no zlib (`compress` false; with
`memory`, `cut_to_sink` uses zlib), and the cut of the build by tiles keeps zlib. Sweden in Chromium at zoom 8 (8 threads, Studio's quack page, Mac mini): the old cut 17.8 s and 861 MiB; the new cut
with zlib in the sink 24.0 s; with no zlib 18.3 s and 625 MiB. The rows of quack are equal.
