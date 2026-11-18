# Speed on France

France: 5.1 GB `.osm.pbf`. Mac mini (12 cores, 16 GB). Time and the highest memory of each job.

| Job | duckOSM, native | duckOSM, browser | Other tool | Details |
|---|---|---|---|---|
| Road network, driving | 3 min 36 s, 1.6 GB | 5 min 16 s, 5.7 GB | | [Browser and native builds](../concepts/browser-native.md#results) |
| OSM features table (`quack`) | 2 min 9 s, 3.8 GB | 4 min 54 s, 6.9 GB | QuackOSM: 9 min 30 s, 9.8 GB | [duckOSM and QuackOSM](../concepts/quackosm.md#results) |
| GDAL's layers, GeoParquet (`quack --gdal`) | 3 min 37 s, 1.0 GB | 5 min 23 s | `ogr2ogr`: 14 min 18 s, 1.7 GB | [duckOSM and GDAL](../concepts/gdal.md#results) |

- The rows are the same as those of the other tool. Each page above tells how they are compared.
- Each page above also gives Kentucky, the disk, the output sizes and the estimates.
- The browser is headless Chromium. Its memory is the WebAssembly memory of all workers.
- Road networks of other tools: [duckOSM and OSMnx](../concepts/osmnx.md).
