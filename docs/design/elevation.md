# Elevation (put the network on real terrain)

**Status:** **core implemented** (2026-07-23). A **post-processing feature**, not a build-pipeline
stage: the standalone `duckosm elevation <db>` command (module `src/duckosm/elevation.py`, sibling
of `matsim.py` / `opendrive.py` / `gmns.py`) enriches an already-built db in place — adds
`<mode>.nodes.ele`, `<mode>.edges.z_from`/`z_to` (all modes present, virtual nodes included) and a
`main.elevation_metadata` provenance row. `--dem <file>` (any GDAL raster), or `--source` with a
coverage-based **auto** resolver (default): EU-DTM bare-earth inside Europe when
`OPENTOPOGRAPHY_API_KEY` is set, else Copernicus GLO-30 (streamed from AWS, no auth, global). Opt-in
deps `rasterio` + `pyproj` (extra `[elevation]`); tests in `tests/test_elevation.py` (9, green in
the full suite). Idea adapted from OSM2World's elevation model — **minus** its constraint solver,
which its own docs call *"currently very fragile and deactivated by default."*

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
duckosm build     config/sodermalm.yaml                              # unchanged — no elevation
duckosm elevation data/db/sodermalm.duckdb --dem markhojd_1m.tif     # local high-res DTM
duckosm elevation data/db/sodermalm.duckdb --source copernicus       # or stream a global DEM
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

## Where to get the DEM (data sources)

