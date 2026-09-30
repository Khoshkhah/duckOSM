# Elevation (put the network on real terrain)

A post-processing step, not part of the build: `duckosm elevation <db>` (`src/duckosm/elevation.py`)
enriches a built db in place with `<mode>.nodes.ele`, `<mode>.edges.z_from` / `z_to` (every mode,
virtual nodes included) and a `main.elevation_metadata` provenance row. Use `--dem <file>` (any
GDAL raster), or `--source` (default `auto`): EU-DTM bare-earth inside Europe when
`OPENTOPOGRAPHY_API_KEY` is set, else Copernicus GLO-30 (streamed from AWS, no account needed,
worldwide). Needs `pip install "duckosm[elevation]"` (`rasterio`, `pyproj`). The idea is adapted
from OSM2World's elevation model, without its constraint solver.

**Done since:** the z-consumers **`matsim`** (node `z`), **`opendrive`** (`<elevationProfile>`,
linear ramp from `z_from` to `z_to`) and **`gis`** (`ele`/`z_from`/`z_to` flow through as attributes
— the GeoPackage/shapefile export keeps every scalar column, so no code path of its own). All emit
height when the columns exist and are unchanged otherwise; live-verified on the enriched Södermalm
db. Copernicus GLO-30 live streaming from AWS is proven end-to-end.

**Deliberately not done — `lanelet2`:** it reads a **GMNS** db (whose `node.z_coord` is currently
`NULL`) and its boundaries are centreline *offsets*, not graph nodes — so elevation would mean
plumbing z through the whole GMNS export (link table across its meso/micro/combined variants) and
then interpolating onto offset points. That's a disproportionate change to drape coarse 30 m DEM z
onto a map its own docstring calls *"not survey-grade"* — false precision, low value. Deferred until
a real Autoware/lanelet2 consumer needs z.

**Still to do:** the **EU-DTM/OpenTopography** fetch is unit-tested for provider selection but its
live download is unproven (no API key in CI) — worth one manual run. Phase 2 (absolute structure
heights) remains deferred.

**Implementation note (PROJ / eclipse-sumo clash):** eclipse-sumo's `import sumo` sets
`PROJ_LIB`/`PROJ_DATA` to its own bundled, GDAL-incompatible `proj.db`, which then makes rasterio
fail every EPSG lookup in the same process (GDAL caches the PROJ context on first touch, so it must
be cleared *before* the first CRS op). `to_elevation` calls `_fix_polluted_proj()` up front to drop
such a sumo-pointed var so rasterio and pyproj fall back to their own bundled data; a no-op in the
normal (clean) `duckosm elevation` process.

## Summary

Every node in a duckOSM network sits at z = 0. Grade separation is *relative* only — roadstyle
stacks bridges/tunnels by OSM `layer` — so "bridge above road" renders, but there is no ground.
Nothing downstream with a z field (MATSim node coords, OpenDRIVE `elevationProfile`, Lanelet2 node
`ele`) can carry real height, so those exports ship flat.

Fix: sample a DEM at every node and store the ground elevation as a scalar `nodes.ele`, then surface
it on edge endpoints (`z_from`/`z_to`) and in the z-aware exporters. This is a raster lookup, not a
solver — the one genuinely missing piece is the absolute ground baseline.

## Design — a post-processing command, not a pipeline stage

Elevation runs **after** the build, exactly like the other enrichers/exporters, so a plain build
stays unchanged and dependency-free, and elevation is optional, re-runnable, and swappable per-DEM
without a rebuild:

```bash
duckosm build -c  sodermalm.yaml                                    # unchanged — no elevation
duckosm elevation monaco.duckdb --dem markhojd_1m.tif     # local high-res DTM
duckosm elevation monaco.duckdb --source copernicus       # or stream a global DEM
```

**Global by default, zero country-specific code.** duckOSM converts OSM anywhere on Earth, so
elevation is worldwide too: the default `--source copernicus` (GLO-30) covers the whole planet
(90°N–90°S, no auth) — hand it any OSM extract, any country, and it works with no configuration.
`--dem` ingests any DEM anywhere through one GDAL reader. **No branch in the code knows which
country a db is in.** The national high-res sources listed below are optional documentation
guidance — a place to look for better-than-30 m data in a given region — never special cases baked
into the tool.

- **New module** `src/duckosm/elevation.py` + an `elevation` subcommand in `cli.py`. **No
  `importer.py` change, no build-config block, no pipeline flag.**
