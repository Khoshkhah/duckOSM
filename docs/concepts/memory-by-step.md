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

## The data that stays in memory between steps

| Data | Size (MiB) | Made in | Freed after | Read by |
|---|---|---|---|---|
| Ways: the highway ways as read, with the fixes | 187 | read ways | the end of the build | network, area tables, lane profile, write raw. The sidewalk rule and the arrows of the fixes change their tags. |
| Node positions | 198 | read nodes | lane profile | network, area tables |
| Turn restrictions | 1 | read restrictions | the end of the build (never freed) | network (9b) |
| Raw part: the elements that the fixes touch, the route relations, the buildings with their nodes | 871 | read the raw part | the end of the build | bus routes (route relations), building grid and building faces (buildings), write raw (touched elements) |
| Building grid | 1 | before the network | the end of the build | street segments (end of the mode) |
| Junctions | 34 | global junctions | lane profile | network, area tables |
| Mode ways (`net.ways`): the ways of the mode after the road filter | 318 | network, prep | the end of the mode | network steps 1, 5 and 9b, the `ways` tables, the arrows of the fixes |
| Network nodes (`net.nodes`) | 25 | network, step 8 | lane profile (kept as the nodes of the modes) | the network files, area tables |
| Buffers of the edge spill files (the edges are on disk) | 29 | network | the end of the mode | the network files |
| Edges and private edges, read back from the spill files | 708 + 70 | read the edges | the end of the mode (they move into the level edges) | street segments, lane edges, bus routes, H3, elevation |
| Street segments | 202 | the end of the mode | building faces | building faces |
| Lane edges (a copy of fields of the edges) | 237 | the end of the mode | lane profile | lane profile |
| Level edges (the edges, moved) | 776 | the end of the mode | lane profile | levels |

## Memory at each step

"Held" is the data from earlier steps that is in memory when the step starts. "Not read" is the part of it that the step
does not read.

| Step | Live at start | Peak | Held from earlier steps | The step reads | Held and not read |
|---|---|---|---|---|---|
| Cut | 0 | 356 | — | region file, boundary | — |
| Read ways | 0 | 253 | — | area file | — |
| Read nodes | 187 | 619 | ways | ways, area file | — |
| Read restrictions | 385 | 404 | ways, node positions | area file | ways, node positions |
| Read the raw part | 386 | 1,423 | ways, node positions, restrictions | area file | ways, node positions, restrictions |
| Global junctions | 1,257 | 1,455 | + raw part | ways | node positions, restrictions, raw part: 1,070 |
| Network, steps 1 to 10 (next table) | 1,292 | 1,793 | ways, node positions, restrictions, junctions, raw part, building grid | ways, node positions, restrictions, junctions | raw part, building grid: 872 |
| Write the network files | 1,665 | 2,086 | + mode ways, network nodes, spill buffers | spill files, mode ways, network nodes | ways, node positions, junctions, raw part, grid, restrictions: 1,292 |
| Read the edges back | 1,665 | 2,443 | the same | spill files | the same: 1,292 and more |
| Bus routes, street segments, lane edges, level edges (the end of the mode) | 2,413 | 2,755 | + edges 778 | edges, raw part (route relations), building grid | ways, node positions, junctions, restrictions, most of the raw part |
| Area tables | 2,532 | 3,220 | ways, node positions, junctions, network nodes, raw part, grid, restrictions, street segments, lane edges, level edges | ways, node positions, network nodes, junctions: 444 | level edges, lane edges, street segments, raw part, grid, restrictions: 2,088 |
| Levels | 2,532 | 3,138 | the same | level edges: 776 | all the rest: 1,756 |
| Lane profile | 2,532 | 3,228 | the same | lane edges, ways: 424 | level edges and all the rest: 2,108 |
| Building faces | 1,263 | 2,817 | ways, raw part, street segments, grid, restrictions | buildings of the raw part, street segments | ways, restrictions, grid; the raw part that is not buildings (not measured) |
| Write raw | 1,060 | 1,796 | ways, raw part, restrictions | area file, touched elements of the raw part, ways | the buildings of the raw part (not measured), restrictions |

The three highest peaks are lane profile (3,228), area tables (3,220) and levels (3,138). In each of them, more than 1.7 GB
is in memory that the step does not read.

### The network substeps

The build writes a mark at the start of each substep; each line is the part that ends at that mark. During all substeps
the build holds 1,292 MiB from the steps before (the table above) and the mode ways (318 MiB). The edges are in spill files
on disk (stages B2 to B4 of [network_stream.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/network_stream.md)).

| Part that ends at the mark | Live at the end | Peak |
|---|---|---|
| Prep: the mode filter (makes the mode ways) | 1,647 | 1,793 |
| Steps 1 and 2: cut the ways into forward edges | 1,642 | 1,683 |
| Step 2b: merge the chains | 1,642 | 1,750 |
| Steps 3 to 7: split the arcs, reverse edges, ids, length | 1,643 | 1,769 |
| Step 8: the nodes | 1,667 | 1,732 |
| Step 8b: the path connectors | 1,667 | 1,667 |
| Steps 8b2 to 10: speeds, private roads, largest part | 1,671 | 1,678 |
| The sort of the edges by `edge_id` | 1,667 | 1,778 |
| Step 9b: turn restrictions | 1,667 | 1,683 |
| Step 9b2: the edge graph, the turn permissions | 1,665 | 1,933 |

## The questions for the decision

For each item that is held and not read, the question is: must it stay in memory, or can the step that reads it read it from
storage (the area file, the spill files or a written table)?

1. **Raw part (871 MiB).** It is read before the network and held to the end. The network does not read it. Only bus routes,
   the building faces and write raw read parts of it, and each part is in the area file.
2. **Edges read back (778 MiB).** The edges are already on disk in the spill files, which can be read more than one time.
   Each later step can read only its fields from them, and the level edges, lane edges and street segments need not be held.
3. **Mode ways (318 MiB).** They are held through the network and to the end of the mode.
4. **Ways and node positions (385 MiB).** The network reads them in each pass. After the network, the area tables, the lane
   profile and write raw read them. They are also in the area file, but the build changes some tags of the ways.
5. **Turn restrictions and building grid (2 MiB).** Small, but held to the end.

## Open proposals

- [PR 130](https://github.com/Khoshkhah/duckOSM/pull/130) (not merged): each step runs when its data is ready, and each item goes
  after the last step that reads it. The files are equal. Kentucky: area tables 3,220 → 1,754, lane profile 3,228 → 2,530.