Two ways to feed the command: `--source` streams a **global** DEM (zero setup, coarse); `--dem`
points at a **local high-res** file (best quality where you have it). The catalogue below covers
both. **For roads you want a DTM (bare earth)** — see [DTM vs DSM](#dtm-vs-dsm--the-one-that-matters-for-roads);
a DSM puts nodes on tree canopy / rooftops.

### Global / pan-European

| Dataset | Res | Coverage | DTM/DSM | Format / access | Auth | License |
|---|---|---|---|---|---|---|
| **Copernicus GLO-30** *(recommended default)* | 30 m | global, 90°N–90°S | **DSM** | COG on AWS `copernicus-dem-30m`, `/vsicurl` streamable | **no** | free, commercial-OK (attribution) |
| Copernicus GLO-90 | 90 m | global | DSM | COG on AWS `copernicus-dem-90m` | no | same |
| **EU-DEM v1.1** | 25 m | Europe incl. all Sweden | hybrid ≈**DTM** | GeoTIFF, **EPSG:3035** (reproject) | no | Copernicus open, commercial-OK |
| **FABDEM v1.2** | 30 m | global (~84°N) | **DTM (bare-earth)** | GeoTIFF | no | **CC-BY-NC-SA — non-commercial only** |
| SRTM v3 / SRTMGL1 | 30 m | **60°N–56°S only** | DSM | HGT/GeoTIFF | Earthdata | public domain |
| NASADEM | 30 m | **60°N–56°S only** | DSM | GeoTIFF/HGT, MS Planetary STAC | Earthdata | public domain |
| ASTER GDEM v3 | 30 m | 83°N–83°S | DSM | COG GeoTIFF | Earthdata | free (attribution) |
| ALOS AW3D30 | 30 m | 82°N–82°S | DSM | GeoTIFF | JAXA acct | free (attribution) |
| GEBCO 2024 / ETOPO 2022 | ~450 m | global land+sea | mixed | NetCDF/GeoTIFF | no | public domain |
| OpenTopography Global DEM API | 30–90 m | global (proxies the above) | varies | bbox-clipped GeoTIFF, one REST call | API key | per-source |

**Rule out SRTM/NASADEM as the default:** hard **60°N ceiling**. Stockholm (59.3°N) is marginal and
the target area reaches ~69°N (`stockholm_county` crosses 60°N at Norrtälje; `sweden` goes far
past) — northern coverage would be empty. GEBCO/ETOPO are ~450 m, coastline/sea-floor context only.

**The GLO-30 DSM caveat:** Copernicus GLO-30 is a *surface* model (tree canopy, buildings). For
open urban roads it's usually fine and it's the only zero-auth, commercial-OK, globally-streamable
option — but over Sweden's forests it biases high. Commercial-safe bare-earth alternatives:
**EU-DEM v1.1** (25 m, ~DTM, covers the north; costs an EPSG:3035 reproject). **FABDEM** is
bare-earth global but **non-commercial** — fine for research, not for a product.

*Copernicus tile naming gotcha:* `Copernicus_DSM_COG_10_N59_00_E018_00_DEM/…​.tif` — the `10` marks
GLO-30 (`30` = GLO-90), **not** the metre resolution. Anonymous read:
`/vsicurl/https://copernicus-dem-30m.s3.amazonaws.com/<tile>/<tile>.tif`.

### National high-resolution (LiDAR) — for the areas you build

1 m LiDAR-derived DTMs, an order of magnitude better than any 30 m global grid — they resolve
Södermalm's cliffs and real bridge approach grades. Use as a local `--dem`.

**This is a starter list, not a closed set of "supported" sources.** Only the three areas duckOSM
actually builds (Sweden, Estonia, Canada) are filled in. For anywhere else, the tool needs no new
code — either use the global default (`--source copernicus` / EU-DEM, works everywhere), or spend
30 seconds finding that country's national open DTM and hand it to `--dem`. E.g. **France → IGN RGE
ALTI 1 m / LiDAR HD** (Etalab 2.0 open, GeoTIFF, Lambert-93 EPSG:2154, `geoservices.ign.fr`);
Germany → the state LiDAR portals; etc. A DEM anywhere on Earth is either a `--source` stream or a
`--dem` file — never a code change.

**Sweden — Lantmäteriet** *(the one for Stockholm/Södermalm)*
- **Markhöjdmodell Nedladdning, grid 1+** — 1 m **DTM**, LZW GeoTIFF, **SWEREF99 TM (EPSG:3006)**,
  vertical RH2000. **License CC0.** Also: a 1 m DSM (Ythöjdmodell), legacy grid 2+/50+, and the raw
  NH / forest LiDAR as LAZ.
- Access: STAC API `STAC-hojd`, host `dl1.lantmateriet.se/hojd/data`, plus WMS/WCS. CC0, **but
  download is gated behind a free GeoTorget registration** (`geotorget.lantmateriet.se`) — open, not
  anonymous.

**Estonia — Maa-amet** *(for Tartu)*
- **Digital Terrain Model** — 1 / 5 / 10 / 25 m **DTM**, GeoTIFF + XYZ, **L-EST97 (EPSG:3301)**,
  vertical EH2000. Also 1 m DSM, CHM, and raw LAZ 1.4 (≥4 pt/m²).
- Access: **fully anonymous**, 1 km² map-sheet tiles or nationwide files, plus WMS/WFS/WCS. Open,
  attribution appreciated but not required — the most frictionless of the three.

**Canada — NRCan (GEO.CA)** *(for Vancouver/Burnaby)*
- **HRDEM Mosaic 1 m** — DTM + DSM, **COG**, **Canada Atlas Lambert (EPSG:3979)**, vertical
  CGVD2013. Legacy CDEM/CDSM (~20 m) and newer MRDEM (30 m) fill gaps.
- Access: **STAC API `datacube.services.geo.ca/stac/api/`** (collections `hrdem-mosaic-1m/-2m`,
  `hrdem-lidar`), plus WMS/WCS, also on OpenTopography. **License OGL-Canada — attribution
  required (not CC0).** COG + STAC = best of the three for windowed remote sampling.
- BC extras: LidarBC portal (OGL-BC); City of Vancouver Open Data 1 m DEM (UTM 10N NAD83(CSRS)).

**CRS note:** the Swedish (3006) and Estonian (3301) national DTMs are in the CRS those duckOSM
areas already work in, but node `geom` is stored lon/lat (4326) regardless — so the sampler always
reprojects 4326 → DEM CRS. It's a no-op only for a 4326 DEM (Copernicus); Sweden/Estonia/Canada all
need the transform.

### Recommendation

- **Default, works for any db on Earth →** `--source copernicus` (GLO-30). Streams, no auth, full
  latitude coverage, no configuration. This is the general worldwide path — accept the DSM canopy
  bias for a first pass.
- **Quality in a specific region →** `--dem` that region's national 1 m DTM (Stockholm →
  Lantmäteriet grid 1+; Paris → IGN LiDAR HD; Tartu → Maa-amet; Vancouver → HRDEM). Pure
  documentation guidance — the tool takes the file, not a country name.
- **Commercial-safe bare-earth global, if the canopy bias bites →** EU-DEM v1.1 (Europe) or the
  relevant national DTM; not FABDEM, which is non-commercial.

---

## Data formats (what these files actually are)

A DEM is almost always a **raster**: a grid of elevation cells + an affine geotransform + a CRS. To
get elevation at a node you reproject the point into the raster CRS, then read the covering cell.

### Raster (the common case)

- **GeoTIFF (`.tif`)** — the default; single-band float32 metres. Universally read by GDAL/rasterio.
- **Cloud-Optimized GeoTIFF (COG)** — a GeoTIFF with internal tiling + overviews, so GDAL can issue
  HTTP **range requests** and stream *only the window you sample* over `/vsicurl/` — no full
  download. This is what makes streaming a continental DEM from S3 practical (Copernicus, HRDEM).
