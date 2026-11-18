# duckOSM and Geofabrik

Geofabrik's free shapefiles and GeoPackages (`<area>-latest-free.shp.zip`, `-free.gpkg.zip`) are among the most used OSM
GIS files. Only Geofabrik makes them, with its own tool, and only for its own regions.
`duckosm quack --geofabrik` makes the **same 20 layers, with the same rows**, from any `.osm.pbf` file: any area or
boundary, a whole country, with your OSM fixes, also as GeoParquet, and also in the browser.

```bash
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco                         # out/monaco_geofabrik.gpkg
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco -f shp                  # out/monaco_geofabrik.shp.zip
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco -f geoparquet           # out/monaco_geofabrik_<layer>.parquet
duckosm quack monaco.osm.pbf --geofabrik -o out --name monaco -f gpkg,shp,geoparquet  # all three from one read
duckosm quack france.osm.pbf --geofabrik -o out --name france --tile-zoom 8           # tile by tile (for a country)
duckosm quack monaco.osm.pbf --geofabrik -o out --fixes osm_fixes.yaml                # with the OSM fixes of Edit OSM data
```

In the browser: the [quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) of duckOSM Studio, output
**Geofabrik layers**, format GeoPackage, shapefiles (one `.zip` file) or GeoParquet.

## Results

Mac mini (12 cores, 16 GB). 1 MB = 10⁶ bytes; the memory is the peak footprint.

| | Monaco (0.7 MB) | Kentucky (148 MB) | Kentucky, `--tile-zoom 8` |
|---|---|---|---|
| Time, GeoPackage | 0.06 s | 5.9 s | 7.1 s |
| Time, shapefiles | 0.07 s | 9.9 s | |
| Time, GeoParquet | 0.06 s | 5.4 s | |
| Time, all three from one read | 0.09 s | 11.3 s | |
| Memory | 0.02 GB | 0.29 to 0.40 GB | 0.23 GB |
| Files: GeoPackage, shapefiles (zip), GeoParquet | 2.4, 0.8, 0.6 MB | 638, 294, 312 MB | |

Kentucky has 1 796 357 rows in the 20 layers. In the browser, Monaco takes 0.6 to 0.9 s for each format.

## The layers

The same file names and table names as Geofabrik's files. A shapefile has the name of the table and `_1`
(`gis_osm_roads_free_1.shp`); a layer larger than 2 GB continues in `_2`, as at Geofabrik.

| Layer | What goes in | Columns after `osm_id`, `code`, `fclass` |
|---|---|---|
| `gis_osm_places_free`, `_a_` | `place=*`: city, town, suburb, island, ... | `population`, `name` |
| `gis_osm_pois_free`, `_a_` | shops, restaurants, schools, benches, ... | `name` |
| `gis_osm_pofw_free`, `_a_` | places of worship by religion and denomination | `name` |
| `gis_osm_natural_free`, `_a_` | trees, peaks, springs, beaches, ... | `name` |
| `gis_osm_traffic_free`, `_a_` | crossings, traffic signals, parking, piers, ... | `name`, `calming` |
| `gis_osm_transport_free`, `_a_` | bus stops, stations, taxi, helipads, ... | `name` |
| `gis_osm_roads_free` | `highway=*` lines | `name`, `ref`, `oneway`, `maxspeed`, `layer`, `bridge`, `tunnel` |
| `gis_osm_railways_free` | `railway=*` lines | `name`, `layer`, `bridge`, `tunnel` |
| `gis_osm_waterways_free` | rivers, streams, canals, drains | `width`, `name` |
| `gis_osm_buildings_a_free` | `building=*` | `name`, `type` |
| `gis_osm_landuse_a_free` | forest, park, residential, grass, ... | `name` |
| `gis_osm_water_a_free` | lakes, reservoirs, river areas, ... | `name` |
| `gis_osm_adminareas_a_free` | `boundary=administrative` by `admin_level` | `name` |
| `gis_osm_protected_areas_a_free` | `boundary=protected_area`, `leisure=nature_reserve` | |

The `_a_` layers have the areas: the closed ways and the complete multipolygons. A node goes into a point layer, a way
into a line layer. The rules (tag, `fclass`, `code`) are in one file of plain lines,
[geofabrik_rules.txt](https://github.com/Khoshkhah/duckOSM/blob/main/rust/duckosm-core/src/geofabrik_rules.txt).

## The rows are the same

Monaco, 2026-11-17: the `.osm.pbf` file and the free GeoPackage and shapefiles from Geofabrik, downloaded on the same
day. Each layer has the same rows: the same `osm_id`, `code`, `fclass`, all column values, and the same geometry
(`ST_Equals`).

| Check | Rows | Result |
|---|---|---|
| GeoPackage, one read | 8 732 in 20 layers | equal |
| Shapefiles, one read | 8 732 | equal |
| GeoPackage, tiles of zoom 12 | 8 732 | equal |
| Each tile alone, then the join (the browser) | 8 732 | equal |
| The page in the browser: GeoPackage, shapefiles, GeoParquet | all | equal to the native files |

One difference, in the geometry only: relation 11384697 (a building) has two inner rings with one edge in common.
Geofabrik joins them into one hole; duckOSM keeps the two rings. The area is the same (`ST_Equals` is true).

The test `duckosm-build/tests/quack_geofabrik_monaco.rs` compares with a golden file that comes from Geofabrik's own
GeoPackage, not from duckOSM.

## Where Geofabrik's PDF and its files do not agree

Geofabrik describes the layers in
[OpenStreetMap Data in Layered GIS Format](https://download.geofabrik.de/osm-data-in-gis-formats-free.pdf). Its files
are not always as the PDF says. duckOSM follows the files:

| The PDF | The files |
|---|---|
| parking: `parking_surface`, `parking_multistorey`, ..., not `access=private` | only `parking`, also with `access=private` |
| `crossing`, `market_place`, `art`, `taxi_rank` | `pedestrian_crossing`, `marketplace`, `artwork`, `taxi` |
| no `fitness_centre`, `sports_hall`, `track` in `pois` | these three are in `pois` |
| `tree`, `street_lamp`, `mobile_phone_shop`, `car_dealership` are not in the free files | they are in the free files |
| `scrub` is `landuse=scrub` | `natural=scrub` |
| guide posts and maps are `tourist_guidepost`, `tourist_map` | `tourist_info` |
| bus stops include `public_transport=stop_position` | only `highway=bus_stop` |
| one feature class for each object | one row for each key: a cafe with `shop=bakery` is a `bakery` row and a `cafe` row |
| `railways` has no `name` | it has `name` |
| no `places_a` layer | `places_a` is in the files |

A class that Monaco does not have follows Geofabrik's 0.8 standard, without the classes that the PDF calls "not in the
free files". Not checked: names such as "fixme" (the PDF says that they become empty), `maxspeed` in mph, and the oneway
of motorways.

## OSM fixes

`--fixes FILE` applies the OSM fixes file of `duckosm build --fixes`, as for [GDAL's layers](../concepts/gdal.md#osm-fixes).

More details: [docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md), "Geofabrik layers".
