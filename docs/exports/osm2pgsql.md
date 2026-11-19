# osm2pgsql tables

`duckosm quack --osm2pgsql` writes the four tables that osm2pgsql writes with its default settings, **with your OSM
fixes**. Many PostGIS map servers and map styles read these tables, for example
[OpenStreetMap Carto](https://github.com/gravitystorm/openstreetmap-carto).

| Reference and format | |
|---|---|
| Format | four tables: into [PostGIS](https://postgis.net/), as GeoParquet (one file for each table) or as a DuckDB file |
| Reference tool | [osm2pgsql](https://osm2pgsql.org/doc/manual.html) 2.3.1, output `pgsql`, with [`default.style`](https://github.com/osm2pgsql-dev/osm2pgsql/blob/master/default.style) and `--hstore` |
| Check | on Monaco, each row equals osm2pgsql's, byte for byte, geometry too. [How each output is checked](../concepts/checks.md) |

```bash
duckosm quack monaco.osm.pbf --osm2pgsql -o out --name monaco                     # 4 GeoParquet files
duckosm quack monaco.osm.pbf --osm2pgsql --postgis "host=localhost dbname=gis"   # into PostGIS, as osm2pgsql
duckosm quack monaco.osm.pbf --osm2pgsql -o out --name monaco --fixes osm_fixes.yaml
```

| Option | Does |
|---|---|
| `--postgis CONNINFO` | writes into PostGIS. The database needs the extensions `postgis` and `hstore`. Tables of the same names are replaced. Not in the browser |
| `-f duckdb` | one DuckDB file, in place of the GeoParquet files |
| `--latlong` | geometry in EPSG:4326. The default is EPSG:3857, as osm2pgsql |
| `--fixes`, `--tile-zoom`, `--no-sort` | as for the [QuackOSM table](../concepts/quackosm.md) |

## The tables

| Table | What goes in |
|---|---|
| `planet_osm_point` | each node with a tag that the style keeps |
| `planet_osm_line` | each way that is not a polygon, and the joined member ways of each `route` and `boundary` relation |
| `planet_osm_polygon` | each closed way that is a polygon, and each polygon of a `multipolygon` or `boundary` relation |
| `planet_osm_roads` | the rows of `planet_osm_line` for major roads, railways and administrative boundaries |

Each table has these columns:

| Column | Type | Holds |
|---|---|---|
| `osm_id` | int8 | the node or way id. A row from a relation has minus the relation id |
| one column for each key of `default.style` | text | the value of the tag, for example `highway`, `name`, `building`, `landuse` |
| `z_order` | int4 | the drawing order: from `layer`, the road class, railway, bridge and tunnel |
| `way_area` | real | the area, in the units of the projection. Not in `planet_osm_point` |
| `tags` | hstore | all other tags. In GeoParquet and DuckDB, a map |
| `way` | geometry | the shape, in EPSG:3857 (or EPSG:4326 with `--latlong`) |

The rules are those of osm2pgsql: the style removes some tags (`note`, `source`, `created_by`, …), a closed way with a
`polygon` key of the style (`building`, `landuse`, …) or `area=yes` is a polygon, a line longer than 100 km is cut into
parts, and a polygon that is not valid is not written.

## OSM fixes

`--fixes` applies the fixes file of [Fix the OSM data](../guides/edit-osm-data.md), with the same rules as the
[QuackOSM table](../concepts/quackosm.md#osm-fixes). osm2pgsql has no such option. On Monaco, only the rows of the
fixed objects change: a removed way loses its polygon, a `keep_nodes` fix makes a ring a line, and the new ways are
new lines.

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB), PostgreSQL 18 with PostGIS 3.7.

| Area (input) | | osm2pgsql 2.3.1 | duckOSM into PostGIS | duckOSM to GeoParquet |
|---|---|---|---|---|
| Monaco (0.6 MB) | Time | 0.25 s | 0.25 s | **0.09 s** |
| | Memory | 29 MB | 52 MB | 62 MB |
| | Size | 4.7 MB of tables | 4.7 MB of tables | 5.8 MB of files |
| Kentucky (148 MB) | Time | **12.4 s** | 21.2 s | 14 s |
| | Memory | **0.62 GB** | 1.22 GB | 1.3 GB |
| | Size | 864 MB of tables | 863 MB of tables | 426 MB of files |
| France (5.1 GB) | | measuring | measuring | measuring |

- **osm2pgsql writes only into PostgreSQL.** So compare its column with "duckOSM into PostGIS": the same four tables, with the same size. GeoParquet is an extra output of duckOSM, with no PostgreSQL.
- **Into PostGIS, duckOSM is 1.7 times slower than osm2pgsql** on Kentucky. On Monaco, most of the time is
  PostgreSQL's own work, the same for both.
- The memory does not include PostgreSQL's own memory.

With `--latlong` (not the default), `way_area` differs in the 6th digit for a few polygons (65 on Monaco).

How duckOSM gets the same bytes, and the test files:
[docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md), "osm2pgsql tables".
