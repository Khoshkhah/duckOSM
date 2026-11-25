# Elevation sources

Where the heights come from, and how the sources compare. How to add them: [Add elevation](../guides/elevation.md).

| You want | Use |
|---|---|
| any area, no setup | `--source copernicus` (the `auto` default outside Europe) |
| Europe, bare ground | `--source eudtm`, with a free key |
| the best heights in one country | `--dem` with its national 1 m DTM |
| bare ground, commercial use, outside Europe | a national DTM; not FABDEM |

The rule for each area: [The ground reference](../concepts/ground-reference.md). It needs a bare-earth model (a DTM):
Copernicus is a surface model (a DSM).

## `--source`

| Name | Model | Resolution | Covers | Type | Needs |
|---|---|---|---|---|---|
| `copernicus` | Copernicus GLO-30 | 30 m | the whole world | DSM | nothing: streamed from AWS |
| `eudtm` | Continental Europe DTM (EU-DTM), from OpenTopography | 30 m | Europe | DTM | a free key in `OPENTOPOGRAPHY_API_KEY` |
| `auto` (default) | `eudtm` when the network lies completely inside the Europe rectangle (longitude −25 to 45, latitude 34 to 72) and the key is set; otherwise `copernicus` | | | | |

The Europe rectangle also covers North Africa north of 34°N, Turkey, Cyprus and the Caucasus.
`--dem` wins over `--source`. Copernicus has no tiles over open sea: nodes there get `--nodata-fill`
(default `0.0`).

