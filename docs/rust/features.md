# The base-map layers (`features.*`) in the app

Status: done, 2026-10-22 (the result is at the end). Kaveh: "also we should add features for creating too not only 3 transport
mode". Kaveh chose approach A. The other questions got the defaults of this document: the box is on, the files have no `tags`, and
the native `--features` and the sea come later.

## What the user gets

- Step 4 (Build) has one more check box: **Base map layers**, under the three modes.
- With the box on, the build also makes duckOSM's `features.*` tables of the area: water, land use, buildings, streets and railways,
  sites, paved areas, public transport, points of interest, traffic signals and crossings, and place names. These are the tables of
  duckOSM's [base-map layers](https://khoshkhah.github.io/duckOSM/reference/features/), with the same rows.
- Step 5 (the result):
  - **Map**: the **Base map** list gets the option **duckOSM layers** (the default after a build with the layers). roadstyle's
    module draws the layers under the roads, in the look of the roads map.
  - **SQL**: the tables are `features.land`, `features.buildings`, and so on, as in a duckOSM `.duckdb` file.
  - **Downloads**: one block for the layers, and in the zip one `features_<table>.parquet` file for each table, with its entry in
    `layers.json`, as duckOSM's GeoParquet export writes them.

## What duckOSM does

1. **The cut.** With a boundary and the layers on, duckOSM cuts the region file with osmium's `smart` strategy, not
   `complete_ways`. `smart` adds the members of the multipolygon relations that touch the area: their ways and the nodes of these
   ways. So a lake or a forest at the border stays whole. The cut goes into `raw.nodes` (all nodes, with their tags), `raw.ways`
   (the ways with 2 or more nodes) and `raw.relations`.
2. **The changes to `raw.ways`.** The OSM fixes and the sidewalk rule change `raw.ways` before the layers are made. In the Monaco
   golden, 49 ways differ from the cut: 7 from the fixes, 42 get `sidewalk=separate` and `duckosm:sidewalk=inferred`. The layers
   keep all tags, so `features.streets` shows these changes.
3. **The layers** (`src/duckosm/features/`, about 680 lines): DuckDB spatial SQL over `raw.*`. A temporary schema `geom` holds one
   line for each way and one area for each multipolygon or boundary relation (`ST_BuildArea(ST_LineMerge(ST_Collect(...)))`, which
   is GEOS). Then each layer is one `CREATE TABLE features.<name> AS SELECT ...` with a tag filter and a `kind` expression. The
   `traffic` layer also gets the `bearing` of each crossing.
4. **The sea** (`features.ocean`): not from OSM. duckOSM reads Overture Maps' ocean polygons from S3 for the box of the area.

## Measured on 2026-10-22

I ran duckOSM's own feature SQL (recorded from `FeaturesBuilder` on duckOSM's `raw.*`) natively and in duckdb-wasm in the app
(headless Chromium, `serve.py` with the COOP and COEP headers):

| | Monaco (golden `raw.*`) | Tartu (osmium `smart` cut of the Estonia file) |
|---|---|---|
| `raw.*` rows: nodes / ways / relations | 34,326 / 5,653 / 336 | 273,651 / 45,619 / 1,087 |
| `raw.*` as Parquet | | 8.3 MB (nodes 5.6 MB) |
| Feature SQL, duckOSM native (DuckDB 1.5.3) | 0.21 s | 0.52 s |
| Feature SQL in the page (duckdb-wasm, DuckDB 1.5.4) | 0.54 s | 1.1–1.4 s |
| DuckDB memory in the page after the SQL | | 58 MiB |
| Rows: native = page | all 11 tables | all 11 tables |

- **The spatial extension loads in the page**, with and without the COEP headers: `INSTALL spatial; LOAD spatial` takes 1.1–1.4 s
  (spatial `28db190`; duckOSM native has `b68b309`). The file comes from `extensions.duckdb.org` with
  `Access-Control-Allow-Origin: *`: 23.6 MB, 4.5 MB compressed. GEOS works (`ST_BuildArea`, `ST_LineMerge`).
- **The page gives the same rows as duckOSM.** For each table I compared `osm_id`, `osm_type`, `kind`, `name`, `tags` and the WKB
  bytes of `geom`. All are equal. The only difference: `traffic.bearing` differs in the last bits (the math functions of
  WebAssembly). Rounded to 1e-9 degrees, it is equal.
- The native rerun of the SQL on the golden `raw.*` gives the golden `features.*`, byte for byte.
- **The `smart` cut costs little more than `complete_ways`.** Tartu with osmium: 3.0 s against 2.0 s, and 9,741 more nodes and 164
  more ways (4 % and 0.4 %).
- Overture's S3 bucket sends `Access-Control-Allow-Origin: *`, so the page can read the sea later.

