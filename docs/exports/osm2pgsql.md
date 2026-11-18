# osm2pgsql tables (PostGIS)

`duckosm quack --osm2pgsql` writes the four tables that osm2pgsql writes with its default settings. Many PostGIS
map servers and map styles (for example [OpenStreetMap Carto](https://github.com/gravitystorm/openstreetmap-carto))
read these tables. duckOSM also takes your OSM fixes from **Edit OSM data** (`--fixes`).

| Reference and format | |
|---|---|
| Reference tool | [osm2pgsql](https://osm2pgsql.org/doc/manual.html) 2.3.1, the `pgsql` output with [`default.style`](https://github.com/osm2pgsql-dev/osm2pgsql/blob/master/default.style) and `--hstore` |
| Tables | `planet_osm_point`, `planet_osm_line`, `planet_osm_polygon`, `planet_osm_roads` |
| Format | [PostGIS](https://postgis.net/) tables (native only), GeoParquet (one file for each table), DuckDB |
| Geometry | EPSG:3857, as osm2pgsql; `--latlong` gives EPSG:4326 |
| Check | the Monaco tables of osm2pgsql and of duckOSM, row by row (below) |

```bash
duckosm quack monaco.osm.pbf --osm2pgsql -o out --name monaco                    # 4 GeoParquet files
duckosm quack monaco.osm.pbf --osm2pgsql -o out --name monaco -f duckdb          # out/monaco_osm2pgsql.duckdb
duckosm quack monaco.osm.pbf --osm2pgsql --postgis "host=localhost dbname=gis"  # into PostGIS, as osm2pgsql
duckosm quack monaco.osm.pbf --osm2pgsql -o out --fixes osm_fixes.yaml           # with the OSM fixes
```

The PostGIS database must have the extensions `postgis` and `hstore` (duckOSM makes them if it has the right to do
so). Tables of the same names are replaced.

## The tables

| Table | What goes in | `osm_id` |
|---|---|---|
| `planet_osm_point` | each node with a tag that the style keeps | the node id |
| `planet_osm_line` | each way with a tag that the style keeps that is not a polygon; the member ways of each relation `route` and `boundary`, joined | the way id; minus the relation id |
| `planet_osm_polygon` | each closed way that is a polygon; each polygon of a relation `multipolygon` or `boundary` | the way id; minus the relation id |
| `planet_osm_roads` | the rows of `planet_osm_line` for major roads, railways and administrative boundaries | as in `planet_osm_line` |

The columns are those of `default.style`: `osm_id`, one text column for each key of the style, `z_order` (int4),
`way_area` (real, not in `planet_osm_point`), `tags` (the other tags, an hstore; a map in GeoParquet and DuckDB) and
the geometry `way`. The rules:

- **Deleted tags.** The style deletes some tags (for example `note`, `source`, `created_by`, `tiger:*`). An object
  with no other tag is not a row.
- **Polygon or line.** A closed way is a polygon when it has a key that the style marks `polygon` (for example
  `building`, `landuse`, `amenity`), or `area=yes`. `area=no` makes it a line. `natural=coastline` is removed, and the way
  becomes a polygon with `area=yes`.
- **`z_order`.** 100 × `layer`, plus a value for the `highway` type (for example 39 for `motorway`, 33 for
  `residential`), plus 35 for a railway, plus 100 for a bridge, minus 100 for a tunnel.
- **Roads.** A line goes into `planet_osm_roads` too when it is a major road (`motorway`, `trunk`, `primary`,
  `secondary` and their links), a railway or `boundary=administrative`.
- **Relations.** A route gets the extra tags of osm2pgsql (`route_name`, `route_pref_color`, and `lcn`, `rcn`, ...
  from `network`). The polygons of a multipolygon are made from all member ways (the roles are not used); a
  multipolygon of two or more polygons is one row for each polygon.
- **Long lines** are cut into parts of at most 100 km (EPSG:3857).
- **Valid polygons only.** In EPSG:3857, a polygon that is not valid is not a row (osm2pgsql's `ST_IsValid` check).

## The Monaco check

`tests/data/monaco.osm.pbf` on the Mac mini: osm2pgsql 2.3.1 (`osm2pgsql --hstore`, default style, no `--slim`) into
one database, `duckosm quack --osm2pgsql --postgis` into a second one, PostgreSQL 18 with PostGIS 3.7. The script
[`tests/osm2pgsql/check.sh`](https://github.com/Khoshkhah/duckOSM/blob/main/tests/osm2pgsql/check.sh) runs both, and
[`compare.sql`](https://github.com/Khoshkhah/duckOSM/blob/main/tests/osm2pgsql/compare.sql) compares the tables row by
row.

| Table | Rows, osm2pgsql | Rows, duckOSM | Rows that differ |
|---|---|---|---|
| `planet_osm_point` | 3959 | 3959 | 0 |
| `planet_osm_line` | 3919 | 3919 | 0 |
| `planet_osm_polygon` | 1837 | 1837 | 0 |
| `planet_osm_roads` | 422 | 422 | 0 |

Each row has the same `osm_id`, the same value in each column (also `z_order` and `way_area`), the same `tags`, and
the same geometry, **byte for byte** (the same points in the same order). The result is the same for the whole file,
tile by tile (`--tile-zoom 12` and `14`), and for each tile alone with the join (the way of the browser).

The golden test `rust/duckosm-build/tests/quack_osm2pgsql_monaco.rs` checks this in CI against
`tests/data/monaco_osm2pgsql.jsonl.gz`: the tables of osm2pgsql, made with
[`golden.sql`](https://github.com/Khoshkhah/duckOSM/blob/main/tests/osm2pgsql/golden.sql).

To be equal to the byte, duckOSM calculates as osm2pgsql does:

- the EPSG:3857 coordinates with osmium's fast formula, and its multiply-adds fused as clang compiles them (else about half of the y values
  differ by 1 nm);
- `way_area` with Boost's formula, (x1 − x2)(y1 + y2), written with 6 digits, as osm2pgsql writes it;
- the rings as osmium makes them: each ring starts at its smallest point, the outer rings counterclockwise, the holes
  clockwise and in the order of their first point.

### Known difference

| Case | Difference | Cause |
|---|---|---|
| `--latlong` (EPSG:4326) | `way_area` differs in the 6th digit in 65 of 1837 polygons; the geometries are equal | In degrees, the area of a small polygon is often exactly halfway between two 6-digit values. Which way the value rounds then depends on the last bit of the sum. |

### The OSM fixes

With the fixes file of the tests (`tests/data/osm_fixes_quack.yaml`), exactly the rows of the fixed objects change,
and no other row:

| Fix | Rows |
|---|---|
| way 23715051 removed | its polygon goes |
| way 25722685, `keep_nodes` | its ring is open now: a line in place of a polygon |
| way 4227277: tags | its line and its road have the new tags |
| way 503462470: `oneway`, `lanes` | its line and its road have `oneway=yes`, `lanes=3` |
| node 21911886: tag edits | its point has the new tags |
| relation 393226: `name` | its polygon (`osm_id` −393226) has the new name |
| new ways 9000000000001 and 9000000000002 | two new lines |

### Time and memory

Mac mini (12 cores, 16 GB), Monaco (0.6 MB), the best of 3 runs. The memory is the peak of the program (PostgreSQL's
own memory is not in it).

| | osm2pgsql | duckOSM |
|---|---|---|
| Into PostGIS | 0.25 s, 29 MB | 0.25 s, 52 MB |
| GeoParquet (4 files, 5.8 MB) | | 0.09 s, 62 MB |
| DuckDB (4.7 MB) | | 0.19 s, 139 MB |
| Tables in PostGIS | 4.7 MB | 4.7 MB |

Monaco is too small to show a difference in time: most of the time is PostgreSQL (COPY, the sort by geometry and the
GiST indexes), which is the same for both.
