# Browser and native builds

duckOSM builds an area in two places, with the same Rust engine. Both give the same rows and the same `edge_id`.

| | Native | Browser |
|---|---|---|
| Program | `duckosm` on your computer | [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/) (WebAssembly) |
| Storage | the disk | the storage of the browser (OPFS) |
| Memory | the RAM of the computer | the tab: at most 4 GB for each WebAssembly module |
| Result | a `.duckdb` file | tables in the page, and a `.duckdb` file when you export |

The steps are the same in both: [How a build works](build.md).

## By tiles

For a large area, both build by map tiles. [pbfcut](https://github.com/Khoshkhah/pbfcut) cuts the OSM file into tiles of
zoom 8, each tile builds alone, and a union joins the tiles. So the memory does not grow with the area: France needs
less than 3 GB natively. The tables are equal to those of one build.

```bash
duckosm tiles france.osm.pbf --boundary france.geojson -o out -- --modes driving
```

`duckosm FILE` makes one build, with no tiles. The local dashboard `duckosm studio` builds by tiles.

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB), headless Chromium for the browser. By tiles. "3" is driving, walking and cycling.

| Area (modes) | Native: time, memory, disk | Browser: time, memory, storage |
|---|---|---|
| Monaco (3) | 0.3 s, 71 MB, 4 MB | 2.7 s, 65 MB, 41 MB |
| Tartu (3) | 1.4 s, 127 MB, 37 MB | 4.0 s, 168 MB, 112 MB |
| Kentucky (driving) | 12.3 s, 0.8 GB, 1.2 GB | 24.6 s, 1.5 GB, 2.1 GB |
| Kentucky (3) | 32.9 s, 1.4 GB, 3.6 GB | 55.8 s, 2.0 GB, 4.6 GB |
| France (driving) | **3 min 36 s, 1.6 GB**, 11.2 GB | **5 min 16 s, 5.7 GB**, 14.9 GB |
| France (3), an estimate from Kentucky | about 10 min, 2.6 GB, 35 GB | about 12 min, 7.5 GB, 33 GB |

- **The disk is the limit, not the memory.** France with three modes needs about 35 GB of disk. It was not built: the
  Mac mini has too little free disk.
- **One build without tiles:** France (driving) needs more than 8 GB natively ([Memory at each step](memory-by-step.md)), and it does not fit in the 4 GB of the browser.
- **The browser's files are about 1.2 times larger:** the browser compresses with Snappy, because it has no zstd.

The memory of each step: [Memory at each step](memory-by-step.md).
