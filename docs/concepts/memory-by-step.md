# Memory at each step of the build

Status: measurement, 2026-11-13. This page shows what is in memory at each step of `duckosm_build::run`, and which of it the
step reads. It describes the code as it is on `main` (b58ea48). It is the base for the decision of what stays in memory and
what goes to storage.

## How it was measured

- **Area and options:** Kentucky, cut from the US South file with `kentucky.geojson`, driving, with `--raw --building-faces
  --levels solve` (the options of the Studio page). Mac mini, 12 cores.
- **Live heap:** a counting allocator (`--features heap-probe`). After each step the build writes the live heap and the
  peak of the step. The numbers are MiB.
- **Size of each held item:** a test copy of `run` stops at five points (before the network, after the network files are
  written, after the edges are read back, at the area tables, at the faces). There it frees the held items one at a time
  and writes the live heap after each. The difference is the size of the item.
- The browser build (Studio) runs the same function `run`. Its numbers are larger, because WebAssembly memory does not
  become smaller.

## The step numbers

The numbers are the numbers of [Offline and online builds](offline-online.md#each-step-where-the-data-is-and-how-it-is-handled).
This page splits some steps into smaller parts:

| Number | Step |
|---|---|
| 0 | Cut to the area file |
| 1.1 | Read the ways |
| 1.2 | Read the node positions |
| 1.3 | Read the turn restrictions |
| 1.4 | Read the raw part |
| 2 | Global junctions |
| 3.1 to 3.13 | The network of the mode (the numbers of [step 3 in detail](offline-online.md#step-3-in-detail-the-network-of-a-mode)) |
| 4 | Write the network files |
| 5.1 | Read the edges back |
| 5.2 | The end of the mode: bus routes, street segments, lane edges, level edges |
| 5.3 | Area tables |
| 5.4 | Levels |
| 5.5 | Lane profile |
| 5.6 | Building faces |
| 7 | `raw.*` (write raw) |

## The data that stays in memory between steps

| Data | Size (MiB) | Made in step | Freed after step | Read by steps |
|---|---|---|---|---|
| Ways: the highway ways as read, with the fixes | 187 | 1.1 | 7 (the end) | 2, 3, 5.3, 5.5, 7. Steps 2 (sidewalk rule) and 5.2 (arrows of the fixes) change their tags. |
| Node positions | 198 | 1.2 | 5.5 | 3, 5.3 |
| Turn restrictions | 1 | 1.3 | never freed | 3.12 |
| Raw part: the elements that the fixes touch, the route relations, the buildings with their nodes | 871 | 1.4 | 7 (the end) | 5.2 (route relations, building grid), 5.6 (buildings), 7 (touched elements) |
| Building grid | 1 | before 3 | 7 (the end) | 5.2 (street segments) |
| Junctions | 34 | 2 | 5.5 | 3, 5.3 |
| Mode ways (`net.ways`): the ways of the mode after the road filter | 318 | 3.1 | 5.2 | 3.2, 3.5, 3.12, 4 (the `ways` tables), 5.2 (arrows of the fixes) |
| Network nodes (`net.nodes`) | 25 | 3.8 | 5.5 | 4, 5.3 |
| Buffers of the edge spill files (the edges are on disk) | 29 | 3 | 5.2 | 4 |
| Edges and private edges, read back from the spill files | 708 + 70 | 5.1 | 5.2 (they move into the level edges) | 5.2 |
| Street segments | 202 | 5.2 | 5.6 | 5.6 |
| Lane edges (a copy of fields of the edges) | 237 | 5.2 | 5.5 | 5.5 |
| Level edges (the edges, moved) | 776 | 5.2 | 5.5 | 5.4 |

## Memory at each step

"Held" is the data from earlier steps that is in memory when the step starts. "Not read" is the part of it that the step
does not read. Sizes in MiB.

| Step | Live at start | Peak | Held from earlier steps | The step reads | Held and not read |
|---|---|---|---|---|---|
| 0. Cut | 0 | 356 | — | region file, boundary | — |
| 1.1 Read the ways | 0 | 253 | — | area file | — |
| 1.2 Read the node positions | 187 | 619 | ways | ways, area file | — |
| 1.3 Read the turn restrictions | 385 | 404 | ways, node positions | area file | ways, node positions |
| 1.4 Read the raw part | 386 | 1,423 | ways, node positions, restrictions | area file | ways, node positions, restrictions |
| 2. Global junctions | 1,257 | 1,455 | + raw part | ways | node positions, restrictions, raw part: 1,070 |
| 3. Network, 3.1 to 3.13 (next table) | 1,292 | 1,933 | ways, node positions, restrictions, junctions, raw part, building grid | ways, node positions, restrictions, junctions | raw part, building grid: 872 |
| 4. Write the network files | 1,665 | 2,086 | + mode ways, network nodes, spill buffers | spill files, mode ways, network nodes | ways, node positions, junctions, raw part, grid, restrictions: 1,292 |
| 5.1 Read the edges back | 1,665 | 2,443 | the same | spill files | the same: 1,292 and more |
| 5.2 The end of the mode | 2,413 | 2,755 | + edges 778 | edges, raw part (route relations), building grid | ways, node positions, junctions, restrictions, most of the raw part |
| 5.3 Area tables | 2,532 | 3,220 | ways, node positions, junctions, network nodes, raw part, grid, restrictions, street segments, lane edges, level edges | ways, node positions, network nodes, junctions: 444 | level edges, lane edges, street segments, raw part, grid, restrictions: 2,088 |
| 5.4 Levels | 2,532 | 3,138 | the same | level edges: 776 | all the rest: 1,756 |
| 5.5 Lane profile | 2,532 | 3,228 | the same | lane edges, ways: 424 | level edges and all the rest: 2,108 |
| 5.6 Building faces | 1,263 | 2,817 | ways, raw part, street segments, grid, restrictions | buildings of the raw part, street segments | ways, restrictions, grid; the raw part that is not buildings (not measured) |
| 7. `raw.*` | 1,060 | 1,796 | ways, raw part, restrictions | area file, touched elements of the raw part, ways | the buildings of the raw part (not measured), restrictions |

The three highest peaks are 5.5 lane profile (3,228), 5.3 area tables (3,220) and 5.4 levels (3,138). In each of them,
more than 1.7 GB is in memory that the step does not read.

### Step 3: the network substeps

During all substeps the build holds 1,292 MiB from the steps before (the table above). From 3.1, it also holds the mode
ways (318 MiB). The edges are in spill files on disk.

| Substep | Live at the end | Peak |
|---|---|---|
| 3.1 Mode filter (makes the mode ways) | 1,647 | 1,793 |
| 3.2 Cut the ways into forward edges | 1,642 | 1,683 |
| 3.3 Merge chains | 1,642 | 1,750 |
| 3.4 to 3.7 Split arcs, reverse edges, `edge_id`, lengths | 1,643 | 1,769 |
| 3.8 Nodes | 1,667 | 1,732 |
| 3.9 Connectors (driving: none) | 1,667 | 1,667 |
| 3.9 to 3.11 Walked edges, speeds, private roads, largest part | 1,671 | 1,678 |
| 3.11 The sort by `edge_id` | 1,667 | 1,778 |
| 3.12 Turn restrictions | 1,667 | 1,683 |
| 3.13 Edge graph | 1,665 | 1,933 |

## The questions for the decision

For each item that is held and not read, the question is: must it stay in memory, or can the step that reads it read it from
storage (the area file, the spill files or a written table)?

1. **Raw part (871 MiB).** It is read in step 1.4 and held to the end. The network does not read it. Only steps 5.2 (bus routes),
   5.6 (building faces) and 7 (`raw.*`) read parts of it, and each part is in the area file.
2. **Edges read back (778 MiB).** The edges are already on disk in the spill files, which can be read more than one time.
   Each later step can read only its fields from them, and the level edges, lane edges and street segments need not be held.
3. **Mode ways (318 MiB).** They are held through the network and to the end of the mode.
4. **Ways and node positions (385 MiB).** The network reads them in each pass. After the network, the area tables, the lane
   profile and write raw read them. They are also in the area file, but the build changes some tags of the ways.
5. **Turn restrictions and building grid (2 MiB).** Small, but held to the end.

## Open proposals

- [PR 130](https://github.com/Khoshkhah/duckOSM/pull/130) (not merged): each step runs when its data is ready, and each item goes
  after the last step that reads it. The files are equal. Kentucky: 5.3 area tables 3,220 → 1,754, 5.5 lane profile 3,228 → 2,530.