Both use the EGM2008 vertical datum. The license goes in `main.elevation_metadata.license`:
`copernicus` writes "Copernicus open (attribution)", free with attribution; `eudtm` writes
"EU-DTM via OpenTopography (see provider terms)". A **DSM** is the top surface (roofs, trees); a **DTM** is the
bare ground ([below](#dtm-or-dsm)).

OpenTopography sends EU-DTM in EPSG:3035 (ETRS89 / LAEA Europe). `eudtm` converts it to EPSG:4326 with 32-bit
floats (the nearest pixel at each point), then samples it. `main.elevation_metadata` gives `dem_crs` EPSG:4326, and
`sha256` is the SHA-256 of the converted file. If OpenTopography sends a CRS that duckOSM cannot convert, the error
gives that CRS: convert the file with `gdalwarp` ([File formats](#file-formats)) and give it with `--dem`.

## Other global models

To use one of these, download it, convert it ([File formats](#file-formats)), and give it with `--dem`.

<!-- dem-sources:global -->
| Model | Resolution | Covers | Type | Access | Licence |
| --- | --- | --- | --- | --- | --- |
| FABDEM v1.2 | 30 m | world (60°S to 80°N) | DTM | GeoTIFF, no account | CC-BY-NC-SA: **not for commercial use** |
| Copernicus GLO-90 | 90 m | world | DSM | COG (Cloud-Optimized GeoTIFF, [below](#file-formats)) on AWS `copernicus-dem-90m`, no account | free, commercial use allowed (attribution) |
| EU-DEM v1.1 | 25 m | Europe | DSM, close to a DTM | GeoTIFF in EPSG:3035, no account | Copernicus open, commercial use allowed |
| SRTM v3 | 30 m | **60°N to 56°S only** | DSM | HGT / GeoTIFF, NASA Earthdata account | public domain |
| NASADEM | 30 m | **60°N to 56°S only** | DSM | GeoTIFF / HGT, NASA Earthdata account | public domain |
| ASTER GDEM v3 | 30 m | 83°N to 83°S | DSM | COG, NASA Earthdata account | free (attribution) |
| ALOS AW3D30 | 30 m | 82°N to 82°S | DSM | GeoTIFF, JAXA account | free (attribution) |
| GEBCO 2024 / ETOPO 2022 | about 450 m | land and sea | mixed | NetCDF / GeoTIFF | public domain |
<!-- /dem-sources:global -->

SRTM and NASADEM stop at 60°N, so they miss most of Scandinavia. GEBCO and ETOPO are too coarse for
roads.

## National models

Many countries publish a 1 m DTM made from LiDAR. It shows cliffs, cuttings and bridge ramps that a
30 m model cannot show. Some examples:

<!-- dem-sources:national -->
| Country | Model | Resolution | CRS (coordinate system) | Access | Licence |
| --- | --- | --- | --- | --- | --- |
| Sweden | Lantmäteriet Markhöjdmodell Nedladdning, grid 1+ | 1 m DTM (also a DSM) | EPSG:3006, heights RH2000 | STAC API `STAC-hojd`, after a free GeoTorget registration | CC0 |
| Estonia | Maa-amet Digital Terrain Model | 1, 5, 10, 25 m DTM (also a DSM) | EPSG:3301, heights EH2000 | map-sheet tiles or whole-country files, no account | open |
| Canada | NRCan HRDEM Mosaic | 1 m DTM and DSM, COG | EPSG:3979, heights CGVD2013 | STAC API `datacube.services.geo.ca/stac/api/` | OGL-Canada (attribution) |
| France | IGN RGE ALTI / LiDAR HD | 1 m DTM | EPSG:2154, heights IGN69 | `geoservices.ign.fr` | Etalab 2.0 |
<!-- /dem-sources:national -->

For other countries, find the open DTM of the national mapping agency. A national model has its own CRS:
convert it to EPSG:4326 before you give it to `--dem` ([File formats](#file-formats)).

## The order of the sources

duckOSM Studio suggests the sources that cover an area, in this order:

1. a national 1 m DTM, where one covers the area;
2. EU-DTM, in Europe;
3. FABDEM (a DTM, **not for commercial use**);
4. Copernicus GLO-30 (a DSM);
5. the other models, by resolution.

## File formats

`--dem` reads one format: a GeoTIFF in EPSG:4326, with one band of 32-bit floats, not compressed or compressed with
deflate. Give a file or an HTTP address.

To convert a model in a different format or CRS (a COG, a VRT of many tiles, `.img`, `.asc`, `.hgt`, a national
projection), use GDAL:

```bash
gdalbuildvrt dem.vrt tiles/*.tif                     # many tiles: join them first
gdalwarp -t_srs EPSG:4326 -ot Float32 -co COMPRESS=DEFLATE dem.vrt dem_4326.tif
duckosm elevation monaco.duckdb --dem dem_4326.tif --product DTM --vertical-datum EGM2008 --license "CC0"
```

These do not work:

| Input | Why | Do this |
|---|---|---|
| LAS / LAZ point clouds | points, not a grid | use the DTM of the provider, or make a raster from the ground points (class 2) with PDAL |
| a WMS service | it gives pictures, not heights | use a WCS service of the same provider, and convert its output with GDAL |

A **STAC** catalog lists which tiles cover an area. Each tile is a COG: download it and convert it as above.

## DTM or DSM

- **DSM** (surface model): the top surface, with roofs, trees and bridge decks.
- **DTM** (terrain model): the bare ground.

For road gradients, use a **DTM**. A DSM puts a node under trees on the tree tops, and a node next to
a building on its roof edge. This gives incorrect slopes. Copernicus GLO-30 is a DSM; EU-DTM, FABDEM
and the national models above are DTMs. EU-DEM is a DSM that is close to a DTM.

With both a DTM and a DSM from the same survey, store the DSM as a second surface
(`--suffix dsm`): `ele_dsm - ele` is then the height of buildings and trees. Two 30 m models from different
surveys give noise, not heights.

## Vertical datum

A height is measured from a reference surface. Most models use the geoid (about sea level, for example EGM2008); GNSS
uses the ellipsoid. The difference can be tens of metres, so do not mix models. `main.elevation_metadata.vertical_datum`
records it.
