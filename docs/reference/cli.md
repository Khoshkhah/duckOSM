# Command line

Every `duckosm` command and its options. `duckosm <command> --help` prints the same list;
`duckosm --version` prints the version. `-h` is short for `--help` everywhere.

| Group | Commands |
|---|---|
| [Build and areas](#build-and-areas) | `duckosm FILE`, `tiles`, `get`, `cut`, `build`, `init-config`, `boundary`, `clip-pbf`, `extract` |
| [OSM data](#osm-data) | `quack`, `quack-tiles`, `quack-union` |
| [Query and inspect](#query-and-inspect) | `info`, `way` |
| [Maps, pages and routing](#maps-and-routing) | `viz`, `levels`, `levels-edit`, `edit`, `studio`, `ask`, `bus`, `route-map`, `route`, `route-points`, `directions`, `route-multimodal`, `route-multimodal-points`, `features-check` |
| [Exports](#exports) | `sumo`, `sumo-map`, `matsim`, `matsim-lanes`, `gmns`, `gmns-map`, `gmns-viz`, `export-gis`, `gis-debug`, `export-graph` |
| [Experimental and lane-level](#experimental-and-lane-level) | `opendrive`, `lanelet2`, `railml`, `lane-graph`, `route-lanes` |
| [Enrich a built database](#enrich-a-built-database) | `elevation`, `admin`, `multimodal` |

`DB` is a built duckOSM database; `GMNS_DB` is a database written by [`gmns`](#gmns). Commands that
enrich a database (`elevation`, `admin`, `multimodal`) change it in place.

## Build and areas

### `duckosm FILE`

The build, all in Rust: the networks of an area from an `.osm.pbf` file, into `<out>/<name>.duckdb` and GeoParquet files.
Guide: [Build a network](../guides/build.md). Its steps: [How a build works](../concepts/build.md).

```text
duckosm [OPTIONS] [PBF]
```

| Option | Does | Default |
|---|---|---|
| `PBF` | the input `.osm.pbf` file | |
| `-c`, `--config FILE` | a [config file](configuration.md): the input, the output, the modes and the options | |
| `--boundary FILE` | the [boundary](boundary.md): the file is cut to it first | the whole file |
| `-o`, `--out DIR` | the output folder | `.` |
| `--name NAME` | the name of the area: the file is `<out>/<name>.duckdb` | `area` |
| `-f`, `--format` | `duckdb`, `geoparquet` or both | both |
| `-m`, `--modes` | `driving`, `walking`, `cycling`, comma separated | `driving` |
| `--fixes FILE` | an [OSM fixes file](../guides/edit-osm-data.md#the-fixes-file) | |
| `--features` | the [base-map layers](features.md); needs `--boundary` | off |
| `--sea` | with `--features`: also `features.ocean` from Overture Maps | off |
| `--building-faces` | the [walls of the buildings](../concepts/building-walls.md); needs `--boundary` | off |
| `--levels METHOD` | the [drawing order](../guides/drawing-order.md): `tags`, `solve` or `none` | `tags` |
| `--dem FILE` | a DEM to sample at each node and edge vertex, with `--dem-product`, `--dem-vertical-datum`, `--dem-license` or `--dem-not-reference` ([the ground reference](../concepts/ground-reference.md)) | |
| `--h3-resolution RES` | the H3 columns at this resolution, 0 to 15 | |
| `--raw` | with `--boundary`: also write `raw.*` | off |
| `--cut-pbf`, `--cut-osm` | with `--boundary`: also write the cut area as `cut.osm.pbf` or `cut.osm` | off |
| `--no-graph`, `--no-restrictions`, `--no-speeds`, `--no-costs`, `--no-merge-segments`, `--no-functional-types`, `--no-bus-routes`, `--no-ways-tables` | turn off one step | |
| `--no-connect-paths`, `--snap-m M` | the [connection of dangling paths](../concepts/cleanup.md), and its distance | on, 10 m |
| `--timing` | print the time of each step | |

`duckosm FILE --help` lists all options.

### `tiles`

Builds a large area by map tiles: one read of the region file cuts it into the tiles of the boundary, each tile builds
alone, and a union joins them. The memory does not grow with the area. [Browser and native builds](../concepts/browser-native.md#by-tiles).

```text
duckosm tiles [OPTIONS] [REGION] [-- BUILD_OPTIONS...]
```

| Option | Does | Default |
|---|---|---|
| `REGION` | the region file, sorted by type and id | |
| `--boundary FILE` | the boundary of the area | |
| `--config FILE` | a config file in place of `REGION` and `--boundary` | |
| `-o`, `--out DIR` | the output folder | |
| `--zoom Z` | the zoom of the tiles | `8` |
| `--memory SIZE` | the memory for the tile builds at the same time, for example `4G` | |
| `--jobs N`, `--tile-threads N` | the tiles that build at the same time, and the threads of each | the cores / 2, `2` |
| `--keep-tiles` | keep the files of each tile | |
| `-- BUILD_OPTIONS` | the options of the build of each tile, as for `duckosm FILE`, for example `-- --modes driving` | |

### `get`

Finds the boundary of an area, downloads the smallest Geofabrik extract that holds it (to a cache folder), and cuts
the extract to the boundary. Writes `<name>.osm.pbf` and `<name>.geojson`. Guide: [The OSM file](../exports/osm-file.md).

```text
duckosm get [OPTIONS] [PLACE]
```

| Option | Does | Default |
|---|---|---|
| `PLACE` | the name of the area, for Nominatim | give one of `PLACE`, `--osm-id`, `--bbox`, `--h3-cell`, `--boundary` |
| `--osm-id ID` | one OSM boundary relation | |
| `--bbox W,S,E,N` | a box in degrees | |
| `--h3-cell CELL` | an H3 cell | |
| `--boundary FILE` | a GeoJSON [boundary file](boundary.md) | |
| `-o`, `--out DIR` | the output folder | `.` |
| `--name NAME` | the name of the two files | from the place, the file or the cell; `area` for a box |
| `--fixes FILE` | an [OSM fixes file](../guides/edit-osm-data.md#the-fixes-file): the cut file has the fixes in it | |
| `--cache DIR` | the folder of the downloads | `$DUCKOSM_CACHE`, else `~/.cache/duckosm` |
| `--refresh` | download the extract and the list of extracts again | the cached files |

### `cut`

Cuts a region file to a boundary and writes the area as a `.osm.pbf` file. Only lists of ids stay in memory.
Guide: [The OSM file](../exports/osm-file.md#duckosm-cut).

```text
duckosm cut [OPTIONS] --boundary FILE -o FILE REGION
```

| Option | Does | Default |
|---|---|---|
| `REGION` | the region file, sorted by type and id (as the files of Geofabrik) | required |
| `--boundary FILE` | the [boundary](boundary.md): GeoJSON, `.poly`, WKT or `W,S,E,N` | required |
| `-o`, `--out FILE` | the area file to write | required |
| `--fixes FILE` | an [OSM fixes file](../guides/edit-osm-data.md#the-fixes-file): the area file has the fixes in it ([what goes in](../exports/osm-file.md#the-fixes-in-the-file)) | |

### `build`

Builds a network from a PBF with a config file, or cuts one from a built database. The validation, the report and the
multimodal graph of this command are not ported to Rust yet; [`duckosm FILE`](#duckosm-file) is the build all in Rust.
Guide: [Build a network](../guides/build.md).

```text
duckosm build [OPTIONS]
```

| Option | Does | Default |
|---|---|---|
| `-c`, `--config PATH` | a [config file](configuration.md). An option below that you type overrides the file | |
| `-p`, `--pbf PATH` | input `.osm.pbf` | |
| `-o`, `--output PATH` | output file | `<name>.duckdb`, named after the boundary file, else the PBF, else the `--source-db` file |
| `-b`, `--boundary PATH` | GeoJSON area to build ([the boundary file](boundary.md)) | none: the whole PBF |
| `--source-db PATH` | cut from this built database instead of a PBF ([how](../guides/prepare-area.md#2-cut-a-built-region)) | |
| `--h3-cell TEXT` | build the area of this H3 cell: its outline is the boundary | |
| `--graph` / `--no-graph` | build `edge_graph`, the graph of legal turns | on |
| `--h3-index` / `--no-h3-index` | add H3 cell ids to nodes and edges | on |
| `--h3-resolution INTEGER` | H3 resolution, 0–15 | `8` |
| `--tile-index` / `--no-tile-index` | add the zoom-28 [tile keys](database.md#tile-keys) to nodes and edges. Only the Rust build (`duckosm --config`, `duckosm <PBF>`) has this option | on |
| `-m`, `--modes TEXT` | `driving`, `walking` or `cycling`; repeat for several | `driving` |
| `--features` / `--no-features` | build the [base-map layers](features.md), `features.*` | on |
| `--levels` / `--no-levels` | the last step: the [drawing order](../guides/drawing-order.md) of the roads (`visualization.edge_levels`), with your edits kept | on |
| `--fixes FILE` | a rules file of [fixes for OSM errors](../guides/edit-osm-data.md#the-fixes-file); overrides the config's `osm_overrides` key | none |
| `--log-file FILE` | also write the log to this file | console only |

With none of `-c`, `-p` and `--source-db`, it uses `config/default.yaml` if that file exists.

### `init-config`

Writes the commented config template, to edit and build from. Guide:
[The config file](../guides/build.md#the-config-file).

```text
duckosm init-config [OPTIONS] [PATH]
```

| Option | Does | Default |
|---|---|---|
| `PATH` | the file to write | `duckosm.yaml` |
| `--force` | overwrite `PATH` if it exists | |

### `boundary`

Finds the boundary of an area by name and writes it as GeoJSON: first in the PBF's own borders
(offline), then with Nominatim (online). Guide: [Prepare an area](../guides/prepare-area.md).

```text
duckosm boundary [OPTIONS] [NAME]
```

| Option | Does | Default |
|---|---|---|
| `--pbf PATH` | search the borders in this PBF first | |
| `--osm-id INTEGER` | take this OSM boundary relation (when a name matches several) | |
| `--offline` | never ask Nominatim | |
| `-o`, `--out FILE` | output GeoJSON ([the boundary file](boundary.md)) | `<name>.geojson` |

### `clip-pbf`

Cuts a PBF down to a GeoJSON area. The strategies have the rules of `osmium extract`. Guide:
[Cut a smaller PBF](../guides/prepare-area.md#1-cut-the-osm-file).

```text
duckosm clip-pbf [OPTIONS] PBF BOUNDARY
```

| Option | Does | Default |
|---|---|---|
| `-o`, `--out FILE` | output PBF | `<boundary name>.osm.pbf` |
| `--strategy` | `complete_ways`: a way that crosses the border is kept whole; `smart`: also completes multipolygons (rivers, land cover), for [base-map layers](features.md); `simple`: cuts ways at the border | `complete_ways` |

### `extract`

Cuts one area out of a built database into a new database, by name, OSM id or GeoJSON. Edge ids are
kept. Guide: [Cut a built region](../guides/prepare-area.md#2-cut-a-built-region).

```text
duckosm extract --source SOURCE --db DB (--name NAME | --osm-id OSM_ID | --boundary BOUNDARY)
```

| Option | Does | Default |
|---|---|---|
| `--source SOURCE` | the built database to cut from | required |
| `--db DB` | the new database (overwritten) | required |
| `--name NAME` | the area, by name in the source's `admin_boundaries` ([add them](../guides/admin-boundaries.md)) | one of the three |
| `--osm-id OSM_ID` | the area, by its `admin_boundaries` OSM id | one of the three |
| `--boundary BOUNDARY` | the area, as a GeoJSON file | one of the three |

## OSM data

### `quack`

The OSM data as tables, with no network, with your OSM fixes. Pages: [QuackOSM table](../concepts/quackosm.md),
[GDAL's OSM layers](../concepts/gdal.md).

```text
duckosm quack [OPTIONS] PBF
```

| Option | Does | Default |
|---|---|---|
| `-o`, `--out DIR` | the output folder | `.` |
| `--name NAME` | the files are `<out>/<name>_quack.*` | `area` |
| `-f`, `--format` | `geoparquet`, `duckdb`; with `--gdal`: `gpkg`, `geoparquet` | |
| `--gdal` | the five layers of GDAL's OSM driver, in place of the QuackOSM table | |
| `--fixes FILE` | an [OSM fixes file](../guides/edit-osm-data.md#the-fixes-file) | |
| `--tile-zoom Z` | make the table tile by tile with the map tiles of zoom `Z`, so the memory follows a tile, not the area | |
| `--tiles N` | tile by tile, with at most `N` nodes in a tile | |
| `--no-sort` | do not sort the rows (faster) | sorted |
| `--threads N` | the threads | all cores |

### `quack-tiles`, `quack-union`

`quack-tiles` writes the QuackOSM table as one table for each map tile of one zoom (`<out>/<z>/<x>/<y>.parquet`).
`quack-union` joins those tile tables into one GeoParquet file. `duckosm quack-tiles --help` and
`duckosm quack-union --help` list their options.

## Query and inspect

### `info`

Prints what a built database holds: per mode its edges, nodes, private edges, km, legal turns
(`edge_graph`) and turn restrictions; when and by which duckOSM version it was built; its time zone;
and the other schemas (raw OSM data, base-map layers, `mm`, boundary, elevation, drawing order). Reads only.

```text
duckosm info [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `--json` | print one JSON object instead of a table | |

```text
$ duckosm info monaco.duckdb
monaco.duckdb  (built 2026-10-01 18:20, duckOSM 0.1.0; time zone Europe/Monaco)
mode      edges  nodes  private_edges     km  edge_graph  turn_restrictions
driving   2,765  1,719            214   92.6       4,953                 38
walking  10,952  4,116            154  244.1      32,476                  -
cycling  10,274  4,151            133  245.5      28,851                  -
also: raw (OSM data), features (12 layers), mm (across modes), boundary
```

A database built before this command existed has no build date; the rest is the same.

### `way`

Prints everything the database knows about one OSM way: its raw row, then its edges in every mode,
in order. A negative `OSM_ID` shows [connector edges](../concepts/cleanup.md#connect-dangling-paths).
Guide: [Look up one OSM way](../guides/query.md#look-up-one-osm-way).

```text
duckosm way [OPTIONS] DB OSM_ID
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | modes to include; repeat for several | every mode in the db |
| `--geom` / `--no-geom` | add the edge geometry as WKT | off |
| `-o`, `--out PATH` | write the table to a `.csv`, `.parquet` or `.json` file instead of printing it | |
| `--json` | print one JSON object: the raw way (`tags`, `refs`) and its edges | |

## Maps, pages and routing {#maps-and-routing}

### `viz`

Draws the maps of a built network with [mapstyle](https://khoshkhah.github.io/mapstyle/): one page with
every mode and one per mode, over the database's own base map. Needs `duckosm[viz]`. Guide:
[Draw a map](../guides/draw-map.md).

```text
duckosm viz [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | only these modes' pages; repeat for several | every mode's page, plus `<name>_map.html` with all modes |
| `--basemap TEXT` | a background from the web: `voyager`, `positron`, `osm`, `satellite`, … | none: the database's own layers |
| `--out-dir TEXT` | output folder; files are `<name>_map.html` and `<name>_<mode>_network.html` | `reports` |
| `--arrows` / `--no-arrows` | one-way arrows, shown when zoomed in | on |
| `--boundary` / `--no-boundary` | draw `main.boundary`, if there is one | on |
| `--google-3d` / `--no-google-3d` | a Google 3D window linked to the map. It needs a Google Maps key in `GOOGLE_MAPS_KEY`, and the page must be served over http at an address on the key's website list | on when `GOOGLE_MAPS_KEY` is set |

### `levels`

Makes the [drawing order](../guides/drawing-order.md) of the roads again, and stores it in the schema
[`visualization`](database.md#visualization-schema). Your edits of the order are kept. The build does this as its last
step; use this command after a build with `--no-levels`. The database is changed in place.

```text
duckosm levels DB
```

### `levels-edit`

The level editor of the database: fix which road is drawn over which (bridges, crossings), and the head and the cap of a
road end. It opens [`duckosm studio`](#studio) on this file, with card 8 (Edit levels) open. "← Steps" writes the drawing
order into the file. Guide: [Store the drawing order](../guides/drawing-order.md).

```text
duckosm levels-edit [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `--port INTEGER` | the editor is at `http://127.0.0.1:PORT/` | `8000` |
| `--no-open` | do not open the page in the browser | off |

### `edit`

Edits the OSM data of the database on a page on your computer, with the panel **Edit OSM data**. Each change goes into
the fixes file at once, and **Build again** builds the database again with it. The page is at `http://127.0.0.1:PORT/`.
Guide: [Fix the OSM data](../guides/edit-osm-data.md).

```text
duckosm edit [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `--fixes FILE` | the fixes file; if it does not exist, the first change makes it | `osm_fixes.yaml` beside the database |
| `--port INTEGER` | the port of the page on 127.0.0.1 | `8790` |

Build again needs the record of the build in the database (`build_config` and `build_dir` of
[`visualization_metadata`](database.md#visualization_metadata)). `duckosm build` writes it, and `duckosm --config` too.

### `studio`

duckOSM Studio on your computer: the area, the OSM data, the settings, the ground (the DEM), the build and the result,
as on the web page. Each area goes into `OUT_DIR/<name>/`. Guide: [duckOSM Studio](../guides/studio.md).

```text
duckosm studio [OPTIONS] [OUT_DIR]
```

| Option | Does | Default |
|---|---|---|
| `--port INTEGER` | the port of the page on 127.0.0.1 | `8792` |
| `--geofabrik-index URL_OR_FILE` | the list of the Geofabrik extracts: a mirror, or a file for a test | `https://download.geofabrik.de/index-v1.json` |
| `--no-open` | do not open the page in the browser | off |

### `ask`

Asks questions about the database in words, on a page on your computer: **Ask** answers with SQL and draws the
result on the map, with your own key of a free AI service. The page does not change the data.
Guide: [Ask in words](../guides/query.md#ask-in-words).

```text
duckosm ask [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `--fixes FILE` | the fixes file whose changes you can ask about | `osm_fixes.yaml` beside the database |
| `--port INTEGER` | the port of the page on 127.0.0.1 | `8791` |

### `bus`

Matches the database's OSM bus route relations to its driving edges and writes schema [`bus`](database.md#bus-schema): `bus.routes` (one row per route, with
its gaps) and `bus.route_edges` (the edges each route travels, in order). A build from a PBF does it too (`options.bus_routes`); this adds it to a file built
before. The database is changed in place. Needs `raw.*` and the driving network. How: [Bus routes](../concepts/networks.md#bus-routes).

```text
duckosm bus DB
```

### `route-map`

Writes duckOSM's route planner over a map drawn by mapstyle: drag a start and an end, pick Drive,
Walk, Cycle or Walk + drive. Routing runs in the browser with the same answers as `route_points()`
and `directions()`. Walk + drive needs [`multimodal`](#multimodal) first. Needs `duckosm[viz]`. Guide:
[The map page](../guides/route.md#the-map-page).

```text
duckosm route-map [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode [driving\|walking\|cycling\|walk+drive]` | the first choice and the mode in front (`walk+drive`: every mode; needs `multimodal`); the page offers every mode | the first mode, walking in front |
| `--basemap TEXT` | a background from the web: `voyager`, `positron`, `osm`, `satellite`, … | none: the database's own layers |
| `--google-3d` / `--no-google-3d` | a Google 3D window linked to the map. It needs a Google Maps key in `GOOGLE_MAPS_KEY`, and the page must be served over http | on when `GOOGLE_MAPS_KEY` is set |
| `-o`, `--out TEXT` | output HTML | `reports/<name>_route_map.html` |

### `route`, `route-points`, `directions` {#route-route-points-directions}

Routing on a built database, from the command line. Each prints JSON. Guide: [Route](../guides/route.md).

```text
duckosm route [OPTIONS] DB FROM_EDGE TO_EDGE
duckosm route-points [OPTIONS] DB LON LAT LON LAT
duckosm directions [OPTIONS] DB ROUTE
duckosm route-multimodal [OPTIONS] DB SRC_NODE DST_NODE          # across modes: needs the mm schema (multimodal)
duckosm route-multimodal-points [OPTIONS] DB LON LAT LON LAT
```

| Option | Command | Does | Default |
|---|---|---|---|
| `-m`, `--mode` | all | the mode | `driving` |
| `--weight` | `route`, `route-points` | `time` (fastest) or `length` (shortest) | `time` |
| `--pairs FILE` | `route` | many routes: one pair of edge ids on each line | |
| `--radius M` | `route-points` | the roads that a point can join, within `M` metres | `50` |
| `--access-kmh V` | `route-points` | the walking speed to the road | `4.5` |
| `ROUTE` | `directions` | a JSON file of a route, as `route` prints it; `-` reads the standard input | |

### `features-check`

Lists the OSM polygons that no `features.*` table holds, grouped by tag, with the reason when the build leaves a group out on purpose.
A group with no reason is a possible miss. Needs `raw.*` and `features.*`, so a clipped database cannot be checked. Reference:
[Check what the build leaves out](features.md#check-what-the-build-leaves-out).

```text
duckosm features-check [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `--limit INTEGER` | how many groups to list | 40 |

## Exports

### `sumo`

Writes a SUMO network; the SUMO edge id is the `edge_id`. Page: [SUMO](../exports/sumo.md).

```text
duckosm sumo [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | the mode | `driving` |
| `--out-dir TEXT` | output folder | `sumo` |
| `--name TEXT` | file name base | the db file name |
| `--connections` / `--no-connections` | write the allowed turns from `edge_graph`; off lets `netconvert` guess them | on |
| `-c`, `--config PATH` | a `netconvert` config (`.netccfg`) instead of the built-in one | |
| `--netconvert` / `--no-netconvert` | build the `.net.xml` with `netconvert`; off writes only the plain XML files | on |
| `--join-junctions` | join close junctions (netconvert's `junctions.join`): a crossroads mapped as two or three nodes becomes one junction ([how](../exports/sumo.md)); not with `-c` | off |

### `matsim`

Writes a MATSim `network.xml`; link ids are `edge_id`s. Page: [MATSim](../exports/matsim.md).

```text
duckosm matsim [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | a mode, a comma list (`driving,cycling`) or `all`: one network with `modes` per link | `driving` |
| `--crs TEXT` | projected CRS for node coordinates | UTM zone of the data |
| `--gzip` / `--no-gzip` | gzip the file | on |
| `-o`, `--out TEXT` | output file | `<name>_network.xml.gz` |

### `matsim-lanes`

Writes MATSim `lanes.xml` and signal files from a GMNS database. The signal timing is a placeholder.
Page: [Lanes and signals](../exports/matsim.md#lanes-and-signals).

```text
duckosm matsim-lanes [OPTIONS] GMNS_DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | the GMNS mode | `driving` |
| `--signals` / `--no-signals` | also write `signalSystems`, `signalGroups`, `signalControl.xml` | on |
| `--cycle INTEGER` | signal cycle length, seconds | `90` |
| `-o`, `--out TEXT` | output folder | `.` |

### `gmns`

Writes a GMNS database: nodes, links, lanes, movements, signals; `link_id` is the `edge_id`.
SUMO's `netconvert` gives which lanes each movement uses; without it, the movements have no lanes and
there is no `lane_connector` (the command says so). Page: [GMNS](../exports/gmns.md).

```text
duckosm gmns [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-o`, `--out TEXT` | output database | `<name>_gmns.duckdb` |
| `-m`, `--mode TEXT` | modes; repeat for several | every mode in the db |
| `--to-csv TEXT` | also write the standard GMNS CSV files to this folder | |
| `--lane-geometry` / `--no-lane-geometry` | compute a line for each lane (needs shapely) | on |
| `--meso` | also build the meso (lane-level) network, `meso_<mode>` | |
| `--meso-mode TEXT` | modes for `--meso`; repeat for several | `driving` |
| `--micro` | also build the micro (cell) network, `micro_<mode>` | |
| `--micro-mode TEXT` | modes for `--micro`; repeat for several | `driving` |
| `--combined` | also write `gmns_all`, one network for all modes | |
| `--drive-side` | `right` or `left`: the side two-way lanes are drawn on | `right` |
| `--tile-zoom Z` | make an area with nodes in two or more map tiles of zoom `Z` one tile at a time, so the memory follows a tile, not the area. `0`: the whole area at once | `9` |

### `gmns-map`

Writes an HTML map of a GMNS database with [lanestyle](https://github.com/Khoshkhah/lanestyle): the roads of the level area
as roadstyle draws them, each as wide as its lanes, with the lanes, lane lines, arrows, zebras and sidewalks on them. Needs
the level area of the core database first: `duckosm levels SOURCE_DB`. Page: [See it](../exports/gmns.md#see-it).

```text
duckosm gmns-map [OPTIONS] --source-db SOURCE_DB GMNS_DB
```

| Option | Does | Default |
|---|---|---|
| `--source-db PATH` | the core database that the GMNS file was made from | required |
| `--area DIR` | the level area | `SOURCE_DB.levels`, made by `duckosm levels SOURCE_DB` |
| `-o`, `--out FILE` | output HTML | `<name>_lanes.html` |

### `sumo-map`

Writes an HTML map of a SUMO net (the `.net.xml` of `duckosm sumo`) with lanestyle and roadstyle: each lane on its own
shape at its width, and the internal lanes through the junctions. Page: [See it](../exports/sumo.md#see-it).

```text
duckosm sumo-map [OPTIONS] NET
```

| Option | Does | Default |
|---|---|---|
| `-o`, `--out PATH` | output HTML | `<name>_lanes.html` |

### `gmns-viz`

Writes an HTML viewer of a GMNS database: lanes, and the meso network when there is one. Hover a
line for its attributes. Page: [See it](../exports/gmns.md#see-it).

```text
duckosm gmns-viz [OPTIONS] GMNS_DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | the GMNS mode | `driving` |
| `-o`, `--out TEXT` | output HTML | `<name>_viewer.html` |

### `export-gis`

Writes a GeoPackage, GeoParquet files or shapefiles: edges, nodes and the boundary, `edge_id` kept. Page:
[GeoPackage, GeoParquet and shapefile](../exports/gis.md).

```text
duckosm export-gis [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | modes; repeat for several | every mode in the db |
| `-f`, `--format` | `gpkg` (GeoPackage), `geoparquet` or `shp` (shapefile) | `gpkg` |
| `-o`, `--out TEXT` | `.gpkg` file, or folder for shapefiles and GeoParquet | `<name>.gpkg` / `<name>_gis/` / `<name>_geoparquet/` |
| `--boundary` / `--no-boundary` | include `main.boundary` as a layer | on |
| `--name TEXT` | layer and file name base | the db file name |

### `gis-debug`

Reads a GeoPackage or shapefile export back through GDAL and writes an HTML page: a map of every
layer and a check of counts, CRS and `edge_id`s. Needs geopandas. Page:
[Check an export](../exports/gis.md#check-an-export).

```text
duckosm gis-debug [OPTIONS] EXPORT_PATH
```

| Option | Does | Default |
|---|---|---|
| `--source-db PATH` | the database to compare against (every `edge_id` must match) | |
| `-o`, `--out TEXT` | output HTML | `<name>_gis_debug.html` |
| `--name TEXT` | name shown on the page | the export file name |
| `--json` | also print the check as JSON: verdict, summary, each layer | |

### `export-graph`

Writes a networkx graph file (GraphML or gpickle). To load a gpickle file, Python needs networkx (and shapely 2 for `--geometry shapely`). Page: [networkx](../exports/networkx.md).

```text
duckosm export-graph [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | the mode | `driving` |
| `-o`, `--out TEXT` | output file; the extension picks the format | `<name>_<mode>.graphml` |
| `-g`, `--graph` | `node`: junctions and roads; `edge`: the edge-based routing graph | `node` |
| `-f`, `--format` | `graphml` or `gpickle` | from `--out`, else `graphml` |
| `--weight` | arc weight, `time` or `length`; `--graph edge` only | `time` |
| `--geometry` | edge geometry, `wkt`, `shapely` or `none`; `--graph node` only; GraphML writes shapely as WKT | `wkt` |
| `--uses TEXT` | more turns for `--graph edge`: a comma list of `bus`, `private`, `pedestrian`, `delivery`, the turns into and out of those restricted edges (`private_edges`); they get an `access` attribute | the mode's own turns only |

## Experimental and lane-level

### `opendrive`

Writes an ASAM OpenDRIVE `.xodr`: roads and lanes. Page: [OpenDRIVE](../exports/opendrive.md).

```text
duckosm opendrive [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | the mode | `driving` |
| `--crs TEXT` | projected CRS for the road lines | UTM zone of the data |
| `--junctions` | also write junctions and turn roads; `DB` must then be a GMNS database | |
| `-o`, `--out TEXT` | output file | `<name>.xodr` |

### `lanelet2`

Writes a Lanelet2 map (`.osm`) from a GMNS database: one lanelet per lane. Page:
[Lanelet2](../exports/lanelet2.md).

```text
duckosm lanelet2 [OPTIONS] GMNS_DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | the GMNS mode | `driving` |
| `-o`, `--out TEXT` | output file | `<name>.lanelet2.osm` |

### `railml`

Writes a railML 2.4 file of the rail network, read from `raw.*` (there is no rail mode). Page:
[railML](../exports/railml.md).

```text
duckosm railml [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-o`, `--out TEXT` | output file | `<name>.railml.xml` |

### `lane-graph`

Builds a lane-level routing graph in a GMNS database: lane-to-lane turns and lane changes, in
`lane_<mode>.lane_edges`. Page: [Lane-level routing](../exports/lane-routing.md).

```text
duckosm lane-graph [OPTIONS] GMNS_DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | the GMNS mode | `driving` |

### `route-lanes`

Finds a lane-level route between two lanes and prints the lanes, cost and manoeuvres. Each of
`FROM_LANE` and `TO_LANE` is a `lane_id`, or an `edge_id` (its lane 1). Uses the table from
[`lane-graph`](#lane-graph), or builds the graph in memory without it. Page: [Lane-level routing](../exports/lane-routing.md).

```text
duckosm route-lanes [OPTIONS] GMNS_DB FROM_LANE TO_LANE
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | the GMNS mode | `driving` |
| `-o`, `--out TEXT` | also write the route as GeoJSON | |
| `--json` | print the route as JSON: `lanes`, `cost`, `maneuvers`, `geometry` (WKT) | |

## Enrich a built database

### `elevation`

Adds the ground height to every node (`nodes.ele`) and edge end (`edges.z_from`, `z_to`), and a row
in `main.elevation_metadata`. Running it again overwrites. Guide:
[Add elevation](../guides/elevation.md); sources: [Elevation sources](elevation-sources.md).

```text
duckosm elevation [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `--dem TEXT` | a GeoTIFF in EPSG:4326 with 32-bit floats, a file or an HTTP address; wins over `--source` | |
| `--source TEXT` | `auto`, `copernicus` or `eudtm` (needs `OPENTOPOGRAPHY_API_KEY`). `auto` picks EU-DTM in Europe when the key is set, else Copernicus | `auto` |
| `-m`, `--mode TEXT` | modes; repeat for several | every mode in the db |
| `--nodata-fill FLOAT` | height written where the model has no value | `0.0` |
| `--suffix NAME` | store as a second surface: `--suffix dsm` writes `ele_dsm`, `z_from_dsm`, `z_to_dsm` | |
| `--product [DTM\|DSM]` | the product of the `--dem` file: `DTM` (bare earth) or `DSM` (surface) | `unknown` |
| `--vertical-datum DATUM` | the vertical datum of the `--dem` file, for example `EGM2008` | `unknown` |
| `--license TEXT` | the license of the `--dem` file | `unknown` |
| `--not-reference` | the `--dem` file is not the area's [ground reference](../concepts/ground-reference.md): its product, its datum and its license can stay `unknown` | |

A `--dem` file in `ele` (no `--suffix`) is the area's ground reference. Give `--product`, `--vertical-datum` and
`--license`, or `--not-reference`. If not, the command stops. The SHA-256 of the file goes into `main.elevation_metadata.sha256`.

### `admin`

Adds `main.admin_boundaries`: every OSM administrative boundary in the PBF, with its parent. Replaces
the table if it exists. Guide:
[Add administrative boundaries](../guides/admin-boundaries.md).

```text
duckosm admin --pbf PBF --db DB [--gpkg GPKG]
```

| Option | Does | Default |
|---|---|---|
| `--pbf PBF` | the OSM PBF | required |
| `--db DB` | the database to add the table to | required |
| `--gpkg GPKG` | GeoPackage of the boundaries: reused if it exists, else written here (the next run does not read the PBF again) | a temporary file |

### `multimodal`

Builds `mm.edges` and `mm.transfers`, so a route can change mode (walk, drive, walk). Needs walking
and at least one other mode. Page: [Routing across modes](../concepts/multimodal.md).

```text
duckosm multimodal [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `--transfer-cost FLOAT` | seconds added at each change of mode | `60.0` |
| `--pair-cost FROM->TO=SECONDS` | another cost for one direction, for example `walking->driving=120`; repeat it for more directions (the build's `multimodal.transfer_costs`) | none |
| `--schema TEXT` | schema for the two tables | `mm` |