- **Mutates a finished db in place:** for each mode schema, read `nodes`, sample the DEM at every
  node, `ele`; then set `edges.z_from`/`z_to` from the source/target node `ele`. `ADD COLUMN IF NOT
  EXISTS` so a re-run with a different DEM just overwrites.
- **Virtual nodes are free here.** Running post-build means the simplifier's negative-id nodes
  already exist in `nodes` with real `geom`, so we sample every node's geometry directly — no
  `raw.nodes` dependency, no per-mode-during-build timing (the fiddly part of the old in-pipeline
  design disappears).

### Two ways to supply elevation (pick one)

1. **`--dem <file>` — you provide the raster.** Format is **auto-detected** (GDAL), you never declare
   it. Canonical **GeoTIFF / COG** (`.tif`); also `.vrt` (mosaic over many tiles), `.img`, `.asc`,
   `.hgt`, DTED — any GDAL raster driver. **Not** LAS/LAZ point clouds (rasterize first). Offline,
   reproducible, whatever resolution you have.
2. **`--source <name>` — the tool fetches it (the default).** No file needed: read the network's
   node bounding box from the db, pick a **global** provider, fetch/stream the covering data, sample.
   Bare `duckosm elevation <db>` runs `--source auto`. `--source` entries are global providers only;
   national/local data always comes in via `--dem`.

#### `--source auto` — how it picks per area

`auto` is a **coverage-based resolver**, not country logic: it takes the db's bbox and returns the
highest-priority provider whose coverage contains it *and* that is usable (credentials present).

| Provider | Coverage | Product | Access | Picked by `auto` when |
|---|---|---|---|---|
| `eudtm` | Continental Europe | **DTM (bare-earth)** | OpenTopography API, needs `OPENTOPOGRAPHY_API_KEY` | bbox ⊆ Europe **and** the key is set |
| `copernicus` | whole planet | DSM | AWS `/vsicurl`, anonymous | otherwise (the always-available fallback) |

So `auto` **upgrades to bare-earth EU-DTM inside Europe when a (free) key is set, and falls back to
Copernicus everywhere else** — it never *requires* setup, it just does better where it can. Adding a
new global provider is one `PROVIDERS` row (metadata + a `covers` bbox + a `usable` check); `auto`
starts picking it by coverage with no other change. `EUROPE_BBOX` is the only region constant, and
it's a coverage extent, not a country list.

### Flags

```
duckosm elevation <db>
  --dem PATH            a raster file/URL — any GDAL format (GeoTIFF/COG/VRT/.img/.asc/.hgt); auto-detected
  --source NAME         global DEM if no --dem: 'auto' (default) | 'copernicus' | 'eudtm'
  --modes m1,m2         default: all mode schemas present
  --nodata-fill FLOAT   value for raster voids / out-of-coverage (default 0.0 = sea level)
  --suffix NAME         store as a second surface: ele_NAME / z_from_NAME / z_to_NAME
```

### Two surfaces in one db (`--suffix`)

A bare-earth **DTM** and a **DSM** (top of buildings and trees) answer different questions, so they
are columns, not alternatives: road gradient wants the DTM — a DSM puts a street node on the tree
canopy above it — while `ele_dsm - ele` is object height above ground, the input to building heights,
canopy height, and shadow modelling.

```bash
duckosm elevation burnaby.duckdb --dem .../BC-Lower_Mainland_2016-1m-dtm.vrt
duckosm elevation burnaby.duckdb --dem .../BC-Lower_Mainland_2016-1m-dsm.vrt --suffix dsm
```

The suffix is interpolated into DDL, so it is validated as a bare identifier. `elevation_metadata`
holds **one row per column** (keyed by `ele_column`), upserted — re-running the DTM pass leaves the
DSM's provenance row intact.

**Resolution decides whether this is meaningful.** The difference is only real when the DEM resolves
the objects: Canada's HRDEM ships a matched 1 m DTM/DSM pair from one LiDAR flight, and gives sane
heights (Metrotown 9.4 m, SFU 32.7 m). Two *independent* 30 m global products do not — differencing
EU-DTM against Copernicus GLO-30 over Södermalm put the "surface" **below** bare earth at 38 % of
nodes, i.e. the result was inter-dataset noise, not objects. Pair a DTM and DSM from the same
acquisition, at a resolution finer than what you are measuring.

### One reader, not per-source importers

