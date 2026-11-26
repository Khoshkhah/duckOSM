# Base-map layers

The `features` schema holds what a base map draws in addition to the networks: water, land use,
buildings, points of interest, place names. Each layer is one table, with the classes of the
[Shortbread](https://shortbread-tiles.org/) vector-tile schema. [`duckosm viz`](../guides/draw-map.md) and
[duckOSM Studio](../guides/studio.md) draw them.

## Build them

| Command | Makes the layers |
|---|---|
| `duckosm FILE --boundary B --features` | yes |
| `duckosm build` | yes; `--no-features` or `options.build_features: false` turns them off |

The layers are made from `raw.*`, at the end of the build, so they take your [OSM fixes](../guides/edit-osm-data.md). With a boundary, each way that crosses the border stays
whole, so rivers and land-use areas stay complete. A database cut from a larger one has no `raw.*`, so it has no layers.
`features.ocean` needs the network (`--sea`); without it, the table is empty. If one layer fails, the build writes a
warning and goes on.

## Columns

Every table has the same columns:

| Column | Type | Description |
|---|---|---|
| `osm_id` | BIGINT | the OSM id; unique only together with `osm_type` |
| `osm_type` | VARCHAR | `node`, `way` or `relation` |
| `kind` | VARCHAR | the class, from the rules below |
| `name` | VARCHAR | the OSM `name` |
| `tags` | MAP(VARCHAR, VARCHAR) | every OSM tag |
| `geom` | GEOMETRY | the shape, in longitude and latitude |

`features.traffic` has one more column: `bearing` (DOUBLE). `features.ocean` has `source` (VARCHAR) in place of `tags`.

An **area** layer holds closed ways and multipolygon relations. A **line** layer holds ways. A
**point** layer holds nodes.

## Layers

In drawing order, bottom first.

| Layer | Shape | From OSM | `kind` |
|---|---|---|---|
| `ocean` | area | the sea, from Overture Maps' ocean polygons (OSM has only coastline lines) | `ocean` |
| `land` | area | `landuse` = `forest`, `grass`, `residential`, `industrial`, `cemetery`, …; `natural` = `wood`, `scrub`, `heath`, `sand`, `beach`, `wetland`, `shrubbery`, …; `leisure` = `park`, `garden`, `pitch`, `stadium`, `playground`, `beach_resort`, …; `amenity=grave_yard` | the `landuse` value, else `natural`, else `leisure`, else `amenity`; `natural=wood` becomes `forest` |
| `water_polygons` | area | `natural` = `water` / `glacier`, `waterway=riverbank`, `landuse` = `reservoir` / `basin`, any `water` tag, `leisure=swimming_pool` | `water`, `river`, `canal`, `reservoir`, `basin`, `dock`, `glacier`, `swimming_pool` |
| `water_lines` | line | `waterway` = `river`, `stream`, `canal`, `ditch`, `drain` | the value; `drain` becomes `ditch` |
| `sites` | area | `amenity` = `parking`, `bicycle_parking`, `bus_station`, `university`, `college`, `school`, `kindergarten`, `hospital`, `prison`; `public_transport` = `platform` / `station`; `leisure=sports_centre`, `landuse=construction`, `military=danger_area`, `man_made=bridge`, `tourism=zoo`, `railway=platform` | the value |
| `paved` | area | open ground that people stand on: `place=square`; `area:highway=*`; a pedestrian, footway, service, living-street, steps, path or cycleway `highway` with `area=yes` (or a multipolygon relation); `man_made` = `pier`, `breakwater`, `groyne`, `quay`; `aeroway` = `helipad` / `heliport`; `amenity=marketplace` | `square`, the `area:highway` value, the `highway` value, `pier`, `quay`, `helipad`, `marketplace`, … |
| `buildings` | area | `building`, except `building=no` | the raw `building` value (`yes` is common); not a Shortbread class. [`duckosm overture-buildings`](cli.md#overture-buildings) adds heights, levels and roofs from Overture Maps to `tags` |
| `streets` | line | every way with `highway`, and `railway` = `rail`, `narrow_gauge`, `tram`, `light_rail`, `funicular`, `subway`, `monorail` | the `highway` value, else the `railway` value |
| `public_transport` | point | `highway=bus_stop`, `railway` = `station`, `halt`, `tram_stop`; `amenity` = `bus_station`, `ferry_terminal`; `aeroway` = `aerodrome`, `helipad`; `aerialway=station` | `bus_stop`, `station`, `halt`, `tram_stop`, `ferry_terminal`, `aerialway_station`, … |
| `pois` | point | `amenity` except `parking`, `bicycle_parking`, `bus_station`, `ferry_terminal`; `shop`, `tourism`, `office`; `leisure` except the values used by `land` and `sites`; `man_made` | the first of those tags that is set |
| `traffic` | point | `highway` = `traffic_signals`, `crossing` | the `highway` value |
| `place_labels` | point | `place` = `city`, `town`, `village`, `hamlet`, `suburb`, `quarter`, `neighbourhood`, `isolated_dwelling`, `farm`, `island`, `locality` | the `place` value |

Notes on some layers:

| Layer | Note |
|---|---|
| `ocean` | Overture's polygons are made from OSM's coastline. The build reads them for the box of the area (plus 10 %, at least 500 m), from Overture's public files, so it needs the network. `source` names the Overture release; `osm_id` and `name` are NULL. `options.sea: false` turns it off. |
| `paved` | not a Shortbread layer. The same ways (a pedestrian area, a pier) also stay lines in `streets`. |
| `traffic` | not a Shortbread layer. On a crossing, `bearing` is the direction of the road under it, in degrees from north, so a map can draw the zebra marking across the road. NULL on traffic signals, and on a crossing that is on no road |
| `streets` | all OSM ways with a `highway` tag, for drawing, not for routing. The routable roads are in the [mode schemas](database.md#mode-schema) |

There is no `boundaries` layer (administrative borders). Use [`duckosm admin`](../guides/admin-boundaries.md).
The full lists of tag values are in
[`rust/duckosm-build/sql/features.sql`](https://github.com/Khoshkhah/duckOSM/blob/main/rust/duckosm-build/sql/features.sql).

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
