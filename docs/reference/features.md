# Base-map layers

The `features` schema holds what a base map draws besides the routing networks: water, land use,
buildings, points of interest, place names. Each layer is one table, with classes from the
[Shortbread](https://shortbread-tiles.org/) vector-tile schema. mapstyle renders
them.

## Build them

```yaml
options:
  build_features: true
```

The layers are built from `raw.*` at the end of the build. With a boundary, the PBF is then cut with
the `smart` strategy, so rivers and land-use areas that cross the border stay whole
(`options.clip_strategy`). A build cut from a parent database has no `raw.*` and can't build them.

On a database that still has `raw.*`, you can also build them afterwards:

```python
import duckdb
from duckosm.features import FeaturesBuilder

con = duckdb.connect("monaco.duckdb")
FeaturesBuilder(con).run()              # -> {layer: row count}
```

A layer that fails is logged and skipped; the build goes on.

## Columns

Every table has the same columns:

| Column | Type | Description |
|---|---|---|
| `osm_id` | BIGINT | the OSM id; unique only together with `osm_type` |
| `osm_type` | VARCHAR | `node`, `way` or `relation` |
| `kind` | VARCHAR | the class, from the rules below |
| `name` | VARCHAR | the OSM `name` |
| `tags` | MAP(VARCHAR, VARCHAR) | every OSM tag |
| `geom` | GEOMETRY | the shape, longitude / latitude |

`features.traffic` has one more: `bearing` (DOUBLE).

An **area** layer holds closed ways and multipolygon relations; a **line** layer holds ways; a
**point** layer holds nodes.

## Layers

In drawing order, bottom first. Monaco counts are from the sample build.

| Layer | Shape | From OSM | `kind` | Monaco |
|---|---|---|---|---|
| `land` | area | `landuse` = `forest`, `grass`, `residential`, `industrial`, `cemetery`, …; `natural` = `wood`, `scrub`, `heath`, `sand`, `beach`, `wetland`, …; `leisure` = `park`, `garden`, `pitch`, `stadium`, `playground`, … | the tag value; `natural=wood` becomes `forest` | 237 |
| `water_polygons` | area | `natural` = `water` / `glacier`, `waterway=riverbank`, `landuse` = `reservoir` / `basin`, any `water` tag | `water`, `river`, `canal`, `reservoir`, `basin`, `dock`, `glacier` | 27 |
| `water_lines` | line | `waterway` = `river`, `stream`, `canal`, `ditch`, `drain` | the value; `drain` becomes `ditch` | 4 |
| `sites` | area | `amenity` = `parking`, `bicycle_parking`, `bus_station`, `university`, `college`, `school`, `kindergarten`, `hospital`, `prison`; `public_transport` = `platform` / `station`; `leisure=sports_centre`, `landuse=construction`, `military=danger_area`, `man_made=bridge` | the value | 79 |
| `buildings` | area | `building`, except `building=no` | the `building` value | 1,262 |
| `streets` | line | every way with `highway`, and `railway` = `rail`, `narrow_gauge`, `tram`, `light_rail`, `funicular`, `subway`, `monorail` | the `highway` value, else the `railway` value | 3,496 |
| `public_transport` | point | `highway=bus_stop`, `railway` = `station`, `halt`, `tram_stop`; `amenity` = `bus_station`, `ferry_terminal`; `aeroway` = `aerodrome`, `helipad`; `aerialway=station` | `bus_stop`, `station`, `halt`, `tram_stop`, `ferry_terminal`, `aerialway_station`, … | 107 |
| `pois` | point | `amenity` (not parking or transport), `shop`, `tourism`, `office`, `leisure` (not the `land` areas), `man_made` | the first of those tags that is set | 1,645 |
| `traffic` | point | `highway` = `traffic_signals`, `crossing` | the `highway` value | 575 |
| `place_labels` | point | `place` = `city`, `town`, `village`, `hamlet`, `suburb`, `quarter`, `neighbourhood`, `isolated_dwelling`, `farm`, `island`, `locality` | the `place` value | 10 |

`traffic` is not a Shortbread layer. On a crossing, `bearing` is the direction of the road it lies
on, in degrees from north, so a map can draw the zebra marking across the road; NULL on traffic
signals.

`streets` is every OSM way with a `highway` tag, whatever the modes keep; it is for drawing, not
routing. Routable roads are in the [mode schemas](database.md#mode-schema).

Not built: a `boundaries` layer (administrative borders); use
[`duckosm admin`](../guides/admin-boundaries.md) instead. The full lists of tag values are in
`src/duckosm/features/layers.py`.