- **VRT (GDAL Virtual Raster)** — an XML sidecar that presents N tiles (local or remote COGs) as
  **one logical dataset**; `gdalbuildvrt dem.vrt tiles/*.tif`, then sample against the whole
  coverage as if one file. How we'll stitch Copernicus tiles for `--source`.
- **ESRI ASCII Grid (`.asc`)** — plain-text grid; readable but huge/uncompressed, convert to COG.
- **`.img` (ERDAS)** — common national-DEM binary; GDAL reads it like a GeoTIFF.
- **SRTM `.hgt`** — headerless int16, one 1° file named by SW corner (`N59E018.hgt`); NODATA −32768.
- **DTED (`.dt1/.dt2`)** — military terrain data; GDAL-native, rarely encountered.

### Point clouds (LAS/LAZ) — NOT directly samplable

`.las`/`.laz` (compressed) are bags of `(x, y, z, classification, …)` **points**, not a grid — there
is no "cell at (x, y)". To sample you must first **rasterize to a DEM** (PDAL `writers.gdal`,
filtering `Classification == 2` for bare-earth ground). **Don't** do this yourself — every provider
above also publishes a ready-made 1 m DTM raster; use that. Reach for LAZ only for custom
classification.

### Web services

- **WCS (Web Coverage Service)** ✅ — returns actual raster values for a bbox; GDAL has a `WCS`
  driver, so it's samplable. Fine when a provider exposes a DEM as WCS.
- **WMS (Web Map Service)** ❌ — returns a *rendered picture* (hillshade RGB); pixels are display
  colours, not metres. Never sample terrain off WMS.
- **STAC** — a JSON catalog for **discovery**: query by bbox → item metadata → asset href is a COG
  URL you open with `/vsicurl/`. How you find which tiles cover the network (Copernicus, HRDEM).
- **OpenTopography API** — one REST call, bbox-clipped GeoTIFF, proxies many datasets (free key).

### DTM vs DSM — the one that matters for roads

- **DSM** (surface) = top reflective surface: roofs, tree canopy, bridge decks.
- **DTM** (terrain) = bare earth, structures/vegetation removed.

**Grounding a road network wants a DTM.** A DSM snaps a node under a tree line onto the canopy
(metres high) or onto a building edge → spurious spikes and wrong grades. (One genuinely hard case
even a perfect DTM can't fix: a real elevated bridge *is* above bare earth, so its deck buries into
the terrain — that's the Phase-2 structure-offset problem below, not a DEM choice.) Copernicus
GLO-30 is DSM; EU-DEM, FABDEM, and all three national 1 m products are DTM.

### Vertical datum (flag, don't solve)

Heights are relative to a reference that sources disagree on — geoid/orthometric (EGM96/EGM2008,
≈ sea level; most DEMs) vs ellipsoidal (raw GNSS). The gap reaches ±100 m globally (tens of m in
Europe). Irrelevant for *relative* ground elevation within one source; just **record each DEM's
vertical datum** and don't blend sources blindly.

---

## Phase 2 — absolute structure heights (deferred, YAGNI)

Build only when a consumer needs bridges at *absolute* z (a glTF/3D-Tiles mesh, or OpenDRIVE with
real deck clearance):

- Per-edge `grade_offset_m` from `(layer, bridge, tunnel)` via a small constant table (bridge deck
  ≈ +Δ·layer, tunnel ≈ −Δ) — the same model roadstyle's decks already assume, made explicit as data.
- Ramp the offset to 0 at junctions shared with a lower-layer road — the only place OSM2World's
  solver earns its keep. One windowed SQL pass over the simplified graph, not a global solve.

`z_from`/`z_to` would then become `terrain + ramped offset`; `nodes.ele` stays bare terrain.

## Visualisation

`scripts/elevation_report.py --db <db>` renders an enriched db as a roadstyle **web report** —
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

New files: `src/duckosm/elevation.py`, `docs/design/elevation.md` (this),
`tests/test_elevation.py`, `scripts/elevation_report.py`. Touched: `cli.py` (+`elevation`
subcommand), `README.md`, `docs/data_dictionary.md` (+3 columns, +`main.elevation_metadata` table),
and the z-consumers `matsim.py` / `opendrive.py` (`gis` needs none — scalar columns pass through).
Opt-in deps `rasterio` + `pyproj` (extra `[elevation]`). **No change to `importer.py`, `config.py`,
or any existing build** — a network without an elevation pass is byte-identical to today's.
