# Elevation sources

Where `duckosm elevation` can get its heights, and how the sources compare. How to run it:
[Add elevation](../guides/elevation.md).

There are two ways to give it a model:

- `--source` downloads a **global** model for the area: nothing to set up, 30 m.
- `--dem` reads a **file** you have, in any raster format GDAL reads: best where a national 1 m
  model exists.

## `--source`

| Name | Model | Resolution | Covers | Type | Needs |
|---|---|---|---|---|---|
| `copernicus` | Copernicus GLO-30 | 30 m | the whole world | DSM | nothing: streamed from AWS |
| `eudtm` | Continental Europe DTM (EU-DTM), from OpenTopography | 30 m | Europe | DTM | a free key in `OPENTOPOGRAPHY_API_KEY` |
| `auto` (default) | `eudtm` when the network lies completely inside Europe (longitude −25 to 45, latitude 34 to 72) and the key is set; otherwise `copernicus` | | | | |

Both use the EGM2008 vertical datum. A **DSM** is the top surface (roofs, trees); a **DTM** is the
bare ground ([below](#dtm-or-dsm)).

## Other global models

To use one of these, download it and pass it with `--dem`.

| Model | Resolution | Covers | Type | Access | Licence |
|---|---|---|---|---|---|
| Copernicus GLO-90 | 90 m | world | DSM | COG on AWS `copernicus-dem-90m`, no account | free, commercial use allowed (attribution) |
| EU-DEM v1.1 | 25 m | Europe | close to DTM | GeoTIFF in EPSG:3035, no account | Copernicus open, commercial use allowed |
| FABDEM v1.2 | 30 m | world (to 84°N) | DTM | GeoTIFF, no account | CC-BY-NC-SA: **not for commercial use** |
| SRTM v3 | 30 m | **60°N to 56°S only** | DSM | HGT / GeoTIFF, NASA Earthdata account | public domain |
| NASADEM | 30 m | **60°N to 56°S only** | DSM | GeoTIFF / HGT, NASA Earthdata account | public domain |
| ASTER GDEM v3 | 30 m | 83°N to 83°S | DSM | COG, NASA Earthdata account | free (attribution) |
| ALOS AW3D30 | 30 m | 82°N to 82°S | DSM | GeoTIFF, JAXA account | free (attribution) |
| GEBCO 2024 / ETOPO 2022 | about 450 m | land and sea | mixed | NetCDF / GeoTIFF | public domain |

SRTM and NASADEM stop at 60°N, so they miss most of Scandinavia. GEBCO and ETOPO are too coarse for
roads.

## National models

Many countries publish a 1 m DTM made from LiDAR. It shows cliffs, cuttings and bridge ramps that a
30 m model can't. Some examples:

| Country | Model | Resolution | CRS | Access | Licence |
|---|---|---|---|---|---|
| Sweden | Lantmäteriet Markhöjdmodell Nedladdning, grid 1+ | 1 m DTM (also a DSM) | EPSG:3006, heights RH2000 | STAC API `STAC-hojd`, after a free GeoTorget registration | CC0 |
| Estonia | Maa-amet Digital Terrain Model | 1, 5, 10, 25 m DTM (also a DSM) | EPSG:3301, heights EH2000 | map-sheet tiles or whole-country files, no account | open |
| Canada | NRCan HRDEM Mosaic | 1 m DTM and DSM, COG | EPSG:3979, heights CGVD2013 | STAC API `datacube.services.geo.ca/stac/api/` | OGL-Canada (attribution) |
| France | IGN RGE ALTI / LiDAR HD | 1 m DTM | EPSG:2154 | `geoservices.ign.fr` | Etalab 2.0 |

For other countries, search for the national mapping agency's open DTM. Any CRS works: node
coordinates are converted to the model's CRS before sampling.

## File formats

**Works with `--dem`**: any raster GDAL reads. The format is detected from the file.

| Format | Note |
|---|---|
| GeoTIFF (`.tif`) | the usual format |
| Cloud-Optimized GeoTIFF (COG) | a GeoTIFF that can be read in parts over HTTP: pass a URL and only the needed part is downloaded |
| VRT (`.vrt`) | a small XML file that joins many tiles into one: `gdalbuildvrt dem.vrt tiles/*.tif` |
| `.img`, `.asc`, `.hgt`, DTED | also read; convert a large `.asc` to GeoTIFF first |

**Doesn't work:**

- **LAS / LAZ** point clouds: points, not a grid. Use the provider's ready-made DTM, or make a
  raster from the ground points (classification 2) with PDAL.
- **WMS** services: they return pictures, not heights. A **WCS** service returns heights and GDAL can
  read it.

A **STAC** catalogue lists which tiles cover an area; each tile is a COG you can pass to `--dem` or
join in a VRT.

## DTM or DSM

- **DSM** (surface model): the top surface, with roofs, trees and bridge decks.
- **DTM** (terrain model): the bare ground.

For road gradients use a **DTM**. A DSM puts a node under trees on the tree tops and a node next to
a building on its roof edge, which gives false slopes. Copernicus GLO-30 is a DSM; EU-DTM, EU-DEM,
FABDEM and the national models above are DTMs.

With both a DTM and a DSM from the same survey, store the DSM as a second surface
(`--suffix dsm`): `ele_dsm - ele` is then the height of buildings and trees. Two unrelated 30 m
models give noise, not heights.

## Vertical datum

Heights are measured from a reference surface, and models differ: most use the geoid (about sea
level, e.g. EGM2008), GNSS uses the ellipsoid. The difference can be tens of metres. Heights from one
model compare fine; don't mix models. `main.elevation_metadata.vertical_datum` records it.

## Which one

| You want | Use |
|---|---|
| any area, no setup | `--source copernicus` (the `auto` default outside Europe) |
| Europe, bare ground | `--source eudtm`, with the key |
| the best heights in one country | `--dem` with its national 1 m DTM |
| bare ground, commercial use, outside Europe | a national DTM; not FABDEM |
