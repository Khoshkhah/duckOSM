# Command line

Every `duckosm` command and its options. `duckosm <command> --help` prints the same list;
`duckosm --version` prints the version. `-h` is short for `--help` everywhere.

| Group | Commands |
|---|---|
| [Build and areas](#build-and-areas) | `build`, `init-config`, `boundary`, `clip-pbf`, `extract` |
| [Query and inspect](#query-and-inspect) | `way` |
| [Maps and routing](#maps-and-routing) | `viz`, `route-map` |
| [Exports](#exports) | `sumo`, `matsim`, `matsim-lanes`, `gmns`, `gmns-map`, `gmns-viz`, `export-gis`, `gis-debug`, `export-graph` |
| [Experimental and lane-level](#experimental-and-lane-level) | `opendrive`, `lanelet2`, `railml`, `lane-graph`, `route-lanes` |
| [Enrich a built database](#enrich-a-built-database) | `elevation`, `admin`, `multimodal` |

`DB` is a built duckOSM database; `GMNS_DB` is a database written by [`gmns`](#gmns). Commands that
enrich a database (`elevation`, `admin`, `multimodal`) change it in place.

## Build and areas

### `build`

Builds a network from a PBF, or cuts one from a built database. Guide: [Build a network](../guides/build.md).

```text
duckosm build [OPTIONS]
```

| Option | Does | Default |
|---|---|---|
| `-c`, `--config PATH` | a [config file](configuration.md). An option below that you type overrides the file | |
| `-p`, `--pbf PATH` | input `.osm.pbf` | |
| `-o`, `--output PATH` | output file | `<name>.duckdb`, named after the boundary file, else the PBF, else the `--source-db` file |
| `-b`, `--boundary PATH` | GeoJSON area to build | none: the whole PBF |
| `--source-db PATH` | cut from this built database instead of a PBF ([how](../guides/prepare-area.md#several-areas-from-one-region)) | |
| `--h3-cell TEXT` | build the area of this H3 cell: its outline is the boundary | |
| `--graph` / `--no-graph` | build `edge_graph`, the graph of legal turns | on |
| `--h3-index` / `--no-h3-index` | add H3 cell ids to nodes and edges | on |
| `--h3-resolution INTEGER` | H3 resolution, 0–15 | `8` |
| `-m`, `--modes TEXT` | `driving`, `walking` or `cycling`; repeat for several | `driving` |
| `--features` / `--no-features` | build the [base-map layers](features.md), `features.*` | on |
| `--fixes FILE` | a rules file of [fixes for OSM errors](../guides/fix-osm-errors.md); overrides the config's `osm_overrides` key | none |
| `--log-file FILE` | also write the log to this file | console only |

With none of `-c`, `-p` and `--source-db`, it uses `config/default.yaml` if that file exists.

### `init-config`

Writes the commented config template, to edit and build from. Guide:
[With a config file](../guides/build.md#with-a-config-file).

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
| `--pbf PATH` | search the borders in this PBF first; needs GDAL's `ogr2ogr` | |
| `--osm-id INTEGER` | take this OSM boundary relation (when a name matches several) | |
| `--offline` | never ask Nominatim | |
| `-o`, `--out FILE` | output GeoJSON | `<name>.geojson` |

### `clip-pbf`

Cuts a PBF down to a GeoJSON area, with `osmium`. Guide:
[Cut a smaller PBF](../guides/prepare-area.md#cut-a-smaller-pbf).

```text
duckosm clip-pbf [OPTIONS] PBF BOUNDARY
```

| Option | Does | Default |
|---|---|---|
| `-o`, `--out FILE` | output PBF | `<boundary name>.osm.pbf` |
| `--strategy` | `complete_ways`: a way that crosses the border is kept whole; `smart`: also completes multipolygons (rivers, land cover), for [base-map layers](features.md); `simple`: cuts ways at the border | `complete_ways` |

### `extract`

Cuts one area out of a built database into a new database, by name, OSM id or GeoJSON. Edge ids are
kept. Guide: [Several areas from one region](../guides/prepare-area.md#several-areas-from-one-region).

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

## Query and inspect

### `info`

Prints what a built database holds: per mode its edges, nodes, private edges, km, legal turns
(`edge_graph`) and turn restrictions; when and by which duckOSM version it was built; its time zone;
and the other schemas (raw OSM data, base-map layers, `mm`, boundary, elevation). Reads only.

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
also: raw (OSM data), features (10 layers), mm (across modes), boundary
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

## Maps and routing

### `viz`

Writes an HTML map of each mode, roads styled by class, with [roadstyle](../guides/draw-map.md).
Needs `duckosm[viz]`.

```text
duckosm viz [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | modes to draw; repeat for several | every mode in the db |
| `--basemap TEXT` | first base map: `voyager`, `positron`, `dark_matter`, `osm`, `satellite`, `blank` | `voyager` |
| `--out-dir TEXT` | output folder; files are `<name>_<mode>_network.html` | `reports` |
| `--arrows` / `--no-arrows` | one-way arrows, shown when zoomed in | on |
| `--boundary` / `--no-boundary` | draw `main.boundary`, if there is one | on |

### `route-map`

Writes an interactive route planner: click a start and an end, pick a mode. Routing runs in the
browser. Walk + drive needs [`multimodal`](#multimodal) first. Needs `duckosm[viz]`. Guide:
[On a map](../guides/route.md#on-a-map).

```text
duckosm route-map [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | modes to include; repeat for several | every mode in the db |
| `--basemap TEXT` | first base map: `osm`, `voyager`, `positron`, `dark_matter`, `satellite`, `blank` | `osm` |
| `-o`, `--out TEXT` | output HTML | `reports/<name>_route_map.html` |

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
Page: [GMNS](../exports/gmns.md).

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

### `gmns-map`

Writes an HTML map of a GMNS database: one ribbon per direction, or every lane at its width.
Page: [See it](../exports/gmns.md#see-it).

```text
duckosm gmns-map [OPTIONS] GMNS_DB
```

| Option | Does | Default |
|---|---|---|
| `--style` | `road`: one ribbon per direction, by class; `lane`: every lane | `road` |
| `-m`, `--mode TEXT` | the GMNS mode | `driving` |
| `-o`, `--out TEXT` | output HTML | `<name>_<style>.html` |

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

Writes a GeoPackage or shapefiles: edges, nodes and the boundary, `edge_id` kept. Page:
[GeoPackage and shapefile](../exports/gis.md).

```text
duckosm export-gis [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode TEXT` | modes; repeat for several | every mode in the db |
| `-f`, `--format` | `gpkg` (GeoPackage) or `shp` (shapefile) | `gpkg` |
| `-o`, `--out TEXT` | `.gpkg` file, or folder for shapefiles | `<name>.gpkg` / `<name>_gis/` |
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

Writes a networkx graph file (GraphML or gpickle). Needs networkx. Page: [networkx](../exports/networkx.md).

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
in `main.elevation_metadata`. Running it again overwrites. Needs `duckosm[elevation]`. Guide:
[Add elevation](../guides/elevation.md); sources: [Elevation sources](elevation-sources.md).

```text
duckosm elevation [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `--dem TEXT` | an elevation file or URL, any format GDAL reads; wins over `--source` | |
| `--source TEXT` | `auto`, `copernicus` or `eudtm` (needs `OPENTOPOGRAPHY_API_KEY`). `auto` picks EU-DTM in Europe when the key is set, else Copernicus | `auto` |
| `-m`, `--mode TEXT` | modes; repeat for several | every mode in the db |
| `--nodata-fill FLOAT` | height written where the model has no value | `0.0` |
| `--suffix NAME` | store as a second surface: `--suffix dsm` writes `ele_dsm`, `z_from_dsm`, `z_to_dsm` | |

### `admin`

Adds `main.admin_boundaries`: every OSM administrative boundary in the PBF, with its parent. Replaces
the table if it exists. Needs GDAL's `ogr2ogr`. Guide:
[Add administrative boundaries](../guides/admin-boundaries.md).

```text
duckosm admin --pbf PBF --db DB [--gpkg GPKG]
```

| Option | Does | Default |
|---|---|---|
| `--pbf PBF` | the OSM PBF | required |
| `--db DB` | the database to add the table to | required |
| `--gpkg GPKG` | GeoPackage of the boundaries: reused if it exists, else written here (skips the slow `ogr2ogr` step next time) | a temporary file |

### `multimodal`

Builds `mm.edges` and `mm.transfers`, so a route can change mode (walk, drive, walk). Needs walking
and at least one other mode. Page: [Routing across modes](../concepts/multimodal.md).

```text
duckosm multimodal [OPTIONS] DB
```

| Option | Does | Default |
|---|---|---|
| `--transfer-cost FLOAT` | seconds added at each change of mode | `60.0` |
| `--schema TEXT` | schema for the two tables | `mm` |
