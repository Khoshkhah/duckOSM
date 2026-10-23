# DuckDB 2.0: what the upgrade needs

Status: 2026-10-23. The code runs on DuckDB 1.5.3 and on DuckDB 2.0.0. The pin stays `duckdb<2`. This note
lists the 2.0 changes that touch duckOSM, what is fixed now, and what the upgrade still needs.

## `edge_id` does not change

`hash()` gives the same values in 2.0.0 and 1.5.3. A test on 3,000 `edge_id` and virtual node ids gave
equal values. A Monaco build on 2.0 gives the same `driving.edges` as the duckosm-rust golden file
(`tests/data/monaco_driving_edges.tsv`).

## Fixed: the code works on 1.5.3 and on 2.0

| 2.0 change | What duckOSM does now |
|---|---|
| The lambda `x -> ...` is off by default. | Every lambda is `lambda x: ...`. DuckDB 1.5.3 accepts this form. |
| `ST_Length_Spheroid`, `ST_Area_Spheroid`, `ST_Distance_Sphere` and `COPY ... (FORMAT gdal, SRS ...)` stop with an error until `geometry_always_xy` is set. | Each connection that loads spatial also runs `SET geometry_always_xy = false`. This is the 1.5 behavior: the spheroid functions take (lat, lon), so the code keeps its `ST_FlipCoordinates`. |
| In Python, an `execute()` result that is not read to the end (`fetchone()`) stops `register()`, `create_function()` and `sql()` on the same connection. | `duckosm.utils.finish(con)` closes the open result before these calls. It calls `con.complete()` on 2.0 and does nothing on 1.5. |
| A column of `NULL` with no cast gets the type `"NULL"`, not `INTEGER`. | `extract.py` casts its `NULL` name. `gmns.py` casts the columns of `curb_seg`. |
| `ST_Read` geometry keeps its CRS in the stored type (`GEOMETRY('EPSG:4326')`). DuckDB 1.5 stores plain `GEOMETRY`. | `main.boundary`, the clip boundary and `admin_boundaries` cast to `GEOMETRY`. |

The other items of the migration guide do not touch duckOSM: `~` and `X'..'` are not used, and no SQL
does `//` or `%` with a divisor that can be zero.

## Not fixed: the upgrade needs these

1. **The file format.** By default, DuckDB 2.0 writes a file that DuckDB 1.5 cannot open. Until every
   tool that reads a duckOSM file uses 2.0 (sensor-matching, SonoFlow, fetching-sweden-data, duckosm-rust),
   open each output file with `config={"storage_compatibility_version": "v1.5.0"}`, or use
   `ATTACH ... (STORAGE_VERSION 'v1.5.0')`. A file made this way opens in 1.5.3 (tested).
2. **Python 3.11 or later.** The Python package of DuckDB 2.0 does not install on Python 3.10.
3. **Ties in `PathConnector`.** The order of the dangling path ends comes from a `GROUP BY` with no
   `ORDER BY`, and `any_value(highway)` picks one of two parallel edges. On Monaco, 2.0 gives other
   values than 1.5.3 for 148 connector edges (`edge_ref` and `is_reverse`, and `highway` on 4 edges).
   The `edge_id` values do not change. A 1.5.3 build with more than one thread can also change them.
   An `ORDER BY` and `min(highway)` make the two versions equal on all 53 tables, but they also change
   the 1.5.3 output. Kaveh must decide this change.
4. **The GMNS export.** On the same input database, the 2.0 export is not equal to the 1.5.3 export.
   Most differences are in lane geometry at about 1e-12 degrees. Some are larger: `any_value()` ties
   (for example `free_speed` in `gmns_all.link`, `gmns.py:731`) and the choice of `to_lane` in walking
   lane connectors. Two 1.5.3 exports with more than one thread also differ. Find the cause before
   the upgrade.
5. **roadstyle.** `visualization.level_edits` is written by roadstyle. With no edits, its columns get
   the type `"NULL"` on 2.0. roadstyle must cast them.

## How it was tested

- The test suite on 1.5.3, with `lambda_syntax = 'DISABLE_SINGLE_ARROW'` on every connection (the
  2.0 rule), and on 2.0.0 (Python 3.12).
- Monaco builds (`config/sample_monaco.yaml` and the duckosm-rust golden config), the GMNS export
  with `--meso --micro --combined` and `features-check`, with one thread, compared table by table
  with a build of the code before this change.
