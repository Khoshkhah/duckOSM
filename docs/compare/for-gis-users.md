# duckOSM for GIS users

This page is for users of OSMnx's features, QuackOSM, osmium, GDAL (`ogr2ogr`), Tilemaker, and osm2pgsql. Each section compares standard GIS toolchains with the duckOSM equivalent. The Python module is `duckosm.gis` ([reference](../reference/sdk/gis.md)), and CLI commands include `duckosm quack`, `duckosm export-gis`, and `duckosm cut`. The notebook [10. OSM data for GIS](https://khoshkhah.github.io/duckOSM/book/10_gis.html) runs each workflow on Monaco.

```bash
pip install "duckosm[gis]"      # geopandas, pyarrow, shapely
```

## Command Line Interface (CLI)

duckOSM provides direct command-line equivalents for common GIS pipelines that traditionally required multiple distinct tools:

```bash
# 1. Extract OSM features table (QuackOSM equivalent)
duckosm quack data/monaco.pbf -o out/features.parquet

# 2. Extract Geofabrik's 20 standard GIS layers (shp or GeoParquet)
duckosm quack data/monaco.pbf --geofabrik -o out/geofabrik/

# 3. Extract GDAL's 5 standard OSM layers (ogr2ogr equivalent, 4x faster)
duckosm quack data/monaco.pbf --gdal -o out/gdal/

# 4. Generate PMTiles vector tiles (Tilemaker / Planetiler equivalent)
duckosm quack data/monaco.pbf --pmtiles -o monaco.pmtiles

# 5. Export routable network to GIS formats (GeoPackage, GeoParquet, Shapefile)
duckosm export-gis data/monaco.duckdb -o out/gis/ --format geoparquet

# 6. Spatially cut large PBF files (osmium extract equivalent)
duckosm cut data/france.pbf -o monaco.osm.pbf -b 7.40,43.72,7.44,43.76
```

## OSM Features: OSMnx, QuackOSM, osmium

| Standard Tool (OSMnx / QuackOSM / osmium) | duckOSM CLI / Python SDK | What duckOSM does |
|---|---|---|
| `ox.features_from_place("Monaco", tags={"building": True})` | `duckosm.gis.features(duckosm.get("Monaco"), tags={"building": True})` | Queries local DuckDB cached extract instead of slow Overpass API |
| `ox.features_from_bbox(bbox, tags)` | `duckosm.gis.features(duckosm.get(bbox=bbox), tags)` | Bounding box spatial filter over PBF features |
| `ox.features_from_point(point, tags, dist=500)` | `duckosm.gis.features(duckosm.get(point=point, dist_m=500), tags)` | Buffer query in metres around coordinate |
| `ox.features_from_xml("file.osm", tags)` | `duckosm.gis.features("file.osm.pbf", tags)` | Fast zero-copy parsing of PBF rather than heavy XML |
| `quackosm.convert_pbf_to_geodataframe(pbf, tags_filter=tags)` | `duckosm.gis.features(pbf, tags)`<br/>`duckosm quack pbf -o out.parquet` | 3 to 7 times faster than QuackOSM, with 2 to 5 times less memory |
| `quackosm.convert_pbf_to_parquet(pbf)` | `duckosm.gis.quack(pbf, formats=["geoparquet"])` | Streamed GeoParquet output using DuckDB spatial |
| `osmium tags-filter` then `ogr2ogr` | `duckosm.gis.features(pbf, tags)`<br/>`duckosm quack pbf --gdal` | Generates GDAL's 5 layers (`points`, `lines`, `multilinestrings`, `multipolygons`, `other_relations`) 4x faster than ogr2ogr |

### Filter Tags

The `tags` dictionary adheres to OSMnx conventions:

| `tags` argument | Matched features |
|---|---|
| `{"building": True}` | Any feature with the `building` tag present |
| `{"amenity": "cafe"}` | Features with `amenity=cafe` |
| `{"amenity": ["cafe", "bar"]}` | Features with any matching value (`amenity IN ('cafe', 'bar')`) |
| `{"amenity": "cafe", "shop": True}` | Union of tags (OR logic) |

### Feature Comparison: OSMnx vs duckOSM

| Capability | OSMnx `features_*` | duckOSM `features()` |
|---|---|---|
| **Data source** | Overpass API server (throttled, rate limits, network timeouts) | Local Geofabrik `.osm.pbf` extracts cached on disk |
| **Parsing speed** | Minutes for medium cities | Seconds (processed via Rust/DuckDB SIMD routines) |
| **Fixes support** | None | Applies `osm_fixes.yaml` before feature materialization |
| **Browser native** | No (requires Python & C libraries) | Yes (runs in WebAssembly via DuckDB-Wasm) |

## The Network as GIS Files

Exporting the Level 2 directed network with topology, edge IDs, and turn restrictions into GIS formats:

| Standard Tool (OSMnx / GDAL) | duckOSM CLI / Python SDK | What duckOSM does |
|---|---|---|
| `ox.save_graph_geopackage(G, "monaco.gpkg")` | `duckosm export-gis data/monaco.duckdb -o monaco.gpkg`<br/>`net.to_gis("monaco.gpkg")` | Writes directed edge and node layers to GeoPackage |
| `ox.graph_to_gdfs(G)` | `net.edges()`, `net.nodes()` | Returns clean GeoDataFrames with stable `edge_id` |
| `ox.save_graph_xml(G, "monaco.osm")` | `duckosm.gis.to_osm_xml(net.driving, "monaco.osm")` | Exports valid OSM XML with `lanes:forward`/`lanes:backward` and `duckosm:edge_id` |
| `gdf.to_parquet("monaco.parquet")` | `net.to_gis("out", fmt="geoparquet")` | Streamed GeoParquet with native DuckDB geometry encoding |
| `ogr2ogr -f "ESRI Shapefile"` | `net.to_gis("out", fmt="shp")` | ESRI Shapefiles per layer |

## Administrative Boundaries

| Standard Tool (OSMnx / osmium) | duckOSM Python SDK | What duckOSM does |
|---|---|---|
| `ox.geocode_to_gdf("Monaco")` | `duckosm.gis.admin_boundaries(net)` | Returns all administrative boundary levels with parent-child hierarchy |
| `osmium tags-filter r/boundary=administrative` | `duckosm.gis.admin(net, area)` | Queries `admin_boundaries` table directly from DuckDB |

## Base Maps, Tiles, and Database Tables

| Standard tool | duckOSM CLI equivalent | Output description |
|---|---|---|
| **Geofabrik shapefiles** (`gis_osm_*_free`) | `duckosm quack pbf --geofabrik` | The exact 20 standard GIS layers, available for any custom area, in Shapefile or GeoParquet |
| **Tilemaker / Planetiler** | `duckosm quack pbf --pmtiles` | Single-file PMTiles vector archive (Shortbread schema) |
| **osm2pgsql into PostGIS** | `duckosm quack pbf --osm2pgsql` | The 4 osm2pgsql schema tables (`point`, `line`, `polygon`, `roads`) inside DuckDB or PostGIS (1.7x faster on Kentucky) |
