# Browser and native builds

duckOSM builds an area in two versions, with the same Rust engine and the same rows.

| | Native | Browser |
|---|---|---|
| Program | `duckosm` on your computer | [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/) in the browser (WebAssembly) |
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

Both versions build an area by tiles: [pbfcut](https://github.com/Khoshkhah/pbfcut) cuts the region file into tiles, each
tile builds alone, and a union joins the tiles. The memory does not grow with the area. Natively, `duckosm tiles` builds by
tiles (`duckosm FILE` is one build). The local dashboard (`duckosm studio`) runs `duckosm tiles` for its build:

```bash
duckosm tiles france.osm.pbf --boundary france.geojson -o out -- --modes driving
```

Mac mini (12 cores, 16 GB), headless Chromium for the browser, 2026-11-17: the levels `solve`, tiles of zoom 8. The tables
of the build by tiles are equal to those of one build. Time, memory, the most disk at one time (the browser: its storage),
and the output.

| Area (modes) | Native | Browser |
|---|---|---|
| Monaco (3) | 0.3 s, 71 MB, 4 MB disk, 4 MB output | 2.7 s, 65 MB, 41 MB storage |
| Tartu (3) | 1.4 s, 127 MB, 37 MB disk, 24 MB output | 4.0 s, 168 MB, 112 MB storage |
| Kentucky (driving) | 12.3 s, 0.8 GB, 1.2 GB disk, 0.7 GB output | 24.6 s, 1.5 GB, 2.1 GB storage |
| Kentucky (3) | 32.9 s, 1.4 GB, 3.6 GB disk, 2.1 GB output | 55.8 s, 2.0 GB, 4.6 GB storage, 2.5 GB after the build |
| France (driving) | 3 min 36 s, 1.6 GB, 11.2 GB disk, 7.2 GB output | 5 min 16 s, 5.7 GB, 14.9 GB storage |
| France (3), **estimate from Kentucky** | about 10 min, 2.6 GB, 35 GB disk, 23 GB output | about 12 min, 7.5 GB, 33 GB storage, 27 GB after the build |

- The three modes are driving, walking and cycling. France with the three modes was not built: the Mac mini has too little free
  disk. Its row is France with driving times the ratio of Kentucky with three modes to Kentucky with driving (native: time 2.7,
  memory 1.65, disk 3.1, output 3.1; browser: time 2.2, memory 1.3, storage 2.2). After the build, the browser keeps the output
  times 1.2.
- France: 25.9 million edges (driving), 104 tiles. The memory of the browser is the WebAssembly memory of all workers at the
  same time.
- The browser's Parquet files are about 1.2 times those of the native program: they are compressed with snap, because zstd is
  a C library (Kentucky, driving, one build: 851 MB in WebAssembly, 704 MB natively). After a build, Studio saves the area in the
  browser (its saved areas): the files of the build move there, so the storage holds one copy of the tables.
- The peak of the disk is the sort of the levels, after the union. Kentucky with three modes: the tile files (2.1 GB natively)
  go while the union reads them, so the union ends at the output (2.1 GB). Then the sorted runs of the levels add 1.5 GB, and
  the peak is 3.6 GB (the browser: 2.5 GB of tables and 1.5 GB of runs, 4.6 GB of storage). The rows go straight into sorted
  runs, so one copy is on the disk at one time (the browser: 6.0 GB of storage before this change).
- One build, for comparison: Kentucky (3) 35.7 s and 3.2 GB natively, 64.5 s in the browser; France (driving) 4 min 31 s and
  8.1 GB natively, and it does not fit in the 4 GB of the browser. On a small area one build is faster by less than 1 s (Tartu:
  0.9 s natively, 3.3 s in the browser).

## The limits

- **Native:** the memory does not limit the area: France by tiles needs less than 3 GB. The disk does: about 35 GB for France
  with the three modes (an estimate).
- **Browser:** the same, with the browser's storage in place of the disk. Each WebAssembly module (each tile, the union)
  has its own 4 GB.