## Two approaches

**A. Rust writes `raw.*`, duckdb-wasm runs duckOSM's feature SQL.** Rust cuts the region file as osmium's `smart` does and writes
`raw_nodes.parquet`, `raw_ways.parquet` and `raw_relations.parquet`. The page loads them into `raw.*` and runs the feature SQL,
which is the SQL of duckOSM, unchanged. Since 2026-10-25 the page then drops `raw.*`: it was more than half of DuckDB's memory
(Monaco 50 of 81 MiB), the map does not read it, and no download has it ([app.md](app.md), "The memory of the page").

**B. A Rust port of the features step.** Rust makes the lines, the polygons and the 11 tables itself and writes them as GeoParquet
files, as the network step does.

| | A: Rust `raw.*` + the SQL | B: port to Rust |
|---|---|---|
| New Rust code | The `smart` cut and the `raw.*` writer (about 300 lines) | The same cut, plus the layers and the polygon assembly (more than 1,500 lines) |
| Equal to duckOSM | Shown above: the same SQL gives the same bytes in the page | Hard: GEOS decides the start point, the direction and the order of the rings of `ST_BuildArea`. The Rust `geo` crate has no `BuildArea` and no `LineMerge`, and the `geos` crate does not compile to WebAssembly without much work. |
| A change in duckOSM's layers | Run the tool that writes the SQL file again | Port the change by hand |
| Download | +4.5 MB (spatial), only when the box is on | No more |
| Time (Tartu) | +1.1 s to load spatial (in parallel with the build) and 1.1–1.4 s for the SQL | Not known; probably less than 1 s |
| Native CLI | The same SQL file in the native DuckDB (`duckosm-duckdb`, 1.5.6, with spatial) | Yes |

**Recommendation: A.** It gives duckOSM's rows now, and it stays equal when duckOSM's layers change. The cost (4.5 MB once, about
2 s) is small next to the download of the region file. B is a large port whose main risk is the equality of the polygons, and it
gives no feature that A does not give.

## The design of A

### 1. Rust: the `smart` cut and `raw.*`

`pbf::read_area` gets an option `raw: bool`. With it off, nothing changes. With it on, the passes keep more:

1. **Node blocks**: the nodes inside, **with their tags**.
2. **From the first block of ways**: **all ways** with a node inside (not only the highway ways), the ways' blocks and their ranges of
   way ids, and **all relations**. Then, in memory: the relations that have a member node inside or a member way kept, their parent
   relations (again and again, as osmium does), and the member ways of the multipolygon relations among them.
3. **New pass, only the blocks of ways that hold a missing member way**: these ways (osmium's third pass).
4. **Node blocks that hold a missing node** (as today): the coordinates **and the tags** of the nodes of all kept ways.

The network step gets the highway ways and the ferry lines of this list (`filter::is_kept_by_reader`), as today. So the cut runs
one time for both.

`raw.ways` must be the ways after the fixes and the sidewalk rule, as in duckOSM:
- A way that the fixes change or add, and a road that gets `sidewalk=separate`, goes into `raw.ways` with the tags and nodes of the
  network step. The sidewalk rule also adds `duckosm:sidewalk=inferred` there (duckOSM writes both; the Rust rule writes only the
  first). The new nodes of the fixes go into `raw.nodes`.
- A risk: duckOSM compares `tags` as a `MAP`, in the order of the keys. The order must be the one of the PBF file, with the added
  keys at the end. The Monaco test shows it.

The writer (`duckosm-parquet`) writes the three tables with the columns and types of duckOSM: `osm_id BIGINT`, `lat DOUBLE`,
`lon DOUBLE`, `tags MAP(VARCHAR, VARCHAR)`, `refs BIGINT[]`, `ref_roles VARCHAR[]`, and `ref_types` as `VARCHAR[]` (the SQL that
loads them casts it to duckOSM's `ENUM`). The coordinates use the formula of the PBF reader, which gives the bits of duckOSM (B2).

The wasm module: `build_area(region, boundary, modes, fixes, raw)`. With `raw`, the result also has the three files. The page with
a cut file (`build`) or Overpass (`build_osm_json`) does not get the layers in this plan: the box is off there, with a note.

### 2. The SQL: one file from duckOSM

`tools/make_features_sql.py` runs duckOSM's `FeaturesBuilder` on an empty `raw.*` and records each SQL statement (as I did for the
measures). It writes `crates/duckosm-build/sql/features.sql`, with the duckOSM version in its first line. No SQL is written by hand.
The `ocean` layer is not in it.

One change to duckOSM's SQL (added after the first measures): `geom.way_line` comes from the column `line` of `raw_ways.parquet`.
Rust writes each way there as WKB, from the points of its nodes, as `ST_MakeLine` makes it. duckOSM's `geom.way_pt` and its
`ST_MakeLine` were 75 % of the SQL time on Tartu.

- The wasm module gives the text: `features_sql()` (`include_str!`). The page needs no other file.
- The native CLI can run the same text in its DuckDB: `duckosm --features`. This is a small step, but it needs the spatial extension
  in the native DuckDB 1.5.6 (`INSTALL spatial` downloads it one time). I put it last, and Kaveh decides if it is in this plan.

### 3. The page

These are the changes, each in its own small commit, so they merge easily with the other session's work:

- `index.html`, step 4: the check box **Base map layers** (on by default, because duckOSM makes the layers by default). A line
  under it: "Water, land use, buildings and more, as duckOSM's `features` tables. The first time, the page downloads DuckDB's spatial
  extension (4.5 MB)."
- `app/duck.js`:
  - When the box is on, `LOAD spatial` starts at once, in parallel with the build.
  - After the build: the three raw files go into `raw.*` and `features.sql` runs. `features_sql.js` uses `main.boundary` for the
    background; since the merge with the tables of group 1 ([tables.md](tables.md)), the build writes it. `raw.*` stays for SQL, as
    in a duckOSM `.duckdb` file. The raw files are not in the downloads.
  - The rows of each `features.*` table go into `tables`, with `mode: "features"`, so the list of downloads and `load.sql` show them.
  - `zipAll`: `features_<table>.parquet` (DuckDB writes them with `COPY`, as duckOSM's `_export_geoparquet`: every column except
    `tags`, because a nested column is not for GDAL), their entries in `layers.json` (`{"file", "kind": "features", "table",
    "count"}`), and one line in `README.txt`.
- `map/roadstyle/features_sql.js` and `map/roadstyle/roads_sql.js`: roadstyle's own files (`rust/roadstyle-wasm/js/`, their only
  copy), which `build.sh` takes at `ROADSTYLE_COMMIT`, the commit of the map module. Until 2026-10-23 they were copies of
  geolibre-roadstyle's in `map/`; `e2e.py` now checks that the served files equal roadstyle's.
