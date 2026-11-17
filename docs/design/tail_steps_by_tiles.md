# The bus routes and the levels with bounded memory (proposal)

Status: proposal, 2026-11-16. It waits for Kaveh's yes.

In a build by tiles, two steps run on the whole area after the union: the bus routes (`union::bus_routes`) and the
drawing levels (`union::levels`, which calls `levels::tiled`). This document gives their time and memory on three areas,
the structures that hold the whole area, and a design that removes them. The result stays equal to the whole build.

The filter of the largest part is not in this document: it was removed on 2026-11-14 (`build_by_tiles.md`, "The largest
part"). Only the help text of `duckosm-rust tiles` still names it. The change corrects that text.

## The measurements

Mac mini (Apple M6, 12 cores, 16 GB), native, `duckosm-rust tiles ... -- --modes driving --format geoparquet`, zoom 8,
main at `85284ac`. France is the box of `france-box.geojson`, Sweden a box of 10.5–24.2° E, 55.0–69.1° N.

| Area | Tiles | Driving edges | Union | Bus routes | Levels (tags) | Levels (solve) | Total |
|---|---|---|---|---|---|---|---|
| Kentucky | 17 | 2.1 M | 0.3 s | 0.6 s | 2.4 s | | 10.3 s |
| Sweden | 183 | 5.3 M | 0.6 s | 2.0 s | 4.7 s | | 29.7 s |
| France | 104 | 25.9 M | 13.8 s | 14.9 s | 40.2 s | 54.8 s ¹ | 305.3 s |

¹ From the union alone on the kept tiles (`examples/tile_union.rs`), not from the full command.

The peak of the Rust heap in each part of the steps (the heap probe, `DUCKOSM_HEAP=1 tile_union`, levels `tags`):

| Part | Kentucky | Sweden | France | Grows with the area |
|---|---|---|---|---|
| Union (block copy of the files) | 25 MB | 25 MB | 25 MB | no |
| Bus 1: read the relations and member ways of the tiles | 3.7 MB | 54 MB | 398 MB | **yes** |
| Bus 2: the member edges to a temporary file | 15 MB | 190 MB | 609 MB | **yes** |
| Bus 3: the batches | 28 MB | 110 MB | 548 MB | **yes** |
| Levels: the sort by tile | 413 MB | 445 MB | 406 MB | no (sort budget 256 MB) |
| Levels: the tiles | 76 MB | 71 MB | 184 MB | **yes** |
| Levels: the rows (sort by `edge_id`) | 252 MB | 517 MB | 520 MB | no (sort budget 256 MB) |

The live heap after bus 1 is 2.3 MB, 48 MB and 377 MB: about 480 bytes for each member way.

The RSS of the process is larger than the heap on macOS, because the allocator keeps freed pages. In France the levels
start at 501 MB RSS with an empty heap: these are the pages that the bus step freed. The peak RSS of the steps after the
union is 1.6 GB (tags) and 1.4 GB (solve); in Sweden it is 1.1 GB. On Linux, `duckosm-rust` gives freed blocks back to
the system (`mallopt`), so its RSS is nearer to the heap.

## What holds the whole area

The bus routes (`union::bus_routes`):

- `relations`: all route relations of the area, with their members and tags (France: 25,985).
- `ways`: all member ways with their nodes and their direction tags (France: 792,227). This is most of the 377 MB.
- `routes`: all rows of `routes_bus`, until the end.
- Each batch finds its member ways with a scan of all member ways (`ways.iter().filter(...)`): 792,227 × 201 batches in
  France. This costs time, not memory.

The levels (`levels::tiled::write_levels_tiled_from`):

- `earlier`: the numbers of each edge that a later tile needs. The step adds an entry when the tile of the edge is done,
  and never removes one. The `tags` rule fills it too, but does not read it.
- The maps of the grid cells (0.1°): `boxes`, `near`, `order`, `waits`, `depth`, `done`. They grow with the number of
  cells that have roads, not with the edges: a few MB for France.
- The two sorts use a fixed budget (256 MB). Their peak (about 520 MB) does not grow from Sweden to France.

## The design

### The bus routes: each batch reads only its own data

The batches stay as they are: consecutive relations in the order of their ids, with about 20,000 member ways each. Only
the place of the data changes: the relations and the member ways go to temporary files sorted by batch, as the member
edges do now.

1. Read the `bus.osm.pbf` file of each tile. Write each relation to a temporary file and each member way to another
   temporary file. Sort the two files by id, and keep one copy of each id (a route is in each tile of its ways).
2. Read the relations in the order of their ids. Give each one its batch (the rule of today). Write the pairs
   (member way id, batch) to a list in memory, sorted by way id. This list is the only structure that grows with the
   area: 12 bytes for each member way and batch (France: about 10 MB).
3. Read the member ways in the order of their ids. Write each way, with its batch, to a temporary file sorted by batch.
4. Read the union edges once, as now: each member edge goes to the temporary file of its batches.
5. For each batch, read its relations, its member ways and its edges from the three sorted files. Walk the routes
   (`bus::routes`, not changed). Write the rows of `route_edges_bus` and of `routes_bus` to part files. Join the part
   files as blocks at the end.

The input of each batch is the same as today: the same relations in the same order, the same member ways and the same
edges in the order of `edge_id`. So the rows are the same and in the same order. The part files of `routes_bus` are in
the order of the batches, which is the order of the relation ids, as today.

What crosses the tile edges: a route has ways in many tiles. Step 1 puts the copies of all tiles together, so a batch
always has all relations and member ways of its routes, as now.

Memory: one batch (its relations, ways and edges), the sort buffers (64 MB) and the list of step 2. The scan of all
member ways for each batch goes, so the step does not become slower.

### The levels: an entry of `earlier` goes when the last tile that needs it has it

At step 2 of the levels, each edge already goes to its own tile and to the later tiles whose box it touches. The step
then knows how many later tiles need the numbers of the edge. The change:

- The edge carries this count to its tile (in place of the flag `needed by a later tile`).
- When the tile of the edge is done, `earlier` keeps the numbers with the count.
- When a later tile starts, it takes the numbers of its edges from `earlier` and decreases each count. An entry with the
  count 0 goes.
- The `tags` rule fills no entries: its tiles read no numbers of other tiles.

What crosses the tile edges: a bridge or a tunnel can cross the edge of a tile. Its edges are in all tiles that its box
touches; one tile owns each edge, and the later tiles read the numbers of the owner, as now. Only the time that an entry
stays in memory changes, so the numbers and the rows do not change.

Memory: the entries of the tiles that are done and of the tiles that still wait for them. The tiles run in the order of
the rows of cells, so these are about one row of cells across the area. This grows with the width of the area, not with
its surface.

This code also runs in the whole build with `--levels solve` (native, and in the browser, where Studio uses solve). The
change makes it smaller there too.

## The checks

- Kentucky by tiles equals the whole build: `routes_bus`, `route_edges_bus` and `edge_levels_visualization` row by row,
  with `tags` and with `solve`. No golden file changes.
- France: the time of the two steps is equal or less, and their heap is less. The expected heap of the bus step: one batch
  and the sort buffers, about 150 MB (now 609 MB). The expected heap of the levels: the two sorts (about 520 MB), the
  same from Sweden to France.
- The numbers are labeled native. The browser has no build by tiles of the network, so the bus step has no browser
  number. The levels change also applies to the browser's whole build with `solve`.

## A fault found during the measurements

With `--keep-tiles`, the union deleted the files of the tiles for each table that has border rows: it gave the delete
flag of the temporary border file to all files. The union now deletes the files of the tiles only without
`--keep-tiles`. The change has this fix.
