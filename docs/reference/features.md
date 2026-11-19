# Base-map layers

The `features` schema holds what a base map draws in addition to the networks: water, land use,
buildings, points of interest, place names. Each layer is one table, with the classes of the
[Shortbread](https://shortbread-tiles.org/) vector-tile schema. [`duckosm viz`](../guides/draw-map.md) and
[duckOSM Studio](../guides/studio.md) draw them.

## Build them

Each build from a PBF makes them, at the end of the build, from `raw.*`.

| Build | Makes the layers | Turn off |
|---|---|---|
| `duckosm build` | always | `--no-features`, or `options.build_features: false` |
| `duckosm` (native) | with `--boundary` and `--features` | omit `--features` |

With a boundary, the build cuts the PBF with the `smart` strategy (`options.clip_strategy`). Thus rivers and
land-use areas that cross the border stay complete. A database cut from a larger database has no `raw.*`, so it
cannot make the layers. If one layer fails, the build writes a warning and continues.

The two builds give the same rows. `features.ocean` needs the network (`options.sea`, or `--sea` in the native
build). Without the network, the table is empty and the build continues.

On a database that still has `raw.*`, you can make the layers later:

```python
import duckdb
from duckosm.features import FeaturesBuilder

con = duckdb.connect("monaco.duckdb")
FeaturesBuilder(con).run()              # -> {layer: row count}
```

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

`features.traffic` has one more column: `bearing` (DOUBLE). `features.ocean` has `source` (VARCHAR) in place of `tags`.

An **area** layer holds closed ways and multipolygon relations. A **line** layer holds ways. A
**point** layer holds nodes.

## Layers

In drawing order, bottom first. The Monaco column is an example from the sample build: it changes with the date of the OSM file.

| Layer | Shape | From OSM | `kind` | Monaco |
|---|---|---|---|---|
| `ocean` | area | the sea: not from the extract (OSM has only `natural=coastline` lines) but Overture Maps' ocean polygons, built from OSM's coastline, read for the area's box (the boundary's, else the extract's; +10 %, at least 500 m) | `ocean` | 1 |
| `land` | area | `landuse` = `forest`, `grass`, `residential`, `industrial`, `cemetery`, …; `natural` = `wood`, `scrub`, `heath`, `sand`, `beach`, `wetland`, `shrubbery`, …; `leisure` = `park`, `garden`, `pitch`, `stadium`, `playground`, `beach_resort`, …; `amenity=grave_yard` | the `landuse` value, else `natural`, else `leisure`, else `amenity`; `natural=wood` becomes `forest` | 241 |
| `water_polygons` | area | `natural` = `water` / `glacier`, `waterway=riverbank`, `landuse` = `reservoir` / `basin`, any `water` tag, `leisure=swimming_pool` | `water`, `river`, `canal`, `reservoir`, `basin`, `dock`, `glacier`, `swimming_pool` | 88 |
| `water_lines` | line | `waterway` = `river`, `stream`, `canal`, `ditch`, `drain` | the value; `drain` becomes `ditch` | 4 |
| `sites` | area | `amenity` = `parking`, `bicycle_parking`, `bus_station`, `university`, `college`, `school`, `kindergarten`, `hospital`, `prison`; `public_transport` = `platform` / `station`; `leisure=sports_centre`, `landuse=construction`, `military=danger_area`, `man_made=bridge`, `tourism=zoo`, `railway=platform` | the value | 80 |
| `paved` | area | open ground that people stand on: `place=square`; `area:highway=*`; a pedestrian, footway, service, living-street, steps, path or cycleway `highway` with `area=yes` (or a multipolygon relation); `man_made` = `pier`, `breakwater`, `groyne`, `quay`; `aeroway` = `helipad` / `heliport`; `amenity=marketplace` | `square`, the `area:highway` value, the `highway` value, `pier`, `quay`, `helipad`, `marketplace`, … | 83 |
| `buildings` | area | `building`, except `building=no` | the raw `building` value (`yes` is common); not a Shortbread class | 1,262 |
| `streets` | line | every way with `highway`, and `railway` = `rail`, `narrow_gauge`, `tram`, `light_rail`, `funicular`, `subway`, `monorail` | the `highway` value, else the `railway` value | 3,496 |
| `public_transport` | point | `highway=bus_stop`, `railway` = `station`, `halt`, `tram_stop`; `amenity` = `bus_station`, `ferry_terminal`; `aeroway` = `aerodrome`, `helipad`; `aerialway=station` | `bus_stop`, `station`, `halt`, `tram_stop`, `ferry_terminal`, `aerialway_station`, … | 107 |
| `pois` | point | `amenity` except `parking`, `bicycle_parking`, `bus_station`, `ferry_terminal`; `shop`, `tourism`, `office`; `leisure` except the values used by `land` and `sites`; `man_made` | the first of those tags that is set | 1,645 |
| `traffic` | point | `highway` = `traffic_signals`, `crossing` | the `highway` value | 575 |
| `place_labels` | point | `place` = `city`, `town`, `village`, `hamlet`, `suburb`, `quarter`, `neighbourhood`, `isolated_dwelling`, `farm`, `island`, `locality` | the `place` value | 10 |

Notes on some layers:

| Layer | Note |
|---|---|
| `ocean` | from Overture Maps (`options.sea: overture`, the default; `false` turns it off). The build reads Overture's public files on S3, so it needs the network. `source` names the Overture release; `osm_id` and `name` are NULL. [Design](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/sea.md) |
| `paved` | not a Shortbread layer. The same ways (a pedestrian area, a pier) also stay lines in `streets`. [Design](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/missing_polygons.md) |
| `traffic` | not a Shortbread layer. On a crossing, `bearing` is the direction of the road under it, in degrees from north, so a map can draw the zebra marking across the road. NULL on traffic signals, and on a crossing that is on no road |
| `streets` | all OSM ways with a `highway` tag, for drawing, not for routing. The routable roads are in the [mode schemas](database.md#mode-schema) |

There is no `boundaries` layer (administrative borders). Use [`duckosm admin`](../guides/admin-boundaries.md).
The full lists of tag values are in
[`src/duckosm/features/layers.py`](https://github.com/Khoshkhah/duckOSM/blob/main/src/duckosm/features/layers.py).

## Check what the build leaves out

OSM has polygons that no layer uses. `duckosm features-check DB` lists the closed ways with tags and the area relations of
`raw.*` that are in no `features.*` table. It groups them by their main tag, with the number of ways and relations and the
area of the ways.

```bash
duckosm features-check monaco.duckdb
```

| Group | Means |
|---|---|
| with a reason in the last column | the build leaves it out on purpose: zones such as `place=neighbourhood`, boundaries, the coastline, parts of buildings, street furniture |
| with no reason | a possible miss: [open an issue](https://github.com/Khoshkhah/duckOSM/issues) |

A way that is a ring of a relation in a table is not in the list. The check needs `raw.*`, so it cannot check a
database cut from a larger database.