- `map/map_worker.js`: `makeStyle(roads, look, features, ...)`. The module at `38cd2aa` already makes the base-map layers from
  `features.parquet`, with the look in `map/look.json`, which already has the layers (`features.layers_yaml`).
- The map: the base-map layers come in the same style as the roads, with sources `ov<i>`. They get the prefix `rs-` with the roads.
  The option **duckOSM layers** of **Base map** shows them; the other options hide them. Outside the area, OpenFreeMap stays.

### 4. Not in this plan

- `features.ocean`: S3 allows the page to read it, but duckOSM reads a folder of Overture files with a `glob`. The time and the
  download in the page are not known. Later, as an option.
- The layers with a cut file or with Overpass (Overpass would need a much larger query).
- `visualization.edge_levels` and the bus routes.

## The checks

1. **Rust, the cut** (`tests/features.rs`): the `raw` cut of `monaco_sample.osm.pbf` with `monaco.geojson` gives the same nodes,
   ways and relations as osmium's `smart` cut (ids, tags, refs, roles, the coordinate bits). The osmium cut goes into `tests/data`
   (Monaco: 0.6 MB). Tartu from the Estonia file against `osmium extract -s smart`, by hand, with the numbers in this file.
2. **Against the Monaco golden** (`tools/compare_features.py --duckosm`): the native build with the fixes and the raw files
   (`--raw`), then `features.sql` in DuckDB. `raw.*` must equal the golden `raw.*`, and each `features.*` table the golden table:
   the same rows on `osm_id`, `osm_type`, `kind`, `name`, `tags` and the WKB of `geom`, and `bearing` to 1e-9 degrees.
3. **The page** (`www/e2e.py`, the box is on by default): Monaco and Tartu from the region file. The page's
   `features_<table>.parquet` files must equal the native result of check 2 (`tools/compare_features.py --page`). The map shows the base-map layers (the picture), and
   the zip has the `features_*.parquet` files and their `layers.json` entries.
4. The times and the memory of Monaco and Tartu go into this file, as for B3 and B4.

The expected time in the page for Tartu: the cut a little longer than today (3.7 s with one thread, 1.7 s with threads), and about
1.5 s for the raw tables and the SQL. Spatial loads during the build. The style with the layers needs more memory than the roads alone
(Tartu: 1,388 MiB for the roads, and 22,494 buildings more). I will measure it. If the style is too large, the buildings are the
first to leave out.

## Steps

