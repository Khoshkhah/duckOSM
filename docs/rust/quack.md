# The features table of QuackOSM (`duckosm-rust quack`)

Status: done, 2026-11-13. Kaveh asked for a duckOSM output that is the same as the output of QuackOSM, so that the two tools
can be compared on the same rows. QuackOSM (kraina-ai) is a Python library that converts an `.osm.pbf` file to one table of
features in GeoParquet or DuckDB.

## The command

```bash
duckosm-rust quack kentucky.osm.pbf -o out --name kentucky -f duckdb,geoparquet
```

- `-f geoparquet` writes `out/kentucky_quack.parquet` (GeoParquet 1.0, WKB, zstd level 3, row groups of 100 000 rows, as
  QuackOSM writes it).
- `-f duckdb` writes the table `main.quackosm` (the table name of QuackOSM) into `out/kentucky_quack.duckdb`. The geometry
  column has DuckDB's `GEOMETRY` type.
- `-f duckdb,geoparquet` writes the two files in one pass.
- `--threads` sets the number of blocks that are decoded at the same time. The default is all the cores.

The input must be sorted by type and id (nodes, then ways, then relations), as the files of Geofabrik are. To make the table of
an area, cut the region file first with `duckosm-rust cut`.

The table has three columns, as in QuackOSM: `feature_id` (`node/<id>`, `way/<id>` or `relation/<id>`), `tags`
(`MAP(VARCHAR, VARCHAR)`) and `geometry`.

## The rules

The rules are the ones of QuackOSM 0.20.0 with its default options (no filters, `ignore_metadata_tags`, compact tags), from its
`pbf_file_reader.py`:

- A node with tags is a POINT.
- A way with tags, with 2 or more nodes and all its nodes in the file, is a LINESTRING. It is a POLYGON when it is closed, has
  4 or more points after the repeated points are removed, and its tags match the polygon rules of QuackOSM
  (`osm_way_polygon_features.json`: `area=yes`, 32 keys, 6 allow lists and 13 deny lists; `area=no` is always a line).
- A relation of the type `multipolygon` or `boundary` with all its member ways in the file is a POLYGON or a MULTIPOLYGON:
  1. The member lines of each role are joined as GEOS `LineMerge` joins them: only at the points where exactly two line ends
     meet.
  2. If one ring is not closed, the relation is not in the table.
  3. An empty role is `outer` (QuackOSM: `COALESCE(ref_role, 'outer')`; `ST_ReadOSM` gives NULL for an empty role). If no
     member has the role `outer`, all rings are outer rings.
  4. Each outer polygon loses the inner polygons that are in it. The union of the outer polygons is the geometry.
- The metadata tags (`area`, `created_by`, `converted_by`, `source`, `time`, `ele`, `note`, `todo`, `fixme`, `FIXME` and the keys
  that start with `openGeoDB:`) are not in the tags. An element with no other tag is not in the table. The polygon rules read
  the tags before this step.
- The coordinates are rounded to 7 decimals, as QuackOSM rounds them.

## How duckOSM makes it

All of it is Rust: `duckosm_core::quack` (the rows), `duckosm_parquet::quack` (the GeoParquet file) and
`duckosm_duckdb::quack` (the `.duckdb` table). There is no Python and no DuckDB spatial extension. The same code builds for
WebAssembly.

1. One pass over the file with duckOSM's block decoder (`pbf::stream_raw`, the blocks decoded in parallel). A node keeps only
   its rounded point. A node with tags becomes a row at once. A way becomes a row as soon as it is read, because its nodes
   came before it. The node lists of the ways stay for the relations.
2. The relations at the end, in parallel.
3. The rows go to the writer in parts, while the file is read. For the `.duckdb` file, a DuckDB table function in Rust
   (`quack_features`) gives the rows to DuckDB's own insert, and DuckDB's core function `ST_GeomFromWKB` makes the
   `GEOMETRY`. No Parquet file is between the two. QuackOSM writes its Parquet file first and then loads it into DuckDB.

