# Memory at each step of the build

Each step writes its tables to storage. A later step reads only what it needs. This page shows the heap of each step in
one build (`duckosm FILE`, not by tiles). The build by tiles and its totals: [Browser and native builds](browser-native.md).

Driving, `--building-faces --levels solve`, on the Mac mini (Apple M6, 12 cores, 16 GB). Kentucky also with `--raw`. The step
numbers are those of [Browser and native builds](browser-native.md#the-steps).

## Heap peak of each step

MiB. "Held" is the data from earlier steps at the start of the step.

| Step | Kentucky: held | Kentucky: peak | France: held | France: peak |
|---|---|---|---|---|
| 0. Cut | 0 | 356 | 0 | 8,179 |
| 1. Read the ways | 0 | 253 | 0 | 3,346 |
| 1. Read the node positions | 187 | 619 | 2,619 | 8,433 |
| 1. Read the turn restrictions | 385 | 414 | 5,001 | 5,069 |
| 1. Read the raw part | 397 | 422 | 5,049 | 5,919 |
| 2. Junctions | 404 | 601 | 5,868 | 8,541 |
| 3. Network | 438 | 618 | 6,412 | 7,622 |
| 4. Write | 413 | 495 | 5,714 | 5,798 |
| 5. Bus routes | 413 | 447 | 5,714 | 10,028 |
| 5. Area tables | 413 | 413 | 5,714 | 5,714 |
| 5. Lane profile | 299 | 750 | 3,927 | 7,700 |
| 5. Levels | 88 | 227 | 462 | not measured (below 4,882) |
| 5. Building faces | 88 | 367 | 462 | 855 |
| 5. `raw.*` | 88 | 192 | not built | not built |

## What grows with the area

In one build, these steps hold a list of the whole area. For a large area, build by tiles: its memory does not grow with
the area ([the limits](browser-native.md#the-limits)).

| Step | France peak | What is in memory |
|---|---|---|
| 0. Cut | 8.2 GB | the ids of all nodes inside the boundary, and the kept ways |
| 1. Read the node positions | 8.4 GB | all ways and the positions of their nodes |
| 2. Junctions | 8.5 GB | the ways, and the count of the ways at each node |
| 3. Network | 7.6 GB | the ways, the junctions, and the mode ways |
| 5. Bus routes | 10.0 GB | the route relations and the lines of their member ways |
| 5. Lane profile | 7.7 GB | the lane fields of all edges |

The levels, the building faces and `raw.*` do not hold a list of the whole area.

What stays between steps, the network substeps and the join of `raw.*`:
[memory in detail](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/memory_by_step_details.md).
