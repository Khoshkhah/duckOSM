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
| 0. Cut | cuts the region file into the map tiles of the area (zoom 8), each tile into a file of its own |
| 1. Read | reads the ways, the coordinates and the turn restrictions |
| 2. Junctions | finds the junctions of all modes |
| 3. Network | makes the edges and the nodes of each mode |
| 4. Write | writes the network files |
| 5. More tables | area tables, levels, bus routes, building faces, `raw.*` |

Each step reads its data in blocks and writes its result to storage. Diagrams and the memory of each step:
[each step in detail](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/browser_native_steps.md).

## Results

Both versions build every area by tiles (since 2026-11-17): [pbfcut](https://github.com/Khoshkhah/pbfcut) cuts the region
file into tiles, each tile builds alone, and a union joins the tiles. The memory does not grow with the area. The local
dashboard (`duckosm-rust studio`) runs `duckosm-rust tiles` for its build:

```bash
duckosm-rust tiles france.osm.pbf --boundary france.geojson -o out -- --modes driving
```

Mac mini (12 cores, 16 GB), headless Chromium for the browser. The three modes (France: driving only), the levels `solve`,
tiles of zoom 8. The tables of the build by tiles are equal to those of one build.

| Area | Native | Browser |
|---|---|---|
| Monaco | 0.27 s, 75 MB | 2.6 s, 66 MB |
| Tartu | 1.4 s, 125 MB | 3.9 s, 180 MB |
| Kentucky | 31.7 s, 1.3 GB | 55.6 s, 2.1 GB |
| France | 3 min 34 s, 1.6 GB | 4 min 56 s, 5.7 GB |

- France: 25.9 million edges, 104 tiles. In the browser, 16.2 GB of browser storage at most; the memory is the WebAssembly
  memory of all workers at the same time.
- One build, for comparison: Kentucky 35.7 s and 3.2 GB natively, 64.5 s in the browser; France 4 min 31 s and 8.1 GB
  natively, and it does not fit in the 4 GB of the browser. On a small area one build is faster by less than 1 s (Tartu:
  0.91 s natively, 3.3 s in the browser).

## The limits

- **Native:** the memory does not limit the area: France by tiles needs less than 2 GB. The disk does: France with the three
  modes needs more than 28 GB of disk for the files of the tiles.
- **Browser:** the same, with the browser's storage in place of the disk. Each WebAssembly module (each tile, the union)
  has its own 4 GB.
