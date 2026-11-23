# Geofabrik layers

Geofabrik's free shapefiles and GeoPackages (`<area>-latest-free.shp.zip`, `-free.gpkg.zip`) are among the most used
OSM GIS files. Geofabrik makes them only for its own regions. `duckosm quack --geofabrik` makes the **same 20 layers,
with the same rows**, from any `.osm.pbf` file: for any area, with your OSM fixes, and in the browser too.

| Reference and format | |
|---|---|
| Format | 20 layers, as one GeoPackage, shapefiles in one `.zip` file, or one GeoParquet file for each layer |
| Reference | Geofabrik's free files, and their description [OpenStreetMap Data in Layered GIS Format](https://download.geofabrik.de/osm-data-in-gis-formats-free.pdf) |
| Check | on Monaco, each row of each layer equals Geofabrik's own GeoPackage and shapefiles. [How each output is checked](../concepts/checks.md) |

```bash
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco                          # -> out/monaco_geofabrik.gpkg
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco -f gpkg,shp,geoparquet   # all three formats from one read
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco --fixes osm_fixes.yaml   # with your OSM fixes
```

The options are those of the [QuackOSM table](../concepts/quackosm.md): `-f` (`gpkg`, `shp`, `geoparquet`), `--fixes`
and `--tile-zoom`. In the browser: the [quack page](https://khoshkhah.github.io/duckosm-studio/quack.html)
of duckOSM Studio, output **Geofabrik layers**.

## The layers

The file names and table names are those of Geofabrik. The `_a_` layers hold the areas.

| Layer | What goes in | Columns after `osm_id`, `code`, `fclass` |
|---|---|---|
| `gis_osm_places_free`, `_a_` | `place=*`: city, town, suburb, island | `population`, `name` |
| `gis_osm_pois_free`, `_a_` | shops, restaurants, schools, benches | `name` |
| `gis_osm_pofw_free`, `_a_` | places of worship | `name` |
| `gis_osm_natural_free`, `_a_` | trees, peaks, springs, beaches | `name` |
| `gis_osm_traffic_free`, `_a_` | crossings, traffic signals, parking, piers | `name`, `calming` |
| `gis_osm_transport_free`, `_a_` | bus stops, stations, taxi, helipads | `name` |
| `gis_osm_roads_free` | `highway=*` lines | `name`, `ref`, `oneway`, `maxspeed`, `layer`, `bridge`, `tunnel` |
| `gis_osm_railways_free` | `railway=*` lines | `name`, `layer`, `bridge`, `tunnel` |
| `gis_osm_waterways_free` | rivers, streams, canals, drains | `width`, `name` |
| `gis_osm_buildings_a_free` | `building=*` | `name`, `type` |
| `gis_osm_landuse_a_free` | forest, park, residential, grass | `name` |
| `gis_osm_water_a_free` | lakes, reservoirs, river areas | `name` |
| `gis_osm_adminareas_a_free` | `boundary=administrative`, by `admin_level` | `name` |
| `gis_osm_protected_areas_a_free` | `boundary=protected_area`, `leisure=nature_reserve` | |

The rules (the tag, `fclass` and `code` of each class) are in one text file,
[geofabrik_rules.txt](https://github.com/Khoshkhah/duckOSM/blob/main/rust/duckosm-core/src/geofabrik_rules.txt).
Where Geofabrik's PDF and its files do not agree, duckOSM follows the files.

## OSM fixes

`--fixes` applies the fixes file of [Fix the OSM data](../guides/edit-osm-data.md) to the 20 layers, with the same
rules as the [QuackOSM table](../concepts/quackosm.md#osm-fixes). Geofabrik's files cannot have your fixes.

## Speed and size

Mac mini (Apple M6, 12 cores, 16 GB).

| | Monaco (0.7 MB `.osm.pbf`) | Kentucky (148 MB `.osm.pbf`) |
|---|---|---|
| Time, GeoPackage | 0.06 s | 5.9 s |
| Time, all three formats from one read | 0.09 s | 11.3 s |
| Memory | 0.02 GB | 0.29 to 0.40 GB |
| GeoPackage, shapefiles (zip), GeoParquet | 2.4, 0.8, 0.6 MB | 638, 294, 312 MB |

The differences between Geofabrik's PDF and its files, and the other measurements:
[docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md), "Geofabrik layers".
