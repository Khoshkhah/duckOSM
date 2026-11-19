# The boundary file

A boundary is the area that you build. `duckosm get`, `duckosm cut` and the build read it from a GeoJSON file.

## The format

A boundary file is [GeoJSON (RFC 7946)](https://datatracker.ietf.org/doc/html/rfc7946). The coordinates are WGS 84
longitude and latitude in degrees (EPSG:4326), in the order **[longitude, latitude]**.

```json
{"type": "FeatureCollection", "features": [
  {"type": "Feature",
   "properties": {"name": "Monaco", "osm_id": 2220322},
   "geometry": {"type": "Polygon", "coordinates": [[[7.409, 43.723], [7.440, 43.751], [7.418, 43.747], [7.409, 43.723]]]}}
]}
```

| Rule | Description |
|---|---|
| Types | `Polygon`, `MultiPolygon`, a `Feature` with one of them, or a `FeatureCollection` of such features. A `Point`, a `LineString` or a `GeometryCollection` is an error. |
| Several features | Give one feature. With more, the cut uses the polygons of all features, but `main.boundary` can keep only the first ([Properties](#properties)). |
| Holes | The second and next rings of a polygon are holes. A point in a hole is outside the area. |
| A point on the border | A node on a ring is inside the area. |
| Rings | Each ring is closed: its last position equals its first, and it has 4 positions or more. |
| A third value | A position `[lon, lat, height]` is accepted. The height is not used. |
| A feature with no geometry | A `Feature` with a `bbox` member (`[west, south, east, north]`) and no geometry is the box. |

`duckosm cut --boundary` also reads three other formats: an osmosis polygon file (`.poly`, as on Geofabrik), WKT
(`POLYGON` or `MULTIPOLYGON`) and a box as text (`7.40,43.72,7.44,43.76`). `duckosm get --boundary` and the build
read only GeoJSON.

## Properties

The build writes the boundary to the table `main.boundary`, with its properties as columns.

| Build | Rows of `main.boundary` |
|---|---|
| `duckosm FILE --boundary ...` and duckOSM Studio | one row: the first feature, with all its properties |
| `duckosm build --boundary ...` | one row for each feature, with all its properties |

| GeoJSON value | Column type |
|---|---|
| an integer | `INTEGER` (`BIGINT` when it is too large) |
| a decimal number | `DOUBLE` |
| `true`, `false` | `BOOLEAN` |
| a string, `null` | `VARCHAR` |
| an object, a list | `VARCHAR`, as JSON text |

For a place, `duckosm boundary` and `duckosm get` write the properties `name`, `osm_id`, `admin_level`, `area_km2` and
`source`. For a box or an H3 cell, `duckosm get` writes only `name`.

## Make a boundary file

| Tool | How |
|---|---|
| `duckosm get` | `duckosm get "Monaco"` writes `monaco.geojson` next to `monaco.osm.pbf` ([The OSM file](../exports/osm-file.md)). |
| `duckosm boundary` | `duckosm boundary "Tartu"` or `duckosm boundary --osm-id 2220322`: an OSM boundary relation, from Nominatim or from a PBF ([CLI](cli.md#boundary)). |
| [geojson.io](https://geojson.io) | Draw a polygon. Then **Save → GeoJSON**. The file is in WGS 84. |
| QGIS | Right-click the layer → **Export → Save Features As**. Format **GeoJSON**, CRS **EPSG:4326**. |
| `ogr2ogr` (GDAL) | `ogr2ogr -f GeoJSON -t_srs EPSG:4326 area.geojson area.shp` |

## Errors

| Error | Cause | Fix |
|---|---|---|
| `boundary: the box ... is not longitude and latitude in degrees` | The coordinates are in metres: another CRS, for example UTM or SWEREF 99 TM. | Export the file again in EPSG:4326 (QGIS or `ogr2ogr -t_srs EPSG:4326`). |
| `boundary: the crs is ...` | The file has a `crs` member that names another CRS. | Export the file again in EPSG:4326. A file in EPSG:4326 needs no `crs` member. |
| `boundary: a ring is not closed` | The last position of a ring is not the first one. | Add the first position again at the end of the ring. |
| `warning: the cut has no nodes`, `no Geofabrik extract covers the boundary`, or `get` names a wrong extract | The positions are `[latitude, longitude]`. Monaco as `[43.73, 7.42]` is a place in Ethiopia. | Change each position to `[longitude, latitude]`. |
| `boundary: Point is not a Polygon or a MultiPolygon` | The geometry is not an area. | Draw a polygon around the area. |

`duckosm get` and `duckosm cut` find the first three errors before they cut. The build does not check them.