The polygon operations use the pure-Rust crates `geo` 0.33 and `i_overlay` 4.5 (both were already in duckOSM's lock file):

- **The ring check.** A ring is valid when no two of its segments meet other than at the point that they share. `geo`'s
  `Validation` compares each segment with each other segment. A boundary ring of 50 000 points took seconds, and five large
  relations of Kentucky took 6 s together. `quack.rs` finds the pairs that meet with a sweep
  (`geo::algorithm::sweep::Intersections`), and the five relations now take 1 s.
- **The repair of an invalid ring.** i_overlay with the even-odd fill rule. This gives the area of GEOS's `ST_MakeValid`.
  `geo`'s `MakeValid` (a triangulation) gave a wrong small triangle for some self-crossing rings.
- **The union and the difference.** i_overlay, called directly with `preserve_input_collinear` and
  `preserve_output_collinear`. GEOS keeps a point that is on a straight edge, and `geo`'s `BooleanOps` removes it. The rings are
  oriented first (shells counterclockwise), because i_overlay takes the fill rule from the direction of the first ring and OSM
  rings come in both directions. The result goes back on the 7-decimal grid: i_overlay calculates on integers and gives the
  input points back with an error of about 1e-14.
- **Fast paths.** When the outer polygons do not meet, the union is the polygons as they are. When the inner polygons are
  single rings, away from the border of the outer polygon and from each other, the difference is the outer ring with the
  inner rings as holes. GEOS gives the same point set in these cases.

## The result on Kentucky

Input: `kentucky-area.osm.pbf` (141 MB). Mac mini (Apple silicon, 12 cores, 16 GB). QuackOSM 0.20.0 with DuckDB 2.0.0. Two runs each.
The memory is the peak RSS of the process and all its child processes (QuackOSM starts a worker process). QuackOSM sorts the
rows by default (`sort_result=True`); duckOSM does not sort, so the table also gives QuackOSM without the sort.

| Output | QuackOSM | QuackOSM, no sort | duckOSM |
|---|---|---|---|
| GeoParquet: time | 14.0 s | 13.1 to 13.3 s | 3.2 s |
| GeoParquet: peak memory | 5.7 to 5.8 GB | 5.8 GB | 1.4 GB |
| GeoParquet: file | 297 MB | 333 MB | 320 MB |
| `.duckdb`: time | 14.8 to 14.9 s | 14.0 s | 3.4 to 3.6 s |
| `.duckdb`: peak memory | 5.8 GB | 5.8 GB | 1.8 GB |
| `.duckdb`: file | 1.2 GB | 1.3 GB | 675 MB |
| CPU time (GeoParquet) | 71.7 s | | 11.0 s |

Both outputs of duckOSM in one pass take 5.0 s and 2.3 GB.

The rows, compared with `cargo run --release --example quack_diff -p duckosm-duckdb -- out/kentucky_quack.duckdb quackosm.parquet`:

- 2 393 139 rows in each table: 556 759 nodes, 1 829 301 ways and 7 079 relations. The ids are the same.
- All rows have the same tags.
- 2 393 125 rows (99.9994 %) have an equal geometry (`ST_Equals`). The other 14 rows have the same area to 8e-8 of it:
  - 12 rows have a point where two edges cross. i_overlay and GEOS calculate this point to a different last digit.
  - 2 ways are rings that touch themselves. GEOS gives a GEOMETRYCOLLECTION of the polygon and the lines that enclose no area;
    duckOSM gives the polygon.

On Monaco (`tests/data/monaco.osm.pbf`) all 9 586 rows are equal. `duckosm-core/tests/quack_monaco.rs` checks the rows of each
kind, the rows of each geometry type and the area against the numbers of QuackOSM.

## What QuackOSM has and duckOSM does not have

The filters: tags (`tags_filter`), OSM ids, custom SQL, and a geometry (`geometry_filter`, features that touch it). The
automatic download of an extract for a geometry, the geocoding, more than one PBF in one call, the tags as columns
(`explode_tags`) and the sort of the rows. `duckosm-rust quack` makes the table of QuackOSM's default options only.
