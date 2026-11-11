# The Ground step

## Problem

The area's ground is one DEM ([The ground reference](../concepts/ground-reference.md)). The map terrain follows it. A person who builds an area in
duckOSM Studio had a plain DEM field: no advice on which DEM to take, and the product, the vertical datum and the licence were not filled in. The build
stops without them.

## Decisions

1. **The sources are data.** `src/duckosm/data/dem_sources.json` has one entry for each source: coverage, resolution, product (`DTM` or `DSM`), CRS, vertical datum,
   licence, commercial use, access (URL, account or key) and `cors`. It ships in the Python package. Two tables of
   [Elevation sources](../reference/elevation-sources.md) are made from it (`scripts/update_elevation_sources_doc.py`), and a test fails when the page differs.
2. **The order is fixed.** `rank`: a national 1 m DTM, EU-DTM, FABDEM, Copernicus GLO-30, the others (by resolution). A source covers an area when the whole box is
   inside its coverage. A country is a coarse polygon, with `exclude` boxes for enclaves (Monaco in France). The lookup is a hint: the download page decides.
3. **Python and JavaScript have the same lookup** (`duckosm.dem_sources.sources_for_bbox`, `web/ground_step/ground_core.js`). There is no Rust function:
   the first host is a web page, and Python reads the file. A test runs the same cases in both. A Rust host can read the same JSON.
4. **The step is a web part, `web/ground_step/`.** The host gives the root element, the catalog and the box of the area. It calls `dem()` at the build and
   gets `{ name, bytes, product, vertical_datum, license, not_reference }`: the fields of `Params.dem_bytes`, `dem_product`, `dem_vertical_datum`,
   `dem_license` and `dem_not_reference`.
5. **A file from a source** (option A): a link to the download page and a file field. The product, the datum and the licence are the catalog's. The step reads the
   TIFF tags. A file that is not EPSG:4326 with one band of 32-bit floats is refused, with the command `gdalwarp -t_srs EPSG:4326 -ot Float32`.
   There is no reprojection in the browser.
6. **No file** (option C, the fallback): a DEM from the AWS Terrain Tiles (Terrarium PNG; the server sends `Access-Control-Allow-Origin: *`). The step takes the highest
   zoom (at most 14) with at most 64 tiles, samples the tiles bilinearly into an EPSG:4326 grid, and writes a GeoTIFF. The product is `DSM`, the datum is `mixed`,
   the label is "Ground: AWS Terrain Tiles" and `not_reference` is true. It is never a reference: a DSM does not pass the checks of the ground reference.
7. **No relay** (option B). The Copernicus COG files on AWS send no CORS header (checked on 2026-11-10), so a page cannot read them. A person who wants GLO-30 downloads it.

## Host contract

`initGroundStep({ root, catalog, bbox, status, onChange, loadTile })` returns `{ refresh(), dem(), choice() }`. `catalog` is the array or the URL of `dem_sources.json`
(a host copies it from `src/duckosm/data/`). The files are in `web/ground_step/`. A new option has a default, so an old host keeps working.
The part uses roadstyle's parts (`rs-opts`, `rs-opt`, `rs-cap`, `rs-link`, `rs-field-msg`) and the host's button `pick`.

## Tests

`tests/test_dem_sources.py` (Monaco, Tartu, Stockholm, Vancouver, the poles; the page; the Python providers), `web/ground_step/ground_units.mjs` (Node), and
`tests/test_ground_step.py` (rasterio reads the written GeoTIFF; the step in Chromium at a phone width).
