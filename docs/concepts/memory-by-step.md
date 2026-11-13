# Memory at each step of the build

Status: measurement, 2026-11-13. This page shows what is in memory at each step of `duckosm_build::run`. It describes the
code on `main` after PR 147 (d7841f3). The page of the morning of the same day (main b58ea48) had a peak of 3.2 GB on
Kentucky; the changes since then are in [What changed](#what-changed).

## How it was measured

- **Areas and options:** driving, `--building-faces --levels solve`, on the Mac mini (12 cores, 16 GB).
  - Kentucky, cut from the US South file with `kentucky.geojson`, also with `--raw` (the options of the Studio page).
  - France, from the Geofabrik France file, with a box around metropolitan France as the boundary, without `--raw` (raw.*
    of France is about 20 GB of files; its memory is that of its sorts, see step 7). France: 27.8 million edges, 49.5
    million buildings, 894 s, peak RSS 7.3 GB.
- **Live heap:** a counting allocator (`--features heap-probe`). At the end of each step the build writes the live heap and
  the peak of the step. The numbers are MiB.
- The browser build (Studio) runs the same function `run`. Its numbers are larger, because WebAssembly memory does not
  become smaller.

## The rule

Each table goes to its file when the step makes it, and a later step reads what it needs from storage: the area file, the
spill files of the edges, or a temporary file. Data that a step does not read is not in memory during that step. The
exceptions are in [What is still held](#what-is-still-held) and [What still grows with the area](#what-still-grows-with-the-area).

## The step numbers

The numbers are the numbers of [Offline and online builds](offline-online.md#each-step-where-the-data-is-and-how-it-is-handled).
The steps after the network run in this order: 5.2, 5.3, 5.5, 5.4, 5.6, 7.

| Number | Step |
|---|---|
| 0 | Cut to the area file |
| 1.1 | Read the ways |
| 1.2 | Read the node positions |
| 1.3 | Read the turn restrictions |
| 1.4 | Read the raw part: the elements that the fixes touch, the route relations and their member ways |
| 2 | Global junctions |
| 3.1 to 3.13 | The network of the mode (the numbers of [step 3 in detail](offline-online.md#step-3-in-detail-the-network-of-a-mode)) |
| 4 | Write the network files |
| 5.2 | Bus routes |
| 5.3 | Area tables |
| 5.5 | Lane profile |
| 5.4 | Levels, by tiles |
| 5.6 | Building faces, by bands of tiles |
| 7 | `raw.*` |

## What stays in memory between steps

| Data | Made in step | Freed after step | Read by steps |
|---|---|---|---|
| Ways: the highway ways as read, with the fixes | 1.1 | 5.5. With `raw.*`, only the ways whose tags the build changed stay, to 7. | 2, 3, 5.3, 5.5, 7 |
| Node positions | 1.2 | before 3: they go to a temporary file sorted by id; only their box stays | 3 (from the file), 5.3 (the box) |
| Turn restrictions | 1.3 | 4 | 3.12 |
| Raw part | 1.4 | 5.2 for the route relations and their member ways; 7 for the elements that the fixes touch | 5.2, 7 |
| Junctions | 2 | 5.3 | 3, 5.3 |
| Network nodes | 3.8 | 5.3 | 4, 5.3 |
| Edges | 3 | never in memory: spill files on disk. The table of the shared tag sets of the edges stays in memory to the end. | 4, 5.2 to 5.6 read their fields from the files |

## Memory at each step

Heap peak of each step, MiB. "Held" is the live heap at the start of the step: the data from earlier steps.

| Step | Kentucky: held | Kentucky: peak | France: held | France: peak |
|---|---|---|---|---|
| 0. Cut | 0 | 356 | 0 | 8,179 |
| 1.1 Read the ways | 0 | 253 | 0 | 3,346 |
| 1.2 Read the node positions | 187 | 619 | 2,619 | 8,433 |
| 1.3 Read the turn restrictions | 385 | 414 | 5,001 | 5,069 |
| 1.4 Read the raw part | 397 | 422 | 5,049 | 5,919 |
| 2. Global junctions | 404 | 601 | 5,868 | 8,541 |
| 3. Network | 438 | 618 | 6,412 | 7,622 |
| 4. Write the network files | 413 | 495 | 5,714 | 5,798 |
| 5.2 Bus routes | 413 | 447 | 5,714 | 10,028 |
| 5.3 Area tables | 413 | 413 | 5,714 | 5,714 |
| 5.5 Lane profile | 299 | 750 | 3,927 | 7,700 |
| 5.4 Levels | 88 | 227 | 462 | not measured (below 4,882) |
| 5.6 Building faces | 88 | 367 | 462 | 855 |
| 7. `raw.*` | 88 | 192 | — | — |

For the levels of France, the mark at the start of the step also has the lane edges, which go at once; the peak of the
levels alone is not measured.

### Step 3: the network substeps (Kentucky)

During all substeps the build holds the ways, the junctions and the raw part (about 420 MiB). The node positions are in a
temporary file, and the edges are in spill files on disk.

| Substep | Peak |
|---|---|
| 3.1 Mode filter (makes the mode ways and writes the `ways` tables) | 627 |
| 3.2 Cut the ways into forward edges (a join on disk with the node positions) | 603 |
| 3.3 Merge chains | 525 |
| 3.4 to 3.7 Split arcs, reverse edges, `edge_id`, lengths | 531 |
| 3.8 Nodes | 513 |
| 3.9 Connectors (driving: none) | 424 |
| 3.9 to 3.11 Walked edges, speeds, private roads, largest part | 435 |
| 3.11 The sort by `edge_id` | 535 |
| 3.12 Turn restrictions | 439 |
| 3.13 Edge graph | 618 |

### Steps 5.6 and 7 in parts

| Part | Kentucky peak | France peak |
|---|---|---|
| 5.6 The fixes and the building multipolygons | 115 | 577 |
| 5.6 The building ways, to a temporary file | 131 | 583 |
| 5.6 The join with the nodes of the file | 216 | 759 |
| 5.6 The rings, to a temporary file | 101 | 615 |
| 5.6 The rings and the streets, sorted by place | 214 | 715 |
| 5.6 The bands of tiles | 367 | 855 |
| 7 The nodes of the ways, sorted | 179 | — |
| 7 The nodes, and the join | 181 | — |
| 7 The ways and the relations | 192 | — |

## What is still held

After step 5.5 the build holds about 90 MiB on Kentucky and 460 MiB on France: the elements that the fixes touch, the ways
whose tags the build changed (with `raw.*`), the table of the shared tag sets of the edges, and small lists.

## What still grows with the area

These steps hold a list of the whole area:

| Step | France peak | What is in memory |
|---|---|---|
| 0. Cut | 8.2 GB | the ids of all nodes inside the boundary (4.1 GB), and the kept ways |
| 1.2 Read the node positions | 8.4 GB | all ways (2.6 GB) and the positions of their nodes |
| 2. Global junctions | 8.5 GB | the ways, and the count of the ways at each node |
| 3. Network | 7.6 GB | the ways, the junctions, and the mode ways of the cut |
| 5.2 Bus routes | 10.0 GB | the route relations and the lines of their member ways |
| 5.5 Lane profile | 7.7 GB | the lane fields of all edges |

The levels (by tiles), the building faces (by bands of tiles) and `raw.*` (sorts on disk) do not hold a list of the whole
area.

## What changed

The changes of 2026-11-13, Kentucky with the Studio options:

| | Morning (main b58ea48) | Now |
|---|---|---|
| Highest heap peak | 3,228 MiB (lane profile) | 750 MiB (lane profile) |
| Peak RSS | 5.7 GB | 2.1 GB |
| Build time, native | 33.0 s | 33.5 s |
| Studio page: memory | 3,251 MiB | 886 to 1,031 MiB |
| Studio page: build time | 49.9 s | 53.5 s |

- The tables go to their files when they are made (PRs 130 to 136, 143); the steps after the network read the edges from
  the spill files.
- The network and `raw.*` get the coordinates from a join on disk (PRs 138, 139, 145).
- The levels run by tiles, and the tiles that wait for no running tile run at the same time (PRs 144, 145). A tile has no
  meet rule between two roads of earlier tiles (roadstyle PR 190, in PR 147).
- The building faces read the area file one way at a time and run by bands of tiles from temporary files (PRs 141, 142,
  146).
- After the lane profile and after the bus routes, the build frees the data that later steps do not read (PRs 145, 147).