There is **no importer per source and none per format.** The design leans entirely on GDAL/rasterio's
driver layer:

- **`--dem <path-or-url>` reads everything.** `rasterio.open()` auto-detects the driver, so one code
  path handles GeoTIFF, COG, ERDAS `.img`, ESRI ASCII, SRTM `.hgt`, DTED, and a VRT mosaic — local
  file *or* remote COG over `/vsicurl/`. Any DEM on Earth flows through this with zero new code. The
  per-source differences (CRS, nodata) are read *from the file* — `ds.crs`, `ds.nodata` — never coded
  per source.
- **`--source` is a 2–3 entry convenience registry, not N importers.** The only thing `--dem` can't do
  for a *global tiled* dataset is pick which tiles cover the bbox. That's ~15 lines per collection
  (`copernicus`: bbox → tile URLs → in-memory VRT; `eudtm`/others: a STAC bbox query or one
  OpenTopography REST call — which itself proxies ~8 datasets). Add an entry only for a source you want
  zero-setup streaming for; everything else is just a `--dem` URL.
- **LAS/LAZ point clouds are the one non-raster type — and they're out of scope, not a new importer.**
  Every provider also ships a ready-made 1 m DTM raster; use that. Raw LiDAR gets pre-rasterized once
  with PDAL → GeoTIFF → `--dem`. One doc line, no code.

So the whole surface is: **one universal reader + a tiny streaming registry + "point clouds → use the
provider's raster."** Not source × format.

### Schema added (update `docs/data_dictionary.md`)

| Table   | Column            | Meaning                                                      |
|---------|-------------------|--------------------------------------------------------------|
| `nodes` | `ele DOUBLE`      | ground elevation, m (2 dp)                                   |
| `edges` | `z_from` / `z_to` | DOUBLE — endpoint ground elevation, m (2 dp; bare terrain — no structure offset in v1) |
| `nodes` | `ele_<name>`      | DOUBLE — a second surface from `--suffix <name>` (e.g. `ele_dsm`) |
| `edges` | `z_from_<name>` / `z_to_<name>` | DOUBLE — the same surface at the edge endpoints |

Geometry stays 2D. No 3D `LINESTRING Z` vertices in v1 — DuckDB spatial's Z support is thin and
nothing consumes per-vertex z; endpoint scalars cover every current exporter.

### Provenance metadata (`main.elevation_metadata`)

After sampling, write a **one-row provenance record** (next to the existing
`main.visualization_metadata`), so the db is self-documenting and reproducible — you can always tell
which DEM a network's heights came from, and spot poor coverage.

