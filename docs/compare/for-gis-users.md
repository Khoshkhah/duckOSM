# duckOSM for GIS users

This page is for users of OSMnx's features, QuackOSM, osmium and ogr2ogr. Each table gives the call of your tool and
the duckOSM call that does the same work. The module is `duckosm.gis`
([reference](../reference/sdk/gis.md)). The notebook
[6. OSM data for GIS](../notebooks/06_gis.md) runs each call on Monaco.

```bash
pip install "duckosm[gis]"      # geopandas
```

## OSM features

| Your tool | duckOSM |
|---|---|
| `ox.features_from_place("Monaco", tags={"building": True})` | `duckosm.gis.features(duckosm.get("Monaco"), tags={"building": True})` |
| `ox.features_from_bbox(bbox, tags)` | `duckosm.gis.features(duckosm.get(bbox=bbox), tags)` |
| `ox.features_from_point(point, tags, dist=500)` | `duckosm.gis.features(duckosm.get(point=point, dist_m=500), tags)` |
| `ox.features_from_xml("file.osm", tags)` | `duckosm.gis.features("file.osm.pbf", tags)` |
| `quackosm.convert_pbf_to_geodataframe(pbf, tags_filter=tags)` | `duckosm.gis.features(pbf, tags)` |
| `quackosm.convert_pbf_to_parquet(pbf)` | `duckosm.gis.quack(pbf, formats=["geoparquet"])` |
| `osmium tags-filter` then `ogr2ogr` | `duckosm.gis.features(pbf, tags)`, or `duckosm.gis.quack(pbf, gdal=True)` for GDAL's five layers |

`tags` has the rules of OSMnx:

| `tags` | Features |
|---|---|
| `{"building": True}` | each feature with the tag `building` |
| `{"amenity": "cafe"}` | the features with `amenity=cafe` |
| `{"amenity": ["cafe", "bar"]}` | the features with one of the values |
| `{"amenity": "cafe", "shop": True}` | the features of one key or of the other (OR) |

| Column | Content |
|---|---|
| `feature_id` | `node/…`, `way/…` or `relation/…`, as in QuackOSM |
| one column for each key of `tags` | the value of the tag |
| `tags` | all tags of the feature, as a dict |
| `geometry` | point, line, polygon or multipolygon, EPSG:4326 |

Differences:

| | OSMnx | duckOSM |
|---|---|---|
| Source of the data | the Overpass API, at each call | the Geofabrik extract, in a cache |
| Rows | OSMnx's rules for polygons | the rows of QuackOSM |
| OSM fixes | no | `fixes="osm_fixes.yaml"` applies your fixes first |

## The network as GIS files

| Your tool | duckOSM |
|---|---|
| `ox.save_graph_geopackage(G, "monaco.gpkg")` | `net.to_gis("monaco.gpkg")` |
| `ox.graph_to_gdfs(G)` | `net.edges()`, `net.nodes()` |
| `ox.save_graph_xml(G, "monaco.osm")` | `duckosm.gis.to_osm_xml(net.driving, "monaco.osm")` |
| `gdf.to_parquet(...)` | `net.to_gis("out", fmt="geoparquet")` |
| `ogr2ogr -f "ESRI Shapefile"` | `net.to_gis("out", fmt="shp")` |

`to_osm_xml` writes one `<way>` for each road. Its id is the `edge_id` of the forward edge. A two-way road is one way,
with `oneway=no` and the lanes of each direction (`lanes:forward`, `lanes:backward`). The tags `duckosm:osm_id` and
`duckosm:edge_id` give the OSM way and the edge.

## Administrative areas

| Your tool | duckOSM |
|---|---|
| `ox.geocode_to_gdf("Monaco")` | `duckosm.gis.admin_boundaries(net)`: all levels of the area, each with its parent |
| `osmium tags-filter r/boundary=administrative` | `duckosm.gis.admin(net, area)`: the table `admin_boundaries` in the database |

## Base maps and tiles

| Your tool | duckOSM |
|---|---|
| Geofabrik's shapefiles (`gis_osm_*_free`) | `duckosm quack --geofabrik` ([Geofabrik layers](../exports/geofabrik.md)) |
| Tilemaker, Planetiler | `duckosm quack --pmtiles` ([Vector tiles](../exports/pmtiles.md)) |
| osm2pgsql | `duckosm quack --osm2pgsql` ([osm2pgsql tables](../exports/osm2pgsql.md)) |