| Step | What | Commit |
|---|---|---|
| F1 | `make_features_sql.py` and `features.sql` | yes |
| F2 | The `raw` cut and the writer in Rust, check 1 | yes |
| F3 | The fixes and the sidewalk tags in `raw.ways`, check 2 | yes |
| F4 | The wasm module and the build worker (`raw`, `features_sql()`) | yes |
| F5 | The page: the box, the tables, the downloads (check 3) | yes, small |
| F6 | The map: `features_sql.js`, the worker, the Base map option | yes, small |
| F7 | The native CLI `--features` (if Kaveh agrees) | yes |
| F8 | The docs: this file, `app.md`, `browser.md`, `development.md` | yes |

## Questions for Kaveh

1. Approach A (Rust `raw.*` and duckOSM's SQL in duckdb-wasm), and not a port to Rust?
2. The box **Base map layers** on by default?
3. `features_<table>.parquet` without `tags`, as duckOSM's GeoParquet export, or with `tags` (DuckDB reads it, GDAL can fail)?
4. The native CLI `--features` (F7) in this plan, or later?
5. The sea (`features.ocean` from Overture) later, as its own step?

## Result (2026-10-22)

Done as planned (steps F1 to F6 and F8), with the way lines in Rust and the changes below. F7 (the native `--features`) and the sea
are not done. The commands are in [development.md](development.md#check-the-base-map-layers).

Checks:
- The `raw` cut of the Monaco sample equals osmium's `smart` cut: the same nodes, ways and relations, with the same tags, members and
  coordinate bits (`tests/raw.rs`). Tartu from the Estonia file against `osmium extract -s smart` is equal too.
- Monaco with duckOSM's fixes, three modes: `raw.*` and the 11 `features.*` tables equal the golden `monaco.duckdb` on every row.
- The page, Monaco and Tartu from the region file: its 11 `features_<table>.parquet` files equal the native result on every row.
- The network files do not change with `--raw` (Monaco and Tartu, 20 files each, byte for byte). The area build of the wasm module
  with `raw` equals the native build, byte for byte (`crates/duckosm-wasm/tests/native.rs`, 23 files).
- `boundary_e2e.py` and the relay check pass. Three builds in one page (with, without, with the layers) show and hide the tables,
  the files and the Base map option correctly.

Times, three runs of Tartu, one of Monaco; the computer had a load average of 2 to 4 from other work:

| The page (8 threads) | Monaco | Tartu, with the layers | Tartu, without |
|---|---|---|---|
| Build (the cut and the networks) | 0.42 s | 2.19–2.25 s | 1.70–1.72 s |
| Tables into DuckDB | 0.46 s | 0.62–0.65 s | 0.71 s |
| The layers (`raw.*` and the SQL) | 0.31 s | 0.70–0.72 s (1.52 s with duckOSM's way lines) | |
| From the click to the first query | 1.88 s | 3.73–3.80 s | 2.60–2.62 s |
| WebAssembly memory of the build | 51 MiB | 190–193 MiB | 130–131 MiB |
| The map: layers, style | 39 layers, 0.82 s | 41 layers, 6.2–6.3 s, 1,387 MiB | 5 layers, about 6 s, 1,387 MiB |

Natively, Tartu takes 1.17 s and 334 MB with `--raw`, and 0.96 s and 228 MB without (before the way lines in Rust).

Changes from the design:
- **The geometry type in the page.** When the spatial extension loads before the first value of the type
  `GEOMETRY('OGC:CRS84')`, duckdb-wasm (spatial `28db190`) fails on each such value with "stoi: no conversion". In the other
  order, it works. `EPSG:4326` works, and native DuckDB does not have this error. A GeoParquet file gives that type, so after
  `LOAD spatial` the page could not read its own network files. The bug is
  [duckdb/duckdb-wasm#2199](https://github.com/duckdb/duckdb-wasm/issues/2199); our facts are in a comment there (2026-10-22). The page now reads the
  geometry as WKB (`ST_GeomFromWKB`, with `enable_geoparquet_conversion` off). The type is `GEOMETRY` without a CRS, the type of
  duckOSM's tables. `load.sql` does not change.
- **The lane arrows of the fixes.** duckOSM writes `turn_lanes_forward` and `turn_lanes_backward` into `raw.ways` as `turn:lanes:*`.
  The Rust reader of the fixes skipped them. After the merge with the tables of group 1, the network's ways hold all their tags, and
  the arrows (`OsmFixes::set_arrows`, for each mode that keeps the way) and the sidewalk tags are set on them. `Raw::sync_tags` then
  copies the changed tags into `raw.ways`, with the order of `map_concat`. A member without a role is NULL, as `ST_READOSM` gives it.
- **The order in the page.** DuckDB loads spatial during the build, and the tables wait for it. The first build of a page
  downloads the extension (4.5 MB), so the time from the click to the first query includes that download.
- The layers are made only from a region file. With a cut file or with Overpass, the box has no effect.