| Column | Example | Source |
|---|---|---|
| `source` | `Copernicus GLO-30` / `<file basename>` | provider name, or the `--dem` file |
| `source_type` | `download` / `file` | which mode ran |
| `uri` | `s3://copernicus-dem-30m/…` / `/abs/path.tif` | tile URLs or file path |
| `resolution_m` | `30` | provider registry, or GDAL pixel size for `--dem` |
| `product` | `DSM` / `DTM` / `unknown` | provider registry (unknown for arbitrary files) |
| `dem_crs` | `EPSG:4326` | read from the raster (`ds.crs`) |
| `vertical_datum` | `EGM2008` / `RH2000` / `unknown` | provider registry |
| `license` | `Copernicus open (attribution)` / `unknown` | provider registry |
| `nodata_fill` | `0.0` | the flag used |
| `n_nodes` | `48213` | nodes sampled |
| `n_nodata` | `12` | nodes that fell back to fill — **free coverage QA** (high = DEM didn't cover the area) |
| `sampled_at` | `2026-07-23T14:22:05Z` | ISO timestamp |
| `duckosm_version` | `0.x` | reproducibility |

For `--source`, every descriptive field is filled from the **provider registry** — the same 1–3
global entries that know how to fetch also carry resolution / product / CRS / datum / license, so the
row populates for free. For `--dem`, we record the path plus GDAL-derived `dem_crs` / `nodata` /
pixel size and leave product / datum / license as `unknown` unless the user passes overrides. One
row, minimal columns — provenance, not a data warehouse.

### Sampling mechanics (the three things that bite)

`nodes.geom` is `ST_Point(lon, lat)` — **EPSG:4326**. `rasterio` samples in the *raster's* CRS, so:

```python
from pyproj import Transformer
import rasterio
with rasterio.open(dem) as ds:
    to_ras = Transformer.from_crs("EPSG:4326", ds.crs, always_xy=True)   # no-op for a 4326 DEM
    xs, ys = to_ras.transform(lons, lats)
    nd = ds.nodata
    ele = [v[0] if v[0] != nd else fill for v in ds.sample(zip(xs, ys))]  # 1) batch  2) mask nodata
```

1. **CRS** — reproject lon/lat into the DEM CRS first. A 4326 DEM (Copernicus) is a no-op; a
   Swedish DTM (EPSG:3006) needs the transform. Getting this wrong yields all-nodata or garbage.
2. **nodata** — check `ds.nodata` and mask it; voids/ocean/tile-edges are sentinels (e.g. −32768,
   −9999), not real heights → replace with `--nodata-fill`.
3. **batch** — feed all nodes to one `ds.sample()`; it reads each covering tile once.

For `--source copernicus`: compute the 1°×1° tiles covering the node bbox, build an in-memory GDAL
VRT over their `/vsicurl` URLs, open once, sample all. GDAL streams only the windows it touches.

### Exporters that gain z (mostly one field each)

| Exporter        | z field                                          | Status |
|-----------------|--------------------------------------------------|-------|
| `opendrive`     | `<elevation>` on each road's `elevationProfile`  | **done** — biggest external win (flat OpenDRIVE→CARLA/esmini was the gap) |
| `matsim`        | `<node z="…">`                                   | **done** — valid `network_v2` optional attribute |
| `gis` / `export-gis` | `ele`/`z_from`/`z_to` as attributes         | **done** — free (export keeps every scalar column); geometry stays 2D |
| `lanelet2`      | `ele` tag on boundary nodes                      | deferred — needs GMNS z plumbing + interpolation for a non-survey-grade map (see above) |

### Test (`tests/test_elevation.py`)

A 3×3 synthetic in-memory GeoTIFF with a known ramp; sample four nodes; assert `ele` matches the
ramp within tolerance, and that a nodata cell falls back to `--nodata-fill`. One file, no fixtures.

---

## Where to get the DEM, file formats, DTM vs DSM, vertical datum

Moved to the published reference: `docs/reference/elevation-sources.md`.

## Phase 2 — absolute structure heights (deferred, YAGNI)

Build only when a consumer needs bridges at *absolute* z (a glTF/3D-Tiles mesh, or OpenDRIVE with
real deck clearance):

- Per-edge `grade_offset_m` from `(layer, bridge, tunnel)` via a small constant table (bridge deck
  ≈ +Δ·layer, tunnel ≈ −Δ) — the same model roadstyle's decks already assume, made explicit as data.
- Ramp the offset to 0 at junctions shared with a lower-layer road — the only place OSM2World's
  solver earns its keep. One windowed SQL pass over the simplified graph, not a global solve.

`z_from`/`z_to` would then become `terrain + ramped offset`; `nodes.ele` stays bare terrain.

## Visualisation

`scripts/elevation_report.py --db <db>` (in the repo's `scripts/` folder; needs a clone) renders an enriched db as a roadstyle **web report** —
roads coloured by mean edge elevation (`(z_from+z_to)/2`), a *Colour by* dropdown
(Elevation / Class / Max speed / Lanes), base-map switcher, hover read-out, and (when roadstyle's
`ui/report/sidebar.html` is found in the roadstyle checkout) a gradient legend + filter + search.
The title self-labels the DEM source from `main.elevation_metadata`, so a Copernicus vs EU-DTM build
is distinguishable at a glance. The report HTML is a gitignored generated artifact; the script is the
committed recipe.

## roadstyle follow-up (separate, tiny)

roadstyle fakes deck heights on flat ground today. Once `edges.z_from`/`z_to` exist, one optional
setting lets decks sit on real terrain. A roadstyle change downstream of this — not duckOSM work.

## Footprint

New files: `src/duckosm/elevation.py`, `docs/elevation.md` (this),
`tests/test_elevation.py`, `scripts/elevation_report.py`. Touched: `cli.py` (+`elevation`
subcommand), `README.md`, `docs/data_dictionary.md` (+3 columns, +`main.elevation_metadata` table),
and the z-consumers `matsim.py` / `opendrive.py` (`gis` needs none — scalar columns pass through).
Opt-in deps `rasterio` + `pyproj` (extra `[elevation]`). **No change to `importer.py`, `config.py`,
or any existing build** — a network without an elevation pass is byte-identical to today's.
