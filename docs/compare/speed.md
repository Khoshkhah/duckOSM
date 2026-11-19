# Speed on France

France: 5.1 GB `.osm.pbf`. Mac mini (Apple M6, 12 cores, 16 GB). The time and the peak memory of each job. France is the
driving network only.

| Job | duckOSM | duckOSM in the browser | Other tool | Details |
|---|---|---|---|---|
| Road network, by tiles | **3 min 36 s, 1.6 GB** | 5 min 16 s, 5.7 GB | | [Browser and native builds](../concepts/browser-native.md#speed-and-memory) |
| OSM features table (`quack`) | **2 min 30 s, 3.8 GB** | 5 min 40 s, 6.9 GB | QuackOSM: 16 min 38 s, 9.8 GB | [QuackOSM table](../concepts/quackosm.md#speed-and-memory) |
| GDAL's layers, GeoParquet (`quack --gdal`) | **3 min 37 s, 1.0 GB** | 5 min 23 s | `ogr2ogr`: 14 min 18 s, 1.7 GB | [GDAL's OSM layers](../concepts/gdal.md#speed-and-memory) |
| OSM file, cut (`cut`) | 35 s, 9.7 GB | | | [The OSM file](../exports/osm-file.md) |
| GIS files, GeoParquet (`export-gis`) | 6.9 s, 7.4 GB | | | [GeoPackage, GeoParquet and shapefile](../exports/gis.md#speed-and-memory) |
| MATSim (`matsim`) | 46 s, 10.0 GB | | | [MATSim](../exports/matsim.md#speed-and-memory) |

- **Not run on France:** the other exports need more free disk than the Mac mini has, or they stop on Kentucky
  (GMNS, and the lane level outputs that read it). Each export page gives its Monaco and Kentucky numbers.
- **In work:** the memory of `cut`, `export-gis` and `matsim` (on Kentucky, `cut` goes from 492 to 220 MB).
- The browser is headless Chromium. Its memory is the WebAssembly memory of all workers.
