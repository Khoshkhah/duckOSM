# Browser and native builds

duckOSM builds an area in two versions, with the same Rust engine and the same rows.

| | Native | Browser |
|---|---|---|
| Program | `duckosm-rust` on your computer | [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/) in the browser (WebAssembly) |
| Storage | the disk | the browser's storage (OPFS) |
| Memory | the RAM of the computer | the tab: at most 4 GB for each WebAssembly module |
| Result | a `.duckdb` file | tables in the page; a `.duckdb` file when you export |

## The steps

| Step | Does |
|---|---|
| 0. Cut | cuts the area from the region file into an area file |
| 1. Read | reads the ways, the coordinates and the turn restrictions |
| 2. Junctions | finds the junctions of all modes |
| 3. Network | makes the edges and the nodes of each mode |
| 4. Write | writes the network files |
| 5. More tables | area tables, levels, bus routes, building faces, `raw.*` |

Each step reads its data in blocks and writes its result to storage. Diagrams and the memory of each step:
[each step in detail](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/browser_native_steps.md).

## Results

Mac mini (12 cores, 16 GB), driving mode.

### Kentucky (one build)

| | Native | Browser |
|---|---|---|
| Time | 28.4 s | 42.4 s |
| Memory | 3.2 GB | 3.3 GB |

With the raw tables, the base-map layers and the building faces.

### France (by tiles)

| | Native | Browser |
|---|---|---|
| Time | 3 min 48 s | – ¹ |
| Memory | 1.6 GB | – ¹ |

France: 25.9 million edges, with the `.duckdb` file. 104 tiles of zoom 8.

¹ Studio does not build by tiles yet. One build of France does not fit in the 4 GB of the browser.

A large area builds tile by tile: [pbfcut](https://github.com/Khoshkhah/pbfcut) cuts the file into tiles, each tile
builds alone, and a union joins the tiles. The memory does not grow with the area.

```bash
duckosm-rust tiles france.osm.pbf --boundary france.geojson -o out -- --modes driving
```

## The limits

- **Native:** none for the area: France by tiles needs less than 2 GB.
- **Browser:** one build must fit in 4 GB, so the largest area is about the size of Kentucky.
