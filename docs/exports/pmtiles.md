# Vector tiles (PMTiles)

`duckosm quack --pmtiles` makes a **base map in one file**: vector tiles of the Shortbread schema, zoom 0 to 14, in one
PMTiles file, **with your OSM fixes**. MapLibre, OpenLayers and QGIS show the file with no tile server. Any Shortbread
style draws it, for example the styles of [VersaTiles](https://versatiles.org).

| Reference and format | |
|---|---|
| Format | one [PMTiles v3](https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md) file of [Mapbox Vector Tiles 2.1](https://github.com/mapbox/vector-tile-spec/tree/master/2.1), zoom 0 to 14 (Web Mercator) |
| Zoom | 0 to 14, as the Shortbread schema says. Zoom 14 holds the full detail: one tile unit is about 0.6 m. Above zoom 14, the map enlarges the tiles of zoom 14, and the lines stay sharp. Each more zoom would have 4 times the tiles, with no new data |
| Schema | [Shortbread 1.0](https://shortbread-tiles.org/schema/1.0/): the layers, their attributes and the first zoom of each feature |
| Reference tool | [Tilemaker](https://tilemaker.org) 3.2.0 with Geofabrik's [Shortbread configuration](https://github.com/shortbread-tiles/shortbread-tilemaker) |
| Check | each tile equals the golden file of the Monaco sample, also by tiles; each tile is checked against the specifications; each file passes `pmtiles verify`. [How each output is checked](../concepts/checks.md) |

```bash
duckosm quack monaco.osm.pbf --pmtiles -o out --name monaco                       # -> out/monaco.pmtiles
duckosm quack monaco.osm.pbf --pmtiles -o out --name monaco --fixes osm_fixes.yaml
```

The options are those of the [QuackOSM table](../concepts/quackosm.md): `--fixes` and `--tile-zoom` (less memory, for a
large area). In the browser: the [quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) of
duckOSM Studio, output **Vector tiles**. The page shows a map of the file.

These tiles are an export for other tools. The duckOSM maps use other tiles: roadstyle makes the tiles of the road
network, with `edge_id`.

## The layers

The layers and attributes of Shortbread 1.0. Most layers have the attribute `kind`, the class of the feature. A label
layer has `name`, `name_en` and `name_de`.

| Layer | What goes in |
|---|---|
| `land`, `sites` | land use and land cover; parking, schools, hospitals |
| `water_polygons`, `water_lines`, and their `_labels` | lakes, rivers, streams, canals |
| `dam_polygons`, `dam_lines`, `pier_polygons`, `pier_lines` | dams and piers |
| `boundaries`, `boundary_labels` | country and state borders (`admin_level` 2 to 4) |
| `buildings`, `addresses` | buildings, house numbers |
| `streets`, `street_polygons`, `street_labels`, `streets_polygons_labels` | roads and railways, pedestrian areas, their names. `streets` also has `link`, `rail`, `tunnel`, `bridge`, `layer`, `oneway`, `surface`, `bicycle`, `horse` |
| `bridges`, `ferries`, `aerialways` | bridge areas, ferry lines, cable cars |
| `public_transport`, `pois` | stops and stations; shops, restaurants, amenities |
| `place_labels` | cities, towns, villages, suburbs |

All attributes and the first zoom of each kind: [Shortbread 1.0](https://shortbread-tiles.org/schema/1.0/).

Not in the file:

| Layer or attribute | Why |
|---|---|
| `ocean` | Shortbread makes it from the water polygons of [osmdata.openstreetmap.de](https://osmdata.openstreetmap.de/data/water-polygons.html), not from the OSM file |
| `name_*` of other languages | Shortbread 1.0 has only `name_en` and `name_de` |
| the `boundary_labels` of a country | only when its boundary relation is complete in the file |

## Speed and size

Mac mini (Apple M6, 12 cores, 16 GB). 1 GB = 10⁹ bytes. The memory is the peak footprint (`/usr/bin/time -l`), and
the disk is the peak of the output and the temporary files.

| Area (source .osm.pbf) | | duckOSM | Tilemaker 3.2.0 |
|---|---|---|---|
| Monaco (0.6 MB) | time | 0.06 s | 1.5 s |
| | memory | 0.05 GB | 1.5 GB |
| | file | 0.2 MB | |
| | tiles | 59 | |
| Kentucky (148 MB) | time | 5.5 s | 5.0 s |
| | memory | 1.13 GB | 2.9 GB |
| | file | 101 MB | 101 MB |
| | tiles | 41 640 | 71 791 |
| France (5.1 GB) | time | 3 min 48 s | 3 min 32 s |
| | memory | 2.7 GB | 3.1 GB |
| | file | 2.95 GB | 2.91 GB |
| | tiles | 308 154 | 623 176 |
| | disk | 18.0 GB | 18.4 GB |

On France, duckOSM runs with `--tile-zoom 8`, and Tilemaker with `--store` (its nodes and ways in a temporary folder,
not in memory). Planetiler: not run, because the Mac mini has no Java runtime. Planetiler also writes the OpenMapTiles
schema, not Shortbread.

## Against Tilemaker

On Monaco, the two files have the same features at zoom 14, with these differences:

| Difference | Cause |
|---|---|
| At low zoom, duckOSM has more line parts | Tilemaker joins the lines that touch |
| duckOSM also labels railways, runways and taxiways | Shortbread 1.0 lists them. Tilemaker labels only `highway` |
| Tilemaker labels pedestrian areas with no name, and writes `historic` as `man_made` | faults of Tilemaker's configuration |
| `public_transport`: duckOSM uses the first zoom of each kind (bus stop 14, station 13) | Shortbread 1.0. Tilemaker puts all of them at zoom 11 |
| `streets`: Tilemaker writes `foot` and `motorcar` | Tilemaker follows a later draft of the schema |

The comparison layer by layer:
[docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md#against-tilemaker), "PMTiles".
